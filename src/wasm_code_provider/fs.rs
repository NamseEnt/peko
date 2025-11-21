use super::*;
use std::path::PathBuf;

#[derive(Clone)]
pub struct FsCodeProvider {
    base_path: PathBuf,
}

impl WasmCodeProvider for FsCodeProvider {
    async fn get_wasm_code(&self, id: &str) -> Result<Vec<u8>> {
        let path = self.base_path.join(id);
        match tokio::fs::read(path).await {
            Ok(code) => Ok(code),
            Err(error) => {
                if error.kind() == std::io::ErrorKind::NotFound {
                    return Err(Error::NotFound);
                }
                Err(Error::IoError { error })
            }
        }
    }
}
