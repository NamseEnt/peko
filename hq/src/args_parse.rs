use color_eyre::eyre::{Result, eyre};

use crate::{
    args::*,
    deployment_db::DeploymentDb,
    dns::{DnsProvider, cloudflare::CloudflareDnsProvider},
    host_provider::{HostProvider, oci_container::OciContainerInstanceHostProvider},
    site::Site,
};

pub struct HqArgsParsed {
    pub sites: Vec<Site>,
    pub deployment_db: DeploymentDb,
}

impl HqArgs {
    pub async fn parse() -> Result<HqArgsParsed> {
        let path = std::env::var("HQ_ARGS_PATH").unwrap_or_else(|_| "hq-args.json".to_string());

        let content = std::fs::read_to_string(&path)
            .map_err(|e| eyre!("Failed to read config file at {}: {}", path, e))?;

        let args: HqArgs = serde_json::from_str(&content)
            .map_err(|e| eyre!("Failed to parse config file: {}", e))?;

        let deployment_db =
            DeploymentDb::new(args.deployment_db.url, args.deployment_db.token).await?;

        let sites = args
            .sites
            .into_iter()
            .map(|args| {
                let host_provider = match args.host_provider {
                    HostProviderArg::OciContainerInstance(args) => {
                        HostProvider::OciContainerInstance(OciContainerInstanceHostProvider::new(
                            args,
                        ))
                    }
                };
                let dns_provider = match args.dns_provider {
                    DnsProviderArg::Cloudflare(args) => {
                        DnsProvider::Cloudflare(CloudflareDnsProvider::new(args, None))
                    }
                };
                Site::new(
                    host_provider,
                    dns_provider,
                    args.cert,
                    deployment_db.clone(),
                )
            })
            .collect();

        Ok(HqArgsParsed {
            sites,
            deployment_db,
        })
    }
}
