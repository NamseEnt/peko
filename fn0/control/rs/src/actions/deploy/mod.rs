use crate::common::auth;
use crate::common::aws_sign;
use crate::common::byoc::ProjectStorage;
use crate::docs::*;
use crate::quota;
use forte_sdk::*;
use serde::{Deserialize, Serialize};

// code_version is minted client-side (it is baked into the static asset URL at
// build time, before this action is reached). A far-future value would win
// every `code_version >` promotion check forever and freeze the project, so
// the action only accepts values close to server time.
const CODE_VERSION_FUTURE_SKEW_MILLIS: u64 = 5 * 60 * 1000;
const CODE_VERSION_PAST_WINDOW_MILLIS: u64 = 24 * 60 * 60 * 1000;

// Static asset keys are `{project_id}/{code_version}/{path}`, so a deployed
// asset URL never changes content and needs no invalidation story. Without a
// stored Cache-Control the CDN bypasses the object and every request bills an
// R2 Class B operation. Signed into the presigned PUT so an upload cannot
// store a weaker policy.
const STATIC_ASSET_CACHE_CONTROL: &str = "public, max-age=31536000, immutable";

#[derive(Deserialize)]
pub struct Input {
    pub project_id: String,
    pub code_version: u64,
    // Required, not defaulted: a deploy that omits it (an older CLI) is
    // rejected at deserialization (400) so the bundle's presigned PUT can
    // never fall back to an unbounded size. See #55.
    pub bundle_size: u64,
    pub files: Vec<FileEntry>,
    // Signing Cache-Control into the static upload URLs makes the header
    // mandatory on the PUT, which a CLI that does not send it would fail with
    // an opaque 403. Older CLIs leave this false and keep the unsigned URLs
    // they know how to use.
    #[serde(default)]
    pub supports_static_asset_cache_control: bool,
    #[serde(default)]
    pub jobs: Vec<CronJob>,
    pub cron_updated_at: DateTime,
    #[serde(default)]
    pub websocket_singletons: Vec<WebSocketSingletonDeclaration>,
}

#[derive(Deserialize)]
pub struct FileEntry {
    pub path: String,
    pub size: u64,
}

#[derive(Serialize)]
pub enum Output {
    Ok {
        presigned_put_url: String,
        object_key: String,
        static_uploads: Vec<StaticUpload>,
    },
    QuotaExceeded {
        reason: String,
    },
    BadCodeVersion {
        reason: String,
    },
    InvalidWebSocketSingleton {
        reason: String,
    },
    NotLoggedIn,
    NotFound,
    InternalError,
}

#[derive(Serialize)]
pub struct StaticUpload {
    pub path: String,
    pub presigned_url: String,
    pub cache_control: String,
}

