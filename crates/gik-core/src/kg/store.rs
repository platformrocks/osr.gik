//! Knowledge Graph store operations.
//!
//! This module provides the storage layer for the Knowledge Graph (KG),
//! backed by gik-db's LanceDB-based KG store.
//!
//! ## Storage Layout
//!
//! KG data is stored per-branch under `.guided/knowledge/<branch>/kg/`.
//! The actual storage format is managed by gik-db (LanceDB tables).
//!
//! ## Lazy Initialization
//!
//! The `kg/` directory is created lazily on first write, not during workspace init.
//! This keeps KG optional for projects that don't use it.
//!
//! ## Migration from JSONL (Phase 4)
//!
//! Prior versions stored KG as JSONL files. The new implementation uses
//! gik-db's LanceDB backend for better query performance and scalability.

use std::fs;
use std::path::{Path, PathBuf};

use crate::db_adapter::DbKgStore;
use crate::errors::GikError;
use crate::workspace::Workspace;

use super::entities::{KgEdge, KgNode, KgStats};

// ============================================================================
// Constants
// ============================================================================

/// Well-known name for the KG directory.
pub const KG_DIR_NAME: &str = "kg";

/// Legacy filename for nodes (JSONL) - kept for migration detection.
pub const NODES_FILENAME: &str = "nodes.jsonl";

/// Legacy filename for edges (JSONL) - kept for migration detection.
pub const EDGES_FILENAME: &str = "edges.jsonl";

/// Legacy filename for stats (JSON) - kept for migration detection.
pub const STATS_FILENAME: &str = "stats.json";

// ============================================================================
// Path Helpers
// ============================================================================

/// Returns the KG directory path for a given workspace and branch.
///
/// Path: `.guided/knowledge/<branch>/kg/`
///
/// Note: The directory may not exist yet (lazy initialization).
pub(crate) fn kg_dir_for_branch(workspace: &Workspace, branch: &str) -> PathBuf {
    workspace.knowledge_root().join(branch).join(KG_DIR_NAME)
}

/// Returns the KG directory path for a given knowledge root and branch.
///
/// This is a lower-level helper that doesn't require a full Workspace.
pub(crate) fn kg_dir_for_knowledge_root(knowledge_root: &Path, branch: &str) -> PathBuf {
    knowledge_root.join(branch).join(KG_DIR_NAME)
}

// ============================================================================
// Initialization
// ============================================================================

/// Ensures the KG directory exists for a given workspace and branch.
///
/// Creates `.guided/knowledge/<branch>/kg/` if it doesn't exist.
/// This is called automatically by write operations, implementing lazy init.
pub fn ensure_kg_dir(workspace: &Workspace, branch: &str) -> Result<PathBuf, GikError> {
    let kg_dir = kg_dir_for_branch(workspace, branch);
    if !kg_dir.exists() {
        fs::create_dir_all(&kg_dir).map_err(|e| GikError::BaseStoreIo {
            path: kg_dir.clone(),
            message: format!("Failed to create KG directory: {}", e),
        })?;
    }
    Ok(kg_dir)
}

/// Ensures the KG directory exists for a given knowledge root and branch.
///
/// Lower-level helper that doesn't require a full Workspace.
pub fn ensure_kg_dir_from_root(knowledge_root: &Path, branch: &str) -> Result<PathBuf, GikError> {
    let kg_dir = kg_dir_for_knowledge_root(knowledge_root, branch);
    if !kg_dir.exists() {
        fs::create_dir_all(&kg_dir).map_err(|e| GikError::BaseStoreIo {
            path: kg_dir.clone(),
            message: format!("Failed to create KG directory: {}", e),
        })?;
    }
    Ok(kg_dir)
}

// ============================================================================
// Store Factory
// ============================================================================

/// Opens a KG store for the given workspace and branch.
///
/// This creates or opens a LanceDB-backed KG store at the standard location:
/// `.guided/knowledge/<branch>/kg/`
///
/// The store is created lazily if it doesn't exist.
///
/// # Arguments
///
/// * `workspace` - The workspace containing the KG
/// * `branch` - The branch name
///
/// # Returns
///
/// A `DbKgStore` instance ready for use.
///
/// # Errors
///
/// Returns an error if the store cannot be created or opened.
pub fn open_kg_store(workspace: &Workspace, branch: &str) -> Result<DbKgStore, GikError> {
    let kg_dir = ensure_kg_dir(workspace, branch)?;

    let config = gik_db::kg::KgStoreConfig::new(&kg_dir).with_branch(branch.to_string());

    DbKgStore::open(&config)
}

/// Opens a KG store for the given knowledge root and branch.
///
/// Lower-level factory that doesn't require a full Workspace.
pub fn open_kg_store_from_root(knowledge_root: &Path, branch: &str) -> Result<DbKgStore, GikError> {
    let kg_dir = ensure_kg_dir_from_root(knowledge_root, branch)?;

    let config = gik_db::kg::KgStoreConfig::new(&kg_dir).with_branch(branch.to_string());

    DbKgStore::open(&config)
}

