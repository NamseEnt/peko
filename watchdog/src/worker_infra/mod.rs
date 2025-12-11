pub mod oci_lambda_proxy;

use crate::WorkerId;
use chrono::{DateTime, Utc};
use futures::StreamExt;
use std::{
    collections::BTreeMap,
    future::Future,
    net::{IpAddr, SocketAddr},
    pin::Pin,
    str::FromStr,
    time::Duration,
};

#[derive(Debug, Clone)]
pub struct WorkerInfo {
    pub id: WorkerId,
    pub instance_created: DateTime<Utc>,
    pub ip: Option<IpAddr>,
    pub instance_state: WorkerInstanceState,
}

#[derive(Debug, Clone)]
pub enum WorkerInstanceState {
    Starting,
    Running,
    Terminating,
}

pub type WorkerInfos = Vec<WorkerInfo>;
pub type WorkerHealthResponseMap = BTreeMap<WorkerId, (WorkerInfo, Option<WorkerHealthResponse>)>;

pub trait WorkerInfra: Send + Sync {
    fn get_worker_infos<'a>(
        &'a self,
    ) -> Pin<Box<dyn Future<Output = color_eyre::Result<WorkerInfos>> + 'a + Send>>;

    fn terminate<'a>(
        &'a self,
        worker_id: &'a WorkerId,
    ) -> Pin<Box<dyn Future<Output = color_eyre::Result<()>> + 'a + Send>>;

    fn launch_instances<'a>(
        &'a self,
        count: usize,
    ) -> Pin<Box<dyn Future<Output = color_eyre::Result<()>> + 'a + Send>>;
}

impl dyn WorkerInfra {
    pub async fn get_worker_health_responses(
        &self,
        domain: &str,
    ) -> color_eyre::Result<WorkerHealthResponseMap> {
        self.get_worker_health_responses_with_options(domain, 443, "https")
            .await
    }

    async fn get_worker_health_responses_with_options(
        &self,
        domain: &str,
        port: u16,
        scheme: &str,
    ) -> color_eyre::Result<WorkerHealthResponseMap> {
        let workers_infos = self.get_worker_infos().await?;
        println!("workers_infos: {workers_infos:?}");
        Ok(futures::stream::iter(workers_infos)
            .map(|worker_info| async move {
                let Some(ip) = worker_info.ip else {
                    return (worker_info.id.clone(), (worker_info, None));
                };
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
                    return (worker_info.id.clone(), (worker_info, None));
                };

                if !res.status().is_success() {
                    return (worker_info.id.clone(), (worker_info, None));
                }

                let Ok(body) = res.text().await else {
                    return (worker_info.id.clone(), (worker_info, None));
                };

                let Ok(kind) = body.parse::<WorkerHealthKind>() else {
                    panic!("Failed to parse health response: {body}");
                };

                (
                    worker_info.id.clone(),
                    (worker_info, Some(WorkerHealthResponse { kind, ip })),
                )
            })
            .buffer_unordered(32)
            .collect()
            .await)
    }

    pub async fn send_terminate_workers(&self, worker_ids: impl IntoIterator<Item = WorkerId>) {
        futures::stream::iter(worker_ids)
            .for_each_concurrent(16, |worker_id| async move {
                if let Err(e) = self.terminate(&worker_id).await {
                    println!("Failed to terminate worker {worker_id:?}: {e}");
                }
            })
            .await
    }
}

#[derive(Debug, Clone)]
pub struct WorkerHealthResponse {
    pub kind: WorkerHealthKind,
    pub ip: IpAddr,
}

#[derive(Debug, Clone)]
pub enum WorkerHealthKind {
    Good,
    GracefulShuttingDown,
}

impl FromStr for WorkerHealthKind {
    type Err = color_eyre::Report;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "good" => Ok(WorkerHealthKind::Good),
            "graceful_shutting_down" => Ok(WorkerHealthKind::GracefulShuttingDown),
            _ => color_eyre::eyre::bail!("invalid health response: {}", s),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    struct MockWorkerInfra {
        workers: WorkerInfos,
    }

