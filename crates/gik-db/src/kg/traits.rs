//! Knowledge Graph store traits and configuration.
//!
//! This module defines the core abstraction for KG storage backends.

use crate::error::DbResult;
use std::path::PathBuf;

use super::entities::{KgEdge, KgNode, KgStats};

// ============================================================================
// KgStoreConfig
// ============================================================================

/// Configuration for a KG store.
#[derive(Debug, Clone)]
pub struct KgStoreConfig {
    /// Path to the KG store directory.
    pub path: PathBuf,

    /// Optional branch name for branch-specific filtering.
    pub branch: Option<String>,
}

impl KgStoreConfig {
    /// Create a new KG store configuration.
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            branch: None,
        }
    }

    /// Set the branch for this configuration.
    pub fn with_branch(mut self, branch: impl Into<String>) -> Self {
        self.branch = Some(branch.into());
        self
    }
}

// ============================================================================
// KgStoreBackend Trait
// ============================================================================

/// Trait for KG storage backend implementations.
///
/// Implementors provide storage and query capabilities for knowledge graph
/// nodes and edges.
///
/// ## Sync Design
///
/// This trait is synchronous. Async backends (like LanceDB) wrap their async
/// operations using `tokio::runtime::Runtime::block_on()` internally.
pub trait KgStoreBackend: Send + Sync {
    // ========================================================================
    // Node Operations
    // ========================================================================

    /// Insert or update nodes in the store.
    ///
    /// Uses upsert semantics: if a node with the same ID exists, it is replaced.
    fn upsert_nodes(&self, nodes: &[KgNode]) -> DbResult<usize>;

    /// Get all nodes in the store.
    fn get_all_nodes(&self) -> DbResult<Vec<KgNode>>;

    /// Get nodes by their IDs.
    fn get_nodes_by_ids(&self, ids: &[&str]) -> DbResult<Vec<KgNode>>;

    /// Get nodes by kind (type).
    fn get_nodes_by_kind(&self, kind: &str) -> DbResult<Vec<KgNode>>;

    /// Get nodes by branch.
    fn get_nodes_by_branch(&self, branch: &str) -> DbResult<Vec<KgNode>>;

    /// Delete nodes by their IDs.
    fn delete_nodes(&self, ids: &[&str]) -> DbResult<usize>;

    /// Count total nodes.
    fn count_nodes(&self) -> DbResult<u64>;

    // ========================================================================
    // Edge Operations
    // ========================================================================

    /// Insert or update edges in the store.
    ///
    /// Uses upsert semantics: if an edge with the same ID exists, it is replaced.
    fn upsert_edges(&self, edges: &[KgEdge]) -> DbResult<usize>;

    /// Get all edges in the store.
    fn get_all_edges(&self) -> DbResult<Vec<KgEdge>>;

    /// Get edges by their IDs.
    fn get_edges_by_ids(&self, ids: &[&str]) -> DbResult<Vec<KgEdge>>;

    /// Get edges originating from a node.
    fn get_edges_from(&self, node_id: &str) -> DbResult<Vec<KgEdge>>;

    /// Get edges pointing to a node.
    fn get_edges_to(&self, node_id: &str) -> DbResult<Vec<KgEdge>>;

    /// Get edges by kind (relationship type).
    fn get_edges_by_kind(&self, kind: &str) -> DbResult<Vec<KgEdge>>;

    /// Get edges by branch.
    fn get_edges_by_branch(&self, branch: &str) -> DbResult<Vec<KgEdge>>;

    /// Delete edges by their IDs.
    fn delete_edges(&self, ids: &[&str]) -> DbResult<usize>;

    /// Count total edges.
    fn count_edges(&self) -> DbResult<u64>;

    // ========================================================================
    // Stats Operations
    // ========================================================================

    /// Get current statistics for the KG.
    fn get_stats(&self) -> DbResult<KgStats>;

    /// Recompute and update statistics.
    fn refresh_stats(&self) -> DbResult<KgStats>;

    // ========================================================================
    // Bulk Operations
    // ========================================================================

    /// Clear all nodes and edges from the store.
    fn clear(&self) -> DbResult<()>;

    /// Flush any pending writes to persistent storage.
    fn flush(&self) -> DbResult<()>;
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_kg_store_config() {
        let config = KgStoreConfig::new("/path/to/kg").with_branch("main");

        assert_eq!(config.path, PathBuf::from("/path/to/kg"));
        assert_eq!(config.branch, Some("main".to_string()));
    }
}
