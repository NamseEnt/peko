//! Object body, in both directions: what `put` sends and what `get` returns.
//!
//! A body is either bytes the caller already holds or a response still being
//! streamed. Streaming costs no copy — the same stream is handed on — so an
//! object can be piped from one request to another, storage to storage or
//! storage to an outgoing call, without ever sitting in memory as a whole.

use crate::Result;
use bytes::Bytes;

pub struct Body(pub(crate) Inner);

pub(crate) enum Inner {
    Bytes(Bytes),
    #[cfg(target_arch = "wasm32")]
    Stream(forte_sdk::http::Body),
    #[cfg(not(target_arch = "wasm32"))]
    Stream(reqwest::Response),
}

impl Body {
    /// `None` for a stream, whose length is unknown until it ends. R2 accepts
    /// such a `PUT` as `Transfer-Encoding: chunked`.
    pub(crate) fn known_length(&self) -> Option<usize> {
        match &self.0 {
            Inner::Bytes(bytes) => Some(bytes.len()),
            Inner::Stream(_) => None,
        }
    }

    /// Reads the body to its end, holding all of it in memory.
    pub async fn bytes(self) -> Result<Bytes> {
        match self.0 {
            Inner::Bytes(bytes) => Ok(bytes),
            #[cfg(target_arch = "wasm32")]
            Inner::Stream(body) => body
                .bytes_limited(usize::MAX)
                .await
                .map_err(|error| crate::Error::Transport(error.to_string())),
            #[cfg(not(target_arch = "wasm32"))]
            Inner::Stream(response) => response
                .bytes()
                .await
                .map_err(|e| crate::Error::Transport(e.to_string())),
        }
    }
}

impl From<Bytes> for Body {
    fn from(value: Bytes) -> Self {
        Body(Inner::Bytes(value))
    }
}

impl From<Vec<u8>> for Body {
    fn from(value: Vec<u8>) -> Self {
        Body(Inner::Bytes(Bytes::from(value)))
    }
}

impl From<&[u8]> for Body {
    fn from(value: &[u8]) -> Self {
        Body(Inner::Bytes(Bytes::copy_from_slice(value)))
    }
}

impl From<String> for Body {
    fn from(value: String) -> Self {
        Body(Inner::Bytes(Bytes::from(value)))
    }
}

impl From<&str> for Body {
    fn from(value: &str) -> Self {
        Body(Inner::Bytes(Bytes::copy_from_slice(value.as_bytes())))
    }
}

#[cfg(target_arch = "wasm32")]
impl From<forte_sdk::http::Body> for Body {
    fn from(value: forte_sdk::http::Body) -> Self {
        Body(Inner::Stream(value))
    }
}

#[cfg(target_arch = "wasm32")]
impl From<Body> for forte_sdk::http::Body {
    fn from(value: Body) -> Self {
        match value.0 {
            Inner::Bytes(bytes) => forte_sdk::http::Body::Bytes(bytes.to_vec()),
            Inner::Stream(body) => body,
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
impl From<Body> for reqwest::Body {
    fn from(value: Body) -> Self {
        match value.0 {
            Inner::Bytes(bytes) => reqwest::Body::from(bytes),
            Inner::Stream(response) => reqwest::Body::wrap_stream(response.bytes_stream()),
        }
    }
}
