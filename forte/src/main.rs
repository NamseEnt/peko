use adapt_cache::AdaptCache;
use anyhow::Result;
use bytes::Bytes;
use fn0::{CodeKind, DeploymentMap, Fn0};
use http_body_util::{BodyExt, combinators::UnsyncBoxBody};
use hyper::Request;
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper_util::rt::TokioIo;
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::net::TcpListener;
use tokio::sync::Mutex;

#[tokio::main]
async fn main() -> Result<()> {
    let mut deployment_map = DeploymentMap::new();
    deployment_map.register_code("backend", CodeKind::Wasm);
    deployment_map.register_code("frontend", CodeKind::Js);

    let cache = SimpleCache::new();

    let fn0 = Arc::new(Fn0::new(cache.clone(), cache, deployment_map));

    let addr = SocketAddr::from(([127, 0, 0, 1], 3000));
    let listener = TcpListener::bind(addr).await?;
    println!("Forte SSR server listening on http://{}", addr);

    loop {
        let (socket, _) = listener.accept().await?;
        let fn0_clone = fn0.clone();

        tokio::spawn(async move {
            let io = TokioIo::new(socket);
            if let Err(err) = http1::Builder::new()
                .serve_connection(
                    io,
                    service_fn(move |req| {
                        let fn0 = fn0_clone.clone();
                        handle_request(req, fn0)
                    }),
                )
                .await
            {
                eprintln!("Failed to serve connection: {}", err);
            }
        });
    }
}

async fn handle_request(
    req: Request<hyper::body::Incoming>,
    fn0: Arc<Fn0<SimpleCache>>,
) -> Result<fn0::Response> {
    let uri = req.uri().clone();
    println!("Received {} {uri}", req.method());

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

#[derive(Clone)]
pub struct SimpleCache {
    memory: Arc<Mutex<HashMap<String, Vec<u8>>>>,
}

impl SimpleCache {
    pub fn new() -> Self {
        Self {
            memory: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    async fn load_file(&self, path: &str) -> Result<Vec<u8>> {
        tokio::fs::read(path).await.map_err(|e| anyhow::anyhow!(e))
    }
}

impl<T: Clone + Send + Sync + 'static, E: Send + 'static> AdaptCache<T, E> for SimpleCache {
    async fn get(
        &self,
        id: &str,
        convert: impl FnOnce(Bytes) -> std::result::Result<(T, usize), E> + Send,
    ) -> std::result::Result<T, adapt_cache::Error<E>> {
        let mut cache = self.memory.lock().await;

        let bytes = if let Some(data) = cache.get(id) {
            Bytes::copy_from_slice(data)
        } else {
            let path = match id {
                "backend" => "../forte-manual/rs/target/wasm32-wasip2/release/backend.wasm",
                "frontend" => "../forte-manual/fe/dist/server.js",
                _ => return Err(adapt_cache::Error::NotFound),
            };

            let mut data = self
                .load_file(path)
                .await
                .map_err(|e| adapt_cache::Error::StorageError(anyhow::anyhow!(e)))?;

            if id == "backend" {
                eprintln!("Compiling backend WASM ({} bytes) to CWASM...", data.len());
                match fn0::compile(&data) {
                    Ok(cwasm) => {
                        eprintln!(
                            "Compilation successful: {} bytes -> {} bytes",
                            data.len(),
                            cwasm.len()
                        );
                        data = cwasm;
                    }
                    Err(e) => {
                        eprintln!("Compilation failed: {:?}", e);
                        return Err(adapt_cache::Error::StorageError(e));
                    }
                }
            }

            cache.insert(id.to_string(), data.clone());
            Bytes::from(data)
        };

        let (converted, _) = convert(bytes).map_err(adapt_cache::Error::ConvertError)?;
        Ok(converted)
    }
}
