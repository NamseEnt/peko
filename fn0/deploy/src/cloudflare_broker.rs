use anyhow::{Context, Result, anyhow};
use reqwest::multipart::{Form, Part};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

use crate::cloudflare_provision::{
    ConnectCredentials, IssuedCertificate, MintedCredentialIds, ProvisionedResources, ReachableZone,
};

const API_BASE: &str = "https://api.cloudflare.com/client/v4";
const BROKER_SCRIPT_NAME: &str = "fn0-broker";
const STORE_NAME: &str = "fn0";
const SETUP_SECRET_NAME: &str = "FN0_SETUP_TOKEN";
const WORKER_COMPATIBILITY_DATE: &str = "2026-08-28";
const BOOTSTRAP_TOKEN_MINUTES: i64 = 10;
const BROKER_SETTINGS_FILE: &str = "cloudflare-broker";
const BROKER_READINESS_ATTEMPTS: u32 = 20;
const BROKER_READINESS_POLL: std::time::Duration = std::time::Duration::from_secs(1);

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BrokerSettings {
    pub account_id: String,
    pub broker_url: String,
}

pub fn broker_settings_path() -> Result<PathBuf> {
    let config_dir =
        dirs::config_dir().ok_or_else(|| anyhow!("could not locate user config dir"))?;
    Ok(config_dir.join("fn0").join(BROKER_SETTINGS_FILE))
}

pub fn load_broker_settings() -> Result<Option<BrokerSettings>> {
    let path = broker_settings_path()?;
    if !path.exists() {
        return Ok(None);
    }
    let content = std::fs::read_to_string(path)?;
    Ok(Some(toml::from_str(&content)?))
}

pub fn save_broker_settings(settings: &BrokerSettings) -> Result<()> {
    let path = broker_settings_path()?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, toml::to_string(settings)?)?;
    Ok(())
}

pub fn clear_broker_settings() -> Result<()> {
    let path = broker_settings_path()?;
    if path.exists() {
        std::fs::remove_file(path)?;
    }
    Ok(())
}

#[derive(Debug, Clone)]
pub struct BrokerClient {
    client: reqwest::Client,
    base_url: String,
    control_token: String,
    account_id: String,
}

#[derive(Debug, Clone, Serialize)]
struct ResolveZoneInput<'a> {
    zone_name: &'a str,
}

#[derive(Debug, Deserialize)]
struct BrokerProvisionResponse {
    zone_name: String,
    frontend_asset_hostname: String,
    public_object_storage_hostname: String,
    private_object_storage_bucket: String,
    public_object_storage_bucket: String,
    frontend_asset_bucket: String,
    worker_access_key_id: String,
    worker_secret: String,
    frontend_asset_access_key_id: String,
    frontend_asset_secret: String,
    purge_token: String,
    minted_token_ids: MintedCredentialIdsResponse,
}

#[derive(Debug, Deserialize)]
struct MintedCredentialIdsResponse {
    worker: String,
    frontend_asset: String,
    purge: String,
}

#[derive(Debug, Serialize)]
struct ProvisionProjectInput<'a> {
    project_id: &'a str,
    zone_id: &'a str,
    app_origin: &'a str,
    app_hostname: &'a str,
}

#[derive(Debug, Serialize)]
struct ZoneInput<'a> {
    project_id: &'a str,
    zone_id: &'a str,
}

#[derive(Debug, Serialize)]
struct CertificateInput<'a> {
    project_id: &'a str,
    zone_id: &'a str,
    hostname: &'a str,
    csr: &'a str,
}

#[derive(Debug, Serialize)]
struct RotateTokenInput<'a> {
    new_setup_token: &'a str,
}

#[derive(Debug, Serialize)]
struct FinalizeDomainInput<'a> {
    project_id: &'a str,
    zone_id: &'a str,
    zone_name: &'a str,
    app_hostname: &'a str,
    origin_hostname: &'a str,
    replaced_app_hostname: Option<&'a str>,
}

#[derive(Debug, Serialize)]
struct RevokeProjectCredentialsInput<'a> {
    project_id: &'a str,
    worker: &'a str,
    frontend_asset: &'a str,
    purge: &'a str,
}

#[derive(Debug, Serialize)]
struct TeardownProjectInput<'a> {
    project_id: &'a str,
    zone_id: &'a str,
    zone_name: &'a str,
    app_hostname: &'a str,
    delete_buckets: bool,
}

#[derive(Debug, Deserialize)]
pub struct TeardownProjectOutcome {
    /// Human-readable lines about anything the broker deliberately left alone
    /// (a DNS record the owner edited, a bucket teardown had not finished
    /// clearing). Empty on a clean run.
    #[serde(default)]
    pub notes: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct OriginCertificateResponse {
    certificate_pem: String,
    not_after_epoch_seconds: i64,
}

#[derive(Debug, Deserialize)]
struct Store {
    id: String,
}

#[derive(Debug, Deserialize)]
struct Secret {
    id: String,
    name: String,
}

#[derive(Debug, Deserialize)]
struct WorkerSubdomain {
    subdomain: String,
}

#[derive(Debug, Deserialize)]
struct CloudflareEnvelope<T> {
    success: bool,
    #[serde(default)]
    errors: Vec<CloudflareError>,
    result: Option<T>,
}

#[derive(Debug, Deserialize)]
struct CloudflareError {
    #[serde(default)]
    code: i64,
    #[serde(default)]
    message: String,
}

impl BrokerClient {
    pub fn new(
        broker_url: String,
        control_url: String,
        control_token: String,
        account_id: String,
    ) -> Result<Self> {
        validate_account_id(&account_id)?;
        let base_url = normalize_https_url(&broker_url, "broker URL")?;
        normalize_https_url(&control_url, "control URL")?;
        Ok(Self {
            client: reqwest::Client::new(),
            base_url,
            control_token,
            account_id,
        })
    }

    pub async fn bootstrap(
        setup_token: String,
        account_id: String,
        control_url: String,
        control_token: String,
    ) -> Result<Self> {
        let control_url = normalize_https_url(&control_url, "control URL")?;
        Self::bootstrap_with_cloudflare_api_base(
            setup_token,
            account_id,
            control_url,
            control_token,
            API_BASE,
            BROKER_READINESS_ATTEMPTS,
        )
        .await
    }

