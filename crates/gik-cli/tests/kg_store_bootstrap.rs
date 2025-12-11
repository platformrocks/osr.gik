//! Integration tests for the Knowledge Graph (KG) store bootstrap.
//!
//! These tests validate the KG storage layer:
//! - Directory creation and lazy initialization
//! - Round-trip serialization/deserialization via LanceDB backend
//! - Branch isolation
//!
//! # Phase 9.1-9.2 (Original)
//!
//! Original tests validated JSONL file format. After Migration 4, the storage
//! is backed by gik-db's LanceDB tables. Tests now focus on API behavior.

use std::fs;
use std::path::PathBuf;
use tempfile::TempDir;

use gik_core::kg::{
    append_edges, append_nodes, compute_stats, init_kg_for_branch, kg_exists, read_all_edges,
    read_all_nodes, read_stats, KgEdge, KgNode, KgStats, KG_DIR_NAME, KG_VERSION,
};
use gik_core::workspace::Workspace;

// ============================================================================
// Test Helper: Workspace Setup
// ============================================================================

/// Get the KG directory path for a workspace and branch.
fn kg_dir_for_branch(workspace: &Workspace, branch: &str) -> PathBuf {
    workspace.knowledge_root().join(branch).join(KG_DIR_NAME)
}

/// Create a temporary workspace for testing.
fn create_test_workspace() -> (TempDir, Workspace) {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");

    // Create the .guided/knowledge directory structure (simulating init_workspace)
    let knowledge_root = temp_dir.path().join(".guided/knowledge");
    fs::create_dir_all(&knowledge_root).expect("Failed to create knowledge root");

    // Create branch directory
    fs::create_dir_all(knowledge_root.join("main")).expect("Failed to create main branch");

    let workspace = Workspace::from_root(temp_dir.path()).expect("Failed to create workspace");

    (temp_dir, workspace)
}

// ============================================================================
// Basic Storage Tests
// ============================================================================

#[test]
fn test_kg_directory_not_created_on_init() {
    let (_temp_dir, workspace) = create_test_workspace();

    // KG directory should NOT exist initially
    assert!(!kg_exists(&workspace, "main"));

    let kg_dir = kg_dir_for_branch(&workspace, "main");
    assert!(!kg_dir.exists());
}

#[test]
fn test_kg_directory_created_on_first_write() {
    let (_temp_dir, workspace) = create_test_workspace();

    // KG directory doesn't exist yet
    assert!(!kg_exists(&workspace, "main"));

    // Write a node - this should create the directory
    let nodes = vec![KgNode::new("file:src/main.rs", "file", "src/main.rs")];
    append_nodes(&workspace, "main", &nodes).unwrap();

    // Now KG directory should exist
    assert!(kg_exists(&workspace, "main"));
    // With LanceDB backend, the directory is created but contains LanceDB data
    assert!(kg_dir_for_branch(&workspace, "main").exists());
}

#[test]
fn test_explicit_init_creates_directory() {
    let (_temp_dir, workspace) = create_test_workspace();

    assert!(!kg_exists(&workspace, "main"));

    let kg_dir = init_kg_for_branch(&workspace, "main").unwrap();

    assert!(kg_exists(&workspace, "main"));
    assert!(kg_dir.exists());
    assert!(kg_dir.is_dir());
}

// ============================================================================
// Node Operations Tests
// ============================================================================

#[test]
fn test_append_and_read_nodes_roundtrip() {
    let (_temp_dir, workspace) = create_test_workspace();

    // Create nodes with various properties
    let nodes = vec![
        KgNode::new("file:src/main.rs", "file", "src/main.rs")
            .with_props(serde_json::json!({"language": "rust", "lines": 150}))
            .with_branch("main"),
        KgNode::new("fn:main", "function", "main()")
            .with_props(serde_json::json!({"visibility": "public"})),
        KgNode::new("mod:utils", "module", "utils").with_props(serde_json::json!({})),
    ];

    // Append nodes
    let count = append_nodes(&workspace, "main", &nodes).unwrap();
    assert_eq!(count, 3);

    // Read nodes back
    let read_nodes = read_all_nodes(&workspace, "main").unwrap();
    assert_eq!(read_nodes.len(), 3);

    // Verify first node
    assert_eq!(read_nodes[0].id, "file:src/main.rs");
    assert_eq!(read_nodes[0].kind, "file");
    assert_eq!(read_nodes[0].label, "src/main.rs");
    assert_eq!(read_nodes[0].props["language"], "rust");
    assert_eq!(read_nodes[0].props["lines"], 150);
    assert_eq!(read_nodes[0].branch, Some("main".to_string()));

    // Verify second node
    assert_eq!(read_nodes[1].id, "fn:main");
    assert_eq!(read_nodes[1].kind, "function");

    // Verify third node
    assert_eq!(read_nodes[2].id, "mod:utils");
    assert_eq!(read_nodes[2].kind, "module");
}

