use std::path::PathBuf;

#[tokio::test]
async fn test_proxy_server_creation() {
    use std::sync::Arc;
    use tokio::sync::RwLock;

    let wasm_path = PathBuf::from("/tmp/forte-test/test-app/backend/target/wasm32-wasip2/debug/backend.wasm");

    if !wasm_path.exists() {
        eprintln!("WASM file not found, skipping test");
        return;
    }

    let runtime = forte::runtime::WasmRuntime::new(&wasm_path)
        .expect("Failed to load WASM runtime");
    let runtime = Arc::new(RwLock::new(runtime));

    let static_dir = PathBuf::from("/tmp/forte-test/test-app/frontend/dist");
    let node_url = "http://127.0.0.1:5173".to_string();

    let _proxy = forte::server::ForteProxy::new(
        runtime,
        static_dir,
        node_url,
    );

    println!("✓ ForteProxy created successfully");
}
