use color_eyre::config::Theme;
use futures::StreamExt;
use lambda_runtime::{LambdaEvent, service_fn, tracing};
use serde::{Deserialize, Serialize};
use std::{
    net::{IpAddr, SocketAddr},
    time::Duration,
};

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

fn main() {
    color_eyre::config::HookBuilder::new()
        .theme(Theme::new())
        .capture_span_trace_by_default(false)
        .add_default_filters()
        .install()
        .unwrap();
    tracing::init_default_subscriber();

    tokio::runtime::Runtime::new().unwrap().block_on(async {
        let func = service_fn(|event: LambdaEvent<Request>| async move {
            tracing::info!(?event);
            let request = event.payload;

            let results = futures::stream::iter(request.ips)
                .then(|ip| {
                    let domain = request.domain.clone();
                    let port = request.port;
                    let scheme = request.scheme.clone();
                    async move {
                        let status = check_health(ip, &domain, port, &scheme).await;
                        HealthCheckResult { ip, status }
                    }
                })
                .collect::<Vec<_>>()
                .await;

            Ok::<_, lambda_runtime::Error>(Response { results })
        });
        lambda_runtime::run(func).await.unwrap();
    });
}

async fn check_health(ip: IpAddr, domain: &str, port: u16, scheme: &str) -> Option<HealthStatus> {
    let addr = SocketAddr::new(ip, port);

    let Ok(res) = reqwest::Client::builder()
        .resolve(&format!("a.{domain}"), addr)
        .timeout(Duration::from_secs(2))
        .build()
        .unwrap()
        .get(format!("{scheme}://a.{domain}:{port}/health"))
        .send()
        .await
    else {
        return None;
    };

    if !res.status().is_success() {
        return None;
    }

    let Ok(body) = res.text().await else {
        return None;
    };

    match body.as_str() {
        "good" => Some(HealthStatus::Good),
        "graceful_shutting_down" => Some(HealthStatus::GracefulShuttingDown),
        _ => None,
    }
}
