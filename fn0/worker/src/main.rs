mod cache;
mod cert_poller;
mod cert_resolver;
mod env_crypto;
mod env_yaml;
mod manifest_poller;
mod queue_consumer;
mod storage_resolver;
mod telemetry;
mod vault_client;
mod websocket;
mod websocket_directory;
mod websocket_quic;
mod worker_pool;

use base64::Engine;
use bytes::Bytes;
use cache::S3BundleCache;
use cert_resolver::SniCertResolver;
use color_eyre::eyre::Result;
use fn0::{
    CrossProjectEnqueueHijack, CrossProjectInvokeDispatcher, CrossProjectInvokeHijack,
    ExecutionContext, MAX_REQUEST_BODY_SIZE, MetricCardinalityGate, ObjectStorageHijack,
    OtlpHijack, PresignGate, PublicStorageHijack, PurgeGate, QueueHijack, RequestBodyTooLarge,
    RequestCancellation, StaticPageCacheHijack, TursoHijack, VaultHijack, WebSocketHijack,
};
use http_body_util::combinators::UnsyncBoxBody;
use http_body_util::{BodyExt, Full};
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper_util::rt::TokioIo;
use mimalloc::MiMalloc;
use std::convert::Infallible;
use std::future::Future;
use std::net::SocketAddr;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::task::{Context, Poll};
use storage_resolver::ManifestStorageResolver;
use tokio::net::TcpListener;
use tokio::sync::mpsc;
use tokio::sync::oneshot;
use tokio::sync::{OwnedSemaphorePermit, Semaphore};
use tokio_rustls::TlsAcceptor;
use tokio_util::sync::CancellationToken;
use vault_client::VaultClient;
use worker_pool::{DispatchError, RequestEnvelope};

#[global_allocator]
static GLOBAL_ALLOCATOR: MiMalloc = MiMalloc;

pub type WorkerContext = ExecutionContext<S3BundleCache>;

const DEFAULT_CACHE_SIZE_BYTES: usize = 512 * 1024 * 1024;
const DEFAULT_OPS_PORT: u16 = 9090;
const REQUEST_DEADLINE: std::time::Duration = std::time::Duration::from_secs(15);
const CONTROL_PROJECT_ID: &str = "fn0-control";
const DEPLOY_STATUS_PATH: &str = "/__forte_action/deploy_status";
const CONTROL_DEPLOY_STATUS_DEADLINE: std::time::Duration = std::time::Duration::from_secs(60);
const REQUEST_BODY_CHUNK_SIZE: usize = 64 * 1024;
const AGGREGATE_REQUEST_BUFFER_SIZE: usize = 8 * 1024 * 1024;
const MAX_CONNECTION_BUFFER_SIZE: usize = 128 * 1024;
const REQUEST_BODY_BUFFER_PERMITS: u32 =
    (MAX_CONNECTION_BUFFER_SIZE / REQUEST_BODY_CHUNK_SIZE) as u32;

fn select_request_deadline(project_id: &str, request_path: &str) -> std::time::Duration {
    if project_id == CONTROL_PROJECT_ID && request_path == DEPLOY_STATUS_PATH {
        CONTROL_DEPLOY_STATUS_DEADLINE
    } else {
        REQUEST_DEADLINE
    }
}

fn declared_request_body_exceeds_limit(headers: &hyper::HeaderMap) -> bool {
    headers
        .get(hyper::header::CONTENT_LENGTH)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.trim().parse::<u64>().ok())
        .is_some_and(|length| length > MAX_REQUEST_BODY_SIZE)
}

pub fn read_pem_env(name: &str) -> Option<String> {
    if let Ok(v) = std::env::var(name) {
        return Some(v);
    }
    let b64 = std::env::var(format!("{name}_BASE64")).ok()?;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(&b64)
        .ok()?;
    String::from_utf8(bytes).ok()
}

fn build_otlp_hijack(metric_gate: Arc<MetricCardinalityGate>) -> Arc<OtlpHijack> {
    let target_host =
        std::env::var("FN0_OTLP_TARGET_HOST").expect("FN0_OTLP_TARGET_HOST must be set");
    let target_scheme: hyper::http::uri::Scheme = std::env::var("FN0_OTLP_TARGET_SCHEME")
        .expect("FN0_OTLP_TARGET_SCHEME must be set")
        .parse()
        .expect("FN0_OTLP_TARGET_SCHEME must be 'http' or 'https'");
    let auth_raw = std::env::var("FN0_OTLP_AUTH").expect("FN0_OTLP_AUTH must be set");
    let auth = if auth_raw.is_empty() {
        None
    } else {
        Some(auth_raw)
    };
    let target_path_prefix =
        std::env::var("FN0_OTLP_TARGET_PATH_PREFIX").unwrap_or_else(|_| "".to_string());
    let placeholder_host = std::env::var("FN0_OTLP_PLACEHOLDER_HOST")
        .unwrap_or_else(|_| "fn0-otel.fn0.dev".to_string());
    Arc::new(OtlpHijack {
        placeholder_host,
        target_scheme,
        target_host,
        target_path_prefix,
        auth,
        metric_gate: Some(metric_gate),
    })
}

fn build_queue_hijack() -> Arc<QueueHijack> {
    Arc::new(QueueHijack::from_env().expect("queue hijack init failed"))
}

fn build_cross_project_enqueue_hijack() -> Arc<CrossProjectEnqueueHijack> {
    Arc::new(
        CrossProjectEnqueueHijack::from_env().expect("control invoke queue hijack init failed"),
    )
}

fn build_cross_project_invoke_hijack() -> Arc<CrossProjectInvokeHijack> {
    Arc::new(
        CrossProjectInvokeHijack::from_env().expect("control invoke direct hijack init failed"),
    )
}

struct WorkerCrossProjectInvokeDispatcher {
    senders: Arc<Vec<mpsc::Sender<RequestEnvelope>>>,
    cache: S3BundleCache,
}

impl CrossProjectInvokeDispatcher for WorkerCrossProjectInvokeDispatcher {
    fn dispatch(
        &self,
        target_project_id: String,
        req: fn0::Request,
    ) -> anyhow::Result<oneshot::Receiver<anyhow::Result<fn0::Response>>> {
        let expected_code_version = req
            .headers()
            .get("x-fn0-internal-expected-code-version")
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.parse::<u64>().ok());
        if let Some(expected_code_version) = expected_code_version {
            let (response_sender, response_receiver) = oneshot::channel();
            let senders = self.senders.clone();
            let cache = self.cache.clone();
            tokio::spawn(async move {
                if cache.code_version(&target_project_id).await != Some(expected_code_version) {
                    let _ = response_sender.send(Err(anyhow::anyhow!(
                        "retryable target code version mismatch"
                    )));
                    return;
                }
                let (inner_sender, inner_receiver) = oneshot::channel();
                let envelope = RequestEnvelope::new(target_project_id, req, inner_sender);
                if let Err(error) = worker_pool::dispatch(&senders, envelope) {
                    let message = match error {
                        DispatchError::Full => "worker pool full",
                        DispatchError::Closed => "worker pool closed",
                    };
                    let _ = response_sender.send(Err(anyhow::anyhow!(message)));
                    return;
                }
                let result = inner_receiver
                    .await
                    .unwrap_or_else(|_| Err(anyhow::anyhow!("worker response dropped")));
                let _ = response_sender.send(result);
            });
            return Ok(response_receiver);
        }
        let (resp_tx, resp_rx) = oneshot::channel();
        let envelope = RequestEnvelope::new(target_project_id, req, resp_tx);
        worker_pool::dispatch(&self.senders, envelope).map_err(|e| match e {
            DispatchError::Full => anyhow::anyhow!("worker pool full"),
            DispatchError::Closed => anyhow::anyhow!("worker pool closed"),
        })?;
        Ok(resp_rx)
    }
}

