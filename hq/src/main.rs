mod args;
mod dns;
mod health_checker;
mod host_id;
mod host_provider;
mod params;
mod reaper;
mod telemetry;

use color_eyre::eyre::Result;
use dashmap::DashMap;
use health_checker::*;
use host_id::*;
use host_provider::*;
use http_body_util::Full;
use hyper::{Request, Response, body::Bytes, server::conn::http1, service::service_fn};
use hyper_util::rt::TokioIo;
use params::HqParams;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::net::TcpListener;
use tracing::*;

fn main() -> Result<()> {
    let rt = tokio::runtime::Runtime::new()?;

    rt.block_on(async move {
        let telemetry_providers = telemetry::setup_otlp()?;
        // let hq_params = HqParams::load()?;

        // let host_info_map = Arc::new(DashMap::new());
        // let health_check_map = Arc::new(DashMap::new());

        let shutdown_signal = async {
            tokio::signal::ctrl_c().await?;
            Ok(())
        };

        // let host_provider = HostProvider::from_params(&hq_params)?;

        let heartbeat_future = async {
            loop {
                telemetry::HqHeartbeat.send();
                info!("heartbeat sent");
                tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
            }
            #[allow(unreachable_code)]
            Ok::<(), color_eyre::eyre::Error>(())
        };

        // let sync_host_info_map_future =
        //     host_provider::run_sync_host_info_map(host_provider.clone(), host_info_map.clone());
        // let health_checker_future =
        //     health_checker::run(host_info_map.clone(), health_check_map.clone());
        // let reaper_future = reaper::run(
        //     host_provider.clone(),
        //     host_info_map.clone(),
        //     health_check_map.clone(),
        // );
        // let dns_sync_ips_future = dns::sync_ips(health_check_map.clone());

        let result = tokio::select! {
            result = shutdown_signal => { result }
            result = web_server() => { result }
            result = heartbeat_future => { result }
            // result = sync_host_info_map_future => { result }
            // result = health_checker_future => { result }
            // result = reaper_future => { result }
            // result = dns_sync_ips_future => { result }
        };

        telemetry::on_shutdown(telemetry_providers)?;

        result
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

async fn route(req: Request<hyper::body::Incoming>) -> Result<Response<Full<Bytes>>> {
    match req.uri().path() {
        "/health" => {
            info!("health check");
            Ok(Response::new(Full::new(Bytes::from("ok"))))
        }
        _ => Ok(Response::builder()
            .status(404)
            .body(Full::new(Bytes::from("not found")))
            .unwrap()),
    }
}
