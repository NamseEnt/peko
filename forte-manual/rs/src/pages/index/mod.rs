use anyhow::Result;
use http::{HeaderMap, HeaderValue};

#[derive(serde::Serialize)]
pub enum Props {
    Ok {},
}

pub async fn handler(_headers: HeaderMap<HeaderValue>) -> Result<Props> {
    let props = Props::Ok {};
    Ok(props)
}
