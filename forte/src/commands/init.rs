use anyhow::{Context, Result};
use std::fs;
use std::path::Path;

use crate::templates;

pub fn execute(project_name: &str) -> Result<()> {
    let project_path = Path::new(project_name);

    // Check if directory already exists
    if project_path.exists() {
        anyhow::bail!("Directory '{}' already exists", project_name);
    }

    println!("Creating Forte project: {}", project_name);

    // Create directory structure
    create_directory_structure(project_path)?;

    // Write template files
    write_template_files(project_path, project_name)?;

    println!("\n✅ Project '{}' created successfully!", project_name);
    println!("\nNext steps:");
    println!("  cd {}", project_name);
    println!("  forte dev");

    Ok(())
}

fn create_directory_structure(base: &Path) -> Result<()> {
    let dirs = [
        // Backend
        "backend/src/routes/index",
        "backend/.cargo",
        // Frontend
        "frontend/src/app/index",
        "frontend/src/forte",
        // Generated (hidden)
        ".generated/backend",
        ".generated/frontend",
    ];

    for dir in &dirs {
        let path = base.join(dir);
        fs::create_dir_all(&path)
            .with_context(|| format!("Failed to create directory: {}", path.display()))?;
    }

    Ok(())
}

fn write_template_files(base: &Path, project_name: &str) -> Result<()> {
    // Root files
    write_file(base, "Forte.toml", templates::FORTE_TOML)?;
    write_file(base, ".env", templates::ENV_TEMPLATE)?;
    write_file(base, ".env.development", templates::ENV_DEV)?;
    write_file(base, ".env.production", templates::ENV_PROD)?;
    write_file(base, "README.md", &templates::readme(project_name))?;
    write_file(base, ".gitignore", templates::GITIGNORE)?;
    write_file(base, "Dockerfile", templates::DOCKERFILE)?;
    write_file(base, ".dockerignore", templates::DOCKERIGNORE)?;

    // Backend
    write_file(base, "backend/Cargo.toml", &templates::backend_cargo_toml(project_name))?;
    write_file(base, "backend/.cargo/config.toml", templates::BACKEND_CARGO_CONFIG)?;
    write_file(base, "backend/src/lib.rs", templates::BACKEND_LIB)?;
    write_file(base, "backend/src/router.rs", templates::BACKEND_ROUTER)?;
    write_file(base, "backend/src/routes/mod.rs", templates::BACKEND_ROUTES_MOD)?;
    write_file(base, "backend/src/routes/index/mod.rs", templates::BACKEND_INDEX_MOD)?;
    write_file(base, "backend/src/routes/index/props.rs", templates::BACKEND_INDEX_PROPS)?;
    write_file(base, "backend/src/routes/index/wrapper.rs", templates::BACKEND_INDEX_WRAPPER)?;

    // Frontend
    write_file(base, "frontend/package.json", &templates::frontend_package_json(project_name))?;
    write_file(base, "frontend/tsconfig.json", templates::FRONTEND_TSCONFIG)?;
    write_file(base, "frontend/vite.config.ts", templates::FRONTEND_VITE_CONFIG)?;
    write_file(base, "frontend/src/app/layout.tsx", templates::FRONTEND_ROOT_LAYOUT)?;
    write_file(base, "frontend/src/app/error.tsx", templates::FRONTEND_ERROR_PAGE)?;
    write_file(base, "frontend/src/app/index/page.tsx", templates::FRONTEND_INDEX_PAGE)?;

    // SSR Server
    write_file(base, ".generated/frontend/server.ts", templates::frontend_ssr::NODE_SSR_SERVER)?;

    // Forte Runtime Library
    write_file(base, "frontend/src/forte/package.json", templates::FORTE_RUNTIME_PACKAGE_JSON)?;
    write_file(base, "frontend/src/forte/index.ts", templates::FORTE_RUNTIME_INDEX)?;
    write_file(base, "frontend/src/forte/Router.tsx", templates::FORTE_RUNTIME_ROUTER)?;
    write_file(base, "frontend/src/forte/Form.tsx", templates::FORTE_RUNTIME_FORM)?;
    write_file(base, "frontend/src/forte/Link.tsx", templates::FORTE_RUNTIME_LINK)?;

    Ok(())
}

fn write_file(base: &Path, relative_path: &str, content: &str) -> Result<()> {
    let path = base.join(relative_path);
    fs::write(&path, content)
        .with_context(|| format!("Failed to write file: {}", path.display()))?;
    Ok(())
}
