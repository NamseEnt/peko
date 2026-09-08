use crate::cross_project_enqueue_hijack::CrossProjectEnqueueHijack;
use crate::cross_project_invoke_hijack::CrossProjectInvokeHijack;
use crate::execute::{ClientState, WasmInjectEnvelope};
use crate::measure_cpu_time::{Clock, TimeTracker, measure_cpu_time};
use crate::metric_gate;
use crate::object_storage_hijack::ObjectStorageHijack;
use crate::otlp_hijack::OtlpHijack;
use crate::presign_gate::PresignDenied;
use crate::public_storage_hijack::PublicStorageHijack;
use crate::queue_hijack::QueueHijack;
use crate::static_page_cache_hijack::StaticPageCacheHijack;
use crate::turso_hijack::TursoHijack;
use crate::vault_hijack::VaultHijack;
use crate::websocket_hijack::WebSocketHijack;
use crate::zstd_decode_body::ZstdDecodeBody;
use crate::{Request, Response, telemetry};
use anyhow::{Result, anyhow};
use bytes::Bytes;
use http_body_util::BodyExt;
use http_body_util::combinators::UnsyncBoxBody;
use hyper::http;
use std::future::Future;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::sync::{mpsc, oneshot};
use wasmtime::AsContextMut;
use wasmtime::component::Accessor;
use wasmtime_wasi::TrappableError;
use wasmtime_wasi_http::p3::Request as P3Request;
use wasmtime_wasi_http::p3::bindings::Service;
use wasmtime_wasi_http::p3::bindings::http::types::ErrorCode;
use wasmtime_wasi_http::p3::{RequestOptions, WasiHttpHooks, default_send_request};

tokio::task_local! {
    pub(crate) static SELF_HOST: String;
    pub(crate) static INVOCATION_DEADLINE: std::time::Instant;
    pub(crate) static INVOCATION_CANCELLATION: tokio_util::sync::CancellationToken;
}

// Inject a request into the wasm instance loop (same project) and await its
// response via oneshot. Used by both call_wasm_direct (JS fetch -> wasm) and
// self_invoke_send (wasm wasi-http hook -> wasm). Replaces the previous raw
// pointer / WASM_RAW task_local plumbing — no unsafe, no
// `accessor.lifetime`-dependent state across tasks.
async fn inject_and_await(
    sender: mpsc::UnboundedSender<WasmInjectEnvelope>,
    req: Request,
) -> Result<Response> {
    let (resp_tx, resp_rx) = oneshot::channel();
    let cancellation = INVOCATION_CANCELLATION
        .try_with(|token| token.clone())
        .unwrap_or_else(|_| tokio_util::sync::CancellationToken::new());
    if sender
        .send(WasmInjectEnvelope::new(req, resp_tx, cancellation))
        .is_err()
    {
        return Err(anyhow!("self-invoke target wasm instance channel closed"));
    }
    resp_rx
        .await
        .unwrap_or_else(|_| Err(anyhow!("self-invoke target dropped response")))
}

pub async fn call_wasm_direct(
    sender: mpsc::UnboundedSender<WasmInjectEnvelope>,
    req: Request,
) -> Result<Response> {
    inject_and_await(sender, req).await
}

pub(crate) fn extract_host(headers: &hyper::HeaderMap) -> Option<String> {
    let value = headers.get(hyper::header::HOST)?;
    let s = value.to_str().ok()?;
    Some(normalize_host(s))
}

fn normalize_host(host: &str) -> String {
    host.split(':').next().unwrap_or(host).to_ascii_lowercase()
}

fn matches_self(uri: &http::Uri, self_host: &str) -> bool {
    let Some(host) = uri.host() else { return false };
    host.eq_ignore_ascii_case(self_host)
}

type HookResponse = (
    http::Response<UnsyncBoxBody<Bytes, ErrorCode>>,
    Box<dyn Future<Output = std::result::Result<(), ErrorCode>> + Send>,
);
type HookResult = std::result::Result<HookResponse, TrappableError<ErrorCode>>;

