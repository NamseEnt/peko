use forte_sdk::*;
use serde::{Deserialize, Serialize};

const CLOUDFLARE_API_BASE: &str = "https://api.cloudflare.com/client/v4";

/// One entry of Cloudflare's by-URL purge, which accepts either a bare URL or
/// an object whose header values select which per-`Origin` cache entry to
/// clear. A bare URL clears the entry for a request that carried no `Origin`.
#[derive(Serialize, Deserialize, Clone)]
#[serde(untagged)]
pub enum CachePurgeFile {
    Url(String),
    Request(CachePurgeRequest),
}

#[derive(Serialize, Deserialize, Clone)]
pub struct CachePurgeRequest {
    pub url: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub headers: Option<CachePurgeHeaders>,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct CachePurgeHeaders {
    #[serde(rename = "Origin")]
    pub origin: String,
}

impl CachePurgeFile {
    pub fn from_url(url: String, origin: Option<&str>) -> Self {
        Self::Request(CachePurgeRequest {
            url,
            headers: origin.map(|origin| CachePurgeHeaders {
                origin: origin.to_string(),
            }),
        })
    }
}

pub struct CloudflareClient {
    api_token: String,
    account_id: String,
    zone_id: Option<String>,
}

impl CloudflareClient {
    /// A client bound to one user's zone, driven by their purge-scoped token.
    /// It can purge and read zone metadata; anything else answers 403.
    pub fn for_zone(api_token: String, account_id: String, zone_id: String) -> Self {
        Self {
            api_token,
            account_id,
            zone_id: Some(zone_id),
        }
    }

    pub fn account_id(&self) -> &str {
        &self.account_id
    }

    async fn call(
        &self,
        method: &str,
        path: &str,
        body: Vec<u8>,
    ) -> anyhow::Result<(u16, Vec<u8>)> {
        let url = format!("{CLOUDFLARE_API_BASE}{path}");
        let req = http::Request::builder()
            .uri(url)
            .method(method)
            .header("Authorization", format!("Bearer {}", self.api_token))
            .header("Content-Type", "application/json")
            .body(body)?;
        let resp = http::Client::new().send(req).await?;
        let status = resp.status().as_u16();
        let body = resp.into_body().bytes().await?.to_vec();
        Ok((status, body))
    }

    /// Platform account only. A user's account is provisioned by the CLI with
    /// a token fn0 never holds.
    pub async fn create_r2_bucket(
        &self,
        bucket_name: &str,
        location_hint: Option<&str>,
    ) -> anyhow::Result<()> {
        #[derive(Serialize)]
        struct Body<'a> {
            name: &'a str,
            #[serde(rename = "locationHint", skip_serializing_if = "Option::is_none")]
            location_hint: Option<&'a str>,
        }
        let payload = serde_json::to_vec(&Body {
            name: bucket_name,
            location_hint,
        })?;
        let path = format!("/accounts/{}/r2/buckets", self.account_id);
        let (status, body) = self.call("POST", &path, payload).await?;
        if (200..300).contains(&status) || response_indicates_already_exists(&body) {
            return Ok(());
        }
        anyhow::bail!(
            "create_r2_bucket {bucket_name} failed (status={status}): {}",
            String::from_utf8_lossy(&body)
        );
    }