    /// Split out of `bootstrap` so tests can point every Cloudflare and
    /// control-plane call at a local mock server instead of the real,
    /// HTTPS-only `api.cloudflare.com` and control URLs. `control_url` is
    /// trusted here rather than re-validated: `bootstrap` already normalized
    /// it, and a test double for it is plain HTTP. `broker_readiness_attempts`
    /// is 0 in tests: the readiness check below hits the freshly-deployed
    /// Worker's own real `*.workers.dev` URL, which does not exist in a test.
    async fn bootstrap_with_cloudflare_api_base(
        setup_token: String,
        account_id: String,
        control_url: String,
        control_token: String,
        cloudflare_api_base: &str,
        broker_readiness_attempts: u32,
    ) -> Result<Self> {
        validate_account_id(&account_id)?;
        let client = reqwest::Client::new();
        let setup_token_id = verify_setup_token(&client, cloudflare_api_base, &setup_token).await?;
        let owner_github_id =
            authorize_control_user(&client, &control_url, &control_token, &account_id).await?;
        let setup_token =
            roll_setup_token_value(&client, cloudflare_api_base, &setup_token, &setup_token_id)
                .await?;
        let bootstrap =
            mint_bootstrap_token(&client, cloudflare_api_base, &setup_token, &account_id).await?;
        let result = async {
            let store_id =
                ensure_secret_store(&client, cloudflare_api_base, &bootstrap.value, &account_id)
                    .await?;
            ensure_setup_secret(
                &client,
                cloudflare_api_base,
                &bootstrap.value,
                &account_id,
                &store_id,
                &setup_token,
            )
            .await?;
            upload_worker(
                &client,
                cloudflare_api_base,
                &bootstrap.value,
                &account_id,
                &control_url,
                &store_id,
                owner_github_id,
            )
            .await?;
            let subdomain = ensure_workers_subdomain(
                &client,
                cloudflare_api_base,
                &bootstrap.value,
                &account_id,
            )
            .await?;
            Ok::<String, anyhow::Error>(subdomain)
        }
        .await;
        if let Err(error) =
            revoke_cloudflare_token(&client, cloudflare_api_base, &setup_token, &bootstrap.id).await
        {
            eprintln!("warning: could not revoke the temporary broker bootstrap token: {error}");
        }
        let subdomain = result?;
        let broker_url = format!("https://{BROKER_SCRIPT_NAME}.{subdomain}.workers.dev");
        wait_for_broker_ready(&client, &broker_url, broker_readiness_attempts).await?;
        // Not `Self::new`: `control_url` was already validated by `bootstrap`
        // (or, in tests, stands in for a plain-HTTP mock) and `BrokerClient`
        // never stores it, so re-checking it here would only reject that mock.
        let base_url = normalize_https_url(&broker_url, "broker URL")?;
        Ok(Self {
            client: reqwest::Client::new(),
            base_url,
            control_token,
            account_id,
        })
    }

    pub fn account_id(&self) -> &str {
        &self.account_id
    }

    pub fn url(&self) -> &str {
        &self.base_url
    }

    pub fn settings(&self) -> BrokerSettings {
        BrokerSettings {
            account_id: self.account_id.clone(),
            broker_url: self.base_url.clone(),
        }
    }

    pub async fn resolve_zone(&self, zone_name: &str) -> Result<ReachableZone> {
        let zone: ReachableZone = self
            .post_json("/v1/resolve-zone", &ResolveZoneInput { zone_name }, true)
            .await?;
        if zone.account_id != self.account_id {
            return Err(anyhow!(
                "the broker returned a zone from account {}, expected {}",
                zone.account_id,
                self.account_id
            ));
        }
        Ok(zone)
    }

    pub async fn provision_project(
        &self,
        project_id: &str,
        zone_id: &str,
        app_origin: &str,
        app_hostname: &str,
    ) -> Result<BrokerProvisioned> {
        let response: BrokerProvisionResponse = self
            .post_json(
                "/v1/provision-project",
                &ProvisionProjectInput {
                    project_id,
                    zone_id,
                    app_origin,
                    app_hostname,
                },
                true,
            )
            .await?;
        Ok(BrokerProvisioned {
            resources: ProvisionedResources {
                zone_name: response.zone_name,
                frontend_asset_hostname: response.frontend_asset_hostname,
                public_object_storage_hostname: response.public_object_storage_hostname,
                private_object_storage_bucket: response.private_object_storage_bucket,
                public_object_storage_bucket: response.public_object_storage_bucket,
                frontend_asset_bucket: response.frontend_asset_bucket,
            },
            credentials: ConnectCredentials {
                worker_access_key_id: response.worker_access_key_id,
                worker_secret: response.worker_secret,
                frontend_asset_access_key_id: response.frontend_asset_access_key_id,
                frontend_asset_secret: response.frontend_asset_secret,
                purge_token: response.purge_token,
            },
            minted: MintedCredentialIds {
                worker: response.minted_token_ids.worker,
                frontend_asset: response.minted_token_ids.frontend_asset,
                purge: response.minted_token_ids.purge,
            },
        })
    }

    pub async fn ensure_websockets(&self, project_id: &str, zone_id: &str) -> Result<()> {
        self.post_empty(
            "/v1/ensure-websockets",
            &ZoneInput {
                project_id,
                zone_id,
            },
        )
        .await
    }

    pub async fn issue_origin_certificate(
        &self,
        project_id: &str,
        zone_id: &str,
        hostname: &str,
    ) -> Result<IssuedCertificate> {
        let key_pair = rcgen::KeyPair::generate()
            .map_err(|error| anyhow!("could not generate a key pair: {error}"))?;
        let mut params = rcgen::CertificateParams::new(vec![hostname.to_string()])
            .map_err(|error| anyhow!("could not build the certificate request: {error}"))?;
        params.distinguished_name = rcgen::DistinguishedName::new();
        params
            .distinguished_name
            .push(rcgen::DnType::CommonName, hostname);
        let csr = params
            .serialize_request(&key_pair)
            .map_err(|error| anyhow!("could not sign the certificate request: {error}"))?
            .pem()
            .map_err(|error| anyhow!("could not encode the certificate request: {error}"))?;
        let response: OriginCertificateResponse = self
            .post_json(
                "/v1/issue-origin-certificate",
                &CertificateInput {
                    project_id,
                    zone_id,
                    hostname,
                    csr: &csr,
                },
                true,
            )
            .await?;
        Ok(IssuedCertificate {
            certificate_pem: response.certificate_pem,
            private_key_pem: key_pair.serialize_pem(),
            not_after_epoch_seconds: response.not_after_epoch_seconds,
        })
    }

    pub async fn finalize_domain(
        &self,
        project_id: &str,
        zone_id: &str,
        zone_name: &str,
        app_hostname: &str,
        origin_hostname: &str,
        replaced_app_hostname: Option<&str>,
    ) -> Result<()> {
        self.post_empty(
            "/v1/finalize-domain",
            &FinalizeDomainInput {
                project_id,
                zone_id,
                zone_name,
                app_hostname,
                origin_hostname,
                replaced_app_hostname,
            },
        )
        .await
    }

    pub async fn revoke_project_credentials(
        &self,
        project_id: &str,
        ids: &MintedCredentialIds,
    ) -> Result<()> {
        self.post_empty(
            "/v1/revoke-project-credentials",
            &RevokeProjectCredentialsInput {
                project_id,
                worker: &ids.worker,
                frontend_asset: &ids.frontend_asset,
                purge: &ids.purge,
            },
        )
        .await
    }