pub async fn handler(req: ForteRequest<'_, Input>) -> Output {
    let Some(user) = auth::bearer_user(req.headers).await else {
        return Output::NotLoggedIn;
    };

    let db = doc_db::turso();
    let project = match (ProjectDocGet {
        project_id: &req.body.project_id,
    })
    .send_with(&db)
    .await
    {
        Ok(Some(p)) => p,
        Ok(None) => return Output::NotFound,
        Err(e) => {
            tracing::error!("deploy ProjectDocGet: {e}");
            return Output::InternalError;
        }
    };

    if project.owner_github_id != user.github_id {
        return Output::NotFound;
    }

    let code_version = req.body.code_version;
    let now_millis: u64 = u64::try_from(forte_sdk::now().timestamp_millis())
        .expect("system clock returns positive timestamp");
    if code_version > now_millis.saturating_add(CODE_VERSION_FUTURE_SKEW_MILLIS)
        || code_version < now_millis.saturating_sub(CODE_VERSION_PAST_WINDOW_MILLIS)
    {
        return Output::BadCodeVersion {
            reason: format!(
                "code_version {code_version} is outside the accepted window of server time {now_millis}"
            ),
        };
    }

    if req.body.files.len() > quota::MAX_FILES_PER_BUILD {
        return Output::QuotaExceeded {
            reason: format!(
                "file count {} exceeds limit {}",
                req.body.files.len(),
                quota::MAX_FILES_PER_BUILD
            ),
        };
    }

    if let Err(reason) = validate_websocket_singletons(&req.body.websocket_singletons) {
        return Output::InvalidWebSocketSingleton { reason };
    }
    let total_size: u64 = req.body.files.iter().map(|f| f.size).sum();
    if total_size > quota::MAX_TOTAL_SIZE_PER_BUILD {
        return Output::QuotaExceeded {
            reason: format!(
                "total size {} bytes exceeds limit {}",
                total_size,
                quota::MAX_TOTAL_SIZE_PER_BUILD
            ),
        };
    }

    let storage = match ProjectStorage::resolve(&db, &req.body.project_id).await {
        Ok(storage) => storage,
        Err(e) => {
            tracing::error!("deploy ProjectStorage::resolve: {e}");
            return Output::InternalError;
        }
    };

    if let Err(e) = ensure_all_resources(&req.body.project_id).await {
        tracing::error!("deploy ensure_all_resources: {e}");
        return Output::InternalError;
    }

    let bundle_env = match BundleEnv::from_env() {
        Ok(v) => v,
        Err(e) => {
            tracing::error!("deploy BundleEnv::from_env: {e}");
            return Output::InternalError;
        }
    };
    let object_key = format!("original/{}/{}.tar", project.project_id, code_version);
    let presigned_put_url = aws_sign::r2_presign_put(aws_sign::R2PresignArgs {
        account_id: &bundle_env.account_id,
        bucket: &bundle_env.bucket,
        region: "auto",
        key: &object_key,
        access_key_id: &bundle_env.access_key_id,
        secret_access_key: &bundle_env.secret_access_key,
        expires_seconds: 600,
        now: forte_sdk::now(),
        content_length: Some(req.body.bundle_size),
        cache_control: None,
    });

    let static_uploads = if req.body.files.is_empty() {
        Vec::new()
    } else {
        let now_dt = forte_sdk::now();
        let cache_control = req
            .body
            .supports_static_asset_cache_control
            .then_some(STATIC_ASSET_CACHE_CONTROL);
        req.body
            .files
            .iter()
            .map(|f| {
                let key = format!("{}/{}", code_version, f.path);
                let url = aws_sign::r2_presign_put(aws_sign::R2PresignArgs {
                    account_id: &storage.account_id,
                    bucket: &storage.frontend_asset_bucket,
                    region: "auto",
                    key: &key,
                    access_key_id: &storage.frontend_asset_keys.access_key_id,
                    secret_access_key: &storage.frontend_asset_keys.secret_access_key,
                    expires_seconds: 600,
                    now: now_dt,
                    content_length: Some(f.size),
                    cache_control,
                });
                StaticUpload {
                    path: f.path.clone(),
                    presigned_url: url,
                    cache_control: cache_control.unwrap_or_default().to_string(),
                }
            })
            .collect()
    };

    if let Err(e) = upsert_cron_config(
        &db,
        req.body.project_id.clone(),
        req.body.jobs.clone(),
        req.body.cron_updated_at,
    )
    .await
    {
        tracing::error!("deploy upsert_cron_config: {e}");
        return Output::InternalError;
    }

    if let Err(e) = upsert_websocket_singleton_config(
        &db,
        req.body.project_id.clone(),
        code_version,
        req.body.websocket_singletons.clone(),
    )
    .await
    {
        tracing::error!("deploy upsert_websocket_singleton_config: {e}");
        return Output::InternalError;
    }

    Output::Ok {
        presigned_put_url,
        object_key,
        static_uploads,
    }
}

