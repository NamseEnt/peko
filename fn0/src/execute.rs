use crate::{Body, Request, Response, telemetry};
use adapt_cache::AdaptCache;
use anyhow::{Result, anyhow};
use bytes::Bytes;
use http_body_util::BodyExt;
use measure_cpu_time::{Clock, TimeTracker, measure_cpu_time};
use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};
use tokio::sync::{mpsc::Sender, oneshot};
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
};

pub struct Job {
    pub req: Request,
    pub res_tx: oneshot::Sender<Response>,
    pub code_id: String,
}

pub struct WasmExecutor {
    job_tx: Sender<Job>,
}

impl WasmExecutor {
    pub fn new<A, C>(proxy_cache: A, clock: C) -> Self
    where
        A: AdaptCache<ProxyPre<ClientState<C>>, wasmtime::Error>,
        C: Clock,
    {
        let (job_tx, mut job_rx) = tokio::sync::mpsc::channel(10 * 1024);
        let engine = Engine::new(&engine_config()).unwrap();

        let mut linker = Linker::new(&engine);
        wasmtime_wasi::p2::add_to_linker_async(&mut linker).unwrap();
        wasmtime_wasi_http::add_only_http_to_linker_async(&mut linker).unwrap();

        tokio::spawn({
            let proxy_cache = proxy_cache.clone();
            let engine = engine.clone();
            let linker = linker.clone();
            let clock = clock.clone();

            async move {
                let mut interval = tokio::time::interval(Duration::from_millis(3));
                loop {
                    tokio::select! {
                        _ = interval.tick() => {
                            engine.increment_epoch();
                        }

                        res = job_rx.recv() => {
                            match res {
                                Some(job) => {
                                    let proxy_cache = proxy_cache.clone();
                                    let engine = engine.clone();
                                    let linker = linker.clone();
                                    let clock = clock.clone();

                                    tokio::spawn(async move {
                                        run_job(job, proxy_cache, engine, linker, clock).await;
                                    });
                                },
                                None => break,
                            }
                        }
                    }
                }
            }
        });

        Self { job_tx }
    }

    pub(crate) async fn run(&self, code_id: &str, request: Request) -> Result<Response> {
        let (res_tx, res_rx) = oneshot::channel();
        let job = Job {
            req: request,
            res_tx,
            code_id: code_id.to_string(),
        };

        self.job_tx
            .send(job)
            .await
            .map_err(|_| anyhow!("job_tx closed"))?;

        Ok(res_rx.await?)
    }
}

pub fn engine_config() -> Config {
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
        .wasm_component_model(true)
        .cranelift_opt_level(wasmtime::OptLevel::None)
        .cache(Some(
            wasmtime::Cache::new(wasmtime::CacheConfig::new()).unwrap(),
        ))
        .parallel_compilation(true);

    config
}

async fn run_job<A, C>(
    job: Job,
    proxy_cache: A,
    engine: Engine,
    linker: Linker<ClientState<C>>,
    clock: C,
) where
    A: AdaptCache<ProxyPre<ClientState<C>>, wasmtime::Error>,
    C: Clock,
{
    let Ok(proxy_pre) = get_proxy_pre(job.code_id.clone(), proxy_cache, engine, linker).await
    else {
        let _ = job.res_tx.send(internal_error_response());
        return;
    };

    let response = handle_request(proxy_pre, job.req, job.code_id, clock).await;

    let _ = job.res_tx.send(response);
}

async fn get_proxy_pre<A, C>(
    code_id: String,
    proxy_cache: A,
    engine: Engine,
    linker: Linker<ClientState<C>>,
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

            telemetry::create_instance(&code_id);
            Ok((proxy_pre, bytes.len()))
        })
        .await
    {
        Ok(proxy_pre) => Ok(proxy_pre),
        Err(error) => {
            telemetry::proxy_cache_error(&code_id, &format!("{error:?}"));
            Err(())
        }
    }
}