    /// Removes the project's Cloudflare footprint that `forte destroy`'s
    /// fn0-side teardown cannot reach: the app hostname's DNS record, the two
    /// public buckets' custom domains, the origin certificate, and the three
    /// minted R2/purge tokens. With `delete_buckets`, also deletes the three
    /// (by then empty) buckets. Every step tolerates its target already being
    /// gone.
    pub async fn teardown_project(
        &self,
        project_id: &str,
        zone_id: &str,
        zone_name: &str,
        app_hostname: &str,
        delete_buckets: bool,
    ) -> Result<TeardownProjectOutcome> {
        self.post_json(
            "/v1/teardown-project",
            &TeardownProjectInput {
                project_id,
                zone_id,
                zone_name,
                app_hostname,
                delete_buckets,
            },
            true,
        )
        .await
    }

    pub async fn rotate_setup_token(&self, new_setup_token: &str) -> Result<()> {
        self.post_empty("/v1/rotate-token", &RotateTokenInput { new_setup_token })
            .await
    }

    pub async fn clear_setup_token(&self) -> Result<()> {
        self.post_empty("/v1/clear-token", &serde_json::json!({}))
            .await
    }

    /// Tears the broker down entirely: the Worker script, its Secrets
    /// Store, and the setup token stored in it. Every project on this
    /// Cloudflare account that used this broker needs `forte cloud init`
    /// run again afterward.
    pub async fn destroy_broker(&self) -> Result<()> {
        self.post_empty("/v1/destroy-broker", &serde_json::json!({}))
            .await
    }

    async fn post_json<T: for<'de> Deserialize<'de>, B: Serialize>(
        &self,
        path: &str,
        body: &B,
        include_auth: bool,
    ) -> Result<T> {
        let mut request = self
            .client
            .post(format!("{}{}", self.base_url, path))
            .header("x-forte-request-id", uuid::Uuid::new_v4().to_string())
            .header(
                "x-forte-request-timestamp",
                chrono::Utc::now().timestamp().to_string(),
            )
            .json(body);
        if include_auth {
            if self.control_token.is_empty() {
                return Err(anyhow!("the broker has no control-plane credential"));
            }
            request = request.bearer_auth(&self.control_token);
        }
        let response = request.send().await?;
        let status = response.status();
        let text = response.text().await?;
        if !status.is_success() {
            return Err(anyhow!("broker request {path} failed ({status}): {text}"));
        }
        serde_json::from_str(&text)
            .with_context(|| format!("broker request {path} returned invalid JSON"))
    }

