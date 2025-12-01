use crate::{
    execute::{Job, Response},
    metrics::{Metrics, MetricsTx},
    *,
};
use hyper::Request;
use std::convert::Infallible;

pub async fn on_fn_call(
    request: Request<hyper::body::Incoming>,
    job_tx: tokio::sync::mpsc::Sender<Job<hyper::body::Incoming>>,
    metrics_tx: MetricsTx,
) -> Result<Response, Infallible> {
    let Some(code_id) = parse_code_id(&request) else {
        metrics_tx.send(Metrics::CodeIdParseError);
        return Ok(internal_error_response());
    };

    let (res_tx, res_rx) = tokio::sync::oneshot::channel();
    let _ = job_tx
        .send(Job {
            req: request,
            res_tx,
            code_id,
        })
        .await;

    Ok(res_rx.await.unwrap_or(internal_error_response()))
}

fn parse_code_id(request: &Request<hyper::body::Incoming>) -> Option<String> {
    request
        .uri()
        .host()?
        .split_once('.')
        .map(|(code_id, _)| code_id.to_string())
}