#[test]
fn test_append_nodes_in_multiple_batches() {
    let (_temp_dir, workspace) = create_test_workspace();

    // First batch
    let batch1 = vec![
        KgNode::new("file:a.rs", "file", "a.rs"),
        KgNode::new("file:b.rs", "file", "b.rs"),
    ];
    append_nodes(&workspace, "main", &batch1).unwrap();

    // Second batch
    let batch2 = vec![KgNode::new("file:c.rs", "file", "c.rs")];
    append_nodes(&workspace, "main", &batch2).unwrap();

    // All nodes should be present
    let all_nodes = read_all_nodes(&workspace, "main").unwrap();
    assert_eq!(all_nodes.len(), 3);
    assert_eq!(all_nodes[0].id, "file:a.rs");
    assert_eq!(all_nodes[1].id, "file:b.rs");
    assert_eq!(all_nodes[2].id, "file:c.rs");
}

#[test]
fn test_read_nodes_empty_returns_empty_vec() {
    let (_temp_dir, workspace) = create_test_workspace();

    // Reading from non-existent file should return empty vec
    let nodes = read_all_nodes(&workspace, "main").unwrap();
    assert!(nodes.is_empty());
}

// ============================================================================
// Edge Operations Tests
// ============================================================================

#[test]
fn test_append_and_read_edges_roundtrip() {
    let (_temp_dir, workspace) = create_test_workspace();

    // Create edges
    let edges = vec![
        KgEdge::new("file:main.rs", "file:lib.rs", "imports")
            .with_props(serde_json::json!({"count": 3})),
        KgEdge::new("fn:main", "fn:helper", "calls").with_props(serde_json::json!({"weight": 1.5})),
        KgEdge::with_id("my-edge-001", "mod:a", "mod:b", "dependsOn"),
    ];

    // Append edges
    let count = append_edges(&workspace, "main", &edges).unwrap();
    assert_eq!(count, 3);

    // Read edges back
    let read_edges = read_all_edges(&workspace, "main").unwrap();
    assert_eq!(read_edges.len(), 3);

    // Verify first edge
    assert!(read_edges[0].id.starts_with("edge:"));
    assert!(read_edges[0].id.contains("imports"));
    assert_eq!(read_edges[0].from, "file:main.rs");
    assert_eq!(read_edges[0].to, "file:lib.rs");
    assert_eq!(read_edges[0].kind, "imports");
    assert_eq!(read_edges[0].props["count"], 3);

    // Verify second edge
    assert_eq!(read_edges[1].from, "fn:main");
    assert_eq!(read_edges[1].to, "fn:helper");
    assert_eq!(read_edges[1].kind, "calls");

    // Verify third edge with explicit ID
    assert_eq!(read_edges[2].id, "my-edge-001");
    assert_eq!(read_edges[2].from, "mod:a");
    assert_eq!(read_edges[2].to, "mod:b");
    assert_eq!(read_edges[2].kind, "dependsOn");
}

#[test]
fn test_read_edges_empty_returns_empty_vec() {
    let (_temp_dir, workspace) = create_test_workspace();

    let edges = read_all_edges(&workspace, "main").unwrap();
    assert!(edges.is_empty());
}

// ============================================================================
// Stats Operations Tests
// ============================================================================

#[test]
fn test_write_and_read_stats_roundtrip() {
    let (_temp_dir, workspace) = create_test_workspace();

    // With LanceDB backend, stats are computed from actual data.
    // write_stats is a no-op for API compatibility.
    // To test stats, we add actual nodes and edges.

    // Add 3 nodes
    let nodes = vec![
        KgNode::new("file:a.rs", "file", "a.rs"),
        KgNode::new("file:b.rs", "file", "b.rs"),
        KgNode::new("file:c.rs", "file", "c.rs"),
    ];
    append_nodes(&workspace, "main", &nodes).unwrap();

    // Add 2 edges
    let edges = vec![
        KgEdge::new("file:a.rs", "file:b.rs", "imports"),
        KgEdge::new("file:b.rs", "file:c.rs", "imports"),
    ];
    append_edges(&workspace, "main", &edges).unwrap();

    // Read stats - should reflect actual data
    let read_stats = read_stats(&workspace, "main").unwrap();
    assert_eq!(read_stats.node_count, 3);
    assert_eq!(read_stats.edge_count, 2);
    assert_eq!(read_stats.version, KG_VERSION);
}