// ============================================================================
// Convenience Functions (delegate to DbKgStore)
// ============================================================================

/// Appends (upserts) nodes for a given workspace and branch.
///
/// This is a convenience function that opens a store, upserts nodes,
/// and flushes changes.
///
/// # Arguments
///
/// * `workspace` - The workspace containing the KG
/// * `branch` - The branch name
/// * `nodes` - Slice of nodes to upsert
///
/// # Returns
///
/// The number of nodes upserted.
pub fn append_nodes(
    workspace: &Workspace,
    branch: &str,
    nodes: &[KgNode],
) -> Result<usize, GikError> {
    if nodes.is_empty() {
        return Ok(0);
    }

    let store = open_kg_store(workspace, branch)?;
    let count = store.upsert_nodes(nodes)?;
    store.flush()?;
    Ok(count)
}

/// Reads all nodes for a given workspace and branch.
///
/// # Arguments
///
/// * `workspace` - The workspace containing the KG
/// * `branch` - The branch name
///
/// # Returns
///
/// All nodes in the store, or an empty vector if the store is empty.
pub fn read_all_nodes(workspace: &Workspace, branch: &str) -> Result<Vec<KgNode>, GikError> {
    // Check if KG directory exists first (lazy init check)
    let kg_dir = kg_dir_for_branch(workspace, branch);
    if !kg_dir.exists() {
        return Ok(Vec::new());
    }

    let store = open_kg_store(workspace, branch)?;
    store.get_all_nodes()
}

/// Appends (upserts) edges for a given workspace and branch.
///
/// This is a convenience function that opens a store, upserts edges,
/// and flushes changes.
///
/// # Arguments
///
/// * `workspace` - The workspace containing the KG
/// * `branch` - The branch name
/// * `edges` - Slice of edges to upsert
///
/// # Returns
///
/// The number of edges upserted.
pub fn append_edges(
    workspace: &Workspace,
    branch: &str,
    edges: &[KgEdge],
) -> Result<usize, GikError> {
    if edges.is_empty() {
        return Ok(0);
    }

    let store = open_kg_store(workspace, branch)?;
    let count = store.upsert_edges(edges)?;
    store.flush()?;
    Ok(count)
}

/// Reads all edges for a given workspace and branch.
///
/// # Arguments
///
/// * `workspace` - The workspace containing the KG
/// * `branch` - The branch name
///
/// # Returns
///
/// All edges in the store, or an empty vector if the store is empty.
pub fn read_all_edges(workspace: &Workspace, branch: &str) -> Result<Vec<KgEdge>, GikError> {
    // Check if KG directory exists first (lazy init check)
    let kg_dir = kg_dir_for_branch(workspace, branch);
    if !kg_dir.exists() {
        return Ok(Vec::new());
    }

    let store = open_kg_store(workspace, branch)?;
    store.get_all_edges()
}

/// Reads KG stats for a given workspace and branch.
///
/// Returns default stats (zero counts, current version) if the store is empty.
///
/// # Arguments
///
/// * `workspace` - The workspace containing the KG
/// * `branch` - The branch name
pub fn read_stats(workspace: &Workspace, branch: &str) -> Result<KgStats, GikError> {
    // Check if KG directory exists first (lazy init check)
    let kg_dir = kg_dir_for_branch(workspace, branch);
    if !kg_dir.exists() {
        return Ok(KgStats::default());
    }

    let store = open_kg_store(workspace, branch)?;
    store.get_stats()
}

/// Writes KG stats for a given workspace and branch.
///
/// Note: With LanceDB backend, stats are computed from actual data.
/// This function refreshes the stats from the store.
///
/// # Arguments
///
/// * `workspace` - The workspace containing the KG
/// * `branch` - The branch name
/// * `_stats` - Ignored; stats are computed from store data
pub fn write_stats(workspace: &Workspace, branch: &str, _stats: &KgStats) -> Result<(), GikError> {
    // With LanceDB, stats are derived from actual data via get_stats()
    // This function exists for API compatibility
    let _store = open_kg_store(workspace, branch)?;
    Ok(())
}

