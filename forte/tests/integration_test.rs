use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::RwLock;

#[tokio::test]
#[ignore] // Manual test only
async fn test_proxy_server_http() {
    let wasm_path = PathBuf::from("/tmp/forte-test/test-app/backend/target/wasm32-wasip2/debug/backend.wasm");

    if !wasm_path.exists() {
        eprintln!("WASM file not found, skipping test");
        return;
    }

    // Load WASM runtime
    let runtime = forte::runtime::WasmRuntime::new(&wasm_path)
        .expect("Failed to load WASM runtime");
    let runtime = Arc::new(RwLock::new(runtime));

    // Create proxy server
    let static_dir = PathBuf::from("/tmp/forte-test/test-app/frontend/dist");
    let node_url = "http://127.0.0.1:5173".to_string();
    let proxy = forte::server::ForteProxy::new(runtime, static_dir, node_url);

    // Start server in background
    let addr: std::net::SocketAddr = "127.0.0.1:13000".parse().unwrap();
    let server_handle = tokio::spawn(async move {
        if let Err(e) = proxy.serve(addr).await {
            eprintln!("Server error: {}", e);
        }
    });

    // Wait for server to start
    tokio::time::sleep(Duration::from_secs(1)).await;

    // Make HTTP request
    let client = reqwest::Client::new();
    let response = client
        .get("http://127.0.0.1:13000/")
        .send()
        .await
        .expect("Failed to send request");

    println!("Response status: {}", response.status());

    let body = response.text().await.expect("Failed to read response body");
    println!("Response body: {}", body);
    println!("Response body length: {}", body.len());

    // Verify response - should be JSON or contain expected text
    let is_json = body.trim().starts_with('{');
    println!("Is JSON: {}", is_json);

    if is_json {
        // Try to parse as JSON
        if let Ok(json) = serde_json::from_str::<serde_json::Value>(&body) {
            println!("Parsed JSON: {}", serde_json::to_string_pretty(&json).unwrap());
        }
    }

    assert!(body.contains("Forte") || body.contains("TODO") || body.contains("message"), "Response should contain expected content");

    // Cleanup
    server_handle.abort();

    println!("✓ HTTP request handled successfully");
}