pub(crate) struct SelfInvokeHooks {
    project_id: String,
    self_invoke_sender: mpsc::UnboundedSender<WasmInjectEnvelope>,
    turso_hijack: Option<Arc<TursoHijack>>,
    otlp_hijack: Option<Arc<OtlpHijack>>,
    queue_hijack: Option<Arc<QueueHijack>>,
    cross_project_enqueue_hijack: Option<Arc<CrossProjectEnqueueHijack>>,
    cross_project_invoke_hijack: Option<Arc<CrossProjectInvokeHijack>>,
    vault_hijack: Option<Arc<VaultHijack>>,
    object_storage_hijack: Option<Arc<ObjectStorageHijack>>,
    public_storage_hijack: Option<Arc<PublicStorageHijack>>,
    static_page_cache_hijack: Option<Arc<StaticPageCacheHijack>>,
    websocket_hijack: Option<Arc<WebSocketHijack>>,
}

impl SelfInvokeHooks {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        project_id: String,
        self_invoke_sender: mpsc::UnboundedSender<WasmInjectEnvelope>,
        turso_hijack: Option<Arc<TursoHijack>>,
        otlp_hijack: Option<Arc<OtlpHijack>>,
        queue_hijack: Option<Arc<QueueHijack>>,
        cross_project_enqueue_hijack: Option<Arc<CrossProjectEnqueueHijack>>,
        cross_project_invoke_hijack: Option<Arc<CrossProjectInvokeHijack>>,
        vault_hijack: Option<Arc<VaultHijack>>,
        object_storage_hijack: Option<Arc<ObjectStorageHijack>>,
        public_storage_hijack: Option<Arc<PublicStorageHijack>>,
        static_page_cache_hijack: Option<Arc<StaticPageCacheHijack>>,
        websocket_hijack: Option<Arc<WebSocketHijack>>,
    ) -> Self {
        Self {
            project_id,
            self_invoke_sender,
            turso_hijack,
            otlp_hijack,
            queue_hijack,
            cross_project_enqueue_hijack,
            cross_project_invoke_hijack,
            vault_hijack,
            object_storage_hijack,
            public_storage_hijack,
            static_page_cache_hijack,
            websocket_hijack,
        }
    }
}

impl WasiHttpHooks for SelfInvokeHooks {
    fn send_request(
        &mut self,
        request: http::Request<UnsyncBoxBody<Bytes, ErrorCode>>,
        options: Option<RequestOptions>,
        _fut: Box<dyn Future<Output = std::result::Result<(), ErrorCode>> + Send>,
    ) -> Box<dyn Future<Output = HookResult> + Send> {
        let self_host = SELF_HOST.try_with(|h| h.clone()).ok();
        let is_self = self_host
            .as_deref()
            .map(|h| matches_self(request.uri(), h))
            .unwrap_or(false);

        if is_self {
            return self_invoke_send(self.self_invoke_sender.clone(), request);
        }

        if let Some(hijack) = self.turso_hijack.clone()
            && hijack.matches(request.uri())
        {
            return turso_send(hijack, self.project_id.clone(), request, options);
        }

        if let Some(hijack) = self.queue_hijack.clone()
            && hijack.matches(request.uri())
        {
            return queue_send(hijack, self.project_id.clone(), request, options);
        }

        if let Some(hijack) = self.cross_project_enqueue_hijack.clone()
            && hijack.matches(request.uri())
        {
            return cross_project_enqueue_send(hijack, self.project_id.clone(), request, options);
        }

        if let Some(hijack) = self.cross_project_invoke_hijack.clone()
            && hijack.matches(request.uri())
        {
            return cross_project_invoke_send(hijack, self.project_id.clone(), request);
        }

        if let Some(hijack) = self.vault_hijack.clone()
            && hijack.matches(request.uri())
        {
            return vault_send(hijack, self.project_id.clone(), request, options);
        }

        if let Some(hijack) = self.otlp_hijack.clone()
            && hijack.matches(request.uri())
        {
            return otlp_send(hijack, self.project_id.clone(), request, options);
        }

        if let Some(hijack) = self.object_storage_hijack.clone()
            && hijack.matches(request.uri())
        {
            return object_storage_send(hijack, self.project_id.clone(), request, options);
        }

        if let Some(hijack) = self.public_storage_hijack.clone()
            && hijack.matches(request.uri())
        {
            return public_storage_send(
                hijack,
                self.queue_hijack.clone(),
                self.project_id.clone(),
                request,
                options,
            );
        }

        if let Some(hijack) = self.static_page_cache_hijack.clone()
            && hijack.matches(request.uri())
        {
            return static_page_cache_send(
                hijack,
                self.queue_hijack.clone(),
                self.project_id.clone(),
                request,
            );
        }

        if let Some(hijack) = self.websocket_hijack.clone()
            && hijack.matches(request.uri())
        {
            let remaining = INVOCATION_DEADLINE
                .try_with(|deadline| deadline.saturating_duration_since(std::time::Instant::now()))
                .unwrap_or_else(|_| std::time::Duration::from_secs(15));
            return websocket_send(hijack, self.project_id.clone(), request, remaining);
        }

        default_send(request, options)
    }
}

