use anyhow::{Context, Result};
use serde::Deserialize;
use std::collections::HashMap;
use std::path::Path;

#[derive(Debug, Deserialize, Default, Clone)]
pub struct ForteConfig {
    #[serde(default)]
    pub env: EnvConfig,

    #[serde(default)]
    pub type_mappings: HashMap<String, String>,

    #[serde(default)]
    pub proxy: ProxyConfig,

    #[serde(default)]
    pub build: BuildConfig,
}

#[derive(Debug, Deserialize, Default, Clone)]
pub struct EnvConfig {
    #[serde(default)]
    pub required: Vec<String>,

    #[serde(default)]
    pub optional: Vec<String>,

    #[serde(default)]
    pub defaults: HashMap<String, String>,

    /// Type mappings for environment variables
    /// Supported types: "String", "i32", "i64", "u32", "u64", "f32", "f64", "bool"
    #[serde(default)]
    pub types: HashMap<String, String>,
}

#[derive(Debug, Deserialize, Default, Clone)]
pub struct ProxyConfig {
    #[serde(default)]
    pub forward_headers: Vec<String>,

    #[serde(default = "default_timeout")]
    pub timeout_ms: u64,
}

fn default_timeout() -> u64 {
    5000
}

#[derive(Debug, Deserialize, Default, Clone)]
pub struct BuildConfig {
    #[serde(default = "default_output_dir")]
    pub output_dir: String,
}

fn default_output_dir() -> String {
    "dist".to_string()
}

impl ForteConfig {
    pub fn load(project_root: &Path) -> Result<Self> {
        let config_path = project_root.join("Forte.toml");

        if !config_path.exists() {
            // Return default config if file doesn't exist
            return Ok(Self::default());
        }

        let content = std::fs::read_to_string(&config_path)
            .context("Failed to read Forte.toml")?;

        let config: ForteConfig = toml::from_str(&content)
            .context("Failed to parse Forte.toml")?;

        Ok(config)
    }
}

pub fn load_env_vars(project_root: &Path, mode: &str) -> Result<HashMap<String, String>> {
    use dotenv::from_path;

    let mut env_vars = HashMap::new();

    // Load .env (base)
    let env_path = project_root.join(".env");
    if env_path.exists() {
        let _ = from_path(&env_path);
    }

    // Load .env.<mode> (development/production)
    let mode_env_path = project_root.join(format!(".env.{}", mode));
    if mode_env_path.exists() {
        let _ = from_path(&mode_env_path);
    }

    // Collect all env vars
    for (key, value) in std::env::vars() {
        env_vars.insert(key, value);
    }

    Ok(env_vars)
}

pub fn validate_env(config: &ForteConfig, env_vars: &HashMap<String, String>) -> Result<()> {
    let mut missing = Vec::new();

    for required in &config.env.required {
        if !env_vars.contains_key(required) && !config.env.defaults.contains_key(required) {
            missing.push(required.clone());
        }
    }

    if !missing.is_empty() {
        anyhow::bail!(
            "Missing required environment variables:\n  - {}\n\nPlease set them in .env or .env.development",
            missing.join("\n  - ")
        );
    }

    Ok(())
}
