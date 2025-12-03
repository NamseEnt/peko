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

use crate::{
    health_recorder::{HealthRecords, update_health_records},
    worker_infra::{WorkerHealthResponseMap, WorkerInstanceState},
};
use futures::{FutureExt, StreamExt};
use health_recorder::HealthRecorder;
use lock::Lock;
use std::{env, sync::Arc};
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
        let lock: Arc<dyn Lock> = match lock_at.as_str() {
            "dynamodb" => Arc::new(lock::dynamodb::DynamoDbLock::new().await),
            _ => panic!("unknown lock type {lock_at}"),
        };

        let health_recorder_at =
            env::var("HEALTH_RECORDER_AT").expect("env var HEALTH_RECORDER_AT is not set");
        let health_recorder: Arc<dyn HealthRecorder> = match health_recorder_at.as_str() {
            "s3" => Arc::new(health_recorder::s3::S3HealthRecorder::new().await),
            _ => panic!("unknown health recorder type {health_recorder_at}"),
        };

        let worker_infra_at =
            env::var("WORKER_INFRA_AT").expect("env var WORKER_INFRA_AT is not set");
        let worker_infra: Arc<dyn WorkerInfra> = match worker_infra_at.as_str() {
            "oci" => Arc::new(OciWorkerInfra::new()),
            _ => panic!("unknown worker infra type {worker_infra_at}"),
        };

        let _result = run_watchdog(lock, health_recorder, worker_infra).await;
    });
}

async fn run_watchdog(
    lock: Arc<dyn Lock>,
    health_recorder: Arc<dyn HealthRecorder>,
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
    let max_start_timeout_secs = env::var("MAX_START_TIMEOUT_SECS")
        .expect("MAX_START_TIMEOUT_SECS must be set")
        .parse::<u64>()
        .unwrap();
    let max_starting_count = env::var("MAX_STARTING_COUNT")
        .expect("MAX_STARTING_COUNT must be set")
        .parse::<usize>()
        .unwrap();

    if !lock.try_lock().await? {
        println!("Failed to get lock");
        return Ok(());
    }

    let (health_records, worker_health_response_map) = futures::try_join!(
        health_recorder.read_all(),
        worker_infra.get_worker_health_responses(&domain)
    )?;

    let now_secs = now_secs();

    let (next_health_records, workers_to_terminate) = update_health_records(
        now_secs,
        health_records,
        worker_health_response_map.clone(),
        max_graceful_shutdown_wait_secs,
        max_healthy_check_retrials,
    )
    .await?;

    let terminate_handles =
        futures::stream::iter(workers_to_terminate).for_each_concurrent(16, |worker_info| {
            let worker_infra = worker_infra.clone();
            async move {
                if let Err(e) = worker_infra.terminate(&worker_info.id).await {
                    println!("Failed to terminate worker {:?}: {e}", worker_info.id);
                }
            }
        });

    futures::try_join!(
        health_recorder.write_all(next_health_records.clone()),
        try_scale_out(
            now_secs,
            max_start_timeout_secs,
            max_starting_count,
            next_health_records,
            worker_health_response_map,
            worker_infra.clone(),
        ),
        terminate_handles.then(|_| async { Ok(()) }),
    )?;

    Ok(())
}

async fn try_scale_out(
    now_secs: u64,
    max_start_timeout_secs: u64,
    max_starting_count: usize,
    health_records: HealthRecords,
    worker_health_response_map: WorkerHealthResponseMap,
    worker_infra: Arc<dyn WorkerInfra>,
) -> anyhow::Result<()> {
    let starting_workers = worker_health_response_map
        .values()
        .filter(|(info, _status)| matches!(info.instance_state, WorkerInstanceState::Starting))
        .map(|(info, _status)| info);

    let (old_starting_workers, fresh_starting_workers): (Vec<_>, Vec<_>) = starting_workers
        .partition(|info| {
            now_secs as i64 - info.instance_created.timestamp() > max_start_timeout_secs as i64
        });

    let terminate_olds =
        futures::stream::iter(old_starting_workers).for_each_concurrent(16, |info| {
            let worker_infra = worker_infra.clone();
            async move {
                let _ = worker_infra.terminate(&info.id).await;
            }
        });

    let start_new = async move {
        let Some(_left_starting_count) =
            max_starting_count.checked_sub(fresh_starting_workers.len())
        else {
            return;
        };

        // TODO: 정상인 워커가 1개도 없으면 1개 시작해라.

        if !fresh_starting_workers.is_empty() {
            return;
        }

        // 그러러면 health_records 를 체크해서 정상인게 없는지 보면 되겠지.
    };

    futures::join!(terminate_olds, start_new);

    Ok(())
}

fn now_secs() -> u64 {
    std::time::UNIX_EPOCH.elapsed().unwrap().as_secs()
}

struct GeneralEnv {
    max_graceful_shutdown_wait_secs: u64,
    max_healthy_check_retrials: usize,
    max_start_timeout_secs: u64,
    max_starting_count: usize,
}
impl GeneralEnv {
    fn new() -> Self {
        Self {
            max_graceful_shutdown_wait_secs: env::var("MAX_GRACEFUL_SHUTDOWN_WAIT_SECS")
                .expect("MAX_GRACEFUL_SHUTDOWN_WAIT_SECS must be set")
                .parse::<u64>()
                .unwrap(),
            max_healthy_check_retrials: env::var("MAX_HEALTHY_CHECK_RETRIES")
                .expect("MAX_HEALTHY_CHECK_RETRIES must be set")
                .parse::<usize>()
                .unwrap(),
            max_start_timeout_secs: env::var("MAX_START_TIMEOUT_SECS")
                .expect("MAX_START_TIMEOUT_SECS must be set")
                .parse::<u64>()
                .unwrap(),
            max_starting_count: env::var("MAX_STARTING_COUNT")
                .expect("MAX_STARTING_COUNT must be set")
                .parse::<usize>()
                .unwrap(),
        }
    }
}
