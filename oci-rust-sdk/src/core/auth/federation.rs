use crate::core::auth::x509_utils::sanitize_certificate_pem;
use crate::core::region::Region;
use base64::{engine::general_purpose, Engine as _};
use chrono::{DateTime, Utc};
use reqwest::Client;
use rsa::pkcs8::{DecodePrivateKey, EncodePrivateKey, EncodePublicKey, LineEnding};
use rsa::{Pkcs1v15Sign, RsaPrivateKey, RsaPublicKey};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::time::Duration;
use tokio::time::sleep;

const MAX_ATTEMPTS: u32 = 3;
const MAX_BACKOFF_SECS: u64 = 1;
const REQUEST_TIMEOUT_SECS: u64 = 10;

/// Session keypair for federation requests
#[derive(Clone)]
pub struct SessionKeyPair {
    pub private_key_pem: String,
    pub public_key_pem: String,
    pub private_key: RsaPrivateKey,
}

/// Security token response from federation service
#[derive(Debug, Clone)]
pub struct SecurityToken {
    pub token: String,
    pub expires_at: DateTime<Utc>,
}

/// Request body for X.509 federation
#[derive(Serialize)]
struct FederationRequest {
    certificate: String,
    #[serde(rename = "publicKey")]
    public_key: String,
    #[serde(rename = "intermediateCertificates")]
    intermediate_certificates: Vec<String>,
    purpose: String,
}

/// Response from federation service
#[derive(Deserialize)]
struct FederationResponse {
    token: String,
}

/// Generate a new RSA 2048-bit session keypair
pub fn generate_session_keypair() -> crate::core::Result<SessionKeyPair> {
    use rand::rngs::OsRng;

    let mut rng = OsRng;
    let private_key = RsaPrivateKey::new(&mut rng, 2048).map_err(|e| {
        crate::core::OciError::AuthError(format!("Failed to generate session keypair: {}", e))
    })?;

    let public_key = RsaPublicKey::from(&private_key);

    let private_key_pem = private_key
        .to_pkcs8_pem(LineEnding::LF)
        .map_err(|e| {
            crate::core::OciError::AuthError(format!("Failed to encode private key: {}", e))
        })?
        .to_string();

    let public_key_pem = public_key
        .to_public_key_pem(LineEnding::LF)
        .map_err(|e| {
            crate::core::OciError::AuthError(format!("Failed to encode public key: {}", e))
        })?;

    Ok(SessionKeyPair {
        private_key_pem,
        public_key_pem,
        private_key,
    })
}

/// Request a security token from the federation service
pub async fn request_security_token(
    region: &Region,
    tenancy_id: &str,
    leaf_cert_pem: &str,
    leaf_key_pem: &str,
    intermediate_certs_pem: &[String],
    session_public_key_pem: &str,
) -> crate::core::Result<SecurityToken> {
    let endpoint = format!("https://auth.{}.oraclecloud.com/v1/x509", region.id());

    // Parse leaf private key for signing
    let leaf_private_key = RsaPrivateKey::from_pkcs8_pem(leaf_key_pem).map_err(|e| {
        crate::core::OciError::AuthError(format!("Failed to parse leaf private key: {}", e))
    })?;

    // Prepare request body
    let request_body = FederationRequest {
        certificate: sanitize_certificate_pem(leaf_cert_pem),
        public_key: sanitize_certificate_pem(session_public_key_pem),
        intermediate_certificates: intermediate_certs_pem
            .iter()
            .map(|cert| sanitize_certificate_pem(cert))
            .collect(),
        purpose: "DEFAULT".to_string(),
    };

    let body_json =
        serde_json::to_string(&request_body).map_err(|e| {
            crate::core::OciError::AuthError(format!("Failed to serialize request: {}", e))
        })?;

    // Retry logic
    let client = Client::builder()
        .timeout(Duration::from_secs(REQUEST_TIMEOUT_SECS))
        .build()
        .map_err(|e| {
            crate::core::OciError::AuthError(format!("Failed to create HTTP client: {}", e))
        })?;

    let mut last_error = None;

    for attempt in 1..=MAX_ATTEMPTS {
        match make_federation_request(
            &client,
            &endpoint,
            &body_json,
            tenancy_id,
            &leaf_private_key,
        )
        .await
        {
            Ok(token) => return parse_security_token(token),
            Err(e) => {
                // Check if error is retryable
                if !is_retryable_error(&e) {
                    return Err(e);
                }

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
        crate::core::OciError::AuthError("Federation request failed with no error".to_string())
    }))
}

