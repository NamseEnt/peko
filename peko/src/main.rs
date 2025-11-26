mod execute;
mod metrics;
mod on_request;

use crate::execute::{Job, Response};
use hyper::Request;
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper_util::rt::TokioIo;
use measure_cpu_time::SystemClock;
use std::convert::Infallible;
use std::net::SocketAddr;
use tokio::net::TcpListener;

fn main() {
    tokio::runtime::Runtime::new()
        .unwrap()
        .block_on(real_main());
}

async fn real_main() {
    let port = 3000;
    let addr = SocketAddr::from(([127, 0, 0, 1], 3000));

    let listener = TcpListener::bind(addr)
        .await
        .unwrap_or_else(|_| panic!("Failed to bind listener on port {port}"));

    let (job_tx, job_rx) = tokio::sync::mpsc::channel(10 * 1024);

    tokio::spawn(async move {
        execute::Executor::new(
            adapt_cache::fs::FsAdaptCache::new("./tmp".into(), 1024 * 1024),
            job_rx,
            metrics::MetricsTx::new(),
            SystemClock,
        )
        .run()
        .await;
    });

    loop {
        let (stream, _) = listener
            .accept()
            .await
            .expect("Failed to accept connection");

        let io = TokioIo::new(stream);

        let job_tx = job_tx.clone();
        tokio::task::spawn(async move {
            if let Err(err) = http1::Builder::new()
                .serve_connection(io, service_fn(|req| hello(req, job_tx.clone())))
                .await
            {
                eprintln!("Error serving connection: {:?}", err);
            }
        });
    }
}

async fn hello(
    request: Request<hyper::body::Incoming>,
    job_tx: tokio::sync::mpsc::Sender<Job<hyper::body::Incoming>>,
) -> Result<Response, Infallible> {
    let (res_tx, res_rx) = tokio::sync::oneshot::channel();
    let _ = job_tx
        .send(Job {
            req: request,
            res_tx,
            code_id: "todo".to_string(),
        })
        .await;
    Ok(res_rx.await.unwrap())
}
