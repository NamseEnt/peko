mod cache;

pub use cache::SimpleCache;

use anyhow::Result;
use fn0::{CodeKind, DeploymentMap, Fn0};
use http_body_util::{BodyExt, Full, combinators::UnsyncBoxBody};
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper::{Request, Response, StatusCode};
use hyper_util::rt::TokioIo;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::net::TcpListener;

pub struct ServerConfig {
    pub port: u16,
    pub backend_path: String,
    pub frontend_path: String,
    pub public_dir: PathBuf,
}

pub struct ServerHandle {
    pub cache: SimpleCache,
}

pub async fn run(config: ServerConfig) -> Result<ServerHandle> {
    let mut deployment_map = DeploymentMap::new();
    deployment_map.register_code("backend", CodeKind::Wasm);
    deployment_map.register_code("frontend", CodeKind::Js);

    let cache = SimpleCache::new(config.backend_path.clone(), config.frontend_path.clone());
    let handle = ServerHandle {
        cache: cache.clone(),
    };

    let fn0 = Arc::new(Fn0::new(cache.clone(), cache, deployment_map));
    let public_dir = Arc::new(config.public_dir);

    let addr = SocketAddr::from(([127, 0, 0, 1], config.port));
    let listener = TcpListener::bind(addr).await?;
    println!("Forte SSR server listening on http://{}", addr);

    tokio::spawn(async move {
        loop {
            let (socket, _) = match listener.accept().await {
                Ok(s) => s,
                Err(e) => {
                    eprintln!("Failed to accept connection: {}", e);
                    continue;
                }
            };
            let fn0_clone = fn0.clone();
            let public_dir_clone = public_dir.clone();

            tokio::spawn(async move {
                let io = TokioIo::new(socket);
                if let Err(err) = http1::Builder::new()
                    .serve_connection(
                        io,
                        service_fn(move |req| {
                            let fn0 = fn0_clone.clone();
                            let public_dir = public_dir_clone.clone();
                            handle_request(req, fn0, public_dir)
                        }),
                    )
                    .await
                {
                    eprintln!("Failed to serve connection: {}", err);
                }
            });
        }
    });

    Ok(handle)
}

async fn handle_request(
    req: Request<hyper::body::Incoming>,
    fn0: Arc<Fn0<SimpleCache>>,
    public_dir: Arc<PathBuf>,
) -> Result<fn0::Response> {
    let uri = req.uri().clone();
    let path = uri.path();
    println!("Received {} {path}", req.method());

    if let Some(static_response) = try_serve_static(&public_dir, path).await {
        return Ok(static_response);
    }

    let backend_response = match fn0
        .run(
            "backend",
            req.map(|body| {
                UnsyncBoxBody::new(body)
                    .map_err(|e| anyhow::anyhow!(e))
                    .boxed_unsync()
            }),
        )
        .await
    {
        Ok(resp) => resp,
        Err(e) => {
            eprintln!("Backend error: {:?}", e);
            return Err(anyhow::anyhow!("Backend error: {:?}", e));
        }
    };

    let backend_status = backend_response.status();

    println!("Backend response status: {}", backend_status);

    if !backend_status.is_success() {
        let (parts, body) = backend_response.into_parts();
        let body_bytes = body.collect().await?.to_bytes();
        let body_str = String::from_utf8_lossy(&body_bytes);
        eprintln!("Backend error response body: {}", body_str);

        return Ok(fn0::Response::from_parts(
            parts,
            UnsyncBoxBody::new(body_str.to_string())
                .map_err(|e| anyhow::anyhow!(e))
                .boxed_unsync(),
        ));
    }

    println!("Preparing frontend request with backend response body");
    let frontend_request = Request::builder()
        .method("POST")
        .uri(uri)
        .header("content-type", "application/json")
        .body(backend_response.into_body())?;

    println!("Calling frontend (ski::run)");
    match fn0.run("frontend", frontend_request).await {
        Ok(resp) => {
            println!("Frontend response status: {}", resp.status());
            Ok(resp)
        }
        Err(e) => {
            eprintln!("Frontend error: {:?}", e);
            Err(e)
        }
    }
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
            println!("[static] Serving {} ({})", file_path.display(), content_type);

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

fn get_content_type(path: &PathBuf) -> &'static str {
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
