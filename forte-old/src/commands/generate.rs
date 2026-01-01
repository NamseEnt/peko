use anyhow::{Context, Result};
use colored::*;
use std::env;
use std::fs;
use std::path::Path;

pub fn execute(resource_type: &str, path: &str) -> Result<()> {
    if resource_type != "route" {
        anyhow::bail!("Unknown resource type: {}. Currently only 'route' is supported.", resource_type);
    }

    let current_dir = env::current_dir().context("Failed to get current directory")?;

    // Verify we're in a Forte project
    let forte_toml = current_dir.join("Forte.toml");
    if !forte_toml.exists() {
        anyhow::bail!(
            "Forte.toml not found. Are you in a Forte project root?\nRun 'forte init <project-name>' to create a new project."
        );
    }

    println!("\n{} {}", "🔧".cyan(), format!("Generating route: {}", path).bold());

    // Convert path format: product/_id_ -> product/[id]
    // Backend: product/_id_ -> backend/src/routes/product/_id_/props.rs
    // Frontend: product/_id_ -> frontend/src/app/product/[id]/page.tsx

    let backend_path = current_dir.join(format!("backend/src/routes/{}", path));
    let props_file = backend_path.join("props.rs");

    // Convert _param_ to [param] for frontend
    let frontend_path_str = path.replace("_", "[").replace("[", "[").replace("]", "]");
    let frontend_path_parts: Vec<&str> = frontend_path_str.split('/').collect();
    let mut frontend_path_clean = Vec::new();
    for part in frontend_path_parts {
        if part.starts_with("[[") {
            // _id_ becomes [[id]]
            let param_name = part.trim_start_matches("[[").trim_end_matches("]]");
            frontend_path_clean.push(format!("[{}]", param_name));
        } else {
            frontend_path_clean.push(part.to_string());
        }
    }
    let frontend_path = current_dir.join(format!("frontend/src/app/{}", frontend_path_clean.join("/")));
    let page_file = frontend_path.join("page.tsx");

    // Create backend route file
    if backend_path.exists() {
        anyhow::bail!("{} Backend route already exists: {}", "✗".red(), backend_path.display());
    }

    fs::create_dir_all(&backend_path)
        .context(format!("Failed to create directory: {}", backend_path.display()))?;

    let props_content = r#"use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
pub struct PageProps {
    pub message: String,
}

pub fn get_props() -> PageProps {
    PageProps {
        message: "Hello from Forte!".to_string(),
    }
}
"#;

    fs::write(&props_file, props_content)
        .context(format!("Failed to write file: {}", props_file.display()))?;

    println!("{} Created {}", "✓".green(), props_file.display().to_string().yellow());

    // Create frontend page file
    if frontend_path.exists() {
        anyhow::bail!("{} Frontend page already exists: {}", "✗".red(), frontend_path.display());
    }

    fs::create_dir_all(&frontend_path)
        .context(format!("Failed to create directory: {}", frontend_path.display()))?;

    let page_content = format!(r#"import * as React from 'react';
import type {{ PageProps }} from './props.gen';

export default function Page({{ pageProps }}: {{ pageProps: PageProps }}) {{
  return (
    <div>
      <h1>Generated Page</h1>
      <p>{{pageProps.message}}</p>
    </div>
  );
}}
"#);

    fs::write(&page_file, page_content)
        .context(format!("Failed to write file: {}", page_file.display()))?;

    println!("{} Created {}", "✓".green(), page_file.display().to_string().yellow());

    println!("\n{} {}", "✨".bold(), "Route generated successfully!".green().bold());
    println!("\n{}", "Next steps:".bold());
    println!("  {} Edit {} to define your page props", "1.".cyan(), props_file.display().to_string().yellow());
    println!("  {} Edit {} to build your UI", "2.".cyan(), page_file.display().to_string().yellow());
    println!("  {} Run {} to start the dev server", "3.".cyan(), "forte dev".yellow());

    Ok(())
}
