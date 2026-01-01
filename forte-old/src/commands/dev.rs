use anyhow::{Context, Result};
use colored::*;
use indicatif::{ProgressBar, ProgressStyle};
use std::env;
use std::net::SocketAddr;
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use tokio::sync::{broadcast, RwLock};
use tokio::io::{AsyncBufReadExt, BufReader};

use crate::config;
use crate::runtime::WasmRuntime;
use crate::server::{CompileStatus, ForteProxy, ForteState};
use crate::watcher;

pub fn execute() -> Result<()> {
    // Create tokio runtime for async operations
    let rt = tokio::runtime::Runtime::new()?;
    rt.block_on(execute_async())
}

async fn execute_async() -> Result<()> {
    let current_dir = env::current_dir().context("Failed to get current directory")?;

    println!("\n{}", "🚀 Starting Forte development server...".bold().cyan());
    println!("{} {}\n", "📁 Project root:".dimmed(), current_dir.display().to_string().yellow());

    // Verify we're in a Forte project
    let forte_toml = current_dir.join("Forte.toml");
    if !forte_toml.exists() {
        anyhow::bail!(
            "Forte.toml not found. Are you in a Forte project root?\nRun 'forte init <project-name>' to create a new project."
        );
    }

    // Load config
    let forte_config = config::ForteConfig::load(&current_dir)?;

    // Load and validate environment variables
    let env_vars = config::load_env_vars(&current_dir, "development")?;
    config::validate_env(&forte_config, &env_vars)?;

    // Initial code generation
    let spinner = ProgressBar::new_spinner();
    spinner.set_style(
        ProgressStyle::default_spinner()
            .tick_strings(&["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"])
            .template("{spinner:.cyan} {msg}")
            .unwrap()
    );
    spinner.set_message("Scanning routes...");
    spinner.enable_steady_tick(std::time::Duration::from_millis(80));

    let routes = watcher::scan_routes(&current_dir)?;
    spinner.finish_with_message(format!("{} Found {} route(s)", "✓".green(), routes.len().to_string().bold()));

    for route in &routes {
        if let Err(e) = watcher::process_props_file(route) {
            eprintln!("{} Error processing {}: {}", "✗".red(), route.props_path.display(), e);
        }
    }

    println!("{} {}", "✓".green(), "Generated backend code".bold());
    crate::codegen::generate_backend_code(&current_dir, &routes, &forte_config)?;

    println!("{} {}", "✓".green(), "Generated frontend code".bold());
    crate::codegen::generate_frontend_code(&current_dir, &routes)?;

    // Build WASM backend
    let spinner = ProgressBar::new_spinner();
    spinner.set_style(
        ProgressStyle::default_spinner()
            .tick_strings(&["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"])
            .template("{spinner:.cyan} {msg}")
            .unwrap()
    );
    spinner.set_message("Building WASM backend...");
    spinner.enable_steady_tick(std::time::Duration::from_millis(80));

    let backend_dir = current_dir.join("backend");
    let build_output = Command::new("cargo")
        .arg("build")
        .arg("--target")
        .arg("wasm32-wasip2")
        .arg("--message-format=json")
        .current_dir(&backend_dir)
        .output()
        .context("Failed to run cargo build")?;

    if !build_output.status.success() {
        spinner.finish_with_message(format!("{} WASM build failed", "✗".red()));
        let stderr = String::from_utf8_lossy(&build_output.stderr);
        let formatted = crate::error_formatter::format_compiler_errors(&stderr);
        eprintln!("\n{}", formatted);
        anyhow::bail!("WASM backend build failed");
    }

    spinner.finish_with_message(format!("{} {}", "✓".green(), "WASM backend built".bold()));

    // Get WASM file path
    let wasm_path = current_dir.join("backend/target/wasm32-wasip2/debug/backend.wasm");
    if !wasm_path.exists() {
        anyhow::bail!("WASM file not found at: {}", wasm_path.display());
    }

    // Load WASM runtime
    println!("{} {}", "✓".green(), "Loading WASM runtime...".bold());
    let wasm_runtime = WasmRuntime::new(&wasm_path)
        .context("Failed to create WASM runtime")?;
    let wasm_runtime = Arc::new(RwLock::new(wasm_runtime));

    // Install frontend dependencies if needed
    let frontend_dir = current_dir.join("frontend");
    let node_modules = frontend_dir.join("node_modules");
    if !node_modules.exists() {
        let spinner = ProgressBar::new_spinner();
        spinner.set_style(
            ProgressStyle::default_spinner()
                .tick_strings(&["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"])
                .template("{spinner:.cyan} {msg}")
                .unwrap()
        );
        spinner.set_message("Installing frontend dependencies...");
        spinner.enable_steady_tick(std::time::Duration::from_millis(80));

        let npm_status = Command::new("npm")
            .arg("install")
            .current_dir(&frontend_dir)
            .status()
            .context("Failed to run npm install")?;

        if !npm_status.success() {
            spinner.finish_with_message(format!("{} npm install failed", "✗".red()));
            anyhow::bail!("npm install failed");
        }

        spinner.finish_with_message(format!("{} {}", "✓".green(), "Frontend dependencies installed".bold()));
    }

    // Start Node.js SSR server with piped stdout to capture port
    let spinner = ProgressBar::new_spinner();
    spinner.set_style(
        ProgressStyle::default_spinner()
            .tick_strings(&["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"])
            .template("{spinner:.cyan} {msg}")
            .unwrap()
    );
    spinner.set_message("Starting Node.js SSR server...");
    spinner.enable_steady_tick(std::time::Duration::from_millis(80));

    let ssr_server_path = current_dir.join(".generated/frontend/server.ts");
    let mut node_process = Command::new("npx")
        .arg("tsx")
        .arg(&ssr_server_path)
        .current_dir(&current_dir)
        .env("RUST_PORT", "3000")  // CLI proxy port for WASM backend
        .stdout(Stdio::piped())  // Capture stdout to read port
        .stderr(Stdio::inherit())
        .spawn()
        .context("Failed to start Node.js SSR server")?;

    // Read the SSR port from stdout asynchronously
    let stdout = node_process.stdout.take().expect("Failed to capture stdout");
    let mut reader = BufReader::new(tokio::process::ChildStdout::from_std(stdout)?).lines();

    let mut ssr_port = String::new();

    // Read with timeout
    let timeout_duration = tokio::time::Duration::from_secs(10);
    let port_result = tokio::time::timeout(timeout_duration, async {
        while let Some(line) = reader.next_line().await? {
            if line.starts_with("SSR_PORT=") {
                return Ok::<String, anyhow::Error>(line.strip_prefix("SSR_PORT=").unwrap().to_string());
            }
        }
        anyhow::bail!("SSR server did not output port")
    }).await;

    match port_result {
        Ok(Ok(port)) => {
            spinner.finish_with_message(format!("{} SSR server started on port {}", "✓".green(), port.bold()));
            ssr_port = port;
        }
        Ok(Err(e)) => {
            spinner.finish_with_message(format!("{} Failed to read SSR port", "✗".red()));
            anyhow::bail!("Failed to read SSR port: {}", e);
        }
        Err(_) => {
            spinner.finish_with_message(format!("{} SSR server timeout", "✗".red()));
            anyhow::bail!("Timeout waiting for SSR server to start");
        }
    }

    let node_handle = Arc::new(Mutex::new(Some(node_process)));
    let node_handle_clone = node_handle.clone();

    // Start WASM hot reload watcher
    println!("{} {}", "✓".green(), "Setting up WASM hot reload...".bold());
    let wasm_runtime_clone = wasm_runtime.clone();
    watcher::watch_wasm(&current_dir, wasm_runtime_clone)?;

    // Set up Ctrl+C handler
    ctrlc::set_handler(move || {
        println!("\n\n{}", "👋 Shutting down...".yellow());
        if let Some(mut child) = node_handle_clone.lock().unwrap().take() {
            let _ = child.kill();
            let _ = child.wait();
        }
        std::process::exit(0);
    })
    .context("Failed to set Ctrl+C handler")?;

    // Create Forte state for internal APIs
    let (hmr_sender, _) = broadcast::channel(16);
    let forte_state = ForteState {
        compile_status: Arc::new(RwLock::new(CompileStatus::Ready)),
        routes: Arc::new(RwLock::new(routes.clone())),
        compile_errors: Arc::new(RwLock::new(Vec::new())),
        hmr_sender,
    };

    // Start CLI proxy server
    let proxy_addr: SocketAddr = "127.0.0.1:3000".parse()?;
    let node_url = format!("http://127.0.0.1:{}", ssr_port); // Use dynamic SSR port
    let static_dir = current_dir.join("frontend/dist");

    let proxy = ForteProxy::new(
        wasm_runtime.clone(),
        static_dir,
        node_url,
        forte_state.clone(),
    );

    // Spawn route watcher in background
    let current_dir_clone = current_dir.clone();
    let forte_config_clone = forte_config.clone();
    let forte_state_clone = forte_state.clone();
    tokio::spawn(async move {
        if let Err(e) = watcher::watch_routes_with_state(&current_dir_clone, &forte_config_clone, forte_state_clone) {
            eprintln!("Route watcher error: {}", e);
        }
    });

    // Start the proxy server (this will block)
    println!();
    proxy.serve(proxy_addr).await?;

    // Clean up
    if let Some(mut child) = node_handle.lock().unwrap().take() {
        let _ = child.kill();
        let _ = child.wait();
    }

    Ok(())
}