fn build_vault_hijack() -> Arc<VaultHijack> {
    Arc::new(VaultHijack::from_env().expect("vault hijack init failed"))
}

fn build_turso_hijack() -> Arc<TursoHijack> {
    let group_token = std::env::var("TURSO_GROUP_TOKEN").expect("TURSO_GROUP_TOKEN must be set");
    let target_host_suffix =
        std::env::var("TURSO_DB_HOST_SUFFIX").expect("TURSO_DB_HOST_SUFFIX must be set");
    let placeholder_host =
        std::env::var("TURSO_PLACEHOLDER_HOST").unwrap_or_else(|_| "fn0-db.fn0.dev".to_string());
    Arc::new(TursoHijack {
        placeholder_host,
        target_host_suffix,
        group_token,
    })
}

fn build_object_storage_hijack(
    resolver: Arc<ManifestStorageResolver>,
    presign_gate: Arc<PresignGate>,
) -> Arc<ObjectStorageHijack> {
    let placeholder_host = std::env::var("FN0_OBJECT_STORAGE_PLACEHOLDER_HOST")
        .unwrap_or_else(|_| "fn0-object-storage.fn0.dev".to_string());
    Arc::new(
        ObjectStorageHijack::new_r2_resolved(placeholder_host, resolver)
            .with_presign_gate(presign_gate),
    )
}

fn build_public_storage_hijack(
    resolver: Arc<ManifestStorageResolver>,
    purge_gate: Arc<PurgeGate>,
) -> Arc<PublicStorageHijack> {
    let placeholder_host = std::env::var("FN0_PUBLIC_STORAGE_PLACEHOLDER_HOST")
        .unwrap_or_else(|_| "fn0-public-storage.fn0.dev".to_string());
    let control_project_id =
        std::env::var("FN0_CONTROL_PROJECT_ID").expect("FN0_CONTROL_PROJECT_ID must be set");
    Arc::new(
        PublicStorageHijack::new_resolved(placeholder_host, resolver, control_project_id)
            .with_purge_gate(purge_gate),
    )
}

fn build_static_page_cache_hijack(purge_gate: Arc<PurgeGate>) -> Arc<StaticPageCacheHijack> {
    let placeholder_host = std::env::var("FN0_STATIC_PAGE_CACHE_PLACEHOLDER_HOST")
        .unwrap_or_else(|_| "fn0-static-page-cache.fn0.dev".to_string());
    let control_project_id =
        std::env::var("FN0_CONTROL_PROJECT_ID").expect("FN0_CONTROL_PROJECT_ID must be set");
    Arc::new(
        StaticPageCacheHijack::new(placeholder_host, control_project_id)
            .with_purge_gate(purge_gate),
    )
}

fn main() -> Result<()> {
    let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
    color_eyre::install()?;

    let otlp_endpoint = std::env::var("OTLP_ENDPOINT").expect("OTLP_ENDPOINT must be set");

    let rt = tokio::runtime::Runtime::new()?;
    let _guard = rt.enter();
    let telemetry_providers = telemetry::setup(&otlp_endpoint)?;
    install_panic_hook();

    let result = rt.block_on(run());

    telemetry::shutdown(telemetry_providers)?;
    result
}

fn install_panic_hook() {
    let prev = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let location = info
            .location()
            .map(|l| format!("{}:{}:{}", l.file(), l.line(), l.column()))
            .unwrap_or_else(|| "<unknown>".to_string());
        let payload = info.payload();
        let message = if let Some(s) = payload.downcast_ref::<&'static str>() {
            (*s).to_string()
        } else if let Some(s) = payload.downcast_ref::<String>() {
            s.clone()
        } else {
            "<non-string panic payload>".to_string()
        };
        let backtrace = std::backtrace::Backtrace::force_capture();
        tracing::error!(
            location = %location,
            panic = %message,
            backtrace = %backtrace,
            "panic captured"
        );
        prev(info);
    }));
}