    async fn post_empty<B: Serialize>(&self, path: &str, body: &B) -> Result<()> {
        let _: serde_json::Value = self.post_json(path, body, true).await?;
        Ok(())
    }
}

pub struct BrokerProvisioned {
    pub resources: ProvisionedResources,
    pub credentials: ConnectCredentials,
    pub minted: MintedCredentialIds,
}

/// Confirms the token is live and returns its own Cloudflare token id, which
/// `bootstrap` keeps so it can revoke this exact token once a fresh clone has
/// taken its place in the broker's Secrets Store.
async fn verify_setup_token(
    client: &reqwest::Client,
    cloudflare_api_base: &str,
    token: &str,
) -> Result<String> {
    #[derive(Deserialize)]
    struct VerifiedToken {
        id: String,
        status: String,
    }
    let verified: VerifiedToken = cloudflare_json(
        client,
        cloudflare_api_base,
        token,
        reqwest::Method::GET,
        "/user/tokens/verify",
        None,
    )
    .await?;
    if verified.status != "active" {
        return Err(anyhow!(
            "the Cloudflare API token is {}, not active",
            verified.status
        ));
    }
    Ok(verified.id)
}

/// Rolls the token's secret in place (`PUT /user/tokens/{id}/value`) and returns
/// the new value. The value the caller passed in — off the clipboard, in the
/// real flow, where a clipboard-history tool may keep a copy — stops working
/// the moment the roll lands. Cloudflare refuses to mint a token with "API
/// Tokens" permissions through the API ("sub-token is not allowed to have
/// permissions to manage other tokens"), so the setup token cannot be
/// re-created; rolling its secret is the only way to retire the copied value
/// while keeping a working setup token, and it keeps the token's id, name, and
/// policies untouched.
async fn roll_setup_token_value(
    client: &reqwest::Client,
    cloudflare_api_base: &str,
    token: &str,
    token_id: &str,
) -> Result<String> {
    let rolled: String = cloudflare_json(
        client,
        cloudflare_api_base,
        token,
        reqwest::Method::PUT,
        &format!("/user/tokens/{token_id}/value"),
        Some(serde_json::json!({})),
    )
    .await?;
    wait_until_token_active(client, cloudflare_api_base, &rolled).await?;
    Ok(rolled)
}

/// A freshly rolled secret is not usable at Cloudflare's edge for a second or
/// two; poll `verify` so the next call does not race the roll.
async fn wait_until_token_active(
    client: &reqwest::Client,
    cloudflare_api_base: &str,
    token: &str,
) -> Result<()> {
    for attempt in 0..10 {
        if attempt > 0 {
            tokio::time::sleep(std::time::Duration::from_secs(1)).await;
        }
        if verify_setup_token(client, cloudflare_api_base, token)
            .await
            .is_ok()
        {
            return Ok(());
        }
    }
    Err(anyhow!(
        "the rolled Cloudflare setup token did not become active"
    ))
}

/// Rolls the secret of the setup token `token` and returns the new value.
/// `forte cloud rotate --setup-token-from-clipboard` runs the clipboard value
/// through this before handing the result to the broker, so the token the
/// broker stores is not the string that sat on the clipboard.
pub async fn roll_clipboard_setup_token(token: String) -> Result<String> {
    let client = reqwest::Client::new();
    let token_id = verify_setup_token(&client, API_BASE, &token).await?;
    roll_setup_token_value(&client, API_BASE, &token, &token_id).await
}

async fn authorize_control_user(
    client: &reqwest::Client,
    control_url: &str,
    control_token: &str,
    account_id: &str,
) -> Result<i64> {
    #[derive(Deserialize)]
    struct Authorization {
        t: String,
        #[serde(rename = "githubId")]
        github_id: Option<i64>,
    }
    let response = client
        .post(format!(
            "{}/__forte_action/cloudflare_broker_authorize",
            control_url.trim_end_matches('/')
        ))
        .bearer_auth(control_token)
        .header("x-forte-request-id", uuid::Uuid::new_v4().to_string())
        .json(&serde_json::json!({
            "operation": "resolve_zone",
            "account_id": account_id,
            "project_id": null,
        }))
        .send()
        .await?;
    let status = response.status();
    let text = response.text().await?;
    if !status.is_success() {
        return Err(anyhow!("fn0-control authorization failed ({status})"));
    }
    let authorization: Authorization = serde_json::from_str(&text)
        .with_context(|| "fn0-control authorization returned invalid JSON")?;
    if authorization.t != "Authorized" {
        return Err(anyhow!("fn0-control did not authorize this user"));
    }
    authorization
        .github_id
        .ok_or_else(|| anyhow!("fn0-control authorization did not return a user id"))
}

#[derive(Debug)]
struct TemporaryBootstrapToken {
    id: String,
    value: String,
}

async fn mint_bootstrap_token(
    client: &reqwest::Client,
    cloudflare_api_base: &str,
    setup_token: &str,
    account_id: &str,
) -> Result<TemporaryBootstrapToken> {
    #[derive(Deserialize)]
    struct PermissionGroup {
        id: Option<String>,
        name: Option<String>,
    }
    #[derive(Deserialize)]
    struct MintedToken {
        id: String,
        value: String,
    }
    let groups: Vec<PermissionGroup> = cloudflare_json(
        client,
        cloudflare_api_base,
        setup_token,
        reqwest::Method::GET,
        "/user/tokens/permission_groups",
        None,
    )
    .await?;
    let secret_store_id = groups
        .iter()
        .find(|group| {
            matches!(
                group.name.as_deref(),
                Some("Secrets Store Edit")
                    | Some("Account Secrets Store Edit")
                    | Some("Secrets Store Write")
            )
        })
        .and_then(|group| group.id.as_deref())
        .ok_or_else(|| anyhow!("Cloudflare did not expose the Secrets Store edit permission"))?;
    let workers_scripts_id = groups
        .iter()
        .find(|group| group.name.as_deref() == Some("Workers Scripts Write"))
        .and_then(|group| group.id.as_deref())
        .ok_or_else(|| anyhow!("Cloudflare did not expose the Workers Scripts Write permission"))?;
    let expires_on = (chrono::Utc::now() + chrono::Duration::minutes(BOOTSTRAP_TOKEN_MINUTES))
        .format("%Y-%m-%dT%H:%M:%SZ")
        .to_string();
    let minted: MintedToken = cloudflare_json(
        client,
        cloudflare_api_base,
        setup_token,
        reqwest::Method::POST,
        "/user/tokens",
        Some(serde_json::json!({
            "name": "fn0 broker bootstrap",
            "policies": [{
                "effect": "allow",
                "resources": { format!("com.cloudflare.api.account.{account_id}"): "*" },
                "permission_groups": [
                    { "id": secret_store_id },
                    { "id": workers_scripts_id },
                ],
            }],
            "expires_on": expires_on,
        })),
    )
    .await?;
    Ok(TemporaryBootstrapToken {
        id: minted.id,
        value: minted.value,
    })
}

async fn revoke_cloudflare_token(
    client: &reqwest::Client,
    cloudflare_api_base: &str,
    token: &str,
    token_id: &str,
) -> Result<()> {
    let path = format!("/user/tokens/{token_id}");
    let request = client
        .request(
            reqwest::Method::DELETE,
            format!("{cloudflare_api_base}{path}"),
        )
        .bearer_auth(token);
    let response = request.send().await?;
    let status = response.status();
    let text = response.text().await?;
    let envelope: CloudflareEnvelope<serde_json::Value> = serde_json::from_str(&text)
        .with_context(|| format!("Cloudflare DELETE {path} returned {status}: {text}"))?;
    if !status.is_success() || !envelope.success {
        return Err(anyhow!(
            "Cloudflare DELETE {path} failed ({status}): {}",
            describe_cloudflare_errors(&envelope.errors)
        ));
    }
    Ok(())
}

async fn ensure_secret_store(
    client: &reqwest::Client,
    cloudflare_api_base: &str,
    token: &str,
    account_id: &str,
) -> Result<String> {
    let stores: Vec<Store> = cloudflare_json(
        client,
        cloudflare_api_base,
        token,
        reqwest::Method::GET,
        &format!("/accounts/{account_id}/secrets_store/stores"),
        None,
    )
    .await?;
    if stores.len() > 1 {
        return Err(anyhow!(
            "Cloudflare account has more than one Secrets Store; select the fn0 store manually"
        ));
    }
    if let Some(store) = stores.first() {
        return Ok(store.id.clone());
    }
    let store: Store = cloudflare_json(
        client,
        cloudflare_api_base,
        token,
        reqwest::Method::POST,
        &format!("/accounts/{account_id}/secrets_store/stores"),
        Some(serde_json::json!({ "name": STORE_NAME })),
    )
    .await?;
    Ok(store.id)
}

async fn ensure_setup_secret(
    client: &reqwest::Client,
    cloudflare_api_base: &str,
    token: &str,
    account_id: &str,
    store_id: &str,
    setup_token: &str,
) -> Result<()> {
    let secrets: Vec<Secret> = cloudflare_json(
        client,
        cloudflare_api_base,
        token,
        reqwest::Method::GET,
        &format!("/accounts/{account_id}/secrets_store/stores/{store_id}/secrets"),
        None,
    )
    .await?;
    if let Some(secret) = secrets
        .iter()
        .find(|secret| secret.name == SETUP_SECRET_NAME)
    {
        // Cloudflare's secret-edit endpoint rejects `name` as an unknown
        // field: a secret's name is fixed at creation and only the create
        // (POST) body below takes one.
        cloudflare_json::<serde_json::Value>(
            client,
            cloudflare_api_base,
            token,
            reqwest::Method::PATCH,
            &format!(
                "/accounts/{account_id}/secrets_store/stores/{store_id}/secrets/{}",
                secret.id
            ),
            Some(serde_json::json!({
                "value": setup_token,
                "scopes": ["workers"],
                "comment": "Forte Cloudflare broker setup token",
            })),
        )
        .await?;
    } else {
        cloudflare_json::<serde_json::Value>(
            client,
            cloudflare_api_base,
            token,
            reqwest::Method::POST,
            &format!("/accounts/{account_id}/secrets_store/stores/{store_id}/secrets"),
            Some(serde_json::json!([{
                "name": SETUP_SECRET_NAME,
                "value": setup_token,
                "scopes": ["workers"],
                "comment": "Forte Cloudflare broker setup token",
            }])),
        )
        .await?;
    }
    Ok(())
}

async fn upload_worker(
    client: &reqwest::Client,
    cloudflare_api_base: &str,
    token: &str,
    account_id: &str,
    control_url: &str,
    store_id: &str,
    owner_github_id: i64,
) -> Result<()> {
    let metadata = serde_json::json!({
        "main_module": "index.mjs",
        "compatibility_date": WORKER_COMPATIBILITY_DATE,
        "bindings": [
            {
                "type": "secrets_store_secret",
                "name": "SETUP_TOKEN",
                "secret_name": SETUP_SECRET_NAME,
                "store_id": store_id,
            },
            {
                "type": "plain_text",
                "name": "CONTROL_URL",
                "text": control_url,
            },
            {
                "type": "plain_text",
                "name": "ACCOUNT_ID",
                "text": account_id,
            },
            {
                "type": "plain_text",
                "name": "STORE_ID",
                "text": store_id,
            },
            {
                "type": "plain_text",
                "name": "OWNER_GITHUB_ID",
                "text": owner_github_id.to_string(),
            },
        ],
    });
    let metadata_part = Part::text(metadata.to_string()).mime_str("application/json")?;
    // "+module", not plain "application/javascript": Cloudflare parses a part
    // without it as the old service-worker script format and chokes on this
    // file's `export` statements.
    let module_part = Part::text(include_str!("cloudflare_broker_worker.mjs").to_string())
        .file_name("index.mjs")
        .mime_str("application/javascript+module")?;
    let form = Form::new()
        .part("metadata", metadata_part)
        .part("index.mjs", module_part);
    let script_name = urlencoding::encode(BROKER_SCRIPT_NAME);
    let response = client
        .put(format!(
            "{cloudflare_api_base}/accounts/{account_id}/workers/scripts/{script_name}"
        ))
        .bearer_auth(token)
        .multipart(form)
        .send()
        .await?;
    let status = response.status();
    let text = response.text().await?;
    let envelope: CloudflareEnvelope<serde_json::Value> = serde_json::from_str(&text)
        .with_context(|| format!("Worker upload returned {status}: {text}"))?;
    if !status.is_success() || !envelope.success {
        return Err(anyhow!(
            "could not upload the fn0 broker Worker ({status}): {}",
            describe_cloudflare_errors(&envelope.errors)
        ));
    }
    // Uploading a script does not by itself put it on workers.dev — that
    // route is a separate, per-script opt-in, without which every request to
    // the *.workers.dev URL bounces at Cloudflare's edge before the Worker
    // ever runs.
    cloudflare_json::<serde_json::Value>(
        client,
        cloudflare_api_base,
        token,
        reqwest::Method::POST,
        &format!("/accounts/{account_id}/workers/scripts/{script_name}/subdomain"),
        Some(serde_json::json!({ "enabled": true })),
    )
    .await?;
    Ok(())
}

async fn ensure_workers_subdomain(
    client: &reqwest::Client,
    cloudflare_api_base: &str,
    token: &str,
    account_id: &str,
) -> Result<String> {
    let path = format!("/accounts/{account_id}/workers/subdomain");
    match cloudflare_json::<WorkerSubdomain>(
        client,
        cloudflare_api_base,
        token,
        reqwest::Method::GET,
        &path,
        None,
    )
    .await
    {
        Ok(subdomain) => Ok(subdomain.subdomain),
        Err(error) if error.to_string().contains("(404)") => {
            let suggested = format!("fn0-{}", &account_id[..8]);
            let subdomain: WorkerSubdomain = cloudflare_json(
                client,
                cloudflare_api_base,
                token,
                reqwest::Method::PUT,
                &path,
                Some(serde_json::json!({ "subdomain": suggested })),
            )
            .await?;
            Ok(subdomain.subdomain)
        }
        Err(error) => Err(error),
    }
}

/// Enabling the workers.dev route (in `upload_worker`) does not take effect
/// at Cloudflare's edge instantly: a request in the seconds right after
/// deploy can still bounce off the edge with error 1042 before the Worker
/// ever runs, which every other broker call would surface as an opaque
/// "broker request failed". Polling the broker's own URL here, once, right
/// after bootstrap, means every other call site can assume the broker is
/// already reachable.
async fn wait_for_broker_ready(
    client: &reqwest::Client,
    broker_url: &str,
    attempts: u32,
) -> Result<()> {
    for attempt in 0..attempts {
        if attempt > 0 {
            tokio::time::sleep(BROKER_READINESS_POLL).await;
        }
        let Ok(response) = client.get(broker_url).send().await else {
            continue;
        };
        let served_by_the_worker = response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .is_some_and(|value| value.starts_with("application/json"));
        if served_by_the_worker {
            return Ok(());
        }
    }
    if attempts == 0 {
        return Ok(());
    }
    Err(anyhow!(
        "the broker Worker at {broker_url} did not become reachable after deployment"
    ))
}

async fn cloudflare_json<T: for<'de> Deserialize<'de>>(
    client: &reqwest::Client,
    cloudflare_api_base: &str,
    token: &str,
    method: reqwest::Method,
    path: &str,
    body: Option<serde_json::Value>,
) -> Result<T> {
    let mut request = client
        .request(method.clone(), format!("{cloudflare_api_base}{path}"))
        .bearer_auth(token);
    if let Some(body) = body {
        request = request.json(&body);
    }
    let response = request.send().await?;
    let status = response.status();
    let text = response.text().await?;
    let envelope: CloudflareEnvelope<T> = serde_json::from_str(&text)
        .with_context(|| format!("Cloudflare {method} {path} returned {status}: {text}"))?;
    if !status.is_success() || !envelope.success {
        return Err(anyhow!(
            "Cloudflare {method} {path} failed ({status}): {}",
            describe_cloudflare_errors(&envelope.errors)
        ));
    }
    envelope
        .result
        .ok_or_else(|| anyhow!("Cloudflare {method} {path} returned success without a result"))
}

fn describe_cloudflare_errors(errors: &[CloudflareError]) -> String {
    if errors.is_empty() {
        return "no detail".to_string();
    }
    errors
        .iter()
        .map(|error| format!("{} ({})", error.message, error.code))
        .collect::<Vec<_>>()
        .join("; ")
}

fn normalize_https_url(url: &str, name: &str) -> Result<String> {
    let trimmed = url.trim().trim_end_matches('/');
    let parsed = reqwest::Url::parse(trimmed).map_err(|_| anyhow!("invalid {name}"))?;
    if parsed.scheme() != "https" || parsed.host_str().is_none() || parsed.query().is_some() {
        return Err(anyhow!("{name} must be an HTTPS origin without a query"));
    }
    Ok(trimmed.to_string())
}

fn validate_account_id(account_id: &str) -> Result<()> {
    if account_id.len() != 32 || !account_id.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(anyhow!("invalid Cloudflare account id"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{BrokerClient, authorize_control_user};
    use serde_json::json;
    use wiremock::matchers::{body_json, body_string_contains, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    const ACCOUNT_ID: &str = "0123456789abcdef0123456789abcdef";

    fn envelope(result: serde_json::Value) -> serde_json::Value {
        json!({ "success": true, "errors": [], "result": result })
    }

    fn error_envelope(message: &str) -> serde_json::Value {
        json!({ "success": false, "errors": [{"code": 1000, "message": message}], "result": null })
    }

    /// `reqwest`'s rustls backend needs a process-wide crypto provider before
    /// it will build a client at all, even one that only ever talks plain
    /// HTTP to a local mock server. Production binaries install this once in
    /// `main`; tests have no `main`, so each test installs it itself.
    fn install_crypto_provider() {
        let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
    }

    fn broker_client_for_test(base_url: String) -> BrokerClient {
        BrokerClient {
            client: reqwest::Client::new(),
            base_url,
            control_token: "control-token".to_string(),
            account_id: ACCOUNT_ID.to_string(),
        }
    }

    async fn mount_verify(cloudflare: &MockServer, status: &str) {
        Mock::given(method("GET"))
            .and(path("/user/tokens/verify"))
            .respond_with(ResponseTemplate::new(200).set_body_json(envelope(json!({
                "id": "existing-setup-token-id",
                "status": status,
            }))))
            .mount(cloudflare)
            .await;
    }

    async fn mount_control_authorized(control: &MockServer, github_id: i64) {
        Mock::given(method("POST"))
            .and(path("/__forte_action/cloudflare_broker_authorize"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "t": "Authorized",
                "githubId": github_id,
            })))
            .mount(control)
            .await;
    }

    async fn mount_bootstrap_token_mint(cloudflare: &MockServer, token_id: &str) {
        // "Secrets Store Write", not "Secrets Store Edit": confirmed against
        // a real Cloudflare API token's own /user/tokens/permission_groups.
        Mock::given(method("GET"))
            .and(path("/user/tokens/permission_groups"))
            .respond_with(ResponseTemplate::new(200).set_body_json(envelope(json!([
                {"id": "secret-store-group", "name": "Secrets Store Write"},
                {"id": "workers-scripts-group", "name": "Workers Scripts Write"},
            ]))))
            .mount(cloudflare)
            .await;
        Mock::given(method("POST"))
            .and(path("/user/tokens"))
            .respond_with(ResponseTemplate::new(200).set_body_json(envelope(json!({
                "id": token_id,
                "value": "temporary-bootstrap-token",
            }))))
            .mount(cloudflare)
            .await;
    }

    // `bootstrap` rolls the secret of the token the caller supplied (off the
    // clipboard, in the real flow) and works with the rolled value from then
    // on. `mount_verify` already returns the token's id as
    // `existing-setup-token-id` and answers the post-roll readiness check.
    async fn mount_setup_token_roll(cloudflare: &MockServer) {
        Mock::given(method("PUT"))
            .and(path("/user/tokens/existing-setup-token-id/value"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(envelope(json!("rolled-setup-token"))),
            )
            .expect(1)
            .mount(cloudflare)
            .await;
    }

    // Auth failure / a setup token already revoked outside the broker: caught
    // at the `verify` step, before any other Cloudflare or control call.
    #[tokio::test]
    async fn bootstrap_rejects_a_setup_token_that_is_no_longer_active() {
        install_crypto_provider();
        let cloudflare = MockServer::start().await;
        let control = MockServer::start().await;
        mount_verify(&cloudflare, "disabled").await;

        let result = BrokerClient::bootstrap_with_cloudflare_api_base(
            "revoked-setup-token".to_string(),
            ACCOUNT_ID.to_string(),
            control.uri(),
            "control-token".to_string(),
            &cloudflare.uri(),
            0,
        )
        .await;

        let error = result.unwrap_err().to_string();
        assert!(error.contains("disabled"), "unexpected error: {error}");
    }

    // Auth failure: when control refuses this user/account pair, the temporary
    // bootstrap token is never minted at all — nothing is left on Cloudflare's side.
    #[tokio::test]
    async fn bootstrap_rejects_when_control_denies_authorization() {
        install_crypto_provider();
        let cloudflare = MockServer::start().await;
        let control = MockServer::start().await;
        mount_verify(&cloudflare, "active").await;
        Mock::given(method("POST"))
            .and(path("/__forte_action/cloudflare_broker_authorize"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "t": "NotFound" })))
            .mount(&control)
            .await;
        Mock::given(method("POST"))
            .and(path("/user/tokens"))
            .respond_with(ResponseTemplate::new(200).set_body_json(envelope(json!({
                "id": "should-not-be-minted",
                "value": "should-not-be-minted",
            }))))
            .expect(0)
            .mount(&cloudflare)
            .await;

        let result = BrokerClient::bootstrap_with_cloudflare_api_base(
            "setup-token".to_string(),
            ACCOUNT_ID.to_string(),
            control.uri(),
            "control-token".to_string(),
            &cloudflare.uri(),
            0,
        )
        .await;

        assert!(result.is_err());
        cloudflare.verify().await;
    }

    // Partial failure: once the bootstrap token is minted, a failure preparing
    // the Secrets Store still gets that temporary token revoked.
    #[tokio::test]
    async fn bootstrap_revokes_the_temporary_token_after_a_mid_flow_failure() {
        install_crypto_provider();
        let cloudflare = MockServer::start().await;
        let control = MockServer::start().await;
        mount_verify(&cloudflare, "active").await;
        mount_control_authorized(&control, 42).await;
        mount_setup_token_roll(&cloudflare).await;
        mount_bootstrap_token_mint(&cloudflare, "bootstrap-token-id").await;
        Mock::given(method("GET"))
            .and(path(
                "/accounts/0123456789abcdef0123456789abcdef/secrets_store/stores",
            ))
            .respond_with(
                ResponseTemplate::new(500)
                    .set_body_json(error_envelope("internal error listing secrets stores")),
            )
            .mount(&cloudflare)
            .await;
        Mock::given(method("DELETE"))
            .and(path("/user/tokens/bootstrap-token-id"))
            .respond_with(ResponseTemplate::new(200).set_body_json(envelope(json!({}))))
            .expect(1)
            .mount(&cloudflare)
            .await;

        let result = BrokerClient::bootstrap_with_cloudflare_api_base(
            "setup-token".to_string(),
            ACCOUNT_ID.to_string(),
            control.uri(),
            "control-token".to_string(),
            &cloudflare.uri(),
            0,
        )
        .await;

        assert!(result.is_err());
        cloudflare.verify().await;
    }

    // External token revocation: if the temporary bootstrap token gets revoked
    // on Cloudflare's side mid-bootstrap so its cleanup DELETE fails, the work
    // that already finished must still count as success — a best-effort cleanup
    // failure must not surface as a bootstrap failure.
    #[tokio::test]
    async fn bootstrap_still_succeeds_when_cleanup_revoke_fails() {
        install_crypto_provider();
        let cloudflare = MockServer::start().await;
        let control = MockServer::start().await;
        mount_verify(&cloudflare, "active").await;
        mount_control_authorized(&control, 42).await;
        mount_setup_token_roll(&cloudflare).await;
        mount_bootstrap_token_mint(&cloudflare, "bootstrap-token-id").await;
        Mock::given(method("GET"))
            .and(path(
                "/accounts/0123456789abcdef0123456789abcdef/secrets_store/stores",
            ))
            .respond_with(ResponseTemplate::new(200).set_body_json(envelope(json!([
                {"id": "store-id"},
            ]))))
            .mount(&cloudflare)
            .await;
        Mock::given(method("GET"))
            .and(path(
                "/accounts/0123456789abcdef0123456789abcdef/secrets_store/stores/store-id/secrets",
            ))
            .respond_with(ResponseTemplate::new(200).set_body_json(envelope(json!([]))))
            .mount(&cloudflare)
            .await;
        Mock::given(method("POST"))
            .and(path(
                "/accounts/0123456789abcdef0123456789abcdef/secrets_store/stores/store-id/secrets",
            ))
            .respond_with(ResponseTemplate::new(200).set_body_json(envelope(json!([{}]))))
            .mount(&cloudflare)
            .await;
        Mock::given(method("PUT"))
            .and(path(
                "/accounts/0123456789abcdef0123456789abcdef/workers/scripts/fn0-broker",
            ))
            .respond_with(ResponseTemplate::new(200).set_body_json(envelope(json!({}))))
            .mount(&cloudflare)
            .await;
        Mock::given(method("POST"))
            .and(path(
                "/accounts/0123456789abcdef0123456789abcdef/workers/scripts/fn0-broker/subdomain",
            ))
            .respond_with(ResponseTemplate::new(200).set_body_json(envelope(json!({
                "enabled": true,
            }))))
            .mount(&cloudflare)
            .await;
        Mock::given(method("GET"))
            .and(path(
                "/accounts/0123456789abcdef0123456789abcdef/workers/subdomain",
            ))
            .respond_with(ResponseTemplate::new(200).set_body_json(envelope(json!({
                "subdomain": "fn0-01234567",
            }))))
            .mount(&cloudflare)
            .await;
        Mock::given(method("DELETE"))
            .and(path("/user/tokens/bootstrap-token-id"))
            .respond_with(
                ResponseTemplate::new(401)
                    .set_body_json(error_envelope("invalid or expired token")),
            )
            .expect(1)
            .mount(&cloudflare)
            .await;

        let result = BrokerClient::bootstrap_with_cloudflare_api_base(
            "setup-token".to_string(),
            ACCOUNT_ID.to_string(),
            control.uri(),
            "control-token".to_string(),
            &cloudflare.uri(),
            0,
        )
        .await;

        assert!(
            result.is_ok(),
            "bootstrap should not fail just because best-effort cleanup did: {result:?}"
        );
        cloudflare.verify().await;
    }

    // Cloudflare's secret-edit endpoint rejects a `name` field as unknown —
    // a secret's name is immutable after creation and only the create (POST)
    // body may carry one. Confirmed against a real account.
    #[tokio::test]
    async fn bootstrap_updates_an_existing_setup_secret_without_its_immutable_name() {
        install_crypto_provider();
        let cloudflare = MockServer::start().await;
        let control = MockServer::start().await;
        mount_verify(&cloudflare, "active").await;
        mount_control_authorized(&control, 42).await;
        mount_setup_token_roll(&cloudflare).await;
        mount_bootstrap_token_mint(&cloudflare, "bootstrap-token-id").await;
        Mock::given(method("GET"))
            .and(path(
                "/accounts/0123456789abcdef0123456789abcdef/secrets_store/stores",
            ))
            .respond_with(ResponseTemplate::new(200).set_body_json(envelope(json!([
                {"id": "store-id"},
            ]))))
            .mount(&cloudflare)
            .await;
        Mock::given(method("GET"))
            .and(path(
                "/accounts/0123456789abcdef0123456789abcdef/secrets_store/stores/store-id/secrets",
            ))
            .respond_with(ResponseTemplate::new(200).set_body_json(envelope(json!([
                {"id": "existing-secret-id", "name": "FN0_SETUP_TOKEN"},
            ]))))
            .mount(&cloudflare)
            .await;
        Mock::given(method("PATCH"))
            .and(path(
                "/accounts/0123456789abcdef0123456789abcdef/secrets_store/stores/store-id/secrets/existing-secret-id",
            ))
            .and(body_json(json!({
                "value": "rolled-setup-token",
                "scopes": ["workers"],
                "comment": "Forte Cloudflare broker setup token",
            })))
            .respond_with(ResponseTemplate::new(200).set_body_json(envelope(json!({}))))
            .expect(1)
            .mount(&cloudflare)
            .await;
        Mock::given(method("PUT"))
            .and(path(
                "/accounts/0123456789abcdef0123456789abcdef/workers/scripts/fn0-broker",
            ))
            .respond_with(ResponseTemplate::new(200).set_body_json(envelope(json!({}))))
            .mount(&cloudflare)
            .await;
        Mock::given(method("POST"))
            .and(path(
                "/accounts/0123456789abcdef0123456789abcdef/workers/scripts/fn0-broker/subdomain",
            ))
            .respond_with(ResponseTemplate::new(200).set_body_json(envelope(json!({
                "enabled": true,
            }))))
            .mount(&cloudflare)
            .await;
        Mock::given(method("GET"))
            .and(path(
                "/accounts/0123456789abcdef0123456789abcdef/workers/subdomain",
            ))
            .respond_with(ResponseTemplate::new(200).set_body_json(envelope(json!({
                "subdomain": "fn0-01234567",
            }))))
            .mount(&cloudflare)
            .await;
        Mock::given(method("DELETE"))
            .and(path("/user/tokens/bootstrap-token-id"))
            .respond_with(ResponseTemplate::new(200).set_body_json(envelope(json!({}))))
            .mount(&cloudflare)
            .await;

        let result = BrokerClient::bootstrap_with_cloudflare_api_base(
            "setup-token".to_string(),
            ACCOUNT_ID.to_string(),
            control.uri(),
            "control-token".to_string(),
            &cloudflare.uri(),
            0,
        )
        .await;

        assert!(result.is_ok(), "unexpected error: {result:?}");
        cloudflare.verify().await;
    }

    // The token the caller supplies comes off the clipboard, where a
    // clipboard-history tool may keep a copy. bootstrap rolls its secret in
    // place and stores the rolled value — the string that was on the clipboard
    // stops working.
    #[tokio::test]
    async fn bootstrap_rolls_the_supplied_setup_token() {
        install_crypto_provider();
        let cloudflare = MockServer::start().await;
        let control = MockServer::start().await;
        mount_verify(&cloudflare, "active").await;
        mount_control_authorized(&control, 42).await;
        mount_setup_token_roll(&cloudflare).await;
        mount_bootstrap_token_mint(&cloudflare, "bootstrap-token-id").await;
        Mock::given(method("GET"))
            .and(path(
                "/accounts/0123456789abcdef0123456789abcdef/secrets_store/stores",
            ))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(envelope(json!([{"id": "store-id"}]))),
            )
            .mount(&cloudflare)
            .await;
        Mock::given(method("GET"))
            .and(path(
                "/accounts/0123456789abcdef0123456789abcdef/secrets_store/stores/store-id/secrets",
            ))
            .respond_with(ResponseTemplate::new(200).set_body_json(envelope(json!([]))))
            .mount(&cloudflare)
            .await;
        Mock::given(method("POST"))
            .and(path(
                "/accounts/0123456789abcdef0123456789abcdef/secrets_store/stores/store-id/secrets",
            ))
            .and(body_string_contains("rolled-setup-token"))
            .respond_with(ResponseTemplate::new(200).set_body_json(envelope(json!([{}]))))
            .expect(1)
            .mount(&cloudflare)
            .await;
        Mock::given(method("PUT"))
            .and(path(
                "/accounts/0123456789abcdef0123456789abcdef/workers/scripts/fn0-broker",
            ))
            .respond_with(ResponseTemplate::new(200).set_body_json(envelope(json!({}))))
            .mount(&cloudflare)
            .await;
        Mock::given(method("POST"))
            .and(path(
                "/accounts/0123456789abcdef0123456789abcdef/workers/scripts/fn0-broker/subdomain",
            ))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(envelope(json!({"enabled": true}))),
            )
            .mount(&cloudflare)
            .await;
        Mock::given(method("GET"))
            .and(path(
                "/accounts/0123456789abcdef0123456789abcdef/workers/subdomain",
            ))
            .respond_with(ResponseTemplate::new(200).set_body_json(envelope(json!({
                "subdomain": "fn0-01234567",
            }))))
            .mount(&cloudflare)
            .await;
        Mock::given(method("DELETE"))
            .and(path("/user/tokens/bootstrap-token-id"))
            .respond_with(ResponseTemplate::new(200).set_body_json(envelope(json!({}))))
            .expect(1)
            .mount(&cloudflare)
            .await;

