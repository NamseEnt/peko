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

#[instrument(skip_all, name = "reaper_loop")]
pub async fn run(
    host_provider: Arc<dyn HostProvider>,
    host_info_map: HostInfoMap,
    health_check_map: HealthCheckMap,
) -> Result<()> {
    info!("Starting reaper loop");

    let mut interval = interval(REAPER_INTERVAL);
    interval.set_missed_tick_behavior(MissedTickBehavior::Skip);

    let mut last_terminate_set = BTreeSet::new();

    loop {
        interval.tick().await;
        info!("reaper tick");

        let mut terminate_set = BTreeSet::new();

        for health_check in health_check_map.iter() {
            if health_check.last_check_time + HEALTH_CHECK_ELAPSED_THRESHOLD < Instant::now()
                && let Some(host_info) = host_info_map.get(health_check.key())
                && host_info.instance_created + REGISTER_ELAPSED_THRESHOLD < Utc::now()
                && host_info.instance_state != HostInstanceState::Terminating
                && !last_terminate_set.contains(health_check.key())
            {
                info!(host_id = %health_check.key(), "Host identified for termination");
                terminate_set.insert(health_check.key().clone());
            }
        }

        telemetry::send_reaper_terminate_candidates(terminate_set.len());

        for host_id in terminate_set.clone() {
            telemetry::send_reaper_terminate_attempts();
            let host_provider = host_provider.clone();
            let span = tracing::info_span!("reaper_terminate_host", host_id = %host_id);

            tokio::spawn(
                async move {
                    sleep(Duration::from_millis(rand::random::<u64>() % 1000)).await;
                    let Err(err) = host_provider.terminate(&host_id).await else {
                        info!("Host terminated successfully");
                        return;
                    };
                    error!(%err, "Failed to terminate host");
                }
                .instrument(span),
            );
        }

        last_terminate_set = terminate_set;
    }
}
