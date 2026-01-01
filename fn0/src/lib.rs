mod deployment;
mod execute;
pub mod telemetry;

use adapt_cache::AdaptCache;
use anyhow::*;
use bytes::Bytes;
use deployment::*;
use execute::*;
pub use deployment::{CodeKind, DeploymentMap};
use http_body_util::combinators::UnsyncBoxBody;
use measure_cpu_time::SystemClock;
use std::string::FromUtf8Error;
use wasmtime::Engine;
use wasmtime_wasi_http::bindings::ProxyPre;

pub type Body = UnsyncBoxBody<Bytes, anyhow::Error>;
pub type Request = hyper::Request<Body>;
pub type Response = hyper::Response<Body>;

pub struct Fn0<J>
where
    J: AdaptCache<String, FromUtf8Error>,
{
    js_cache: J,
    deployment_map: DeploymentMap,
    wasm_executor: WasmExecutor,
}

impl<J> Fn0<J>
where
    J: AdaptCache<String, FromUtf8Error>,
{
    pub fn new<W>(wasm_proxy_cache: W, js_cache: J, deployment_map: DeploymentMap) -> Self
    where
        W: AdaptCache<ProxyPre<ClientState<SystemClock>>, wasmtime::Error>,
    {
        Self {
            js_cache,
            deployment_map,
            wasm_executor: WasmExecutor::new(wasm_proxy_cache, SystemClock),
        }
    }
    pub async fn run(&self, code_id: &str, request: Request) -> Result<Response> {
        let Some(code_kind) = self.deployment_map.code_kind(code_id) else {
            return Err(anyhow!("code_id not found"));
        };
        match code_kind {
            CodeKind::Wasm => Ok(self.wasm_executor.run(code_id, request).await?),
            CodeKind::Js => {
                let js_code = self
                    .js_cache
                    .get(code_id, |bytes| {
                        String::from_utf8(bytes.to_vec()).map(|str| (str, bytes.len()))
                    })
                    .await
                    .map_err(|err| anyhow!("Failed to get JS code: {:?}", err))?;
                let response = ski::run(&js_code, request).await?;
                Ok(response)
            }
        }
    }
}

pub fn compile(wasm_bytes: &[u8]) -> Result<Vec<u8>> {
    let engine = Engine::new(&engine_config())?;

    // Check if it's a component by looking for component-specific markers
    if wasm_bytes.len() > 8 && wasm_bytes[4..8] == [0x0d, 0x00, 0x01, 0x00] {
        // This is a WebAssembly Component
        engine.precompile_component(wasm_bytes)
    } else {
        // This is a standard WebAssembly Module
        engine.precompile_module(wasm_bytes)
    }
}
