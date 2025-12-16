use super::*;
use std::{env, future::Future, net::IpAddr, pin::Pin};

const DEFAULT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(2);

pub struct CloudflareDns {
    client: reqwest::Client,
    zone_id: String,
    asterisk_domain: String,
    api_token: String,
    api_url: String,
}

impl CloudflareDns {
    pub fn new(api_url: Option<String>) -> Self {
        Self {
            client: reqwest::Client::builder()
                .local_address("[::]:0".parse().ok())
                .build()
                .unwrap(),
            zone_id: env::var("CLOUDFLARE_ZONE_ID").expect("env var CLOUDFLARE_ZONE_ID is not set"),
            asterisk_domain: env::var("CLOUDFLARE_ASTERISK_DOMAIN")
                .expect("env var CLOUDFLARE_ASTERISK_DOMAIN is not set"),
            api_token: env::var("CLOUDFLARE_API_TOKEN")
                .expect("env var CLOUDFLARE_API_TOKEN is not set"),
            api_url: api_url.unwrap_or_else(|| "https://api.cloudflare.com/client/v4".to_string()),
        }
    }
    async fn list_records(&self) -> color_eyre::Result<Vec<Record>> {
        let url = format!("{}/zones/{}/dns_records", self.api_url, self.zone_id);
        let params = [
            ("per_page", "5000000"),
            ("name.exact", self.asterisk_domain.as_str()),
        ];

        #[derive(Debug, serde::Deserialize)]
        struct CloudflareDnsRecordsResponse {
            success: bool,
            result: Option<Vec<RecordResponse>>,
            #[allow(dead_code)]
            errors: serde_json::Value,
        }

        #[derive(Debug, serde::Deserialize)]
        struct RecordResponse {
            r#type: String,
            content: String,
            id: String,
        }

        let text = self
            .client
            .get(url)
            .header("Authorization", format!("Bearer {}", self.api_token))
            .query(&params)
            .timeout(DEFAULT_TIMEOUT)
            .send()
            .await?
            .text()
            .await?;

        let response: CloudflareDnsRecordsResponse = serde_json::from_str(&text)?;

        if !response.success {
            eprintln!("Failed to list records: {response:?}");
            return Err(color_eyre::eyre::eyre!("Failed to list records"));
        }

        Ok(response
            .result
            .unwrap_or_default()
            .into_iter()
            .filter(|record| record.r#type == "A" || record.r#type == "AAAA")
            .map(|record| Record {
                ip: record.content.parse().unwrap(),
                id: record.id,
            })
            .collect())
    }
}

impl Dns for CloudflareDns {
    fn sync_ips<'a>(
        &'a self,
        ips: Vec<IpAddr>,
    ) -> Pin<Box<dyn Future<Output = color_eyre::Result<()>> + 'a + Send>> {
        Box::pin(async move {
            let old_records = self.list_records().await?;

            let new_ips = ips
                .iter()
                .filter(|ip| old_records.iter().all(|record| record.ip != **ip));

            let deleted_ips = old_records
                .iter()
                .filter(|record| ips.iter().all(|ip| record.ip != *ip));

            #[derive(serde::Serialize)]
            struct Body<'a> {
                deletes: Vec<Delete<'a>>,
                posts: Vec<Post<'a>>,
            }

            #[derive(serde::Serialize)]
            struct Delete<'a> {
                id: &'a str,
            }

            #[derive(serde::Serialize)]
            struct Post<'a> {
                name: &'a str,
                ttl: usize,
                r#type: &'static str,
                content: String,
                proxied: bool,
            }

            let response = self
                .client
                .post(format!(
                    "{}/zones/{}/dns_records/batch",
                    self.api_url, self.zone_id
                ))
                .header("Content-Type", "application/json")
                .header("Authorization", format!("Bearer {}", self.api_token))
                .body(serde_json::to_string(&Body {
                    deletes: deleted_ips
                        .map(|record| Delete {
                            id: record.id.as_str(),
                        })
                        .collect(),
                    posts: new_ips
                        .map(|ip| Post {
                            name: &self.asterisk_domain,
                            ttl: 60,
                            r#type: match ip {
                                IpAddr::V4(_) => "A",
                                IpAddr::V6(_) => "AAAA",
                            },
                            content: ip.to_string(),
                            proxied: false,
                        })
                        .collect(),
                })?)
                .timeout(DEFAULT_TIMEOUT)
                .send()
                .await?
                .text()
                .await?;

            println!("cloudflare sync_ips dns_records/batch Response: {response}");

            Ok(())
        })
    }
}

struct Record {
    ip: IpAddr,
    id: String,
}
