//! Integration tests for memory-metrics and prune-memory CLI commands.
//!
//! These tests validate Phase 7.4 requirements:
//! - `gik memory-metrics` shows entry count, token estimate, and pruning policy
//! - `gik prune-memory` removes entries based on policy
//!
//! # Note
//!
//! Since there's no CLI command for memory ingestion, these tests use
//! `gik-core` library directly for ingestion, then CLI for metrics/pruning.

mod common;

use predicates::prelude::*;
use tempfile::TempDir;

// Import gik-core for direct memory ingestion
use gik_core::{GikEngine, MemoryEntry, MemoryScope, MemorySource};

use common::gik_cmd;

// ============================================================================
// memory-metrics tests
// ============================================================================

/// Test that memory-metrics works on empty workspace.
#[test]
fn test_memory_metrics_empty_workspace() {
    let temp = TempDir::new().expect("create temp dir");
    let workspace = temp.path();

    // Initialize workspace
    gik_cmd()
        .current_dir(workspace)
        .arg("init")
        .assert()
        .success();

    // Run memory-metrics (no memory ingested yet)
    gik_cmd()
        .current_dir(workspace)
        .arg("memory-metrics")
        .assert()
        .success()
        .stdout(predicate::str::contains("MEMORY METRICS"))
        .stdout(predicate::str::contains("Entry count"));
}

/// Test that memory-metrics --json returns valid JSON.
#[test]
fn test_memory_metrics_json_format() {
    let temp = TempDir::new().expect("create temp dir");
    let workspace = temp.path();

    // Initialize workspace
    gik_cmd()
        .current_dir(workspace)
        .arg("init")
        .assert()
        .success();

    // Run memory-metrics --json
    let output = gik_cmd()
        .current_dir(workspace)
        .arg("memory-metrics")
        .arg("--json")
        .assert()
        .success();

    let stdout = String::from_utf8_lossy(&output.get_output().stdout);
    let json: serde_json::Value =
        serde_json::from_str(&stdout).expect("memory-metrics --json should return valid JSON");

    // Verify expected fields
    assert!(json.get("branch").is_some(), "Should have branch field");
    assert!(json.get("metrics").is_some(), "Should have metrics field");

    let metrics = &json["metrics"];
    assert!(
        metrics.get("entryCount").is_some(),
        "Should have entryCount in metrics"
    );
    assert!(
        metrics.get("estimatedTokenCount").is_some(),
        "Should have estimatedTokenCount in metrics"
    );
}

/// Test that memory-metrics shows correct counts after ingestion.
#[test]
fn test_memory_metrics_after_ingestion() {
    let temp = TempDir::new().expect("create temp dir");
    let workspace = temp.path();

    // Initialize workspace
    gik_cmd()
        .current_dir(workspace)
        .arg("init")
        .assert()
        .success();

    // Ingest memory entries via gik-core
    let engine = GikEngine::with_defaults().expect("create engine");
    let ws = engine
        .resolve_workspace(workspace)
        .expect("resolve workspace");

    let memory_entries = vec![
        MemoryEntry::new(
            MemoryScope::Project,
            MemorySource::Decision,
            "We chose Rust for its memory safety and performance guarantees.",
        )
        .with_title("Language choice: Rust"),
        MemoryEntry::new(
            MemoryScope::Project,
            MemorySource::Observation,
            "Build times improved 40% after enabling incremental compilation.",
        )
        .with_title("Build performance observation"),
    ];

    let result = engine
        .ingest_memory(&ws, memory_entries, Some("Add test memories"))
        .expect("ingest memory");

    assert!(
        result.result.ingested_count >= 2,
        "Should have ingested memory entries"
    );

    // Run memory-metrics --json
    let output = gik_cmd()
        .current_dir(workspace)
        .arg("memory-metrics")
        .arg("--json")
        .assert()
        .success();

    let stdout = String::from_utf8_lossy(&output.get_output().stdout);
    let json: serde_json::Value = serde_json::from_str(&stdout).expect("valid JSON");

    let entry_count = json["metrics"]["entryCount"].as_u64().unwrap_or(0);
    assert!(
        entry_count >= 2,
        "Entry count should be at least 2 after ingestion, got {}",
        entry_count
    );

    let token_count = json["metrics"]["estimatedTokenCount"].as_u64().unwrap_or(0);
    assert!(
        token_count > 0,
        "Token count should be > 0 after ingestion, got {}",
        token_count
    );
}

