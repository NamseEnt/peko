pub mod add;
pub mod dev;
pub mod init;

use clap::{Parser, Subcommand};
use std::path::PathBuf;

#[derive(Parser)]
#[command(name = "forte")]
#[command(about = "Forte - Fullstack Framework", long_about = None)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Subcommand)]
pub enum Commands {
    /// Start the development server with hot reload
    Dev {
        /// Project directory (defaults to current directory)
        #[arg(short, long)]
        project: Option<PathBuf>,

        /// Port to listen on (auto-selects from 3000 if not specified)
        #[arg(short = 'P', long)]
        port: Option<u16>,
    },

    /// Initialize a new Forte project
    Init {
        /// Project name
        name: String,
    },

    /// Add a new page or action
    Add {
        #[command(subcommand)]
        command: AddCommands,
    },

    /// Build the project for production
    Build {
        /// Project directory (defaults to current directory)
        #[arg(short, long)]
        project: Option<PathBuf>,
    },
}

#[derive(Subcommand)]
pub enum AddCommands {
    /// Add a new page
    Page {
        /// Page path (e.g., "product/[id]")
        path: String,
    },

    /// Add a new action
    Action {
        /// Action path (e.g., "user/login")
        path: String,
    },
}
