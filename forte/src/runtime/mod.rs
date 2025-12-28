use anyhow::{Context, Result};
use http_body_util::{BodyExt, Full};
use hyper::body::Bytes;
use std::path::Path;
use std::sync::Arc;
use wasmtime::component::*;
use wasmtime::{Config, Engine, Store};
use wasmtime_wasi::{ResourceTable, WasiCtx, WasiCtxBuilder, WasiView};
use wasmtime_wasi_http::{body::HyperOutgoingBody, WasiHttpCtx, WasiHttpView};

// wasmtime-wasi-http가 제공하는 bindings 사용
use wasmtime_wasi_http::bindings::{http::types as http_types, ProxyPre};

/// WASM 런타임 상태
pub struct ServerState {
    wasi: WasiCtx,
    http: WasiHttpCtx,
    table: ResourceTable,
}

impl WasiView for ServerState {
    fn ctx(&mut self) -> &mut WasiCtx {
        &mut self.wasi
    }

    fn table(&mut self) -> &mut ResourceTable {
        &mut self.table
    }
}

impl WasiHttpView for ServerState {
    fn ctx(&mut self) -> &mut WasiHttpCtx {
        &mut self.http
    }

    fn table(&mut self) -> &mut ResourceTable {
        &mut self.table
    }
}

/// Wasmtime을 사용한 WASM 런타임
pub struct WasmRuntime {
    engine: Engine,
    proxy_pre: ProxyPre<ServerState>,
    linker: Arc<Linker<ServerState>>,
}

impl WasmRuntime {
    /// WASM 파일로부터 런타임 생성
    pub fn new(wasm_path: &Path) -> Result<Self> {
        let mut config = Config::new();
        config.wasm_component_model(true);
        config.async_support(true);

        let engine = Engine::new(&config)?;
        let component = Component::from_file(&engine, wasm_path)
            .with_context(|| format!("Failed to load WASM from {}", wasm_path.display()))?;

        let mut linker = Linker::new(&engine);
        wasmtime_wasi::add_to_linker_async(&mut linker)?;
        wasmtime_wasi_http::add_only_http_to_linker_async(&mut linker)?;

        let proxy_pre = ProxyPre::new(linker.instantiate_pre(&component)?)?;

        Ok(Self {
            engine,
            proxy_pre,
            linker: Arc::new(linker),
        })
    }

    /// WASM 모듈 리로드 (핫 리로드용)
    pub fn reload(&mut self, wasm_path: &Path) -> Result<()> {
        let new_component = Component::from_file(&self.engine, wasm_path)
            .with_context(|| format!("Failed to reload WASM from {}", wasm_path.display()))?;

        let new_proxy_pre = ProxyPre::new(self.linker.instantiate_pre(&new_component)?)?;
        self.proxy_pre = new_proxy_pre;

        println!("✓ WASM module reloaded from {}", wasm_path.display());

        Ok(())
    }

    /// HTTP 요청 처리 - wasmtime-wasi-http를 사용하여 실제 WASM 호출
    pub async fn handle_request(
        &self,
        req: hyper::Request<hyper::body::Incoming>,
    ) -> Result<hyper::Response<Full<Bytes>>> {
        // Create per-request state
        let mut store = Store::new(
            &self.engine,
            ServerState {
                wasi: WasiCtxBuilder::new()
                    .inherit_stdio()
                    .inherit_env()
                    .build(),
                http: WasiHttpCtx::new(),
                table: ResourceTable::new(),
            },
        );

        // Create oneshot channel for response
        let (sender, receiver) = tokio::sync::oneshot::channel();

        // Convert to WASI request using wasmtime-wasi-http helpers
        // The request body is hyper::body::Incoming which has Error = hyper::Error
        let incoming_req = store
            .data_mut()
            .new_incoming_request(http_types::Scheme::Http, req)
            .context("Failed to create incoming request")?;

        let response_out = store
            .data_mut()
            .new_response_outparam(sender)
            .context("Failed to create response outparam")?;

        // Instantiate the component and call the handler
        let proxy = self
            .proxy_pre
            .instantiate_async(&mut store)
            .await
            .context("Failed to instantiate component")?;

        // Spawn task to call the WASM handler
        let task = wasmtime_wasi::runtime::spawn(async move {
            proxy
                .wasi_http_incoming_handler()
                .call_handle(&mut store, incoming_req, response_out)
                .await
                .context("Failed to call WASM handler")?;
            Ok::<_, anyhow::Error>(())
        });

        // Wait for response from the channel
        let response = match receiver.await {
            Ok(Ok(resp)) => resp,
            Ok(Err(e)) => {
                anyhow::bail!("Component returned error: {:?}", e);
            }
            Err(_) => {
                // Channel closed without sending - wait for task to get error
                let _ = task.await;
                anyhow::bail!("Component did not send response");
            }
        };

        // Convert WASI response back to hyper response
        let (resp_parts, resp_body) = response.into_parts();

        // Collect the body - resp_body is BoxBody, not Option
        let body_bytes = resp_body
            .collect()
            .await
            .context("Failed to collect response body")?
            .to_bytes();

        let hyper_resp = hyper::Response::from_parts(resp_parts, Full::new(body_bytes));

        Ok(hyper_resp)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_wasm_runtime_creation() {
        // 테스트용 WASM 파일이 있다면 로드 테스트
    }
}
