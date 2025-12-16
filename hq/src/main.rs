mod dns;
mod health_checker;
mod host_id;
mod host_infra;
mod reaper;

use color_eyre::eyre::Result;
use dashmap::DashMap;
use health_checker::*;
use host_id::*;
use host_infra::*;
use http_body_util::Full;
use hyper::body::Bytes;
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper::{Request, Response};
use hyper_util::rt::TokioIo;
use std::convert::Infallible;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::net::TcpListener;

fn main() -> Result<()> {
    tokio::runtime::Runtime::new()
        .unwrap()
        .block_on(async move {
            // let host_infra = Arc::new(host_infra::oci::OciHostInfra::new());
            // let host_info_map = Arc::new(DashMap::new());
            // let health_check_map = Arc::new(DashMap::new());

            tokio::try_join!(
                web_server(),
                // host_infra::run_sync_host_info_map(host_infra.clone(), host_info_map.clone()),
                // health_checker::run(host_info_map.clone(), health_check_map.clone()),
                // reaper::run(
                //     host_infra.clone(),
                //     host_info_map.clone(),
                //     health_check_map.clone()
                // ),
                // dns::sync_ips(health_check_map.clone()),
            )
        })?;
    Ok(())
}

async fn web_server() -> Result<()> {
    let addr = SocketAddr::from(([0, 0, 0, 0], 8080));
    let listener = TcpListener::bind(addr).await?;

    loop {
        let (stream, _) = listener.accept().await?;
        let io = TokioIo::new(stream);

        tokio::task::spawn(async move {
            if let Err(err) = http1::Builder::new()
                .serve_connection(io, service_fn(route))
                .await
            {
                eprintln!("Error serving connection: {:?}", err);
            }
        });
    }
}

async fn route(req: Request<hyper::body::Incoming>) -> Result<Response<Full<Bytes>>, Infallible> {
    match req.uri().path() {
        "/health" => Ok(Response::new(Full::new(Bytes::from("ok")))),
        _ => Ok(Response::builder()
            .status(404)
            .body(Full::new(Bytes::from("not found")))
            .unwrap()),
    }
}