fn websocket_send(
    hijack: Arc<WebSocketHijack>,
    project_id: String,
    request: http::Request<UnsyncBoxBody<Bytes, ErrorCode>>,
    remaining: std::time::Duration,
) -> Box<dyn Future<Output = HookResult> + Send> {
    Box::new(async move {
        let response = hijack
            .handle_command(&project_id, request, remaining)
            .await?;
        let transmit: Box<dyn Future<Output = std::result::Result<(), ErrorCode>> + Send> =
            Box::new(async { Ok(()) });
        Ok((response, transmit))
    })
}

fn self_invoke_send(
    sender: mpsc::UnboundedSender<WasmInjectEnvelope>,
    request: http::Request<UnsyncBoxBody<Bytes, ErrorCode>>,
) -> Box<dyn Future<Output = HookResult> + Send> {
    Box::new(async move {
        let req: Request = request.map(|body| {
            body.map_err(|ec: ErrorCode| anyhow!("error_code: {ec:?}"))
                .boxed_unsync()
        });
        let resp = match inject_and_await(sender, req).await {
            Ok(r) => r,
            Err(e) => return Err(ErrorCode::InternalError(Some(format!("{e:?}"))).into()),
        };
        let http_resp = resp.map(|body| {
            body.map_err(|err: anyhow::Error| ErrorCode::InternalError(Some(err.to_string())))
                .boxed_unsync()
        });
        let io: Box<dyn Future<Output = std::result::Result<(), ErrorCode>> + Send> =
            Box::new(async { Ok(()) });
        Ok((http_resp, io))
    })
}

fn turso_send(
    hijack: Arc<TursoHijack>,
    project_id: String,
    mut request: http::Request<UnsyncBoxBody<Bytes, ErrorCode>>,
    options: Option<RequestOptions>,
) -> Box<dyn Future<Output = HookResult> + Send> {
    Box::new(async move {
        if let Err(e) = hijack.rewrite(&mut request, &project_id) {
            return Err(e.into());
        }

        let send_start = std::time::Instant::now();
        let (res, io) = default_send_request(request, options).await?;
        telemetry::stage_duration("hijack_turso", send_start.elapsed());
        let res = res.map(BodyExt::boxed_unsync);
        let io: Box<dyn Future<Output = std::result::Result<(), ErrorCode>> + Send> = Box::new(io);
        Ok((res, io))
    })
}

fn queue_send(
    hijack: Arc<QueueHijack>,
    project_id: String,
    request: http::Request<UnsyncBoxBody<Bytes, ErrorCode>>,
    options: Option<RequestOptions>,
) -> Box<dyn Future<Output = HookResult> + Send> {
    Box::new(async move {
        let (_parts, body) = request.into_parts();
        let body_bytes = match body.collect().await {
            Ok(c) => c.to_bytes(),
            Err(e) => return Err(ErrorCode::InternalError(Some(format!("{e:?}"))).into()),
        };

        let action = match hijack.handle_enqueue(&project_id, &body_bytes) {
            Ok(a) => a,
            Err(ec) => return Err(ec.into()),
        };

        hijack.record_usage(&project_id);

        match action {
            crate::queue_hijack::HijackAction::Forward(signed) => {
                let send_start = std::time::Instant::now();
                let (res, io) = default_send_request(signed, options).await?;
                telemetry::stage_duration("hijack_queue", send_start.elapsed());
                let res = res.map(BodyExt::boxed_unsync);
                let io: Box<dyn Future<Output = std::result::Result<(), ErrorCode>> + Send> =
                    Box::new(io);
                Ok((res, io))
            }
            crate::queue_hijack::HijackAction::Synthesized(resp) => {
                let io: Box<dyn Future<Output = std::result::Result<(), ErrorCode>> + Send> =
                    Box::new(async { Ok(()) });
                Ok((resp, io))
            }
        }
    })
}