async fn run() -> Result<()> {
    let cwasm_bucket = std::env::var("CWASM_BUCKET").expect("CWASM_BUCKET is required");
    let s3_endpoint = std::env::var("S3_ENDPOINT").expect("S3_ENDPOINT is required");
    let s3_region = std::env::var("S3_REGION").unwrap_or_else(|_| "us-east-1".to_string());
    let s3_access_key_id =
        std::env::var("AWS_ACCESS_KEY_ID").expect("AWS_ACCESS_KEY_ID is required");
    let s3_secret_access_key =
        std::env::var("AWS_SECRET_ACCESS_KEY").expect("AWS_SECRET_ACCESS_KEY is required");
    let user_port: u16 = std::env::var("HTTP_PORT")
        .unwrap_or_else(|_| "443".to_string())
        .parse()
        .expect("HTTP_PORT must be a valid port");
    let ops_port: u16 = std::env::var("FN0_WORKER_OPS_PORT")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(DEFAULT_OPS_PORT);

    let vault_client = Arc::new(
        VaultClient::from_env()
            .map_err(|err| color_eyre::eyre::eyre!("vault client init: {err}"))?,
    );

    let cache_size_bytes = std::env::var("FN0_BUNDLE_CACHE_SIZE_BYTES")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(DEFAULT_CACHE_SIZE_BYTES);

    let operator = opendal::Operator::new(
        opendal::services::S3::default()
            .bucket(&cwasm_bucket)
            .region(&s3_region)
            .endpoint(&s3_endpoint)
            .access_key_id(&s3_access_key_id)
            .secret_access_key(&s3_secret_access_key)
            .disable_config_load()
            .disable_ec2_metadata(),
    )?
    .finish();

    let engine = fn0::build_engine().map_err(|e| color_eyre::eyre::eyre!("{e}"))?;
    fn0::spawn_epoch_ticker(engine.clone());
    let linker = fn0::build_linker(&engine);

    let cache = S3BundleCache::new(
        engine.clone(),
        linker.clone(),
        operator,
        vault_client.clone(),
        cache_size_bytes,
    );
    let storage_resolver = Arc::new(ManifestStorageResolver::new(vault_client.clone()));
    let direct_hijack = build_cross_project_invoke_hijack();
    let presign_gate = Arc::new(PresignGate::new());
    let purge_gate = Arc::new(PurgeGate::new());
    let metric_gate = Arc::new(MetricCardinalityGate::new());
    let websocket_hijack = Arc::new(WebSocketHijack::from_env());

    let execution_context = Arc::new(
        ExecutionContext::new(engine, linker, cache.clone())
            .with_turso_hijack(build_turso_hijack())
            .with_queue_hijack(build_queue_hijack())
            .with_cross_project_enqueue_hijack(build_cross_project_enqueue_hijack())
            .with_cross_project_invoke_hijack(direct_hijack.clone())
            .with_vault_hijack(build_vault_hijack())
            .with_otlp_hijack(build_otlp_hijack(metric_gate.clone()))
            .with_object_storage_hijack(build_object_storage_hijack(
                storage_resolver.clone(),
                presign_gate.clone(),
            ))
            .with_public_storage_hijack(build_public_storage_hijack(
                storage_resolver.clone(),
                purge_gate.clone(),
            ))
            .with_static_page_cache_hijack(build_static_page_cache_hijack(purge_gate))
            .with_websocket_hijack(websocket_hijack.clone()),
    );

    // Recorded on the worker's own meter rather than stamped into guest
    // payloads: this is operator-facing capacity data about the shared metrics
    // node, not a project's own telemetry.
    let active_series_gate = metric_gate.clone();
    let _active_series_gauge = opentelemetry::global::meter("fn0-worker")
        .u64_observable_gauge("fn0.metric.active_series")
        .with_unit("1")
        .with_description("Active metric series the cardinality gate tracks, per project")
        .with_callback(move |observer| {
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|duration| duration.as_secs() as i64)
                .unwrap_or(0);
            for (project_id, count) in active_series_gate.snapshot(now) {
                observer.observe(
                    count as u64,
                    &[opentelemetry::KeyValue::new("project_id", project_id)],
                );
            }
        })
        .build();

    let manifest_loaded = Arc::new(AtomicBool::new(false));
    let instance_count = Arc::new(AtomicU64::new(0));
    let drain_flag = Arc::new(AtomicBool::new(false));

    let num_workers = worker_pool::default_num_threads();
    let worker_senders = Arc::new(worker_pool::spawn_workers(
        execution_context.clone(),
        num_workers,
    ));
    tracing::info!(threads = num_workers, "worker threads started");

    direct_hijack.set_dispatcher(Arc::new(WorkerCrossProjectInvokeDispatcher {
        senders: worker_senders.clone(),
        cache: cache.clone(),
    }));
    let websocket_service = websocket::WebSocketService::new(worker_senders.clone())
        .await
        .map_err(|error| color_eyre::eyre::eyre!("websocket service init: {error:#}"))?;
    websocket_hijack.set_dispatcher(websocket_service.clone());

    let manifest_db =
        manifest_poller::build_database_from_env().map_err(|e| color_eyre::eyre::eyre!("{e}"))?;
    let manifest_handle = tokio::spawn({
        let cache = cache.clone();
        let manifest_loaded = manifest_loaded.clone();
        let storage_resolver = storage_resolver.clone();
        let websocket_service = websocket_service.clone();
        async move {
            manifest_poller::run(
                manifest_db,
                cache,
                storage_resolver,
                manifest_loaded,
                websocket_service,
            )
            .await;
        }
    });

    let cert_resolver = Arc::new(build_cert_resolver()?);
    let cert_db =
        manifest_poller::build_database_from_env().map_err(|e| color_eyre::eyre::eyre!("{e}"))?;
    let cert_handle = tokio::spawn(cert_poller::run(
        cert_db,
        cert_resolver.clone(),
        vault_client.clone(),
    ));

    let queue_consumer_handle = {
        let config = queue_consumer::QueueConsumerConfig::from_env()
            .map_err(|err| color_eyre::eyre::eyre!("queue consumer config: {err}"))?;
        let worker_senders = worker_senders.clone();
        let cache = Arc::new(cache.clone());
        tokio::spawn(async move {
            queue_consumer::run(config, cache, worker_senders).await;
        })
    };

    let apex_route = apex_route_from_env();

    let user_handle = tokio::spawn({
        let worker_senders = worker_senders.clone();
        let instance_count = instance_count.clone();
        let cache = cache.clone();
        let drain_flag = drain_flag.clone();
        let cert_resolver = cert_resolver.clone();
        let websocket_service = websocket_service.clone();
        async move {
            if let Err(err) = run_user_server(
                user_port,
                worker_senders,
                instance_count,
                drain_flag,
                cache,
                apex_route,
                cert_resolver,
                websocket_service,
            )
            .await
            {
                tracing::error!(%err, "user server error");
            }
        }
    });

    let ops_handle = tokio::spawn({
        let manifest_loaded = manifest_loaded.clone();
        let instance_count = instance_count.clone();
        let drain_flag = drain_flag.clone();
        let websocket_service = websocket_service.clone();
        async move {
            if let Err(err) = run_ops_server(
                ops_port,
                manifest_loaded,
                instance_count,
                drain_flag,
                websocket_service,
            )
            .await
            {
                tracing::error!(%err, "ops server error");
            }
        }
    });

    tokio::select! {
        _ = manifest_handle => {},
        _ = cert_handle => {},
        _ = user_handle => {},
        _ = ops_handle => {},
        _ = queue_consumer_handle => {},
        _ = tokio::signal::ctrl_c() => {
            tracing::info!("Received ctrl-c, shutting down");
        },
    }

    Ok(())
}

struct ApexRoute {
    domain: String,
    project_id: String,
}

fn apex_route_from_env() -> Option<Arc<ApexRoute>> {
    match (
        std::env::var("FN0_APEX_DOMAIN"),
        std::env::var("FN0_APEX_PROJECT_ID"),
    ) {
        (Ok(domain), Ok(project_id)) => Some(Arc::new(ApexRoute { domain, project_id })),
        _ => None,
    }
}

fn build_cert_resolver() -> Result<SniCertResolver> {
    let cert_pem =
        read_pem_env("ORIGIN_CERT_PEM").expect("ORIGIN_CERT_PEM (or _BASE64) must be set");
    let key_pem = read_pem_env("ORIGIN_KEY_PEM").expect("ORIGIN_KEY_PEM (or _BASE64) must be set");
    let fallback = cert_resolver::certified_key(&cert_pem, &key_pem)
        .map_err(|error| color_eyre::eyre::eyre!("platform origin certificate: {error}"))?;
    Ok(SniCertResolver::new(fallback))
}