async fn handle_request<C>(
    pre: ProxyPre<ClientState<C>>,
    req: Request,
    code_id: String,
    clock: C,
) -> Response
where
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
            if cpu_time > Duration::from_millis(1000) {
                telemetry::cpu_timeout(&state.code_id, cpu_time);
                state.is_timeout.store(true, Ordering::Relaxed);
                return Ok(wasmtime::UpdateDeadline::Interrupt);
            }
            Ok(wasmtime::UpdateDeadline::Continue(1))
        }
    });

    let (tx, rx) = tokio::sync::oneshot::channel();
    let req: wasmtime::component::Resource<wasmtime_wasi_http::types::HostIncomingRequest> =
        match store.data_mut().new_incoming_request(
            Scheme::Http,
            req.map(|body| {
                body.map_err(|err| ErrorCode::InternalError(Some(err.to_string())))
                    .boxed_unsync()
            }),
        ) {
            Ok(x) => x,
            Err(error) => {
                telemetry::wasmtime_error("new_incoming_request", &code_id, &format!("{error:?}"));
                return internal_error_response();
            }
        };
    let out = match store.data_mut().new_response_outparam(tx) {
        Ok(x) => x,
        Err(error) => {
            telemetry::wasmtime_error("new_response_outparam", &code_id, &format!("{error:?}"));
            return internal_error_response();
        }
    };

    // NOTE: Bad guys can put infinite loop in initialization code so it would be needed to measure cpu time from here.
    // But also it includes wasmtime's instantiation.
    // I guess we can put limit with fuel for initialization.
    let proxy = match pre.instantiate_async(&mut store).await {
        Ok(x) => x,
        Err(error) => {
            telemetry::wasmtime_error("instantiate_async", &code_id, &format!("{error:?}"));
            return internal_error_response();
        }
    };

    let task = tokio::task::spawn({
        let code_id = code_id.clone();
        async move {
            let result = measure_cpu_time(
                time_tracker.clone(),
                proxy
                    .wasi_http_incoming_handler()
                    .call_handle(store, req, out),
            )
            .await;

            telemetry::cpu_time(&code_id, time_tracker.duration());

            result
        }
    });

    let result = rx.await;

    if let Err(_oneshot_recv_err) = result {
        let result = task.await;
        if let Err(error) = result {
            telemetry::request_task_join_error(&code_id, &format!("{error:?}"));
            return internal_error_response();
        }
        let result = result.unwrap();

        if let Err(error) = result {
            match error.downcast::<wasmtime::Trap>() {
                Ok(trap) => {
                    telemetry::trapped(&code_id, &format!("{trap:?}"));
                }
                Err(error) => {
                    telemetry::canceled_unexpectedly(&code_id, &format!("{error:?}"));
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
        return response.map(|body| {
            body.map_err(|error_code| anyhow!("error_code: {error_code:?}"))
                .boxed_unsync()
        });
    }

    let error_code: ErrorCode = result.unwrap_err();

    telemetry::proxy_returns_error_code(&code_id, &format!("{error_code:?}"));
    internal_error_response()
}

fn response(status: hyper::StatusCode, body: Bytes) -> Response {
    let body = http_body_util::Full::new(body).map_err(|never| match never {});
    let mut res = hyper::Response::new(Body::new(body));
    *res.status_mut() = status;
    res
}

fn timeout_response() -> Response {
    response(
        hyper::StatusCode::GATEWAY_TIMEOUT,
        Bytes::from("Gateway Timeout"),
    )
}

fn internal_error_response() -> Response {
    response(
        hyper::StatusCode::INTERNAL_SERVER_ERROR,
        Bytes::from("Internal Server Error"),
    )
}

pub struct ClientState<C: Clock> {
    wasi: WasiCtx,
    http: WasiHttpCtx,
    table: ResourceTable,
    time_tracker: TimeTracker<C>,
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