fn cross_project_enqueue_send(
    hijack: Arc<CrossProjectEnqueueHijack>,
    project_id: String,
    request: http::Request<UnsyncBoxBody<Bytes, ErrorCode>>,
    options: Option<RequestOptions>,
) -> Box<dyn Future<Output = HookResult> + Send> {
    Box::new(async move {
        let (_parts, body) = request.into_parts();
        let body_bytes = match body.collect().await {
            Ok(c) => c.to_bytes(),
            Err(e) => return Err(ErrorCode::InternalError(Some(format!("{e:?}"))).into()),
        };

        let action = match hijack.handle_enqueue(&project_id, &body_bytes) {
            Ok(a) => a,
            Err(ec) => return Err(ec.into()),
        };

        match action {
            crate::cross_project_enqueue_hijack::HijackAction::Forward(signed) => {
                let send_start = std::time::Instant::now();
                let (res, io) = default_send_request(signed, options).await?;
                telemetry::stage_duration("hijack_cross_project_enqueue", send_start.elapsed());
                let res = res.map(BodyExt::boxed_unsync);
                let io: Box<dyn Future<Output = std::result::Result<(), ErrorCode>> + Send> =
                    Box::new(io);
                Ok((res, io))
            }
            crate::cross_project_enqueue_hijack::HijackAction::Synthesized(resp) => {
                let io: Box<dyn Future<Output = std::result::Result<(), ErrorCode>> + Send> =
                    Box::new(async { Ok(()) });
                Ok((resp, io))
            }
        }
    })
}

fn cross_project_invoke_send(
    hijack: Arc<CrossProjectInvokeHijack>,
    caller_project_id: String,
    request: http::Request<UnsyncBoxBody<Bytes, ErrorCode>>,
) -> Box<dyn Future<Output = HookResult> + Send> {
    Box::new(async move {
        let send_start = std::time::Instant::now();
        let resp = match hijack.handle_invoke(&caller_project_id, request).await {
            Ok(r) => r,
            Err(ec) => return Err(ec.into()),
        };
        telemetry::stage_duration("hijack_cross_project_invoke", send_start.elapsed());
        let io: Box<dyn Future<Output = std::result::Result<(), ErrorCode>> + Send> =
            Box::new(async { Ok(()) });
        Ok((resp, io))
    })
}

fn vault_send(
    hijack: Arc<VaultHijack>,
    project_id: String,
    request: http::Request<UnsyncBoxBody<Bytes, ErrorCode>>,
    options: Option<RequestOptions>,
) -> Box<dyn Future<Output = HookResult> + Send> {
    Box::new(async move {
        let (parts, body) = request.into_parts();
        let body_bytes = match body.collect().await {
            Ok(c) => c.to_bytes(),
            Err(e) => return Err(ErrorCode::InternalError(Some(format!("{e:?}"))).into()),
        };

        let method = parts.method.as_str();
        let path = parts
            .uri
            .path_and_query()
            .map(|pq| pq.path())
            .unwrap_or("/");

        let signed = match hijack.build_signed_request(&project_id, method, path, &body_bytes) {
            Ok(req) => req,
            Err(ec) => return Err(ec.into()),
        };

        let send_start = std::time::Instant::now();
        let (res, io) = default_send_request(signed, options).await?;
        telemetry::stage_duration("hijack_vault", send_start.elapsed());
        let res = res.map(BodyExt::boxed_unsync);
        let io: Box<dyn Future<Output = std::result::Result<(), ErrorCode>> + Send> = Box::new(io);
        Ok((res, io))
    })
}

fn otlp_send(
    hijack: Arc<OtlpHijack>,
    project_id: String,
    mut request: http::Request<UnsyncBoxBody<Bytes, ErrorCode>>,
    options: Option<RequestOptions>,
) -> Box<dyn Future<Output = HookResult> + Send> {
    Box::new(async move {
        if let Err(e) = hijack.rewrite(&mut request, &project_id) {
            return Err(e.into());
        }

        let (parts, body) = request.into_parts();
        let body_bytes = match body.collect().await {
            Ok(c) => c.to_bytes(),
            Err(e) => return Err(ErrorCode::InternalError(Some(format!("{e:?}"))).into()),
        };
        let body_bytes = match hijack.metric_gate() {
            Some(gate) if parts.uri.path().ends_with("/v1/metrics") => {
                metric_gate::enforce_request_bytes(gate, &project_id, body_bytes)
            }
            _ => body_bytes,
        };
        let forward_body = http_body_util::Full::new(body_bytes)
            .map_err(|never: std::convert::Infallible| match never {})
            .boxed_unsync();
        let forward_request = http::Request::from_parts(parts, forward_body);

        tokio::task::spawn_local(async move {
            let send_start = std::time::Instant::now();
            match default_send_request(forward_request, options).await {
                Ok((_resp, io)) => {
                    let _ = io.await;
                    telemetry::stage_duration("hijack_otlp", send_start.elapsed());
                }
                Err(err) => {
                    tracing::warn!(?err, "otlp forward failed");
                }
            }
        });

        let response = http::Response::builder()
            .status(202)
            .body(
                http_body_util::Empty::<Bytes>::new()
                    .map_err(|never: std::convert::Infallible| match never {})
                    .boxed_unsync(),
            )
            .map_err(|e| ErrorCode::InternalError(Some(e.to_string())))?;
        let io: Box<dyn Future<Output = std::result::Result<(), ErrorCode>> + Send> =
            Box::new(async { Ok(()) });
        Ok((response, io))
    })
}

