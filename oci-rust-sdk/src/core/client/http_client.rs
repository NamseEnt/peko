use crate::core::auth::provider::AuthProvider;
use crate::core::auth::signer::RequestSigner;
use crate::core::error::{OciError, ServiceErrorResponse};
use crate::core::retry::Retrier;
use reqwest::Method;
use serde::de::DeserializeOwned;
use std::sync::Arc;
use std::time::Duration;

/// OCI API client for making requests to Oracle Cloud Infrastructure services
pub struct OciClient {
    client: reqwest::Client,
    signer: RequestSigner,
    endpoint: String,
    retrier: Retrier,
}

impl OciClient {
    pub fn new(
        auth_provider: Arc<dyn AuthProvider>,
        endpoint: String,
        timeout: Duration,
    ) -> crate::core::Result<Self> {
        let client = reqwest::Client::builder()
            .user_agent(format!("oci-rust-sdk/{}", env!("CARGO_PKG_VERSION")))
            .timeout(timeout)
            .build()
            .map_err(OciError::HttpError)?;

        let signer = RequestSigner::new(auth_provider)?;
        let retrier = Retrier::new();

        Ok(Self {
            client,
            signer,
            endpoint,
            retrier,
        })
    }

    /// Create a new client with custom retry configuration
    pub fn with_retrier(mut self, retrier: Retrier) -> Self {
        self.retrier = retrier;
        self
    }

    /// Make a GET request
    pub async fn get<T>(&self, path: &str) -> crate::core::Result<OciResponse<T>>
    where
        T: DeserializeOwned,
    {
        self.request(Method::GET, path, None::<&()>).await
    }

    /// Make a POST request
    pub async fn post<B, T>(
        &self,
        path: &str,
        body: Option<&B>,
    ) -> crate::core::Result<OciResponse<T>>
    where
        B: serde::Serialize,
        T: DeserializeOwned,
    {
        self.request(Method::POST, path, body).await
    }

    /// Make a PUT request
    pub async fn put<B, T>(
        &self,
        path: &str,
        body: Option<&B>,
    ) -> crate::core::Result<OciResponse<T>>
    where
        B: serde::Serialize,
        T: DeserializeOwned,
    {
        self.request(Method::PUT, path, body).await
    }

    /// Make a DELETE request
    pub async fn delete<T>(&self, path: &str) -> crate::core::Result<OciResponse<T>>
    where
        T: DeserializeOwned,
    {
        self.request(Method::DELETE, path, None::<&()>).await
    }

    /// Make an HTTP request with retry logic
    async fn request<B, T>(
        &self,
        method: Method,
        path: &str,
        body: Option<&B>,
    ) -> crate::core::Result<OciResponse<T>>
    where
        B: serde::Serialize,
        T: DeserializeOwned,
    {
        // Serialize body once (outside retry loop)
        let body_bytes = if let Some(b) = body {
            Some(serde_json::to_vec(b)?)
        } else {
            None
        };

        // Execute with retry
        self.retrier
            .execute_with_retry(|| {
                let body_ref = body_bytes.as_deref();
                self.execute_request(method.clone(), path, body_ref)
            })
            .await
    }

    /// Execute a single HTTP request (called by retry logic)
    async fn execute_request<T>(
        &self,
        method: Method,
        path: &str,
        body: Option<&[u8]>,
    ) -> crate::core::Result<OciResponse<T>>
    where
        T: DeserializeOwned,
    {
        // Build URL
        let url = format!("{}{}", self.endpoint, path);
        let parsed_url =
            url::Url::parse(&url).map_err(|e| OciError::Other(format!("Invalid URL: {}", e)))?;

        // Create request builder
        let mut request_builder = self.client.request(method.clone(), url);

        // Add body if present
        if let Some(body_bytes) = body {
            request_builder = request_builder.body(body_bytes.to_vec());
        }

        // Build request to get headers
        let mut request = request_builder.build().map_err(OciError::HttpError)?;

        // Sign the request
        self.signer
            .sign_request(method.as_str(), &parsed_url, request.headers_mut(), body)?;

        // Execute request
        let response = self.client.execute(request).await?;

        // Check status
        let status = response.status();
        let headers = response.headers().clone();

        if !status.is_success() {
            // Parse error response
            let error_text = response.text().await?;

            let error = if let Ok(service_error) =
                serde_json::from_str::<ServiceErrorResponse>(&error_text)
            {
                OciError::from_response(status.as_u16(), service_error.code, service_error.message)
            } else {
                OciError::from_response(status.as_u16(), "Unknown".to_string(), error_text)
            };

            return Err(error);
        }

        // Parse response body
        let body = if status == reqwest::StatusCode::NO_CONTENT {
            // No content to parse
            serde_json::from_str("{}")?
        } else {
            response.json().await?
        };

        Ok(OciResponse { body, headers })
    }
}

/// HTTP response from OCI service
pub struct OciResponse<T> {
    pub body: T,
    pub headers: reqwest::header::HeaderMap,
}

impl<T> OciResponse<T> {
    /// Get a header value as a string
    pub fn get_header(&self, name: &str) -> Option<String> {
        self.headers
            .get(name)
            .and_then(|v| v.to_str().ok())
            .map(|s| s.to_string())
    }

    /// Get opc-request-id header
    pub fn request_id(&self) -> Option<String> {
        self.get_header("opc-request-id")
    }

    /// Get opc-next-page header (for pagination)
    pub fn next_page(&self) -> Option<String> {
        self.get_header("opc-next-page")
    }
}
