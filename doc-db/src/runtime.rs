use anyhow::Result;
use std::time::Duration;

#[cfg(target_arch = "wasm32")]
pub(crate) async fn http_post_json(
    url: &str,
    body: Vec<u8>,
    auth_token: Option<&str>,
) -> Result<Vec<u8>> {
    use anyhow::bail;
    use forte_sdk::http::{Client, HeaderValue, Request, Uri};
    use std::str::FromStr;

    let uri = Uri::from_str(url).map_err(|e| anyhow::anyhow!("Invalid URI: {e}"))?;
    let mut builder = Request::post(&uri).header(
        "Content-Type",
        HeaderValue::from_str("application/json")
            .map_err(|e| anyhow::anyhow!("Invalid Content-Type: {e}"))?,
    );
    if let Some(token) = auth_token {
        builder = builder.header(
            "Authorization",
            HeaderValue::from_str(&format!("Bearer {token}"))
                .map_err(|e| anyhow::anyhow!("Invalid Authorization: {e}"))?,
        );
    }
    let request = builder
        .body(body)
        .map_err(|e| anyhow::anyhow!("Failed to build request: {e}"))?;
    let client = Client::new();
    let response = client.send(request).await?;
    if !response.status().is_success() {
        bail!("HTTP request failed with status: {}", response.status());
    }
    let bytes = response.into_body().bytes_limited(usize::MAX).await?;
    Ok(bytes.to_vec())
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) async fn http_post_json(
    url: &str,
    body: Vec<u8>,
    auth_token: Option<&str>,
) -> Result<Vec<u8>> {
    use anyhow::bail;
    use std::sync::OnceLock;

    static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
    let client = CLIENT.get_or_init(reqwest::Client::new);

    let mut req = client
        .post(url)
        .header("Content-Type", "application/json")
        .body(body);
    if let Some(token) = auth_token {
        req = req.header("Authorization", format!("Bearer {token}"));
    }
    let resp = req.send().await?;
    if !resp.status().is_success() {
        bail!("HTTP request failed with status: {}", resp.status());
    }
    Ok(resp.bytes().await?.to_vec())
}

#[cfg(target_arch = "wasm32")]
pub(crate) async fn sleep(duration: Duration) {
    forte_sdk::time_wasi::sleep(duration).await;
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) async fn sleep(duration: Duration) {
    tokio::time::sleep(duration).await;
}

#[cfg(target_arch = "wasm32")]
pub(crate) async fn random_bytes(buf: &mut [u8]) {
    forte_sdk::rand::get_insecure_random_bytes(buf);
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) async fn random_bytes(buf: &mut [u8]) {
    use rand::RngCore;
    rand::thread_rng().fill_bytes(buf);
}