fn object_storage_send(
    hijack: Arc<ObjectStorageHijack>,
    project_id: String,
    request: http::Request<UnsyncBoxBody<Bytes, ErrorCode>>,
    options: Option<RequestOptions>,
) -> Box<dyn Future<Output = HookResult> + Send> {
    Box::new(async move {
        let io: Box<dyn Future<Output = std::result::Result<(), ErrorCode>> + Send> =
            Box::new(async { Ok(()) });

        if let Some(presign) = presign_request(&request) {
            if let Some(gate) = hijack.presign_gate() {
                let epoch_hour = chrono::Utc::now().timestamp() / 3600;
                if let Err(denied) = gate.try_mint(&project_id, epoch_hour) {
                    let message = match denied {
                        PresignDenied::RateLimited => "presign refused: hourly mint limit reached",
                    };
                    let resp = http::Response::builder()
                        .status(429)
                        .body(
                            http_body_util::Full::new(Bytes::from_static(message.as_bytes()))
                                .map_err(|never: std::convert::Infallible| match never {})
                                .boxed_unsync(),
                        )
                        .map_err(|e| ErrorCode::InternalError(Some(e.to_string())))?;
                    return Ok((resp, io));
                }
            }
            let url = match hijack.presign(
                &request,
                &project_id,
                presign.method,
                presign.expires_secs,
                presign.content_length,
            ) {
                Ok(url) => url,
                Err(ec) => return Err(ec.into()),
            };
            let resp = http::Response::builder()
                .status(200)
                .body(
                    http_body_util::Full::new(Bytes::from(url))
                        .map_err(|never: std::convert::Infallible| match never {})
                        .boxed_unsync(),
                )
                .map_err(|e| ErrorCode::InternalError(Some(e.to_string())))?;
            return Ok((resp, io));
        }

        if hijack.is_local() {
            let resp = match hijack.serve_local(request).await {
                Ok(resp) => resp,
                Err(ec) => return Err(ec.into()),
            };
            return Ok((resp, io));
        }

        let mut request = request;
        if let Err(e) = hijack.sign_r2(&mut request, &project_id) {
            return Err(e.into());
        }
        let send_start = std::time::Instant::now();
        let (res, send_io) = default_send_request(request, options).await?;
        telemetry::stage_duration("hijack_object_storage", send_start.elapsed());
        let res = res.map(BodyExt::boxed_unsync);
        let send_io: Box<dyn Future<Output = std::result::Result<(), ErrorCode>> + Send> =
            Box::new(send_io);
        Ok((res, send_io))
    })
}

