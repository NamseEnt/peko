use std::fmt;

use wit_bindgen::rt::async_support::{FutureReader, StreamReader, StreamResult, StreamWriter};

use crate::bindings::wasi::http::client;
use crate::bindings::wasi::http::types as p3;
use crate::bindings::{wit_future, wit_stream};

pub use bytes::Bytes;
pub use http::request::Builder as RequestBuilder;
pub use http::uri::{Authority, PathAndQuery, Scheme, Uri};
pub use http::{HeaderMap, HeaderName, HeaderValue, Method, Request, Response, StatusCode};

pub mod body {
    pub use super::{Body, BodyError, Bytes};
}

pub const DEFAULT_BODY_BUFFER_LIMIT: usize = 1024 * 1024;
pub const BODY_CHUNK_SIZE: usize = 64 * 1024;

pub enum Body {
    Empty,
    Bytes(Vec<u8>),
    Stream(StreamReader<u8>),
    Incoming {
        reader: StreamReader<u8>,
        trailers: Option<FutureReader<core::result::Result<Option<p3::Trailers>, p3::ErrorCode>>>,
    },
}

#[derive(Debug)]
pub enum BodyError {
    TooLarge { limit: usize },
    Wasi(p3::ErrorCode),
    Cancelled,
    InvalidUtf8(std::string::FromUtf8Error),
}

impl BodyError {
    pub fn is_too_large(&self) -> bool {
        match self {
            BodyError::TooLarge { .. } => true,
            BodyError::Wasi(p3::ErrorCode::HttpRequestBodySize(_)) => true,
            BodyError::Wasi(_) | BodyError::Cancelled | BodyError::InvalidUtf8(_) => false,
        }
    }
}

#[derive(Debug)]
pub enum Error {
    Headers(p3::HeaderError),
    InvalidScheme,
    InvalidAuthority,
    InvalidPathWithQuery,
    InvalidMethod,
    Wasi(p3::ErrorCode),
    BuildResponse(http::Error),
    Json(serde_json::Error),
    Body(BodyError),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Headers(e) => write!(f, "invalid headers: {e:?}"),
            Error::InvalidScheme => write!(f, "invalid scheme"),
            Error::InvalidAuthority => write!(f, "invalid authority"),
            Error::InvalidPathWithQuery => write!(f, "invalid path-with-query"),
            Error::InvalidMethod => write!(f, "invalid method"),
            Error::Wasi(ec) => write!(f, "wasi http error: {ec:?}"),
            Error::BuildResponse(e) => write!(f, "failed to build response: {e}"),
            Error::Json(e) => write!(f, "failed to decode JSON: {e}"),
            Error::Body(e) => write!(f, "failed to read body: {e}"),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Error::BuildResponse(e) => Some(e),
            Error::Json(e) => Some(e),
            Error::Body(e) => Some(e),
            _ => None,
        }
    }
}

impl From<http::Error> for Error {
    fn from(value: http::Error) -> Self {
        Error::BuildResponse(value)
    }
}

impl From<serde_json::Error> for Error {
    fn from(value: serde_json::Error) -> Self {
        Error::Json(value)
    }
}

impl From<BodyError> for Error {
    fn from(value: BodyError) -> Self {
        Error::Body(value)
    }
}

impl fmt::Display for BodyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            BodyError::TooLarge { limit } => {
                write!(f, "request body exceeds the {limit} byte buffering limit")
            }
            BodyError::Wasi(error) => write!(f, "WASI HTTP body error: {error:?}"),
            BodyError::Cancelled => write!(f, "request body delivery was cancelled"),
            BodyError::InvalidUtf8(error) => write!(f, "request body is not valid UTF-8: {error}"),
        }
    }
}

impl std::error::Error for BodyError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            BodyError::InvalidUtf8(error) => Some(error),
            _ => None,
        }
    }
}

pub type Result<T> = core::result::Result<T, Error>;

impl Body {
    pub fn empty() -> Self {
        Body::Empty
    }

    pub fn channel() -> (StreamWriter<u8>, Body) {
        let (writer, reader) = wit_stream::new::<u8>();
        (writer, Body::Stream(reader))
    }

    pub(crate) fn incoming(
        reader: StreamReader<u8>,
        trailers: FutureReader<core::result::Result<Option<p3::Trailers>, p3::ErrorCode>>,
    ) -> Self {
        Body::Incoming {
            reader,
            trailers: Some(trailers),
        }
    }

