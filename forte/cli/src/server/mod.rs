mod cache;
pub mod vite_dev;
pub mod websocket;

use anyhow::Result;
pub use cache::SimpleCache;
use fn0::{
    CodeExecutor, ExecutionContext, ObjectStorageHijack, PublicStorageHijack, QueueHijack,
    StaticPageCacheHijack,
};
use http_body_util::{BodyExt, Full, combinators::UnsyncBoxBody};
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper::{Method, Request, Response, StatusCode};
use hyper_util::rt::TokioIo;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
#[cfg(unix)]
use tokio::net::UnixStream;

pub struct ServerConfig {
    pub port: u16,
    pub wasm_path: String,
    pub js_path: String,
    pub public_dir: PathBuf,
    pub vite_socket_path: Option<PathBuf>,
    pub env_vars: Vec<(String, String)>,
    pub queue_hijack: Option<Arc<QueueHijack>>,
    pub object_storage_hijack: Option<Arc<ObjectStorageHijack>>,
    pub public_storage_hijack: Option<Arc<PublicStorageHijack>>,
    pub static_page_cache_hijack: Option<Arc<StaticPageCacheHijack>>,
    pub websocket_hijack: Option<Arc<fn0::WebSocketHijack>>,
}

pub struct ServerHandle {
    pub ctx: Arc<ExecutionContext<SimpleCache>>,
    pub executor: std::rc::Rc<CodeExecutor<SimpleCache>>,
    pub websocket_service: Arc<websocket::LocalWebSocketService>,
}

pub const DEV_CODE_ID: &str = "app";

pub async fn run(config: ServerConfig) -> Result<ServerHandle> {
    let engine = fn0::build_engine()?;
    fn0::spawn_epoch_ticker(engine.clone());
    let linker = fn0::build_linker(&engine);

    let cache = SimpleCache::new(
        config.wasm_path.clone(),
        config.js_path.clone(),
        engine.clone(),
        linker.clone(),
        config.env_vars,
    );

    let vite_socket_path = config.vite_socket_path.map(Arc::new);
    let websocket_hijack = config.websocket_hijack.clone();

    let mut ctx = ExecutionContext::new(engine, linker, cache);
    if let Some(hijack) = config.queue_hijack {
        ctx = ctx.with_queue_hijack(hijack);
    }
    if let Some(hijack) = config.public_storage_hijack {
        ctx = ctx.with_public_storage_hijack(hijack);
    }
    if let Some(hijack) = config.object_storage_hijack {
        ctx = ctx.with_object_storage_hijack(hijack);
    }
    if let Some(hijack) = config.static_page_cache_hijack {
        ctx = ctx.with_static_page_cache_hijack(hijack);
    }
    if let Some(hijack) = websocket_hijack.clone() {
        ctx = ctx.with_websocket_hijack(hijack);
    }
    let ctx = Arc::new(ctx);
    let executor = std::rc::Rc::new(CodeExecutor::new(ctx.clone()));
    let websocket_service =
        websocket::LocalWebSocketService::start(executor.clone(), websocket_hijack);

    let public_dir = Arc::new(config.public_dir);

    let handle = ServerHandle {
        ctx: ctx.clone(),
        executor: executor.clone(),
        websocket_service: websocket_service.clone(),
    };

    let addr = SocketAddr::from(([0, 0, 0, 0], config.port));
    let listener = TcpListener::bind(addr).await?;
    println!("Listening on http://localhost:{}", config.port);

    tokio::task::spawn_local(async move {
        loop {
            let (socket, peer_addr) = match listener.accept().await {
                Ok(s) => s,
                Err(e) => {
                    eprintln!("Failed to accept connection: {}", e);
                    continue;
                }
            };
            let is_loopback = peer_addr.ip().is_loopback();
            let executor_clone = executor.clone();
            let public_dir_clone = public_dir.clone();
            let vite_socket_path_clone = vite_socket_path.clone();
            let websocket_service_clone = websocket_service.clone();

            tokio::task::spawn_local(async move {
                let io = TokioIo::new(socket);
                let conn = http1::Builder::new().serve_connection(
                    io,
                    service_fn(move |req| {
                        let executor = executor_clone.clone();
                        let public_dir = public_dir_clone.clone();
                        let vite_socket = vite_socket_path_clone.clone();
                        let websocket_service = websocket_service_clone.clone();
                        handle_request(
                            req,
                            executor,
                            public_dir,
                            vite_socket,
                            websocket_service,
                            peer_addr,
                            is_loopback,
                        )
                    }),
                );
                if let Err(err) = conn.with_upgrades().await {
                    eprintln!("Failed to serve connection: {}", err);
                }
            });
        }
    });

    Ok(handle)
}