fn public_storage_send(
    hijack: Arc<PublicStorageHijack>,
    queue_hijack: Option<Arc<QueueHijack>>,
    project_id: String,
    request: http::Request<UnsyncBoxBody<Bytes, ErrorCode>>,
    options: Option<RequestOptions>,
) -> Box<dyn Future<Output = HookResult> + Send> {
    Box::new(async move {
        let mut request = request;

        if request.headers().contains_key("x-fn0-public-purge") {
            if !hijack.allow_purge(&project_id) {
                let resp = text_response(429, "purge refused: hourly limit reached".to_string())?;
                return Ok((resp, empty_io()));
            }
            let Some(url) = hijack.public_url_for(&project_id, request.uri().path()) else {
                let resp = text_response(500, "public storage is not configured".to_string())?;
                return Ok((resp, empty_io()));
            };
            enqueue_public_object_purge(queue_hijack.as_deref(), &hijack, &project_id, url).await;
            return Ok((accepted_response()?, empty_io()));
        }

        if let Some(presign) = public_presign_request(&request) {
            let url = match hijack.presign_put(
                &request,
                &project_id,
                &presign.content_type,
                presign.expires_secs,
                presign.content_length,
            ) {
                Ok(url) => url,
                Err(ec) => return Err(ec.into()),
            };
            return Ok((text_response(200, url)?, empty_io()));
        }

        // `forte dev` has no edge, so the marker falls through to the local
        // store and the app sees the same bytes it would in production.
        if request.headers().contains_key("x-fn0-public-cdn-get") && !hijack.is_local() {
            let Some(url) = hijack.public_url_for(&project_id, request.uri().path()) else {
                let resp = text_response(500, "public storage is not configured".to_string())?;
                return Ok((resp, empty_io()));
            };
            return public_cdn_get(url, options).await;
        }

        let changes_content = matches!(
            request.method(),
            &http::Method::PUT | &http::Method::POST | &http::Method::DELETE
        );
        let public_url = changes_content
            .then(|| hijack.public_url_for(&project_id, request.uri().path()))
            .flatten();

        if hijack.is_local() {
            let resp = match hijack.serve_local(request).await {
                Ok(resp) => resp,
                Err(ec) => return Err(ec.into()),
            };
            let io: Box<dyn Future<Output = std::result::Result<(), ErrorCode>> + Send> =
                Box::new(async { Ok(()) });
            return Ok((resp, io));
        }

        if let Err(e) = hijack.sign(&mut request, &project_id) {
            return Err(e.into());
        }
        let send_start = std::time::Instant::now();
        let (res, send_io) = default_send_request(request, options).await?;
        telemetry::stage_duration("hijack_public_storage", send_start.elapsed());

        if let Some(url) = public_url
            && res.status().is_success()
        {
            enqueue_public_object_purge(queue_hijack.as_deref(), &hijack, &project_id, url).await;
        }

        let res = res.map(BodyExt::boxed_unsync);
        let send_io: Box<dyn Future<Output = std::result::Result<(), ErrorCode>> + Send> =
            Box::new(send_io);
        Ok((res, send_io))
    })
}

/// Hands a guest's page invalidation to control.
///
/// The paths are validated here rather than in control so a typo comes back to
/// the app as a `400` on its own call. Resolving them to URLs stays in control,
/// which knows which host serves the project and holds the Cloudflare token.
fn static_page_cache_send(
    hijack: Arc<StaticPageCacheHijack>,
    queue_hijack: Option<Arc<QueueHijack>>,
    project_id: String,
    request: http::Request<UnsyncBoxBody<Bytes, ErrorCode>>,
) -> Box<dyn Future<Output = HookResult> + Send> {
    Box::new(async move {
        let (_parts, body) = request.into_parts();
        let body_bytes = match body.collect().await {
            Ok(collected) => collected.to_bytes(),
            Err(error) => return Err(ErrorCode::InternalError(Some(format!("{error:?}"))).into()),
        };

        let paths = match hijack.parse_paths(&body_bytes) {
            Ok(paths) => paths,
            Err(message) => return Ok((text_response(400, message)?, empty_io())),
        };
        let Some(control_project_id) = hijack.control_project_id() else {
            return Ok((accepted_response()?, empty_io()));
        };
        if paths.is_empty() {
            return Ok((accepted_response()?, empty_io()));
        }
        if !hijack.allow_purge(&project_id) {
            let resp = text_response(429, "purge refused: hourly limit reached".to_string())?;
            return Ok((resp, empty_io()));
        }

        let Some(queue_hijack) = queue_hijack else {
            tracing::warn!(
                project_id,
                "static page purge requested with no queue to purge through"
            );
            let resp = text_response(500, "purge queue is not configured".to_string())?;
            return Ok((resp, empty_io()));
        };
        let payload = serde_json::json!({ "project_id": project_id, "paths": paths });
        let action =
            queue_hijack.build_platform_enqueue(control_project_id, "static_page_purge", payload);
        match action {
            Ok(crate::queue_hijack::HijackAction::Synthesized(_)) => {}
            Ok(crate::queue_hijack::HijackAction::Forward(request)) => {
                if let Err(error) = default_send_request(request, None).await {
                    tracing::warn!(?error, "static page purge enqueue failed");
                    let resp = text_response(500, "purge could not be queued".to_string())?;
                    return Ok((resp, empty_io()));
                }
            }
            Err(error) => {
                tracing::warn!(?error, "static page purge enqueue could not be built");
                let resp = text_response(500, "purge could not be queued".to_string())?;
                return Ok((resp, empty_io()));
            }
        }
        Ok((accepted_response()?, empty_io()))
    })
}

