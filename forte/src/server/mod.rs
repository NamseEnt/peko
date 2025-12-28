use anyhow::{Context, Result};
use futures_util::{SinkExt, StreamExt};
use http_body_util::{combinators::BoxBody, BodyExt, Full};
use hyper::body::Bytes;
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper::{Request, Response, StatusCode};
use hyper_util::rt::TokioIo;
use std::convert::Infallible;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::net::TcpListener;
use tokio::sync::{broadcast, RwLock};
use tokio_tungstenite::tungstenite::protocol::Message;
use tokio_tungstenite::WebSocketStream;
use tower::ServiceExt;
use tower_http::services::ServeDir;

use crate::runtime::WasmRuntime;
use crate::watcher::RouteInfo;

/// 요청 타입 분류
#[derive(Debug)]
enum RequestType {
    /// 정적 파일 (/client/*, /static/*)
    Static,
    /// CLI 내부 API (/__forte/*)
    Internal,
    /// API 전용 (/api/*) - JSON만 반환
    ApiOnly,
    /// 페이지 요청 - SSR 필요
    Page,
}

/// 컴파일 상태
#[derive(Debug, Clone, serde::Serialize)]
#[serde(tag = "status", rename_all = "lowercase")]
pub enum CompileStatus {
    Ready,
    Compiling { message: String },
    Error { message: String },
}

/// 컴파일 에러 정보
#[derive(Debug, Clone, serde::Serialize)]
pub struct CompileError {
    pub file: String,
    pub line: Option<u32>,
    pub column: Option<u32>,
    pub message: String,
    pub code: Option<String>,
}

/// Forte 내부 상태
#[derive(Clone)]
pub struct ForteState {
    pub compile_status: Arc<RwLock<CompileStatus>>,
    pub routes: Arc<RwLock<Vec<RouteInfo>>>,
    pub compile_errors: Arc<RwLock<Vec<CompileError>>>,
    pub hmr_sender: broadcast::Sender<String>,
}

/// Forte CLI 프록시 서버
pub struct ForteProxy {
    wasm_runtime: Arc<RwLock<WasmRuntime>>,
    static_dir: PathBuf,
    node_url: String,
    state: ForteState,
}

impl ForteProxy {
    pub fn new(
        wasm_runtime: Arc<RwLock<WasmRuntime>>,
        static_dir: PathBuf,
        node_url: String,
        state: ForteState,
    ) -> Self {
        Self {
            wasm_runtime,
            static_dir,
            node_url,
            state,
        }
    }

    /// 프록시 서버 시작
    pub async fn serve(self, addr: SocketAddr) -> Result<()> {
        let listener = TcpListener::bind(addr)
            .await
            .with_context(|| format!("Failed to bind to {}", addr))?;

        println!("✓ Forte proxy server listening on http://{}", addr);

        let proxy = Arc::new(self);

        loop {
            let (stream, _) = listener.accept().await?;
            let io = TokioIo::new(stream);
            let proxy_clone = proxy.clone();

            tokio::task::spawn(async move {
                let service = service_fn(move |req| {
                    let proxy = proxy_clone.clone();
                    async move { proxy.handle(req).await }
                });

                if let Err(err) = http1::Builder::new()
                    .serve_connection(io, service)
                    .await
                {
                    eprintln!("Error serving connection: {:?}", err);
                }
            });
        }
    }

    /// HTTP 요청 처리
    async fn handle(
        &self,
        req: Request<hyper::body::Incoming>,
    ) -> Result<Response<BoxBody<Bytes, std::io::Error>>> {
        let path = req.uri().path();
        let request_type = self.classify_request(path);

        match request_type {
            RequestType::Static => self.handle_static(req).await,
            RequestType::Internal => self.handle_internal(req).await,
            RequestType::ApiOnly => self.handle_api_only(req).await,
            RequestType::Page => self.handle_page(req).await,
        }
    }

    /// 요청 타입 분류
    fn classify_request(&self, path: &str) -> RequestType {
        if path.starts_with("/client/") || path.starts_with("/static/") {
            RequestType::Static
        } else if path.starts_with("/__forte/") {
            RequestType::Internal
        } else if path.starts_with("/api/") || path.starts_with("/__wasm/") {
            RequestType::ApiOnly
        } else {
            RequestType::Page
        }
    }

    /// 정적 파일 서빙
    async fn handle_static(
        &self,
        req: Request<hyper::body::Incoming>,
    ) -> Result<Response<BoxBody<Bytes, std::io::Error>>> {
        let serve_dir = ServeDir::new(&self.static_dir);

        let response = serve_dir
            .oneshot(req)
            .await
            .map_err(|e| anyhow::anyhow!("Static file error: {}", e))?;

        // Convert tower-http body to our BoxBody by collecting into bytes
        let (parts, body) = response.into_parts();
        let bytes = body
            .collect()
            .await
            .map_err(|e| std::io::Error::other(format!("Body collection error: {}", e)))?
            .to_bytes();

        let new_body = Full::new(bytes)
            .map_err(|_: std::convert::Infallible| std::io::Error::other("never"))
            .boxed();

        Ok(Response::from_parts(parts, new_body))
    }