async fn handle_request(
    mut req: Request<hyper::body::Incoming>,
    executor: std::rc::Rc<CodeExecutor<SimpleCache>>,
    public_dir: Arc<PathBuf>,
    vite_socket_path: Option<Arc<PathBuf>>,
    websocket_service: Arc<websocket::LocalWebSocketService>,
    peer_addr: SocketAddr,
    is_loopback: bool,
) -> Result<fn0::Response> {
    let uri = req.uri().clone();
    let path = uri.path();
    let path_with_query = uri.path_and_query().map(|pq| pq.as_str()).unwrap_or(path);

    if (path == "/ws" || path.starts_with("/ws/"))
        && fastwebsockets::upgrade::is_upgrade_request(&req)
    {
        return websocket_service
            .handle_inbound(req, executor, DEV_CODE_ID, Some(peer_addr))
            .await;
    }

    if path.starts_with("/__fn0_queue_task/") {
        return Ok(Response::builder()
            .status(StatusCode::FORBIDDEN)
            .body(
                Full::new(bytes::Bytes::from("Forbidden"))
                    .map_err(|e| anyhow::anyhow!("{e}"))
                    .boxed_unsync(),
            )
            .unwrap());
    }

    if let Some(encoded_key) = path.strip_prefix("/__fn0_object_storage/") {
        let encoded_key = encoded_key.to_string();
        return serve_object_storage(req, &encoded_key, &executor).await;
    }

    // Public objects are world-readable in production; in dev the same URL is
    // served straight off the local store with no signature.
    if let Some(encoded_key) = path.strip_prefix("/__fn0_public_storage/") {
        let encoded_key = encoded_key.to_string();
        return serve_public_storage(req, &encoded_key, &executor).await;
    }

    {
        let headers = req.headers_mut();
        headers.remove("x-fn0-admin");
        headers.remove("x-fn0-admin-github-login");
        headers.remove("x-fn0-admin-task");
    }

    if path.starts_with("/__forte_admin/") {
        if !is_loopback {
            return Ok(Response::builder()
                .status(StatusCode::FORBIDDEN)
                .body(
                    Full::new(bytes::Bytes::from("Forbidden (admin only on loopback)"))
                        .map_err(|e| anyhow::anyhow!("{e}"))
                        .boxed_unsync(),
                )
                .unwrap());
        }
        req.headers_mut()
            .insert("x-fn0-admin", "true".parse().unwrap());
        req.headers_mut()
            .insert("x-fn0-admin-github-login", "local-dev".parse().unwrap());
        if let Some(task_name) = path.strip_prefix("/__forte_admin/")
            && let Ok(v) = hyper::header::HeaderValue::from_str(task_name)
        {
            req.headers_mut().insert("x-fn0-admin-task", v);
        }
    }

    if let Some(hook_name) = path.strip_prefix("/__self_invoke/")
        && req.headers().contains_key("x-forte-prefetch-miss")
    {
        eprintln!(
            "[forte] hook '{hook_name}' fell back to client fetch \
             (not pre-fetched during SSR — extra round-trip, slower TTI). \
             Likely cause: caller component is gated by useEffect-only state, \
             or rendered conditionally outside the SSR tree."
        );
    }

    if should_proxy_to_vite(path)
        && let Some(socket_path) = &vite_socket_path
    {
        return proxy_to_vite_uds(socket_path, path_with_query).await;
    }

    if vite_socket_path.is_some()
        && let Some(static_response) = try_serve_static(&public_dir, path).await
    {
        return Ok(static_response);
    }

    let mapped_req = req.map(|body| {
        UnsyncBoxBody::new(body)
            .map_err(|e| anyhow::anyhow!(e))
            .boxed_unsync()
    });

    if let Some(socket_path) = &vite_socket_path {
        let original_headers = mapped_req.headers().clone();

        let backend_response = match executor.run_backend_only("app", mapped_req).await {
            Ok(resp) => resp,
            Err(e) => {
                eprintln!("Backend error: {:?}", e);
                return Err(anyhow::anyhow!("Backend error: {:?}", e));
            }
        };

        let backend_status = backend_response.status();

        if backend_status.is_redirection() {
            return Ok(backend_response);
        }

        if backend_status.is_client_error() || backend_status.is_server_error() {
            let (parts, body) = backend_response.into_parts();
            let body_bytes = body.collect().await?.to_bytes();
            let body_str = String::from_utf8_lossy(&body_bytes);
            if backend_status != StatusCode::NOT_FOUND {
                eprintln!("Backend error: {} {} - {}", backend_status, path, body_str);
            }

            return Ok(fn0::Response::from_parts(
                parts,
                UnsyncBoxBody::new(body_str.to_string())
                    .map_err(|e| anyhow::anyhow!(e))
                    .boxed_unsync(),
            ));
        }

        if path.starts_with("/__self_invoke/")
            || path.starts_with("/__forte_action/")
            || path.starts_with("/__forte_admin/")
            || path.starts_with("/api/")
        {
            return Ok(backend_response);
        }

        let backend_set_cookies: Vec<_> = backend_response
            .headers()
            .get_all(http::header::SET_COOKIE)
            .iter()
            .cloned()
            .collect();

        let (_, body) = backend_response.into_parts();
        let body_bytes = body.collect().await?.to_bytes();
        let props: serde_json::Value = serde_json::from_slice(&body_bytes)?;

        let cookie_header = original_headers
            .get(http::header::COOKIE)
            .and_then(|v| v.to_str().ok())
            .map(|s| s.to_string());

        let host = original_headers
            .get(http::header::HOST)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("localhost");
        let full_url = format!("http://{}{}", host, uri);
        let mut ssr_response =
            call_vite_ssr_uds(socket_path, &full_url, props, cookie_header).await?;

        for cookie_value in backend_set_cookies {
            ssr_response
                .headers_mut()
                .append(http::header::SET_COOKIE, cookie_value);
        }

        return Ok(ssr_response);
    }

    match executor.run("app", "", mapped_req, None).await {
        Ok(resp) => Ok(resp),
        Err(e) => {
            eprintln!("Request error: {:?}", e);
            Err(anyhow::anyhow!("Request error: {:?}", e))
        }
    }
}

