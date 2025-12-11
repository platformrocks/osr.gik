//! Integration tests for the GIK CLI.
//!
//! These tests exercise the complete CLI flow from init to ask/reindex/log.
//! They use real embeddings (Candle backend) to validate actual behavior.
//!
//! # Test Strategy
//!
//! - Each test creates a fresh temporary workspace
//! - Commands are run via `assert_cmd` against the actual `gik` binary
//! - Tests validate both exit codes and filesystem artifacts
//! - Real Candle embeddings are used (tests may take ~2-3s each)

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

/// Create a minimal README for testing docs base.
fn create_test_readme(dir: &std::path::Path, content: &str) {
    fs::write(dir.join("README.md"), content).expect("write README");
}

// ============================================================================
// Integration Tests
// ============================================================================

#[test]
fn test_init_creates_workspace_structure() {
    let temp = TempDir::new().expect("create temp dir");
    let workspace = temp.path();

    // Run gik init
    gik_cmd()
        .current_dir(workspace)
        .arg("init")
        .assert()
        .success()
        .stdout(predicate::str::contains("Initialized GIK workspace"));

    // Verify .guided directory structure
    assert!(workspace.join(".guided").exists(), ".guided should exist");
    assert!(
        workspace.join(".guided/knowledge").exists(),
        ".guided/knowledge should exist"
    );
    assert!(
        workspace.join(".guided/knowledge/main").exists(),
        "main branch dir should exist"
    );
    assert!(
        workspace
            .join(".guided/knowledge/main/timeline.jsonl")
            .exists(),
        "timeline.jsonl should exist"
    );
    assert!(
        workspace.join(".guided/knowledge/main/HEAD").exists(),
        "HEAD should exist"
    );
}

#[test]
fn test_init_is_idempotent() {
    let temp = TempDir::new().expect("create temp dir");
    let workspace = temp.path();

    // First init
    gik_cmd()
        .current_dir(workspace)
        .arg("init")
        .assert()
        .success();

    // Second init should also succeed (idempotent)
    gik_cmd()
        .current_dir(workspace)
        .arg("init")
        .assert()
        .success()
        .stdout(predicate::str::contains("already initialized"));
}

#[test]
fn test_status_shows_workspace_info() {
    let temp = TempDir::new().expect("create temp dir");
    let workspace = temp.path();

    // Init first
    gik_cmd()
        .current_dir(workspace)
        .arg("init")
        .assert()
        .success();

    // Check status
    gik_cmd()
        .current_dir(workspace)
        .arg("status")
        .assert()
        .success()
        .stdout(predicate::str::contains("STATUS"))
        .stdout(predicate::str::contains("Branch: main"))
        .stdout(predicate::str::contains("initialized: yes"));
}

#[test]
fn test_add_stages_files() {
    let temp = TempDir::new().expect("create temp dir");
    let workspace = temp.path();

    // Init
    gik_cmd()
        .current_dir(workspace)
        .arg("init")
        .assert()
        .success();

    // Create a test file
    create_test_source(workspace, "lib.rs", "/// A test module\npub fn hello() {}");

    // Add the file
    gik_cmd()
        .current_dir(workspace)
        .arg("add")
        .arg("src/")
        .assert()
        .success()
        .stdout(predicate::str::contains("Staged"));

    // Verify staging file exists
    assert!(
        workspace
            .join(".guided/knowledge/main/staging/pending.jsonl")
            .exists(),
        "pending.jsonl should exist after add"
    );

    // Status should show pending
    gik_cmd()
        .current_dir(workspace)
        .arg("status")
        .assert()
        .success()
        .stdout(predicate::str::contains("pending=1"));
}