    pub async fn read_chunk(&mut self) -> core::result::Result<Option<Bytes>, BodyError> {
        match self {
            Body::Empty => Ok(None),
            Body::Bytes(bytes) => {
                let bytes = core::mem::take(bytes);
                *self = Body::Empty;
                if bytes.is_empty() {
                    Ok(None)
                } else {
                    Ok(Some(Bytes::from(bytes)))
                }
            }
            Body::Stream(reader) => read_stream_chunk(reader).await,
            Body::Incoming { reader, trailers } => read_incoming_chunk(reader, trailers).await,
        }
    }

    pub async fn bytes_limited(mut self, limit: usize) -> core::result::Result<Bytes, BodyError> {
        let mut buffered = Vec::new();
        while let Some(chunk) = self.read_chunk().await? {
            let remaining = limit.saturating_sub(buffered.len());
            if chunk.len() > remaining {
                return Err(BodyError::TooLarge { limit });
            }
            buffered.extend_from_slice(&chunk);
        }
        Ok(Bytes::from(buffered))
    }

    pub async fn bytes(self) -> Bytes {
        self.bytes_limited(usize::MAX).await.unwrap_or_default()
    }

    pub async fn json<T: serde::de::DeserializeOwned>(self) -> Result<T> {
        self.json_limited(DEFAULT_BODY_BUFFER_LIMIT).await
    }

    pub async fn json_limited<T: serde::de::DeserializeOwned>(self, limit: usize) -> Result<T> {
        let bytes = self.bytes_limited(limit).await.map_err(Error::Body)?;
        serde_json::from_slice(&bytes).map_err(Error::Json)
    }

    pub async fn text(self) -> Result<String> {
        self.text_limited(DEFAULT_BODY_BUFFER_LIMIT).await
    }

    pub async fn text_limited(self, limit: usize) -> Result<String> {
        let bytes = self.bytes_limited(limit).await.map_err(Error::Body)?;
        String::from_utf8(bytes.to_vec())
            .map_err(|error| Error::Body(BodyError::InvalidUtf8(error)))
    }

    pub async fn form(self) -> Result<Vec<(String, String)>> {
        self.form_limited(DEFAULT_BODY_BUFFER_LIMIT).await
    }

    pub async fn form_limited(self, limit: usize) -> Result<Vec<(String, String)>> {
        let bytes = self.bytes_limited(limit).await.map_err(Error::Body)?;
        Ok(form_urlencoded::parse(&bytes)
            .map(|(key, value)| (key.into_owned(), value.into_owned()))
            .collect())
    }
}

async fn read_stream_chunk(
    reader: &mut StreamReader<u8>,
) -> core::result::Result<Option<Bytes>, BodyError> {
    loop {
        let (status, bytes) = reader.read(Vec::with_capacity(BODY_CHUNK_SIZE)).await;
        match status {
            StreamResult::Complete(_) if bytes.is_empty() => continue,
            StreamResult::Complete(_) | StreamResult::Dropped => {
                return Ok((!bytes.is_empty()).then(|| Bytes::from(bytes)));
            }
            StreamResult::Cancelled => return Err(BodyError::Cancelled),
        }
    }
}

async fn read_incoming_chunk(
    reader: &mut StreamReader<u8>,
    trailers: &mut Option<FutureReader<core::result::Result<Option<p3::Trailers>, p3::ErrorCode>>>,
) -> core::result::Result<Option<Bytes>, BodyError> {
    loop {
        let (status, bytes) = reader.read(Vec::with_capacity(BODY_CHUNK_SIZE)).await;
        match status {
            StreamResult::Complete(_) if bytes.is_empty() => continue,
            StreamResult::Complete(_) => return Ok(Some(Bytes::from(bytes))),
            StreamResult::Dropped => {
                let trailer_result = match trailers.take() {
                    Some(trailer_reader) => trailer_reader.await,
                    None => Ok(None),
                };
                match trailer_result {
                    Ok(_) => return Ok((!bytes.is_empty()).then(|| Bytes::from(bytes))),
                    Err(error) => return Err(BodyError::Wasi(error)),
                }
            }
            StreamResult::Cancelled => return Err(BodyError::Cancelled),
        }
    }
}

impl From<Vec<u8>> for Body {
    fn from(v: Vec<u8>) -> Self {
        Body::Bytes(v)
    }
}

impl From<&[u8]> for Body {
    fn from(v: &[u8]) -> Self {
        Body::Bytes(v.to_vec())
    }
}