fn validate_websocket_singletons(
    declarations: &[WebSocketSingletonDeclaration],
) -> Result<(), String> {
    if declarations.len() > fn0_shared_schema::MAX_WEBSOCKET_SINGLETONS_PER_PROJECT {
        return Err(format!(
            "websocket singleton count {} exceeds limit {}",
            declarations.len(),
            fn0_shared_schema::MAX_WEBSOCKET_SINGLETONS_PER_PROJECT
        ));
    }
    let mut singleton_ids = std::collections::HashSet::new();
    for declaration in declarations {
        if declaration.singleton_id.is_empty() {
            return Err("singleton_id must not be empty".to_string());
        }
        if !singleton_ids.insert(declaration.singleton_id.as_str()) {
            return Err(format!(
                "duplicate singleton_id '{}'",
                declaration.singleton_id
            ));
        }
        if declaration.route_path != format!("/ws_singleton/{}", declaration.singleton_id)
            || declaration.singleton_id.split('/').any(|segment| {
                segment.is_empty()
                    || segment == "."
                    || segment == ".."
                    || segment.starts_with('[')
                    || segment.ends_with(']')
            })
        {
            return Err(format!(
                "invalid route_path for '{}'",
                declaration.singleton_id
            ));
        }
    }
    Ok(())
}

async fn upsert_websocket_singleton_config(
    db: &doc_db::Database,
    project_id: String,
    code_version: u64,
    declarations: Vec<WebSocketSingletonDeclaration>,
) -> Result<(), String> {
    let result = db
        .trx(|trx| {
            let project_id = project_id.clone();
            let declarations = declarations.clone();
            async move {
                match trx
                    .get(WebSocketSingletonConfigDocGet {
                        project_id: project_id.as_str(),
                        code_version,
                    })
                    .await?
                {
                    Some(mut handle) => {
                        handle.declarations = declarations;
                    }
                    None => {
                        trx.create(WebSocketSingletonConfigDoc {
                            project_id,
                            code_version,
                            declarations,
                        })?;
                    }
                }
                trx.commit::<_, ()>(())
            }
        })
        .await;
    match result {
        doc_db::TrxResult::Committed(()) => Ok(()),
        doc_db::TrxResult::Cancelled(()) => unreachable!(),
        doc_db::TrxResult::Conflict(_) => Err("websocket singleton config conflict".to_string()),
        doc_db::TrxResult::Err(error) => Err(error.to_string()),
    }
}

async fn upsert_cron_config(
    db: &doc_db::Database,
    project_id: String,
    jobs: Vec<CronJob>,
    updated_at: DateTime,
) -> Result<(), String> {
    let result = db
        .trx(|trx| {
            let project_id = project_id.clone();
            let jobs = jobs.clone();
            async move {
                let existing = trx
                    .get(CronConfigDocGet {
                        project_id: project_id.as_str(),
                    })
                    .await?;
                match existing {
                    Some(mut handle) => {
                        if handle.updated_at < updated_at {
                            handle.jobs = jobs;
                            handle.updated_at = updated_at;
                        }
                    }
                    None => {
                        trx.create(CronConfigDoc {
                            project_id,
                            jobs,
                            updated_at,
                        })?;
                    }
                }
                trx.commit::<_, ()>(())
            }
        })
        .await;
    match result {
        doc_db::TrxResult::Committed(()) => Ok(()),
        doc_db::TrxResult::Cancelled(()) => unreachable!(),
        doc_db::TrxResult::Conflict(_) => Err("cron config conflict".to_string()),
        doc_db::TrxResult::Err(e) => Err(e.to_string()),
    }
}

struct BundleEnv {
    account_id: String,
    bucket: String,
    access_key_id: String,
    secret_access_key: String,
}

impl BundleEnv {
    fn from_env() -> anyhow::Result<Self> {
        Ok(Self {
            account_id: std::env::var("FN0_BUNDLE_STORE_ACCOUNT_ID")
                .map_err(|_| anyhow::anyhow!("FN0_BUNDLE_STORE_ACCOUNT_ID not set"))?,
            bucket: std::env::var("FN0_BUNDLE_STORE_BUCKET")
                .map_err(|_| anyhow::anyhow!("FN0_BUNDLE_STORE_BUCKET not set"))?,
            access_key_id: std::env::var("FN0_BUNDLE_STORE_ACCESS_KEY_ID")
                .map_err(|_| anyhow::anyhow!("FN0_BUNDLE_STORE_ACCESS_KEY_ID not set"))?,
            secret_access_key: std::env::var("FN0_BUNDLE_STORE_SECRET_ACCESS_KEY")
                .map_err(|_| anyhow::anyhow!("FN0_BUNDLE_STORE_SECRET_ACCESS_KEY not set"))?,
        })
    }
}

