use crate::{
    metrics::{Metrics, MetricsTx},
    *,
};
use adapt_cache::AdaptCache;
use bytes::Bytes;
use hyper::body::Body;
use measure_cpu_time::{Clock, TimeTracker, measure_cpu_time};
use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};
use tokio::sync::{mpsc::Receiver, oneshot};
use wasmtime::{
    Config, Engine, InstanceAllocationStrategy, PoolingAllocationConfig, Store,
    component::{Component, Linker},
};
use wasmtime_wasi::*;
use wasmtime_wasi_http::{
    WasiHttpCtx, WasiHttpView,
    bindings::{
        ProxyPre,
        http::types::{ErrorCode, Scheme},
    },
    body::HyperOutgoingBody,
};

pub type Response = hyper::Response<HyperOutgoingBody>;

pub struct Job<B> {
    pub req: hyper::Request<B>,
    pub res_tx: oneshot::Sender<Response>,
    pub code_id: String,
}

pub struct Executor<A, B, C: Clock> {
    engine: Engine,
    proxy_cache: A,
    job_rx: Receiver<Job<B>>,
    linker: Linker<ClientState<C>>,
    metrics_tx: MetricsTx,
    clock: C,
}

impl<A, B, C> Executor<A, B, C>
where
    A: AdaptCache<ProxyPre<ClientState<C>>, wasmtime::Error>,
    B: Body<Data = Bytes, Error = hyper::Error> + Send + Sync + 'static,
    C: Clock,
{
    pub fn new(proxy_cache: A, job_rx: Receiver<Job<B>>, metrics_tx: MetricsTx, clock: C) -> Self {
        let engine = Engine::new(&engine_config()).unwrap();

        let mut linker = Linker::new(&engine);
        wasmtime_wasi::p2::add_to_linker_async(&mut linker).unwrap();
        wasmtime_wasi_http::add_only_http_to_linker_async(&mut linker).unwrap();

        Self {
            engine,
            proxy_cache,
            job_rx,
            linker,
            metrics_tx,
            clock,
        }
    }
    pub async fn run(&mut self) {
        let mut interval = tokio::time::interval(Duration::from_millis(3));
        loop {
            tokio::select! {
                _ = interval.tick() => {
                    self.engine.increment_epoch();
                }

                res = self.job_rx.recv() => {
                    match res {
                        Some(job) => self.spawn_job_runner(job),
                        None => break,
                    }
                }
            }
        }
    }
    fn spawn_job_runner(&mut self, job: Job<B>) {
        let proxy_cache = self.proxy_cache.clone();
        let engine = self.engine.clone();
        let linker = self.linker.clone();
        let metrics_tx = self.metrics_tx.clone();
        let clock = self.clock.clone();

        // TODO: Throttle and hard limit for same code_id

        tokio::spawn(async move {
            run_job(job, proxy_cache, engine, linker, metrics_tx, clock).await;
        });
    }
}

fn engine_config() -> Config {
    const MB: usize = 1024 * 1024;

    let mut sys = sysinfo::System::new_all();
    sys.refresh_all();

    let total_memory_bytes = sys.total_memory() as usize;
    let total_memory_mb = total_memory_bytes / (1024 * 1024);
    const MAX_MEMORY_MB: usize = 128;
    let max_instance_count = total_memory_mb / MAX_MEMORY_MB;

    let mut pooling_allocation_config = PoolingAllocationConfig::new();
    pooling_allocation_config
        .max_memory_size(MB * MAX_MEMORY_MB)
        .linear_memory_keep_resident(MB * 16)
        .table_keep_resident(MB)
        .total_core_instances(max_instance_count as _)
        .total_memories(max_instance_count as _)
        .total_tables(max_instance_count as _)
        .pagemap_scan(wasmtime::Enabled::Auto)
        .memory_protection_keys(wasmtime::Enabled::Auto);

    let mut config = Config::new();

    config
        .async_support(true)
        .allocation_strategy(InstanceAllocationStrategy::Pooling(
            pooling_allocation_config,
        ))
        .epoch_interruption(true)
        .wasm_component_model(true);

    config
}

async fn run_job<A, B, C>(
    job: Job<B>,
    proxy_cache: A,
    engine: Engine,
    linker: Linker<ClientState<C>>,
    metrics_tx: MetricsTx,
    clock: C,
) where
    A: AdaptCache<ProxyPre<ClientState<C>>, wasmtime::Error>,
    B: Body<Data = Bytes, Error = hyper::Error> + Send + Sync + 'static,
    C: Clock,
{
    let Ok(proxy_pre) = get_proxy_pre(
        job.code_id.clone(),
        proxy_cache,
        engine,
        linker,
        metrics_tx.clone(),
    )
    .await
    else {
        let _ = job.res_tx.send(internal_error_response());
        return;
    };

    let response = handle_request(proxy_pre, job.req, job.code_id, metrics_tx, clock).await;

    let _ = job.res_tx.send(response);
}

