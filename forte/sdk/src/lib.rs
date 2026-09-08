pub mod bindings;
pub mod cookie_sign;
pub mod http;
pub mod metrics;
pub(crate) mod otel;
pub mod rand;
pub mod runtime;
pub mod serve;
pub mod static_page_cache;
#[cfg(feature = "test-harness")]
pub mod test_harness;
pub mod time_wasi;
pub mod websocket;

pub use anyhow;
pub use chrono;
pub use cookie::{self, Cookie, CookieBuilder, CookieJar};
pub use form_urlencoded;
pub use forte_json;
pub use forte_macros::{cache_static, forte_doc, test};
pub use futures;
pub use hex;
#[cfg(feature = "test-harness")]
pub use inventory;
pub use serde;
pub use serde_json;
pub use sha2;
pub use time;
pub use tracing;
pub use uuid::Uuid;
pub use wit_bindgen;

pub type DateTime = chrono::DateTime<chrono::Utc>;

pub mod http_header {
    pub use http::header::*;
}

pub fn now() -> DateTime {
    chrono::Utc::now()
}

pub struct ForteRequest<'a, Body = http::Body> {
    pub uri_authority: &'a str,
    pub method: &'a ::http::Method,
    pub headers: &'a ::http::HeaderMap,
    pub jar: &'a mut CookieJar,
    pub raw_body: &'a [u8],
    pub body: Body,
}

pub type ForteResponse = ::http::Response<http::Body>;
