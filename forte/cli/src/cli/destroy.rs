use anyhow::{Result, anyhow};
use std::path::PathBuf;

use super::project_config::{CloudConfig, clear_cloud_config, read_cloud_config};
use fn0_deploy::{BrokerClient, DomainStatus, credentials::Credentials};

pub async fn run(project_dir: PathBuf, yes: bool, delete_buckets: bool) -> Result<()> {
    let config = read_cloud_config(&project_dir)?;
    let project_id = config
        .project_id
        .clone()
        .ok_or_else(|| anyhow!("'project_id' field missing in Forte.toml. Nothing to destroy."))?;

    if !yes {
        let bucket_line = if delete_buckets {
            "\n             --delete-buckets: the three R2 buckets are deleted too, contents and all."
        } else {
            ""
        };
        println!(
            "This permanently deletes project '{project_id}' and ALL of its resources:\n\
             routing, custom domain, deployed bundles, static assets, object storage, and its database.{bucket_line}"
        );
        let answer = inquire::Text::new("Type the project id to confirm:").prompt()?;
        if answer.trim() != project_id {
            return Err(anyhow!(
                "confirmation did not match '{project_id}'; aborted."
            ));
        }
    }

    let origin_hostname = expected_origin_hostname(&config, &project_id).await?;

    // fn0-side teardown (routing, bundles, buckets emptied, database) runs on
    // the control plane; the Cloudflare footprint it cannot reach is cleaned
    // through the broker here.
    fn0_deploy::delete_project_if_present(&project_id).await?;

    teardown_cloudflare(
        &config,
        &project_id,
        origin_hostname.as_deref(),
        delete_buckets,
    )
    .await?;

    clear_cloud_config(&project_dir)?;
    println!(
        "Removed cloud configuration from Forte.toml (next `forte deploy` creates a new project)"
    );
    println!("Teardown of '{project_id}' enqueued; resources are being deleted.");
    Ok(())
}

async fn teardown_cloudflare(
    config: &CloudConfig,
    project_id: &str,
    origin_hostname: Option<&str>,
    delete_buckets: bool,
) -> Result<()> {
    let (Some(zone_name), Some(app_hostname)) = (config.zone.as_deref(), config.domain.as_deref())
    else {
        return Ok(());
    };
    let creds = fn0_deploy::credentials::require()?;
    let Some(broker) = load_broker(config, &creds)? else {
        return Ok(());
    };

    println!("cleaning up the project's Cloudflare resources through the setup broker...");
    let zone = broker.resolve_zone(zone_name).await?;
    let outcome = broker
        .teardown_project(
            project_id,
            &zone.zone_id,
            zone_name,
            app_hostname,
            origin_hostname,
            delete_buckets,
        )
        .await?;
    println!(
        "  DNS record, bucket custom domains, origin certificate, and minted tokens cleaned up{}",
        if delete_buckets {
            "; bucket deletion requested"
        } else {
            ""
        }
    );
    for note in &outcome.notes {
        println!("  note: {note}");
    }
    Ok(())
}

async fn expected_origin_hostname(
    config: &CloudConfig,
    project_id: &str,
) -> Result<Option<String>> {
    if config.zone.is_none() || config.domain.is_none() {
        return Ok(None);
    }
    let creds = fn0_deploy::credentials::require()?;
    let configured_domain = config.domain.as_deref().unwrap_or_default();
    match fn0_deploy::fetch_domain_status(&creds, project_id).await? {
        DomainStatus::SelfHosted {
            domain,
            origin_hostname,
            ..
        } if domain == configured_domain && !origin_hostname.is_empty() => {
            Ok(Some(origin_hostname))
        }
        DomainStatus::NoDomain => Ok(None),
        DomainStatus::SelfHosted { .. } => Ok(None),
        DomainStatus::NotLoggedIn => Err(anyhow!("control rejected token; run `fn0 login` again.")),
        DomainStatus::NotFound => Ok(None),
        DomainStatus::InternalError => Err(anyhow!(
            "domain_status: server error; check fn0-control logs"
        )),
    }
}

fn load_broker(config: &CloudConfig, creds: &Credentials) -> Result<Option<BrokerClient>> {
    match (
        config.cloudflare_account_id.clone(),
        config.cloudflare_broker_url.clone(),
    ) {
        (Some(account_id), Some(broker_url)) => Ok(Some(BrokerClient::new(
            broker_url,
            creds.control_url.clone(),
            creds.token.clone(),
            account_id,
        )?)),
        (None, None) => match fn0_deploy::load_broker_settings()? {
            Some(settings) => Ok(Some(BrokerClient::new(
                settings.broker_url,
                creds.control_url.clone(),
                creds.token.clone(),
                settings.account_id,
            )?)),
            None => Ok(None),
        },
        _ => Err(anyhow!(
            "Forte.toml must contain both cloudflare_account_id and cloudflare_broker_url"
        )),
    }
}