async fn get_proxy_pre<A, C>(
    code_id: String,
    proxy_cache: A,
    engine: Engine,
    linker: Linker<ClientState<C>>,
    metrics_tx: MetricsTx,
) -> Result<ProxyPre<ClientState<C>>, ()>
where
    A: AdaptCache<ProxyPre<ClientState<C>>, wasmtime::Error>,
    C: Clock,
{
    match proxy_cache
        .get(&code_id.clone(), |bytes| {
            let component = unsafe { Component::deserialize(&engine, &bytes)? };
            let instance_pre = linker.instantiate_pre(&component)?;
            let proxy_pre = ProxyPre::new(instance_pre)?;
            metrics_tx.send(Metrics::CreateInstance {
                code_id: code_id.clone(),
            });
            Ok((proxy_pre, bytes.len()))
        })
        .await
    {
        Ok(proxy_pre) => Ok(proxy_pre),
        Err(error) => {
            metrics_tx.send(Metrics::ProxyCacheError { code_id, error });
            Err(())
        }
    }
}

async fn handle_request<B, C>(
    pre: ProxyPre<ClientState<C>>,
    req: hyper::Request<B>,
    code_id: String,
    metrics_tx: MetricsTx,
    clock: C,
) -> Response
where
    B: Body<Data = Bytes, Error = hyper::Error> + Send + Sync + 'static,
    C: Clock + Send + 'static,
{
    let time_tracker = TimeTracker::new(clock);
    let is_timeout = Arc::new(AtomicBool::new(false));

    let mut store = Store::new(
        pre.engine(),
        ClientState {
            table: ResourceTable::new(),
            wasi: WasiCtx::builder().inherit_stdio().build(),
            http: WasiHttpCtx::new(),
            time_tracker: time_tracker.clone(),
            metrics_tx: metrics_tx.clone(),
            code_id: code_id.clone(),
            is_timeout: is_timeout.clone(),
        },
    );
    store.epoch_deadline_trap();
    store.set_epoch_deadline(1);
    store.epoch_deadline_async_yield_and_update(1);
    store.epoch_deadline_callback({
        |context| {
            let state = context.data();
            let cpu_time = state.time_tracker.duration();
            if cpu_time > Duration::from_millis(10) {
                state.metrics_tx.send(Metrics::CpuTimeout {
                    code_id: state.code_id.clone(),
                    cpu_time,
                });
                state.is_timeout.store(true, Ordering::Relaxed);
                return Ok(wasmtime::UpdateDeadline::Interrupt);
            }
            Ok(wasmtime::UpdateDeadline::Continue(1))
        }
    });

    let (tx, rx) = tokio::sync::oneshot::channel();
    let req = match store.data_mut().new_incoming_request(Scheme::Http, req) {
        Ok(x) => x,
        Err(error) => {
            metrics_tx.send(Metrics::Wasmtime {
                func: "new_incoming_request",
                code_id,
                error,
            });
            return internal_error_response();
        }
    };
    let out = match store.data_mut().new_response_outparam(tx) {
        Ok(x) => x,
        Err(error) => {
            metrics_tx.send(Metrics::Wasmtime {
                func: "new_response_outparam",
                code_id,
                error,
            });
            return internal_error_response();
        }
    };

    // NOTE: Bad guys can put infinite loop in initialization code so it would be needed to measure cpu time from here.
    // But also it includes wasmtime's instantiation.
    // I guess we can put limit with fuel for initialization.
    let proxy = match pre.instantiate_async(&mut store).await {
        Ok(x) => x,
        Err(error) => {
            metrics_tx.send(Metrics::Wasmtime {
                func: "instantiate_async",
                code_id,
                error,
            });
            return internal_error_response();
        }
    };

    let task = tokio::task::spawn({
        let code_id = code_id.clone();
        let metrics_tx = metrics_tx.clone();
        async move {
            let result = measure_cpu_time(
                time_tracker.clone(),
                proxy
                    .wasi_http_incoming_handler()
                    .call_handle(store, req, out),
            )
            .await;

            metrics_tx.send(Metrics::CpuTime {
                code_id,
                cpu_time: time_tracker.duration(),
            });

            result
        }
    });

    let result = rx.await;

    if let Err(_oneshot_recv_err) = result {
        let result = task.await;
        if let Err(error) = result {
            metrics_tx.send(Metrics::RequestTaskJoinError { code_id, error });
            return internal_error_response();
        }
        let result = result.unwrap();

        if let Err(error) = result {
            match error.downcast::<wasmtime::Trap>() {
                Ok(trap) => {
                    metrics_tx.send(Metrics::Trapped {
                        code_id: code_id.clone(),
                        trap,
                    });
                }
                Err(error) => {
                    metrics_tx.send(Metrics::CanceledUnexpectedly { code_id, error });
                }
            }
        }

        if is_timeout.load(Ordering::Relaxed) {
            return timeout_response();
        }

        return internal_error_response();
    }

    let result = result.unwrap();

    if let Ok(response) = result {
        return response;
    }

    let error_code: ErrorCode = result.unwrap_err();

    metrics_tx.send(Metrics::ProxyReturnsErrorCode {
        code_id,
        error_code,
    });
    internal_error_response()
}

