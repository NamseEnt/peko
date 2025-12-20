use schemars::schema_for;

#[path = "src/args.rs"]
mod args;

fn main() {
    let schema = schema_for!(args::HqArgs);
    let schema_json = serde_json::to_string_pretty(&schema).unwrap();

    std::fs::write("host-provider.schema.json", schema_json).expect("Failed to write schema file");

    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=src/args.rs");
}
