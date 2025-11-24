use crate::wasm_code_provider::{self, CachingProvider};
use bytes::Bytes;
use http_body_util::BodyExt;
use tokio::sync::mpsc::{Receiver, UnboundedReceiver, UnboundedSender, unbounded_channel};
use wasmtime::{Engine, Store, component::Linker};
use wasmtime_wasi::*;
use wasmtime_wasi_http::{
    WasiHttpCtx, WasiHttpView,
    bindings::{
        ProxyPre,
        http::types::{ErrorCode, Scheme},
    },
    body::HyperOutgoingBody,
};

pub struct Job {
    pub req: hyper::Request<hyper::body::Incoming>,
    pub res: hyper::Response<hyper::body::Incoming>,
    pub code_id: String,
    pub fn_name: String,
}

pub struct Executor<Wcp: CachingProvider> {
    engine: Engine,
    wasm_code_provider: Wcp,
    job_rx: Receiver<Job>,
    linker: Linker<ClientState>,
    free_instances: Vec<MyInstance>,
    free_instance_tx: UnboundedSender<MyInstance>,
    free_instance_rx: UnboundedReceiver<MyInstance>,
}

impl<Wcp: CachingProvider> Executor<Wcp> {
    pub fn new(engine: Engine, wasm_code_provider: Wcp, job_rx: Receiver<Job>) -> Self {
        let mut linker = Linker::new(&engine);
        wasmtime_wasi::p2::add_to_linker_async(&mut linker).unwrap();
        wasmtime_wasi_http::add_only_http_to_linker_async(&mut linker).unwrap();
        let (free_instance_tx, free_instance_rx) = unbounded_channel::<MyInstance>();
        Self {
            engine,
            wasm_code_provider,
            job_rx,
            linker,
            free_instances: Vec::new(),
            free_instance_tx,
            free_instance_rx,
        }
    }
    pub async fn run(&mut self) {
        tokio::select! {
            biased;
            Some(instance) = self.free_instance_rx.recv() => {
                self.free_instances.push(instance)
            }
            Some(job) = self.job_rx.recv() => {
                self.run_job(job);
            }
        }
    }
    fn run_job(&mut self, job: Job) {
        let free_instance = self.try_pop_free_instance(&job.code_id);
        let wasm_code_provider = self.wasm_code_provider.clone();
        let engine = self.engine.clone();
        let linker = self.linker.clone();
        let free_instance_tx = self.free_instance_tx.clone();

        // TODO: Throttle and hard limit for same code_id

        tokio::spawn(async move {
            let instance = if let Some(free_instance) = free_instance {
                free_instance
            } else {
                let result =
                    MyInstance::new(&job.code_id, &engine, &linker, wasm_code_provider).await;
                match result {
                    Ok(instance) => instance,
                    Err(_error) => {
                        // TODO: send error response
                        return;
                    }
                }
            };

            // TODO: call handle_request with the instance.pre and job.req
            // let result = handle_request(instance.pre.clone(), job.req).await;
            let _ = free_instance_tx.send(instance);
        });
    }
    fn try_pop_free_instance(&mut self, code_id: &str) -> Option<MyInstance> {
        let index = self
            .free_instances
            .iter()
            .enumerate()
            .find_map(|(index, instance)| {
                if instance.code_id == code_id {
                    Some(index)
                } else {
                    None
                }
            })?;
        Some(self.free_instances.remove(index))
    }
}

#[derive(Debug)]
pub enum Error {
    WasmCodeProvider(wasm_code_provider::Error),
    Wasmtime(wasmtime::Error),
    FuncNotFound,
}
impl From<wasm_code_provider::Error> for Error {
    fn from(value: wasm_code_provider::Error) -> Self {
        Self::WasmCodeProvider(value)
    }
}
impl From<wasmtime::Error> for Error {
    fn from(value: wasmtime::Error) -> Self {
        Self::Wasmtime(value)
    }
}

struct MyInstance {
    code_id: String,
    pre: ProxyPre<ClientState>,
}

