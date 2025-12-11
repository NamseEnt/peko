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

mod dns;
mod health_recorder;
mod lock;
mod scaling;
mod worker_infra;

use crate::{
    dns::Dns,
    health_recorder::{get_healthy_ips, get_workers_to_terminate, update_health_records},
    scaling::try_scale_out,
};
use chrono::{DateTime, Duration, Utc};
use color_eyre::config::Theme;
use futures::{FutureExt, StreamExt};
use health_recorder::HealthRecorder;
use lambda_runtime::{LambdaEvent, service_fn, tracing};
use lock::Lock;
use std::{env, sync::Arc};
use worker_infra::{WorkerInfra, oci::OciWorkerInfra};

#[derive(
    Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, serde::Serialize, serde::Deserialize,
)]
#[repr(transparent)]
#[serde(transparent)]
struct WorkerId(String);

const DEFAULT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(2);

fn main() {
    color_eyre::config::HookBuilder::new()
        .theme(Theme::new())
        .capture_span_trace_by_default(false)
        .add_default_filters()
        .add_frame_filter(Box::new(|frames| {
            frames.retain(|frame| {
                let Some(path) = &frame.filename else {
                    return false;
                };
                !path.to_string_lossy().contains(".cargo")
                    && !path.to_string_lossy().contains(".rustup")
            });
        }))
        .install()
        .unwrap();
    tracing::init_default_subscriber();

    tokio::runtime::Runtime::new().unwrap().block_on(async {
        let func = service_fn(|event: LambdaEvent<serde_json::Value>| async move {
            tracing::info!(?event);
            let context = Context::new();

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
            println!("worker_infra_at: {worker_infra_at}");
            let worker_infra: Arc<dyn WorkerInfra> = match worker_infra_at.as_str() {
                "oci" => Arc::new(OciWorkerInfra::new().await),
                _ => panic!("unknown worker infra type {worker_infra_at}"),
            };

            let dns_at = env::var("DNS_AT").expect("env var DNS_AT is not set");
            let dns: Arc<dyn Dns> = match dns_at.as_str() {
                "cloudflare" => Arc::new(dns::cloudflare::CloudflareDns::new(None).await),
                _ => panic!("unknown dns type {dns_at}"),
            };
            run_watchdog(&context, lock, health_recorder, worker_infra, dns)
                .await
                .map_err(|err| {
                    tracing::error!(?err);
                    lambda_runtime::Error::from(err)
                })
        });
        lambda_runtime::run(func).await.unwrap();
    });
}

async fn run_watchdog(
    context: &Context,
    lock: Arc<dyn Lock>,
    health_recorder: Arc<dyn HealthRecorder>,
    worker_infra: Arc<dyn WorkerInfra>,
    dns: Arc<dyn Dns>,
) -> color_eyre::Result<()> {
    if !lock.try_lock(context).await? {
        println!("Failed to get lock");
        return Ok(());
    }
    println!("lock acquired");

    let (mut health_records, worker_health_response_map) = futures::try_join!(
        health_recorder.read_all().then(|result| async {
            if result.is_ok() {
                println!("health_recorder.read_all() completed");
            }
            result
        }),
        worker_infra
            .get_worker_health_responses(&context.domain)
            .then(|result| async {
                if result.is_ok() {
                    println!("worker_infra.get_worker_health_responses() completed");
                }
                result
            })
    )?;

    println!("health_records: {:?}", health_records);
    println!(
        "worker_health_response_map: {:?}",
        worker_health_response_map
    );

    match update_health_records(
        context,
        &mut health_records,
        worker_health_response_map.clone(),
    ) {
        Ok(_) => {
            println!("Successfully updated health records");
        }
        Err(e) => {
            eprintln!("Failed to update health records: {e}");
            return Ok(());
        }
    };

    println!("health_records after update: {:?}", health_records);
    let workers_to_terminate = get_workers_to_terminate(&health_records);
    println!("workers to terminate: {:?}", workers_to_terminate);
    let helathy_ips = get_healthy_ips(&health_records);
    println!("healthy ips: {:?}", helathy_ips);

    futures::join!(
        health_recorder
            .write_all(health_records.clone())
            .then(|result| async {
                match result {
                    Ok(_) => println!("Successfully wrote health records"),
                    Err(err) => eprintln!("Failed to write health records: {err:?}"),
                }
            }),
        try_scale_out(
            context,
            health_records.clone(),
            worker_health_response_map,
            worker_infra.as_ref(),
        )
        .then(|result| async {
            match result {
                Ok(_) => println!("Successfully scaled out"),
                Err(err) => eprintln!("Failed to scale out: {err:?}"),
            }
        }),
        worker_infra
            .send_terminate_workers(workers_to_terminate)
            .then(|_| async { println!("sent terminate workers") }),
        dns.sync_ips(helathy_ips).then(|result| async {
            match result {
                Ok(_) => println!("Successfully synced ips"),
                Err(err) => eprintln!("Failed to sync ips: {err:?}"),
            }
        }),
    );

    println!("{:?}", health_records);

    Ok(())
}

struct Context {
    start_time: DateTime<Utc>,
    domain: String,
    max_graceful_shutdown_wait_time: Duration,
    max_healthy_check_retrials: usize,
    max_start_timeout: Duration,
    max_starting_count: usize,
}
impl Context {
    fn new() -> Self {
        Self {
            start_time: Utc::now(),
            domain: env::var("DOMAIN").expect("env var DOMAIN is not set"),
            max_graceful_shutdown_wait_time: Duration::seconds(
                env::var("MAX_GRACEFUL_SHUTDOWN_WAIT_SECS")
                    .expect("MAX_GRACEFUL_SHUTDOWN_WAIT_SECS must be set")
                    .parse::<u64>()
                    .expect("Failed to parse MAX_GRACEFUL_SHUTDOWN_WAIT_SECS")
                    as i64,
            ),
            max_healthy_check_retrials: env::var("MAX_HEALTHY_CHECK_RETRIES")
                .expect("MAX_HEALTHY_CHECK_RETRIES must be set")
                .parse::<usize>()
                .unwrap(),
            max_start_timeout: Duration::seconds(
                env::var("MAX_START_TIMEOUT_SECS")
                    .expect("MAX_START_TIMEOUT_SECS must be set")
                    .parse::<u64>()
                    .unwrap() as i64,
            ),
            max_starting_count: env::var("MAX_STARTING_COUNT")
                .expect("MAX_STARTING_COUNT must be set")
                .parse::<usize>()
                .unwrap(),
        }
    }
}
