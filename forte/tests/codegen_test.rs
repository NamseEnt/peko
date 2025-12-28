use forte::{codegen, config, watcher};
use std::fs;
use std::path::Path;
use tempfile::TempDir;

#[test]
fn test_full_codegen_pipeline() {
    // Create temporary directory
    let temp_dir = TempDir::new().unwrap();
    let project_root = temp_dir.path();

    // Create directory structure
    let backend_routes = project_root.join("backend/src/routes/index");
    fs::create_dir_all(&backend_routes).unwrap();

    let frontend_app = project_root.join("frontend/src/app/index");
    fs::create_dir_all(&frontend_app).unwrap();

    // Create a simple props.rs file
    let props_content = r#"
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
pub struct PageProps {
    pub title: String,
    pub count: i32,
}

pub fn get_props() -> PageProps {
    PageProps {
        title: "Test Page".to_string(),
        count: 42,
    }
}
"#;

    fs::write(backend_routes.join("props.rs"), props_content).unwrap();

    // Create minimal Forte.toml
    let forte_toml = r#"
[project]
name = "test-app"
"#;
    fs::write(project_root.join("Forte.toml"), forte_toml).unwrap();

    // Load config
    let config = config::ForteConfig::load(project_root).unwrap();

    // Scan routes
    let routes = watcher::scan_routes(project_root).unwrap();
    assert_eq!(routes.len(), 1, "Should find exactly one route");

    let route = &routes[0];
    assert!(route.has_get_props, "Route should have get_props");
    assert!(!route.has_action_input, "Route should not have action_input");

    // Process props file
    watcher::process_props_file(route).unwrap();

    // Generate backend code
    codegen::generate_backend_code(project_root, &routes, &config).unwrap();

    // Generate frontend code
    codegen::generate_frontend_code(project_root, &routes).unwrap();

    // Verify generated files exist
    let gen_ts_path = frontend_app.join("props.gen.ts");
    assert!(
        gen_ts_path.exists(),
        "Generated TypeScript file should exist at {:?}",
        gen_ts_path
    );

    // Read and verify generated TypeScript
    let gen_ts_content = fs::read_to_string(&gen_ts_path).unwrap();
    assert!(
        gen_ts_content.contains("export interface PageProps"),
        "Should export PageProps interface"
    );
    assert!(
        gen_ts_content.contains("title: string"),
        "Should have title field"
    );
    assert!(
        gen_ts_content.contains("count: number"),
        "Should have count field with number type"
    );

    // Verify backend generated code exists
    let backend_gen = project_root.join(".generated/backend");
    assert!(
        backend_gen.exists(),
        "Backend generated directory should exist"
    );
}

#[test]
fn test_route_with_action() {
    let temp_dir = TempDir::new().unwrap();
    let project_root = temp_dir.path();

    let backend_routes = project_root.join("backend/src/routes/submit");
    fs::create_dir_all(&backend_routes).unwrap();

    let frontend_app = project_root.join("frontend/src/app/submit");
    fs::create_dir_all(&frontend_app).unwrap();

    // Create props.rs with action
    let props_content = r#"
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
pub struct ActionInput {
    pub name: String,
    pub email: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ActionResult {
    pub success: bool,
    pub message: String,
}

pub fn post_action(input: ActionInput) -> ActionResult {
    ActionResult {
        success: true,
        message: format!("Hello, {}!", input.name),
    }
}
"#;

    fs::write(backend_routes.join("props.rs"), props_content).unwrap();

    let forte_toml = r#"
[project]
name = "test-app"
"#;
    fs::write(project_root.join("Forte.toml"), forte_toml).unwrap();

    let config = config::ForteConfig::load(project_root).unwrap();
    let routes = watcher::scan_routes(project_root).unwrap();

    assert_eq!(routes.len(), 1);
    let route = &routes[0];
    assert!(route.has_action_input, "Route should have action");

    watcher::process_props_file(route).unwrap();
    codegen::generate_backend_code(project_root, &routes, &config).unwrap();
    codegen::generate_frontend_code(project_root, &routes).unwrap();

    let gen_ts_path = frontend_app.join("props.gen.ts");
    let gen_ts_content = fs::read_to_string(&gen_ts_path).unwrap();

    assert!(
        gen_ts_content.contains("export interface ActionInput"),
        "Should export ActionInput"
    );
    assert!(
        gen_ts_content.contains("export interface ActionResult"),
        "Should export ActionResult"
    );
}
