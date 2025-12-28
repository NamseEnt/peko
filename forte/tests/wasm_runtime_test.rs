use std::path::PathBuf;

#[tokio::test]
#[ignore] // Ignore by default since it requires a WASM file
async fn test_wasm_runtime_loads() {
    // This test requires a pre-built WASM file
    let wasm_path = PathBuf::from("/tmp/forte-test/test-app/backend/target/wasm32-wasip2/debug/backend.wasm");

    if !wasm_path.exists() {
        eprintln!("WASM file not found at {:?}, skipping test", wasm_path);
        return;
    }

    // Try to load the WASM runtime
    let result = forte::runtime::WasmRuntime::new(&wasm_path);

    match result {
        Ok(_runtime) => {
            println!("✓ WASM runtime loaded successfully");
        }
        Err(e) => {
            panic!("Failed to load WASM runtime: {}", e);
        }
    }
}
