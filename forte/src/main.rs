mod cli;
mod server;

use anyhow::Result;
use clap::Parser;
use cli::{AddCommands, Cli, Commands};

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Commands::Dev { project, port } => {
            let options = cli::dev::DevOptions {
                project_dir: project.unwrap_or_else(|| ".".into()),
                port,
            };
            cli::dev::run(options).await?;
        }

        Commands::Init { name } => {
            cli::init::run(&name)?;
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
            println!("TODO: Build project at '{}'", project_dir.display());
            // TODO: Implement build
        }
    }

    Ok(())
}