    impl WorkerInfra for MockWorkerInfra {
        fn get_worker_infos<'a>(
            &'a self,
        ) -> Pin<Box<dyn Future<Output = color_eyre::Result<WorkerInfos>> + 'a + Send>> {
            let workers = self.workers.clone();
            Box::pin(async move { Ok(workers) })
        }

        fn terminate<'a>(
            &'a self,
            _worker_id: &'a WorkerId,
        ) -> Pin<Box<dyn Future<Output = color_eyre::Result<()>> + 'a + Send>> {
            unimplemented!()
        }

        fn launch_instances<'a>(
            &'a self,
            _count: usize,
        ) -> Pin<Box<dyn Future<Output = color_eyre::Result<()>> + 'a + Send>> {
            unimplemented!()
        }
    }

    #[tokio::test]
    async fn test_get_worker_health_responses_all_good() {
        let mock_server = MockServer::start().await;
        let uri = mock_server.uri();
        let uri = uri.strip_prefix("http://").unwrap();
        let mut parts = uri.split(':');
        let ip: IpAddr = parts.next().unwrap().parse().unwrap();
        let port: u16 = parts.next().unwrap().parse().unwrap();

        Mock::given(method("GET"))
            .and(path("/health"))
            .respond_with(ResponseTemplate::new(200).set_body_string("good"))
            .mount(&mock_server)
            .await;

        let workers = vec![WorkerInfo {
            id: WorkerId("worker_1".to_string()),
            instance_created: Utc::now(),
            ip: Some(ip),
            instance_state: WorkerInstanceState::Running,
        }];

        let infra = MockWorkerInfra { workers };
        let infra: &dyn WorkerInfra = &infra;
        let responses = infra
            .get_worker_health_responses_with_options("example.com", port, "http")
            .await
            .unwrap();

        assert_eq!(responses.len(), 1);
        let (_, response) = responses.get(&WorkerId("worker_1".to_string())).unwrap();
        let response = response.as_ref().unwrap();
        assert!(matches!(response.kind, WorkerHealthKind::Good));
        assert_eq!(response.ip, ip);
    }

    #[tokio::test]
    async fn test_get_worker_health_responses_partial_failure() {
        let mock_server = MockServer::start().await;
        let uri = mock_server.uri();
        let uri = uri.strip_prefix("http://").unwrap();
        let mut parts = uri.split(':');
        let ip: IpAddr = parts.next().unwrap().parse().unwrap();
        let port: u16 = parts.next().unwrap().parse().unwrap();

        Mock::given(method("GET"))
            .and(path("/health"))
            .respond_with(ResponseTemplate::new(200).set_body_string("good"))
            .mount(&mock_server)
            .await;

        let workers = vec![
            WorkerInfo {
                id: WorkerId("worker_ok".to_string()),
                instance_created: Utc::now(),
                ip: Some(ip),
                instance_state: WorkerInstanceState::Running,
            },
            WorkerInfo {
                id: WorkerId("worker_unreachable".to_string()),
                instance_created: Utc::now(),
                ip: Some("192.0.2.0".parse().unwrap()),
                instance_state: WorkerInstanceState::Running,
            },
        ];

        let infra = MockWorkerInfra { workers };
        // The unreachable worker will timeout after 2 seconds.
        let infra: &dyn WorkerInfra = &infra;
        let responses = infra
            .get_worker_health_responses_with_options("example.com", port, "http")
            .await
            .unwrap();

        assert_eq!(responses.len(), 2);

        let (_, res_ok) = responses.get(&WorkerId("worker_ok".to_string())).unwrap();
        assert!(res_ok.is_some());

        let (_, res_bad) = responses
            .get(&WorkerId("worker_unreachable".to_string()))
            .unwrap();
        assert!(res_bad.is_none());
    }

    #[tokio::test]
    async fn test_get_worker_health_responses_500() {
        let mock_server = MockServer::start().await;
        let uri = mock_server.uri();
        let uri = uri.strip_prefix("http://").unwrap();
        let mut parts = uri.split(':');
        let ip: IpAddr = parts.next().unwrap().parse().unwrap();
        let port: u16 = parts.next().unwrap().parse().unwrap();

        Mock::given(method("GET"))
            .and(path("/health"))
            .respond_with(ResponseTemplate::new(500))
            .mount(&mock_server)
            .await;

        let workers = vec![WorkerInfo {
            id: WorkerId("worker_500".to_string()),
            instance_created: Utc::now(),
            ip: Some(ip),
            instance_state: WorkerInstanceState::Running,
        }];

        let infra = MockWorkerInfra { workers };
        let infra: &dyn WorkerInfra = &infra;
        let responses = infra
            .get_worker_health_responses_with_options("example.com", port, "http")
            .await
            .unwrap();

        // Should return None
        let (_, res) = responses.get(&WorkerId("worker_500".to_string())).unwrap();
        assert!(res.is_none());
    }
}