/// Serves the `forte dev` public-object route. Read-only: writes go through
/// the hijack like they do in production, so the dev path cannot drift into
/// accepting uploads the real CDN origin would never take.
async fn serve_public_storage(
    req: Request<hyper::body::Incoming>,
    encoded_key: &str,
    executor: &CodeExecutor<SimpleCache>,
) -> Result<fn0::Response> {
    let key = percent_decode(encoded_key);
    let Some(hijack) = executor.context().public_storage_hijack().cloned() else {
        return Ok(text_response(
            StatusCode::NOT_FOUND,
            "public storage not configured",
        ));
    };
    if !matches!(*req.method(), Method::GET | Method::HEAD) {
        return Ok(text_response(StatusCode::METHOD_NOT_ALLOWED, ""));
    }
    match hijack.dev_read(&key) {
        fn0::DevReadResult::Found { data, content_type } => {
            let mut builder = Response::builder().status(StatusCode::OK);
            if let Some(content_type) = content_type {
                builder = builder.header(http::header::CONTENT_TYPE, content_type);
            }
            Ok(builder
                .body(
                    Full::new(bytes::Bytes::from(data))
                        .map_err(|e| anyhow::anyhow!("{e}"))
                        .boxed_unsync(),
                )
                .unwrap())
        }
        _ => Ok(text_response(StatusCode::NOT_FOUND, "Not Found")),
    }
}

