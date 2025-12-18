mod execute;
mod tcp_listener;
mod telemetry;
use bytes::Bytes;
use execute::Job;
use http_body_util::BodyExt;
use hyper::{Request, StatusCode, server::conn::http1};
use hyper_util::{rt::TokioIo, service::TowerToHyperService};
use measure_cpu_time::SystemClock;
use std::{convert::Infallible, path::PathBuf, time::Duration};
use tower::ServiceBuilder;
use tower_http::timeout::TimeoutLayer;
use wasmtime_wasi_http::{bindings::http::types::ErrorCode, body::HyperOutgoingBody};

pub struct Config {
    pub port: Option<u16>,
    pub wasm_path: Option<PathBuf>,
    pub otlp_endpoint: Option<String>,
}

pub async fn run(config: Config) -> anyhow::Result<()> {
    let listener = {
        let port = config.port.unwrap_or(3000);
        let increment_on_fail = config.port.is_none();
        tcp_listener::open_tcp_listener(port, increment_on_fail)?
    };

    println!(
        "Server on http://localhost:{}",
        listener.local_addr().unwrap().port()
    );

    let code_id_parser = match &config.wasm_path {
        Some(wasm_path) => CodeIdParser::Local {
            wasm_path: wasm_path.clone(),
        },
        None => CodeIdParser::Url,
    };

    let proxy_cache = match config.wasm_path {
        Some(wasm_path) => adapt_cache::fs::FsAdaptCache::new(
            wasm_path
                .parent()
                .ok_or(anyhow::anyhow!("cannot find wasm_path's parent"))?
                .to_path_buf(),
            1024 * 1024,
        ),
        None => todo!(),
    };

    let (job_tx, job_rx) = tokio::sync::mpsc::channel(10 * 1024);

    // Setup telemetry
    let telemetry_providers = telemetry::setup_telemetry(config.otlp_endpoint)?;

    tokio::spawn({
        async move {
            execute::Executor::new(proxy_cache, job_rx, SystemClock, false)
                .run()
                .await;
        }
    });

    let result = loop {
        let (stream, _) = match listener.accept().await {
            Ok(x) => x,
            Err(e) => break Err(e.into()),
        };

        let tower_service = ServiceBuilder::new()
            .layer(TimeoutLayer::with_status_code(
                StatusCode::GATEWAY_TIMEOUT,
                Duration::from_secs(60),
            ))
            .service(tower::util::service_fn({
                let code_id_parser = code_id_parser.clone();
                let job_tx = job_tx.clone();

                move |req: Request<hyper::body::Incoming>| {
                    let code_id_parser = code_id_parser.clone();
                    let job_tx = job_tx.clone();

                    async move {
                        let Some(code_id) = code_id_parser.parse(&req) else {
                            telemetry::code_id_parse_error();
                            return internal_error();
                        };

                        let (res_tx, res_rx) = tokio::sync::oneshot::channel();
                        let Ok(_) = job_tx
                            .send(Job {
                                req,
                                res_tx,
                                code_id: code_id.clone(),
                            })
                            .await
                        else {
                            telemetry::oneshot_drop_before_response(&code_id);
                            return internal_error();
                        };

                        match res_rx.await {
                            Ok(res) => Ok(res),
                            Err(_err) => internal_error(),
                        }
                    }
                }
            }));

        tokio::task::spawn(async move {
            if let Err(err) = http1::Builder::new()
                .serve_connection(
                    TokioIo::new(stream),
                    TowerToHyperService::new(tower_service),
                )
                .await
            {
                eprintln!("Error serving connection: {:?}", err);
            }
        });
    };

    telemetry::shutdown_telemetry(telemetry_providers)?;
    result
}

fn internal_error()
-> Result<hyper::Response<http_body_util::combinators::BoxBody<Bytes, ErrorCode>>, Infallible> {
    let body = http_body_util::Full::new(Bytes::from("Internal Server Error"))
        .map_err(|_| ErrorCode::InternalError(None));
    let mut res = hyper::Response::new(HyperOutgoingBody::new(body));
    *res.status_mut() = hyper::StatusCode::INTERNAL_SERVER_ERROR;
    Ok(res)
}

#[derive(Debug, Clone)]
enum CodeIdParser {
    Local { wasm_path: PathBuf },
    Url,
}

impl CodeIdParser {
    fn parse(&self, request: &Request<hyper::body::Incoming>) -> Option<String> {
        match self {
            CodeIdParser::Local { wasm_path } => wasm_path
                .file_name()
                .map(|x| x.to_string_lossy().to_string()),
            CodeIdParser::Url => request
                .uri()
                .host()
                .and_then(|host| host.split_once('.').map(|(code_id, _)| code_id.to_string())),
        }
    }
}
