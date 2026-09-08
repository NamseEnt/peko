//! Full teardown of one project, enqueued by the `delete_project` action.
//!
//! The queue is at-least-once with redelivery on failure, so every step
//! tolerates already-deleted resources and the identity docs (`ProjectDoc`,
//! the owner's `UserDoc.projects` entry) are removed last: a redelivered
//! message re-runs the whole sequence and converges.
//!
//! Known race: the owner can re-deploy the project while teardown is in
//! flight and resurrect some resources. That is the owner racing their own
//! destroy, so it is accepted rather than locked against.

use crate::common::byoc::{self, ProjectStorage};
use crate::common::r2_store::{BundleStore, ProjectR2Store, parse_compiled_key};
use crate::docs::*;
use forte_sdk::*;
use serde::{Deserialize, Serialize};

const QUERY_PAGE_LIMIT: usize = 256;

#[derive(Serialize, Deserialize)]
pub struct Input {
    pub project_id: String,
}

pub async fn handle(input: Input) -> anyhow::Result<()> {
    let project_id = input.project_id;
    let db = doc_db::turso();
    let now = forte_sdk::now();

    let storage = ProjectStorage::resolve_if_connected(&db, &project_id).await?;

    delete_domain(&db, &project_id).await?;
    remove_routing_and_cron(&db, &project_id).await?;
    delete_bundle_store_objects(&project_id, now).await?;
    if let Some(storage) = &storage {
        empty_project_buckets(&project_id, storage, now).await?;
    }
    delete_cloudflare_config(&db, &project_id).await?;
    delete_turso_database(&project_id).await?;
    delete_compiled_bundle_docs(&db, &project_id).await?;
    delete_identity_docs(&db, &project_id).await?;

    tracing::info!(%project_id, "project_teardown complete");
    Ok(())
}

// The hostname registration must go before the manifest entry: the domain name
// only lives in the manifest, so deleting the entry first would strand the
// hostname on a redelivered message.
async fn delete_domain(db: &doc_db::Database, project_id: &str) -> anyhow::Result<()> {
    let Some(manifest) = (WorkerManifestDocGet {}).send_with(db).await? else {
        return Ok(());
    };
    let Some(domain) = manifest
        .project_manifests
        .get(project_id)
        .map(|entry| entry.domain.clone())
    else {
        return Ok(());
    };
    if domain.is_empty() {
        return Ok(());
    }
    crate::common::cert_manifest::remove(db, &domain).await?;
    tracing::info!(%project_id, %domain, "project_teardown: domain deleted");
    Ok(())
}

async fn remove_routing_and_cron(db: &doc_db::Database, project_id: &str) -> anyhow::Result<()> {
    let result = db
        .trx(|trx| {
            let project_id = project_id.to_string();
            async move {
                if let Some(mut manifest) = trx.get(WorkerManifestDocGet {}).await?
                    && manifest.project_manifests.contains_key(&project_id)
                {
                    manifest.project_manifests.remove(&project_id);
                    manifest.manifest_version += 1;
                }
                if let Some(cron) = trx
                    .get(CronConfigDocGet {
                        project_id: &project_id,
                    })
                    .await?
                {
                    cron.delete();
                }
                trx.commit::<_, std::convert::Infallible>(())
            }
        })
        .await;
    match result {
        doc_db::TrxResult::Committed(()) => Ok(()),
        doc_db::TrxResult::Cancelled(cancel) => match cancel {},
        doc_db::TrxResult::Conflict(d) => {
            anyhow::bail!("remove_routing_and_cron trx conflict: {d:?}")
        }
        doc_db::TrxResult::Err(e) => Err(e),
    }
}

async fn delete_bundle_store_objects(project_id: &str, now: DateTime) -> anyhow::Result<()> {
    let store = BundleStore::from_env()?;
    for object in store
        .list_all(&format!("original/{project_id}/"), now)
        .await?
    {
        store.delete(&object.key, now).await?;
    }
    for object in store.list_all("compiled/", now).await? {
        let Some((key_project_id, _)) = parse_compiled_key(&object.key) else {
            continue;
        };
        if key_project_id == project_id {
            store.delete(&object.key, now).await?;
        }
    }
    Ok(())
}