// ============================================================================
// prune-memory tests
// ============================================================================

/// Test that prune-memory works on empty workspace (no-op).
#[test]
fn test_prune_memory_empty_workspace() {
    let temp = TempDir::new().expect("create temp dir");
    let workspace = temp.path();

    // Initialize workspace
    gik_cmd()
        .current_dir(workspace)
        .arg("init")
        .assert()
        .success();

    // Run prune-memory (no memory to prune)
    gik_cmd()
        .current_dir(workspace)
        .arg("prune-memory")
        .arg("--max-entries")
        .arg("10")
        .assert()
        .success()
        .stdout(predicate::str::contains("pruned").or(predicate::str::contains("No entries")));
}

/// Test that prune-memory --json returns valid JSON.
#[test]
fn test_prune_memory_json_format() {
    let temp = TempDir::new().expect("create temp dir");
    let workspace = temp.path();

    // Initialize workspace
    gik_cmd()
        .current_dir(workspace)
        .arg("init")
        .assert()
        .success();

    // Run prune-memory --json
    let output = gik_cmd()
        .current_dir(workspace)
        .arg("prune-memory")
        .arg("--max-entries")
        .arg("10")
        .arg("--json")
        .assert()
        .success();

    let stdout = String::from_utf8_lossy(&output.get_output().stdout);
    let json: serde_json::Value =
        serde_json::from_str(&stdout).expect("prune-memory --json should return valid JSON");

    // Verify expected fields
    assert!(json.get("result").is_some(), "Should have result field");

    let result = &json["result"];
    assert!(
        result.get("prunedCount").is_some(),
        "Should have prunedCount"
    );
    assert!(
        result.get("metricsBefore").is_some(),
        "Should have metricsBefore"
    );
    assert!(
        result.get("metricsAfter").is_some(),
        "Should have metricsAfter"
    );
}

/// Test that prune-memory actually prunes entries when max-entries is exceeded.
#[test]
fn test_prune_memory_removes_excess_entries() {
    let temp = TempDir::new().expect("create temp dir");
    let workspace = temp.path();

    // Initialize workspace
    gik_cmd()
        .current_dir(workspace)
        .arg("init")
        .assert()
        .success();

    // Ingest several memory entries via gik-core
    let engine = GikEngine::with_defaults().expect("create engine");
    let ws = engine
        .resolve_workspace(workspace)
        .expect("resolve workspace");

    let memory_entries: Vec<_> = (1..=5)
        .map(|i| {
            MemoryEntry::new(
                MemoryScope::Project,
                MemorySource::Observation,
                format!(
                    "Observation number {} with some content to make it non-trivial.",
                    i
                ),
            )
            .with_title(format!("Observation {}", i))
        })
        .collect();

    let result = engine
        .ingest_memory(&ws, memory_entries, Some("Add test observations"))
        .expect("ingest memory");

    assert!(
        result.result.ingested_count >= 5,
        "Should have ingested 5 entries"
    );

    // Verify we have 5+ entries before pruning
    let output_before = gik_cmd()
        .current_dir(workspace)
        .arg("memory-metrics")
        .arg("--json")
        .assert()
        .success();

    let stdout_before = String::from_utf8_lossy(&output_before.get_output().stdout);
    let json_before: serde_json::Value = serde_json::from_str(&stdout_before).expect("valid JSON");
    let count_before = json_before["metrics"]["entryCount"].as_u64().unwrap_or(0);
    assert!(
        count_before >= 5,
        "Should have at least 5 entries before pruning"
    );

    // Prune to max 2 entries
    let output_prune = gik_cmd()
        .current_dir(workspace)
        .arg("prune-memory")
        .arg("--max-entries")
        .arg("2")
        .arg("--mode")
        .arg("archive")
        .arg("--json")
        .assert()
        .success();

    let stdout_prune = String::from_utf8_lossy(&output_prune.get_output().stdout);
    let json_prune: serde_json::Value = serde_json::from_str(&stdout_prune).expect("valid JSON");

    let pruned_count = json_prune["result"]["prunedCount"].as_u64().unwrap_or(0);
    let count_after = json_prune["result"]["metricsAfter"]["entryCount"]
        .as_u64()
        .unwrap_or(0);

    // Should have pruned at least 3 entries (5 - 2 = 3)
    assert!(
        pruned_count >= 3,
        "Should have pruned at least 3 entries, got {}",
        pruned_count
    );

    // Should have at most 2 entries remaining
    assert!(
        count_after <= 2,
        "Should have at most 2 entries after pruning, got {}",
        count_after
    );
}

