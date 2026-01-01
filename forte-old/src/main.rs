mod codegen;
mod commands;
mod config;
mod error_formatter;
mod generator;
mod parser;
mod runtime;
mod server;
mod templates;
mod watcher;

use anyhow::Result;
use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "forte")]
#[command(about = "A full-stack Rust+React framework with type-safe routing", long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Initialize a new Forte project
    Init {
        /// Project name
        project_name: String,
    },
    /// Start development server
    Dev,
    /// Build for production
    Build,
    /// Run tests
    Test,
    /// Generate new route
    Generate {
        /// Type of resource to generate (e.g., "route")
        resource_type: String,
        /// Route path (e.g., "product/_id_")
        path: String,
    },
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Commands::Init { project_name } => {
            commands::init::execute(&project_name)?;
        }
        Commands::Dev => {
            commands::dev::execute()?;
        }
        Commands::Build => {
            commands::build::execute()?;
        }
        Commands::Test => {
            commands::test::execute()?;
        }
        Commands::Generate { resource_type, path } => {
            commands::generate::execute(&resource_type, &path)?;
        }
    }

    Ok(())
}
