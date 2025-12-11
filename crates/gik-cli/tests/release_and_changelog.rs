//! Integration tests for `gik release` and CHANGELOG.md generation.
//!
//! These tests verify that:
//! - `gik release` generates CHANGELOG.md from commit history
//! - Conventional Commits are properly parsed and grouped
//! - `--dry-run` previews without writing
//! - `--json` returns structured output
//! - Running release multiple times is idempotent

mod common;

use predicates::prelude::*;
use std::fs;
use tempfile::TempDir;

use common::gik_cmd;

/// Create a minimal source file for testing.
fn create_source(dir: &std::path::Path, filename: &str, content: &str) {
    let src_dir = dir.join("src");
    fs::create_dir_all(&src_dir).expect("create src dir");
    fs::write(src_dir.join(filename), content).expect("write source file");
}

/// Setup a workspace with init and multiple conventional commits.
fn setup_workspace_with_commits(workspace: &std::path::Path) {
    // Init
    gik_cmd()
        .current_dir(workspace)
        .arg("init")
        .assert()
        .success();

    // First commit: feat
    create_source(
        workspace,
        "feature.rs",
        "/// Feature module\npub fn feature_one() {}",
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
        .arg("feat: add example module")
        .assert()
        .success();

    // Second commit: fix
    create_source(
        workspace,
        "fix.rs",
        "/// Fix module\npub fn handle_error() {}",
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
        .arg("fix: handle missing config")
        .assert()
        .success();
}

// ============================================================================
// Release Tests
// ============================================================================

#[test]
fn test_release_generates_changelog() {
    let temp = TempDir::new().expect("create temp dir");
    let workspace = temp.path();

    setup_workspace_with_commits(workspace);

    // Run release
    gik_cmd()
        .current_dir(workspace)
        .arg("release")
        .assert()
        .success()
        .stdout(predicate::str::contains("RELEASE"))
        .stdout(predicate::str::contains("entries processed"));

    // Verify CHANGELOG.md exists
    let changelog_path = workspace.join("CHANGELOG.md");
    assert!(changelog_path.exists(), "CHANGELOG.md should be created");

    // Verify content includes feat and fix entries
    let content = fs::read_to_string(&changelog_path).expect("read CHANGELOG.md");
    assert!(
        content.contains("# Changelog"),
        "Should have Changelog header"
    );
    assert!(
        content.contains("add example module"),
        "Should contain feat commit message"
    );
    assert!(
        content.contains("handle missing config"),
        "Should contain fix commit message"
    );
}

#[test]
fn test_release_with_tag() {
    let temp = TempDir::new().expect("create temp dir");
    let workspace = temp.path();

    setup_workspace_with_commits(workspace);

    // Run release with tag
    gik_cmd()
        .current_dir(workspace)
        .arg("release")
        .arg("--tag")
        .arg("v1.0.0")
        .assert()
        .success()
        .stdout(predicate::str::contains("Tag: v1.0.0"));

    // Verify CHANGELOG.md contains the tag
    let content = fs::read_to_string(workspace.join("CHANGELOG.md")).expect("read CHANGELOG.md");
    assert!(content.contains("## v1.0.0"), "Should have v1.0.0 heading");
}

#[test]
fn test_release_dry_run() {
    let temp = TempDir::new().expect("create temp dir");
    let workspace = temp.path();

    setup_workspace_with_commits(workspace);

    // Run release with --dry-run
    gik_cmd()
        .current_dir(workspace)
        .arg("release")
        .arg("--dry-run")
        .assert()
        .success()
        .stdout(predicate::str::contains("dry-run"))
        .stdout(predicate::str::contains("entries processed"));

    // CHANGELOG.md should NOT be created
    assert!(
        !workspace.join("CHANGELOG.md").exists(),
        "CHANGELOG.md should NOT be created in dry-run mode"
    );
}

#[test]
fn test_release_json_output() {
    let temp = TempDir::new().expect("create temp dir");
    let workspace = temp.path();

    setup_workspace_with_commits(workspace);

    // Run release with --json
    let output = gik_cmd()
        .current_dir(workspace)
        .arg("release")
        .arg("--json")
        .assert()
        .success();

    let stdout = String::from_utf8_lossy(&output.get_output().stdout);
    let result: serde_json::Value =
        serde_json::from_str(&stdout).expect("release --json should return valid JSON");

    // Verify JSON structure
    assert!(result.get("tag").is_some(), "Should have tag field");
    assert!(
        result.get("changelogPath").is_some(),
        "Should have changelogPath field"
    );
    assert!(result.get("summary").is_some(), "Should have summary field");

    let summary = result.get("summary").unwrap();
    assert!(
        summary.get("totalEntries").is_some(),
        "Summary should have totalEntries"
    );
    assert!(
        summary.get("groups").is_some(),
        "Summary should have groups"
    );
}

#[test]
fn test_release_is_idempotent() {
    let temp = TempDir::new().expect("create temp dir");
    let workspace = temp.path();

    setup_workspace_with_commits(workspace);

    // First release
    gik_cmd()
        .current_dir(workspace)
        .arg("release")
        .arg("--tag")
        .arg("v1.0.0")
        .assert()
        .success();

    let content1 = fs::read_to_string(workspace.join("CHANGELOG.md")).expect("read CHANGELOG.md");

    // Second release (should overwrite cleanly)
    gik_cmd()
        .current_dir(workspace)
        .arg("release")
        .arg("--tag")
        .arg("v1.0.0")
        .assert()
        .success();

    let content2 = fs::read_to_string(workspace.join("CHANGELOG.md")).expect("read CHANGELOG.md");

    // Content should be identical (idempotent)
    assert_eq!(
        content1, content2,
        "Running release twice should produce identical output"
    );
}

#[test]
fn test_release_groups_by_conventional_commit_type() {
    let temp = TempDir::new().expect("create temp dir");
    let workspace = temp.path();

    // Init
    gik_cmd()
        .current_dir(workspace)
        .arg("init")
        .assert()
        .success();

    // Multiple commits with different types
    let commits = [
        ("feat.rs", "feat: add new feature"),
        ("fix.rs", "fix: resolve bug"),
        ("docs.rs", "docs: update documentation"),
        ("chore.rs", "chore: cleanup code"),
    ];

    for (filename, message) in commits {
        create_source(
            workspace,
            filename,
            &format!("// {}\npub fn f() {{}}", message),
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
            .arg(message)
            .assert()
            .success();
    }

    // Run release
    gik_cmd()
        .current_dir(workspace)
        .arg("release")
        .assert()
        .success();

    // Verify CHANGELOG.md has grouped sections
    let content = fs::read_to_string(workspace.join("CHANGELOG.md")).expect("read CHANGELOG.md");

    assert!(
        content.contains("### Features"),
        "Should have Features section"
    );
    assert!(
        content.contains("### Bug Fixes"),
        "Should have Bug Fixes section"
    );
    assert!(
        content.contains("### Documentation"),
        "Should have Documentation section"
    );
    assert!(content.contains("### Chores"), "Should have Chores section");
}

#[test]
fn test_release_handles_breaking_changes() {
    let temp = TempDir::new().expect("create temp dir");
    let workspace = temp.path();

    gik_cmd()
        .current_dir(workspace)
        .arg("init")
        .assert()
        .success();

    // Commit with breaking change marker
    create_source(workspace, "breaking.rs", "pub fn breaking() {}");
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
        .arg("feat!: breaking API change")
        .assert()
        .success();

    // Run release
    gik_cmd()
        .current_dir(workspace)
        .arg("release")
        .assert()
        .success();

    // Verify CHANGELOG.md marks breaking change
    let content = fs::read_to_string(workspace.join("CHANGELOG.md")).expect("read CHANGELOG.md");
    assert!(content.contains("BREAKING"), "Should mark breaking changes");
}

#[test]
fn test_release_handles_scoped_commits() {
    let temp = TempDir::new().expect("create temp dir");
    let workspace = temp.path();

    gik_cmd()
        .current_dir(workspace)
        .arg("init")
        .assert()
        .success();

    // Commit with scope
    create_source(workspace, "cli.rs", "pub fn cli_feature() {}");
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
        .arg("feat(cli): add new command")
        .assert()
        .success();

    // Run release
    gik_cmd()
        .current_dir(workspace)
        .arg("release")
        .assert()
        .success();

    // Verify CHANGELOG.md includes scope
    let content = fs::read_to_string(workspace.join("CHANGELOG.md")).expect("read CHANGELOG.md");
    assert!(content.contains("**cli:**"), "Should include scope in bold");
}

#[test]
fn test_release_empty_workspace() {
    let temp = TempDir::new().expect("create temp dir");
    let workspace = temp.path();

    // Just init, no commits
    gik_cmd()
        .current_dir(workspace)
        .arg("init")
        .assert()
        .success();

    // Release should still work (with 0 entries)
    gik_cmd()
        .current_dir(workspace)
        .arg("release")
        .assert()
        .success()
        .stdout(predicate::str::contains("No commit entries found"));
}
