use super::*;
use std::path::PathBuf;

#[derive(Clone)]
pub struct FsCachingProvider {
    base_path: PathBuf,
}

impl FsCachingProvider {
    pub fn new(base_path: PathBuf) -> Self {
        Self { base_path }
    }
}

impl<T> CachingProvider<T> for FsCachingProvider {
    async fn get(&self, id: &str, instantiator: impl Instantiator<T>) -> Result<T> {
        let path = self.base_path.join(id);
        match tokio::fs::read(path).await {
            Ok(code) => {
                let (instance, _size) = instantiator.instantiate(Bytes::from(code))?;
                // TODO: Cache instance
                Ok(instance)
            }
            Err(error) => {
                if error.kind() == std::io::ErrorKind::NotFound {
                    return Err(Error::NotFound);
                }
                Err(Error::ProviderError(anyhow::anyhow!(error)))
            }
        }
    }
}