impl From<String> for Body {
    fn from(v: String) -> Self {
        Body::Bytes(v.into_bytes())
    }
}

impl From<&str> for Body {
    fn from(v: &str) -> Self {
        Body::Bytes(v.as_bytes().to_vec())
    }
}

impl From<Bytes> for Body {
    fn from(v: Bytes) -> Self {
        Body::Bytes(v.to_vec())
    }
}

impl From<()> for Body {
    fn from(_: ()) -> Self {
        Body::Empty
    }
}

#[derive(Default, Clone, Debug)]
pub struct Client {}

impl Client {
    pub fn new() -> Self {
        Self {}
    }

    pub async fn send<B: Into<Body>>(&self, req: Request<B>) -> Result<Response<Body>> {
        let (parts, body) = req.into_parts();
        let body: Body = body.into();

        let header_entries: Vec<(String, Vec<u8>)> = parts
            .headers
            .iter()
            .map(|(name, value)| (name.as_str().to_string(), value.as_bytes().to_vec()))
            .collect();
        let fields = p3::Fields::from_list(&header_entries).map_err(Error::Headers)?;

        let contents_reader = match body {
            Body::Empty => None,
            Body::Bytes(bytes) if bytes.is_empty() => None,
            Body::Bytes(bytes) => {
                let (mut writer, reader) = wit_stream::new::<u8>();
                crate::runtime::spawn(async move {
                    let _leftover = writer.write_all(bytes).await;
                    drop(writer);
                });
                Some(reader)
            }
            Body::Stream(reader) => Some(reader),
            Body::Incoming { reader, .. } => Some(reader),
        };

        let (trailers_writer, trailers_reader) = wit_future::new::<
            core::result::Result<Option<p3::Trailers>, p3::ErrorCode>,
        >(|| Ok(None));
        crate::runtime::spawn(async move {
            drop(trailers_writer);
        });

        let (wasi_req, _transmit) =
            p3::Request::new(fields, contents_reader, trailers_reader, None);

        wasi_req
            .set_method(&convert_method(&parts.method))
            .map_err(|_| Error::InvalidMethod)?;

        if let Some(scheme) = parts.uri.scheme_str() {
            wasi_req
                .set_scheme(Some(&convert_scheme(scheme)))
                .map_err(|_| Error::InvalidScheme)?;
        }
        if let Some(authority) = parts.uri.authority() {
            wasi_req
                .set_authority(Some(authority.as_str()))
                .map_err(|_| Error::InvalidAuthority)?;
        }
        if let Some(pq) = parts.uri.path_and_query() {
            wasi_req
                .set_path_with_query(Some(pq.as_str()))
                .map_err(|_| Error::InvalidPathWithQuery)?;
        }

        let wasi_resp = client::send(wasi_req).await.map_err(Error::Wasi)?;

        let status = wasi_resp.get_status_code();
        let wasi_headers = wasi_resp.get_headers();
        let header_list = wasi_headers.copy_all();
        drop(wasi_headers);

        let (res_trailers_writer, res_trailers_reader) =
            wit_future::new::<core::result::Result<(), p3::ErrorCode>>(|| Ok(()));
        crate::runtime::spawn(async move {
            drop(res_trailers_writer);
        });
        let (body_stream, body_trailers) =
            p3::Response::consume_body(wasi_resp, res_trailers_reader);

        let mut builder = Response::builder().status(status);
        for (name, value) in header_list {
            builder = builder.header(name, value);
        }
        builder
            .body(Body::Incoming {
                reader: body_stream,
                trailers: Some(body_trailers),
            })
            .map_err(Error::from)
    }
}

fn convert_method(m: &Method) -> p3::Method {
    match *m {
        Method::GET => p3::Method::Get,
        Method::HEAD => p3::Method::Head,
        Method::POST => p3::Method::Post,
        Method::PUT => p3::Method::Put,
        Method::DELETE => p3::Method::Delete,
        Method::CONNECT => p3::Method::Connect,
        Method::OPTIONS => p3::Method::Options,
        Method::TRACE => p3::Method::Trace,
        Method::PATCH => p3::Method::Patch,
        _ => p3::Method::Other(m.as_str().to_string()),
    }
}

fn convert_scheme(s: &str) -> p3::Scheme {
    match s {
        "http" => p3::Scheme::Http,
        "https" => p3::Scheme::Https,
        other => p3::Scheme::Other(other.to_string()),
    }
}
