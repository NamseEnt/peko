pub mod context;
pub mod dns;
pub mod host_id;
pub mod host_registry;
pub mod task_orchestrator;

use chrono::Duration;
pub use context::Context;
pub use host_id::HostId;

const DEFAULT_HEALTH_CHECK_TIMEOUT: Duration = Duration::seconds(5);

pub async fn check_health_single(host_info: &HostInfo, domain: &str) -> Option<HostHealthResponse> {
    let Some(ip) = host_info.ip else {
        return None;
    };

    let client = reqwest::Client::new();
    let url = format!("https://{}/health", ip);

    let response = tokio::time::timeout(
        DEFAULT_HEALTH_CHECK_TIMEOUT.to_std().unwrap(),
        client.get(&url).header("Host", domain).send(),
    )
    .await;

    match response {
        Ok(Ok(resp)) if resp.status().is_success() => {
            let body = resp.text().await.ok()?;
            Some(HostHealthResponse {
                kind: match body.trim() {
                    "good" => HostHealthKind::Good,
                    "graceful_shutting_down" => HostHealthKind::GracefulShuttingDown,
                    _ => return None,
                },
                ip,
            })
        }
        _ => None,
    }
}
