use reqwest::Client;
use std::time::Duration;
use tokio::time::sleep;

const IMDS_BASE_URL: &str = "http://169.254.169.254/opc/v2";
const AUTHORIZATION_HEADER: &str = "Bearer Oracle";
const MAX_ATTEMPTS: u32 = 3;
const REQUEST_TIMEOUT_SECS: u64 = 10;
const MAX_BACKOFF_SECS: u64 = 1;

/// Client for Instance Metadata Service (IMDS)
///
/// Provides access to instance metadata including region and identity certificates
pub struct ImdsClient {
    client: Client,
    base_url: String,
}

impl ImdsClient {
    /// Create a new IMDS client with default settings
    pub fn new() -> crate::core::Result<Self> {
        let client = Client::builder()
            .timeout(Duration::from_secs(REQUEST_TIMEOUT_SECS))
            .build()
            .map_err(|e| {
                crate::core::OciError::AuthError(format!("Failed to create HTTP client: {}", e))
            })?;

        Ok(Self {
            client,
            base_url: IMDS_BASE_URL.to_string(),
        })
    }

    /// Create IMDS client with custom base URL (for testing)
    #[cfg(test)]
    pub fn with_base_url(base_url: String) -> crate::core::Result<Self> {
        let client = Client::builder()
            .timeout(Duration::from_secs(REQUEST_TIMEOUT_SECS))
            .build()
            .map_err(|e| {
                crate::core::OciError::AuthError(format!("Failed to create HTTP client: {}", e))
            })?;

        Ok(Self { client, base_url })
    }

    /// Get the instance region
    pub async fn get_region(&self) -> crate::core::Result<String> {
        self.fetch_with_retry("/instance/region").await
    }

    /// Get the leaf certificate
    pub async fn get_leaf_certificate(&self) -> crate::core::Result<String> {
        self.fetch_with_retry("/identity/cert.pem").await
    }

    /// Get the leaf private key
    pub async fn get_leaf_private_key(&self) -> crate::core::Result<String> {
        self.fetch_with_retry("/identity/key.pem").await
    }

    /// Get intermediate certificates
    pub async fn get_intermediate_certificates(&self) -> crate::core::Result<Vec<String>> {
        let response = self.fetch_with_retry("/identity/intermediate.pem").await?;

        // Split PEM file into individual certificates
        let certs: Vec<String> = response
            .split("-----END CERTIFICATE-----")
            .filter_map(|cert| {
                let trimmed = cert.trim();
                if trimmed.is_empty() {
                    None
                } else {
                    Some(format!("{}-----END CERTIFICATE-----", trimmed))
                }
            })
            .collect();

        Ok(certs)
    }

    /// Fetch data from IMDS with retry logic
    async fn fetch_with_retry(&self, path: &str) -> crate::core::Result<String> {
        let url = format!("{}{}", self.base_url, path);
        let mut last_error = None;

        for attempt in 1..=MAX_ATTEMPTS {
            match self.fetch_once(&url).await {
                Ok(data) => return Ok(data),
                Err(e) => {
                    last_error = Some(e);

                    // Don't sleep after the last attempt
                    if attempt < MAX_ATTEMPTS {
                        let backoff = calculate_backoff_with_jitter(attempt, MAX_BACKOFF_SECS);
                        sleep(backoff).await;
                    }
                }
            }
        }

        Err(last_error.unwrap_or_else(|| {
            crate::core::OciError::AuthError("IMDS fetch failed with no error".to_string())
        }))
    }

    /// Perform a single IMDS fetch
    async fn fetch_once(&self, url: &str) -> crate::core::Result<String> {
        let response = self
            .client
            .get(url)
            .header("Authorization", AUTHORIZATION_HEADER)
            .send()
            .await
            .map_err(|e| {
                crate::core::OciError::AuthError(format!(
                    "IMDS request failed for {}: {}",
                    url, e
                ))
            })?;

        if !response.status().is_success() {
            return Err(crate::core::OciError::AuthError(format!(
                "IMDS returned non-success status {} for {}",
                response.status(),
                url
            )));
        }

        response.text().await.map_err(|e| {
            crate::core::OciError::AuthError(format!("Failed to read IMDS response: {}", e))
        })
    }
}

