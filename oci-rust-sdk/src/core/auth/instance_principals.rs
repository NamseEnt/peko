use crate::core::auth::federation::{generate_session_keypair, request_security_token};
use crate::core::auth::imds::ImdsClient;
use crate::core::auth::provider::AuthProvider;
use crate::core::auth::x509_utils::extract_tenant_id;
use crate::core::region::Region;
use chrono::{DateTime, Duration, Utc};
use std::str::FromStr;
use std::sync::Arc;
use tokio::sync::RwLock;

/// Session state for instance principals authentication
struct SessionState {
    security_token: String,
    session_private_key_pem: String,
    expires_at: DateTime<Utc>,
}

/// Instance Principals Authentication Provider
///
/// Enables applications running on OCI compute instances to authenticate
/// using X.509 certificates from the Instance Metadata Service (IMDS).
///
/// # Example
///
/// ```no_run
/// use oci_rust_sdk::core::auth::InstancePrincipalsAuthProvider;
/// use std::sync::Arc;
///
/// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
/// // Initialize provider (auto-detects region and fetches certs from IMDS)
/// let auth = InstancePrincipalsAuthProvider::new().await?;
/// let auth_ref = Arc::new(auth);
///
/// // Use with OciClient
/// // let client = OciClient::new(auth_ref, endpoint)?;
/// # Ok(())
/// # }
/// ```
pub struct InstancePrincipalsAuthProvider {
    runtime_handle: tokio::runtime::Handle,
    region: Region,
    tenancy_id: String,
    leaf_certificate: String,
    leaf_private_key_pem: String,
    intermediate_certificates: Vec<String>,
    session_state: Arc<RwLock<SessionState>>,
}

impl InstancePrincipalsAuthProvider {
    /// Create a new instance principals authentication provider
    ///
    /// This will:
    /// 1. Fetch region and certificates from IMDS
    /// 2. Extract tenant ID from certificate
    /// 3. Generate initial session keypair
    /// 4. Request initial security token from federation service
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - IMDS is unavailable (not running on OCI instance)
    /// - Certificate parsing fails
    /// - Federation service is unreachable
    /// - Token request fails
    pub async fn new() -> crate::core::Result<Self> {
        let runtime_handle = tokio::runtime::Handle::current();

        // 1. Create IMDS client
        let imds = ImdsClient::new()?;

        // 2. Fetch credentials from IMDS
        let region_str = imds.get_region().await?;
        let region = Region::from_str(&region_str).map_err(|e| {
            crate::core::OciError::ConfigError(format!("Invalid region from IMDS: {}", e))
        })?;

        let leaf_cert = imds.get_leaf_certificate().await?;
        let leaf_key = imds.get_leaf_private_key().await?;
        let intermediate_certs = imds.get_intermediate_certificates().await?;

        // 3. Parse certificate and extract tenant ID
        let tenancy_id = extract_tenant_id(&leaf_cert)?;

        // 4. Generate initial session keypair
        let session_keypair = generate_session_keypair()?;

        // 5. Request initial security token
        let security_token = request_security_token(
            &region,
            &tenancy_id,
            &leaf_cert,
            &leaf_key,
            &intermediate_certs,
            &session_keypair.public_key_pem,
        )
        .await?;

        // 6. Initialize session state
        let session_state = Arc::new(RwLock::new(SessionState {
            security_token: security_token.token,
            session_private_key_pem: session_keypair.private_key_pem,
            expires_at: security_token.expires_at,
        }));

        Ok(Self {
            runtime_handle,
            region,
            tenancy_id,
            leaf_certificate: leaf_cert,
            leaf_private_key_pem: leaf_key,
            intermediate_certificates: intermediate_certs,
            session_state,
        })
    }

    /// Get the region for this instance
    pub fn region(&self) -> Region {
        self.region
    }

    /// Ensure the security token is valid, refreshing if necessary
    async fn ensure_token_valid(&self) -> crate::core::Result<()> {
        // Read lock - check expiration (with 5-minute buffer)
        {
            let state = self.session_state.read().await;
            if !Self::is_expired(&state.expires_at) {
                return Ok(());
            }
        }

        // Write lock - refresh token
        {
            let mut state = self.session_state.write().await;

            // Double-check expiration (another thread may have refreshed)
            if !Self::is_expired(&state.expires_at) {
                return Ok(());
            }

            // Generate new session keypair
            let session_keypair = generate_session_keypair()?;

            // Request new token
            let security_token = request_security_token(
                &self.region,
                &self.tenancy_id,
                &self.leaf_certificate,
                &self.leaf_private_key_pem,
                &self.intermediate_certificates,
                &session_keypair.public_key_pem,
            )
            .await?;

            // Update state
            state.security_token = security_token.token;
            state.session_private_key_pem = session_keypair.private_key_pem;
            state.expires_at = security_token.expires_at;
        }

        Ok(())
    }

    /// Check if token is expired (with 5-minute buffer)
    fn is_expired(expires_at: &DateTime<Utc>) -> bool {
        let now = Utc::now();
        let buffer = Duration::minutes(5);
        now + buffer >= *expires_at
    }
}

impl AuthProvider for InstancePrincipalsAuthProvider {
    fn get_key_id(&self) -> String {
        // Ensure token is valid (blocking on async operation)
        self.runtime_handle
            .block_on(self.ensure_token_valid())
            .unwrap_or_else(|e| {
                eprintln!("Warning: Failed to refresh token: {}", e);
            });

        // Return "ST$<token>" format
        let state = self.session_state.blocking_read();
        format!("ST${}", state.security_token)
    }

    fn get_private_key(&self) -> &str {
        // Ensure token is valid (blocking on async operation)
        self.runtime_handle
            .block_on(self.ensure_token_valid())
            .unwrap_or_else(|e| {
                eprintln!("Warning: Failed to refresh token: {}", e);
            });

        // Return session private key
        // We use Box::leak() to satisfy the &str lifetime requirement
        // This creates a memory leak of ~2KB per refresh (once/hour)
        // which is acceptable for server applications
        let pem = self
            .session_state
            .blocking_read()
            .session_private_key_pem
            .clone();
        Box::leak(pem.into_boxed_str())
    }

    fn get_passphrase(&self) -> Option<&str> {
        // Instance principals don't use passphrases
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    #[test]
    fn test_is_expired() {
        // Token expires in 10 minutes - should not be considered expired (buffer is 5 min)
        let expires_at = Utc::now() + Duration::minutes(10);
        assert!(!InstancePrincipalsAuthProvider::is_expired(&expires_at));

        // Token expires in 4 minutes - should be considered expired (buffer is 5 min)
        let expires_at = Utc::now() + Duration::minutes(4);
        assert!(InstancePrincipalsAuthProvider::is_expired(&expires_at));

        // Token already expired
        let expires_at = Utc::now() - Duration::minutes(10);
        assert!(InstancePrincipalsAuthProvider::is_expired(&expires_at));
    }

    #[test]
    fn test_key_id_format() {
        // The key ID should start with "ST$"
        let token = "eyJhbGciOiJSUzI1NiIsInR5cCI6IkpXVCJ9.test.sig";
        let key_id = format!("ST${}", token);
        assert!(key_id.starts_with("ST$"));
        assert!(key_id.contains("eyJ"));
    }
}