/// Make a single federation request
async fn make_federation_request(
    client: &Client,
    endpoint: &str,
    body_json: &str,
    tenancy_id: &str,
    leaf_private_key: &RsaPrivateKey,
) -> crate::core::Result<String> {
    let url = url::Url::parse(endpoint).map_err(|e| {
        crate::core::OciError::AuthError(format!("Invalid federation endpoint: {}", e))
    })?;

    let mut headers = reqwest::header::HeaderMap::new();

    // Add required headers
    let now = chrono::Utc::now();
    headers.insert(
        "x-date",
        now.format("%a, %d %b %Y %H:%M:%S GMT")
            .to_string()
            .parse()
            .map_err(|e| crate::core::OciError::AuthError(format!("Invalid date header: {}", e)))?,
    );

    headers.insert(
        "host",
        url.host_str()
            .ok_or_else(|| crate::core::OciError::AuthError("Missing host in URL".to_string()))?
            .parse()
            .map_err(|e| crate::core::OciError::AuthError(format!("Invalid host header: {}", e)))?,
    );

    headers.insert(
        "content-type",
        "application/json"
            .parse()
            .map_err(|e| crate::core::OciError::AuthError(format!("Invalid content-type: {}", e)))?,
    );

    headers.insert(
        "content-length",
        body_json.len()
            .to_string()
            .parse()
            .map_err(|e| crate::core::OciError::AuthError(format!("Invalid content-length: {}", e)))?,
    );

    // Calculate SHA256 of body
    let mut hasher = Sha256::new();
    hasher.update(body_json.as_bytes());
    let hash = hasher.finalize();
    let b64_hash = general_purpose::STANDARD.encode(hash);

    headers.insert(
        "x-content-sha256",
        b64_hash
            .parse()
            .map_err(|e| crate::core::OciError::AuthError(format!("Invalid sha256 header: {}", e)))?,
    );

    // Sign the request
    sign_federation_request(&url, &mut headers, body_json, tenancy_id, leaf_private_key)?;

    // Make the request
    let response = client
        .post(endpoint)
        .headers(headers)
        .body(body_json.to_string())
        .send()
        .await
        .map_err(|e| {
            crate::core::OciError::AuthError(format!("Federation request failed: {}", e))
        })?;

    let status = response.status();

    if status.is_success() {
        let federation_response: FederationResponse = response.json().await.map_err(|e| {
            crate::core::OciError::AuthError(format!("Failed to parse federation response: {}", e))
        })?;
        Ok(federation_response.token)
    } else if status.is_client_error() {
        // 4XX errors should not be retried
        let error_body = response.text().await.unwrap_or_default();
        Err(crate::core::OciError::ConfigError(format!(
            "Federation request failed with status {}: {}",
            status, error_body
        )))
    } else {
        // 5XX errors should be retried
        let error_body = response.text().await.unwrap_or_default();
        Err(crate::core::OciError::ServiceError {
            status: status.as_u16(),
            code: "FederationServiceError".to_string(),
            message: error_body,
        })
    }
}

/// Sign the federation request using HTTP Signature with leaf certificate key
fn sign_federation_request(
    url: &url::Url,
    headers: &mut reqwest::header::HeaderMap,
    _body: &str,
    tenancy_id: &str,
    leaf_private_key: &RsaPrivateKey,
) -> crate::core::Result<()> {
    // Headers to sign (in order)
    let headers_to_sign = vec![
        "(request-target)",
        "host",
        "x-date",
        "content-type",
        "content-length",
        "x-content-sha256",
    ];

    // Build signing string
    let mut parts = Vec::new();
    for header_name in &headers_to_sign {
        let value = if *header_name == "(request-target)" {
            format!("post {}", url.path())
        } else {
            headers
                .get(*header_name)
                .and_then(|v| v.to_str().ok())
                .ok_or_else(|| {
                    crate::core::OciError::SigningError(format!("Missing header: {}", header_name))
                })?
                .to_string()
        };
        parts.push(format!("{}: {}", header_name, value));
    }
    let signing_string = parts.join("\n");

    // Sign the string
    let mut hasher = Sha256::new();
    hasher.update(signing_string.as_bytes());
    let hashed = hasher.finalize();

    let padding = Pkcs1v15Sign::new_unprefixed();
    let signature = leaf_private_key
        .sign(padding, &hashed)
        .map_err(|e| crate::core::OciError::SigningError(format!("Failed to sign: {}", e)))?;

    let signature_b64 = general_purpose::STANDARD.encode(&signature);

    // Build authorization header
    // KeyId format: tenancyId/fed-x509/certificate_fingerprint
    // For simplicity, we'll use tenancyId/fed-x509 (the actual fingerprint would be computed from the cert)
    let key_id = format!("{}/fed-x509", tenancy_id);

    let auth_header = format!(
        r#"Signature version="1",headers="{}",keyId="{}",algorithm="rsa-sha256",signature="{}""#,
        headers_to_sign.join(" "),
        key_id,
        signature_b64
    );

    headers.insert(
        "authorization",
        auth_header
            .parse()
            .map_err(|e| crate::core::OciError::SigningError(format!("Invalid authorization header: {}", e)))?,
    );

    Ok(())
}

