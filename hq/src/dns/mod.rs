pub mod cloudflare;
use tokio::time::MissedTickBehavior;

use crate::*;
use std::{future::Future, net::IpAddr, pin::Pin};

pub trait Dns: Send + Sync {
    fn sync_ips<'a>(
        &'a self,
        ips: Vec<IpAddr>,
    ) -> Pin<Box<dyn Future<Output = color_eyre::Result<()>> + 'a + Send>>;
}

// 4. dns-syncer는 5초마다, healthy(=health check한지 7.5초 이하)인 인스턴스들을 dns와
//    싱크를 맞춰야한다. 단, 매번 api 호출할 필요 없이 인메모리 캐시와 비교하여 변경사항
//    없으면 api를 보내지 않는다.\

const SYNC_INTERVAL: std::time::Duration = std::time::Duration::from_secs(5);
const HEALTHY_THRESHOLD: std::time::Duration = std::time::Duration::from_millis(7500);

#[instrument(skip_all, name = "dns_sync_loop")]
pub async fn sync_ips(health_check_map: HealthCheckMap) -> Result<()> {
    info!("Starting DNS sync loop");

    let dns = cloudflare::CloudflareDns::new(None);

    let mut interval = tokio::time::interval(SYNC_INTERVAL);
    interval.set_missed_tick_behavior(MissedTickBehavior::Skip);

    loop {
        interval.tick().await;
        info!("dns sync tick");

        let ips: Vec<_> = health_check_map
            .iter()
            .filter_map(|health_check| {
                if health_check.last_check_time.elapsed() > HEALTHY_THRESHOLD {
                    return None;
                }
                Some(health_check.ip)
            })
            .collect();

        telemetry::send_dns_healthy_ips(ips.len());

        if let Err(err) = dns.sync_ips(ips).await {
            error!(%err, "Failed to sync ips");
            telemetry::send_dns_sync_status(false);
        } else {
            telemetry::send_dns_sync_status(true);
        }
    }
}
