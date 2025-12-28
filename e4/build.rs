#[allow(dead_code)]
#[path = "src/runtime_options.rs"]
mod runtime_options;

use deno_core::JsRuntimeForSnapshot;
use runtime_options::runtime_options;
use std::{env, fs, path::PathBuf};

fn main() {
    let runtime = JsRuntimeForSnapshot::new(runtime_options());
    let snapshot = runtime.snapshot();
    let out_dir = PathBuf::from(env::var_os("OUT_DIR").unwrap());
    let file_path = out_dir.join("RUNJS_SNAPSHOT.bin");
    fs::write(file_path, snapshot).expect("Failed to write snapshot");
}
