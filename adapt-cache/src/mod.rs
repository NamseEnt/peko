pub mod fs;
pub mod s3;

use crate::execute::ClientState;
use bytes::Bytes;

pub type Result<T> = std::result::Result<T, Error>;
pub trait CachingProvider<T>: Clone + Send + Sync + 'static {
    fn get(
        &self,
        id: &str,
        instantiator: impl Instantiator<T>,
    ) -> impl Future<Output = Result<T>> + Send;
}

pub trait Instantiator<T>: Send {
    fn instantiate(self, bytes: Bytes) -> Result<(T, usize)>;
}

#[derive(Debug)]
pub enum Error {
    NotFound,
    WasmTime(wasmtime::Error),
    ProviderError(anyhow::Error),
}
impl From<wasmtime::Error> for Error {
    fn from(value: wasmtime::Error) -> Self {
        Self::WasmTime(value)
    }
}
