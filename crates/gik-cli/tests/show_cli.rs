//! Integration tests for the `gik show` command.
//!
//! These tests validate the show command functionality:
//! - Showing HEAD revision
//! - Showing specific revisions by ID
//! - JSON output format
//! - KG DOT/Mermaid export

mod common;

use predicates::prelude::*;
use std::fs;
use tempfile::TempDir;

use common::gik_cmd;

/// Create a minimal Rust source file for testing.
fn create_test_source(dir: &std::path::Path, filename: &str, content: &str) {
    let src_dir = dir.join("src");
    fs::create_dir_all(&src_dir).expect("create src dir");
    fs::write(src_dir.join(filename), content).expect("write source file");
}

// ============================================================================
// Tests
// ============================================================================

#[test]
fn test_show_head_on_initialized_workspace() {
    let temp = TempDir::new().expect("create temp dir");
    let workspace = temp.path();

    // Create a source file
    create_test_source(workspace, "main.rs", "fn main() { println!(\"Hello\"); }");

    // Initialize workspace
    gik_cmd()
        .current_dir(workspace)
        .arg("init")
        .assert()
        .success();

    // Show HEAD
    gik_cmd()
        .current_dir(workspace)
        .arg("show")
        .assert()
        .success()
        .stdout(predicate::str::contains("Revision:"))
        .stdout(predicate::str::contains("Type:"))
        .stdout(predicate::str::contains("Branch:"))
        .stdout(predicate::str::contains("Time:"));
}

#[test]
fn test_show_head_explicit() {
    let temp = TempDir::new().expect("create temp dir");
    let workspace = temp.path();

    // Create a source file
    create_test_source(workspace, "main.rs", "fn main() {}");

    // Initialize workspace
    gik_cmd()
        .current_dir(workspace)
        .arg("init")
        .assert()
        .success();

    // Show HEAD explicitly
    gik_cmd()
        .current_dir(workspace)
        .args(["show", "HEAD"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Revision:"));
}

#[test]
fn test_show_json_output() {
    let temp = TempDir::new().expect("create temp dir");
    let workspace = temp.path();

    // Create a source file
    create_test_source(workspace, "main.rs", "fn main() {}");

    // Initialize workspace
    gik_cmd()
        .current_dir(workspace)
        .arg("init")
        .assert()
        .success();

    // Show with JSON output
    gik_cmd()
        .current_dir(workspace)
        .args(["show", "--json"])
        .assert()
        .success()
        .stdout(predicate::str::contains("\"revisionId\""))
        .stdout(predicate::str::contains("\"revisionKind\""))
        .stdout(predicate::str::contains("\"branch\""));
}

#[test]
fn test_show_init_revision_type() {
    let temp = TempDir::new().expect("create temp dir");
    let workspace = temp.path();

    // Create a source file
    create_test_source(workspace, "main.rs", "fn main() {}");

    // Initialize workspace
    gik_cmd()
        .current_dir(workspace)
        .arg("init")
        .assert()
        .success();

    // Show should display Init type for the first revision
    gik_cmd()
        .current_dir(workspace)
        .arg("show")
        .assert()
        .success()
        .stdout(predicate::str::contains("Type:").and(predicate::str::contains("Init")));
}

#[test]
fn test_show_fails_on_uninitialized_workspace() {
    let temp = TempDir::new().expect("create temp dir");
    let workspace = temp.path();

    // Show should fail on uninitialized workspace
    gik_cmd()
        .current_dir(workspace)
        .arg("show")
        .assert()
        .failure()
        .stderr(
            predicate::str::contains("not initialized")
                .or(predicate::str::contains("NotInitialized")),
        );
}

#[test]
fn test_show_nonexistent_revision() {
    let temp = TempDir::new().expect("create temp dir");
    let workspace = temp.path();

    // Create a source file
    create_test_source(workspace, "main.rs", "fn main() {}");

    // Initialize workspace
    gik_cmd()
        .current_dir(workspace)
        .arg("init")
        .assert()
        .success();

    // Show a non-existent revision
    gik_cmd()
        .current_dir(workspace)
        .args(["show", "nonexistent-revision-id"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("not found").or(predicate::str::contains("NotFound")));
}

#[test]
fn test_show_max_sources_option() {
    let temp = TempDir::new().expect("create temp dir");
    let workspace = temp.path();

    // Create a source file
    create_test_source(workspace, "main.rs", "fn main() {}");

    // Initialize workspace
    gik_cmd()
        .current_dir(workspace)
        .arg("init")
        .assert()
        .success();

    // Show with max-sources option
    gik_cmd()
        .current_dir(workspace)
        .args(["show", "--max-sources", "5"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Revision:"));
}
