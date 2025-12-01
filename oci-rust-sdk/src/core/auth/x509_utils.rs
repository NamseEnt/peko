use base64::{engine::general_purpose, Engine as _};
use x509_parser::prelude::*;

/// Extract tenant OCID from X.509 certificate subject DN
///
/// Looks for "opc-tenant:" or "opc-identity:" prefix in subject components
pub fn extract_tenant_id(cert_pem: &str) -> crate::core::Result<String> {
    // Manually parse PEM to DER
    let cert_der = pem_to_der(cert_pem)?;

    let (_, cert) = X509Certificate::from_der(&cert_der).map_err(|e| {
        crate::core::OciError::AuthError(format!("Failed to parse X.509 certificate: {}", e))
    })?;

    // Search through subject DN components
    for rdn in cert.subject().iter() {
        for attr in rdn.iter() {
            if let Ok(value_str) = attr.as_str() {
                if let Some(tenant_id) = extract_tenant_from_value(value_str) {
                    return Ok(tenant_id);
                }
            }
        }
    }

    Err(crate::core::OciError::AuthError(
        "Certificate does not contain tenant ID (opc-tenant or opc-identity)".to_string(),
    ))
}

/// Helper function to extract tenant ID from a subject component value
fn extract_tenant_from_value(value: &str) -> Option<String> {
    const OPC_TENANT: &str = "opc-tenant:";
    const OPC_IDENTITY: &str = "opc-identity:";

    if let Some(idx) = value.find(OPC_TENANT) {
        return Some(value[idx + OPC_TENANT.len()..].to_string());
    }

    if let Some(idx) = value.find(OPC_IDENTITY) {
        return Some(value[idx + OPC_IDENTITY.len()..].to_string());
    }

    None
}

/// Sanitize PEM certificate by removing header and footer
///
/// Converts from:
/// ```text
/// -----BEGIN CERTIFICATE-----
/// MIICx...
/// -----END CERTIFICATE-----
/// ```
///
/// To: `MIICx...` (just the base64 content)
pub fn sanitize_certificate_pem(pem: &str) -> String {
    pem.lines()
        .filter(|line| !line.starts_with("-----"))
        .collect::<Vec<_>>()
        .join("")
}

/// Convert PEM to DER by decoding base64
fn pem_to_der(pem: &str) -> crate::core::Result<Vec<u8>> {
    // Remove headers and decode base64
    let base64_data = pem
        .lines()
        .filter(|line| !line.starts_with("-----"))
        .collect::<Vec<_>>()
        .join("");

    general_purpose::STANDARD
        .decode(base64_data)
        .map_err(|e| crate::core::OciError::AuthError(format!("Failed to decode PEM: {}", e)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sanitize_certificate() {
        let cert = r#"-----BEGIN CERTIFICATE-----
MIICxTCCAa2gAwIBAgIUQw4YgqVmKGjGPXHMRBZf8hNOiHg
-----END CERTIFICATE-----"#;
        let sanitized = sanitize_certificate_pem(cert);
        assert!(!sanitized.contains("-----BEGIN"));
        assert!(!sanitized.contains("-----END"));
        assert!(sanitized.starts_with("MIIC"));
    }

    #[test]
    fn test_extract_tenant_from_invalid_cert() {
        let invalid_cert = r#"-----BEGIN CERTIFICATE-----
Invalid certificate data
-----END CERTIFICATE-----"#;

        let result = extract_tenant_id(invalid_cert);
        assert!(result.is_err());
    }

    #[test]
    fn test_extract_tenant_from_value() {
        // Test with opc-tenant prefix
        let value = "ocid1.tenancy.oc1..xxx,opc-tenant:ocid1.tenancy.oc1..aaaabbbb";
        let tenant = extract_tenant_from_value(value).unwrap();
        assert_eq!(tenant, "ocid1.tenancy.oc1..aaaabbbb");

        // Test with opc-identity prefix
        let value = "opc-identity:ocid1.tenancy.oc1..ccccdddd";
        let tenant = extract_tenant_from_value(value).unwrap();
        assert_eq!(tenant, "ocid1.tenancy.oc1..ccccdddd");

        // Test without prefix
        let value = "some other value";
        let tenant = extract_tenant_from_value(value);
        assert!(tenant.is_none());
    }
}
