use crate::*;
use chrono::Utc;
use color_eyre::eyre::Result;
use std::collections::BTreeSet;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::time::{MissedTickBehavior, interval, sleep};

const REAPER_INTERVAL: Duration = Duration::from_secs(10);
const REGISTER_ELAPSED_THRESHOLD: Duration = Duration::from_secs(60);
const HEALTH_CHECK_ELAPSED_THRESHOLD: Duration = Duration::from_secs(15);

pub async fn run(
    host_infra: Arc<dyn HostInfra>,
    host_info_map: HostInfoMap,
    health_check_map: HealthCheckMap,
) -> Result<()> {
    let mut interval = interval(REAPER_INTERVAL);
    interval.set_missed_tick_behavior(MissedTickBehavior::Skip);

    let mut last_terminate_set = BTreeSet::new();

    loop {
        interval.tick().await;

        let mut terminate_set = BTreeSet::new();

        for health_check in health_check_map.iter() {
            if health_check.last_check_time + HEALTH_CHECK_ELAPSED_THRESHOLD < Instant::now()
                && let Some(host_info) = host_info_map.get(health_check.key())
                && host_info.instance_created + REGISTER_ELAPSED_THRESHOLD < Utc::now()
                && host_info.instance_state != HostInstanceState::Terminating
                && !last_terminate_set.contains(health_check.key())
            {
                terminate_set.insert(health_check.key().clone());
            }
        }

        for host_id in terminate_set.clone() {
            let host_infra = host_infra.clone();
            tokio::spawn(async move {
                sleep(Duration::from_millis(rand::random::<u64>() % 1000)).await;
                let Err(err) = host_infra.terminate(&host_id).await else {
                    return;
                };
                eprintln!("Failed to terminate host {:?}: {:?}", host_id, err);
            });
        }

        last_terminate_set = terminate_set;
    }
}
