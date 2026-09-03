mod cli;
mod server;
mod sqld;
mod tools;

use anyhow::Result;
use clap::Parser;
use cli::{AddCommands, AdminCommands, Cli, CloudCommands, Commands, DbCommands, EnvCommands};

fn main() -> Result<()> {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    let local = tokio::task::LocalSet::new();
    rt.block_on(local.run_until(async_main()))
}

async fn async_main() -> Result<()> {
    let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
    let cli = Cli::parse();

    match cli.command {
        Commands::Dev { project, port } => {
            let options = cli::dev::DevOptions {
                project_dir: project.unwrap_or_else(|| ".".into()),
                port,
            };
            cli::dev::run(options).await?;
        }

        Commands::Init { name, dev } => {
            cli::init::run(&name, dev)?;
        }

        Commands::Login { token } => {
            cli::login::run(token).await?;
        }

        Commands::Add { command } => match command {
            AddCommands::Page { path } => {
                cli::add::add_page(&path)?;
            }
            AddCommands::Action { path } => {
                cli::add::add_action(&path)?;
            }
        },

        Commands::Build { project } => {
            let project_dir = project.unwrap_or_else(|| ".".into());
            cli::build::run_build(cli::build::BuildOptions {
                project_dir,
                static_base_url: None,
            })
            .await?;
        }

        Commands::Deploy { project } => {
            let project_dir = project.unwrap_or_else(|| ".".into());
            cli::deploy::run(project_dir).await?;
        }

        Commands::Destroy {
            yes,
            delete_buckets,
        } => {
            cli::destroy::run(".".into(), yes, delete_buckets).await?;
        }

        Commands::Open { project, print } => {
            let project_dir = project.unwrap_or_else(|| ".".into());
            cli::open::run(project_dir, print).await?;
        }

        Commands::Purge { keys, project } => {
            let project_dir = project.unwrap_or_else(|| ".".into());
            cli::purge::run(keys, project_dir).await?;
        }

        Commands::PurgePage { paths, project } => {
            let project_dir = project.unwrap_or_else(|| ".".into());
            cli::purge_page::run(paths, project_dir).await?;
        }

        Commands::Admin { command } => match command {
            AdminCommands::Run {
                task,
                project,
                input_file,
                input,
                timeout_seconds,
            } => {
                let project_dir = project.unwrap_or_else(|| ".".into());
                cli::admin::run(task, project_dir, input_file, input, timeout_seconds).await?;
            }
            AdminCommands::RunLocal {
                task,
                port,
                input_file,
                input,
                timeout_seconds,
            } => {
                cli::admin::run_local(task, port, input_file, input, timeout_seconds).await?;
            }
        },

        Commands::Db { command } => match command {
            DbCommands::Query {
                sql,
                args,
                project,
                json,
                timeout_seconds,
            } => {
                let project_dir = project.unwrap_or_else(|| ".".into());
                cli::db::query(sql, args, project_dir, json, timeout_seconds).await?;
            }
            DbCommands::Exec {
                file,
                project,
                json,
                timeout_seconds,
            } => {
                let project_dir = project.unwrap_or_else(|| ".".into());
                cli::db::exec(file, project_dir, json, timeout_seconds).await?;
            }
        },

        Commands::Cloud { command } => match command {
            CloudCommands::Init {
                project,
                project_name,
                zone,
                setup_token_from_clipboard,
            } => {
                let project_dir = project.unwrap_or_else(|| ".".into());
                cli::cloud::init(project_dir, project_name, zone, setup_token_from_clipboard)
                    .await?;
            }
            CloudCommands::Rotate {
                project,
                setup_token_from_clipboard,
            } => {
                let project_dir = project.unwrap_or_else(|| ".".into());
                cli::cloud::rotate(project_dir, setup_token_from_clipboard).await?;
            }
            CloudCommands::Clear { project, yes } => {
                let project_dir = project.unwrap_or_else(|| ".".into());
                cli::cloud::clear(project_dir, yes).await?;
            }
            CloudCommands::Destroy { project, yes } => {
                let project_dir = project.unwrap_or_else(|| ".".into());
                cli::cloud::destroy(project_dir, yes).await?;
            }
        },

        Commands::Env { command } => match command {
            EnvCommands::Set {
                key,
                value,
                secret,
                project,
            } => {
                let project_dir = project.unwrap_or_else(|| ".".into());
                cli::env::set(&project_dir, key, value, secret).await?;
            }
            EnvCommands::Migrate { project } => {
                let project_dir = project.unwrap_or_else(|| ".".into());
                cli::env::migrate(&project_dir)?;
            }
        },
    }

    Ok(())
}
