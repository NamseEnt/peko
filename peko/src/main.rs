mod execute;
mod list_neighbors;
mod metrics;
mod on_fn_call;
mod warm_up_map;

use crate::execute::{Job, Response};
use bytes::Bytes;
use http_body_util::BodyExt;
use hyper::server::conn::http1;
use hyper::{Request, StatusCode};
use hyper_util::rt::TokioIo;
use hyper_util::service::TowerToHyperService;
use measure_cpu_time::SystemClock;
use std::convert::Infallible;
use std::net::SocketAddr;
use std::time::Duration;
use tokio::net::TcpListener;
use tower::ServiceBuilder;
use tower_http::timeout::TimeoutLayer;
use wasmtime_wasi_http::bindings::http::types::ErrorCode;
use wasmtime_wasi_http::body::HyperOutgoingBody;

fn main() {
    tokio::runtime::Runtime::new()
        .unwrap()
        .block_on(real_main());
}

async fn real_main() {
    let port = 3000;
    let addr = SocketAddr::from(([127, 0, 0, 1], port));

    let listener = TcpListener::bind(addr)
        .await
        .unwrap_or_else(|_| panic!("Failed to bind listener on port {port}"));

    let (job_tx, job_rx) = tokio::sync::mpsc::channel(10 * 1024);
    let metrics_tx = metrics::MetricsTx::new();

    tokio::spawn({
        let metrics_tx = metrics_tx.clone();
        async move {
            execute::Executor::new(
                adapt_cache::fs::FsAdaptCache::new("./tmp".into(), 1024 * 1024),
                job_rx,
                metrics_tx.clone(),
                SystemClock,
            )
            .run()
            .await;
        }
    });

    let timeout_layer =
        TimeoutLayer::with_status_code(StatusCode::GATEWAY_TIMEOUT, Duration::from_secs(15));

    loop {
        let (stream, _) = listener
            .accept()
            .await
            .expect("Failed to accept connection");

        let io = TokioIo::new(stream);

        let job_tx = job_tx.clone();
        let metrics_tx = metrics_tx.clone();

        let tower_service =
            ServiceBuilder::new()
                .layer(timeout_layer)
                .service(tower::util::service_fn(
                    move |req: Request<hyper::body::Incoming>| {
                        let job_tx = job_tx.clone();
                        let metrics_tx = metrics_tx.clone();
                        async move {
                            match req.uri().path() {
                                "/fn_call" => {
                                    on_fn_call::on_fn_call(req, job_tx.clone(), metrics_tx.clone())
                                        .await
                                }
                                "/role" => role().await,
                                _ => {
                                    let body = http_body_util::Full::new(Bytes::from("Not found"))
                                        .map_err(|_| ErrorCode::InternalError(None));
                                    let mut res =
                                        hyper::Response::new(HyperOutgoingBody::new(body));
                                    *res.status_mut() = hyper::StatusCode::NOT_FOUND;
                                    Ok(res)
                                }
                            }
                        }
                    },
                ));
        let hyper_service = TowerToHyperService::new(tower_service);

        tokio::task::spawn(async move {
            if let Err(err) = http1::Builder::new()
                .serve_connection(io, hyper_service)
                .await
            {
                eprintln!("Error serving connection: {:?}", err);
            }
        });
    }
}

async fn role() -> Result<Response, Infallible> {
    let body = http_body_util::Full::new(Bytes::from("worker"))
        .map_err(|_| ErrorCode::InternalError(None));
    let res = hyper::Response::new(HyperOutgoingBody::new(body));
    Ok(res)
}

fn response(status: hyper::StatusCode, body: Bytes) -> Response {
    let body = http_body_util::Full::new(body).map_err(|_| ErrorCode::InternalError(None));
    let mut res = hyper::Response::new(HyperOutgoingBody::new(body));
    *res.status_mut() = status;
    res
}

fn internal_error_response() -> Response {
    response(
        hyper::StatusCode::INTERNAL_SERVER_ERROR,
        Bytes::from("Internal Server Error"),
    )
}