    /// CLI 내부 API 처리
    async fn handle_internal(
        &self,
        req: Request<hyper::body::Incoming>,
    ) -> Result<Response<BoxBody<Bytes, std::io::Error>>> {
        let path = req.uri().path();

        match path {
            "/__forte/status" => self.handle_status().await,
            "/__forte/routes" => self.handle_routes().await,
            "/__forte/errors" => self.handle_errors().await,
            "/__forte/hmr" => self.handle_hmr(req).await,
            _ => Ok(Response::builder()
                .status(StatusCode::NOT_FOUND)
                .body(
                    Full::new(Bytes::from("Not Found"))
                        .map_err(|_: Infallible| std::io::Error::other("never"))
                        .boxed(),
                )
                .unwrap()),
        }
    }

    /// 컴파일 상태 반환
    async fn handle_status(&self) -> Result<Response<BoxBody<Bytes, std::io::Error>>> {
        let status = self.state.compile_status.read().await;
        let json = serde_json::to_string(&*status)?;

        Ok(Response::builder()
            .status(StatusCode::OK)
            .header("Content-Type", "application/json")
            .body(
                Full::new(Bytes::from(json))
                    .map_err(|_: Infallible| std::io::Error::other("never"))
                    .boxed(),
            )
            .unwrap())
    }

    /// 라우트 목록 반환
    async fn handle_routes(&self) -> Result<Response<BoxBody<Bytes, std::io::Error>>> {
        let routes = self.state.routes.read().await;

        // 간단한 라우트 정보만 반환
        let route_list: Vec<_> = routes
            .iter()
            .map(|r| {
                serde_json::json!({
                    "path": r.props_path.display().to_string(),
                    "has_get": r.has_get_props,
                    "has_post": r.has_action_input,
                })
            })
            .collect();

        let json = serde_json::to_string(&route_list)?;

        Ok(Response::builder()
            .status(StatusCode::OK)
            .header("Content-Type", "application/json")
            .body(
                Full::new(Bytes::from(json))
                    .map_err(|_: Infallible| std::io::Error::other("never"))
                    .boxed(),
            )
            .unwrap())
    }

    /// 컴파일 에러 목록 반환
    async fn handle_errors(&self) -> Result<Response<BoxBody<Bytes, std::io::Error>>> {
        let errors = self.state.compile_errors.read().await;
        let json = serde_json::to_string(&*errors)?;

        Ok(Response::builder()
            .status(StatusCode::OK)
            .header("Content-Type", "application/json")
            .body(
                Full::new(Bytes::from(json))
                    .map_err(|_: Infallible| std::io::Error::other("never"))
                    .boxed(),
            )
            .unwrap())
    }

    /// HMR WebSocket 연결
    async fn handle_hmr(
        &self,
        mut req: Request<hyper::body::Incoming>,
    ) -> Result<Response<BoxBody<Bytes, std::io::Error>>> {
        let headers = req.headers();

        // Check if this is a WebSocket upgrade request
        let is_upgrade = headers
            .get("upgrade")
            .and_then(|v| v.to_str().ok())
            .map(|v| v.eq_ignore_ascii_case("websocket"))
            .unwrap_or(false);

        if !is_upgrade {
            return Ok(Response::builder()
                .status(StatusCode::BAD_REQUEST)
                .body(
                    Full::new(Bytes::from("Expected WebSocket upgrade"))
                        .map_err(|_: Infallible| std::io::Error::other("never"))
                        .boxed(),
                )
                .unwrap());
        }

        // Get WebSocket key and generate accept key
        let key = headers
            .get("sec-websocket-key")
            .and_then(|v| v.to_str().ok())
            .context("Missing Sec-WebSocket-Key")?;

        use sha1::{Digest, Sha1};
        use base64::{Engine, engine::general_purpose::STANDARD};

        let mut hasher = Sha1::new();
        hasher.update(key.as_bytes());
        hasher.update(b"258EAFA5-E914-47DA-95CA-C5AB0DC85B11");
        let accept = STANDARD.encode(hasher.finalize());

        // Subscribe to HMR events
        let hmr_rx = self.state.hmr_sender.subscribe();

        // Spawn WebSocket handler task
        tokio::spawn(async move {
            match hyper::upgrade::on(&mut req).await {
                Ok(upgraded) => {
                    let ws = WebSocketStream::from_raw_socket(
                        TokioIo::new(upgraded),
                        tokio_tungstenite::tungstenite::protocol::Role::Server,
                        None,
                    )
                    .await;

                    if let Err(e) = handle_websocket(ws, hmr_rx).await {
                        eprintln!("WebSocket error: {}", e);
                    }
                }
                Err(e) => eprintln!("WebSocket upgrade error: {}", e),
            }
        });

        // Return upgrade response
        Ok(Response::builder()
            .status(StatusCode::SWITCHING_PROTOCOLS)
            .header("Upgrade", "websocket")
            .header("Connection", "Upgrade")
            .header("Sec-WebSocket-Accept", accept)
            .body(
                Full::new(Bytes::new())
                    .map_err(|_: Infallible| std::io::Error::other("never"))
                    .boxed(),
            )
            .unwrap())
    }

