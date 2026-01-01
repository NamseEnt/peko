use anyhow::{Context, Result};
use std::env;
use std::fs;
use std::path::Path;
use std::process::Command;

use crate::config;
use crate::watcher;

pub fn execute() -> Result<()> {
    let current_dir = env::current_dir().context("Failed to get current directory")?;

    println!("Building Forte project for production...");
    println!("Project root: {}\n", current_dir.display());

    // Verify we're in a Forte project
    let forte_toml = current_dir.join("Forte.toml");
    if !forte_toml.exists() {
        anyhow::bail!(
            "Forte.toml not found. Are you in a Forte project root?\nRun 'forte init <project-name>' to create a new project."
        );
    }

    // Load config
    let forte_config = config::ForteConfig::load(&current_dir)?;

    // Load and validate environment variables for production
    let env_vars = config::load_env_vars(&current_dir, "production")?;
    config::validate_env(&forte_config, &env_vars)?;

    // Generate code
    println!("Generating code...");
    let routes = watcher::scan_routes(&current_dir)?;
    for route in &routes {
        if let Err(e) = watcher::process_props_file(route) {
            eprintln!("Error processing {}: {}", route.props_path.display(), e);
        }
    }
    crate::codegen::generate_backend_code(&current_dir, &routes, &forte_config)?;
    crate::codegen::generate_frontend_code(&current_dir, &routes)?;

    // Build backend WASM in release mode
    println!("\nBuilding backend WASM (release mode)...");
    let backend_dir = current_dir.join("backend");
    let build_status = Command::new("cargo")
        .arg("build")
        .arg("--release")
        .arg("--target")
        .arg("wasm32-wasip2")
        .current_dir(&backend_dir)
        .status()
        .context("Failed to run cargo build --release --target wasm32-wasip2")?;

    if !build_status.success() {
        anyhow::bail!("Backend WASM build failed");
    }

    // Get WASM file path
    let wasm_path = backend_dir.join("target/wasm32-wasip2/release/backend.wasm");
    if !wasm_path.exists() {
        anyhow::bail!("WASM file not found at: {}", wasm_path.display());
    }

    // Report original WASM size
    let original_size = fs::metadata(&wasm_path)
        .context("Failed to get WASM file metadata")?
        .len();
    println!("  Original WASM size: {:.2} MB", original_size as f64 / 1_048_576.0);

    // Run wasm-opt if available
    println!("\nOptimizing WASM with wasm-opt...");
    let wasm_opt_result = Command::new("wasm-opt")
        .arg("-O3")
        .arg(&wasm_path)
        .arg("-o")
        .arg(&wasm_path)
        .status();

    match wasm_opt_result {
        Ok(status) if status.success() => {
            let optimized_size = fs::metadata(&wasm_path)
                .context("Failed to get optimized WASM file metadata")?
                .len();
            let reduction = ((original_size - optimized_size) as f64 / original_size as f64) * 100.0;
            println!("  ✓ Optimized WASM size: {:.2} MB ({:.1}% reduction)",
                optimized_size as f64 / 1_048_576.0, reduction);
        }
        Ok(_) => {
            println!("  ⚠ wasm-opt failed, continuing with unoptimized WASM");
        }
        Err(_) => {
            println!("  ⚠ wasm-opt not found. Install it for smaller WASM bundles:");
            println!("     npm install -g wasm-opt");
            println!("     or: brew install binaryen");
        }
    }

    // Install frontend dependencies if needed
    let frontend_dir = current_dir.join("frontend");
    let node_modules = frontend_dir.join("node_modules");
    if !node_modules.exists() {
        println!("\nInstalling frontend dependencies...");
        let npm_status = Command::new("npm")
            .arg("install")
            .current_dir(&frontend_dir)
            .status()
            .context("Failed to run npm install")?;

        if !npm_status.success() {
            anyhow::bail!("npm install failed");
        }
    }

    // Build frontend with Vite
    println!("\nBuilding frontend (production mode)...");
    let vite_status = Command::new("npm")
        .arg("run")
        .arg("build")
        .current_dir(&frontend_dir)
        .status()
        .context("Failed to run npm run build")?;

    if !vite_status.success() {
        anyhow::bail!("Frontend build failed");
    }

    // Analyze frontend bundle size
    println!("\nAnalyzing frontend bundle size...");
    analyze_frontend_bundle(&frontend_dir)?;

    // Create dist directory and copy artifacts
    let dist_dir = current_dir.join("dist");
    if dist_dir.exists() {
        fs::remove_dir_all(&dist_dir).context("Failed to remove old dist directory")?;
    }
    fs::create_dir_all(&dist_dir).context("Failed to create dist directory")?;

    // Copy backend WASM
    println!("\nCopying artifacts to dist/...");
    let backend_wasm_src = wasm_path;
    let backend_wasm_dst = dist_dir.join("backend.wasm");

    fs::copy(&backend_wasm_src, &backend_wasm_dst)
        .with_context(|| format!("Failed to copy backend WASM from {} to {}",
            backend_wasm_src.display(), backend_wasm_dst.display()))?;

    let wasm_size = fs::metadata(&backend_wasm_dst)?.len();
    println!("  ✓ Copied backend.wasm ({:.2} MB)", wasm_size as f64 / 1_048_576.0);

    // Copy frontend dist
    let frontend_dist_src = frontend_dir.join("dist");
    let frontend_dist_dst = dist_dir.join("public");

    copy_dir_all(&frontend_dist_src, &frontend_dist_dst)
        .context("Failed to copy frontend dist")?;

    // Copy .env.production if it exists
    let env_prod_src = current_dir.join(".env.production");
    if env_prod_src.exists() {
        let env_prod_dst = dist_dir.join(".env.production");
        fs::copy(&env_prod_src, &env_prod_dst)
            .context("Failed to copy .env.production")?;
        println!("  ✓ Copied .env.production");
    }

    println!("\n✓ Build complete!");
    println!("\nProduction artifacts in dist/:");
    println!("  backend.wasm    - WASM backend ({:.2} MB)", wasm_size as f64 / 1_048_576.0);
    println!("  public/         - Frontend assets");
    if env_prod_src.exists() {
        println!("  .env.production - Production environment");
    }

    println!("\nNext steps:");
    println!("  1. Deploy the dist/ directory to your server");
    println!("  2. Run the WASM backend with a WASI-compatible runtime:");
    println!("     wasmtime run --wasi preview2 dist/backend.wasm");
    println!("  3. Or use the provided Docker container (see Dockerfile)");

    Ok(())
}