/// Serves the `forte dev` object-storage route that presigned URLs point at.
/// Reads/writes the local store via the object-storage hijack.
async fn serve_object_storage(
    req: Request<hyper::body::Incoming>,
    encoded_key: &str,
    executor: &CodeExecutor<SimpleCache>,
) -> Result<fn0::Response> {
    let key = percent_decode(encoded_key);
    let Some(hijack) = executor.context().object_storage_hijack().cloned() else {
        return Ok(text_response(
            StatusCode::NOT_FOUND,
            "object storage not configured",
        ));
    };
    match *req.method() {
        Method::GET | Method::HEAD => match hijack.dev_read(&key) {
            fn0::DevReadResult::Found { data, content_type } => {
                let mut builder = Response::builder().status(StatusCode::OK);
                if let Some(content_type) = content_type {
                    builder = builder.header(http::header::CONTENT_TYPE, content_type);
                }
                Ok(builder
                    .body(
                        Full::new(bytes::Bytes::from(data))
                            .map_err(|e| anyhow::anyhow!("{e}"))
                            .boxed_unsync(),
                    )
                    .unwrap())
            }
            _ => Ok(text_response(StatusCode::NOT_FOUND, "Not Found")),
        },
        Method::PUT => {
            let content_type = req
                .headers()
                .get(http::header::CONTENT_TYPE)
                .and_then(|v| v.to_str().ok())
                .map(|s| s.to_string());
            let data = req.into_body().collect().await?.to_bytes();
            match hijack.dev_write(&key, &data, content_type.as_deref()) {
                Ok(()) => Ok(text_response(StatusCode::OK, "")),
                Err(e) => Ok(text_response(StatusCode::BAD_REQUEST, &e)),
            }
        }
        _ => Ok(text_response(StatusCode::METHOD_NOT_ALLOWED, "")),
    }
}

fn text_response(status: StatusCode, body: &str) -> fn0::Response {
    Response::builder()
        .status(status)
        .body(
            Full::new(bytes::Bytes::from(body.to_string()))
                .map_err(|e| anyhow::anyhow!("{e}"))
                .boxed_unsync(),
        )
        .unwrap()
}

fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && i + 3 <= bytes.len()
            && let Ok(byte) = u8::from_str_radix(&s[i + 1..i + 3], 16)
        {
            out.push(byte);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

async fn try_serve_static(public_dir: &PathBuf, path: &str) -> Option<fn0::Response> {
    let file_path = if path == "/favicon.ico" {
        public_dir.join("favicon.ico")
    } else if path.starts_with("/public/") {
        let relative_path = path.strip_prefix("/public/").unwrap_or(path);
        public_dir.join(relative_path)
    } else {
        return None;
    };

    if !file_path.starts_with(public_dir) {
        return Some(
            Response::builder()
                .status(StatusCode::FORBIDDEN)
                .body(
                    Full::new(bytes::Bytes::from("Forbidden"))
                        .map_err(|e| anyhow::anyhow!("{e}"))
                        .boxed_unsync(),
                )
                .unwrap(),
        );
    }

    match tokio::fs::read(&file_path).await {
        Ok(contents) => {
            let content_type = get_content_type(&file_path);
            Some(
                Response::builder()
                    .status(StatusCode::OK)
                    .header("content-type", content_type)
                    .header("cache-control", "public, max-age=3600")
                    .body(
                        Full::new(bytes::Bytes::from(contents))
                            .map_err(|e| anyhow::anyhow!("{e}"))
                            .boxed_unsync(),
                    )
                    .unwrap(),
            )
        }
        Err(_) => Some(
            Response::builder()
                .status(StatusCode::NOT_FOUND)
                .body(
                    Full::new(bytes::Bytes::from("Not Found"))
                        .map_err(|e| anyhow::anyhow!("{e}"))
                        .boxed_unsync(),
                )
                .unwrap(),
        ),
    }
}

fn get_content_type(path: &std::path::Path) -> &'static str {
    match path.extension().and_then(|e| e.to_str()) {
        Some("html") => "text/html; charset=utf-8",
        Some("css") => "text/css; charset=utf-8",
        Some("js") => "application/javascript; charset=utf-8",
        Some("json") => "application/json; charset=utf-8",
        Some("png") => "image/png",
        Some("jpg") | Some("jpeg") => "image/jpeg",
        Some("gif") => "image/gif",
        Some("svg") => "image/svg+xml",
        Some("ico") => "image/x-icon",
        Some("webp") => "image/webp",
        Some("woff") => "font/woff",
        Some("woff2") => "font/woff2",
        Some("ttf") => "font/ttf",
        Some("otf") => "font/otf",
        Some("eot") => "application/vnd.ms-fontobject",
        Some("txt") => "text/plain; charset=utf-8",
        Some("xml") => "application/xml; charset=utf-8",
        Some("pdf") => "application/pdf",
        Some("mp4") => "video/mp4",
        Some("webm") => "video/webm",
        Some("mp3") => "audio/mpeg",
        Some("wav") => "audio/wav",
        _ => "application/octet-stream",
    }
}

