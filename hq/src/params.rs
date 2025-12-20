use crate::args::*;
use crate::host_provider::{HostProvider, oci_container::OciContainerInstanceHostProvider};
use color_eyre::Result;
use std::sync::Arc;

pub struct HqParams {
    pub host_providers: Vec<Arc<dyn HostProvider>>,
}
impl HqParams {
    pub fn load() -> Result<Self> {
        let path = std::env::var("HOST_PROVIDER_CONFIG_PATH")
            .unwrap_or_else(|_| "host-provider.json".to_string());

        let content = std::fs::read_to_string(&path).map_err(|e| {
            color_eyre::eyre::eyre!("Failed to read config file at {}: {}", path, e)
        })?;

        let args: HqArgs = serde_json::from_str(&content)
            .map_err(|e| color_eyre::eyre::eyre!("Failed to parse config file: {}", e))?;

        Ok(HqParams {
            host_providers: args
                .providers
                .into_iter()
                .map(|provider| -> Result<Arc<dyn HostProvider>> {
                    match provider {
                        HostProviderVariant::OciContainerInstance(args) => {
                            let provider = OciContainerInstanceHostProvider::new(args)?;
                            Ok(Arc::new(provider) as Arc<dyn HostProvider>)
                        }
                    }
                })
                .collect::<Result<Vec<_>>>()?,
        })
    }
}