impl MyInstance {
    async fn new<Wcp: CachingProvider>(
        code_id: &str,
        engine: &Engine,
        linker: &Linker<ClientState>,
        wasm_code_provider: Wcp,
    ) -> Result<Self, Error> {
        let proxy_pre = wasm_code_provider.get(code_id, engine, linker).await?;

        Ok(Self {
            code_id: code_id.to_string(),
            pre: proxy_pre,
        })
    }

    async fn handle_request(
        &self,
        req: hyper::Request<hyper::body::Incoming>,
    ) -> Result<hyper::Response<HyperOutgoingBody>, Error> {
        let pre = self.pre.clone();
        let mut store = Store::new(
            pre.engine(),
            ClientState {
                table: ResourceTable::new(),
                wasi: WasiCtx::builder().inherit_stdio().build(),
                http: WasiHttpCtx::new(),
            },
        );
        let (tx, rx) = tokio::sync::oneshot::channel();
        let req = store.data_mut().new_incoming_request(Scheme::Http, req)?;
        let out = store.data_mut().new_response_outparam(tx)?;

        // Run the http request itself in a separate task so the task can
        // optionally continue to execute beyond after the initial
        // headers/response code are sent.
        let task = tokio::task::spawn(async move {
            let proxy = pre.instantiate_async(&mut store).await?;

            proxy
                .wasi_http_incoming_handler()
                .call_handle(store, req, out)
                .await?;

            wasmtime::Result::<()>::Ok(())
        });

        if let Ok(Ok(resp)) = rx.await {
            return Ok(resp);
        }

        task.abort();

        let body = http_body_util::Full::new(Bytes::from("Internal Server Error"))
            .map_err(|_| ErrorCode::InternalError(None));
        let mut res = hyper::Response::new(HyperOutgoingBody::new(body));
        *res.status_mut() = hyper::StatusCode::INTERNAL_SERVER_ERROR;
        Ok(res)
    }
}

pub struct ClientState {
    wasi: WasiCtx,
    http: WasiHttpCtx,
    table: ResourceTable,
}

impl WasiView for ClientState {
    fn ctx(&mut self) -> WasiCtxView<'_> {
        WasiCtxView {
            ctx: &mut self.wasi,
            table: &mut self.table,
        }
    }
}

impl WasiHttpView for ClientState {
    fn ctx(&mut self) -> &mut WasiHttpCtx {
        &mut self.http
    }

    fn table(&mut self) -> &mut ResourceTable {
        &mut self.table
    }
}

