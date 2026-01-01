use crate::config::ForteConfig;
use crate::watcher::RouteInfo;
use anyhow::{Context, Result};
use std::path::Path;

/// Generate all backend code files (.generated/backend/*)
pub fn generate_backend_code(project_root: &Path, routes: &[RouteInfo], config: &ForteConfig) -> Result<()> {
    let gen_dir = project_root.join(".generated/backend");
    std::fs::create_dir_all(&gen_dir).context("Failed to create .generated/backend")?;

    // Generate router.rs
    let router_rs = generate_router_module(routes)?;
    std::fs::write(gen_dir.join("router.rs"), router_rs)?;

    // Generate main.rs
    let main_rs = generate_main_module();
    std::fs::write(gen_dir.join("main.rs"), main_rs)?;

    // Generate env.rs
    let env_rs = generate_env_module(config);
    std::fs::write(gen_dir.join("env.rs"), env_rs)?;

    // Generate .env.example in project root
    let env_example = generate_env_example(config);
    let env_example_path = project_root.join(".env.example");
    std::fs::write(env_example_path, env_example)?;

    // Update backend/src/routes/mod.rs
    let routes_mod = generate_routes_mod(routes)?;
    let routes_mod_path = project_root.join("backend/src/routes/mod.rs");
    std::fs::write(routes_mod_path, routes_mod)?;

    // Update backend/src/lib.rs
    let lib_rs = generate_lib_module(routes);
    let lib_rs_path = project_root.join("backend/src/lib.rs");
    std::fs::write(lib_rs_path, lib_rs)?;

    // Generate router.rs module
    let router_rs = generate_router_wasm_module();
    let router_rs_path = project_root.join("backend/src/router.rs");
    std::fs::write(router_rs_path, router_rs)?;

    // Generate wrapper handlers for each route
    for route in routes {
        let wrapper_code = generate_wrapper_handler(route)?;
        let route_path = extract_route_path(&route.props_path)?;
        let route_dir = project_root.join("backend/src/routes").join(&route_path);
        let wrapper_path = route_dir.join("wrapper.rs");
        std::fs::write(wrapper_path, wrapper_code)?;

        // Update the route's mod.rs to export wrapper
        let mod_rs_path = route_dir.join("mod.rs");
        let mod_content = "pub mod props;\npub mod wrapper;\n\npub use props::*;\npub use wrapper::*;\n";
        std::fs::write(mod_rs_path, mod_content)?;

        // Create intermediate mod.rs files for nested paths
        // For example, product/_id_ needs product/mod.rs
        let path_components: Vec<&str> = route_path.split('/').collect();
        if path_components.len() > 1 {
            for i in 0..path_components.len() - 1 {
                let partial_path: String = path_components[..=i].join("/");
                let partial_dir = project_root.join("backend/src/routes").join(&partial_path);
                let partial_mod = partial_dir.join("mod.rs");

                // Only create if it doesn't exist yet
                if !partial_mod.exists() {
                    let next_component = path_components[i + 1];
                    let mod_content = format!("pub mod {};\n", next_component);
                    std::fs::write(&partial_mod, mod_content)?;
                }
            }
        }
    }

    println!("  ✓ Generated backend code in .generated/backend/");

    Ok(())
}

