use anyhow::{Context, Result};
use std::env;
use std::process::Command;

pub fn execute() -> Result<()> {
    let current_dir = env::current_dir().context("Failed to get current directory")?;

    println!("Running Forte project tests...");
    println!("Project root: {}\n", current_dir.display());

    // Verify we're in a Forte project
    let forte_toml = current_dir.join("Forte.toml");
    if !forte_toml.exists() {
        anyhow::bail!(
            "Forte.toml not found. Are you in a Forte project root?\nRun 'forte init <project-name>' to create a new project."
        );
    }

    let mut all_tests_passed = true;

    // Run backend tests
    println!("Running backend tests...");
    println!("{}", "=".repeat(60));
    let backend_dir = current_dir.join("backend");
    let backend_test_status = Command::new("cargo")
        .arg("test")
        .current_dir(&backend_dir)
        .status()
        .context("Failed to run cargo test")?;

    if !backend_test_status.success() {
        eprintln!("\n❌ Backend tests failed");
        all_tests_passed = false;
    } else {
        println!("\n✓ Backend tests passed");
    }

    // Run frontend tests
    println!("\n{}", "=".repeat(60));
    println!("Running frontend tests...");
    println!("{}", "=".repeat(60));
    let frontend_dir = current_dir.join("frontend");

    // Check if frontend has a test script
    let package_json_path = frontend_dir.join("package.json");
    if package_json_path.exists() {
        let package_json = std::fs::read_to_string(&package_json_path)
            .context("Failed to read frontend package.json")?;

        if package_json.contains("\"test\"") {
            let frontend_test_status = Command::new("npm")
                .arg("test")
                .arg("--")
                .arg("--run") // For Vitest, run once and exit
                .current_dir(&frontend_dir)
                .status()
                .context("Failed to run npm test")?;

            if !frontend_test_status.success() {
                eprintln!("\n❌ Frontend tests failed");
                all_tests_passed = false;
            } else {
                println!("\n✓ Frontend tests passed");
            }
        } else {
            println!("\n⚠️  No test script found in frontend/package.json");
            println!("Add a \"test\" script to run frontend tests");
        }
    } else {
        println!("\n⚠️  No package.json found in frontend directory");
    }

    // Summary
    println!("\n{}", "=".repeat(60));
    if all_tests_passed {
        println!("✓ All tests passed!");
        Ok(())
    } else {
        anyhow::bail!("Some tests failed. See output above for details.");
    }
}
