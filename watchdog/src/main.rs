//! # Rules
//!
//! 1. If no response for `MaxHealthCheckRetries` times, terminate the instance.
//! 2. Wait for `HealthCheckTimeout` for each request.
//! 3. Interval is every minute, not guaranteed. Only at least next check is after 30 seconds is guaranteed.
//! 4. Watchdog tries to make single master at once, but second master would be started if first master takes more than 30 seconds.
//!
//! # Internal Implementation
//!
//! ## Lock
//!
//! Best effort to make single master at once.
//!
//! ### DynamoDB
//!
//! - PK: `master_lock`
//! - SK: `_`
//! - Attributes:
//!   - `last_start_time`: `timestamp`
//! - Description
//!   - Read `last_start_time` and if it is older than 30 seconds, try update with optimistic locking.
//!   - If success, you are successful to get master lock. If fail, exit.
//!
//! ## Health Recorder
//!
//! Save health information of each instance
//!
//! ### S3 or Single File
//!
//! One of big file, including all instances health information, like
//! ```json
//! {
//!     "<instance_id>": {
//!         "unhealty_count": 0,
//!     }
//! }
//! ```
//!

mod health_recorder;
mod lock;
mod worker_infra;

use crate::{health_recorder::HealthRecord, worker_infra::WorkerInfo};
use futures::{FutureExt, StreamExt};
use health_recorder::HealthRecorder;
use lock::Lock;
use std::{collections::BTreeMap, env, net::SocketAddr, str::FromStr, sync::Arc, time::Duration};
use worker_infra::{WorkerInfra, oci::OciWorkerInfra};

#[derive(
    Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, serde::Serialize, serde::Deserialize,
)]
#[repr(transparent)]
#[serde(transparent)]
struct WorkerId(String);

fn main() {
    tokio::runtime::Runtime::new().unwrap().block_on(async {
        let lock_at = env::var("LOCK_AT").expect("env var LOCK_AT is not set");
        let lock: Box<dyn Lock> = match lock_at.as_str() {
            "dynamodb" => Box::new(lock::dynamodb::DynamoDbLock::new().await),
            _ => panic!("unknown lock type {lock_at}"),
        };

        let health_recorder_at =
            env::var("HEALTH_RECORDER_AT").expect("env var HEALTH_RECORDER_AT is not set");
        let health_recorder: Box<dyn HealthRecorder> = match health_recorder_at.as_str() {
            "s3" => Box::new(health_recorder::s3::S3HealthRecorder::new().await),
            _ => panic!("unknown health recorder type {health_recorder_at}"),
        };

        let worker_infra_at =
            env::var("WORKER_INFRA_AT").expect("env var WORKER_INFRA_AT is not set");
        let worker_infra: Arc<dyn WorkerInfra> = match worker_infra_at.as_str() {
            "oci" => Arc::new(OciWorkerInfra::new()),
            _ => panic!("unknown worker infra type {worker_infra_at}"),
        };

        let _result = run_watchdog(lock.as_ref(), health_recorder.as_ref(), worker_infra).await;
    });
}

async fn run_watchdog(
    lock: &dyn Lock,
    health_recorder: &dyn HealthRecorder,
    worker_infra: Arc<dyn WorkerInfra>,
) -> anyhow::Result<()> {
    let domain = env::var("DOMAIN").expect("env var DOMAIN is not set");
    let max_graceful_shutdown_wait_secs = env::var("MAX_GRACEFUL_SHUTDOWN_WAIT_SECS")
        .expect("MAX_GRACEFUL_SHUTDOWN_WAIT_SECS must be set")
        .parse::<u64>()
        .unwrap();
    let max_healthy_check_retrials = env::var("MAX_HEALTHY_CHECK_RETRIES")
        .expect("MAX_HEALTHY_CHECK_RETRIES must be set")
        .parse::<usize>()
        .unwrap();

    if !lock.try_lock().await? {
        println!("Failed to get lock");
        return Ok(());
    }

    let (health_records, health_responses) = futures::try_join!(
        health_recorder.read_all(),
        get_worker_health_responses(&domain, worker_infra.as_ref())
    )?;

    let now_secs = now_secs();

    let mut next_health_records = health_records.clone();
    let mut workers_to_terminate = vec![];

    for worker_id in health_records.keys() {
        let health_record = next_health_records.get_mut(worker_id).unwrap();
        let Some((worker_info, worker_status)) = health_responses.get(worker_id) else {
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

    for (worker_id, (_worker_info, worker_status)) in health_responses {
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

    let terminate_handles = workers_to_terminate.into_iter().map(|worker_info| {
        let worker_id = worker_info.id.clone();
        let worker_infra = worker_infra.clone();
        tokio::spawn(async move {
            if let Err(e) = worker_infra.terminate(&worker_id).await {
                println!("Failed to terminate worker {worker_id:?}: {e}");
            }
        })
    });

    futures::future::try_join(
        health_recorder.write_all(next_health_records),
        futures::future::join_all(terminate_handles).map(|_| Ok(())),
    )
    .await?;

    Ok(())
}

async fn get_worker_health_responses(
    domain: &str,
    worker_infra: &dyn WorkerInfra,
) -> anyhow::Result<BTreeMap<WorkerId, (WorkerInfo, Option<WorkerStatus>)>> {
    let workers_infos = worker_infra.get_worker_infos().await?;
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

enum WorkerStatus {
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

fn now_secs() -> u64 {
    std::time::UNIX_EPOCH.elapsed().unwrap().as_secs()
}