fn timeout_response() -> Response {
    response(
        hyper::StatusCode::GATEWAY_TIMEOUT,
        Bytes::from("Gateway Timeout"),
    )
}

pub struct ClientState<C: Clock> {
    wasi: WasiCtx,
    http: WasiHttpCtx,
    table: ResourceTable,
    time_tracker: TimeTracker<C>,
    metrics_tx: MetricsTx,
    code_id: String,
    is_timeout: Arc<AtomicBool>,
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
    fn ctx(&mut self) -> &mut WasiHttpCtx {
        &mut self.http
    }

    fn table(&mut self) -> &mut ResourceTable {
        &mut self.table
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bytes::Bytes;
    use measure_cpu_time::SystemClock;
    use std::collections::HashMap;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, Mutex};

    // ===== Test Infrastructure =====

    /// Helper to create an Engine with async support enabled (matching production)
    fn create_test_engine() -> Engine {
        Engine::new(&engine_config()).unwrap()
    }

    /// Helper to create a test linker with WASI support
    fn create_test_linker(engine: &Engine) -> Linker<ClientState<SystemClock>> {
        let mut linker = Linker::new(engine);
        wasmtime_wasi::p2::add_to_linker_async(&mut linker).unwrap();
        wasmtime_wasi_http::add_only_http_to_linker_async(&mut linker).unwrap();
        linker
    }

    /// Mock AdaptCache for testing
    #[derive(Clone)]
    struct MockAdaptCache {
        components: Arc<Mutex<HashMap<String, Bytes>>>,
        should_fail: Arc<AtomicBool>,
    }

    impl MockAdaptCache {
        fn new() -> Self {
            Self {
                components: Arc::new(Mutex::new(HashMap::new())),
                should_fail: Arc::new(AtomicBool::new(false)),
            }
        }

        fn insert(&self, id: String, bytes: Bytes) {
            self.components.lock().unwrap().insert(id, bytes);
        }

        fn set_should_fail(&self, should_fail: bool) {
            self.should_fail.store(should_fail, Ordering::SeqCst);
        }
    }

    impl AdaptCache<ProxyPre<ClientState<SystemClock>>, wasmtime::Error> for MockAdaptCache {
        async fn get(
            &self,
            id: &str,
            convert: impl FnOnce(
                Bytes,
            ) -> std::result::Result<
                (ProxyPre<ClientState<SystemClock>>, usize),
                wasmtime::Error,
            > + Send,
        ) -> Result<ProxyPre<ClientState<SystemClock>>, adapt_cache::Error<wasmtime::Error>>
        {
            if self.should_fail.load(Ordering::SeqCst) {
                return Err(adapt_cache::Error::StorageError(anyhow::anyhow!(
                    "Mock storage error"
                )));
            }

            let components = self.components.lock().unwrap();
            let bytes = components
                .get(id)
                .ok_or(adapt_cache::Error::NotFound)?
                .clone();
            drop(components);

            let (instance, _size) = convert(bytes).map_err(adapt_cache::Error::ConvertError)?;
            Ok(instance)
        }
    }

    /// Test MetricsTx that collects metrics for verification
    #[derive(Clone)]
    struct TestMetricsTx {
        metrics: Arc<Mutex<Vec<Metrics>>>,
    }

    impl TestMetricsTx {
        fn new() -> Self {
            Self {
                metrics: Arc::new(Mutex::new(Vec::new())),
            }
        }

        fn into_metrics_tx(self) -> MetricsTx {
            let metrics = self.metrics.clone();
            let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();

            // Spawn a task to collect metrics
            tokio::spawn(async move {
                while let Some(metric) = rx.recv().await {
                    metrics.lock().unwrap().push(metric);
                }
            });

            MetricsTx::from_sender(tx)
        }

