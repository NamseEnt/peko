use anyhow::Result;
use std::fs;
use std::path::Path;

pub fn add_page(path: &str) -> Result<()> {
    let project_dir = Path::new(".");

    if !project_dir.join("Forte.toml").exists() {
        anyhow::bail!("Not a Forte project (Forte.toml not found)");
    }

    let page_path = normalize_page_path(path);
    let rs_page_dir = project_dir.join(format!("rs/src/pages/{}", page_path));
    let fe_page_dir = project_dir.join(format!("fe/src/pages/{}", page_path));

    if rs_page_dir.exists() || fe_page_dir.exists() {
        anyhow::bail!("Page '{}' already exists", path);
    }

    fs::create_dir_all(&rs_page_dir)?;
    fs::create_dir_all(&fe_page_dir)?;

    let component_name = path_to_component_name(&page_path);

    fs::write(rs_page_dir.join("mod.rs"), generate_backend_page(&page_path))?;

    fs::write(fe_page_dir.join("page.tsx"), generate_frontend_page(&component_name))?;

    println!("Created page '{}'", path);
    println!("  - rs/src/pages/{}/mod.rs", page_path);
    println!("  - fe/src/pages/{}/page.tsx", page_path);

    Ok(())
}

fn normalize_page_path(path: &str) -> String {
    path.trim_matches('/').to_string()
}

fn path_to_component_name(path: &str) -> String {
    let parts: Vec<&str> = path.split('/').collect();
    let last_part = parts.last().unwrap_or(&"page");

    let name = if last_part.starts_with('[') && last_part.ends_with(']') {
        &last_part[1..last_part.len() - 1]
    } else {
        last_part
    };

    let mut result = String::new();
    let mut capitalize_next = true;
    for c in name.chars() {
        if c == '_' || c == '-' {
            capitalize_next = true;
        } else if capitalize_next {
            result.push(c.to_ascii_uppercase());
            capitalize_next = false;
        } else {
            result.push(c);
        }
    }

    format!("{}Page", result)
}

fn generate_backend_page(page_path: &str) -> String {
    let params = extract_params(page_path);

    if params.is_empty() {
        r#"use anyhow::Result;
use http::HeaderMap;
use serde::Serialize;

#[derive(Serialize)]
pub enum Props {
    Ok { message: String },
}

pub async fn handler(_headers: HeaderMap) -> Result<Props> {
    Ok(Props::Ok {
        message: "Hello from Forte!".to_string(),
    })
}
"#
        .to_string()
    } else {
        let param_fields = params
            .iter()
            .map(|p| format!("    pub {}: String,", p))
            .collect::<Vec<_>>()
            .join("\n");

        format!(
            r#"use anyhow::Result;
use http::HeaderMap;
use serde::{{Deserialize, Serialize}};

#[derive(Deserialize)]
pub struct Params {{
{param_fields}
}}

#[derive(Serialize)]
pub enum Props {{
    Ok {{ message: String }},
}}

pub async fn handler(_headers: HeaderMap, params: Params) -> Result<Props> {{
    Ok(Props::Ok {{
        message: format!("Hello from Forte!"),
    }})
}}
"#
        )
    }
}

fn generate_frontend_page(component_name: &str) -> String {
    format!(
        r#"import type {{ Props }} from "./.props";

export default function {component_name}(props: Props) {{
    if (props.t !== "Ok") {{
        return <div>Error loading page</div>;
    }}

    return (
        <div>
            <h1>{component_name}</h1>
            <p>{{props.v.message}}</p>
        </div>
    );
}}
"#
    )
}

fn extract_params(path: &str) -> Vec<String> {
    path.split('/')
        .filter(|s| s.starts_with('[') && s.ends_with(']'))
        .map(|s| s[1..s.len() - 1].to_string())
        .collect()
}

pub fn add_action(path: &str) -> Result<()> {
    let project_dir = Path::new(".");

    if !project_dir.join("Forte.toml").exists() {
        anyhow::bail!("Not a Forte project (Forte.toml not found)");
    }

    let action_path = normalize_action_path(path);
    let rs_action_file = project_dir.join(format!("rs/src/actions/{}.rs", action_path));
    let fe_action_file = project_dir.join(format!("fe/src/actions/{}.ts", action_path));

    if rs_action_file.exists() || fe_action_file.exists() {
        anyhow::bail!("Action '{}' already exists", path);
    }

    if let Some(parent) = rs_action_file.parent() {
        fs::create_dir_all(parent)?;
    }
    if let Some(parent) = fe_action_file.parent() {
        fs::create_dir_all(parent)?;
    }

    let action_name = path_to_action_name(&action_path);

    fs::write(&rs_action_file, generate_backend_action(&action_name))?;
    fs::write(&fe_action_file, generate_frontend_action(&action_name, &action_path))?;

    println!("Created action '{}'", path);
    println!("  - rs/src/actions/{}.rs", action_path);
    println!("  - fe/src/actions/{}.ts", action_path);

    Ok(())
}

fn normalize_action_path(path: &str) -> String {
    path.trim_matches('/').replace('-', "_")
}

fn path_to_action_name(path: &str) -> String {
    let parts: Vec<&str> = path.split('/').collect();
    let last_part = parts.last().unwrap_or(&"action");

    let mut result = String::new();
    let mut capitalize_next = true;
    for c in last_part.chars() {
        if c == '_' || c == '-' {
            capitalize_next = true;
        } else if capitalize_next {
            result.push(c.to_ascii_uppercase());
            capitalize_next = false;
        } else {
            result.push(c);
        }
    }

    result
}

fn generate_backend_action(action_name: &str) -> String {
    format!(
        r#"use anyhow::Result;
use serde::{{Deserialize, Serialize}};

#[derive(Deserialize)]
pub struct {action_name}Input {{
    pub message: String,
}}

#[derive(Serialize)]
pub enum {action_name}Output {{
    Ok {{ result: String }},
    Error {{ message: String }},
}}

pub async fn action(input: {action_name}Input) -> Result<{action_name}Output> {{
    Ok({action_name}Output::Ok {{
        result: format!("Received: {{}}", input.message),
    }})
}}
"#
    )
}

fn generate_frontend_action(action_name: &str, action_path: &str) -> String {
    let fn_name = to_camel_case(action_name);
    format!(
        r#"import type {{ {action_name}Input, {action_name}Output }} from "./{action_path}.types";

export async function {fn_name}(input: {action_name}Input): Promise<{action_name}Output> {{
    const response = await fetch("/_action/{action_path}", {{
        method: "POST",
        headers: {{ "Content-Type": "application/json" }},
        body: JSON.stringify(input),
    }});
    return response.json();
}}
"#
    )
}

fn to_camel_case(name: &str) -> String {
    let mut result = String::new();
    let mut first = true;
    for c in name.chars() {
        if first {
            result.push(c.to_ascii_lowercase());
            first = false;
        } else {
            result.push(c);
        }
    }
    result
}
