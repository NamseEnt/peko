pub mod add;
pub mod admin;
pub mod build;
pub mod cloud;
pub mod cron;
pub mod db;
pub mod deploy;
pub mod destroy;
pub mod dev;
pub mod env;
pub mod fe_runtime;
pub mod init;
pub mod login;
pub mod open;
pub mod project_config;
pub mod purge;
pub mod purge_page;

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
    Dev {
        #[arg(short, long)]
        project: Option<PathBuf>,
        #[arg(short = 'P', long)]
        port: Option<u16>,
    },
    Init {
        name: String,
        #[arg(long)]
        dev: bool,
    },
    Login {
        #[arg(long)]
        token: Option<String>,
    },
    Add {
        #[command(subcommand)]
        command: AddCommands,
    },
    Build {
        #[arg(short, long)]
        project: Option<PathBuf>,
    },
    /// Build and deploy this project
    ///
    /// Never prompts. A project that has not been through `forte cloud init`
    /// is refused rather than set up here, so this behaves the same in CI as
    /// it does on a terminal.
    Deploy {
        #[arg(short, long)]
        project: Option<PathBuf>,
    },
    /// Delete the deployed project and all of its resources
    Destroy {
        #[arg(long)]
        yes: bool,
    },
    /// Print the deployed app URL and open it in the browser
    Open {
        #[arg(short, long)]
        project: Option<PathBuf>,
        /// Print the URL without opening a browser
        #[arg(long)]
        print: bool,
    },
    /// Invalidate the edge copy of public objects
    Purge {
        /// Keys inside the project's public namespace, e.g. captures/1/0.mp4
        #[arg(required = true)]
        keys: Vec<String>,
        #[arg(short, long)]
        project: Option<PathBuf>,
    },
    /// Invalidate the edge copy of `cache_static` pages
    PurgePage {
        /// Route paths as a visitor requests them, e.g. /episode/1
        #[arg(required = true)]
        paths: Vec<String>,
        #[arg(short, long)]
        project: Option<PathBuf>,
    },
    Admin {
        #[command(subcommand)]
        command: AdminCommands,
    },
    /// Run SQL against the project's cloud doc-db through control
    Db {
        #[command(subcommand)]
        command: DbCommands,
    },
    /// Set this project up on your own Cloudflare account
    Cloud {
        #[command(subcommand)]
        command: CloudCommands,
    },
    Env {
        #[command(subcommand)]
        command: EnvCommands,
    },
}

#[derive(Subcommand)]
pub enum EnvCommands {
    /// Set an env entry (plain by default, encrypted when --secret)
    Set {
        key: String,
        value: String,
        #[arg(long)]
        secret: bool,
        #[arg(short, long)]
        project: Option<PathBuf>,
    },
    /// Convert a legacy .env file into env.local.yaml
    Migrate {
        #[arg(short, long)]
        project: Option<PathBuf>,
    },
}

#[derive(Subcommand)]
pub enum CloudCommands {
    /// Give this project an identity, a Cloudflare account and a domain
    Init {
        #[arg(short, long)]
        project: Option<PathBuf>,
        #[arg(
            long,
            help = "Project identity and DNS label; required for a new project"
        )]
        project_name: Option<String>,
        #[arg(
            long,
            help = "Cloudflare zone name, not a zone ID; required for a new project"
        )]
        zone: Option<String>,
        #[arg(
            long,
            help = "Read the first-time setup token from the clipboard instead of a prompt, for when an AI agent creates it in the Cloudflare dashboard"
        )]
        setup_token_from_clipboard: bool,
    },
    #[command(about = "Replace the setup token stored by the Cloudflare broker")]
    Rotate {
        #[arg(short, long)]
        project: Option<PathBuf>,
        #[arg(
            long,
            help = "Read the replacement setup token from the clipboard instead of a prompt"
        )]
        setup_token_from_clipboard: bool,
    },
    #[command(about = "Delete the broker setup secret and revoke its token")]
    Clear {
        #[arg(short, long)]
        project: Option<PathBuf>,
        #[arg(long)]
        yes: bool,
    },
    #[command(
        about = "Delete the broker Worker, its Secrets Store, and the setup token stored in it"
    )]
    Destroy {
        #[arg(short, long)]
        project: Option<PathBuf>,
        #[arg(long)]
        yes: bool,
    },
}

#[derive(Subcommand)]
pub enum DbCommands {
    /// Run one SQL statement and print its rows and row read/write counts
    Query {
        sql: String,
        /// Bind a `?` placeholder; parsed as JSON, or taken as a plain string
        #[arg(long = "arg")]
        args: Vec<String>,
        #[arg(short, long)]
        project: Option<PathBuf>,
        #[arg(long)]
        json: bool,
        #[arg(long, default_value_t = 300)]
        timeout_seconds: u64,
    },
    /// Run every statement of a SQL file as one transaction
    Exec {
        file: PathBuf,
        #[arg(short, long)]
        project: Option<PathBuf>,
        #[arg(long)]
        json: bool,
        #[arg(long, default_value_t = 300)]
        timeout_seconds: u64,
    },
}

#[derive(Subcommand)]
pub enum AdminCommands {
    Run {
        task: String,
        #[arg(short, long)]
        project: Option<PathBuf>,
        #[arg(long)]
        input_file: Option<PathBuf>,
        #[arg(long)]
        input: Option<String>,
        #[arg(long, default_value_t = 300)]
        timeout_seconds: u64,
    },
    RunLocal {
        task: String,
        #[arg(short = 'P', long, default_value_t = 3000)]
        port: u16,
        #[arg(long)]
        input_file: Option<PathBuf>,
        #[arg(long)]
        input: Option<String>,
        #[arg(long, default_value_t = 300)]
        timeout_seconds: u64,
    },
}

#[derive(Subcommand)]
pub enum AddCommands {
    Page { path: String },
    Action { path: String },
}
