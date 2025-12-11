//! Knowledge Graph (KG) module for GIK.
//!
//! This module provides the core infrastructure for the Knowledge Graph:
//! - [`KgNode`] - Nodes representing entities (files, modules, symbols, concepts)
//! - [`KgEdge`] - Edges representing relationships between nodes
//! - [`KgStats`] - Aggregate statistics for the graph
//! - [`KgExtractor`] - Trait for extracting KG from bases
//! - [`KgSyncResult`] - Result of KG synchronization
//!
//! ## Storage
//!
//! KG data is stored per-branch under `.guided/knowledge/<branch>/kg/`
//! using gik-db's LanceDB backend for efficient queries and scalability.
//!
//! ## Lazy Initialization
//!
//! The `kg/` directory is created lazily on first write (not during `gik init`).
//! This keeps KG optional for projects that don't use it.
//!
//! ## Migration History
//!
//! - Phase 9.1-9.2: Initial JSONL-based storage
//! - Phase 4 (Migration 4): Migrated to gik-db LanceDB backend

pub mod entities;
pub mod export;
pub mod extractor;
pub mod lang;
pub mod query;
pub mod store;
pub mod sync;

// Re-export core types
pub use entities::{KgEdge, KgNode, KgStats, KG_VERSION};

// Re-export export types
pub use export::{export_kg, export_to_dot, export_to_mermaid, KgExportFormat, KgExportOptions};

// Re-export extractor types
pub use extractor::{
    DefaultKgExtractor, KgExtractionConfig, KgExtractionResult, KgExtractor, DEFAULT_KG_BASES,
};

// Re-export query types
pub use query::{
    build_ask_kg_context, detect_exhaustive_intent, search_kg_exhaustive, AskKgResult,
    ExhaustiveQueryIntent, KgQueryConfig, RagChunkRef,
};

// Re-export sync types
pub use sync::{clear_branch_kg, sync_branch_kg, sync_branch_kg_default, KgSyncResult};

// Re-export store functions and factory
pub use store::{
    append_edges, append_nodes, compute_stats, ensure_kg_dir, open_kg_store,
    open_kg_store_from_root, read_all_edges, read_all_nodes, read_stats, write_stats,
    EDGES_FILENAME, KG_DIR_NAME, NODES_FILENAME, STATS_FILENAME,
};

use crate::errors::GikError;
use crate::workspace::Workspace;

/// Initializes the KG directory for a given workspace and branch.
///
/// Creates `.guided/knowledge/<branch>/kg/` if it doesn't exist.
///
/// This is typically called lazily on first write, but can be called
/// explicitly if needed.
///
/// # Arguments
///
/// * `workspace` - The workspace to initialize KG for
/// * `branch` - The branch name
///
/// # Returns
///
/// The path to the created KG directory.
///
/// # Errors
///
/// Returns an error if the directory cannot be created.
pub fn init_kg_for_branch(
    workspace: &Workspace,
    branch: &str,
) -> Result<std::path::PathBuf, GikError> {
    ensure_kg_dir(workspace, branch)
}

/// Checks if the KG directory exists for a given workspace and branch.
///
/// Since KG is lazily initialized, this returns `false` for projects
/// that haven't written any KG data yet.
pub fn kg_exists(workspace: &Workspace, branch: &str) -> bool {
    store::kg_dir_for_branch(workspace, branch).exists()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn create_test_workspace() -> (TempDir, Workspace) {
        let temp_dir = TempDir::new().expect("Failed to create temp dir");

        // Create the .guided/knowledge directory structure
        let knowledge_root = temp_dir.path().join(".guided/knowledge");
        fs::create_dir_all(&knowledge_root).expect("Failed to create knowledge root");

        let workspace = Workspace::from_root(temp_dir.path()).expect("Failed to create workspace");

        (temp_dir, workspace)
    }

    #[test]
    fn test_init_kg_for_branch() {
        let (_temp_dir, workspace) = create_test_workspace();

        assert!(!kg_exists(&workspace, "main"));

        let kg_dir = init_kg_for_branch(&workspace, "main").unwrap();

        assert!(kg_exists(&workspace, "main"));
        assert!(kg_dir.exists());
    }

    #[test]
    fn test_kg_exists_false_initially() {
        let (_temp_dir, workspace) = create_test_workspace();

        assert!(!kg_exists(&workspace, "main"));
        assert!(!kg_exists(&workspace, "feature-x"));
    }
}
