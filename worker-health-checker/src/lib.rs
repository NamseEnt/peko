use aws_config::BehaviorVersion;
use aws_sdk_lambda::primitives::Blob;
use serde::{Deserialize, Serialize};
use std::net::IpAddr;

#[derive(Debug, Serialize, Deserialize)]
pub struct Request {
    pub ips: Vec<IpAddr>,
    pub domain: String,
    pub port: u16,
    pub scheme: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Response {
    pub results: Vec<HealthCheckResult>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct HealthCheckResult {
    pub ip: IpAddr,
    pub status: Option<HealthStatus>,
}

#[derive(Debug, Serialize, Deserialize)]
pub enum HealthStatus {
    Good,
    GracefulShuttingDown,
}

pub struct WorkerHealthChecker {
    fn_name: String,
    client: aws_sdk_lambda::Client,
}

impl WorkerHealthChecker {
    pub async fn new(fn_name: String) -> Self {
        let config = aws_config::load_defaults(BehaviorVersion::latest())
            .await
            .to_builder()
            .use_dual_stack(true)
            .build();
        let client = aws_sdk_lambda::Client::new(&config);
        Self { fn_name, client }
    }

    pub async fn check_health(
        &self,
        ips: Vec<IpAddr>,
        domain: String,
        port: u16,
        scheme: String,
    ) -> Result<Response, String> {
        let request = Request {
            ips,
            domain,
            port,
            scheme,
        };
        let request = serde_json::to_string(&request).unwrap();

        let response = self
            .client
            .invoke()
            .function_name(&self.fn_name)
            .payload(Blob::new(request))
            .send()
            .await
            .map_err(|err| err.to_string())?;

        if let Some(function_error) = response.function_error {
            return Err(function_error);
        }

        let Some(payload) = response.payload else {
            return Err("no payload".to_string());
        };

        serde_json::from_slice(payload.as_ref()).map_err(|e| e.to_string())
    }
}