/// Buckets are deliberately absent here. They were created by the CLI at
/// connect time with a token that is gone, and `cloudflare_connect` refuses a
/// configuration whose buckets it cannot reach — so by the time a deploy runs,
/// they exist.
async fn ensure_all_resources(project_id: &str) -> anyhow::Result<()> {
    ensure_turso_database(project_id).await
}

async fn ensure_turso_database(project_id: &str) -> anyhow::Result<()> {
    let api_token = std::env::var("FN0_TURSO_API_TOKEN")
        .map_err(|_| anyhow::anyhow!("FN0_TURSO_API_TOKEN not set"))?;
    let org_slug = std::env::var("FN0_TURSO_ORG_SLUG")
        .map_err(|_| anyhow::anyhow!("FN0_TURSO_ORG_SLUG not set"))?;
    let group_name = std::env::var("FN0_TURSO_GROUP_NAME")
        .map_err(|_| anyhow::anyhow!("FN0_TURSO_GROUP_NAME not set"))?;

    let url = format!("https://api.turso.tech/v1/organizations/{org_slug}/databases");
    let body = serde_json::to_vec(&serde_json::json!({
        "name": project_id,
        "group": group_name,
    }))?;
    let req = http::Request::builder()
        .uri(url)
        .method("POST")
        .header("Authorization", format!("Bearer {api_token}"))
        .header("Content-Type", "application/json")
        .body(body)?;
    let resp = http::Client::new().send(req).await?;
    let status = resp.status().as_u16();
    let body_bytes = resp.into_body().bytes().await?.to_vec();
    if (200..300).contains(&status) || status == 409 || database_already_exists(&body_bytes) {
        return Ok(());
    }
    anyhow::bail!(
        "turso ensure_database {project_id} failed (status={status}): {}",
        String::from_utf8_lossy(&body_bytes)
    );
}

/// Turso's docs promise 409 with `database with name <name> already exists`, but
/// since 2026-07-30 it answers a duplicate name with 400 and the message below,
/// which broke every deploy: this call is the provisioning backstop each one runs.
/// Reported as https://github.com/tursodatabase/turso-docs/issues/409.
///
/// The whole message is compared rather than a fragment, so a reworded error
/// fails the deploy loudly instead of being read as something it is not.
fn database_already_exists(body: &[u8]) -> bool {
    const ALREADY_EXISTS: &str = "database with same name already exists";

    #[derive(serde::Deserialize)]
    struct TursoError {
        error: String,
    }

    serde_json::from_slice::<TursoError>(body).is_ok_and(|parsed| parsed.error == ALREADY_EXISTS)
}

#[cfg(test)]
mod websocket_singleton_validation_tests {
    use super::validate_websocket_singletons;
    use crate::docs::WebSocketSingletonDeclaration;

    fn declarations(count: usize) -> Vec<WebSocketSingletonDeclaration> {
        (0..count)
            .map(|declaration_index| WebSocketSingletonDeclaration {
                singleton_id: format!("feed-{declaration_index}"),
                route_path: format!("/ws_singleton/feed-{declaration_index}"),
            })
            .collect()
    }

    #[test]
    fn accepts_singleton_limit() {
        let declarations = declarations(fn0_shared_schema::MAX_WEBSOCKET_SINGLETONS_PER_PROJECT);
        assert!(validate_websocket_singletons(&declarations).is_ok());
    }

    #[test]
    fn rejects_more_than_singleton_limit() {
        let declarations =
            declarations(fn0_shared_schema::MAX_WEBSOCKET_SINGLETONS_PER_PROJECT + 1);
        let error = validate_websocket_singletons(&declarations).unwrap_err();
        assert!(error.contains("exceeds limit"));
    }
}
