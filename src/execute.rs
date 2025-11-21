use crate::wasm_code_provider::{self, WasmCodeProvider};
use tokio::sync::{
    mpsc::{Receiver, UnboundedReceiver, UnboundedSender, unbounded_channel},
    oneshot,
};
use wasmtime::{
    component::{Component, Instance, Val},
    *,
};

pub struct Request {
    code_id: String,
    fn_name: String,
    params: Vec<Val>,
    response_tx: oneshot::Sender<Result<Vec<Val>, Error>>,
}

pub struct Executor<Wcp: WasmCodeProvider> {
    engine: Engine,
    wasm_code_provider: Wcp,
    request_rx: Receiver<Request>,
    linker: component::Linker<()>,
    free_instances: Vec<MyInstance>,
    free_instance_tx: UnboundedSender<MyInstance>,
    free_instance_rx: UnboundedReceiver<MyInstance>,
}

impl<Wcp: WasmCodeProvider> Executor<Wcp> {
    pub fn new(engine: Engine, wasm_code_provider: Wcp, request_rx: Receiver<Request>) -> Self {
        let linker = component::Linker::new(&engine);
        let (free_instance_tx, free_instance_rx) = unbounded_channel::<MyInstance>();
        Self {
            engine,
            wasm_code_provider,
            request_rx,
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
            Some(request) = self.request_rx.recv() => {
                self.on_request(request);
            }
        }
    }
    fn on_request(&mut self, request: Request) {
        let free_instance = self.try_pop_free_instance(&request.code_id);
        let wasm_code_provider = self.wasm_code_provider.clone();
        let engine = self.engine.clone();
        let linker = self.linker.clone();
        let free_instance_tx = self.free_instance_tx.clone();

        // TODO: Throttle and hard limit for same code_id

        tokio::spawn(async move {
            let mut instance = if let Some(free_instance) = free_instance {
                free_instance
            } else {
                let result =
                    MyInstance::new(request.code_id, engine, linker, wasm_code_provider).await;
                match result {
                    Ok(instance) => instance,
                    Err(error) => {
                        let _ = request.response_tx.send(Err(error));
                        return;
                    }
                }
            };

            let result = instance.execute(&request.fn_name, &request.params);
            let _ = request.response_tx.send(result);
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

struct MyInstance {
    code_id: String,
    inner: Instance,
    store: Store<()>,
}

impl MyInstance {
    fn execute(&mut self, fn_name: &str, params: &[Val]) -> Result<Vec<Val>, Error> {
        let Some(func) = self.inner.get_func(&mut self.store, fn_name) else {
            return Err(Error::FuncNotFound);
        };

        let mut results = vec![Val::Bool(true); func.results(&self.store).len()];

        func.call(&mut self.store, params, &mut results)?;

        Ok(results)
    }

    async fn new<Wcp: WasmCodeProvider>(
        code_id: String,
        engine: Engine,
        linker: component::Linker<()>,
        wasm_code_provider: Wcp,
    ) -> Result<Self, Error> {
        let wasm_code = wasm_code_provider.get_wasm_code(&code_id).await?;
        let component = Component::new(&engine, wasm_code)?;

        let mut store = Store::new(&engine, ());
        let instance = linker.instantiate(&mut store, &component)?;
        Ok(MyInstance {
            code_id,
            inner: instance,
            store,
        })
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use tokio::sync::mpsc;

    #[derive(Clone)]
    struct MockWasmCodeProvider {
        codes: HashMap<String, Vec<u8>>,
    }

    impl MockWasmCodeProvider {
        fn new() -> Self {
            Self {
                codes: HashMap::new(),
            }
        }

        fn with_code(mut self, id: &str, wasm_bytes: Vec<u8>) -> Self {
            self.codes.insert(id.to_string(), wasm_bytes);
            self
        }
    }

    impl WasmCodeProvider for MockWasmCodeProvider {
        async fn get_wasm_code(&self, id: &str) -> wasm_code_provider::Result<Vec<u8>> {
            self.codes
                .get(id)
                .cloned()
                .ok_or(wasm_code_provider::Error::NotFound)
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

    fn setup_executor() -> (Executor<MockWasmCodeProvider>, mpsc::Sender<Request>) {
        let engine = Engine::default();
        let (tx, rx) = mpsc::channel(100);
        let provider =
            MockWasmCodeProvider::new().with_code("test-component", create_test_component_bytes());
        let executor = Executor::new(engine, provider, rx);
        (executor, tx)
    }

    async fn send_request(
        tx: &mpsc::Sender<Request>,
        code_id: String,
        fn_name: String,
        params: Vec<Val>,
    ) -> Result<Vec<Val>, Error> {
        let (response_tx, response_rx) = oneshot::channel();
        tx.send(Request {
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
    async fn test_executor_basic_request() {
        let (mut executor, tx) = setup_executor();

        tokio::spawn(async move {
            loop {
                executor.run().await;
            }
        });

        let result = send_request(
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

        let add_result = send_request(
            &tx,
            "test-add".to_string(),
            "add".to_string(),
            vec![Val::U32(5), Val::U32(3)],
        )
        .await
        .unwrap();

        assert_eq!(add_result, vec![Val::U32(8)]);

        let mul_result = send_request(
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
    async fn test_executor_sequential_requests() {
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
            let result = send_request(
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
    async fn test_executor_concurrent_requests() {
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
                send_request(
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

        let result = send_request(
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

        let result = send_request(
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
            "test-component".to_string(),
            executor.engine.clone(),
            executor.linker.clone(),
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
            "component-a".to_string(),
            executor.engine.clone(),
            executor.linker.clone(),
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

        let mut instance = MyInstance::new(
            "test-component".to_string(),
            engine.clone(),
            linker.clone(),
            provider.clone(),
        )
        .await
        .unwrap();

        let result = instance
            .execute("add", &[Val::U32(7), Val::U32(3)])
            .unwrap();
        assert_eq!(result, vec![Val::U32(10)]);

        let mut instance2 = MyInstance::new("test-component".to_string(), engine, linker, provider)
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

        let mut instance = MyInstance::new("test-component".to_string(), engine, linker, provider)
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

        let result =
            MyInstance::new("invalid-component".to_string(), engine, linker, provider).await;

        assert!(matches!(result, Err(Error::WasmCodeProvider(_))));
    }
}