#[allow(clippy::too_many_arguments)]
async fn run_user_server(
    port: u16,
    worker_senders: Arc<Vec<mpsc::Sender<RequestEnvelope>>>,
    instance_count: Arc<AtomicU64>,
    drain_flag: Arc<AtomicBool>,
    cache: S3BundleCache,
    apex_route: Option<Arc<ApexRoute>>,
    cert_resolver: Arc<SniCertResolver>,
    websocket_service: Arc<websocket::WebSocketService>,
) -> Result<()> {
    let tls_acceptor = {
        let config = rustls::ServerConfig::builder()
            .with_no_client_auth()
            .with_cert_resolver(cert_resolver);
        TlsAcceptor::from(Arc::new(config))
    };

    let addr = SocketAddr::from(([0, 0, 0, 0], port));
    let listener = TcpListener::bind(addr).await?;
    tracing::info!(%addr, "user server listening (TLS)");
    let stream_budget = Arc::new(Semaphore::new(
        AGGREGATE_REQUEST_BUFFER_SIZE / REQUEST_BODY_CHUNK_SIZE,
    ));

    loop {
        let (socket, peer_addr) = listener.accept().await?;

        let worker_senders = worker_senders.clone();
        let instance_count = instance_count.clone();
        let drain_flag = drain_flag.clone();
        let tls_acceptor = tls_acceptor.clone();
        let cache = cache.clone();
        let apex_route = apex_route.clone();
        let websocket_service = websocket_service.clone();
        let stream_budget = stream_budget.clone();

        tokio::spawn(async move {
            // Sniff first byte to multiplex TLS user traffic (Cloudflare → NLB
            // → here, TLS ClientHello starts with 0x16) vs plain HTTP /health
            // (OCI NLB health check on the same 443 port). peek() leaves the
            // byte in the socket so the chosen handler can read it again.
            let mut sniff_buf = [0u8; 1];
            let first_byte = match tokio::time::timeout(
                std::time::Duration::from_secs(5),
                socket.peek(&mut sniff_buf),
            )
            .await
            {
                Ok(Ok(0)) => return,
                Ok(Ok(_)) => sniff_buf[0],
                Ok(Err(err)) => {
                    tracing::warn!(%err, "peek failed");
                    return;
                }
                Err(_) => {
                    tracing::warn!("peek timeout");
                    return;
                }
            };

            if first_byte == 0x16 {
                let service = service_fn(move |req| {
                    let worker_senders = worker_senders.clone();
                    let instance_count = instance_count.clone();
                    let drain_flag = drain_flag.clone();
                    let cache = cache.clone();
                    let apex_route = apex_route.clone();
                    let websocket_service = websocket_service.clone();
                    let stream_budget = stream_budget.clone();
                    async move {
                        handle_user_request(
                            req,
                            worker_senders,
                            instance_count,
                            drain_flag,
                            cache,
                            apex_route,
                            websocket_service,
                            peer_addr,
                            stream_budget,
                        )
                        .await
                    }
                });

                let result = match tls_acceptor.accept(socket).await {
                    Ok(tls_stream) => {
                        let mut connection_builder = http1::Builder::new();
                        connection_builder.max_buf_size(MAX_CONNECTION_BUFFER_SIZE);
                        connection_builder
                            .serve_connection(TokioIo::new(tls_stream), service)
                            .with_upgrades()
                            .await
                    }
                    Err(err) => {
                        tracing::error!(%err, "TLS handshake failed");
                        return;
                    }
                };

                if let Err(err) = result {
                    tracing::error!(%err, "Failed to serve connection");
                }
            } else {
                let service = service_fn(handle_plain_health_request);
                if let Err(err) = http1::Builder::new()
                    .serve_connection(TokioIo::new(socket), service)
                    .await
                {
                    tracing::warn!(%err, "plain HTTP serve failed");
                }
            }
        });
    }
}

async fn run_ops_server(
    port: u16,
    manifest_loaded: Arc<AtomicBool>,
    instance_count: Arc<AtomicU64>,
    drain_flag: Arc<AtomicBool>,
    websocket_service: Arc<websocket::WebSocketService>,
) -> Result<()> {
    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    let listener = TcpListener::bind(addr).await?;
    tracing::info!(%addr, "ops server listening");

    loop {
        let (socket, _peer_addr) = listener.accept().await?;
        let manifest_loaded = manifest_loaded.clone();
        let instance_count = instance_count.clone();
        let drain_flag = drain_flag.clone();
        let websocket_service = websocket_service.clone();

        tokio::spawn(async move {
            let service = service_fn(move |req| {
                let manifest_loaded = manifest_loaded.clone();
                let instance_count = instance_count.clone();
                let drain_flag = drain_flag.clone();
                let websocket_service = websocket_service.clone();
                async move {
                    handle_ops_request(
                        req,
                        manifest_loaded,
                        instance_count,
                        drain_flag,
                        websocket_service,
                    )
                    .await
                }
            });

            if let Err(err) = http1::Builder::new()
                .serve_connection(TokioIo::new(socket), service)
                .await
            {
                tracing::error!(%err, "Failed to serve ops connection");
            }
        });
    }
}

struct InFlightGuard {
    counter: Arc<AtomicU64>,
}
impl InFlightGuard {
    fn new(counter: Arc<AtomicU64>) -> Self {
        counter.fetch_add(1, Ordering::Relaxed);
        Self { counter }
    }
}
impl Drop for InFlightGuard {
    fn drop(&mut self) {
        self.counter.fetch_sub(1, Ordering::Relaxed);
    }
}

type HyperResponse = hyper::Response<fn0::Body>;

fn full_body(body_bytes: Bytes) -> fn0::Body {
    Full::new(body_bytes)
        .map_err(|error: Infallible| match error {})
        .boxed_unsync()
}

type BodyBudgetPermitFuture =
    Pin<Box<dyn Future<Output = Result<OwnedSemaphorePermit, tokio::sync::AcquireError>> + Send>>;

struct LimitedRequestBody<InnerBody> {
    inner: InnerBody,
    limit: u64,
    received: u64,
    stopped: bool,
    pending_data: Option<Bytes>,
    too_large: Arc<AtomicBool>,
    cancellation: CancellationToken,
    stream_budget: Arc<Semaphore>,
    budget_permit: Option<OwnedSemaphorePermit>,
    budget_waiter: Option<BodyBudgetPermitFuture>,
}

impl<InnerBody> LimitedRequestBody<InnerBody> {
    fn new(
        inner: InnerBody,
        stream_budget: Arc<Semaphore>,
        too_large: Arc<AtomicBool>,
        cancellation: CancellationToken,
    ) -> Self {
        Self::with_limit(
            inner,
            MAX_REQUEST_BODY_SIZE,
            stream_budget,
            too_large,
            cancellation,
        )
    }

    fn with_limit(
        inner: InnerBody,
        limit: u64,
        stream_budget: Arc<Semaphore>,
        too_large: Arc<AtomicBool>,
        cancellation: CancellationToken,
    ) -> Self {
        Self {
            inner,
            limit,
            received: 0,
            stopped: false,
            pending_data: None,
            too_large,
            cancellation,
            stream_budget,
            budget_permit: None,
            budget_waiter: None,
        }
    }
}

