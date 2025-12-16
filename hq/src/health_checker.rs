use crate::*;
use color_eyre::eyre::{Ok, Result};
use std::net::IpAddr;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::time::{MissedTickBehavior, interval, sleep};

pub struct HealthCheck {
    client: reqwest::Client,
    pub ip: IpAddr,
    pub last_check_time: Instant,
}
pub type HealthCheckMap = Arc<DashMap<HostId, HealthCheck>>;

const HEALTH_CHECK_INTERVAL: Duration = Duration::from_secs(5);
const HEALTH_CHECK_TIMEOUT: Duration = Duration::from_secs(5);

pub async fn run(host_info_map: HostInfoMap, health_check_map: HealthCheckMap) -> Result<()> {
    let mut interval = interval(HEALTH_CHECK_INTERVAL);
    interval.set_missed_tick_behavior(MissedTickBehavior::Skip);

    loop {
        interval.tick().await;

        remove_terminated_hosts(host_info_map.clone(), health_check_map.clone());
        send_health_check(host_info_map.clone(), health_check_map.clone());
    }
}

fn remove_terminated_hosts(host_info_map: HostInfoMap, health_check_map: HealthCheckMap) {
    health_check_map.retain(|id, _| {
        let Some(info) = host_info_map.get(id) else {
            return false;
        };
        info.instance_state != HostInstanceState::Terminating
    });
}

fn send_health_check(host_info_map: HostInfoMap, health_check_map: HealthCheckMap) {
    // TODO: Pass this domain from env
    const DOMAIN: &str = "fn0.dev";

    for info in host_info_map.iter() {
        let Some(ip) = info.ip else { continue };
        if info.instance_state != HostInstanceState::Running {
            continue;
        }

        let client = health_check_map
            .get(&info.id)
            .map(|info| info.client.clone())
            .unwrap_or_else(|| {
                reqwest::ClientBuilder::new()
                    .resolve(&format!("health.{DOMAIN}"), SocketAddr::new(ip, 443))
                    .timeout(HEALTH_CHECK_TIMEOUT)
                    .build()
                    .unwrap()
            });

        let health_check_map = health_check_map.clone();
        let host_id = info.id.clone();

        tokio::spawn(async move {
            sleep(Duration::from_millis(rand::random::<u64>() % 1000)).await;

            client
                .get(format!("https://health.{DOMAIN}/health"))
                .send()
                .await?
                .error_for_status()?;

            health_check_map
                .entry(host_id.clone())
                .and_modify(|health_check| {
                    health_check.last_check_time = Instant::now();
                })
                .or_insert_with(|| HealthCheck {
                    last_check_time: Instant::now(),
                    ip,
                    client,
                });

            Ok(())
        });
    }
}