#[test]
fn test_full_flow_init_add_commit_log() {
    let temp = TempDir::new().expect("create temp dir");
    let workspace = temp.path();

    // === INIT ===
    gik_cmd()
        .current_dir(workspace)
        .arg("init")
        .assert()
        .success();

    // === CREATE FILES ===
    create_test_source(
        workspace,
        "main.rs",
        r#"//! Main application entry point.
//! This module handles CLI argument parsing and application bootstrap.

fn main() {
    println!("Hello, GIK!");
}
"#,
    );

    create_test_readme(
        workspace,
        r#"# Test Project

A sample project for GIK integration testing.

## Features

- Feature A
- Feature B
"#,
    );

    // === ADD ===
    gik_cmd()
        .current_dir(workspace)
        .arg("add")
        .arg("src/")
        .assert()
        .success();

    gik_cmd()
        .current_dir(workspace)
        .arg("add")
        .arg("README.md")
        .assert()
        .success();

    // === COMMIT ===
    // Note: This uses real Candle embeddings
    gik_cmd()
        .current_dir(workspace)
        .arg("commit")
        .arg("-m")
        .arg("feat: initial project setup")
        .assert()
        .success()
        .stdout(predicate::str::contains("[ok]"));

    // Verify bases were created
    assert!(
        workspace.join(".guided/knowledge/main/bases/code").exists(),
        "code base should exist"
    );
    assert!(
        workspace.join(".guided/knowledge/main/bases/docs").exists(),
        "docs base should exist"
    );

    // Verify model-info exists
    assert!(
        workspace
            .join(".guided/knowledge/main/bases/code/model-info.json")
            .exists(),
        "model-info.json should exist for code base"
    );

    // === LOG ===
    gik_cmd()
        .current_dir(workspace)
        .arg("log")
        .assert()
        .success()
        .stdout(predicate::str::contains("TIMELINE"))
        .stdout(predicate::str::contains("commit"))
        .stdout(predicate::str::contains("init"));

    // === LOG --json ===
    let output = gik_cmd()
        .current_dir(workspace)
        .arg("log")
        .arg("--json")
        .assert()
        .success();

    let stdout = String::from_utf8_lossy(&output.get_output().stdout);
    let entries: Vec<serde_json::Value> =
        serde_json::from_str(&stdout).expect("log --json should return valid JSON array");

    // Should have at least 2 entries: Init and Commit
    assert!(
        entries.len() >= 2,
        "Should have at least Init and Commit entries"
    );

    // Check that we have both init and commit operations
    let operations: Vec<&str> = entries
        .iter()
        .filter_map(|e| e.get("operation").and_then(|o| o.as_str()))
        .collect();
    assert!(operations.contains(&"init"), "Should have init operation");
    assert!(
        operations.contains(&"commit"),
        "Should have commit operation"
    );
}

#[test]
fn test_ask_returns_context() {
    let temp = TempDir::new().expect("create temp dir");
    let workspace = temp.path();

    // Setup workspace with content
    gik_cmd()
        .current_dir(workspace)
        .arg("init")
        .assert()
        .success();

    create_test_source(
        workspace,
        "calculator.rs",
        r#"//! Calculator module for basic arithmetic operations.

/// Add two numbers together.
pub fn add(a: i32, b: i32) -> i32 {
    a + b
}

/// Subtract b from a.
pub fn subtract(a: i32, b: i32) -> i32 {
    a - b
}

/// Multiply two numbers.
pub fn multiply(a: i32, b: i32) -> i32 {
    a * b
}
"#,
    );

    gik_cmd()
        .current_dir(workspace)
        .arg("add")
        .arg("src/")
        .assert()
        .success();

    gik_cmd()
        .current_dir(workspace)
        .arg("commit")
        .arg("-m")
        .arg("feat: add calculator module")
        .assert()
        .success();

    // === ASK ===
    // Note: Small test files may not produce strong matches, so we just check the command runs
    gik_cmd()
        .current_dir(workspace)
        .arg("ask")
        .arg("What arithmetic operations are available?")
        .assert()
        .success()
        .stdout(predicate::str::contains("Query:"));

    // === ASK --json ===
    let output = gik_cmd()
        .current_dir(workspace)
        .arg("ask")
        .arg("--json")
        .arg("What does this project do?")
        .assert()
        .success();

    let stdout = String::from_utf8_lossy(&output.get_output().stdout);
    let bundle: serde_json::Value =
        serde_json::from_str(&stdout).expect("ask --json should return valid JSON");

    // Verify bundle structure
    assert!(
        bundle.get("question").is_some(),
        "Bundle should have question"
    );
    assert!(
        bundle.get("ragChunks").is_some(),
        "Bundle should have ragChunks"
    );
    assert!(
        bundle.get("revisionId").is_some(),
        "Bundle should have revisionId"
    );

    // Verify ask log was created (branch-agnostic path)
    assert!(
        workspace
            .join(".guided/knowledge/asks/ask.log.jsonl")
            .exists(),
        "ask.log.jsonl should exist after ask"
    );
}

#[test]
fn test_reindex_rebuilds_index() {
    let temp = TempDir::new().expect("create temp dir");
    let workspace = temp.path();

    // Setup workspace with content
    gik_cmd()
        .current_dir(workspace)
        .arg("init")
        .assert()
        .success();

    create_test_source(
        workspace,
        "lib.rs",
        "/// A simple library.\npub fn greet() -> &'static str { \"Hello\" }",
    );

    gik_cmd()
        .current_dir(workspace)
        .arg("add")
        .arg("src/")
        .assert()
        .success();

    gik_cmd()
        .current_dir(workspace)
        .arg("commit")
        .arg("-m")
        .arg("chore: initial setup")
        .assert()
        .success();

    // === REINDEX ===
    // Since model hasn't changed, it may say "No reindex needed" or "Reindexed"
    gik_cmd()
        .current_dir(workspace)
        .arg("reindex")
        .arg("--base")
        .arg("code")
        .assert()
        .success();
    // Output depends on whether model changed

    // === REINDEX --dry-run ===
    gik_cmd()
        .current_dir(workspace)
        .arg("reindex")
        .arg("--base")
        .arg("code")
        .arg("--dry-run")
        .assert()
        .success();
    // dry-run output varies based on state

    // Check log - reindex may or may not be recorded depending on whether it was needed
    gik_cmd()
        .current_dir(workspace)
        .arg("log")
        .assert()
        .success()
        .stdout(predicate::str::contains("TIMELINE"));
}

