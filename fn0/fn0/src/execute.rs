use crate::cache::Bundle;
use crate::measure_cpu_time::{Clock, SystemClock, TimeTracker};
use crate::object_storage_hijack::ObjectStorageHijack;
use crate::public_storage_hijack::PublicStorageHijack;
use crate::self_invoke::{
    self, INVOCATION_CANCELLATION, INVOCATION_DEADLINE, SELF_HOST, SelfInvokeHooks, call_service,
};
use crate::static_page_cache_hijack::StaticPageCacheHijack;
use crate::turso_hijack::TursoHijack;
use crate::websocket_hijack::WebSocketHijack;
use crate::{Request, Response, telemetry};
use anyhow::{Result, anyhow};
use futures::stream::{FuturesUnordered, StreamExt};
use http_body_util::BodyExt;
use std::future::Future;
use std::io;
use std::pin::Pin;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::task::{Context, Poll};
use std::time::Duration;
use tokio::io::AsyncWrite;
use tokio::sync::{mpsc, oneshot};
use tokio_util::sync::CancellationToken;
use wasmtime::{Engine, Store, component::Linker};
use wasmtime_wasi::cli::AsyncStdoutStream;
use wasmtime_wasi::*;
use wasmtime_wasi_http::{
    WasiHttpCtx,
    p3::{Request as P3Request, WasiHttpCtxView, WasiHttpView, bindings::http::types::ErrorCode},
};

struct TracingWriter {
    project_id: String,
    is_stderr: bool,
    buf: Vec<u8>,
}

impl TracingWriter {
    fn new(project_id: String, is_stderr: bool) -> Self {
        Self {
            project_id,
            is_stderr,
            buf: Vec::with_capacity(1024),
        }
    }

    fn emit_line(&self, line: &str) {
        if self.is_stderr {
            tracing::error!(project_id = %self.project_id, stream = "stderr", "{}", line);
        } else {
            tracing::info!(project_id = %self.project_id, stream = "stdout", "{}", line);
        }
    }
}

impl AsyncWrite for TracingWriter {
    fn poll_write(
        self: Pin<&mut Self>,
        _cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        let this = Pin::get_mut(self);
        this.buf.extend_from_slice(buf);
        while let Some(pos) = this.buf.iter().position(|&b| b == b'\n') {
            let line: Vec<u8> = this.buf.drain(..=pos).collect();
            let line_str = String::from_utf8_lossy(&line[..line.len() - 1]);
            let trimmed = line_str.trim_end_matches('\r');
            if !trimmed.is_empty() {
                this.emit_line(trimmed);
            }
        }
        Poll::Ready(Ok(buf.len()))
    }

    fn poll_flush(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Poll::Ready(Ok(()))
    }

    fn poll_shutdown(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        let this = Pin::get_mut(self);
        if !this.buf.is_empty() {
            let line_str = String::from_utf8_lossy(&this.buf);
            let trimmed = line_str.trim_end_matches(['\r', '\n']);
            if !trimmed.is_empty() {
                this.emit_line(trimmed);
            }
            this.buf.clear();
        }
        Poll::Ready(Ok(()))
    }
}

fn make_tracing_stream(project_id: String, is_stderr: bool) -> AsyncStdoutStream {
    AsyncStdoutStream::new(4096, TracingWriter::new(project_id, is_stderr))
}

pub use fn0_wasmtime::engine_config;

pub fn build_linker(engine: &Engine) -> Linker<ClientState<SystemClock>> {
    let mut linker = Linker::new(engine);
    wasmtime_wasi::p2::add_to_linker_async(&mut linker).unwrap();
    wasmtime_wasi::p3::add_to_linker(&mut linker).unwrap();
    wasmtime_wasi_http::p3::add_to_linker(&mut linker).unwrap();
    linker
}

