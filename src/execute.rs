use crate::{
    conversion::json::{convert_json_to_vals, convert_wit_vals_to_json},
    wasm_code_provider::{self, WasmCodeProvider},
};
use wasmtime::{
    component::{Component, Val},
    *,
};

// TODO: Separate Value Conversion from this function
pub async fn execute<Wcp: WasmCodeProvider>(
    engine: Engine,
    wasm_code_provider: Wcp,
    code_id: &str,
    fn_name: &str,
    body_bytes: &[u8],
) -> Result<Vec<u8>, Error> {
    let wasm_code = wasm_code_provider.get_wasm_code(code_id).await?;
    let component = Component::new(&engine, wasm_code)?;
    let linker = component::Linker::new(&engine);
    let mut store = Store::new(&engine, ());
    let instance = linker.instantiate(&mut store, &component)?;

    let Some(func) = instance.get_func(&mut store, fn_name) else {
        return Err(Error::FuncNotFound);
    };

    let params = convert_json_to_vals(func.params(&store), body_bytes)
        .ok_or(Error::ParsingRequestBodyFailed)?;
    let mut results = vec![Val::Bool(true); func.results(&store).len()];

    func.call(&mut store, &params, &mut results)?;

    convert_wit_vals_to_json(results).ok_or(Error::ParsingResponseBodyFailed)
}

pub enum Error {
    WasmCodeProvider(wasm_code_provider::Error),
    Wasmtime(wasmtime::Error),
    FuncNotFound,
    ParsingRequestBodyFailed,
    ParsingResponseBodyFailed,
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