#[test]
fn test_read_stats_default_when_missing() {
    let (_temp_dir, workspace) = create_test_workspace();

    // Reading from non-existent file should return default stats
    let stats = read_stats(&workspace, "main").unwrap();
    assert_eq!(stats.node_count, 0);
    assert_eq!(stats.edge_count, 0);
    assert_eq!(stats.version, KG_VERSION);
}

#[test]
fn test_compute_stats_from_files() {
    let (_temp_dir, workspace) = create_test_workspace();

    // Add some nodes and edges
    let nodes = vec![
        KgNode::new("file:a.rs", "file", "a.rs"),
        KgNode::new("file:b.rs", "file", "b.rs"),
        KgNode::new("file:c.rs", "file", "c.rs"),
    ];
    append_nodes(&workspace, "main", &nodes).unwrap();

    let edges = vec![
        KgEdge::new("file:a.rs", "file:b.rs", "imports"),
        KgEdge::new("file:b.rs", "file:c.rs", "imports"),
    ];
    append_edges(&workspace, "main", &edges).unwrap();

    // Compute stats
    let stats = compute_stats(&workspace, "main").unwrap();
    assert_eq!(stats.node_count, 3);
    assert_eq!(stats.edge_count, 2);
    assert_eq!(stats.version, KG_VERSION);
}

// ============================================================================
// Data Integrity Tests (replaces JSONL format tests after LanceDB migration)
// ============================================================================

#[test]
fn test_nodes_data_integrity() {
    let (_temp_dir, workspace) = create_test_workspace();

    let nodes = vec![
        KgNode::new("file:a.rs", "file", "a.rs"),
        KgNode::new("file:b.rs", "file", "b.rs"),
    ];
    append_nodes(&workspace, "main", &nodes).unwrap();

    // Read back via API and verify data integrity
    let read_nodes = read_all_nodes(&workspace, "main").unwrap();
    assert_eq!(read_nodes.len(), 2);

    // Verify all nodes have required fields
    for node in &read_nodes {
        assert!(!node.id.is_empty(), "Node should have an id");
        assert!(!node.kind.is_empty(), "Node should have a kind");
        assert!(!node.label.is_empty(), "Node should have a label");
        // Timestamps are set by the store
        assert!(
            node.created_at.timestamp() > 0,
            "Node should have createdAt"
        );
        assert!(
            node.updated_at.timestamp() > 0,
            "Node should have updatedAt"
        );
    }

    // Check both expected nodes exist
    let ids: Vec<_> = read_nodes.iter().map(|n| n.id.as_str()).collect();
    assert!(ids.contains(&"file:a.rs"));
    assert!(ids.contains(&"file:b.rs"));
}

#[test]
fn test_edges_data_integrity() {
    let (_temp_dir, workspace) = create_test_workspace();

    let edges = vec![KgEdge::new("file:a.rs", "file:b.rs", "imports")];
    append_edges(&workspace, "main", &edges).unwrap();

    // Read back via API and verify data integrity
    let read_edges = read_all_edges(&workspace, "main").unwrap();
    assert_eq!(read_edges.len(), 1);

    let edge = &read_edges[0];
    assert!(!edge.id.is_empty(), "Edge should have an id");
    assert_eq!(edge.from, "file:a.rs");
    assert_eq!(edge.to, "file:b.rs");
    assert_eq!(edge.kind, "imports");
    assert!(
        edge.created_at.timestamp() > 0,
        "Edge should have createdAt"
    );
    assert!(
        edge.updated_at.timestamp() > 0,
        "Edge should have updatedAt"
    );
}

