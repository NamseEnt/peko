use std::env;
use std::fs;
use std::path::{Path, PathBuf};

fn main() {
    if env::var("INSIDE_BUILD_SCRIPT_ANALYSIS").is_ok() {
        return;
    }
    genearte_route();
}

fn genearte_route() {
    let manifest_dir = env::var("CARGO_MANIFEST_DIR").unwrap();
    let src_path = Path::new(&manifest_dir).join("src").join("pages");

    let dest_path = Path::new(&manifest_dir)
        .join("src")
        .join("route_generated.rs");

    println!("cargo:rerun-if-changed=src/pages");

    let mut routes = Vec::new();

    if src_path.exists() {
        collect_routes(&src_path, &src_path, &mut routes);
    }

    let mut output = String::new();

    for route in &routes {
        let file_path = route.file_path.to_string_lossy().replace("\\", "/");
        let relative_path = format!("pages/{}", file_path.split("src/pages/").nth(1).unwrap());

        output.push_str(&format!("#[path = \"{}\"]\n", relative_path));
        output.push_str(&format!("mod {};\n", route.mod_name));
    }

    output.push_str("\nuse anyhow::Result;\n");
    output.push_str("use wstd::http::{Error, Request, Response, StatusCode, body::Body};\n\n");

    output.push_str("#[wstd::http_server]\n");
    output
        .push_str("pub async fn main(request: Request<Body>) -> Result<Response<Body>, Error> {\n");
    output.push_str("    let (parts, _body) = request.into_parts();\n");
    output.push_str("    let headers = parts.headers;\n");
    output.push_str("    let path = parts.uri.path_and_query().unwrap().as_str();\n");
    output.push_str("    println!(\"serving {path}\");\n");
    output.push_str("    let path_parts: Vec<&str> = path.split('/').skip(1).collect();\n\n");
    output.push_str("    println!(\"path_parts: {:?}\", path_parts);\n");

    for route in &routes {
        output.push_str(&generate_route_block(route));
    }

    output.push_str("    Ok(Response::builder()\n");
    output.push_str("        .status(StatusCode::NOT_FOUND)\n");
    output.push_str("        .body(Body::empty())\n");
    output.push_str("        .unwrap())\n");
    output.push_str("}\n");

    fs::write(&dest_path, output).unwrap();
}

struct RouteInfo {
    file_path: PathBuf,
    mod_name: String,
    segments: Vec<String>,
}

fn collect_routes(base_dir: &Path, current_dir: &Path, routes: &mut Vec<RouteInfo>) {
    let entries = fs::read_dir(current_dir).unwrap();

    for entry in entries {
        let entry = entry.unwrap();
        let path = entry.path();

        if path.is_dir() {
            collect_routes(base_dir, &path, routes);
        } else if path.file_name().unwrap() == "mod.rs" {
            let content = fs::read_to_string(&path).unwrap();
            if content.contains("pub async fn handler(") {
                process_route(base_dir, &path, routes);
            }
        }
    }
}

fn process_route(base_dir: &Path, file_path: &Path, routes: &mut Vec<RouteInfo>) {
    let dir_path = file_path.parent().unwrap();
    let relative = dir_path.strip_prefix(base_dir).unwrap();

    let mut segments = Vec::new();
    for component in relative.components() {
        segments.push(component.as_os_str().to_string_lossy().to_string());
    }

    let relative_str = relative.to_string_lossy();
    let mod_name_suffix = relative_str
        .replace(std::path::MAIN_SEPARATOR, "_")
        .replace("[", "1")
        .replace("]", "1");

    let mod_name = format!("pages_{}", mod_name_suffix);

    routes.push(RouteInfo {
        file_path: file_path.to_path_buf(),
        mod_name,
        segments,
    });
}

fn generate_route_block(route: &RouteInfo) -> String {
    let mut block = String::new();

    let url_parts = if route.segments.len() == 1 && route.segments[0] == "index" {
        vec!["".to_string()]
    } else {
        route.segments.clone()
    };

    let part_count = url_parts.len();

    block.push_str("    if path_parts.len() == ");
    block.push_str(&part_count.to_string());

    for (i, part) in url_parts.iter().enumerate() {
        if part.starts_with('[') {
            continue;
        }
        if part.is_empty() {
            block.push_str(&format!(" && path_parts[{i}].is_empty()"));
        } else {
            block.push_str(&format!(" && path_parts[{i}] == \"{part}\""));
        }
    }
    block.push_str(" {\n");

    let mut args = vec!["headers".to_string()];

    for (i, part) in url_parts.iter().enumerate() {
        if part.starts_with('[') && part.ends_with(']') {
            let var_name = &part[1..part.len() - 1];
            block.push_str(&format!(
                "        let Ok({var_name}) = path_parts[{i}].parse() else {{\n",
            ));
            block.push_str("            return Ok(Response::builder()\n");
            block.push_str("                .status(StatusCode::BAD_REQUEST)\n");
            block.push_str("                .body(Body::from(\"Invalid parameter\"))\n");
            block.push_str("                .unwrap());\n        };\n");
            args.push(var_name.to_string());
        }
    }

    block.push_str(&format!("        println!(\"Calling {}::handler\");\n", route.mod_name));
    block.push_str(&format!("        match {}::handler({}).await {{\n", route.mod_name, args.join(", ")));
    block.push_str("            Ok(props) => {\n");
    block.push_str("                println!(\"Handler succeeded, converting to stream\");\n");
    block.push_str("                let stream = forte_json::to_stream(&props);\n");
    block.push_str("                println!(\"Stream conversion completed\");\n");
    block.push_str("                return Ok(Response::new(Body::from_stream(stream)));\n");
    block.push_str("            }\n");
    block.push_str("            Err(e) => {\n");
    block.push_str(&format!("                eprintln!(\"{}::handler failed: {{:?}}\", e);\n", route.mod_name));
    block.push_str("                return Ok(Response::builder()\n");
    block.push_str("                    .status(StatusCode::INTERNAL_SERVER_ERROR)\n");
    block.push_str("                    .body(Body::from(format!(\"Handler error: {:?}\", e)))\n");
    block.push_str("                    .unwrap());\n");
    block.push_str("            }\n");
    block.push_str("        }\n");
    block.push_str("    }\n");

    block
}
