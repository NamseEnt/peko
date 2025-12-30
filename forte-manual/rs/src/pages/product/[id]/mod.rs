mod utils;

use anyhow::Result;
use http::{HeaderMap, HeaderValue};

#[derive(serde::Serialize)]
pub enum Props {
    Ok {},
}

pub async fn handler(_headers: HeaderMap<HeaderValue>, _id: usize) -> Result<Props> {
    let props = Props::Ok {};
    Ok(props)
}