    /// Platform account only.
    pub async fn put_r2_bucket_cors(
        &self,
        bucket_name: &str,
        methods: &[&str],
        allow_origin: &str,
        expose_headers: &[&str],
    ) -> anyhow::Result<()> {
        #[derive(Serialize)]
        struct Body<'a> {
            rules: Vec<Rule<'a>>,
        }
        #[derive(Serialize)]
        struct Rule<'a> {
            allowed: Allowed<'a>,
            #[serde(rename = "exposeHeaders")]
            expose_headers: Vec<&'a str>,
            #[serde(rename = "maxAgeSeconds")]
            max_age_seconds: u32,
        }
        #[derive(Serialize)]
        struct Allowed<'a> {
            methods: Vec<&'a str>,
            origins: Vec<&'a str>,
            headers: Vec<&'a str>,
        }
        let payload = serde_json::to_vec(&Body {
            rules: vec![Rule {
                allowed: Allowed {
                    methods: methods.to_vec(),
                    origins: vec![allow_origin],
                    headers: vec!["*"],
                },
                expose_headers: expose_headers.to_vec(),
                max_age_seconds: 86400,
            }],
        })?;
        let path = format!(
            "/accounts/{}/r2/buckets/{}/cors",
            self.account_id, bucket_name
        );
        let (status, body) = self.call("PUT", &path, payload).await?;
        if (200..300).contains(&status) {
            return Ok(());
        }
        anyhow::bail!(
            "put_r2_bucket_cors {bucket_name} failed (status={status}): {}",
            String::from_utf8_lossy(&body)
        );
    }

    /// Platform account only.
    pub async fn delete_r2_bucket(&self, bucket_name: &str) -> anyhow::Result<()> {
        let path = format!("/accounts/{}/r2/buckets/{}", self.account_id, bucket_name);
        let (status, body) = self.call("DELETE", &path, Vec::new()).await?;
        if (200..300).contains(&status) || status == 404 {
            return Ok(());
        }
        anyhow::bail!(
            "delete_r2_bucket {bucket_name} failed (status={status}): {}",
            String::from_utf8_lossy(&body)
        );
    }

    pub async fn r2_bucket_exists(&self, bucket_name: &str) -> anyhow::Result<bool> {
        let path = format!("/accounts/{}/r2/buckets/{}", self.account_id, bucket_name);
        let (status, body) = self.call("GET", &path, Vec::new()).await?;
        if (200..300).contains(&status) {
            return Ok(true);
        }
        if status == 404 {
            return Ok(false);
        }
        anyhow::bail!(
            "r2_bucket_exists {bucket_name} failed (status={status}): {}",
            String::from_utf8_lossy(&body)
        );
    }

    pub async fn list_r2_bucket_names(&self, name_contains: &str) -> anyhow::Result<Vec<String>> {
        #[derive(Deserialize)]
        struct Envelope {
            result: Option<ListResult>,
            #[serde(default)]
            result_info: Option<ListResultInfo>,
        }
        #[derive(Deserialize)]
        struct ListResult {
            buckets: Vec<Bucket>,
        }
        #[derive(Deserialize)]
        struct Bucket {
            name: String,
        }
        #[derive(Deserialize)]
        struct ListResultInfo {
            cursor: Option<String>,
        }

        const PER_PAGE: usize = 1000;
        let mut names = Vec::new();
        let mut cursor: Option<String> = None;
        loop {
            let mut path = format!(
                "/accounts/{}/r2/buckets?name_contains={name_contains}&per_page={PER_PAGE}",
                self.account_id
            );
            if let Some(cursor) = &cursor {
                path.push_str(&format!("&cursor={cursor}"));
            }
            let (status, body) = self.call("GET", &path, Vec::new()).await?;
            if !(200..300).contains(&status) {
                anyhow::bail!(
                    "list_r2_bucket_names failed (status={status}): {}",
                    String::from_utf8_lossy(&body)
                );
            }
            let envelope: Envelope = serde_json::from_slice(&body)?;
            let buckets = envelope
                .result
                .ok_or_else(|| anyhow::anyhow!("list_r2_bucket_names: missing result"))?
                .buckets;
            let page_len = buckets.len();
            names.extend(buckets.into_iter().map(|b| b.name));
            if page_len < PER_PAGE {
                break;
            }
            match envelope.result_info.and_then(|i| i.cursor) {
                Some(next) => cursor = Some(next),
                None => break,
            }
        }
        Ok(names)
    }

    pub async fn purge_cache_tags(&self, tags: &[&str]) -> anyhow::Result<()> {
        #[derive(Serialize)]
        struct Body<'a> {
            tags: &'a [&'a str],
        }
        let payload = serde_json::to_vec(&Body { tags })?;
        let path = format!("/zones/{}/purge_cache", self.zone_id()?);
        let (status, body) = self.call("POST", &path, payload).await?;
        if (200..300).contains(&status) {
            return Ok(());
        }
        anyhow::bail!(
            "purge_cache_tags failed (status={status}): {}",
            String::from_utf8_lossy(&body)
        );
    }

    /// Purges by exact URL. Cloudflare accepts 100 per request against a far
    /// larger budget than the tag path (800/s vs 5/min account-wide), so this is
    /// the route for per-object invalidation.
    ///
    /// Requires the zone's Cache Rule to match `PURGE`; without it the API still
    /// answers `success: true` while the edge keeps serving the old object.
    pub async fn purge_cache_urls(&self, files: &[CachePurgeFile]) -> anyhow::Result<()> {
        #[derive(Serialize)]
        struct Body<'a> {
            files: &'a [CachePurgeFile],
        }
        let payload = serde_json::to_vec(&Body { files })?;
        let path = format!("/zones/{}/purge_cache", self.zone_id()?);
        let (status, body) = self.call("POST", &path, payload).await?;
        if (200..300).contains(&status) {
            return Ok(());
        }
        anyhow::bail!(
            "purge_cache_urls failed (status={status}): {}",
            String::from_utf8_lossy(&body)
        );
    }
}