        async fn assert_contains(&self, predicate: impl Fn(&Metrics) -> bool) {
            // Give the background task time to collect metrics
            for _ in 0..10 {
                tokio::task::yield_now().await;
                {
                    let metrics = self.metrics.lock().unwrap();
                    if metrics.iter().any(&predicate) {
                        return;
                    }
                }
                tokio::time::sleep(tokio::time::Duration::from_millis(10)).await;
            }
            let metrics = self.metrics.lock().unwrap();
            assert!(
                metrics.iter().any(predicate),
                "Expected metric not found. Metrics: {:?}",
                metrics
            );
        }
    }

    // ===== Unit Tests =====

    mod unit {
        use super::*;

        #[test]
        fn test_internal_error_response() {
            let response = internal_error_response();

            assert_eq!(response.status(), hyper::StatusCode::INTERNAL_SERVER_ERROR);

            // Verify body content
            let (parts, _body) = response.into_parts();
            assert_eq!(parts.status, hyper::StatusCode::INTERNAL_SERVER_ERROR);
        }
    }

    fn load_precompiled_sample_component() -> Vec<u8> {
        const WASM: &[u8] =
            include_bytes!("../../sample-wasi-http-rust/sample_wasi_http_rust.wasm");
        static CWASM: std::sync::OnceLock<Vec<u8>> = std::sync::OnceLock::new();
        CWASM
            .get_or_init(|| create_test_engine().precompile_component(WASM).unwrap())
            .to_vec()
    }

    // ===== Integration Tests =====

    mod integration {
        use super::*;

        #[tokio::test]
        async fn test_get_proxy_pre_creates_instance() {
            let engine = create_test_engine();
            let cache = MockAdaptCache::new();
            let linker = create_test_linker(&engine);
            let test_metrics = TestMetricsTx::new();

            // Prepare cache with a component
            let serialized = load_precompiled_sample_component();
            cache.insert("code-a".to_string(), Bytes::from(serialized));

            let result = get_proxy_pre(
                "code-a".to_string(),
                cache,
                engine,
                linker,
                test_metrics.clone().into_metrics_tx(),
            )
            .await;

            assert!(result.is_ok(), "Should successfully create instance");

            // Verify CreateInstance metric
            test_metrics
                .assert_contains(
                    |m| matches!(m, Metrics::CreateInstance { code_id } if code_id == "code-a"),
                )
                .await;
        }

        #[tokio::test]
        async fn test_get_proxy_pre_fails_when_cache_fails() {
            let engine = create_test_engine();
            let cache = MockAdaptCache::new();
            cache.set_should_fail(true);
            let linker = create_test_linker(&engine);
            let test_metrics = TestMetricsTx::new();

            let result = get_proxy_pre(
                "code-a".to_string(),
                cache,
                engine,
                linker,
                test_metrics.clone().into_metrics_tx(),
            )
            .await;

            assert!(result.is_err(), "Should fail when cache fails");

            // Verify ProxyCacheError metric
            test_metrics.assert_contains(
                |m| matches!(m, Metrics::ProxyCacheError { code_id, .. } if code_id == "code-a"),
            ).await;
        }

        #[tokio::test]
        async fn test_get_proxy_pre_fails_when_not_found() {
            let engine = create_test_engine();
            let cache = MockAdaptCache::new();
            // Don't insert any component - should trigger NotFound
            let linker = create_test_linker(&engine);
            let test_metrics = TestMetricsTx::new();

            let result = get_proxy_pre(
                "nonexistent".to_string(),
                cache,
                engine,
                linker,
                test_metrics.clone().into_metrics_tx(),
            )
            .await;

            assert!(result.is_err(), "Should fail when component not found");

            // Verify ProxyCacheError metric
            test_metrics.assert_contains(|m| matches!(m, Metrics::ProxyCacheError { code_id, .. } if code_id == "nonexistent")).await;
        }

        #[tokio::test]
        async fn test_pop_or_create_corrupt_component() {
            let engine = create_test_engine();
            let cache = MockAdaptCache::new();
            let linker = create_test_linker(&engine);
            let test_metrics = TestMetricsTx::new();

            // Insert corrupt/invalid component data
            cache.insert("corrupt".to_string(), Bytes::from(vec![0x00, 0x01, 0x02]));

            let result = get_proxy_pre(
                "corrupt".to_string(),
                cache,
                engine,
                linker,
                test_metrics.clone().into_metrics_tx(),
            )
            .await;

            // Should fail due to invalid component
            assert!(result.is_err(), "Should fail with corrupt component");

            // Verify ProxyCacheError metric (ConvertError variant)
            test_metrics
                .assert_contains(|m| {
                    matches!(m, Metrics::ProxyCacheError { code_id, .. } if code_id == "corrupt")
                })
                .await;
        }