struct PublicPresignRequest {
    content_type: String,
    expires_secs: u64,
    content_length: Option<u64>,
}

fn public_presign_request(
    request: &http::Request<UnsyncBoxBody<Bytes, ErrorCode>>,
) -> Option<PublicPresignRequest> {
    let headers = request.headers();
    let expires = headers.get("x-fn0-presign-put")?;
    let content_type = headers.get("x-fn0-presign-content-type")?;
    let content_length = match headers.get("x-fn0-presign-content-length") {
        Some(value) => Some(value.to_str().ok()?.parse().ok()?),
        None => None,
    };
    Some(PublicPresignRequest {
        content_type: content_type.to_str().ok()?.to_string(),
        expires_secs: expires.to_str().ok()?.parse().ok()?,
        content_length,
    })
}

fn empty_io() -> Box<dyn Future<Output = std::result::Result<(), ErrorCode>> + Send> {
    Box::new(async { Ok(()) })
}

fn text_response(
    status: u16,
    body: String,
) -> std::result::Result<http::Response<UnsyncBoxBody<Bytes, ErrorCode>>, ErrorCode> {
    http::Response::builder()
        .status(status)
        .body(
            http_body_util::Full::new(Bytes::from(body))
                .map_err(|never: std::convert::Infallible| match never {})
                .boxed_unsync(),
        )
        .map_err(|e| ErrorCode::InternalError(Some(e.to_string())))
}

fn accepted_response()
-> std::result::Result<http::Response<UnsyncBoxBody<Bytes, ErrorCode>>, ErrorCode> {
    text_response(202, String::new())
}

/// Reads a public object from the CDN instead of the bucket, so an edge hit
/// costs the project no bucket operation.
///
/// `zstd` is asked for on the app's behalf and decoded before the bytes reach
/// it, because the app's contract is that a read returns what was stored — the
/// encoding is an artefact of this hop. It is the only encoding offered: the
/// edge compresses per request rather than caching a compressed copy, so a
/// second algorithm would buy nothing and cost decode time.
///
/// An encoding that was never asked for means the bytes are encoded for some
/// other reason — an object written out of band with its own `Content-Encoding`
/// — and decoding it would hand back something other than what is stored, so it
/// is refused rather than guessed at.
async fn public_cdn_get(url: String, options: Option<RequestOptions>) -> HookResult {
    let request = http::Request::builder()
        .method(http::Method::GET)
        .uri(&url)
        .header("accept-encoding", "zstd")
        .body(
            http_body_util::Empty::<Bytes>::new()
                .map_err(|never: std::convert::Infallible| match never {})
                .boxed_unsync(),
        )
        .map_err(|e| ErrorCode::InternalError(Some(e.to_string())))?;

    let send_start = std::time::Instant::now();
    let (res, send_io) = default_send_request(request, options).await?;
    telemetry::stage_duration("hijack_public_storage_cdn", send_start.elapsed());

    let (mut parts, body) = res.into_parts();
    let body = body.boxed_unsync();
    let body = match parts.headers.get("content-encoding") {
        None => body,
        Some(encoding) if encoding.as_bytes().eq_ignore_ascii_case(b"zstd") => {
            parts.headers.remove("content-encoding");
            // The decoded length is not known until the stream ends, and a
            // stale one would describe the compressed bytes.
            parts.headers.remove("content-length");
            ZstdDecodeBody::new(body)?.boxed_unsync()
        }
        Some(encoding) => {
            let encoding = String::from_utf8_lossy(encoding.as_bytes()).into_owned();
            let resp = text_response(
                502,
                format!("public object came back with unrequested encoding {encoding}"),
            )?;
            return Ok((resp, empty_io()));
        }
    };

    let send_io: Box<dyn Future<Output = std::result::Result<(), ErrorCode>> + Send> =
        Box::new(send_io);
    Ok((http::Response::from_parts(parts, body), send_io))
}