impl<InnerBody> http_body::Body for LimitedRequestBody<InnerBody>
where
    InnerBody: http_body::Body<Data = Bytes> + Unpin,
    InnerBody::Error: std::error::Error + Send + Sync + 'static,
{
    type Data = Bytes;
    type Error = anyhow::Error;

    fn poll_frame(
        mut self: Pin<&mut Self>,
        context: &mut Context<'_>,
    ) -> Poll<Option<Result<http_body::Frame<Self::Data>, Self::Error>>> {
        if self.stopped {
            return Poll::Ready(None);
        }

        self.budget_permit.take();
        if self.budget_waiter.is_none() {
            self.budget_waiter = Some(Box::pin(
                self.stream_budget
                    .clone()
                    .acquire_many_owned(REQUEST_BODY_BUFFER_PERMITS),
            ));
        }
        let budget_result = self
            .budget_waiter
            .as_mut()
            .expect("body budget waiter must exist")
            .as_mut()
            .poll(context);
        let budget_permit = match budget_result {
            Poll::Pending => return Poll::Pending,
            Poll::Ready(Ok(budget_permit)) => budget_permit,
            Poll::Ready(Err(error)) => {
                self.stopped = true;
                self.budget_permit = None;
                return Poll::Ready(Some(Err(anyhow::Error::new(error))));
            }
        };
        self.budget_waiter = None;
        self.budget_permit = Some(budget_permit);

        if let Some(mut pending_data) = self.pending_data.take() {
            let chunk_length = pending_data.len().min(REQUEST_BODY_CHUNK_SIZE);
            let chunk = pending_data.split_to(chunk_length);
            if !pending_data.is_empty() {
                self.pending_data = Some(pending_data);
            }
            return Poll::Ready(Some(Ok(http_body::Frame::data(chunk))));
        }

        match Pin::new(&mut self.inner).poll_frame(context) {
            Poll::Pending => Poll::Pending,
            Poll::Ready(Some(Ok(frame))) => {
                let Some(data) = frame.data_ref() else {
                    self.budget_permit = None;
                    return Poll::Ready(Some(Ok(frame)));
                };
                let data_length = data.len() as u64;
                let received = self.received.saturating_add(data_length);
                if received > self.limit {
                    self.stopped = true;
                    self.too_large.store(true, Ordering::Release);
                    self.cancellation.cancel();
                    self.budget_permit = None;
                    return Poll::Ready(Some(Err(anyhow::Error::new(RequestBodyTooLarge {
                        limit: self.limit,
                    }))));
                }
                self.received = received;
                let mut data = frame
                    .into_data()
                    .expect("frame data was present when the frame was inspected");
                let chunk_length = data.len().min(REQUEST_BODY_CHUNK_SIZE);
                let chunk = data.split_to(chunk_length);
                if !data.is_empty() {
                    self.pending_data = Some(data);
                }
                Poll::Ready(Some(Ok(http_body::Frame::data(chunk))))
            }
            Poll::Ready(Some(Err(error))) => {
                self.stopped = true;
                self.cancellation.cancel();
                self.budget_permit = None;
                Poll::Ready(Some(Err(anyhow::Error::new(error))))
            }
            Poll::Ready(None) => {
                self.stopped = true;
                self.budget_permit = None;
                Poll::Ready(None)
            }
        }
    }

    fn is_end_stream(&self) -> bool {
        self.stopped || self.pending_data.is_none() && self.inner.is_end_stream()
    }

    fn size_hint(&self) -> http_body::SizeHint {
        let mut hint = self.inner.size_hint();
        let remaining = self.limit.saturating_sub(self.received);
        hint.set_lower(hint.lower().min(remaining));
        if let Some(upper) = hint.upper() {
            hint.set_upper(upper.min(remaining));
        } else {
            hint.set_upper(remaining);
        }
        hint
    }
}

struct CancellationGuard {
    token: CancellationToken,
    armed: bool,
}

impl CancellationGuard {
    fn new(token: CancellationToken) -> Self {
        Self { token, armed: true }
    }

    fn disarm(&mut self) {
        self.armed = false;
    }
}

impl Drop for CancellationGuard {
    fn drop(&mut self) {
        if self.armed {
            self.token.cancel();
        }
    }
}

struct CancellationBody {
    inner: fn0::Body,
    token: Option<CancellationToken>,
    _in_flight_guard: Option<InFlightGuard>,
    deadline: Pin<Box<tokio::time::Sleep>>,
}

impl CancellationBody {
    fn new(
        inner: fn0::Body,
        token: CancellationToken,
        in_flight: InFlightGuard,
        deadline: tokio::time::Instant,
    ) -> Self {
        Self {
            inner,
            token: Some(token),
            _in_flight_guard: Some(in_flight),
            deadline: Box::pin(tokio::time::sleep_until(deadline)),
        }
    }

    fn cancel(&mut self) {
        if let Some(token) = self.token.take() {
            token.cancel();
        }
    }
}

impl Drop for CancellationBody {
    fn drop(&mut self) {
        self.cancel();
    }
}

impl http_body::Body for CancellationBody {
    type Data = Bytes;
    type Error = anyhow::Error;

    fn poll_frame(
        mut self: Pin<&mut Self>,
        context: &mut Context<'_>,
    ) -> Poll<Option<Result<http_body::Frame<Self::Data>, Self::Error>>> {
        if self.token.is_none() {
            return Poll::Ready(None);
        }
        if self.deadline.as_mut().poll(context).is_ready() {
            self.cancel();
            return Poll::Ready(Some(Err(anyhow::anyhow!(
                "request execution deadline exceeded"
            ))));
        }
        match Pin::new(&mut self.inner).poll_frame(context) {
            Poll::Ready(None) => {
                self.cancel();
                Poll::Ready(None)
            }
            result => result,
        }
    }

    fn is_end_stream(&self) -> bool {
        self.token.is_none() || self.inner.is_end_stream()
    }

    fn size_hint(&self) -> http_body::SizeHint {
        self.inner.size_hint()
    }
}

async fn handle_plain_health_request(
    req: hyper::Request<hyper::body::Incoming>,
) -> std::result::Result<HyperResponse, anyhow::Error> {
    if req.method() == hyper::Method::GET && req.uri().path() == "/health" {
        Ok(hyper::Response::new(full_body(Bytes::from("ok"))))
    } else {
        Ok(hyper::Response::builder()
            .status(404)
            .body(full_body(Bytes::new()))
            .unwrap())
    }
}

async fn handle_ops_request(
    req: hyper::Request<hyper::body::Incoming>,
    manifest_loaded: Arc<AtomicBool>,
    instance_count: Arc<AtomicU64>,
    drain_flag: Arc<AtomicBool>,
    websocket_service: Arc<websocket::WebSocketService>,
) -> std::result::Result<HyperResponse, anyhow::Error> {
    match (req.method(), req.uri().path()) {
        (&hyper::Method::GET, "/ready") => {
            if manifest_loaded.load(Ordering::Acquire) {
                Ok(hyper::Response::new(full_body(Bytes::from("ready"))))
            } else {
                Ok(hyper::Response::builder()
                    .status(503)
                    .body(full_body(Bytes::from("manifest not loaded")))
                    .unwrap())
            }
        }
        (&hyper::Method::POST, "/drain") => {
            drain_flag.store(true, Ordering::Relaxed);
            websocket_service.close_all().await;
            tracing::info!("worker entered drain mode");
            Ok(hyper::Response::new(full_body(Bytes::from("draining"))))
        }
        (&hyper::Method::GET, "/status") => {
            let active_count = instance_count.load(Ordering::Relaxed)
                + websocket_service.connection_count() as u64;
            let body = serde_json::json!({
                "instances": active_count,
                "websocket_connections": websocket_service.connection_count(),
                "draining": drain_flag.load(Ordering::Relaxed),
            });
            let s = serde_json::to_string(&body).unwrap();
            Ok(hyper::Response::builder()
                .status(200)
                .header("content-type", "application/json")
                .body(full_body(Bytes::from(s)))
                .unwrap())
        }
        (&hyper::Method::GET, "/health") => {
            Ok(hyper::Response::new(full_body(Bytes::from("good"))))
        }
        (&hyper::Method::GET, "/role") => {
            Ok(hyper::Response::new(full_body(Bytes::from("worker"))))
        }
        _ => Ok(hyper::Response::builder()
            .status(404)
            .body(full_body(Bytes::from("not found")))
            .unwrap()),
    }
}