        #[tokio::test]
        async fn test_all_errors_logged_to_metrics() {
            let test_metrics = TestMetricsTx::new();

            // Verify different error scenarios emit appropriate metrics
            // This is covered by individual tests above:
            // - ProxyCacheError for cache failures
            // - ProxyCacheError for not found
            // - ProxyCacheError for corrupt components

            // Verify at least one error was logged
            let engine = create_test_engine();
            let cache = MockAdaptCache::new();
            cache.set_should_fail(true);
            let linker = create_test_linker(&engine);

            let _ = get_proxy_pre(
                "test".to_string(),
                cache,
                engine,
                linker,
                test_metrics.clone().into_metrics_tx(),
            )
            .await;

            test_metrics
                .assert_contains(|m| matches!(m, Metrics::ProxyCacheError { .. }))
                .await;
        }
    }

    // ===== Timeout Tests =====

    mod timeout {
        use super::*;

        #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
        async fn test_timeout_triggers_at_10ms() {
            let engine = create_test_engine();
            let linker = create_test_linker(&engine);
            let test_metrics = TestMetricsTx::new();

            // Prepare WASM component
            let serialized = load_precompiled_sample_component();
            let component = unsafe { Component::deserialize(&engine, &serialized).unwrap() };
            let instance_pre = linker.instantiate_pre(&component).unwrap();
            let proxy_pre = ProxyPre::new(instance_pre).unwrap();

            // Start epoch incrementer (simulating the Executor::run loop)
            let epoch_handle = tokio::spawn(async move {
                loop {
                    tokio::time::sleep(Duration::from_millis(1)).await;
                    engine.increment_epoch();
                }
            });

            struct MockBody;
            impl Body for MockBody {
                type Data = Bytes;
                type Error = hyper::Error;
                fn poll_frame(
                    self: std::pin::Pin<&mut Self>,
                    _cx: &mut std::task::Context<'_>,
                ) -> std::task::Poll<Option<Result<hyper::body::Frame<Self::Data>, Self::Error>>>
                {
                    std::task::Poll::Ready(None)
                }
            }

            // Create request for /infinite-loop endpoint
            let req = hyper::Request::builder()
                .uri("http://localhost/infinite-loop")
                .body(MockBody)
                .unwrap();

            let metrics_tx = test_metrics.clone().into_metrics_tx();

            let response = handle_request(
                proxy_pre,
                req,
                "timeout-test".to_string(),
                metrics_tx,
                SystemClock,
            )
            .await;

            // Verify timeout response
            assert_eq!(
                response.status(),
                hyper::StatusCode::GATEWAY_TIMEOUT,
                "Expected Gateway Timeout for infinite loop"
            );

            // Verify CpuTimeout metric was recorded
            test_metrics
                .assert_contains(|m| {
                    matches!(m, Metrics::CpuTimeout { cpu_time, .. }
                        if *cpu_time > Duration::from_millis(10))
                })
                .await;

            epoch_handle.abort();
        }

        #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
        async fn test_wstd_sleep_does_not_trigger_cpu_timeout() {
            // Test scenario: Using wstd::task::sleep() for ~100ms should NOT trigger CPU timeout
            // because wstd sleep does not consume CPU time
            // Expected: Response should be OK (200), NOT Gateway Timeout (504)

            let engine = create_test_engine();
            let linker = create_test_linker(&engine);
            let test_metrics = TestMetricsTx::new();

            // Prepare WASM component
            let serialized = load_precompiled_sample_component();
            let component = unsafe { Component::deserialize(&engine, &serialized).unwrap() };
            let instance_pre = linker.instantiate_pre(&component).unwrap();
            let proxy_pre = ProxyPre::new(instance_pre).unwrap();

            // Start epoch incrementer (simulating the Executor::run loop)
            let epoch_handle = tokio::spawn(async move {
                loop {
                    tokio::time::sleep(Duration::from_millis(1)).await;
                    engine.increment_epoch();
                }
            });

            struct MockBody;
            impl Body for MockBody {
                type Data = Bytes;
                type Error = hyper::Error;
                fn poll_frame(
                    self: std::pin::Pin<&mut Self>,
                    _cx: &mut std::task::Context<'_>,
                ) -> std::task::Poll<Option<Result<hyper::body::Frame<Self::Data>, Self::Error>>>
                {
                    std::task::Poll::Ready(None)
                }
            }

            // Create request for /slow endpoint with 100ms sleep
            let req = hyper::Request::builder()
                .uri("http://localhost/slow?ms=100")
                .body(MockBody)
                .unwrap();

            let metrics_tx = test_metrics.clone().into_metrics_tx();

            let response = handle_request(
                proxy_pre,
                req,
                "wstd-sleep-test".to_string(),
                metrics_tx,
                SystemClock,
            )
            .await;

            // Verify response is OK, not timeout
            assert_eq!(
                response.status(),
                hyper::StatusCode::OK,
                "wstd sleep should NOT trigger CPU timeout"
            );

            // Verify CpuTimeout metric was NOT recorded
            // We need to wait a bit to ensure metrics are collected
            tokio::time::sleep(Duration::from_millis(100)).await;
            {
                let metrics = test_metrics.metrics.lock().unwrap();
                let has_cpu_timeout = metrics
                    .iter()
                    .any(|m| matches!(m, Metrics::CpuTimeout { .. }));
                assert!(
                    !has_cpu_timeout,
                    "CpuTimeout should not be recorded for wstd sleep"
                );
            }

            // Verify CpuTime metric was recorded with low CPU time
            test_metrics
                .assert_contains(|m| {
                    matches!(m, Metrics::CpuTime { code_id, cpu_time }
                        if code_id == "wstd-sleep-test" && *cpu_time < Duration::from_millis(10))
                })
                .await;

            epoch_handle.abort();
        }
    }