        let result = BrokerClient::bootstrap_with_cloudflare_api_base(
            "clipboard-setup-token".to_string(),
            ACCOUNT_ID.to_string(),
            control.uri(),
            "control-token".to_string(),
            &cloudflare.uri(),
            0,
        )
        .await;

        assert!(result.is_ok(), "unexpected error: {result:?}");
        cloudflare.verify().await;
    }

    // A failure rolling the setup token leaves nothing deployed — no bootstrap
    // token, no Worker — so it aborts rather than half-installing a broker.
    #[tokio::test]
    async fn bootstrap_fails_when_the_setup_token_cannot_be_rolled() {
        install_crypto_provider();
        let cloudflare = MockServer::start().await;
        let control = MockServer::start().await;
        mount_verify(&cloudflare, "active").await;
        mount_control_authorized(&control, 42).await;
        Mock::given(method("PUT"))
            .and(path("/user/tokens/existing-setup-token-id/value"))
            .respond_with(
                ResponseTemplate::new(403)
                    .set_body_json(error_envelope("insufficient permissions")),
            )
            .mount(&cloudflare)
            .await;
        Mock::given(method("POST"))
            .and(path("/user/tokens"))
            .respond_with(ResponseTemplate::new(200).set_body_json(envelope(json!({
                "id": "should-not-be-minted",
                "value": "should-not-be-minted",
            }))))
            .expect(0)
            .mount(&cloudflare)
            .await;

        let result = BrokerClient::bootstrap_with_cloudflare_api_base(
            "clipboard-setup-token".to_string(),
            ACCOUNT_ID.to_string(),
            control.uri(),
            "control-token".to_string(),
            &cloudflare.uri(),
            0,
        )
        .await;

        assert!(result.is_err());
        cloudflare.verify().await;
    }

    // Auth failure: a control response that is not `Authorized` refuses the request.
    #[tokio::test]
    async fn authorize_control_user_rejects_a_non_authorized_response() {
        install_crypto_provider();
        let control = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/__forte_action/cloudflare_broker_authorize"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "t": "NotLoggedIn" })))
            .mount(&control)
            .await;

        let result = authorize_control_user(
            &reqwest::Client::new(),
            &control.uri(),
            "control-token",
            ACCOUNT_ID,
        )
        .await;

        assert!(result.is_err());
    }

    // Rate limit / replay: when the broker itself returns 429 or 409, the
    // client propagates that as a failure rather than retrying silently.
    #[tokio::test]
    async fn resolve_zone_propagates_the_brokers_rate_limit_response() {
        install_crypto_provider();
        let broker = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/resolve-zone"))
            .respond_with(ResponseTemplate::new(429).set_body_json(json!({
                "error": "rate limit exceeded",
            })))
            .mount(&broker)
            .await;
        let client = broker_client_for_test(broker.uri());

        let error = client.resolve_zone("example.com").await.unwrap_err();
        assert!(
            error.to_string().contains("429"),
            "unexpected error: {error}"
        );
    }

    #[tokio::test]
    async fn resolve_zone_propagates_a_duplicate_request_rejection() {
        install_crypto_provider();
        let broker = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/resolve-zone"))
            .respond_with(ResponseTemplate::new(409).set_body_json(json!({
                "error": "duplicate request",
            })))
            .mount(&broker)
            .await;
        let client = broker_client_for_test(broker.uri());

        let error = client.resolve_zone("example.com").await.unwrap_err();
        assert!(
            error.to_string().contains("409"),
            "unexpected error: {error}"
        );
    }

    #[tokio::test]
    async fn teardown_project_sends_its_inputs_and_returns_the_brokers_notes() {
        install_crypto_provider();
        let broker = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/teardown-project"))
            .and(body_json(json!({
                "project_id": "abcd1234",
                "zone_id": "a1b2c3d4e5f6a1b2c3d4e5f6a1b2c3d4",
                "zone_name": "example.com",
                "app_hostname": "my-app.example.com",
                "delete_buckets": true,
            })))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "ok": true,
                "notes": ["left bucket fn0-abcd1234-frontend-asset: still not empty"],
            })))
            .mount(&broker)
            .await;
        let client = broker_client_for_test(broker.uri());

        let outcome = client
            .teardown_project(
                "abcd1234",
                "a1b2c3d4e5f6a1b2c3d4e5f6a1b2c3d4",
                "example.com",
                "my-app.example.com",
                true,
            )
            .await
            .unwrap();

        assert_eq!(outcome.notes.len(), 1);
    }

    #[tokio::test]
    async fn teardown_project_propagates_a_broker_failure() {
        install_crypto_provider();
        let broker = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/teardown-project"))
            .respond_with(ResponseTemplate::new(500).set_body_json(json!({ "error": "boom" })))
            .mount(&broker)
            .await;
        let client = broker_client_for_test(broker.uri());

        let error = client
            .teardown_project(
                "abcd1234",
                "a1b2c3d4e5f6a1b2c3d4e5f6a1b2c3d4",
                "example.com",
                "my-app.example.com",
                false,
            )
            .await
            .unwrap_err();

        assert!(
            error.to_string().contains("500"),
            "unexpected error: {error}"
        );
    }

    // Enabling the workers.dev route does not make it live at Cloudflare's
    // edge instantly (confirmed against a real account: the first request
    // right after deploy can still 404 there before the Worker ever runs),
    // so bootstrap polls the broker's own URL before declaring it ready.
    #[tokio::test]
    async fn wait_for_broker_ready_succeeds_once_the_worker_answers_with_json() {
        install_crypto_provider();
        let broker = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(
                ResponseTemplate::new(405)
                    .set_body_json(serde_json::json!({"error": "method not allowed"})),
            )
            .mount(&broker)
            .await;

        let result = super::wait_for_broker_ready(&reqwest::Client::new(), &broker.uri(), 1).await;

        assert!(result.is_ok(), "unexpected error: {result:?}");
    }

    #[tokio::test]
    async fn wait_for_broker_ready_gives_up_on_an_edge_error_that_never_clears() {
        install_crypto_provider();
        let broker = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(
                ResponseTemplate::new(404)
                    .set_body_string("error code: 1042")
                    .insert_header("content-type", "text/plain"),
            )
            .mount(&broker)
            .await;

        let result = super::wait_for_broker_ready(&reqwest::Client::new(), &broker.uri(), 2).await;

        assert!(result.is_err());
    }
}