/// Test that prune-memory mode=delete works.
#[test]
fn test_prune_memory_delete_mode() {
    let temp = TempDir::new().expect("create temp dir");
    let workspace = temp.path();

    // Initialize workspace
    gik_cmd()
        .current_dir(workspace)
        .arg("init")
        .assert()
        .success();

    // Ingest memory entries
    let engine = GikEngine::with_defaults().expect("create engine");
    let ws = engine
        .resolve_workspace(workspace)
        .expect("resolve workspace");

    let memory_entries = vec![MemoryEntry::new(
        MemoryScope::Project,
        MemorySource::ManualNote,
        "Note to be deleted.",
    )
    .with_title("Delete me")];

    engine
        .ingest_memory(&ws, memory_entries, Some("test"))
        .expect("ingest");

    // Prune with delete mode
    let output = gik_cmd()
        .current_dir(workspace)
        .arg("prune-memory")
        .arg("--max-entries")
        .arg("0")
        .arg("--mode")
        .arg("delete")
        .arg("--json")
        .assert()
        .success();

    let stdout = String::from_utf8_lossy(&output.get_output().stdout);
    let json: serde_json::Value = serde_json::from_str(&stdout).expect("valid JSON");

    // Mode should be delete
    let mode = json["result"]["mode"].as_str().unwrap_or("");
    assert_eq!(mode, "delete", "Mode should be 'delete'");
}

/// Test that prune-memory validates mode parameter.
#[test]
fn test_prune_memory_invalid_mode() {
    let temp = TempDir::new().expect("create temp dir");
    let workspace = temp.path();

    // Initialize workspace
    gik_cmd()
        .current_dir(workspace)
        .arg("init")
        .assert()
        .success();

    // Run prune-memory with invalid mode
    gik_cmd()
        .current_dir(workspace)
        .arg("prune-memory")
        .arg("--mode")
        .arg("invalid_mode")
        .assert()
        .failure()
        .stderr(predicate::str::contains("Invalid prune mode"));
}

/// Test that prune-memory with --message creates a revision.
#[test]
fn test_prune_memory_creates_revision() {
    let temp = TempDir::new().expect("create temp dir");
    let workspace = temp.path();

    // Initialize workspace
    gik_cmd()
        .current_dir(workspace)
        .arg("init")
        .assert()
        .success();

    // Ingest memory entries
    let engine = GikEngine::with_defaults().expect("create engine");
    let ws = engine
        .resolve_workspace(workspace)
        .expect("resolve workspace");

    let memory_entries = vec![MemoryEntry::new(
        MemoryScope::Project,
        MemorySource::ManualNote,
        "Note to prune.",
    )];

    engine
        .ingest_memory(&ws, memory_entries, Some("test"))
        .expect("ingest");

    // Prune with message
    let output = gik_cmd()
        .current_dir(workspace)
        .arg("prune-memory")
        .arg("--max-entries")
        .arg("0")
        .arg("--message")
        .arg("Prune all for testing")
        .arg("--json")
        .assert()
        .success();

    let stdout = String::from_utf8_lossy(&output.get_output().stdout);
    let json: serde_json::Value = serde_json::from_str(&stdout).expect("valid JSON");

    // If entries were pruned, revisionId should be present
    let pruned_count = json["result"]["prunedCount"].as_u64().unwrap_or(0);
    if pruned_count > 0 {
        assert!(
            json.get("revisionId").is_some(),
            "Should have revisionId when entries are pruned"
        );
    }
}