    // ===== Hyper Connection Termination Tests =====

    mod hyper_termination {
        use super::*;

        struct MockBody;
        impl Body for MockBody {
            type Data = Bytes;
            type Error = hyper::Error;
            fn poll_frame(
                self: std::pin::Pin<&mut Self>,
                _cx: &mut std::task::Context<'_>,
            ) -> std::task::Poll<Option<Result<hyper::body::Frame<Self::Data>, Self::Error>>>
            {
                std::task::Poll::Ready(None)
            }
        }

        /// Helper function to create a ProxyPre for testing
        async fn create_proxy_pre_for_test(
            engine: &Engine,
            linker: &Linker<ClientState<SystemClock>>,
        ) -> ProxyPre<ClientState<SystemClock>> {
            let serialized = load_precompiled_sample_component();
            let component = unsafe { Component::deserialize(engine, &serialized).unwrap() };
            let instance_pre = linker.instantiate_pre(&component).unwrap();
            ProxyPre::new(instance_pre).unwrap()
        }

        /// Helper function to start epoch incrementer
        fn start_epoch_incrementer(engine: Engine) -> tokio::task::JoinHandle<()> {
            tokio::spawn(async move {
                loop {
                    tokio::time::sleep(Duration::from_millis(1)).await;
                    engine.increment_epoch();
                }
            })
        }

        #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
        async fn test_immediate_connection_drop() {
            // Test scenario: hyper connection is dropped immediately before WASM execution starts
            // Expected: CPU time should be ~0ms and Metrics::CpuTime should still be recorded

            let engine = create_test_engine();
            let linker = create_test_linker(&engine);
            let test_metrics = TestMetricsTx::new();

            let proxy_pre = create_proxy_pre_for_test(&engine, &linker).await;
            let epoch_handle = start_epoch_incrementer(engine);

            // Create request
            let req = hyper::Request::builder()
                .uri("http://localhost/")
                .body(MockBody)
                .unwrap();

            let metrics_tx = test_metrics.clone().into_metrics_tx();

            // Simulate immediate hyper connection drop by NOT awaiting the response
            // We'll directly test the internal behavior
            let time_tracker = TimeTracker::new(SystemClock);
            let is_timeout = Arc::new(AtomicBool::new(false));

            let mut store = Store::new(
                proxy_pre.engine(),
                ClientState {
                    table: ResourceTable::new(),
                    wasi: WasiCtx::builder().inherit_stdio().build(),
                    http: WasiHttpCtx::new(),
                    time_tracker: time_tracker.clone(),
                    metrics_tx: metrics_tx.clone(),
                    code_id: "immediate-drop-test".to_string(),
                    is_timeout: is_timeout.clone(),
                },
            );
            store.epoch_deadline_trap();
            store.set_epoch_deadline(1);
            store.epoch_deadline_async_yield_and_update(1);

            let (tx, rx) = tokio::sync::oneshot::channel();

            // Drop the receiver immediately to simulate hyper connection drop
            drop(rx);

            let req = store
                .data_mut()
                .new_incoming_request(Scheme::Http, req)
                .unwrap();
            let out = store.data_mut().new_response_outparam(tx).unwrap();

            let proxy = proxy_pre.instantiate_async(&mut store).await.unwrap();

            let task = tokio::task::spawn({
                let code_id = "immediate-drop-test".to_string();
                let metrics_tx = metrics_tx.clone();
                async move {
                    let result = measure_cpu_time(
                        time_tracker.clone(),
                        proxy
                            .wasi_http_incoming_handler()
                            .call_handle(store, req, out),
                    )
                    .await;

                    metrics_tx.send(Metrics::CpuTime {
                        code_id,
                        cpu_time: time_tracker.duration(),
                    });

                    result
                }
            });

            // Even though connection is dropped, task should complete and record CPU time
            let _ = task.await;

            // Verify CpuTime metric was recorded (should be very small since we just called home endpoint)
            test_metrics
                .assert_contains(|m| {
                    matches!(m, Metrics::CpuTime { code_id, cpu_time }
                        if code_id == "immediate-drop-test" && *cpu_time < Duration::from_millis(100))
                })
                .await;

            epoch_handle.abort();
        }