/// The Cloudflare API token id, which doubles as the R2 S3 access key id. The
/// matching secret is the SHA-256 of the token value, derived by the caller.
pub struct VerifiedToken {
    pub id: String,
}

impl CloudflareClient {
    /// Cloudflare has two kinds of API token and each verifies at its own
    /// endpoint: one made under My Profile is user-owned and only
    /// `/user/tokens/verify` accepts it, while one made under the account's own
    /// API Tokens page answers at `/accounts/{id}/tokens/verify`. Asking the
    /// wrong one returns `Invalid API Token`, which reads as a bad credential
    /// rather than the wrong URL. Onboarding tells people to use My Profile, so
    /// that is tried first.
    pub async fn verify_token(&self) -> anyhow::Result<VerifiedToken> {
        match self.verify_token_at("/user/tokens/verify").await {
            Ok(token) => Ok(token),
            Err(user_error) => {
                let account_path = format!("/accounts/{}/tokens/verify", self.account_id);
                self.verify_token_at(&account_path).await.map_err(|_| {
                    // The user-token error is the one to surface: it is the
                    // path onboarding documents.
                    user_error
                })
            }
        }
    }

    async fn verify_token_at(&self, path: &str) -> anyhow::Result<VerifiedToken> {
        #[derive(Deserialize)]
        struct Envelope {
            result: Option<TokenResult>,
        }
        #[derive(Deserialize)]
        struct TokenResult {
            id: String,
            status: String,
        }
        let (status, body) = self.call("GET", path, Vec::new()).await?;
        if !(200..300).contains(&status) {
            anyhow::bail!(
                "verify_token failed (status={status}): {}",
                String::from_utf8_lossy(&body)
            );
        }
        let result = serde_json::from_slice::<Envelope>(&body)?
            .result
            .ok_or_else(|| anyhow::anyhow!("verify_token: missing result"))?;
        if result.status != "active" {
            anyhow::bail!("token status is {}, expected active", result.status);
        }
        Ok(VerifiedToken { id: result.id })
    }

    pub async fn zone_name(&self) -> anyhow::Result<String> {
        #[derive(Deserialize)]
        struct Envelope {
            result: Option<ZoneResult>,
        }
        #[derive(Deserialize)]
        struct ZoneResult {
            name: String,
        }
        let path = format!("/zones/{}", self.zone_id()?);
        let (status, body) = self.call("GET", &path, Vec::new()).await?;
        if !(200..300).contains(&status) {
            anyhow::bail!(
                "zone_name failed (status={status}): {}",
                String::from_utf8_lossy(&body)
            );
        }
        Ok(serde_json::from_slice::<Envelope>(&body)?
            .result
            .ok_or_else(|| anyhow::anyhow!("zone_name: missing result"))?
            .name)
    }

    fn zone_id(&self) -> anyhow::Result<&str> {
        self.zone_id
            .as_deref()
            .ok_or_else(|| anyhow::anyhow!("no Cloudflare zone id for this client"))
    }
}

#[derive(Deserialize)]
struct CloudflareEnvelope {
    #[serde(default)]
    errors: Vec<CloudflareError>,
}

#[derive(Deserialize)]
struct CloudflareError {
    #[serde(default)]
    message: String,
}

fn response_indicates_already_exists(body: &[u8]) -> bool {
    let Ok(env) = serde_json::from_slice::<CloudflareEnvelope>(body) else {
        return false;
    };
    env.errors.iter().any(|e| {
        let m = e.message.to_lowercase();
        m.contains("already exists")
            || m.contains("already configured")
            || m.contains("already in use")
            || m.contains("duplicate")
    })
}