/// Analyze frontend bundle size
fn analyze_frontend_bundle(frontend_dir: &Path) -> Result<()> {
    let dist_dir = frontend_dir.join("dist");
    if !dist_dir.exists() {
        return Ok(());
    }

    let mut total_size: u64 = 0;
    let mut js_size: u64 = 0;
    let mut css_size: u64 = 0;
    let mut asset_size: u64 = 0;

    fn walk_dir(dir: &Path, total: &mut u64, js: &mut u64, css: &mut u64, assets: &mut u64) -> Result<()> {
        for entry in fs::read_dir(dir)? {
            let entry = entry?;
            let path = entry.path();

            if path.is_dir() {
                walk_dir(&path, total, js, css, assets)?;
            } else {
                let size = fs::metadata(&path)?.len();
                *total += size;

                if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
                    match ext {
                        "js" => *js += size,
                        "css" => *css += size,
                        _ => *assets += size,
                    }
                }
            }
        }
        Ok(())
    }

    walk_dir(&dist_dir, &mut total_size, &mut js_size, &mut css_size, &mut asset_size)?;

    println!("  Total size:  {:.2} MB", total_size as f64 / 1_048_576.0);
    println!("  JavaScript:  {:.2} MB", js_size as f64 / 1_048_576.0);
    println!("  CSS:         {:.2} MB", css_size as f64 / 1_048_576.0);
    println!("  Other assets: {:.2} MB", asset_size as f64 / 1_048_576.0);

    Ok(())
}

/// Recursively copy a directory
fn copy_dir_all(src: &Path, dst: &Path) -> Result<()> {
    fs::create_dir_all(dst).context("Failed to create destination directory")?;

    for entry in fs::read_dir(src).context("Failed to read source directory")? {
        let entry = entry.context("Failed to read directory entry")?;
        let ty = entry.file_type().context("Failed to get file type")?;
        let src_path = entry.path();
        let dst_path = dst.join(entry.file_name());

        if ty.is_dir() {
            copy_dir_all(&src_path, &dst_path)?;
        } else {
            fs::copy(&src_path, &dst_path)
                .with_context(|| format!("Failed to copy {} to {}",
                    src_path.display(), dst_path.display()))?;
        }
    }

    Ok(())
}
