pub mod oci;

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

#[derive(Clone)]
pub struct WorkerInfo {
    pub id: WorkerId,
    pub ip: Option<IpAddr>,
    pub instance_state: WorkerInstanceState,
    pub instance_created: DateTime<Utc>,
}

#[derive(Clone)]
pub enum WorkerInstanceState {
    Starting,
    Running,
    Terminating,
}

pub type WorkerInfos = Vec<WorkerInfo>;
pub type WorkerHealthResponseMap = BTreeMap<WorkerId, (WorkerInfo, Option<WorkerStatus>)>;

pub trait WorkerInfra: Send + Sync {
    fn get_worker_infos<'a>(
        &'a self,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<WorkerInfos>> + 'a + Send>>;

    fn terminate<'a>(
        &'a self,
        worker_id: &'a WorkerId,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<()>> + 'a + Send>>;
}

impl dyn WorkerInfra {
    pub async fn get_worker_health_responses(
        &self,
        domain: &str,
    ) -> anyhow::Result<WorkerHealthResponseMap> {
        let workers_infos = self.get_worker_infos().await?;
        Ok(futures::stream::iter(workers_infos)
            .map(|worker_info| async move {
                let Some(ip) = worker_info.ip else {
                    return (worker_info.id.clone(), (worker_info, None));
                };
                let addr = SocketAddr::new(ip, 443);
                let Ok(res) = reqwest::Client::builder()
                    .resolve(domain, addr)
                    .timeout(Duration::from_secs(2))
                    .build()
                    .unwrap()
                    .get(format!("https://{domain}/health"))
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

                let Ok(worker_status) = body.parse::<WorkerStatus>() else {
                    panic!("Failed to parse health response: {body}");
                };

                (worker_info.id.clone(), (worker_info, Some(worker_status)))
            })
            .buffer_unordered(32)
            .collect()
            .await)
    }
}

#[derive(Clone)]
pub enum WorkerStatus {
    Good,
    ShuttingDown,
}

impl FromStr for WorkerStatus {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "good" => Ok(WorkerStatus::Good),
            "shutting_down" => Ok(WorkerStatus::ShuttingDown),
            _ => anyhow::bail!("invalid health response: {}", s),
        }
    }
}