pub fn spawn_epoch_ticker(engine: Engine) {
    std::thread::Builder::new()
        .name("fn0-epoch-ticker".into())
        .spawn(move || {
            loop {
                std::thread::sleep(Duration::from_millis(3));
                engine.increment_epoch();
            }
        })
        .expect("failed to spawn epoch ticker thread");
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn build_store<C>(
    engine: &Engine,
    project_id: &str,
    env_vars: &[(String, String)],
    time_tracker: TimeTracker<C>,
    is_timeout: Arc<AtomicBool>,
    hooks: SelfInvokeHooks,
    turso_hijack: Option<&TursoHijack>,
    queue_hijack: Option<&crate::QueueHijack>,
    cross_project_enqueue_hijack: Option<&crate::CrossProjectEnqueueHijack>,
    cross_project_invoke_hijack: Option<&crate::CrossProjectInvokeHijack>,
    vault_hijack: Option<&crate::VaultHijack>,
    object_storage_hijack: Option<&ObjectStorageHijack>,
    public_storage_hijack: Option<&PublicStorageHijack>,
    static_page_cache_hijack: Option<&StaticPageCacheHijack>,
    websocket_hijack: Option<&WebSocketHijack>,
) -> Store<ClientState<C>>
where
    C: Clock,
{
    let wasi = {
        let mut builder = WasiCtx::builder();
        builder.stdout(make_tracing_stream(project_id.to_string(), false));
        builder.stderr(make_tracing_stream(project_id.to_string(), true));
        for (key, value) in env_vars {
            if turso_hijack.is_some() && (key == "TURSO_URL" || key == "TURSO_AUTH_TOKEN") {
                continue;
            }
            if queue_hijack.is_some() && key == "FN0_QUEUE_URL" {
                continue;
            }
            if cross_project_enqueue_hijack.is_some() && key == "FN0_CROSS_PROJECT_ENQUEUE_URL" {
                continue;
            }
            if cross_project_invoke_hijack.is_some() && key == "FN0_CROSS_PROJECT_INVOKE_URL" {
                continue;
            }
            if vault_hijack.is_some() && key == "FN0_VAULT_URL" {
                continue;
            }
            if object_storage_hijack.is_some() && key == "FN0_OBJECT_STORAGE_URL" {
                continue;
            }
            if public_storage_hijack.is_some()
                && (key == "FN0_PUBLIC_STORAGE_URL" || key == "FN0_PUBLIC_STORAGE_BASE_URL")
            {
                continue;
            }
            if static_page_cache_hijack.is_some() && key == "FN0_STATIC_PAGE_CACHE_URL" {
                continue;
            }
            if websocket_hijack.is_some() && key == "FN0_WEBSOCKET_URL" {
                continue;
            }
            builder.env(key, value);
        }
        if let Some(hijack) = turso_hijack {
            builder.env("TURSO_URL", format!("http://{}", hijack.placeholder_host));
            builder.env("TURSO_AUTH_TOKEN", "");
        }
        if let Some(hijack) = queue_hijack {
            builder.env("FN0_QUEUE_URL", hijack.placeholder_url());
        }
        if let Some(hijack) = cross_project_enqueue_hijack
            && project_id == hijack.allowed_caller_project_id()
        {
            builder.env("FN0_CROSS_PROJECT_ENQUEUE_URL", hijack.placeholder_url());
        }
        if let Some(hijack) = cross_project_invoke_hijack
            && project_id == hijack.allowed_caller_project_id()
        {
            builder.env("FN0_CROSS_PROJECT_INVOKE_URL", hijack.placeholder_url());
        }
        if let Some(hijack) = vault_hijack {
            builder.env("FN0_VAULT_URL", hijack.placeholder_url());
        }
        if let Some(hijack) = object_storage_hijack {
            builder.env("FN0_OBJECT_STORAGE_URL", hijack.placeholder_url());
        }
        // Both or neither: `object_storage::public::bucket()` needs the base URL
        // to build the URLs it hands back, so injecting only the endpoint would
        // hand the guest a bucket that panics on first use.
        if let Some(hijack) = public_storage_hijack
            && let Some(base_url) = hijack.public_base_url_for(project_id)
        {
            builder.env("FN0_PUBLIC_STORAGE_URL", hijack.placeholder_url());
            builder.env("FN0_PUBLIC_STORAGE_BASE_URL", base_url);
        }
        if let Some(hijack) = static_page_cache_hijack {
            builder.env("FN0_STATIC_PAGE_CACHE_URL", hijack.placeholder_url());
        }
        if let Some(hijack) = websocket_hijack {
            builder.env("FN0_WEBSOCKET_URL", hijack.placeholder_url());
        }
        builder.build()
    };

    let mut store = Store::new(
        engine,
        ClientState {
            table: ResourceTable::new(),
            wasi,
            http: WasiHttpCtx::new(),
            time_tracker,
            is_timeout,
            hooks,
        },
    );
    store.epoch_deadline_trap();
    store.set_epoch_deadline(1);
    store.epoch_deadline_async_yield_and_update(1);
    let project_id_for_timeout = project_id.to_string();
    store.epoch_deadline_callback(move |context| {
        let state = context.data();
        let cpu_time = state.time_tracker.duration();
        if cpu_time > Duration::from_millis(1000) {
            telemetry::cpu_timeout(&project_id_for_timeout, cpu_time);
            state.is_timeout.store(true, Ordering::Relaxed);
            return Ok(wasmtime::UpdateDeadline::Interrupt);
        }
        Ok(wasmtime::UpdateDeadline::Continue(1))
    });

    store
}

pub struct WasmInjectEnvelope {
    pub request: Request,
    pub response_sender: oneshot::Sender<Result<Response>>,
    pub cancellation: CancellationToken,
}

impl WasmInjectEnvelope {
    pub fn new(
        request: Request,
        response_sender: oneshot::Sender<Result<Response>>,
        cancellation: CancellationToken,
    ) -> Self {
        Self {
            request,
            response_sender,
            cancellation,
        }
    }
}

#[allow(clippy::too_many_arguments)]
pub async fn run_wasm_instance_loop(
    engine: &Engine,
    bundle: Arc<Bundle>,
    project_id: String,
    self_invoke_sender: mpsc::UnboundedSender<WasmInjectEnvelope>,
    mut rx: mpsc::UnboundedReceiver<WasmInjectEnvelope>,
    turso_hijack: Option<Arc<TursoHijack>>,
    otlp_hijack: Option<Arc<crate::OtlpHijack>>,
    queue_hijack: Option<Arc<crate::QueueHijack>>,
    cross_project_enqueue_hijack: Option<Arc<crate::CrossProjectEnqueueHijack>>,
    cross_project_invoke_hijack: Option<Arc<crate::CrossProjectInvokeHijack>>,
    vault_hijack: Option<Arc<crate::VaultHijack>>,
    object_storage_hijack: Option<Arc<ObjectStorageHijack>>,
    public_storage_hijack: Option<Arc<PublicStorageHijack>>,
    static_page_cache_hijack: Option<Arc<StaticPageCacheHijack>>,
    websocket_hijack: Option<Arc<WebSocketHijack>>,
) -> Result<()> {
    let time_tracker = TimeTracker::new(SystemClock);
    let is_timeout = Arc::new(AtomicBool::new(false));

    let mut store = build_store(
        engine,
        &project_id,
        &bundle.env_vars,
        time_tracker.clone(),
        is_timeout.clone(),
        SelfInvokeHooks::new(
            project_id.clone(),
            self_invoke_sender,
            turso_hijack.clone(),
            otlp_hijack.clone(),
            queue_hijack.clone(),
            cross_project_enqueue_hijack.clone(),
            cross_project_invoke_hijack.clone(),
            vault_hijack.clone(),
            object_storage_hijack.clone(),
            public_storage_hijack.clone(),
            static_page_cache_hijack.clone(),
            websocket_hijack.clone(),
        ),
        turso_hijack.as_deref(),
        queue_hijack.as_deref(),
        cross_project_enqueue_hijack.as_deref(),
        cross_project_invoke_hijack.as_deref(),
        vault_hijack.as_deref(),
        object_storage_hijack.as_deref(),
        public_storage_hijack.as_deref(),
        static_page_cache_hijack.as_deref(),
        websocket_hijack.as_deref(),
    );

    let instantiate_start = std::time::Instant::now();
    let service = bundle
        .service_pre
        .instantiate_async(&mut store)
        .await
        .map_err(|error| {
            telemetry::wasmtime_error("instantiate_async", &format!("{error:?}"));
            anyhow!("instantiate_async failed: {error:?}")
        })?;
    telemetry::stage_duration("instantiate", instantiate_start.elapsed());

    let project_id_for_cpu = project_id.clone();
    let run_result = store
        .run_concurrent(async move |accessor| -> Result<()> {
            let mut pending: FuturesUnordered<Pin<Box<dyn Future<Output = ()> + Send>>> =
                FuturesUnordered::new();

            loop {
                tokio::select! {
                    biased;
                    maybe = rx.recv() => {
                        match maybe {
                            Some(envelope) => {
                                let WasmInjectEnvelope {
                                    request,
                                    response_sender,
                                    cancellation,
                                } = envelope;
                                let self_host = self_invoke::extract_host(request.headers())
                                    .unwrap_or_default();
                                let service_ref = &service;
                                let time_tracker = time_tracker.clone();
                                let is_timeout = is_timeout.clone();
                                pending.push(Box::pin(async move {
                                    let call_start = std::time::Instant::now();
                                    let invocation_deadline =
                                        std::time::Instant::now() + Duration::from_secs(15);
                                    let invocation = INVOCATION_DEADLINE.scope(
                                        invocation_deadline,
                                        INVOCATION_CANCELLATION.scope(
                                            cancellation.clone(),
                                            SELF_HOST.scope(self_host, async move {
                                                let req_http = request.map(|body| {
                                                    body.map_err(|error| {
                                                        if let Some(limit_error) = error
                                                            .downcast_ref::<crate::RequestBodyTooLarge>()
                                                        {
                                                            ErrorCode::HttpRequestBodySize(Some(
                                                                limit_error.limit,
                                                            ))
                                                        } else {
                                                            ErrorCode::InternalError(Some(
                                                                error.to_string(),
                                                            ))
                                                        }
                                                    })
                                                    .boxed_unsync()
                                                });
                                                let (p3_req, req_io) =
                                                    P3Request::from_http(req_http);
                                                call_service(
                                                    accessor,
                                                    service_ref,
                                                    p3_req,
                                                    req_io,
                                                    time_tracker,
                                                    &is_timeout,
                                                )
                                                .await
                                            }),
                                        ),
                                    );
                                    let result = tokio::select! {
                                        _ = cancellation.cancelled() => {
                                            Err(anyhow!("request cancelled"))
                                        }
                                        result = invocation => result,
                                    };
                                    telemetry::stage_duration("wasm_call", call_start.elapsed());
                                    if response_sender.send(result).is_err() {
                                        telemetry::oneshot_drop_before_response();
                                    }
                                }));
                            }
                            None => {
                                while pending.next().await.is_some() {}
                                break;
                            }
                        }
                    }
                    Some(()) = pending.next() => {}
                }
            }

            telemetry::cpu_time(&project_id_for_cpu, time_tracker.duration());
            Ok(())
        })
        .await;

    match run_result {
        Ok(inner) => inner,
        Err(error) => {
            telemetry::wasmtime_error("run_concurrent", &format!("{error:?}"));
            Err(anyhow!("run_concurrent failed: {error:?}"))
        }
    }
}

pub struct ClientState<C: Clock> {
    wasi: WasiCtx,
    http: WasiHttpCtx,
    table: ResourceTable,
    pub(crate) time_tracker: TimeTracker<C>,
    pub(crate) is_timeout: Arc<AtomicBool>,
    hooks: SelfInvokeHooks,
}

impl<C: Clock> WasiView for ClientState<C> {
    fn ctx(&mut self) -> WasiCtxView<'_> {
        WasiCtxView {
            ctx: &mut self.wasi,
            table: &mut self.table,
        }
    }
}

impl<C: Clock> WasiHttpView for ClientState<C> {
    fn http(&mut self) -> WasiHttpCtxView<'_> {
        WasiHttpCtxView {
            ctx: &mut self.http,
            table: &mut self.table,
            hooks: &mut self.hooks,
        }
    }
}
