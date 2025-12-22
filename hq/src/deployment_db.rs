use bytes::Buf;
use color_eyre::eyre::Result;
use libsql::{Builder, Database, Row};
use std::{sync::Arc, time::Duration};
use tokio::time::MissedTickBehavior;
use tracing::warn;

use crate::telemetry;

#[derive(Clone)]
pub struct DeploymentDb {
    cache: Arc<boxcar::Vec<Deployment>>,
    db: Arc<Database>,
}

const PK: &str = "deployments";

impl DeploymentDb {
    pub async fn new(url: String, token: String) -> Result<Self> {
        let db = Builder::new_remote(url, token).build().await?;
        let cache = Arc::new(load_all(&db).await?);
        telemetry::send_deployment_db_cached_count(cache.count());
        Ok(Self {
            cache,
            db: Arc::new(db),
        })
    }

    pub async fn run_sync(&self) {
        let mut interval = tokio::time::interval(deployment_id_sync_interval_ms());
        interval.set_missed_tick_behavior(MissedTickBehavior::Skip);

        loop {
            interval.tick().await;

            let conn = match self.db.connect() {
                Ok(conn) => {
                    telemetry::send_deployment_db_connect_status(true);
                    conn
                }
                Err(err) => {
                    warn!(%err, "Failed to connect to libsql");
                    telemetry::send_deployment_db_connect_status(false);
                    continue;
                }
            };

            let sk = self.cache.count() as i64;

            let mut rows = match conn
                .query(
                    "SELECT value FROM docs WHERE pk = ? AND sk > ? ORDER BY sk ASC",
                    libsql::params!(PK, sk),
                )
                .await
            {
                Ok(stream) => {
                    telemetry::send_deployment_db_fetch_status(true);
                    stream
                }
                Err(err) => {
                    warn!(%err, "Failed to fetch deployments");
                    telemetry::send_deployment_db_fetch_status(false);
                    telemetry::send_deployment_db_sync_status(false);
                    break;
                }
            };

            let mut new_count = 0;
            let mut had_error = false;

            loop {
                match rows.next().await {
                    Ok(Some(row)) => {
                        self.cache.push(row_to_deployment(row));
                        new_count += 1;
                    }
                    Ok(None) => break,
                    Err(err) => {
                        warn!(%err, "Failed to fetch deployments");
                        telemetry::send_deployment_db_fetch_status(false);
                        had_error = true;
                        break;
                    }
                }
            }

            if new_count > 0 {
                telemetry::send_deployment_db_new_loaded(new_count);
            }

            telemetry::send_deployment_db_cached_count(self.cache.count());

            if !had_error {
                telemetry::send_deployment_db_sync_status(true);
            } else {
                telemetry::send_deployment_db_sync_status(false);
            }
        }
    }

    pub fn slice_updates(&self, deployment_id_start_excluded: DeploymentId) -> Vec<Deployment> {
        assert!(deployment_id_start_excluded.0 as usize <= self.cache.count());
        self.cache
            .iter()
            .skip(deployment_id_start_excluded.0 as usize)
            .map(|(_, deployment)| *deployment)
            .collect()
    }

    pub fn last_deployment_id(&self) -> u64 {
        self.cache.count() as _
    }
}

fn row_to_deployment(row: Row) -> Deployment {
    let bytes: [u8; 16] = row.get(0).unwrap();
    let mut cursor = bytes.as_slice();

    Deployment {
        code_id: cursor.get_u64(),
        code_version: cursor.get_u64(),
    }
}

async fn load_all(db: &Database) -> Result<boxcar::Vec<Deployment>> {
    let mut out = Vec::new();

    let mut rows = db
        .connect()?
        .query(
            "SELECT value FROM docs WHERE pk = ? ORDER BY sk ASC",
            libsql::params!(PK),
        )
        .await?;

    while let Some(row) = rows.next().await? {
        out.push(row_to_deployment(row));
    }

    Ok(boxcar::Vec::from_iter(out))
}

#[derive(Clone, Copy)]
pub struct Deployment {
    pub code_id: u64,
    pub code_version: u64,
}

#[derive(Eq, Hash, PartialEq, Clone, Debug, Ord, PartialOrd)]
pub struct DeploymentId(pub u64);

fn deployment_id_sync_interval_ms() -> Duration {
    match std::env::var("DEPLOYMENT_ID_SYNC_INTERVAL_MS") {
        Ok(s) => match s.parse() {
            Ok(v) => return Duration::from_millis(v),
            Err(err) => warn!(%err, "DEPLOYMENT_ID_SYNC_INTERVAL_MS is not a valid number"),
        },
        Err(err) => warn!(%err, "Fail to get DEPLOYMENT_ID_SYNC_INTERVAL_MS from env"),
    }
    Duration::from_secs(2)
}