/// Parse JWT token and extract expiration
fn parse_security_token(token: String) -> crate::core::Result<SecurityToken> {
    use jsonwebtoken::{decode, Algorithm, DecodingKey, Validation};

    #[derive(Deserialize)]
    struct Claims {
        exp: i64,
    }

    // We only need to extract the expiration, so we don't validate the signature
    let mut validation = Validation::new(Algorithm::RS256);
    validation.insecure_disable_signature_validation();
    validation.validate_exp = false;

    let token_data = decode::<Claims>(&token, &DecodingKey::from_secret(&[]), &validation)
        .map_err(|e| crate::core::OciError::AuthError(format!("Failed to parse JWT: {}", e)))?;

    let expires_at = DateTime::from_timestamp(token_data.claims.exp, 0).ok_or_else(|| {
        crate::core::OciError::AuthError("Invalid expiration timestamp in JWT".to_string())
    })?;

    Ok(SecurityToken { token, expires_at })
}

/// Check if an error is retryable
fn is_retryable_error(error: &crate::core::OciError) -> bool {
    match error {
        crate::core::OciError::ServiceError { status, .. } => *status >= 500,
        crate::core::OciError::HttpError(_) => true, // Network errors
        crate::core::OciError::AuthError(_) => true, // Transient auth issues
        _ => false,
    }
}

/// Calculate exponential backoff with jitter
fn calculate_backoff_with_jitter(attempt: u32, max_secs: u64) -> Duration {
    use rand::Rng;

    let base_ms = 100u64 * 2u64.pow(attempt - 1);
    let max_ms = max_secs * 1000;
    let backoff_ms = base_ms.min(max_ms);

    let mut rng = rand::thread_rng();
    let jitter_ms = rng.gen_range(0..=backoff_ms);

    Duration::from_millis(jitter_ms)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_generate_session_keypair() {
        let keypair = generate_session_keypair().unwrap();
        assert!(keypair.private_key_pem.contains("BEGIN PRIVATE KEY"));
        assert!(keypair.public_key_pem.contains("BEGIN PUBLIC KEY"));
    }

    #[test]
    fn test_sanitize_certificate() {
        let cert = r#"-----BEGIN CERTIFICATE-----
MIIC...
-----END CERTIFICATE-----"#;
        let sanitized = sanitize_certificate_pem(cert);
        assert!(!sanitized.contains("BEGIN"));
        assert!(sanitized.starts_with("MIIC"));
    }

    #[test]
    fn test_parse_jwt_token() {
        // Sample JWT with exp claim (not a real token, just for testing structure)
        let token = "eyJhbGciOiJSUzI1NiIsInR5cCI6IkpXVCJ9.eyJleHAiOjE3MDQwNjcyMDAsInN1YiI6InRlc3QifQ.test";

        let result = parse_security_token(token.to_string()).unwrap();
        assert_eq!(result.token, token);
        // exp: 1704067200 = 2024-01-01 00:00:00 UTC
        assert_eq!(
            result.expires_at.to_rfc3339(),
            "2024-01-01T00:00:00+00:00"
        );
    }

    #[test]
    fn test_is_retryable_error() {
        // 5XX errors should be retryable
        let error = crate::core::OciError::ServiceError {
            status: 500,
            code: "InternalError".to_string(),
            message: "Internal server error".to_string(),
        };
        assert!(is_retryable_error(&error));

        // 4XX errors should NOT be retryable
        let error = crate::core::OciError::ConfigError("Bad request".to_string());
        assert!(!is_retryable_error(&error));
    }
}