async fn handle_websocket_upgrade(
    mut request: hyper::Request<hyper::body::Incoming>,
    project_id: String,
    websocket_service: Arc<websocket::WebSocketService>,
    peer_addr: SocketAddr,
) -> std::result::Result<HyperResponse, anyhow::Error> {
    let capacity_guard = match websocket_service.reserve_capacity(&project_id) {
        Ok(capacity_guard) => capacity_guard,
        Err(websocket::CapacityError::Project) => {
            return Ok(hyper::Response::builder()
                .status(429)
                .header("retry-after", "1")
                .body(full_body(Bytes::new()))
                .unwrap());
        }
        Err(websocket::CapacityError::Worker) => {
            return Ok(hyper::Response::builder()
                .status(503)
                .header("retry-after", "1")
                .body(full_body(Bytes::new()))
                .unwrap());
        }
    };
    let connection_id = websocket::WebSocketService::connection_id();
    let request_headers = request.headers().clone();
    let route_uri = websocket_route_uri(request.uri(), &request_headers)?;
    let connect_response = match websocket_service
        .invoke_connect(
            &project_id,
            &connection_id,
            &route_uri,
            &request_headers,
            Some(peer_addr),
        )
        .await
    {
        Ok(response) => response,
        Err(error) => {
            tracing::warn!(%project_id, %error, "websocket on_connect failed");
            return Ok(hyper::Response::builder()
                .status(500)
                .body(full_body(Bytes::new()))
                .unwrap());
        }
    };
    let decision = connect_response
        .headers()
        .get("x-fn0-internal-websocket-decision")
        .and_then(|value| value.to_str().ok());
    if connect_response.status() != hyper::StatusCode::NO_CONTENT || decision != Some("accept") {
        let mut response = hyper::Response::builder().status(connect_response.status());
        for (header_name, header_value) in connect_response.headers() {
            if websocket_handshake_header_allowed(header_name) {
                response = response.header(header_name, header_value);
            }
        }
        return Ok(response.body(full_body(Bytes::new()))?);
    }

    let selected_protocol = connect_response
        .headers()
        .get("x-fn0-internal-websocket-protocol")
        .and_then(|value| value.to_str().ok())
        .map(str::to_string);
    if let Some(selected_protocol) = selected_protocol.as_deref()
        && !requested_websocket_protocols(&request_headers)
            .iter()
            .any(|requested_protocol| requested_protocol == selected_protocol)
    {
        return Ok(hyper::Response::builder()
            .status(500)
            .body(full_body(Bytes::new()))
            .unwrap());
    }

    if let Err(error) = websocket_service
        .publish_connection(&project_id, &connection_id)
        .await
    {
        tracing::warn!(%project_id, %error, "websocket directory publish failed");
        return Ok(hyper::Response::builder()
            .status(503)
            .header("retry-after", "1")
            .body(full_body(Bytes::new()))
            .unwrap());
    }

    let (upgrade_response, upgrade_future) = match fastwebsockets::upgrade::upgrade(&mut request) {
        Ok(upgrade) => upgrade,
        Err(error) => {
            websocket_service.unpublish_connection(&connection_id).await;
            tracing::warn!(%project_id, %error, "invalid websocket upgrade request");
            return Ok(hyper::Response::builder()
                .status(400)
                .body(full_body(Bytes::new()))
                .unwrap());
        }
    };
    let (upgrade_parts, _) = upgrade_response.into_parts();
    let mut response = hyper::Response::from_parts(upgrade_parts, full_body(Bytes::new()));
    for (header_name, header_value) in connect_response.headers() {
        if websocket_handshake_header_allowed(header_name) {
            response
                .headers_mut()
                .append(header_name.clone(), header_value.clone());
        }
    }
    if let Some(selected_protocol) = selected_protocol {
        response.headers_mut().insert(
            hyper::header::SEC_WEBSOCKET_PROTOCOL,
            selected_protocol.parse()?,
        );
    }
    websocket_service.spawn_connection(
        project_id,
        connection_id,
        route_uri,
        upgrade_future,
        capacity_guard,
    );
    Ok(response)
}

fn requested_websocket_protocols(headers: &hyper::HeaderMap) -> Vec<String> {
    headers
        .get_all(hyper::header::SEC_WEBSOCKET_PROTOCOL)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .flat_map(|value| value.split(','))
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .collect()
}

fn websocket_route_uri(uri: &hyper::Uri, headers: &hyper::HeaderMap) -> anyhow::Result<hyper::Uri> {
    if uri.authority().is_some() {
        return Ok(uri.clone());
    }
    let host = headers
        .get(hyper::header::HOST)
        .and_then(|value| value.to_str().ok())
        .ok_or_else(|| anyhow::anyhow!("websocket request missing host"))?;
    Ok(format!("https://{host}{uri}").parse()?)
}

fn websocket_handshake_header_allowed(header_name: &hyper::header::HeaderName) -> bool {
    !matches!(
        header_name.as_str(),
        "connection"
            | "upgrade"
            | "sec-websocket-accept"
            | "sec-websocket-protocol"
            | "content-length"
            | "transfer-encoding"
    ) && !header_name.as_str().starts_with("x-fn0-")
        && !header_name.as_str().starts_with("sec-websocket-")
}