    /// API 전용 요청 처리 (WASM만 호출, JSON 응답)
    async fn handle_api_only(
        &self,
        mut req: Request<hyper::body::Incoming>,
    ) -> Result<Response<BoxBody<Bytes, std::io::Error>>> {
        // Strip /__wasm/ prefix if present (for SSR server calls)
        let path = req.uri().path();
        if path.starts_with("/__wasm/") {
            let new_path = path.strip_prefix("/__wasm").unwrap();
            let new_uri = format!("{}{}", new_path,
                req.uri().query().map(|q| format!("?{}", q)).unwrap_or_default());
            *req.uri_mut() = new_uri.parse()
                .context("Failed to parse URI")?;
        }

        // WASM 런타임에서 직접 처리
        let runtime = self.wasm_runtime.read().await;
        let wasm_response = runtime.handle_request(req).await?;

        Ok(wasm_response.map(|body| {
            body.map_err(|_| std::io::Error::other("body error"))
                .boxed()
        }))
    }

    /// 페이지 요청 처리 (Forward to SSR server)
    async fn handle_page(
        &self,
        req: Request<hyper::body::Incoming>,
    ) -> Result<Response<BoxBody<Bytes, std::io::Error>>> {
        let path = req.uri().path().to_string();
        let method = req.method().clone();

        // Forward request to SSR server
        let ssr_url = format!("{}{}", self.node_url, path);

        let client = reqwest::Client::new();
        let ssr_response = client
            .request(method, &ssr_url)
            .send()
            .await;

        match ssr_response {
            Ok(resp) => {
                let status = resp.status();
                let content_type = resp.headers().get("content-type")
                    .and_then(|v| v.to_str().ok())
                    .unwrap_or("text/html; charset=utf-8")
                    .to_string(); // Clone before consuming resp

                let body_text = resp.text().await
                    .context("Failed to read SSR response")?;

                let response = Response::builder()
                    .status(status)
                    .header("Content-Type", content_type)
                    .body(
                        Full::new(Bytes::from(body_text))
                            .map_err(|_: std::convert::Infallible| std::io::Error::other("never"))
                            .boxed(),
                    )
                    .unwrap();

                Ok(response)
            }
            Err(e) => {
                let response = Response::builder()
                    .status(StatusCode::BAD_GATEWAY)
                    .body(
                        Full::new(Bytes::from(format!("SSR server error: {}", e)))
                            .map_err(|_: std::convert::Infallible| std::io::Error::other("never"))
                            .boxed(),
                    )
                    .unwrap();

                Ok(response)
            }
        }
    }
}

/// WebSocket 연결을 처리하고 HMR 이벤트를 브로드캐스트
async fn handle_websocket<S>(
    mut ws: WebSocketStream<S>,
    mut hmr_rx: broadcast::Receiver<String>,
) -> Result<()>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    // Send initial connection message
    ws.send(Message::Text(
        serde_json::json!({
            "type": "connected",
            "message": "HMR WebSocket connected"
        })
        .to_string(),
    ))
    .await?;

    loop {
        tokio::select! {
            // Receive HMR events from broadcast channel
            event = hmr_rx.recv() => {
                match event {
                    Ok(message) => {
                        // Send reload message to client
                        if let Err(e) = ws.send(Message::Text(message)).await {
                            eprintln!("Failed to send HMR message: {}", e);
                            break;
                        }
                    }
                    Err(broadcast::error::RecvError::Lagged(n)) => {
                        eprintln!("HMR receiver lagged by {} messages", n);
                    }
                    Err(broadcast::error::RecvError::Closed) => {
                        break;
                    }
                }
            }
            // Handle incoming WebSocket messages (ping/pong)
            msg = ws.next() => {
                match msg {
                    Some(Ok(Message::Close(_))) => {
                        break;
                    }
                    Some(Ok(Message::Ping(data))) => {
                        if let Err(e) = ws.send(Message::Pong(data)).await {
                            eprintln!("Failed to send pong: {}", e);
                            break;
                        }
                    }
                    Some(Err(e)) => {
                        eprintln!("WebSocket error: {}", e);
                        break;
                    }
                    None => break,
                    _ => {}
                }
            }
        }
    }

    Ok(())
}