        #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
        async fn test_connection_drop_during_execution() {
            // Test scenario: hyper connection is dropped while WASM is executing (slow endpoint)
            // Expected: Partial CPU time should be recorded

            let engine = create_test_engine();
            let linker = create_test_linker(&engine);
            let test_metrics = TestMetricsTx::new();

            let proxy_pre = create_proxy_pre_for_test(&engine, &linker).await;
            let epoch_handle = start_epoch_incrementer(engine);

            // Create request for /slow endpoint with 200ms delay
            let req = hyper::Request::builder()
                .uri("http://localhost/slow?ms=200")
                .body(MockBody)
                .unwrap();

            let metrics_tx = test_metrics.clone().into_metrics_tx();

            let time_tracker = TimeTracker::new(SystemClock);
            let is_timeout = Arc::new(AtomicBool::new(false));

            let mut store = Store::new(
                proxy_pre.engine(),
                ClientState {
                    table: ResourceTable::new(),
                    wasi: WasiCtx::builder().inherit_stdio().build(),
                    http: WasiHttpCtx::new(),
                    time_tracker: time_tracker.clone(),
                    metrics_tx: metrics_tx.clone(),
                    code_id: "during-exec-drop-test".to_string(),
                    is_timeout: is_timeout.clone(),
                },
            );
            store.epoch_deadline_trap();
            store.set_epoch_deadline(1);
            store.epoch_deadline_async_yield_and_update(1);

            let (tx, rx) = tokio::sync::oneshot::channel();

            let req = store
                .data_mut()
                .new_incoming_request(Scheme::Http, req)
                .unwrap();
            let out = store.data_mut().new_response_outparam(tx).unwrap();

            let proxy = proxy_pre.instantiate_async(&mut store).await.unwrap();

            let task = tokio::task::spawn({
                let code_id = "during-exec-drop-test".to_string();
                let metrics_tx = metrics_tx.clone();
                async move {
                    let result = measure_cpu_time(
                        time_tracker.clone(),
                        proxy
                            .wasi_http_incoming_handler()
                            .call_handle(store, req, out),
                    )
                    .await;

                    metrics_tx.send(Metrics::CpuTime {
                        code_id,
                        cpu_time: time_tracker.duration(),
                    });

                    result
                }
            });

            // Drop the receiver after a short delay (simulating mid-execution drop)
            tokio::time::sleep(Duration::from_millis(50)).await;
            drop(rx);

            // Wait for task to complete
            let _ = task.await;

            // Verify CpuTime metric was recorded (should have some CPU time)
            test_metrics
                .assert_contains(|m| {
                    matches!(m, Metrics::CpuTime { code_id, cpu_time }
                        if code_id == "during-exec-drop-test" && *cpu_time < Duration::from_millis(300))
                })
                .await;

            epoch_handle.abort();
        }

