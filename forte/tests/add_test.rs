use assert_cmd::Command;
use predicates::prelude::*;

fn setup_project(temp: &tempfile::TempDir) -> std::path::PathBuf {
    Command::cargo_bin("forte")
        .unwrap()
        .args(["init", "my-app"])
        .current_dir(temp)
        .assert()
        .success();

    temp.path().join("my-app")
}

// ============ Page Tests ============

#[test]
fn test_add_page_creates_files() {
    let temp = tempfile::tempdir().unwrap();
    let project_dir = setup_project(&temp);

    Command::cargo_bin("forte")
        .unwrap()
        .args(["add", "page", "about"])
        .current_dir(&project_dir)
        .assert()
        .success()
        .stdout(predicate::str::contains("Created page 'about'"));

    assert!(project_dir.join("rs/src/pages/about/mod.rs").exists());
    assert!(project_dir.join("fe/src/pages/about/page.tsx").exists());

    let backend_content = std::fs::read_to_string(project_dir.join("rs/src/pages/about/mod.rs")).unwrap();
    assert!(backend_content.contains("pub async fn handler"));
    assert!(backend_content.contains("pub enum Props"));

    let frontend_content = std::fs::read_to_string(project_dir.join("fe/src/pages/about/page.tsx")).unwrap();
    assert!(frontend_content.contains("AboutPage"));
    assert!(frontend_content.contains("Props"));
}

#[test]
fn test_add_page_nested_path() {
    let temp = tempfile::tempdir().unwrap();
    let project_dir = setup_project(&temp);

    Command::cargo_bin("forte")
        .unwrap()
        .args(["add", "page", "product/detail"])
        .current_dir(&project_dir)
        .assert()
        .success();

    assert!(project_dir.join("rs/src/pages/product/detail/mod.rs").exists());
    assert!(project_dir.join("fe/src/pages/product/detail/page.tsx").exists());
}

#[test]
fn test_add_page_dynamic_route() {
    let temp = tempfile::tempdir().unwrap();
    let project_dir = setup_project(&temp);

    Command::cargo_bin("forte")
        .unwrap()
        .args(["add", "page", "product/[id]"])
        .current_dir(&project_dir)
        .assert()
        .success();

    assert!(project_dir.join("rs/src/pages/product/[id]/mod.rs").exists());
    assert!(project_dir.join("fe/src/pages/product/[id]/page.tsx").exists());

    let backend_content = std::fs::read_to_string(project_dir.join("rs/src/pages/product/[id]/mod.rs")).unwrap();
    assert!(backend_content.contains("pub struct Params"));
    assert!(backend_content.contains("pub id: String"));
}

#[test]
fn test_add_page_fails_outside_project() {
    let temp = tempfile::tempdir().unwrap();

    Command::cargo_bin("forte")
        .unwrap()
        .args(["add", "page", "about"])
        .current_dir(&temp)
        .assert()
        .failure()
        .stderr(predicate::str::contains("Not a Forte project"));
}

#[test]
fn test_add_page_fails_if_exists() {
    let temp = tempfile::tempdir().unwrap();
    let project_dir = setup_project(&temp);

    Command::cargo_bin("forte")
        .unwrap()
        .args(["add", "page", "about"])
        .current_dir(&project_dir)
        .assert()
        .success();

    Command::cargo_bin("forte")
        .unwrap()
        .args(["add", "page", "about"])
        .current_dir(&project_dir)
        .assert()
        .failure()
        .stderr(predicate::str::contains("already exists"));
}

// ============ Action Tests ============

#[test]
fn test_add_action_creates_files() {
    let temp = tempfile::tempdir().unwrap();
    let project_dir = setup_project(&temp);

    Command::cargo_bin("forte")
        .unwrap()
        .args(["add", "action", "user/login"])
        .current_dir(&project_dir)
        .assert()
        .success()
        .stdout(predicate::str::contains("Created action 'user/login'"));

    assert!(project_dir.join("rs/src/actions/user/login.rs").exists());
    assert!(project_dir.join("fe/src/actions/user/login.ts").exists());

    let backend_content = std::fs::read_to_string(project_dir.join("rs/src/actions/user/login.rs")).unwrap();
    assert!(backend_content.contains("pub async fn action"));
    assert!(backend_content.contains("LoginInput"));
    assert!(backend_content.contains("LoginOutput"));

    let frontend_content = std::fs::read_to_string(project_dir.join("fe/src/actions/user/login.ts")).unwrap();
    assert!(frontend_content.contains("export async function login"));
    assert!(frontend_content.contains("/_action/user/login"));
}

#[test]
fn test_add_action_simple_path() {
    let temp = tempfile::tempdir().unwrap();
    let project_dir = setup_project(&temp);

    Command::cargo_bin("forte")
        .unwrap()
        .args(["add", "action", "subscribe"])
        .current_dir(&project_dir)
        .assert()
        .success();

    assert!(project_dir.join("rs/src/actions/subscribe.rs").exists());
    assert!(project_dir.join("fe/src/actions/subscribe.ts").exists());

    let backend_content = std::fs::read_to_string(project_dir.join("rs/src/actions/subscribe.rs")).unwrap();
    assert!(backend_content.contains("SubscribeInput"));
    assert!(backend_content.contains("SubscribeOutput"));
}

#[test]
fn test_add_action_fails_outside_project() {
    let temp = tempfile::tempdir().unwrap();

    Command::cargo_bin("forte")
        .unwrap()
        .args(["add", "action", "test"])
        .current_dir(&temp)
        .assert()
        .failure()
        .stderr(predicate::str::contains("Not a Forte project"));
}

#[test]
fn test_add_action_fails_if_exists() {
    let temp = tempfile::tempdir().unwrap();
    let project_dir = setup_project(&temp);

    Command::cargo_bin("forte")
        .unwrap()
        .args(["add", "action", "test"])
        .current_dir(&project_dir)
        .assert()
        .success();

    Command::cargo_bin("forte")
        .unwrap()
        .args(["add", "action", "test"])
        .current_dir(&project_dir)
        .assert()
        .failure()
        .stderr(predicate::str::contains("already exists"));
}
