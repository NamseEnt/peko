pub mod s3;
mod update_health_records;

use crate::{
    WorkerId,
    worker_infra::{WorkerHealthResponseMap, WorkerInstanceState},
};
use chrono::{DateTime, TimeDelta, Utc};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, future::Future, net::IpAddr, pin::Pin};
pub use update_health_records::*;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HealthRecord {
    pub state: HealthState,
    pub state_transited_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum HealthState {
    Starting,
    Healthy { ip: IpAddr },
    RetryingCheck { retrials: usize },
    MarkedForTermination,
    GracefulShuttingDown,
    TerminatedConfirm,
    InvisibleOnInfra,
}

pub type HealthRecords = BTreeMap<WorkerId, HealthRecord>;

pub trait HealthRecorder: Send + Sync {
    fn read_all<'a>(
        &'a self,
    ) -> Pin<Box<dyn Future<Output = color_eyre::Result<HealthRecords>> + 'a + Send>>;
    fn write_all<'a>(
        &'a self,
        records: HealthRecords,
    ) -> Pin<Box<dyn Future<Output = color_eyre::Result<()>> + 'a + Send>>;
}

pub fn get_workers_to_terminate(health_records: &HealthRecords) -> Vec<WorkerId> {
    health_records
        .iter()
        .filter_map(|(id, record)| {
            if let HealthState::MarkedForTermination = record.state {
                Some(id.clone())
            } else {
                None
            }
        })
        .collect()
}

pub fn get_healthy_ips(health_records: &HealthRecords) -> Vec<IpAddr> {
    health_records
        .iter()
        .filter_map(|(_, record)| {
            if let HealthState::Healthy { ip } = record.state {
                Some(ip)
            } else {
                None
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::Ipv4Addr;

    fn create_health_record(state: HealthState) -> HealthRecord {
        HealthRecord {
            state,
            state_transited_at: Utc::now(),
        }
    }

    #[test]
    fn test_get_workers_to_terminate() {
        let mut health_records = HealthRecords::new();

        // Case 1: Empty records
        assert!(get_workers_to_terminate(&health_records).is_empty());

        // Case 2: Mixed states
        health_records.insert(
            WorkerId("worker-1".to_string()),
            create_health_record(HealthState::MarkedForTermination),
        );
        health_records.insert(
            WorkerId("worker-2".to_string()),
            create_health_record(HealthState::Healthy {
                ip: IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)),
            }),
        );
        health_records.insert(
            WorkerId("worker-3".to_string()),
            create_health_record(HealthState::Starting),
        );
        health_records.insert(
            WorkerId("worker-4".to_string()),
            create_health_record(HealthState::MarkedForTermination),
        );

        let workers_to_terminate = get_workers_to_terminate(&health_records);
        assert_eq!(workers_to_terminate.len(), 2);
        assert!(workers_to_terminate.contains(&WorkerId("worker-1".to_string())));
        assert!(workers_to_terminate.contains(&WorkerId("worker-4".to_string())));
        assert!(!workers_to_terminate.contains(&WorkerId("worker-2".to_string())));
    }

    #[test]
    fn test_get_healthy_ips() {
        let mut health_records = HealthRecords::new();

        // Case 1: Empty records
        assert!(get_healthy_ips(&health_records).is_empty());

        // Case 2: Mixed states
        let ip1 = IpAddr::V4(Ipv4Addr::new(192, 168, 0, 1));
        let ip2 = IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1));

        health_records.insert(
            WorkerId("worker-1".to_string()),
            create_health_record(HealthState::Healthy { ip: ip1 }),
        );
        health_records.insert(
            WorkerId("worker-2".to_string()),
            create_health_record(HealthState::Starting),
        );
        health_records.insert(
            WorkerId("worker-3".to_string()),
            create_health_record(HealthState::Healthy { ip: ip2 }),
        );
        health_records.insert(
            WorkerId("worker-4".to_string()),
            create_health_record(HealthState::MarkedForTermination),
        );

        let healthy_ips = get_healthy_ips(&health_records);
        assert_eq!(healthy_ips.len(), 2);
        assert!(healthy_ips.contains(&ip1));
        assert!(healthy_ips.contains(&ip2));
    }
}