// Tests temporarily disabled - need to be rewritten for ProxyPre
/*
#[cfg(test)]
mod tests {
    use super::*;
    use bytes::Bytes;
    use std::collections::HashMap;
    use tokio::sync::mpsc;
    use wasmtime::component::{Component, InstancePre, Linker};

    #[derive(Clone)]
    struct MockWasmCodeProvider {
        codes: HashMap<String, Bytes>,
    }

    impl MockWasmCodeProvider {
        fn new() -> Self {
            Self {
                codes: HashMap::new(),
            }
        }

        fn with_code(mut self, id: &str, wasm_bytes: Vec<u8>) -> Self {
            self.codes.insert(id.to_string(), Bytes::from(wasm_bytes));
            self
        }
    }

    impl WasmCodeProvider for MockWasmCodeProvider {
        async fn get_proxy_pre(
            &self,
            id: &str,
            engine: &Engine,
            linker: &Linker<ClientState>,
        ) -> wasm_code_provider::Result<InstancePre<()>> {
            let bytes = self
                .codes
                .get(id)
                .cloned()
                .ok_or(wasm_code_provider::Error::NotFound)?;

            let component = Component::new(engine, bytes)
                .map_err(|e| wasm_code_provider::Error::ProviderError(e.into()))?;

            let instance_pre = linker
                .instantiate_pre(&component)
                .map_err(|e| wasm_code_provider::Error::ProviderError(e.into()))?;

            Ok(instance_pre)
        }
    }

    fn create_test_component_bytes() -> Vec<u8> {
        let wat = r#"
            (component
                (core module $m
                    (func (export "add") (param i32 i32) (result i32)
                        local.get 0
                        local.get 1
                        i32.add
                    )
                    (func (export "mul") (param i32 i32) (result i32)
                        local.get 0
                        local.get 1
                        i32.mul
                    )
                    (memory (export "memory") 1)
                )
                (core instance $i (instantiate $m))

                (func (export "add") (param "a" u32) (param "b" u32) (result u32)
                    (canon lift (core func $i "add") (memory $i "memory"))
                )
                (func (export "mul") (param "a" u32) (param "b" u32) (result u32)
                    (canon lift (core func $i "mul") (memory $i "memory"))
                )
            )
        "#;

        wat.as_bytes().to_vec()
    }

    fn setup_executor() -> (Executor<MockWasmCodeProvider>, mpsc::Sender<Job>) {
        let engine = Engine::default();
        let (tx, rx) = mpsc::channel(100);
        let provider =
            MockWasmCodeProvider::new().with_code("test-component", create_test_component_bytes());
        let executor = Executor::new(engine, provider, rx);
        (executor, tx)
    }

    async fn send_job(
        tx: &mpsc::Sender<Job>,
        code_id: String,
        fn_name: String,
        params: Vec<Val>,
    ) -> Result<Vec<Val>, Error> {
        let (response_tx, response_rx) = oneshot::channel();
        tx.send(Job {
            code_id,
            fn_name,
            params,
            response_tx,
        })
        .await
        .unwrap();
        response_rx.await.unwrap()
    }

    #[tokio::test]
    async fn test_executor_basic_job() {
        let (mut executor, tx) = setup_executor();

        tokio::spawn(async move {
            loop {
                executor.run().await;
            }
        });

        let result = send_job(
            &tx,
            "test-component".to_string(),
            "add".to_string(),
            vec![Val::U32(10), Val::U32(20)],
        )
        .await
        .unwrap();

        assert_eq!(result, vec![Val::U32(30)]);
    }

    #[tokio::test]
    async fn test_executor_multiple_functions() {
        let engine = Engine::default();
        let (tx, rx) = mpsc::channel(100);
        let provider = MockWasmCodeProvider::new()
            .with_code("test-add", create_test_component_bytes())
            .with_code("test-mul", create_test_component_bytes());
        let executor = Executor::new(engine, provider, rx);

        tokio::spawn(async move {
            let mut executor = executor;
            loop {
                executor.run().await;
            }
        });

        let add_result = send_job(
            &tx,
            "test-add".to_string(),
            "add".to_string(),
            vec![Val::U32(5), Val::U32(3)],
        )
        .await
        .unwrap();

        assert_eq!(add_result, vec![Val::U32(8)]);

        let mul_result = send_job(
            &tx,
            "test-mul".to_string(),
            "mul".to_string(),
            vec![Val::U32(5), Val::U32(3)],
        )
        .await
        .unwrap();

        assert_eq!(mul_result, vec![Val::U32(15)]);
    }

    #[tokio::test]
    async fn test_executor_sequential_jobs() {
        let engine = Engine::default();
        let (tx, rx) = mpsc::channel(100);
        let mut provider = MockWasmCodeProvider::new();
        for i in 0..5 {
            provider = provider.with_code(&format!("test-{}", i), create_test_component_bytes());
        }
        let executor = Executor::new(engine, provider, rx);

        tokio::spawn(async move {
            let mut executor = executor;
            loop {
                executor.run().await;
            }
        });

        for i in 0..5 {
            let result = send_job(
                &tx,
                format!("test-{}", i),
                "add".to_string(),
                vec![Val::U32(i), Val::U32(1)],
            )
            .await
            .unwrap();

            assert_eq!(result, vec![Val::U32(i + 1)]);
        }
    }

    #[tokio::test]
    async fn test_executor_concurrent_jobs() {
        let (mut executor, tx) = setup_executor();

        tokio::spawn(async move {
            loop {
                executor.run().await;
            }
        });

        let mut handles = vec![];

        for i in 0..10 {
            let tx_clone = tx.clone();
            let handle = tokio::spawn(async move {
                send_job(
                    &tx_clone,
                    "test-component".to_string(),
                    "add".to_string(),
                    vec![Val::U32(i), Val::U32(1)],
                )
                .await
            });
            handles.push(handle);
        }

        for (i, handle) in handles.into_iter().enumerate() {
            let result = handle.await.unwrap().unwrap();
            assert_eq!(result, vec![Val::U32(i as u32 + 1)]);
        }
    }

    #[tokio::test]
    async fn test_executor_func_not_found() {
        let (mut executor, tx) = setup_executor();

        tokio::spawn(async move {
            loop {
                executor.run().await;
            }
        });

        let result = send_job(
            &tx,
            "test-component".to_string(),
            "nonexistent".to_string(),
            vec![],
        )
        .await;

        assert!(matches!(result, Err(Error::FuncNotFound)));
    }

    #[tokio::test]
    async fn test_executor_code_not_found() {
        let (mut executor, tx) = setup_executor();

        tokio::spawn(async move {
            loop {
                executor.run().await;
            }
        });

        let result = send_job(
            &tx,
            "nonexistent-component".to_string(),
            "add".to_string(),
            vec![],
        )
        .await;

        assert!(matches!(result, Err(Error::WasmCodeProvider(_))));
    }

    #[tokio::test]
    async fn test_try_pop_free_instance() {
        let engine = Engine::default();
        let (_tx, rx) = mpsc::channel(100);
        let provider =
            MockWasmCodeProvider::new().with_code("test-component", create_test_component_bytes());
        let mut executor = Executor::new(engine, provider, rx);

        assert!(executor.try_pop_free_instance("test-component").is_none());

        let instance = MyInstance::new(
            "test-component",
            &executor.engine,
            &executor.linker,
            executor.wasm_code_provider.clone(),
        )
        .await
        .unwrap();

        executor.free_instances.push(instance);

        let popped = executor.try_pop_free_instance("test-component");
        assert!(popped.is_some());
        assert_eq!(popped.unwrap().code_id, "test-component");

        assert!(executor.try_pop_free_instance("test-component").is_none());
    }

    #[tokio::test]
    async fn test_try_pop_free_instance_different_code_id() {
        let engine = Engine::default();
        let (_tx, rx) = mpsc::channel(100);
        let provider = MockWasmCodeProvider::new()
            .with_code("component-a", create_test_component_bytes())
            .with_code("test-component", create_test_component_bytes());
        let mut executor = Executor::new(engine, provider, rx);

        let instance = MyInstance::new(
            "component-a",
            &executor.engine,
            &executor.linker,
            executor.wasm_code_provider.clone(),
        )
        .await
        .unwrap();

        executor.free_instances.push(instance);

        assert!(executor.try_pop_free_instance("component-b").is_none());
        assert!(executor.try_pop_free_instance("component-a").is_some());
    }

    #[tokio::test]
    async fn test_my_instance_execute() {
        let engine = Engine::default();
        let linker = component::Linker::new(&engine);
        let provider =
            MockWasmCodeProvider::new().with_code("test-component", create_test_component_bytes());

        let mut instance = MyInstance::new("test-component", &engine, &linker, provider.clone())
            .await
            .unwrap();

        let result = instance
            .execute("add", &[Val::U32(7), Val::U32(3)])
            .unwrap();
        assert_eq!(result, vec![Val::U32(10)]);

        let mut instance2 = MyInstance::new("test-component", &engine, &linker, provider.clone())
            .await
            .unwrap();

        let result = instance2
            .execute("mul", &[Val::U32(7), Val::U32(3)])
            .unwrap();
        assert_eq!(result, vec![Val::U32(21)]);
    }

    #[tokio::test]
    async fn test_my_instance_execute_func_not_found() {
        let engine = Engine::default();
        let linker = component::Linker::new(&engine);
        let provider =
            MockWasmCodeProvider::new().with_code("test-component", create_test_component_bytes());

        let mut instance = MyInstance::new("test-component", &engine, &linker, provider.clone())
            .await
            .unwrap();

        let result = instance.execute("nonexistent", &[]);
        assert!(matches!(result, Err(Error::FuncNotFound)));
    }

    #[tokio::test]
    async fn test_my_instance_new_invalid_code() {
        let engine = Engine::default();
        let linker = component::Linker::new(&engine);
        let provider = MockWasmCodeProvider::new();

        let result = MyInstance::new("invalid-component", &engine, &linker, provider.clone()).await;

        assert!(matches!(result, Err(Error::WasmCodeProvider(_))));
    }
}
*/