#[allow(clippy::too_many_arguments)]
async fn handle_user_request(
    mut req: hyper::Request<hyper::body::Incoming>,
    worker_senders: Arc<Vec<mpsc::Sender<RequestEnvelope>>>,
    instance_count: Arc<AtomicU64>,
    drain_flag: Arc<AtomicBool>,
    cache: S3BundleCache,
    apex_route: Option<Arc<ApexRoute>>,
    websocket_service: Arc<websocket::WebSocketService>,
    peer_addr: SocketAddr,
    stream_budget: Arc<Semaphore>,
) -> std::result::Result<HyperResponse, anyhow::Error> {
    if req.uri().path().starts_with("/__fn0_queue_task/") {
        return Ok(hyper::Response::builder()
            .status(403)
            .body(full_body(Bytes::from("Forbidden")))
            .unwrap());
    }

    if drain_flag.load(Ordering::Relaxed) {
        return Ok(hyper::Response::builder()
            .status(503)
            .header("connection", "close")
            .body(full_body(Bytes::from("draining")))
            .unwrap());
    }

    if declared_request_body_exceeds_limit(req.headers()) {
        return Ok(hyper::Response::builder()
            .status(413)
            .header("connection", "close")
            .body(full_body(Bytes::from("Payload Too Large")))
            .unwrap());
    }

    let in_flight_guard = InFlightGuard::new(instance_count);
    let cancellation = CancellationToken::new();
    let mut cancellation_guard = CancellationGuard::new(cancellation.clone());
    let body_too_large = Arc::new(AtomicBool::new(false));
    req.extensions_mut()
        .insert(RequestCancellation(cancellation.clone()));

    let internal_headers: Vec<hyper::header::HeaderName> = req
        .headers()
        .keys()
        .filter(|header_name| header_name.as_str().starts_with("x-fn0-internal-"))
        .cloned()
        .collect();
    for header_name in internal_headers {
        req.headers_mut().remove(header_name);
    }

    let host = req
        .headers()
        .get("host")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();
    let host_no_port = host.split(':').next().unwrap_or("").to_string();
    let request_path = req.uri().path().to_string();

    let resolve_start = std::time::Instant::now();
    let project_id = match apex_route.as_deref() {
        Some(apex) if apex.domain == host_no_port => apex.project_id.clone(),
        _ => match cache.resolve_domain(&host_no_port).await {
            Some(sub) => sub,
            None => {
                fn0::telemetry::stage_duration("resolve_domain", resolve_start.elapsed());
                return Ok(hyper::Response::builder()
                    .status(404)
                    .body(full_body(Bytes::from("Not Found")))
                    .unwrap());
            }
        },
    };
    fn0::telemetry::stage_duration("resolve_domain", resolve_start.elapsed());

    if fastwebsockets::upgrade::is_upgrade_request(&req) {
        return handle_websocket_upgrade(req, project_id, websocket_service, peer_addr).await;
    }

    let mapped_req = req.map(|body| {
        UnsyncBoxBody::new(LimitedRequestBody::new(
            body,
            stream_budget,
            body_too_large.clone(),
            cancellation.clone(),
        ))
        .boxed_unsync()
    });

    let (resp_tx, resp_rx) = oneshot::channel();
    let selected_request_deadline = select_request_deadline(&project_id, &request_path);
    let request_deadline = tokio::time::Instant::now() + selected_request_deadline;
    let envelope = RequestEnvelope::new(project_id.clone(), mapped_req, resp_tx)
        .with_execution_deadline(selected_request_deadline);

    if let Err(err) = worker_pool::dispatch(&worker_senders, envelope) {
        match err {
            DispatchError::Full => {
                tracing::warn!(%project_id, "worker queue full");
                return Ok(hyper::Response::builder()
                    .status(503)
                    .body(full_body(Bytes::from("Service Unavailable")))
                    .unwrap());
            }
            DispatchError::Closed => {
                tracing::error!(%project_id, "worker queue closed");
                return Ok(hyper::Response::builder()
                    .status(500)
                    .body(full_body(Bytes::from("Internal Server Error")))
                    .unwrap());
            }
        }
    }

    let run_result = match tokio::time::timeout_at(request_deadline, resp_rx).await {
        Ok(Ok(r)) => r,
        Ok(Err(_)) => {
            tracing::error!(%project_id, "worker dropped response channel");
            return Ok(hyper::Response::builder()
                .status(500)
                .body(full_body(Bytes::from("Internal Server Error")))
                .unwrap());
        }
        Err(_) => {
            fn0::telemetry::request_deadline_exceeded();
            tracing::error!(%project_id, "request exceeded deadline");
            return Ok(hyper::Response::builder()
                .status(504)
                .header("connection", "close")
                .body(full_body(Bytes::from("Gateway Timeout")))
                .unwrap());
        }
    };

    match run_result {
        Ok(resp) => {
            if body_too_large.load(Ordering::Acquire) {
                return Ok(hyper::Response::builder()
                    .status(413)
                    .header("connection", "close")
                    .body(full_body(Bytes::from("Payload Too Large")))
                    .unwrap());
            }
            let (parts, body) = resp.into_parts();
            cancellation_guard.disarm();
            let response_body = UnsyncBoxBody::new(CancellationBody::new(
                body,
                cancellation,
                in_flight_guard,
                request_deadline,
            ))
            .boxed_unsync();
            Ok(hyper::Response::from_parts(parts, response_body))
        }
        Err(err) => {
            if body_too_large.load(Ordering::Acquire)
                || err
                    .chain()
                    .any(|cause| cause.downcast_ref::<fn0::RequestBodyTooLarge>().is_some())
                || err.to_string().contains("HttpRequestBodySize")
            {
                return Ok(hyper::Response::builder()
                    .status(413)
                    .header("connection", "close")
                    .body(full_body(Bytes::from("Payload Too Large")))
                    .unwrap());
            }
            // Walk the chain: singleflight and the fetch path wrap this, and a
            // wrapped NotFound answered 502 instead of 404, which reads as a
            // broken deploy rather than an absent one.
            let not_found = err.chain().any(|cause| {
                matches!(
                    cause.downcast_ref::<fn0::cache::Error>(),
                    Some(fn0::cache::Error::NotFound)
                )
            });
            if not_found {
                return Ok(hyper::Response::builder()
                    .status(404)
                    .header("content-type", "text/plain; charset=utf-8")
                    .body(full_body(Bytes::from(
                        "No application is deployed at this subdomain.",
                    )))
                    .unwrap());
            }
            // The cause goes in the message, not a field: the log pipeline
            // forwards message bodies and drops structured fields, so a field
            // here is invisible exactly when an outage makes it matter.
            tracing::error!(%project_id, path = %request_path, "Failed to run fn0: {err:#}");
            Ok(hyper::Response::builder()
                .status(502)
                .body(full_body(Bytes::from("Bad Gateway")))
                .unwrap())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        CONTROL_DEPLOY_STATUS_DEADLINE, CancellationBody, CancellationGuard, DEPLOY_STATUS_PATH,
        InFlightGuard, LimitedRequestBody, MAX_REQUEST_BODY_SIZE, REQUEST_BODY_BUFFER_PERMITS,
        REQUEST_BODY_CHUNK_SIZE, REQUEST_DEADLINE, declared_request_body_exceeds_limit,
        select_request_deadline,
    };
    use bytes::Bytes;
    use futures::{StreamExt, stream};
    use http_body::Frame;
    use http_body_util::{BodyExt, StreamBody};
    use hyper::HeaderMap;
    use std::convert::Infallible;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
    use std::time::Duration;
    use tokio::sync::Semaphore;
    use tokio_util::sync::CancellationToken;

    #[test]
    fn deploy_status_gets_extended_deadline_only_for_control_project() {
        assert_eq!(
            select_request_deadline("fn0-control", DEPLOY_STATUS_PATH),
            CONTROL_DEPLOY_STATUS_DEADLINE
        );
        assert_eq!(
            select_request_deadline("fn0-control", "/__forte_action/other"),
            REQUEST_DEADLINE
        );
        assert_eq!(
            select_request_deadline("other-project", DEPLOY_STATUS_PATH),
            REQUEST_DEADLINE
        );
    }

    #[test]
    fn content_length_limit_rejects_only_values_above_the_transport_limit() {
        let mut headers = HeaderMap::new();
        headers.insert(
            hyper::header::CONTENT_LENGTH,
            MAX_REQUEST_BODY_SIZE.to_string().parse().unwrap(),
        );
        assert!(!declared_request_body_exceeds_limit(&headers));
        headers.insert(
            hyper::header::CONTENT_LENGTH,
            (MAX_REQUEST_BODY_SIZE + 1).to_string().parse().unwrap(),
        );
        assert!(declared_request_body_exceeds_limit(&headers));
    }

    #[tokio::test]
    async fn received_bytes_over_the_limit_cancel_the_request() {
        let inner = StreamBody::new(stream::iter([
            Ok::<_, Infallible>(Frame::data(Bytes::from_static(b"1234"))),
            Ok::<_, Infallible>(Frame::data(Bytes::from_static(b"56789"))),
        ]));
        let too_large = Arc::new(AtomicBool::new(false));
        let cancellation = CancellationToken::new();
        let budget = Arc::new(Semaphore::new(REQUEST_BODY_BUFFER_PERMITS as usize));
        let mut body = LimitedRequestBody::with_limit(
            inner,
            8,
            budget,
            too_large.clone(),
            cancellation.clone(),
        );

        let first = body
            .frame()
            .await
            .expect("first frame")
            .expect("first frame must succeed")
            .into_data()
            .expect("first frame must contain data");
        assert_eq!(first.as_ref(), b"1234");
        let error = body
            .frame()
            .await
            .expect("limit error frame")
            .expect_err("second frame must exceed the limit");
        let limit_error = error
            .downcast_ref::<fn0::RequestBodyTooLarge>()
            .expect("typed limit error");
        assert_eq!(limit_error.limit, 8);
        assert!(too_large.load(Ordering::Acquire));
        assert!(cancellation.is_cancelled());
    }

    #[tokio::test]
    async fn client_disconnect_cancels_body_delivery() {
        let inner = StreamBody::new(stream::iter([Err::<Frame<Bytes>, _>(std::io::Error::new(
            std::io::ErrorKind::ConnectionReset,
            "client disconnected",
        ))]));
        let cancellation = CancellationToken::new();
        let mut body = LimitedRequestBody::new(
            inner,
            Arc::new(Semaphore::new(REQUEST_BODY_BUFFER_PERMITS as usize)),
            Arc::new(AtomicBool::new(false)),
            cancellation.clone(),
        );

        body.frame()
            .await
            .expect("disconnect error frame")
            .expect_err("disconnect must fail body delivery");
        assert!(cancellation.is_cancelled());
    }

    #[tokio::test]
    async fn near_limit_stream_is_consumed_one_chunk_at_a_time() {
        let chunk_count = MAX_REQUEST_BODY_SIZE as usize / REQUEST_BODY_CHUNK_SIZE;
        let chunks = (0..chunk_count).map(|_| {
            Ok::<_, Infallible>(Frame::data(Bytes::from(vec![
                0_u8;
                REQUEST_BODY_CHUNK_SIZE
            ])))
        });
        let inner = StreamBody::new(stream::iter(chunks));
        let budget = Arc::new(Semaphore::new(REQUEST_BODY_BUFFER_PERMITS as usize));
        let mut body = LimitedRequestBody::new(
            inner,
            budget,
            Arc::new(AtomicBool::new(false)),
            CancellationToken::new(),
        );
        let mut received = 0_u64;
        let mut largest_chunk = 0_usize;

        while let Some(frame) = body.frame().await {
            let data = frame
                .expect("stream frame must succeed")
                .into_data()
                .expect("stream frame must contain data");
            received += data.len() as u64;
            largest_chunk = largest_chunk.max(data.len());
        }

        assert_eq!(received, MAX_REQUEST_BODY_SIZE);
        assert_eq!(largest_chunk, REQUEST_BODY_CHUNK_SIZE);
    }

    #[tokio::test]
    async fn concurrent_streams_wait_for_the_aggregate_budget() {
        let budget = Arc::new(Semaphore::new((REQUEST_BODY_BUFFER_PERMITS * 2) as usize));
        let make_body = || {
            let inner = StreamBody::new(stream::iter([Ok::<_, Infallible>(Frame::data(
                Bytes::from_static(b"chunk"),
            ))]));
            LimitedRequestBody::new(
                inner,
                budget.clone(),
                Arc::new(AtomicBool::new(false)),
                CancellationToken::new(),
            )
        };
        let mut first_body = make_body();
        let mut second_body = make_body();
        let mut waiting_body = make_body();

        first_body
            .frame()
            .await
            .expect("first body frame")
            .expect("first body frame must succeed");
        second_body
            .frame()
            .await
            .expect("second body frame")
            .expect("second body frame must succeed");
        assert_eq!(budget.available_permits(), 0);
        assert!(
            tokio::time::timeout(Duration::from_millis(10), waiting_body.frame())
                .await
                .is_err()
        );

        drop(first_body);
        waiting_body
            .frame()
            .await
            .expect("waiting body frame")
            .expect("waiting body frame must succeed");
    }

    #[tokio::test]
    async fn response_body_delivers_before_the_stream_finishes() {
        let (sender, receiver) = futures::channel::mpsc::unbounded::<Bytes>();
        let stream = receiver.map(|chunk| Ok::<_, anyhow::Error>(Frame::data(chunk)));
        let inner = StreamBody::new(stream).boxed_unsync();
        let cancellation = CancellationToken::new();
        let in_flight_count = Arc::new(AtomicU64::new(0));
        let in_flight = InFlightGuard::new(in_flight_count.clone());
        let mut body = CancellationBody::new(
            inner,
            cancellation.clone(),
            in_flight,
            tokio::time::Instant::now() + Duration::from_secs(1),
        );

        sender
            .unbounded_send(Bytes::from_static(b"first"))
            .expect("first response chunk must send");
        let first = tokio::time::timeout(Duration::from_millis(100), body.frame())
            .await
            .expect("first response chunk must arrive before stream completion")
            .expect("first response frame")
            .expect("first response frame must succeed")
            .into_data()
            .expect("first response frame must contain data");
        assert_eq!(first.as_ref(), b"first");
        assert!(!cancellation.is_cancelled());
        assert_eq!(in_flight_count.load(Ordering::Relaxed), 1);

        drop(body);
        assert!(cancellation.is_cancelled());
        assert_eq!(in_flight_count.load(Ordering::Relaxed), 0);
    }

    #[tokio::test]
    async fn response_stream_stops_at_the_request_deadline() {
        let (_sender, receiver) = futures::channel::mpsc::unbounded::<Bytes>();
        let stream = receiver.map(|chunk| Ok::<_, anyhow::Error>(Frame::data(chunk)));
        let inner = StreamBody::new(stream).boxed_unsync();
        let cancellation = CancellationToken::new();
        let in_flight_count = Arc::new(AtomicU64::new(0));
        let in_flight = InFlightGuard::new(in_flight_count.clone());
        let mut body = CancellationBody::new(
            inner,
            cancellation.clone(),
            in_flight,
            tokio::time::Instant::now() + Duration::from_millis(10),
        );

        body.frame()
            .await
            .expect("deadline error frame")
            .expect_err("response stream must stop at its deadline");
        assert!(cancellation.is_cancelled());
        assert_eq!(in_flight_count.load(Ordering::Relaxed), 1);
        drop(body);
        assert_eq!(in_flight_count.load(Ordering::Relaxed), 0);
    }

    #[tokio::test]
    async fn deadline_drop_cancels_associated_work() {
        assert_eq!(REQUEST_DEADLINE, Duration::from_secs(15));
        let cancellation = CancellationToken::new();
        let future_cancellation = cancellation.clone();
        let result = tokio::time::timeout(Duration::from_millis(10), async move {
            let _guard = CancellationGuard::new(future_cancellation);
            futures::future::pending::<()>().await;
        })
        .await;

        assert!(result.is_err());
        assert!(cancellation.is_cancelled());
    }
}
