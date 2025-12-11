use super::*;
use crate::*;
use std::{env, net::IpAddr};

pub struct CloudflareDns {
    client: reqwest::Client,
    zone_id: String,
    /// ex) *.mydomain.com
    asterisk_domain: String,
    api_token: String,
    api_url: String,
}

impl CloudflareDns {
    pub async fn new(api_url: Option<String>) -> Self {
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

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn set_env_vars() {
        unsafe {
            env::set_var("CLOUDFLARE_ZONE_ID", "test_zone_id");
            env::set_var("CLOUDFLARE_ASTERISK_DOMAIN", "*.example.com");
            env::set_var("CLOUDFLARE_API_TOKEN", "test_token");
        }
    }

    #[tokio::test]
    async fn test_list_records() {
        set_env_vars();
        let mock_server = MockServer::start().await;

        let response_body = serde_json::json!({
            "success": true,
            "result": [
                {
                    "type": "A",
                    "content": "1.1.1.1",
                    "id": "record1"
                },
                {
                    "type": "AAAA",
                    "content": "2001:db8::1",
                    "id": "record2"
                }
            ]
        });

        Mock::given(method("GET"))
            .and(path("/zones/test_zone_id/dns_records"))
            .respond_with(ResponseTemplate::new(200).set_body_json(response_body))
            .mount(&mock_server)
            .await;

        let dns = CloudflareDns::new(Some(mock_server.uri())).await;
        let records = dns.list_records().await.unwrap();

        assert_eq!(records.len(), 2);
        assert_eq!(records[0].ip, "1.1.1.1".parse::<IpAddr>().unwrap());
        assert_eq!(records[0].id, "record1");
        assert_eq!(records[1].ip, "2001:db8::1".parse::<IpAddr>().unwrap());
        assert_eq!(records[1].id, "record2");
    }

    #[tokio::test]
    async fn test_sync_ips() {
        set_env_vars();
        let mock_server = MockServer::start().await;

        // Initial records: 1.1.1.1 (keep), 2.2.2.2 (delete), 2001:db8::1 (keep), 2001:db8::2 (delete)
        let list_response_body = serde_json::json!({
            "success": true,
            "result": [
                {
                    "type": "A",
                    "content": "1.1.1.1",
                    "id": "record1"
                },
                {
                    "type": "A",
                    "content": "2.2.2.2",
                    "id": "record2"
                },
                {
                    "type": "AAAA",
                    "content": "2001:db8::1",
                    "id": "record3_ipv6"
                },
                {
                    "type": "AAAA",
                    "content": "2001:db8::2",
                    "id": "record4_ipv6"
                }
            ]
        });

        Mock::given(method("GET"))
            .and(path("/zones/test_zone_id/dns_records"))
            .respond_with(ResponseTemplate::new(200).set_body_json(list_response_body))
            .mount(&mock_server)
            .await;

        // Expect batch update: delete record2, record4_ipv6, add 3.3.3.3, 2001:db8::3
        Mock::given(method("POST"))
            .and(path("/zones/test_zone_id/dns_records/batch"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(serde_json::json!({"success": true})),
            )
            .mount(&mock_server)
            .await;

        let dns = CloudflareDns::new(Some(mock_server.uri())).await;
        // New IPs: 1.1.1.1 (existing), 3.3.3.3 (new), 2001:db8::1 (existing), 2001:db8::3 (new)
        let new_ips = vec![
            "1.1.1.1".parse().unwrap(),
            "3.3.3.3".parse().unwrap(),
            "2001:db8::1".parse().unwrap(),
            "2001:db8::3".parse().unwrap(),
        ];

        dns.sync_ips(new_ips).await.unwrap();

        let received_requests = mock_server.received_requests().await.unwrap();
        assert_eq!(received_requests.len(), 2); // 1 list, 1 batch update

        let batch_request = &received_requests[1]; // The POST request
        let body: serde_json::Value = serde_json::from_slice(&batch_request.body).unwrap();

        // Verify deletions
        let deletes = body["deletes"].as_array().unwrap();
        assert_eq!(deletes.len(), 2);
        // We can't guarantee order of deletes, so check existence
        let deleted_ids: Vec<&str> = deletes.iter().map(|v| v.as_str().unwrap()).collect();
        assert!(deleted_ids.contains(&"record2"));
        assert!(deleted_ids.contains(&"record4_ipv6"));

        // Verify posts
        let posts = body["posts"].as_array().unwrap();
        assert_eq!(posts.len(), 2);

        // Convert posts to a more checkable format
        let post_contents: Vec<(&str, &str)> = posts
            .iter()
            .map(|p| (p["content"].as_str().unwrap(), p["type"].as_str().unwrap()))
            .collect();

        assert!(post_contents.contains(&("3.3.3.3", "A")));
        assert!(post_contents.contains(&("2001:db8::3", "AAAA")));

        for post in posts {
            assert_eq!(post["name"], "*.example.com");
        }
    }
}