fn generate_lib_module(routes: &[RouteInfo]) -> String {
    let mut route_registrations = String::new();

    for route in routes {
        let route_path = extract_route_path(&route.props_path).unwrap_or_else(|_| "index".to_string());
        let url_path = convert_to_url_path(&route_path);
        let module_path = route_path_to_module_path(&route_path);

        // Register GET handler
        route_registrations.push_str(&format!(
            "    router.add(\"GET\", \"{}\", routes::{}::wrapper_get);\n",
            url_path, module_path
        ));

        // Register POST handler if route has actions
        if route.has_action_input {
            route_registrations.push_str(&format!(
                "    router.add(\"POST\", \"{}\", routes::{}::wrapper_post);\n",
                url_path, module_path
            ));
        }
    }

    format!(r#"// [Generated] WASM backend entry point
// This file is auto-managed by Forte CLI

use wstd::http::body::IncomingBody;
use wstd::http::server::{{Finished, Responder}};
use wstd::http::Request;

pub mod routes;
mod router;

#[wstd::http_server]
async fn main(req: Request<IncomingBody>, res: Responder) -> Finished {{
    // Create router and register routes
    let mut router = router::Router::new();
{}
    // Handle the request
    router.handle(req, res).await
}}
"#, route_registrations)
}

fn generate_wrapper_handler(route: &RouteInfo) -> Result<String> {
    let route_path = extract_route_path(&route.props_path)?;
    let params = extract_route_params(&route_path);

    let mut output = String::new();
    output.push_str("// [Generated] Wrapper handlers\n");
    output.push_str("// This file is auto-managed by Forte CLI\n\n");
    output.push_str("use wstd::http::body::{BoundedBody, IncomingBody};\n");
    output.push_str("use wstd::http::server::{Finished, Responder};\n");
    output.push_str("use wstd::http::{IntoBody, Request, Response, StatusCode};\n");
    output.push_str("use crate::router::extract_path_params;\n");
    output.push_str("use std::pin::Pin;\n");

    // Add AsyncRead import if route has actions
    if route.has_action_input {
        output.push_str("use wstd::io::AsyncRead;\n");
    }

    output.push_str("\n");

    // Generate wrapper_get - boxed version for handler
    output.push_str("pub fn wrapper_get(req: Request<IncomingBody>, res: Responder) -> Pin<Box<dyn std::future::Future<Output = Finished>>> {\n");
    output.push_str("    Box::pin(wrapper_get_impl(req, res))\n");
    output.push_str("}\n\n");

    // Generate actual implementation
    output.push_str("async fn wrapper_get_impl(req: Request<IncomingBody>, res: Responder) -> Finished {\n");

    if params.is_empty() {
        // No path parameters
        output.push_str("    // Call user's get_props function\n");
        output.push_str("    let result = super::props::get_props().await;\n");
    } else {
        // Has path parameters - extract them
        output.push_str("    // Extract path parameters\n");
        output.push_str("    let path_params = extract_path_params(&req);\n");

        // Create Path struct initialization
        let path_struct_name = get_path_struct_name(&route_path);
        output.push_str(&format!("    let path = super::props::{} {{\n", path_struct_name));

        for param in &params {
            output.push_str(&format!(
                "        {}: path_params.get(\"{}\").and_then(|s| s.parse().ok()).unwrap_or_default(),\n",
                param, param
            ));
        }

        output.push_str("    };\n\n");
        output.push_str("    // Call user's get_props function\n");
        output.push_str("    let result = super::props::get_props(path).await;\n");
    }

    output.push_str("\n");
    output.push_str("    // Handle Result (anyhow::Result<PageProps>)\n");
    output.push_str("    match result {\n");
    output.push_str("        Ok(props) => {\n");
    output.push_str("            // Serialize to JSON\n");
    output.push_str("            let json = serde_json::to_string(&props).unwrap();\n\n");
    output.push_str("            // Return JSON response\n");
    output.push_str("            let response: Response<BoundedBody<Vec<u8>>> = Response::builder()\n");
    output.push_str("                .status(StatusCode::OK)\n");
    output.push_str("                .header(\"Content-Type\", \"application/json\")\n");
    output.push_str("                .body(json.into_body())\n");
    output.push_str("                .unwrap();\n\n");
    output.push_str("            res.respond(response).await\n");
    output.push_str("        }\n");
    output.push_str("        Err(e) => {\n");
    output.push_str("            // System error - return 500 Internal Server Error\n");
    output.push_str("            let error_msg = format!(\"{{\\\"error\\\": \\\"Internal Server Error\\\", \\\"message\\\": \\\"{}\\\"}}\", e);\n\n");
    output.push_str("            let response: Response<BoundedBody<Vec<u8>>> = Response::builder()\n");
    output.push_str("                .status(StatusCode::INTERNAL_SERVER_ERROR)\n");
    output.push_str("                .header(\"Content-Type\", \"application/json\")\n");
    output.push_str("                .body(error_msg.into_body())\n");
    output.push_str("                .unwrap();\n\n");
    output.push_str("            res.respond(response).await\n");
    output.push_str("        }\n");
    output.push_str("    }\n");
    output.push_str("}\n\n");

    // Generate wrapper_post if route has actions
    if route.has_action_input {
        output.push_str("pub fn wrapper_post(req: Request<IncomingBody>, res: Responder) -> Pin<Box<dyn std::future::Future<Output = Finished>>> {\n");
        output.push_str("    Box::pin(wrapper_post_impl(req, res))\n");
        output.push_str("}\n\n");

        output.push_str("async fn wrapper_post_impl(mut req: Request<IncomingBody>, res: Responder) -> Finished {\n");

        if params.is_empty() {
            // No path parameters
            output.push_str("    // Parse request body\n");
            output.push_str("    let mut body_bytes = Vec::new();\n");
            output.push_str("    if let Err(e) = req.body_mut().read_to_end(&mut body_bytes).await {\n");
            output.push_str("        let error_msg = format!(\"{{\\\"error\\\": \\\"Bad Request\\\", \\\"message\\\": \\\"Failed to read body: {}\\\"}}\", e);\n");
            output.push_str("        let response: Response<BoundedBody<Vec<u8>>> = Response::builder()\n");
            output.push_str("            .status(StatusCode::BAD_REQUEST)\n");
            output.push_str("            .header(\"Content-Type\", \"application/json\")\n");
            output.push_str("            .body(error_msg.into_body())\n");
            output.push_str("            .unwrap();\n");
            output.push_str("        return res.respond(response).await;\n");
            output.push_str("    }\n\n");

            output.push_str("    // Deserialize ActionInput\n");
            output.push_str("    let input: super::props::ActionInput = match serde_json::from_slice(&body_bytes) {\n");
            output.push_str("        Ok(input) => input,\n");
            output.push_str("        Err(e) => {\n");
            output.push_str("            let error_msg = format!(\"{{\\\"error\\\": \\\"Bad Request\\\", \\\"message\\\": \\\"Invalid JSON: {}\\\"}}\", e);\n");
            output.push_str("            let response: Response<BoundedBody<Vec<u8>>> = Response::builder()\n");
            output.push_str("                .status(StatusCode::BAD_REQUEST)\n");
            output.push_str("                .header(\"Content-Type\", \"application/json\")\n");
            output.push_str("                .body(error_msg.into_body())\n");
            output.push_str("                .unwrap();\n");
            output.push_str("            return res.respond(response).await;\n");
            output.push_str("        }\n");
            output.push_str("    };\n\n");

            // Add validation check if ActionInput has #[derive(Validate)]
            if route.has_validate {
                output.push_str("    // Validate input\n");
                output.push_str("    if let Err(validation_errors) = input.validate() {\n");
                output.push_str("        let errors: Vec<_> = validation_errors.field_errors().iter().flat_map(|(field, errors)| {\n");
                output.push_str("            errors.iter().map(move |error| {\n");
                output.push_str("                serde_json::json!({\n");
                output.push_str("                    \"field\": field,\n");
                output.push_str("                    \"message\": error.message.as_ref().map(|m| m.to_string()).unwrap_or_else(|| error.code.to_string())\n");
                output.push_str("                })\n");
                output.push_str("            })\n");
                output.push_str("        }).collect();\n");
                output.push_str("        let error_response = serde_json::json!({\n");
                output.push_str("            \"error\": \"Validation Error\",\n");
                output.push_str("            \"errors\": errors\n");
                output.push_str("        });\n");
                output.push_str("        let response: Response<BoundedBody<Vec<u8>>> = Response::builder()\n");
                output.push_str("            .status(StatusCode::BAD_REQUEST)\n");
                output.push_str("            .header(\"Content-Type\", \"application/json\")\n");
                output.push_str("            .body(serde_json::to_string(&error_response).unwrap().into_body())\n");
                output.push_str("            .unwrap();\n");
                output.push_str("        return res.respond(response).await;\n");
                output.push_str("    }\n\n");
            }

            output.push_str("    // Call user's post_action function\n");
            output.push_str("    let result = super::props::post_action(input).await;\n\n");
        } else {
            // Has path parameters
            output.push_str("    // Extract path parameters\n");
            output.push_str("    let path_params = extract_path_params(&req);\n");

            let path_struct_name = get_path_struct_name(&route_path);
            output.push_str(&format!("    let path = super::props::{} {{\n", path_struct_name));

            for param in &params {
                output.push_str(&format!(
                    "        {}: path_params.get(\"{}\").and_then(|s| s.parse().ok()).unwrap_or_default(),\n",
                    param, param
                ));
            }

            output.push_str("    };\n\n");

            output.push_str("    // Parse request body\n");
            output.push_str("    let mut body_bytes = Vec::new();\n");
            output.push_str("    if let Err(e) = req.body_mut().read_to_end(&mut body_bytes).await {\n");
            output.push_str("        let error_msg = format!(\"{{\\\"error\\\": \\\"Bad Request\\\", \\\"message\\\": \\\"Failed to read body: {}\\\"}}\", e);\n");
            output.push_str("        let response: Response<BoundedBody<Vec<u8>>> = Response::builder()\n");
            output.push_str("            .status(StatusCode::BAD_REQUEST)\n");
            output.push_str("            .header(\"Content-Type\", \"application/json\")\n");
            output.push_str("            .body(error_msg.into_body())\n");
            output.push_str("            .unwrap();\n");
            output.push_str("        return res.respond(response).await;\n");
            output.push_str("    }\n\n");

            output.push_str("    // Deserialize ActionInput\n");
            output.push_str("    let input: super::props::ActionInput = match serde_json::from_slice(&body_bytes) {\n");
            output.push_str("        Ok(input) => input,\n");
            output.push_str("        Err(e) => {\n");
            output.push_str("            let error_msg = format!(\"{{\\\"error\\\": \\\"Bad Request\\\", \\\"message\\\": \\\"Invalid JSON: {}\\\"}}\", e);\n");
            output.push_str("            let response: Response<BoundedBody<Vec<u8>>> = Response::builder()\n");
            output.push_str("                .status(StatusCode::BAD_REQUEST)\n");
            output.push_str("                .header(\"Content-Type\", \"application/json\")\n");
            output.push_str("                .body(error_msg.into_body())\n");
            output.push_str("                .unwrap();\n");
            output.push_str("            return res.respond(response).await;\n");
            output.push_str("        }\n");
            output.push_str("    };\n\n");

            // Add validation check if ActionInput has #[derive(Validate)]
            if route.has_validate {
                output.push_str("    // Validate input\n");
                output.push_str("    if let Err(validation_errors) = input.validate() {\n");
                output.push_str("        let errors: Vec<_> = validation_errors.field_errors().iter().flat_map(|(field, errors)| {\n");
                output.push_str("            errors.iter().map(move |error| {\n");
                output.push_str("                serde_json::json!({\n");
                output.push_str("                    \"field\": field,\n");
                output.push_str("                    \"message\": error.message.as_ref().map(|m| m.to_string()).unwrap_or_else(|| error.code.to_string())\n");
                output.push_str("                })\n");
                output.push_str("            })\n");
                output.push_str("        }).collect();\n");
                output.push_str("        let error_response = serde_json::json!({\n");
                output.push_str("            \"error\": \"Validation Error\",\n");
                output.push_str("            \"errors\": errors\n");
                output.push_str("        });\n");
                output.push_str("        let response: Response<BoundedBody<Vec<u8>>> = Response::builder()\n");
                output.push_str("            .status(StatusCode::BAD_REQUEST)\n");
                output.push_str("            .header(\"Content-Type\", \"application/json\")\n");
                output.push_str("            .body(serde_json::to_string(&error_response).unwrap().into_body())\n");
                output.push_str("            .unwrap();\n");
                output.push_str("        return res.respond(response).await;\n");
                output.push_str("    }\n\n");
            }

            output.push_str("    // Call user's post_action function\n");
            output.push_str("    let result = super::props::post_action(path, input).await;\n\n");
        }

        // Handle 3-level Result
        output.push_str("    // Handle 3-level Result: anyhow::Result<Result<Response, Error>>\n");
        output.push_str("    match result {\n");
        output.push_str("        Err(e) => {\n");
        output.push_str("            // System error - return 500 Internal Server Error\n");
        output.push_str("            let error_msg = format!(\"{{\\\"error\\\": \\\"Internal Server Error\\\", \\\"message\\\": \\\"{}\\\"}}\", e);\n");
        output.push_str("            let response: Response<BoundedBody<Vec<u8>>> = Response::builder()\n");
        output.push_str("                .status(StatusCode::INTERNAL_SERVER_ERROR)\n");
        output.push_str("                .header(\"Content-Type\", \"application/json\")\n");
        output.push_str("                .body(error_msg.into_body())\n");
        output.push_str("                .unwrap();\n");
        output.push_str("            res.respond(response).await\n");
        output.push_str("        }\n");
        output.push_str("        Ok(Ok(response_data)) => {\n");
        output.push_str("            // Success - return 200 with Response JSON\n");
        output.push_str("            let json = serde_json::to_string(&response_data).unwrap();\n");
        output.push_str("            let response: Response<BoundedBody<Vec<u8>>> = Response::builder()\n");
        output.push_str("                .status(StatusCode::OK)\n");
        output.push_str("                .header(\"Content-Type\", \"application/json\")\n");
        output.push_str("                .body(json.into_body())\n");
        output.push_str("                .unwrap();\n");
        output.push_str("            res.respond(response).await\n");
        output.push_str("        }\n");
        output.push_str("        Ok(Err(error_data)) => {\n");
        output.push_str("            // Business logic error - return 200 with Error JSON\n");
        output.push_str("            let json = serde_json::to_string(&error_data).unwrap();\n");
        output.push_str("            let response: Response<BoundedBody<Vec<u8>>> = Response::builder()\n");
        output.push_str("                .status(StatusCode::OK)\n");
        output.push_str("                .header(\"Content-Type\", \"application/json\")\n");
        output.push_str("                .body(json.into_body())\n");
        output.push_str("                .unwrap();\n");
        output.push_str("            res.respond(response).await\n");
        output.push_str("        }\n");
        output.push_str("    }\n");
        output.push_str("}\n");
    }

    Ok(output)
}

fn generate_router_wasm_module() -> String {
    r#"// [Generated] Dynamic router for WASM backend
// This file is auto-managed by Forte CLI

use std::collections::HashMap;
use wstd::http::body::{BoundedBody, IncomingBody};
use wstd::http::server::{Finished, Responder};
use wstd::http::{IntoBody, Request, Response, StatusCode};

pub type Handler = fn(Request<IncomingBody>, Responder) -> std::pin::Pin<Box<dyn std::future::Future<Output = Finished>>>;

pub struct Router {
    routes: Vec<Route>,
}

struct Route {
    method: &'static str,
    pattern: PathPattern,
    handler: Handler,
}

#[derive(Debug)]
struct PathPattern {
    segments: Vec<Segment>,
}

#[derive(Debug)]
enum Segment {
    Static(String),
    Param(String),
}

impl PathPattern {
    fn from_path(path: &str) -> Self {
        let segments = path
            .trim_start_matches('/')
            .split('/')
            .filter(|s| !s.is_empty())
            .map(|s| {
                if s.starts_with(':') {
                    Segment::Param(s[1..].to_string())
                } else {
                    Segment::Static(s.to_string())
                }
            })
            .collect();

        PathPattern { segments }
    }

    fn matches(&self, path: &str) -> Option<HashMap<String, String>> {
        let path_segments: Vec<&str> = path
            .trim_start_matches('/')
            .split('/')
            .filter(|s| !s.is_empty())
            .collect();

        // Check if segment count matches
        if path_segments.len() != self.segments.len() {
            // Special case: empty path matches empty pattern
            if path_segments.is_empty() && self.segments.is_empty() {
                return Some(HashMap::new());
            }
            return None;
        }

        let mut params = HashMap::new();

        for (pattern_seg, path_seg) in self.segments.iter().zip(path_segments.iter()) {
            match pattern_seg {
                Segment::Static(expected) => {
                    if expected != path_seg {
                        return None;
                    }
                }
                Segment::Param(name) => {
                    params.insert(name.clone(), path_seg.to_string());
                }
            }
        }

        Some(params)
    }
}

impl Router {
    pub fn new() -> Self {
        Router { routes: vec![] }
    }

    pub fn add(&mut self, method: &'static str, path: &'static str, handler: Handler) {
        let pattern = PathPattern::from_path(path);
        self.routes.push(Route {
            method,
            pattern,
            handler,
        });
    }

    pub async fn handle(self, req: Request<IncomingBody>, res: Responder) -> Finished {
        let path = req.uri().path();
        let method = req.method().as_str();

        // Find matching route
        for route in &self.routes {
            if route.method != method {
                continue;
            }

            if let Some(params) = route.pattern.matches(path) {
                // Store params in request extensions
                let (mut parts, body) = req.into_parts();
                parts.extensions.insert(params);
                let req = Request::from_parts(parts, body);

                // Call handler
                return (route.handler)(req, res).await;
            }
        }

        // No matching route - 404
        let response: Response<BoundedBody<Vec<u8>>> = Response::builder()
            .status(StatusCode::NOT_FOUND)
            .body("Not Found".into_body())
            .unwrap();

        res.respond(response).await
    }
}

// Path parameters extractor
pub struct PathParams(pub HashMap<String, String>);

impl PathParams {
    pub fn get(&self, key: &str) -> Option<&str> {
        self.0.get(key).map(|s| s.as_str())
    }
}

pub fn extract_path_params(req: &Request<IncomingBody>) -> PathParams {
    req.extensions()
        .get::<HashMap<String, String>>()
        .cloned()
        .map(PathParams)
        .unwrap_or_else(|| PathParams(HashMap::new()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_path_pattern_static() {
        let pattern = PathPattern::from_path("/about");
        assert!(pattern.matches("/about").is_some());
        assert!(pattern.matches("/").is_none());
        assert!(pattern.matches("/about/more").is_none());
    }

    #[test]
    fn test_path_pattern_param() {
        let pattern = PathPattern::from_path("/product/:id");
        let params = pattern.matches("/product/123").unwrap();
        assert_eq!(params.get("id"), Some(&"123".to_string()));
        assert!(pattern.matches("/product").is_none());
        assert!(pattern.matches("/product/123/more").is_none());
    }

    #[test]
    fn test_path_pattern_multiple_params() {
        let pattern = PathPattern::from_path("/user/:userId/post/:postId");
        let params = pattern.matches("/user/42/post/99").unwrap();
        assert_eq!(params.get("userId"), Some(&"42".to_string()));
        assert_eq!(params.get("postId"), Some(&"99".to_string()));
    }

    #[test]
    fn test_path_pattern_root() {
        let pattern = PathPattern::from_path("/");
        assert!(pattern.matches("/").is_some());
        assert!(pattern.matches("/about").is_none());
    }
}
"#.to_string()
}

fn generate_router_module(routes: &[RouteInfo]) -> Result<String> {
    let mut output = String::new();
    let has_actions = routes.iter().any(|r| r.has_action_input);
    let has_path_params = routes.iter().any(|r| {
        let route_path = extract_route_path(&r.props_path).unwrap_or_default();
        !extract_route_params(&route_path).is_empty()
    });

    output.push_str("// [Generated] Do not edit manually\n\n");

    // Build conditional imports
    let mut routing_imports = vec!["get"];
    if has_actions {
        routing_imports.push("post");
    }

    let mut extract_imports = vec!["Json"];
    if has_path_params {
        extract_imports.push("Path");
    }

    output.push_str(&format!(
        "use axum::{{Router, routing::{{{}}}, extract::{{{}}}}};\n",
        routing_imports.join(", "),
        extract_imports.join(", ")
    ));

    if has_actions {
        output.push_str("use serde::Deserialize;\n");
    }

    output.push_str("\nuse backend::routes;\n");

    // Only import ActionResult if there are routes with actions
    if has_actions {
        output.push_str("use backend::ActionResult;\n");
    }

    output.push_str("use super::error::AppError;\n");

    // Import ValidatedJson if there are actions
    if has_actions {
        output.push_str("use super::error::ValidatedJson;\n");
    }

    output.push_str("\n");

    // Generate route handlers
    for route in routes {
        let route_path = extract_route_path(&route.props_path)?;
        let handler_name = route_path_to_handler_name(&route_path);
        let module_path = route_path_to_module_path(&route_path);
        let params = extract_route_params(&route_path);

        if params.is_empty() {
            // No parameters - simple handler
            output.push_str(&format!("// Handler for: {}\n", route_path));
            output.push_str(&format!(
                "async fn {}() -> Result<Json<routes::{}::PageProps>, AppError> {{\n",
                handler_name, module_path
            ));
            output.push_str(&format!(
                "    let props = routes::{}::get_props().await;\n",
                module_path
            ));
            output.push_str("    Ok(Json(props))\n");
            output.push_str("}\n\n");
        } else {
            // Has parameters - need Path extractor
            // Use the actual *Path struct from the route module
            let path_struct_name = get_path_struct_name(&route_path);

            // Generate handler with Path extractor
            output.push_str(&format!("// Handler for: {}\n", route_path));
            output.push_str(&format!(
                "async fn {}(Path(path): Path<routes::{}::{}>) -> Result<Json<routes::{}::PageProps>, AppError> {{\n",
                handler_name, module_path, path_struct_name, module_path
            ));

            // Call get_props with path struct
            output.push_str(&format!(
                "    let props = routes::{}::get_props(path).await;\n",
                module_path
            ));
            output.push_str("    Ok(Json(props))\n");
            output.push_str("}\n\n");
        }

        // Generate POST handler if route has ActionInput
        if route.has_action_input {
            let action_handler_name = format!("{}_action", handler_name);
            let params = extract_route_params(&route_path);

            if params.is_empty() {
                // No path parameters
                output.push_str(&format!("// POST handler for: {}\n", route_path));
                output.push_str(&format!(
                    "async fn {}(ValidatedJson(input): ValidatedJson<routes::{}::ActionInput>) -> Result<Json<ActionResult<routes::{}::PageProps>>, AppError> {{\n",
                    action_handler_name, module_path, module_path
                ));
                output.push_str(&format!(
                    "    let result = routes::{}::post_action(input).await;\n",
                    module_path
                ));
                output.push_str("    Ok(Json(result))\n");
                output.push_str("}\n\n");
            } else {
                // Has path parameters
                let path_struct_name = get_path_struct_name(&route_path);
                output.push_str(&format!("// POST handler for: {}\n", route_path));
                output.push_str(&format!(
                    "async fn {}(Path(path): Path<routes::{}::{}>, ValidatedJson(input): ValidatedJson<routes::{}::ActionInput>) -> Result<Json<ActionResult<routes::{}::PageProps>>, AppError> {{\n",
                    action_handler_name, module_path, path_struct_name, module_path, module_path
                ));
                output.push_str(&format!(
                    "    let result = routes::{}::post_action(path, input).await;\n",
                    module_path
                ));
                output.push_str("    Ok(Json(result))\n");
                output.push_str("}\n\n");
            }
        }
    }

    // Generate create_router function
    output.push_str("pub fn create_router() -> Router {\n");
    output.push_str("    Router::new()\n");

    for route in routes {
        let route_path = extract_route_path(&route.props_path)?;
        let url_path = convert_to_url_path(&route_path);
        let handler_name = route_path_to_handler_name(&route_path);

        if route.has_action_input {
            // Route with both GET and POST
            let action_handler_name = format!("{}_action", handler_name);
            output.push_str(&format!(
                "        .route(\"{}\", get({}).post({}))\n",
                url_path, handler_name, action_handler_name
            ));
        } else {
            // GET only route
            output.push_str(&format!(
                "        .route(\"{}\", get({}))\n",
                url_path, handler_name
            ));
        }
    }

    output.push_str("}\n");

    Ok(output)
}

fn generate_main_module() -> String {
    r#"// [Generated] Do not edit manually

mod error;
mod router;

#[tokio::main]
async fn main() {
    let app = router::create_router();

    let port = std::env::var("RUST_PORT").unwrap_or_else(|_| "8080".to_string());
    let addr = format!("127.0.0.1:{}", port);

    println!("Backend server listening on http://{}", addr);

    let listener = tokio::net::TcpListener::bind(&addr)
        .await
        .expect("Failed to bind to port");

    axum::serve(listener, app)
        .await
        .expect("Server failed");
}
"#
    .to_string()
}

fn generate_routes_mod(routes: &[RouteInfo]) -> Result<String> {
    let mut output = String::new();

    output.push_str("// [Generated] Do not edit manually\n");
    output.push_str("// This file is managed by the Forte CLI\n\n");

    // Collect all top-level modules
    let mut modules = std::collections::HashSet::new();
    for route in routes {
        let route_path = extract_route_path(&route.props_path)?;
        let first_segment = route_path.split('/').next().unwrap_or("index");
        modules.insert(first_segment.to_string());
    }

    // Generate pub mod declarations
    let mut sorted_modules: Vec<_> = modules.into_iter().collect();
    sorted_modules.sort();

    for module in sorted_modules {
        output.push_str(&format!("pub mod {};\n", module));
    }

    Ok(output)
}

/// Extract route path from props.rs full path
/// Example: /path/to/backend/src/routes/product/_id_/props.rs -> product/_id_
fn extract_route_path(props_path: &Path) -> Result<String> {
    let path_str = props_path.to_str().context("Invalid UTF-8 in path")?;

    // Find "routes/" and extract everything after it until "/props.rs"
    if let Some(routes_idx) = path_str.find("routes/") {
        let after_routes = &path_str[routes_idx + 7..]; // Skip "routes/"
        if let Some(props_idx) = after_routes.find("/props.rs") {
            return Ok(after_routes[..props_idx].to_string());
        }
    }

    anyhow::bail!("Could not extract route path from: {}", path_str)
}

/// Convert route path to handler function name
/// Example: product/_id_ -> handler_product_id
fn route_path_to_handler_name(route_path: &str) -> String {
    let normalized = route_path
        .replace('/', "_")
        .replace("_id_", "id")
        .replace("_userId_", "user_id")
        .replace("_postId_", "post_id");

    format!("handler_{}", normalized)
}

/// Convert route path to module path
/// Example: product/_id_ -> product::_id_
fn route_path_to_module_path(route_path: &str) -> String {
    route_path.replace('/', "::")
}

/// Extract route parameters from route path
/// Example: product/_id_/review/_reviewId_ -> vec!["id", "reviewId"]
fn extract_route_params(route_path: &str) -> Vec<String> {
    let mut params = Vec::new();

    for segment in route_path.split('/') {
        if segment.starts_with('_') && segment.ends_with('_') && segment.len() > 2 {
            let param_name = &segment[1..segment.len() - 1];
            params.push(param_name.to_string());
        }
    }

    params
}

/// Convert string to PascalCase
/// Example: handler_product_id -> HandlerProductId
fn to_pascal_case(s: &str) -> String {
    s.split('_')
        .map(|word| {
            let mut chars = word.chars();
            match chars.next() {
                None => String::new(),
                Some(first) => first.to_uppercase().chain(chars).collect(),
            }
        })
        .collect()
}

/// Get the expected *Path struct name for a route
/// Example: product/_id_ -> ProductIdPath
fn get_path_struct_name(route_path: &str) -> String {
    let normalized = route_path
        .replace('/', "_")
        .replace("_id_", "Id")
        .replace("_userId_", "UserId")
        .replace("_postId_", "PostId");

    format!("{}Path", to_pascal_case(&normalized))
}

/// Convert route path to URL path
/// Example: product/_id_ -> /product/:id
fn convert_to_url_path(route_path: &str) -> String {
    let mut url_path = String::from("/");

    for segment in route_path.split('/') {
        if segment.is_empty() {
            continue;
        }

        if segment == "index" {
            continue; // index becomes root path
        }

        // Skip route groups (segments wrapped in parentheses like (marketing))
        // These are used for organization but don't appear in URLs
        if segment.starts_with('(') && segment.ends_with(')') {
            continue;
        }

        // Convert _paramName_ to :paramName
        if segment.starts_with('_') && segment.ends_with('_') && segment.len() > 2 {
            let param_name = &segment[1..segment.len() - 1];
            url_path.push(':');
            url_path.push_str(param_name);
        } else {
            url_path.push_str(segment);
        }

        url_path.push('/');
    }

    // Remove trailing slash unless it's the root path
    if url_path.len() > 1 && url_path.ends_with('/') {
        url_path.pop();
    }

    url_path
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_url_conversion() {
        assert_eq!(convert_to_url_path("index"), "/");
        assert_eq!(convert_to_url_path("about"), "/about");
        assert_eq!(convert_to_url_path("product/_id_"), "/product/:id");
        assert_eq!(
            convert_to_url_path("user/_userId_/post/_postId_"),
            "/user/:userId/post/:postId"
        );
    }

    #[test]
    fn test_route_groups() {
        // Route groups (wrapped in parentheses) should not appear in URLs
        assert_eq!(convert_to_url_path("(marketing)/about"), "/about");
        assert_eq!(convert_to_url_path("(marketing)/contact"), "/contact");
        assert_eq!(
            convert_to_url_path("(admin)/dashboard"),
            "/dashboard"
        );
        // Multiple groups in one path
        assert_eq!(
            convert_to_url_path("(app)/(dashboard)/settings"),
            "/settings"
        );
        // Groups with dynamic segments
        assert_eq!(
            convert_to_url_path("(blog)/post/_id_"),
            "/post/:id"
        );
    }

    #[test]
    fn test_handler_name() {
        assert_eq!(route_path_to_handler_name("index"), "handler_index");
        assert_eq!(
            route_path_to_handler_name("product/_id_"),
            "handler_product_id"
        );
    }

    #[test]
    fn test_env_module_generation() {
        use std::collections::HashMap;

        let mut config = crate::config::ForteConfig::default();
        config.env.required = vec!["DATABASE_URL".to_string(), "PORT".to_string()];
        config.env.optional = vec!["DEBUG".to_string()];

        let mut types = HashMap::new();
        types.insert("PORT".to_string(), "i32".to_string());
        types.insert("DEBUG".to_string(), "bool".to_string());
        config.env.types = types;

        let mut defaults = HashMap::new();
        defaults.insert("PORT".to_string(), "3000".to_string());
        config.env.defaults = defaults;

        let env_rs = generate_env_module(&config);

        // Check struct fields
        assert!(env_rs.contains("pub database_url: String"));
        assert!(env_rs.contains("pub port: i32"));
        assert!(env_rs.contains("pub debug: Option<bool>"));

        // Check parsing logic
        assert!(env_rs.contains("parse::<i32>()"));
        assert!(env_rs.contains("parse::<bool>()"));
    }

    #[test]
    fn test_env_example_generation() {
        use std::collections::HashMap;

        let mut config = crate::config::ForteConfig::default();
        config.env.required = vec!["DATABASE_URL".to_string(), "PORT".to_string()];
        config.env.optional = vec!["DEBUG".to_string()];

        let mut types = HashMap::new();
        types.insert("PORT".to_string(), "i32".to_string());
        types.insert("DEBUG".to_string(), "bool".to_string());
        config.env.types = types;

        let mut defaults = HashMap::new();
        defaults.insert("PORT".to_string(), "3000".to_string());
        defaults.insert("DEBUG".to_string(), "false".to_string());
        config.env.defaults = defaults;

        let env_example = generate_env_example(&config);

        // Check required vars
        assert!(env_example.contains("DATABASE_URL="));
        assert!(env_example.contains("PORT=3000"));

        // Check optional vars (should be commented out)
        assert!(env_example.contains("# DEBUG=false"));

        // Check type comments
        assert!(env_example.contains("integer"));
        assert!(env_example.contains("boolean"));
    }
}

/// Get the Rust type for an environment variable
fn get_env_var_type(var_name: &str, config: &ForteConfig) -> String {
    config.env.types
        .get(var_name)
        .map(|s| s.as_str())
        .unwrap_or("String")
        .to_string()
}

/// Generate parsing code for an environment variable based on its type
fn generate_env_parse_code(var_name: &str, _field_name: &str, var_type: &str, default: Option<&String>) -> String {
    match var_type {
        "String" => {
            if let Some(default_val) = default {
                format!(
                    "std::env::var(\"{}\").unwrap_or_else(|_| \"{}\".to_string())",
                    var_name, default_val
                )
            } else {
                format!(
                    "std::env::var(\"{}\").expect(\"Missing required env var: {}\")",
                    var_name, var_name
                )
            }
        }
        "i32" | "i64" | "u32" | "u64" | "f32" | "f64" => {
            if let Some(default_val) = default {
                format!(
                    "std::env::var(\"{}\")\n                .unwrap_or_else(|_| \"{}\".to_string())\n                .parse::<{}>()\n                .expect(\"Failed to parse {} as {}\")",
                    var_name, default_val, var_type, var_name, var_type
                )
            } else {
                format!(
                    "std::env::var(\"{}\")\n                .expect(\"Missing required env var: {}\")\n                .parse::<{}>()\n                .expect(\"Failed to parse {} as {}\")",
                    var_name, var_name, var_type, var_name, var_type
                )
            }
        }
        "bool" => {
            if let Some(default_val) = default {
                format!(
                    "std::env::var(\"{}\")\n                .unwrap_or_else(|_| \"{}\".to_string())\n                .parse::<bool>()\n                .expect(\"Failed to parse {} as bool (use 'true' or 'false')\")",
                    var_name, default_val, var_name
                )
            } else {
                format!(
                    "std::env::var(\"{}\")\n                .expect(\"Missing required env var: {}\")\n                .parse::<bool>()\n                .expect(\"Failed to parse {} as bool (use 'true' or 'false')\")",
                    var_name, var_name, var_name
                )
            }
        }
        _ => {
            // Unknown type, default to String
            if let Some(default_val) = default {
                format!(
                    "std::env::var(\"{}\").unwrap_or_else(|_| \"{}\".to_string())",
                    var_name, default_val
                )
            } else {
                format!(
                    "std::env::var(\"{}\").expect(\"Missing required env var: {}\")",
                    var_name, var_name
                )
            }
        }
    }
}

/// Generate parsing code for optional environment variables
fn generate_optional_env_parse_code(var_name: &str, var_type: &str, default: Option<&String>) -> String {
    match var_type {
        "String" => {
            if let Some(default_val) = default {
                format!(
                    "Some(std::env::var(\"{}\").unwrap_or_else(|_| \"{}\".to_string()))",
                    var_name, default_val
                )
            } else {
                format!("std::env::var(\"{}\").ok()", var_name)
            }
        }
        "i32" | "i64" | "u32" | "u64" | "f32" | "f64" | "bool" => {
            if let Some(default_val) = default {
                format!(
                    "std::env::var(\"{}\")\n                .unwrap_or_else(|_| \"{}\".to_string())\n                .parse::<{}>()\n                .ok()",
                    var_name, default_val, var_type
                )
            } else {
                format!(
                    "std::env::var(\"{}\")\n                .ok()\n                .and_then(|s| s.parse::<{}>().ok())",
                    var_name, var_type
                )
            }
        }
        _ => {
            // Unknown type, default to String
            if let Some(default_val) = default {
                format!(
                    "Some(std::env::var(\"{}\").unwrap_or_else(|_| \"{}\".to_string()))",
                    var_name, default_val
                )
            } else {
                format!("std::env::var(\"{}\").ok()", var_name)
            }
        }
    }
}

/// Generate .env.example file based on Forte.toml configuration
fn generate_env_example(config: &ForteConfig) -> String {
    let mut output = String::new();

    output.push_str("# Environment Variables Example\n");
    output.push_str("# Copy this file to .env and fill in the values\n");
    output.push_str("# Generated from Forte.toml\n\n");

    // Add required variables
    if !config.env.required.is_empty() {
        output.push_str("# Required variables\n");
        for var in &config.env.required {
            let var_type = get_env_var_type(var, config);
            let type_comment = match var_type.as_str() {
                "String" => "string",
                "i32" | "i64" | "u32" | "u64" => "integer",
                "f32" | "f64" => "float",
                "bool" => "boolean (true/false)",
                _ => "string",
            };

            if let Some(default) = config.env.defaults.get(var) {
                output.push_str(&format!("{}={}  # {} (default: {})\n", var, default, type_comment, default));
            } else {
                output.push_str(&format!("{}=  # {} (required)\n", var, type_comment));
            }
        }
        output.push_str("\n");
    }

    // Add optional variables
    if !config.env.optional.is_empty() {
        output.push_str("# Optional variables\n");
        for var in &config.env.optional {
            let var_type = get_env_var_type(var, config);
            let type_comment = match var_type.as_str() {
                "String" => "string",
                "i32" | "i64" | "u32" | "u64" => "integer",
                "f32" | "f64" => "float",
                "bool" => "boolean (true/false)",
                _ => "string",
            };

            if let Some(default) = config.env.defaults.get(var) {
                output.push_str(&format!("# {}={}  # {} (optional, default: {})\n", var, default, type_comment, default));
            } else {
                output.push_str(&format!("# {}=  # {} (optional)\n", var, type_comment));
            }
        }
    }

    output
}

fn generate_env_module(config: &ForteConfig) -> String {
    let mut output = String::new();

    output.push_str("// [Generated] Do not edit manually\n");
    output.push_str("// Environment variables with type safety\n");
    output.push_str("//\n");
    output.push_str("// Configure types in Forte.toml:\n");
    output.push_str("// [env]\n");
    output.push_str("// required = [\"PORT\", \"DATABASE_URL\"]\n");
    output.push_str("// types = { PORT = \"i32\", DEBUG = \"bool\" }\n");
    output.push_str("// defaults = { PORT = \"3000\", DEBUG = \"false\" }\n\n");
    output.push_str("use std::sync::OnceLock;\n\n");

    // Generate Env struct
    output.push_str("pub struct Env {\n");

    // Add required fields with their types
    for var in &config.env.required {
        let field_name = var.to_lowercase();
        let var_type = get_env_var_type(var, config);
        output.push_str(&format!("    pub {}: {},\n", field_name, var_type));
    }

    // Add optional fields with their types
    for var in &config.env.optional {
        let field_name = var.to_lowercase();
        let var_type = get_env_var_type(var, config);
        output.push_str(&format!("    pub {}: Option<{}>,\n", field_name, var_type));
    }

    output.push_str("}\n\n");

    // Generate load function
    output.push_str("impl Env {\n");
    output.push_str("    fn load() -> Self {\n");
    output.push_str("        Self {\n");

    // Load required vars with type parsing
    for var in &config.env.required {
        let field_name = var.to_lowercase();
        let var_type = get_env_var_type(var, config);
        let default = config.env.defaults.get(var);
        let parse_code = generate_env_parse_code(var, &field_name, &var_type, default);
        output.push_str(&format!("            {}: {},\n", field_name, parse_code));
    }

    // Load optional vars with type parsing
    for var in &config.env.optional {
        let field_name = var.to_lowercase();
        let var_type = get_env_var_type(var, config);
        let default = config.env.defaults.get(var);
        let parse_code = generate_optional_env_parse_code(var, &var_type, default);
        output.push_str(&format!("            {}: {},\n", field_name, parse_code));
    }

    output.push_str("        }\n");
    output.push_str("    }\n");
    output.push_str("}\n\n");

    // Global static
    output.push_str("static ENV: OnceLock<Env> = OnceLock::new();\n\n");
    output.push_str("pub fn env() -> &'static Env {\n");
    output.push_str("    ENV.get_or_init(|| Env::load())\n");
    output.push_str("}\n");

    output
}
