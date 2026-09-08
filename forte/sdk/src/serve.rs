use std::fmt;
use std::future::Future;

use crate::bindings::wasi::http::types as p3;
use crate::bindings::{wit_future, wit_stream};
use crate::http::Body;
use tracing::Instrument;

#[derive(Debug)]
pub enum ServeError {
    InvalidUri(http::uri::InvalidUri),
    BuildRequest(http::Error),
    Headers(p3::HeaderError),
    InvalidStatusCode,
}

impl fmt::Display for ServeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ServeError::InvalidUri(e) => write!(f, "invalid uri: {e}"),
            ServeError::BuildRequest(e) => write!(f, "failed to build http::Request: {e}"),
            ServeError::Headers(e) => write!(f, "invalid response headers: {e:?}"),
            ServeError::InvalidStatusCode => write!(f, "invalid status code"),
        }
    }
}

impl std::error::Error for ServeError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            ServeError::InvalidUri(e) => Some(e),
            ServeError::BuildRequest(e) => Some(e),
            _ => None,
        }
    }
}

pub async fn serve<F, Fut, E>(
    req: p3::Request,
    dispatch: F,
) -> core::result::Result<p3::Response, p3::ErrorCode>
where
    F: FnOnce(::http::Request<Body>) -> Fut,
    Fut: Future<Output = core::result::Result<::http::Response<Body>, E>>,
    E: fmt::Debug,
{
    crate::otel::init_once();

    let http_req = match p3_to_http_request(req).await {
        Ok(r) => r,
        Err(e) => return Err(p3::ErrorCode::InternalError(Some(format!("{e}")))),
    };

    let method_str = http_req.method().as_str().to_string();
    let path_str = http_req.uri().path().to_string();
    let span = tracing::info_span!(
        "http.request",
        otel.name = %format!("{} {}", method_str, path_str),
        http.request.method = %method_str,
        url.path = %path_str,
        http.response.status_code = tracing::field::Empty,
    );

    let http_resp = match dispatch(http_req).instrument(span.clone()).await {
        Ok(r) => r,
        Err(e) => {
            tracing::error!(error = ?e, "dispatch failed");
            crate::metrics::flush();
            let error_message = format!("{e:#?}");
            if error_message.contains("request body exceeds") {
                return Err(p3::ErrorCode::HttpRequestBodySize(Some(
                    crate::http::DEFAULT_BODY_BUFFER_LIMIT as u64,
                )));
            }
            if error_message.contains("HttpRequestBodySize") {
                return Err(p3::ErrorCode::HttpRequestBodySize(None));
            }
            return Err(p3::ErrorCode::InternalError(Some(error_message)));
        }
    };

    span.record("http.response.status_code", http_resp.status().as_u16());
    crate::metrics::flush();

    http_response_to_p3(http_resp)
        .await
        .map_err(|e| p3::ErrorCode::InternalError(Some(format!("{e}"))))
}

async fn p3_to_http_request(
    req: p3::Request,
) -> core::result::Result<::http::Request<Body>, ServeError> {
    let method = method_from_p3(req.get_method());

    let scheme_str = match req.get_scheme() {
        Some(p3::Scheme::Http) => "http".to_string(),
        Some(p3::Scheme::Https) => "https".to_string(),
        Some(p3::Scheme::Other(s)) => s,
        None => "http".to_string(),
    };
    let path_with_query = req.get_path_with_query().unwrap_or_else(|| "/".into());

    let wasi_headers = req.get_headers();
    let header_list = wasi_headers.copy_all();
    drop(wasi_headers);

    let authority = match req.get_authority() {
        Some(a) if !a.is_empty() => a,
        _ => header_list
            .iter()
            .find(|(name, _)| name.eq_ignore_ascii_case("host"))
            .and_then(|(_, value)| std::str::from_utf8(value).ok().map(str::to_owned))
            .unwrap_or_default(),
    };

    let uri: ::http::Uri = format!("{scheme_str}://{authority}{path_with_query}")
        .parse()
        .map_err(ServeError::InvalidUri)?;

    let mut builder = ::http::Request::builder().method(method).uri(uri);
    for (name, value) in header_list {
        builder = builder.header(name, value);
    }

    let (trailers_writer, trailers_reader) =
        wit_future::new::<core::result::Result<(), p3::ErrorCode>>(|| Ok(()));
    crate::runtime::spawn(async move {
        drop(trailers_writer);
    });
    let (body_stream, body_trailers) = p3::Request::consume_body(req, trailers_reader);

    builder
        .body(Body::incoming(body_stream, body_trailers))
        .map_err(ServeError::BuildRequest)
}

async fn http_response_to_p3(
    resp: ::http::Response<Body>,
) -> core::result::Result<p3::Response, ServeError> {
    let (parts, body) = resp.into_parts();

    let header_entries: Vec<(String, Vec<u8>)> = parts
        .headers
        .iter()
        .map(|(name, value)| (name.as_str().to_string(), value.as_bytes().to_vec()))
        .collect();
    let fields = p3::Fields::from_list(&header_entries).map_err(ServeError::Headers)?;

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

    let (trailers_writer, trailers_reader) =
        wit_future::new::<core::result::Result<Option<p3::Trailers>, p3::ErrorCode>>(|| Ok(None));
    crate::runtime::spawn(async move {
        drop(trailers_writer);
    });

    let (wasi_resp, _transmit) = p3::Response::new(fields, contents_reader, trailers_reader);
    wasi_resp
        .set_status_code(parts.status.as_u16())
        .map_err(|_| ServeError::InvalidStatusCode)?;

    Ok(wasi_resp)
}

fn method_from_p3(m: p3::Method) -> ::http::Method {
    match m {
        p3::Method::Get => ::http::Method::GET,
        p3::Method::Head => ::http::Method::HEAD,
        p3::Method::Post => ::http::Method::POST,
        p3::Method::Put => ::http::Method::PUT,
        p3::Method::Delete => ::http::Method::DELETE,
        p3::Method::Connect => ::http::Method::CONNECT,
        p3::Method::Options => ::http::Method::OPTIONS,
        p3::Method::Trace => ::http::Method::TRACE,
        p3::Method::Patch => ::http::Method::PATCH,
        p3::Method::Other(s) => {
            ::http::Method::from_bytes(s.as_bytes()).unwrap_or(::http::Method::GET)
        }
    }
}