/// Calculate exponential backoff with jitter
fn calculate_backoff_with_jitter(attempt: u32, max_secs: u64) -> Duration {
    use rand::Rng;

    // Exponential backoff: 2^(attempt-1) * 100ms, capped at max_secs
    let base_ms = 100u64 * 2u64.pow(attempt - 1);
    let max_ms = max_secs * 1000;
    let backoff_ms = base_ms.min(max_ms);

    // Add jitter: random value between 0 and backoff_ms
    let mut rng = rand::thread_rng();
    let jitter_ms = rng.gen_range(0..=backoff_ms);

    Duration::from_millis(jitter_ms)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_imds_fetch_region() {
        let mut server = mockito::Server::new_async().await;
        let _m = server
            .mock("GET", "/opc/v2/instance/region")
            .match_header("Authorization", "Bearer Oracle")
            .with_status(200)
            .with_body("ap-seoul-1")
            .create_async()
            .await;

        let client = ImdsClient::with_base_url(format!("{}/opc/v2", server.url())).unwrap();
        let region = client.get_region().await.unwrap();
        assert_eq!(region, "ap-seoul-1");
    }

    #[tokio::test]
    async fn test_imds_fetch_certificate() {
        let mut server = mockito::Server::new_async().await;
        let cert_pem = r#"-----BEGIN CERTIFICATE-----
MIICxTCCAa2gAwIBAgIUQw4YgqVmKGjGPXHMRBZf8hNOiHgwDQYJKoZIhvcNAQEL
-----END CERTIFICATE-----"#;

        let _m = server
            .mock("GET", "/opc/v2/identity/cert.pem")
            .match_header("Authorization", "Bearer Oracle")
            .with_status(200)
            .with_body(cert_pem)
            .create_async()
            .await;

        let client = ImdsClient::with_base_url(format!("{}/opc/v2", server.url())).unwrap();
        let cert = client.get_leaf_certificate().await.unwrap();
        assert!(cert.contains("BEGIN CERTIFICATE"));
    }

    #[tokio::test]
    async fn test_imds_max_retries_exceeded() {
        let mut server = mockito::Server::new_async().await;
        let _m = server
            .mock("GET", "/opc/v2/instance/region")
            .match_header("Authorization", "Bearer Oracle")
            .with_status(500)
            .expect(3) // All attempts fail
            .create_async()
            .await;

        let client = ImdsClient::with_base_url(format!("{}/opc/v2", server.url())).unwrap();
        let result = client.get_region().await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_split_intermediate_certificates() {
        let mut server = mockito::Server::new_async().await;
        let certs_pem = r#"-----BEGIN CERTIFICATE-----
CERT1
-----END CERTIFICATE-----
-----BEGIN CERTIFICATE-----
CERT2
-----END CERTIFICATE-----"#;

        let _m = server
            .mock("GET", "/opc/v2/identity/intermediate.pem")
            .match_header("Authorization", "Bearer Oracle")
            .with_status(200)
            .with_body(certs_pem)
            .create_async()
            .await;

        let client = ImdsClient::with_base_url(format!("{}/opc/v2", server.url())).unwrap();
        let certs = client.get_intermediate_certificates().await.unwrap();
        assert_eq!(certs.len(), 2);
    }

    #[test]
    fn test_backoff_calculation() {
        let backoff1 = calculate_backoff_with_jitter(1, 1);
        assert!(backoff1.as_millis() <= 100); // First attempt: 0-100ms

        let backoff2 = calculate_backoff_with_jitter(2, 1);
        assert!(backoff2.as_millis() <= 200); // Second attempt: 0-200ms

        let backoff3 = calculate_backoff_with_jitter(3, 1);
        assert!(backoff3.as_millis() <= 400); // Third attempt: 0-400ms

        let backoff4 = calculate_backoff_with_jitter(10, 1);
        assert!(backoff4.as_millis() <= 1000); // Capped at max_secs
    }
}