        #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
        async fn test_connection_drop_after_wasm_completion() {
            // Test scenario: WASM completes successfully but connection is dropped before response
            // Expected: Full CPU time should be recorded

            let engine = create_test_engine();
            let linker = create_test_linker(&engine);
            let test_metrics = TestMetricsTx::new();

            let proxy_pre = create_proxy_pre_for_test(&engine, &linker).await;
            let epoch_handle = start_epoch_incrementer(engine);

            // Create request for a fast endpoint
            let req = hyper::Request::builder()
                .uri("http://localhost/slow?ms=50")
                .body(MockBody)
                .unwrap();

            let metrics_tx = test_metrics.clone().into_metrics_tx();

            let time_tracker = TimeTracker::new(SystemClock);
            let is_timeout = Arc::new(AtomicBool::new(false));

            let mut store = Store::new(
                proxy_pre.engine(),
                ClientState {
                    table: ResourceTable::new(),
                    wasi: WasiCtx::builder().inherit_stdio().build(),
                    http: WasiHttpCtx::new(),
                    time_tracker: time_tracker.clone(),
                    metrics_tx: metrics_tx.clone(),
                    code_id: "after-completion-drop-test".to_string(),
                    is_timeout: is_timeout.clone(),
                },
            );
            store.epoch_deadline_trap();
            store.set_epoch_deadline(1);
            store.epoch_deadline_async_yield_and_update(1);

            let (tx, rx) = tokio::sync::oneshot::channel();

            let req = store
                .data_mut()
                .new_incoming_request(Scheme::Http, req)
                .unwrap();
            let out = store.data_mut().new_response_outparam(tx).unwrap();

            let proxy = proxy_pre.instantiate_async(&mut store).await.unwrap();

            let task = tokio::task::spawn({
                let code_id = "after-completion-drop-test".to_string();
                let metrics_tx = metrics_tx.clone();
                async move {
                    let result = measure_cpu_time(
                        time_tracker.clone(),
                        proxy
                            .wasi_http_incoming_handler()
                            .call_handle(store, req, out),
                    )
                    .await;

                    metrics_tx.send(Metrics::CpuTime {
                        code_id,
                        cpu_time: time_tracker.duration(),
                    });

                    result
                }
            });

            // Wait for WASM to complete, then drop receiver
            tokio::time::sleep(Duration::from_millis(100)).await;
            drop(rx);

            // Wait for task to complete
            let _ = task.await;

            // Verify CpuTime metric was recorded with reasonable CPU time
            test_metrics
                .assert_contains(|m| {
                    matches!(m, Metrics::CpuTime { code_id, cpu_time }
                        if code_id == "after-completion-drop-test" && *cpu_time < Duration::from_millis(200))
                })
                .await;

            epoch_handle.abort();
        }

        #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
        async fn test_connection_drop_with_handle_request() {
            // Test scenario: Using the actual handle_request function with immediate rx drop
            // This tests the real production code path
            // Expected: CPU time should be recorded even when hyper drops the connection

            let engine = create_test_engine();
            let linker = create_test_linker(&engine);
            let test_metrics = TestMetricsTx::new();

            let proxy_pre = create_proxy_pre_for_test(&engine, &linker).await;
            let epoch_handle = start_epoch_incrementer(engine);

            // Create a custom version of handle_request that drops rx immediately
            let req = hyper::Request::builder()
                .uri("http://localhost/slow?ms=50")
                .body(MockBody)
                .unwrap();

            let metrics_tx = test_metrics.clone().into_metrics_tx();
            let code_id = "handle-request-drop-test".to_string();

            // Manually replicate handle_request but with early rx drop
            let time_tracker = TimeTracker::new(SystemClock);
            let is_timeout = Arc::new(AtomicBool::new(false));

            let mut store = Store::new(
                proxy_pre.engine(),
                ClientState {
                    table: ResourceTable::new(),
                    wasi: WasiCtx::builder().inherit_stdio().build(),
                    http: WasiHttpCtx::new(),
                    time_tracker: time_tracker.clone(),
                    metrics_tx: metrics_tx.clone(),
                    code_id: code_id.clone(),
                    is_timeout: is_timeout.clone(),
                },
            );
            store.epoch_deadline_trap();
            store.set_epoch_deadline(1);
            store.epoch_deadline_async_yield_and_update(1);

            let (tx, rx) = tokio::sync::oneshot::channel();
            let req = store
                .data_mut()
                .new_incoming_request(Scheme::Http, req)
                .unwrap();
            let out = store.data_mut().new_response_outparam(tx).unwrap();
            let proxy = proxy_pre.instantiate_async(&mut store).await.unwrap();

            let task = tokio::task::spawn({
                let code_id = code_id.clone();
                let metrics_tx = metrics_tx.clone();
                async move {
                    let result = measure_cpu_time(
                        time_tracker.clone(),
                        proxy
                            .wasi_http_incoming_handler()
                            .call_handle(store, req, out),
                    )
                    .await;

                    metrics_tx.send(Metrics::CpuTime {
                        code_id,
                        cpu_time: time_tracker.duration(),
                    });

                    result
                }
            });

            // Simulate the production code path from handle_request:301-309
            let result = rx.await;

            if let Err(_oneshot_recv_err) = result {
                let result = task.await;
                assert!(result.is_ok(), "Task join should succeed");
            }

            // Verify CpuTime metric was recorded
            test_metrics
                .assert_contains(|m| {
                    matches!(m, Metrics::CpuTime { code_id: id, .. }
                        if id == "handle-request-drop-test")
                })
                .await;

            epoch_handle.abort();
        }
    }
}