#[test]
fn test_stats_data_integrity() {
    let (_temp_dir, workspace) = create_test_workspace();

    // Add data so we can verify stats
    let nodes = vec![
        KgNode::new("file:a.rs", "file", "a.rs"),
        KgNode::new("file:b.rs", "file", "b.rs"),
    ];
    append_nodes(&workspace, "main", &nodes).unwrap();

    let edges = vec![KgEdge::new("file:a.rs", "file:b.rs", "imports")];
    append_edges(&workspace, "main", &edges).unwrap();

    // Read stats via API
    let stats = read_stats(&workspace, "main").unwrap();
    assert_eq!(stats.node_count, 2);
    assert_eq!(stats.edge_count, 1);
    assert_eq!(stats.version, KG_VERSION);
    // last_updated should be set
    assert!(
        stats.last_updated.timestamp() > 0,
        "Stats should have last_updated"
    );
}

// ============================================================================
// Branch Isolation Tests
// ============================================================================

#[test]
fn test_different_branches_are_isolated() {
    let (temp_dir, workspace) = create_test_workspace();

    // Create another branch directory
    fs::create_dir_all(temp_dir.path().join(".guided/knowledge/feature-x"))
        .expect("create feature branch");

    // Add nodes to main branch
    let main_nodes = vec![KgNode::new("file:main.rs", "file", "main.rs")];
    append_nodes(&workspace, "main", &main_nodes).unwrap();

    // Add nodes to feature branch
    let feature_nodes = vec![
        KgNode::new("file:feature.rs", "file", "feature.rs"),
        KgNode::new("file:feature2.rs", "file", "feature2.rs"),
    ];
    append_nodes(&workspace, "feature-x", &feature_nodes).unwrap();

    // Read from each branch
    let main_read = read_all_nodes(&workspace, "main").unwrap();
    let feature_read = read_all_nodes(&workspace, "feature-x").unwrap();

    // Branches should be isolated
    assert_eq!(main_read.len(), 1);
    assert_eq!(main_read[0].id, "file:main.rs");

    assert_eq!(feature_read.len(), 2);
    assert_eq!(feature_read[0].id, "file:feature.rs");
    assert_eq!(feature_read[1].id, "file:feature2.rs");
}

// ============================================================================
// Timestamp Tests
// ============================================================================

#[test]
fn test_node_timestamps_are_set() {
    let (_temp_dir, workspace) = create_test_workspace();

    let before = chrono::Utc::now();

    let nodes = vec![KgNode::new("file:test.rs", "file", "test.rs")];
    append_nodes(&workspace, "main", &nodes).unwrap();

    let after = chrono::Utc::now();

    let read_nodes = read_all_nodes(&workspace, "main").unwrap();
    let node = &read_nodes[0];

    // Timestamps should be between before and after
    assert!(node.created_at >= before);
    assert!(node.created_at <= after);
    assert!(node.updated_at >= before);
    assert!(node.updated_at <= after);
    assert_eq!(node.created_at, node.updated_at);
}

#[test]
fn test_edge_timestamps_are_set() {
    let (_temp_dir, workspace) = create_test_workspace();

    let before = chrono::Utc::now();

    let edges = vec![KgEdge::new("file:a.rs", "file:b.rs", "imports")];
    append_edges(&workspace, "main", &edges).unwrap();

    let after = chrono::Utc::now();

    let read_edges = read_all_edges(&workspace, "main").unwrap();
    let edge = &read_edges[0];

    assert!(edge.created_at >= before);
    assert!(edge.created_at <= after);
    assert!(edge.updated_at >= before);
    assert!(edge.updated_at <= after);
}

// ============================================================================
// Edge ID Generation Tests
// ============================================================================

#[test]
fn test_edge_id_generation() {
    // Auto-generated ID should follow pattern: edge:<hash>-><kind>
    let edge = KgEdge::new("file:a.rs", "file:b.rs", "imports");
    assert!(edge.id.starts_with("edge:"));
    assert!(edge.id.ends_with("->imports"));

    // Same from/to should generate same hash
    let edge2 = KgEdge::new("file:a.rs", "file:b.rs", "calls");
    // Different kind = different suffix
    assert!(edge2.id.ends_with("->calls"));

    // Explicit ID should be used as-is
    let edge3 = KgEdge::with_id("custom-id", "file:a.rs", "file:b.rs", "imports");
    assert_eq!(edge3.id, "custom-id");
}

// ============================================================================
// KG Version Tests
// ============================================================================

#[test]
fn test_kg_version_constant() {
    assert_eq!(KG_VERSION, "kg-v1");
}

#[test]
fn test_stats_version_is_set() {
    let stats = KgStats::default();
    assert_eq!(stats.version, KG_VERSION);

    let stats2 = KgStats::new(10, 20);
    assert_eq!(stats2.version, KG_VERSION);
}
