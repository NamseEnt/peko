pub mod fs;

#[derive(Debug)]
pub enum Error {
    NotFound,
    IoError { error: std::io::Error },
}

pub type Result<T> = std::result::Result<T, Error>;

pub trait WasmCodeProvider: Clone + Send + Sync + 'static {
    fn get_wasm_code(&self, id: &str) -> impl Future<Output = Result<Vec<u8>>> + Send;
}