fn should_proxy_to_vite(path: &str) -> bool {
    path.starts_with("/src/")
        || path.starts_with("/.forte/")
        || path.starts_with("/@vite/")
        || path.starts_with("/@id/")
        || path.starts_with("/@fs/")
        || path.starts_with("/__vite")
        || path.starts_with("/node_modules/")
        || path == "/@react-refresh"
}

#[cfg(unix)]
async fn proxy_to_vite_uds(socket_path: &Path, path: &str) -> Result<fn0::Response> {
    let mut stream = UnixStream::connect(socket_path).await?;

    let request = format!(
        "GET {} HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n",
        path
    );
    stream.write_all(request.as_bytes()).await?;

    let mut response_bytes = Vec::new();
    stream.read_to_end(&mut response_bytes).await?;

    parse_http_response(&response_bytes)
}

#[cfg(unix)]
async fn call_vite_ssr_uds(
    socket_path: &Path,
    url: &str,
    props: serde_json::Value,
    cookie: Option<String>,
) -> Result<fn0::Response> {
    let mut stream = UnixStream::connect(socket_path).await?;

    let body = serde_json::json!({
        "url": url,
        "props": props,
        "cookie": cookie,
    })
    .to_string();

    let request = format!(
        "POST /__ssr_render HTTP/1.1\r\nHost: localhost\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        body.len(),
        body
    );
    stream.write_all(request.as_bytes()).await?;

    let mut response_bytes = Vec::new();
    stream.read_to_end(&mut response_bytes).await?;

    parse_http_response(&response_bytes)
}

fn parse_http_response(response_bytes: &[u8]) -> Result<fn0::Response> {
    let response_str = String::from_utf8_lossy(response_bytes);

    let header_end = response_str
        .find("\r\n\r\n")
        .ok_or_else(|| anyhow::anyhow!("Invalid HTTP response"))?;

    let header_part = &response_str[..header_end];
    let body_start = header_end + 4;
    let raw_body = &response_bytes[body_start..];

    let mut lines = header_part.lines();
    let status_line = lines
        .next()
        .ok_or_else(|| anyhow::anyhow!("Missing status line"))?;

    let status_code: u16 = status_line
        .split_whitespace()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(500);

    let mut builder = Response::builder().status(status_code);
    let mut is_chunked = false;

    for line in lines {
        if let Some((key, value)) = line.split_once(": ") {
            let key_lower = key.to_lowercase();
            if key_lower == "transfer-encoding" && value.to_lowercase().contains("chunked") {
                is_chunked = true;
            }
            if key_lower != "transfer-encoding" && key_lower != "content-length" {
                builder = builder.header(key, value);
            }
        }
    }

    let body = if is_chunked || looks_like_chunked(raw_body) {
        decode_chunked_body(raw_body)
    } else {
        raw_body.to_vec()
    };

    let body_bytes = bytes::Bytes::from(body);
    let body_len = body_bytes.len();
    Ok(builder
        .header(hyper::header::CONTENT_LENGTH, body_len.to_string())
        .body(
            Full::new(body_bytes)
                .map_err(|e| anyhow::anyhow!("{e}"))
                .boxed_unsync(),
        )?)
}

fn looks_like_chunked(data: &[u8]) -> bool {
    let crlf_pos = data.windows(2).position(|w| w == b"\r\n");
    if let Some(pos) = crlf_pos
        && pos > 0
        && pos <= 8
    {
        let potential_size = &data[..pos];
        if let Ok(s) = std::str::from_utf8(potential_size) {
            return usize::from_str_radix(s.trim(), 16).is_ok();
        }
    }
    false
}

fn decode_chunked_body(data: &[u8]) -> Vec<u8> {
    let mut result = Vec::new();
    let mut pos = 0;

    while pos < data.len() {
        let chunk_size_end = data[pos..]
            .windows(2)
            .position(|w| w == b"\r\n")
            .map(|p| pos + p);

        let Some(chunk_size_end) = chunk_size_end else {
            break;
        };

        let chunk_size_str = String::from_utf8_lossy(&data[pos..chunk_size_end]);
        let chunk_size = usize::from_str_radix(chunk_size_str.trim(), 16).unwrap_or(0);

        if chunk_size == 0 {
            break;
        }

        let chunk_start = chunk_size_end + 2;
        let chunk_end = chunk_start + chunk_size;

        if chunk_end <= data.len() {
            result.extend_from_slice(&data[chunk_start..chunk_end]);
        }

        pos = chunk_end + 2;
    }

    result
}