/// Empties every bucket the project owns.
///
/// The buckets themselves are left standing: fn0 holds only object-scoped
/// credentials there, by design, and they sit in the owner's own account for
/// them to remove.
///
/// A bucket that cannot be listed is logged and skipped rather than failing the
/// teardown. S3 `ListObjectsV2` errors on a missing bucket instead of returning
/// an empty page, and a project that never wrote may not have one.
async fn empty_project_buckets(
    project_id: &str,
    storage: &ProjectStorage,
    now: DateTime,
) -> anyhow::Result<()> {
    for store in [
        ProjectR2Store::frontend_assets(storage),
        ProjectR2Store::private_objects(storage),
        ProjectR2Store::public_objects(storage),
    ] {
        match store.list_all("", now).await {
            Ok(objects) => {
                for object in objects {
                    store.delete(&object.key, now).await?;
                }
            }
            Err(error) => {
                let bucket = store.bucket();
                tracing::warn!(%project_id, %bucket, %error, "project_teardown: bucket unreadable, skipping");
            }
        }
    }
    Ok(())
}

async fn delete_cloudflare_config(db: &doc_db::Database, project_id: &str) -> anyhow::Result<()> {
    byoc::publish_storage_to_manifest(db, project_id, None).await?;
    (ProjectCloudflareConfigDocDelete { project_id })
        .send_with(db)
        .await?;
    Ok(())
}

async fn delete_turso_database(project_id: &str) -> anyhow::Result<()> {
    let api_token = std::env::var("FN0_TURSO_API_TOKEN")
        .map_err(|_| anyhow::anyhow!("FN0_TURSO_API_TOKEN not set"))?;
    let org_slug = std::env::var("FN0_TURSO_ORG_SLUG")
        .map_err(|_| anyhow::anyhow!("FN0_TURSO_ORG_SLUG not set"))?;

    let url = format!("https://api.turso.tech/v1/organizations/{org_slug}/databases/{project_id}");
    let req = http::Request::builder()
        .uri(url)
        .method("DELETE")
        .header("Authorization", format!("Bearer {api_token}"))
        .body(Vec::new())?;
    let resp = http::Client::new().send(req).await?;
    let status = resp.status().as_u16();
    if (200..300).contains(&status) || status == 404 {
        return Ok(());
    }
    let body_bytes = resp.into_body().bytes().await?.to_vec();
    anyhow::bail!(
        "turso delete_database {project_id} failed (status={status}): {}",
        String::from_utf8_lossy(&body_bytes)
    );
}

async fn delete_compiled_bundle_docs(
    db: &doc_db::Database,
    project_id: &str,
) -> anyhow::Result<()> {
    loop {
        let page: Vec<CompiledBundleDoc> = (CompiledBundleDocQuery {
            project_id,
            code_version: None,
            limit: Some(QUERY_PAGE_LIMIT),
        })
        .send_with(db)
        .await?;
        if page.is_empty() {
            return Ok(());
        }
        for doc in &page {
            (CompiledBundleDocDelete {
                project_id,
                code_version: doc.code_version,
            })
            .send_with(db)
            .await?;
        }
    }
}

async fn delete_identity_docs(db: &doc_db::Database, project_id: &str) -> anyhow::Result<()> {
    let result = db
        .trx(|trx| {
            let project_id = project_id.to_string();
            async move {
                let owner_github_id = match trx
                    .get(ProjectDocGet {
                        project_id: &project_id,
                    })
                    .await?
                {
                    Some(project) => {
                        let github_id = project.owner_github_id;
                        project.delete();
                        Some(github_id)
                    }
                    None => None,
                };
                if let Some(github_id) = owner_github_id
                    && let Some(mut user) = trx.get(UserDocGet { github_id }).await?
                    && user.projects.iter().any(|e| e.project_id == project_id)
                {
                    user.projects.retain(|e| e.project_id != project_id);
                }
                trx.commit::<_, std::convert::Infallible>(())
            }
        })
        .await;
    match result {
        doc_db::TrxResult::Committed(()) => Ok(()),
        doc_db::TrxResult::Cancelled(cancel) => match cancel {},
        doc_db::TrxResult::Conflict(d) => {
            anyhow::bail!("delete_identity_docs trx conflict: {d:?}")
        }
        doc_db::TrxResult::Err(e) => Err(e),
    }
}