/// Computes stats by counting nodes and edges from the store.
///
/// Returns a new KgStats with:
/// - `node_count` from the store's node count
/// - `edge_count` from the store's edge count  
/// - `last_updated` set to now
/// - `version` set to current KG version
pub fn compute_stats(workspace: &Workspace, branch: &str) -> Result<KgStats, GikError> {
    // Check if KG directory exists first (lazy init check)
    let kg_dir = kg_dir_for_branch(workspace, branch);
    if !kg_dir.exists() {
        return Ok(KgStats::default());
    }

    let store = open_kg_store(workspace, branch)?;
    store.get_stats()
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use tempfile::TempDir;

    /// Create a temporary workspace for testing.
    fn create_test_workspace() -> (TempDir, Workspace) {
        let temp_dir = TempDir::new().expect("Failed to create temp dir");

        // Create the .guided/knowledge directory structure
        let knowledge_root = temp_dir.path().join(".guided/knowledge");
        fs::create_dir_all(&knowledge_root).expect("Failed to create knowledge root");

        let workspace = Workspace::from_root(temp_dir.path()).expect("Failed to create workspace");

        (temp_dir, workspace)
    }

    #[test]
    fn test_kg_dir_for_branch() {
        let (_temp_dir, workspace) = create_test_workspace();
        let path = kg_dir_for_branch(&workspace, "main");

        assert!(path.ends_with(".guided/knowledge/main/kg"));
    }

    #[test]
    fn test_ensure_kg_dir_creates_directory() {
        let (_temp_dir, workspace) = create_test_workspace();

        let kg_dir = ensure_kg_dir(&workspace, "main").unwrap();

        assert!(kg_dir.exists());
        assert!(kg_dir.is_dir());
    }

    #[test]
    fn test_append_and_read_nodes() {
        let (_temp_dir, workspace) = create_test_workspace();

        let nodes = vec![
            KgNode::new("file:src/main.rs", "file", "src/main.rs")
                .with_props(json!({"language": "rust"})),
            KgNode::new("file:src/lib.rs", "file", "src/lib.rs")
                .with_props(json!({"language": "rust"})),
        ];

        // Append nodes
        let count = append_nodes(&workspace, "main", &nodes).unwrap();
        assert_eq!(count, 2);

        // Read nodes back
        let read_nodes = read_all_nodes(&workspace, "main").unwrap();
        assert_eq!(read_nodes.len(), 2);

        // Check both nodes exist (order may vary with LanceDB)
        let ids: Vec<_> = read_nodes.iter().map(|n| n.id.as_str()).collect();
        assert!(ids.contains(&"file:src/main.rs"));
        assert!(ids.contains(&"file:src/lib.rs"));
    }

    #[test]
    fn test_append_nodes_empty_slice() {
        let (_temp_dir, workspace) = create_test_workspace();

        let count = append_nodes(&workspace, "main", &[]).unwrap();
        assert_eq!(count, 0);
    }

    #[test]
    fn test_read_nodes_nonexistent_store() {
        let (_temp_dir, workspace) = create_test_workspace();

        let nodes = read_all_nodes(&workspace, "main").unwrap();
        assert!(nodes.is_empty());
    }

    #[test]
    fn test_append_multiple_batches() {
        let (_temp_dir, workspace) = create_test_workspace();

        // First batch
        let nodes1 = vec![KgNode::new("file:a.rs", "file", "a.rs")];
        append_nodes(&workspace, "main", &nodes1).unwrap();

        // Second batch
        let nodes2 = vec![
            KgNode::new("file:b.rs", "file", "b.rs"),
            KgNode::new("file:c.rs", "file", "c.rs"),
        ];
        append_nodes(&workspace, "main", &nodes2).unwrap();

        // Should have all 3 nodes
        let read_nodes = read_all_nodes(&workspace, "main").unwrap();
        assert_eq!(read_nodes.len(), 3);
    }

    #[test]
    fn test_append_and_read_edges() {
        let (_temp_dir, workspace) = create_test_workspace();

        let edges = vec![
            KgEdge::new("file:main.rs", "file:lib.rs", "imports"),
            KgEdge::new("file:lib.rs", "file:util.rs", "imports"),
        ];

        // Append edges
        let count = append_edges(&workspace, "main", &edges).unwrap();
        assert_eq!(count, 2);

        // Read edges back
        let read_edges = read_all_edges(&workspace, "main").unwrap();
        assert_eq!(read_edges.len(), 2);
    }

    #[test]
    fn test_read_edges_nonexistent_store() {
        let (_temp_dir, workspace) = create_test_workspace();

        let edges = read_all_edges(&workspace, "main").unwrap();
        assert!(edges.is_empty());
    }

    #[test]
    fn test_read_stats_nonexistent_store() {
        let (_temp_dir, workspace) = create_test_workspace();

        let stats = read_stats(&workspace, "main").unwrap();
        assert_eq!(stats.node_count, 0);
        assert_eq!(stats.edge_count, 0);
    }

    #[test]
    fn test_compute_stats() {
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
    }

    #[test]
    fn test_different_branches() {
        let (_temp_dir, workspace) = create_test_workspace();

        // Add nodes to different branches
        let main_nodes = vec![KgNode::new("file:main.rs", "file", "main.rs")];
        append_nodes(&workspace, "main", &main_nodes).unwrap();

        let feature_nodes = vec![
            KgNode::new("file:feature.rs", "file", "feature.rs"),
            KgNode::new("file:feature2.rs", "file", "feature2.rs"),
        ];
        append_nodes(&workspace, "feature-x", &feature_nodes).unwrap();

        // Read back from each branch
        let main_read = read_all_nodes(&workspace, "main").unwrap();
        let feature_read = read_all_nodes(&workspace, "feature-x").unwrap();

        assert_eq!(main_read.len(), 1);
        assert_eq!(feature_read.len(), 2);
    }
}