#[test]
fn test_bases_command() {
    let temp = TempDir::new().expect("create temp dir");
    let workspace = temp.path();

    gik_cmd()
        .current_dir(workspace)
        .arg("init")
        .assert()
        .success();

    // Before any commits, bases command should work
    gik_cmd()
        .current_dir(workspace)
        .arg("bases")
        .assert()
        .success();

    // Add and commit to create bases
    create_test_source(workspace, "lib.rs", "pub fn test() {}");
    gik_cmd()
        .current_dir(workspace)
        .arg("add")
        .arg("src/")
        .assert()
        .success();

    gik_cmd()
        .current_dir(workspace)
        .arg("commit")
        .arg("-m")
        .arg("test")
        .assert()
        .success();

    // Now bases should show code
    gik_cmd()
        .current_dir(workspace)
        .arg("bases")
        .assert()
        .success()
        .stdout(predicate::str::contains("code"));
}

#[test]
fn test_stats_command() {
    let temp = TempDir::new().expect("create temp dir");
    let workspace = temp.path();

    gik_cmd()
        .current_dir(workspace)
        .arg("init")
        .assert()
        .success();

    create_test_source(workspace, "lib.rs", "pub fn test() {}");
    gik_cmd()
        .current_dir(workspace)
        .arg("add")
        .arg("src/")
        .assert()
        .success();

    gik_cmd()
        .current_dir(workspace)
        .arg("commit")
        .arg("-m")
        .arg("test")
        .assert()
        .success();

    // Stats command - may show "no data" if stats not fully implemented
    gik_cmd()
        .current_dir(workspace)
        .arg("stats")
        .assert()
        .success()
        .stdout(predicate::str::contains("STATS"));

    // Stats --json
    let output = gik_cmd()
        .current_dir(workspace)
        .arg("stats")
        .arg("--json")
        .assert()
        .success();

    let stdout = String::from_utf8_lossy(&output.get_output().stdout);
    // JSON output should be valid, even if empty
    let _stats: serde_json::Value =
        serde_json::from_str(&stdout).expect("stats --json should return valid JSON");
}

// ============================================================================
// Phase 8.6: Empty File Handling Tests
// ============================================================================

/// phase8.6-add-001: Empty files are skipped during add.
///
/// Files with 0 bytes should not be staged because they would cause
/// embedding failures during commit.
#[test]
fn test_empty_files_skipped_in_add() {
    let temp = TempDir::new().expect("create temp dir");
    let workspace = temp.path();

    // Init
    gik_cmd()
        .current_dir(workspace)
        .arg("init")
        .assert()
        .success();

    // Create an empty file
    let src_dir = workspace.join("src");
    fs::create_dir_all(&src_dir).expect("create src dir");
    fs::write(src_dir.join("empty.rs"), "").expect("write empty file");

    // Create a non-empty file
    fs::write(src_dir.join("valid.rs"), "fn main() {}").expect("write valid file");

    // Add directory - should skip the empty file but add the valid one
    // Output format: "[ok] Staged N source(s) for indexing"
    gik_cmd()
        .current_dir(workspace)
        .arg("add")
        .arg("src/")
        .assert()
        .success()
        .stdout(predicate::str::contains("Staged 1 source(s)"));
}

/// phase8.6-add-002: Single empty file shows skip message.
///
/// When adding a single empty file directly (not via directory), the
/// skip should be visible in the output.
#[test]
fn test_single_empty_file_shows_skip_reason() {
    let temp = TempDir::new().expect("create temp dir");
    let workspace = temp.path();

    // Init
    gik_cmd()
        .current_dir(workspace)
        .arg("init")
        .assert()
        .success();

    // Create an empty file
    let src_dir = workspace.join("src");
    fs::create_dir_all(&src_dir).expect("create src dir");
    fs::write(src_dir.join("empty.rs"), "").expect("write empty file");

    // Add the single empty file directly
    // Command exits with code 1 when all files are skipped (warning)
    // Stdout should contain skip info with "empty" reason
    gik_cmd()
        .current_dir(workspace)
        .arg("add")
        .arg("src/empty.rs")
        .assert()
        .code(1)
        .stdout(predicate::str::contains("empty file (0 bytes)"));
}
