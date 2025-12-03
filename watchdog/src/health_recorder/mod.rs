pub mod s3;

use crate::{
    WorkerId,
    worker_infra::{WorkerHealthResponseMap, WorkerInfos, WorkerStatus},
};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, future::Future, pin::Pin};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HealthRecord {
    pub health_check_retrials: usize,
    pub graceful_shutdown_start_at: Option<u64>,
}

pub type HealthRecords = BTreeMap<WorkerId, HealthRecord>;
pub type WorkersToTerminate = WorkerInfos;

pub trait HealthRecorder: Send + Sync {
    fn read_all<'a>(
        &'a self,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<HealthRecords>> + 'a + Send>>;
    fn write_all<'a>(
        &'a self,
        records: HealthRecords,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<()>> + 'a + Send>>;
}

pub async fn update_health_records(
    now_secs: u64,
    health_records: HealthRecords,
    worker_health_response_map: WorkerHealthResponseMap,
    max_graceful_shutdown_wait_secs: u64,
    max_healthy_check_retrials: usize,
) -> anyhow::Result<(HealthRecords, WorkersToTerminate)> {
    let mut next_health_records = health_records.clone();
    let mut workers_to_terminate = vec![];

    for worker_id in health_records.keys() {
        let health_record = next_health_records.get_mut(worker_id).unwrap();
        let Some((worker_info, worker_status)) = worker_health_response_map.get(worker_id) else {
            next_health_records.remove(worker_id);
            continue;
        };

        match worker_status {
            Some(worker_status) => match worker_status {
                WorkerStatus::Good => {
                    health_record.health_check_retrials = 0;
                    health_record.graceful_shutdown_start_at = None;
                }
                WorkerStatus::ShuttingDown => {
                    if health_record.graceful_shutdown_start_at.is_none() {
                        health_record.graceful_shutdown_start_at = Some(now_secs);
                    }
                    if now_secs
                        < health_record.graceful_shutdown_start_at.unwrap()
                            + max_graceful_shutdown_wait_secs
                    {
                        workers_to_terminate.push(worker_info.clone());
                    }
                }
            },
            None => {
                health_record.health_check_retrials += 1;

                if health_record.health_check_retrials >= max_healthy_check_retrials {
                    workers_to_terminate.push(worker_info.clone());
                }
            }
        }
    }

    for (worker_id, (_worker_info, worker_status)) in worker_health_response_map {
        if health_records.contains_key(&worker_id) {
            continue;
        }

        match worker_status {
            Some(worker_status) => match worker_status {
                WorkerStatus::Good => {
                    next_health_records.insert(
                        worker_id,
                        HealthRecord {
                            health_check_retrials: 0,
                            graceful_shutdown_start_at: None,
                        },
                    );
                }
                WorkerStatus::ShuttingDown => {
                    next_health_records.insert(
                        worker_id,
                        HealthRecord {
                            health_check_retrials: 0,
                            graceful_shutdown_start_at: Some(now_secs),
                        },
                    );
                }
            },
            None => {
                next_health_records.insert(
                    worker_id,
                    HealthRecord {
                        health_check_retrials: 1,
                        graceful_shutdown_start_at: None,
                    },
                );
            }
        }
    }

    Ok((next_health_records, workers_to_terminate))
}
