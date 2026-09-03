use anyhow::{Result, anyhow};
use std::path::PathBuf;

use super::project_config::{CloudConfig, clear_cloud_config, read_cloud_config};
use fn0_deploy::{BrokerClient, credentials::Credentials};

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

    // fn0-side teardown (routing, bundles, buckets emptied, database) runs on
    // the control plane; the Cloudflare footprint it cannot reach is cleaned
    // through the broker here.
    fn0_deploy::delete_project(&project_id).await?;

    if let Err(error) = teardown_cloudflare(&config, &project_id, delete_buckets).await {
        eprintln!(
            "warning: fn0-side teardown is enqueued, but cleaning up the project's Cloudflare \
             resources through the setup broker failed ({error}). Remove the app DNS record, the \
             two bucket custom domains, the origin certificate, and the `fn0 worker/frontend \
             assets/cache purge ({project_id})` tokens yourself."
        );
    }

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
            delete_buckets,
        )
        .await?;
    println!(
        "  DNS record, bucket custom domains, origin certificate, and minted tokens removed{}",
        if delete_buckets {
            "; buckets deleted"
        } else {
            ""
        }
    );
    for note in &outcome.notes {
        println!("  note: {note}");
    }
    Ok(())
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