/// Invalidates the edge copy of a public object the app just replaced.
///
/// The worker holds no Cloudflare credentials by design — a zone-wide purge
/// capability on every worker node is a wider blast radius than the control
/// plane — so this hands the work to control, which already owns the token and
/// is where purge batching belongs.
///
/// A failure here is logged rather than surfaced: the write itself succeeded,
/// and failing the app's call would tell it to retry a `put` that already
/// landed. The stale edge copy is the cost, bounded by the queue's own retry.
async fn enqueue_public_object_purge(
    queue_hijack: Option<&QueueHijack>,
    hijack: &PublicStorageHijack,
    project_id: &str,
    url: String,
) {
    let Some(queue_hijack) = queue_hijack else {
        tracing::warn!(url, "public object written with no queue to purge through");
        return;
    };
    let payload = serde_json::json!({ "project_id": project_id, "urls": [url] });
    let action = queue_hijack.build_platform_enqueue(
        hijack.control_project_id(),
        "public_object_purge",
        payload,
    );
    match action {
        Ok(crate::queue_hijack::HijackAction::Synthesized(_)) => {}
        Ok(crate::queue_hijack::HijackAction::Forward(request)) => {
            if let Err(error) = default_send_request(request, None).await {
                tracing::warn!(?error, "public object purge enqueue failed");
            }
        }
        Err(error) => tracing::warn!(?error, "public object purge enqueue could not be built"),
    }
}

struct PresignRequest {
    method: &'static str,
    expires_secs: u64,
    content_length: Option<u64>,
}

fn presign_request(
    request: &http::Request<UnsyncBoxBody<Bytes, ErrorCode>>,
) -> Option<PresignRequest> {
    let headers = request.headers();
    let (method, expires) = match headers.get("x-fn0-presign-get") {
        Some(value) => ("GET", value),
        None => ("PUT", headers.get("x-fn0-presign-put")?),
    };
    let content_length = match headers.get("x-fn0-presign-content-length") {
        Some(value) => Some(value.to_str().ok()?.parse().ok()?),
        None => None,
    };
    Some(PresignRequest {
        method,
        expires_secs: expires.to_str().ok()?.parse().ok()?,
        content_length,
    })
}

fn default_send(
    request: http::Request<UnsyncBoxBody<Bytes, ErrorCode>>,
    options: Option<RequestOptions>,
) -> Box<dyn Future<Output = HookResult> + Send> {
    Box::new(async move {
        let send_start = std::time::Instant::now();
        let (res, io) = default_send_request(request, options).await?;
        telemetry::stage_duration("outbound_fetch", send_start.elapsed());
        let res = res.map(BodyExt::boxed_unsync);
        let io: Box<dyn Future<Output = std::result::Result<(), ErrorCode>> + Send> = Box::new(io);
        Ok((res, io))
    })
}

pub(crate) async fn call_service<C: Clock>(
    accessor: &Accessor<ClientState<C>>,
    service: &Service,
    p3_req: P3Request,
    req_io: impl Future<Output = std::result::Result<(), ErrorCode>> + Send + 'static,
    time_tracker: TimeTracker<C>,
    is_timeout: &Arc<AtomicBool>,
) -> Result<Response> {
    let handle_fut = service.handle(accessor, p3_req);
    let handle_result = measure_cpu_time(time_tracker, handle_fut).await;

    match handle_result {
        Ok(Ok(resp)) => {
            let http_resp = accessor
                .with(|mut access| resp.into_http(access.as_context_mut(), req_io))
                .map_err(|error| {
                    telemetry::wasmtime_error("response_into_http", &format!("{error:?}"));
                    anyhow!("response into_http failed: {error:?}")
                })?;
            Ok(http_resp.map(|body| {
                body.map_err(|ec| anyhow!("error_code: {ec:?}"))
                    .boxed_unsync()
            }))
        }
        Ok(Err(ec)) => {
            telemetry::proxy_returns_error_code(&format!("{ec:?}"));
            Err(anyhow!("proxy returned error code: {ec:?}"))
        }
        Err(error) => Err(classify_wasm_error(error, is_timeout)),
    }
}

pub(crate) fn classify_wasm_error(
    error: wasmtime::Error,
    is_timeout: &Arc<AtomicBool>,
) -> anyhow::Error {
    match error.downcast::<wasmtime::Trap>() {
        Ok(trap) => {
            telemetry::trapped(&format!("{trap:?}"));
            if is_timeout.load(Ordering::Relaxed) {
                anyhow!("CPU time limit exceeded (trapped: {trap:?})")
            } else {
                anyhow!("wasm trapped: {trap:?}")
            }
        }
        Err(error) => {
            telemetry::canceled_unexpectedly(&format!("{error:?}"));
            anyhow!("wasm error: {error:?}")
        }
    }
}
