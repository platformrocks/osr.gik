//! Knowledge Graph synchronization module.
//!
//! This module provides helpers to synchronize the KG store with extraction
//! results for a branch. Sync uses a **full rebuild** strategy:
//!
//! 1. Run extractor for the branch
//! 2. Clear existing KG data
//! 3. Write new nodes and edges
//! 4. Stats are computed from store data
//!
//! ## Full Rebuild Strategy
//!
//! Sync implements full rebuild per branch for simplicity:
//! - On `gik commit`: KG is recomputed from all current bases
//! - On `gik reindex`: KG is recomputed after index rebuild
//!
//! This ensures deterministic KG state at the cost of performance on large repos.
//!
//! ## Backend
//!
//! KG storage uses gik-db's LanceDB backend via the `open_kg_store()` factory.

use std::fs;

use crate::errors::GikError;
use crate::workspace::Workspace;

use super::extractor::{DefaultKgExtractor, KgExtractionConfig, KgExtractor};
use super::store::{kg_dir_for_branch, open_kg_store};

// ============================================================================
// KgSyncResult
// ============================================================================

/// Result of a KG sync operation.
#[derive(Debug, Clone, Default)]
pub struct KgSyncResult {
    /// Number of nodes written.
    pub nodes_written: usize,

    /// Number of edges written.
    pub edges_written: usize,

    /// Number of files processed.
    pub files_processed: usize,

    /// Whether the sync was a full rebuild.
    pub full_rebuild: bool,

    /// Any warnings from extraction.
    pub warnings: Vec<String>,
}

impl KgSyncResult {
    /// Create a new sync result.
    pub fn new() -> Self {
        Self::default()
    }
}

// ============================================================================
// Sync Functions
// ============================================================================

/// Synchronize the KG for a branch using the given extractor.
///
/// This function performs a **full rebuild** of the KG:
/// 1. Runs extraction on all bases
/// 2. Clears existing KG data (if any)
/// 3. Writes new nodes and edges
/// 4. Stats are derived from store data
///
/// # Arguments
///
/// * `workspace` - The workspace to sync KG for
/// * `branch` - The branch name
/// * `extractor` - The extractor implementation to use
/// * `cfg` - Extraction configuration
///
/// # Returns
///
/// A [`KgSyncResult`] with details of the operation.
///
/// # Errors
///
/// Returns an error if extraction fails or KG cannot be written.
pub fn sync_branch_kg(
    workspace: &Workspace,
    branch: &str,
    extractor: &impl KgExtractor,
    cfg: &KgExtractionConfig,
) -> Result<KgSyncResult, GikError> {
    // Run extraction
    let extraction = extractor.extract_for_branch(workspace, branch, cfg)?;

    // If no nodes extracted, skip KG creation (keep it lazy)
    if extraction.nodes.is_empty() {
        return Ok(KgSyncResult {
            nodes_written: 0,
            edges_written: 0,
            files_processed: extraction.files_processed,
            full_rebuild: true,
            warnings: extraction.warnings,
        });
    }

    // Open the KG store (creates directory if needed)
    let store = open_kg_store(workspace, branch)?;

    // Clear existing data (full rebuild)
    store.clear()?;

    // Write nodes
    let nodes_count = extraction.nodes.len();
    if !extraction.nodes.is_empty() {
        store.upsert_nodes(&extraction.nodes)?;
    }

    // Write edges
    let edges_count = extraction.edges.len();
    if !extraction.edges.is_empty() {
        store.upsert_edges(&extraction.edges)?;
    }

    // Flush to ensure data is persisted
    store.flush()?;

    Ok(KgSyncResult {
        nodes_written: nodes_count,
        edges_written: edges_count,
        files_processed: extraction.files_processed,
        full_rebuild: true,
        warnings: extraction.warnings,
    })
}

/// Synchronize the KG for a branch using the default extractor and config.
///
/// This is a convenience function that uses [`DefaultKgExtractor`] and
/// [`KgExtractionConfig::default()`].
///
/// # Arguments
///
/// * `workspace` - The workspace to sync KG for
/// * `branch` - The branch name
///
/// # Returns
///
/// A [`KgSyncResult`] with details of the operation.
pub fn sync_branch_kg_default(
    workspace: &Workspace,
    branch: &str,
) -> Result<KgSyncResult, GikError> {
    let extractor = DefaultKgExtractor::new();
    let cfg = KgExtractionConfig::default();
    sync_branch_kg(workspace, branch, &extractor, &cfg)
}

/// Clear the KG for a branch.
///
/// Removes all KG data for the branch by deleting the entire KG directory.
/// Does nothing if the KG directory doesn't exist.
///
/// # Arguments
///
/// * `workspace` - The workspace
/// * `branch` - The branch name
///
/// # Returns
///
/// `Ok(())` on success or if KG doesn't exist.
pub fn clear_branch_kg(workspace: &Workspace, branch: &str) -> Result<(), GikError> {
    let kg_dir = kg_dir_for_branch(workspace, branch);

    if !kg_dir.exists() {
        return Ok(());
    }

    fs::remove_dir_all(&kg_dir).map_err(|e| GikError::BaseStoreIo {
        path: kg_dir,
        message: format!("Failed to clear KG directory: {}", e),
    })?;

    Ok(())
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kg::{kg_exists, read_all_edges, read_all_nodes, read_stats};
    use std::fs;
    use tempfile::TempDir;

    fn create_test_workspace() -> (TempDir, Workspace) {
        let temp_dir = TempDir::new().expect("Failed to create temp dir");

        // Create the .guided/knowledge directory structure
        let knowledge_root = temp_dir.path().join(".guided/knowledge");
        fs::create_dir_all(&knowledge_root).expect("Failed to create knowledge root");

        // Create branch directory
        fs::create_dir_all(knowledge_root.join("main")).expect("Failed to create main branch");

        let workspace = Workspace::from_root(temp_dir.path()).expect("Failed to create workspace");

        (temp_dir, workspace)
    }

    fn create_test_workspace_with_code_base() -> (TempDir, Workspace) {
        let (temp_dir, workspace) = create_test_workspace();

        // Create code base with some sources
        let code_base_dir = workspace.knowledge_root().join("main/bases/code");
        fs::create_dir_all(&code_base_dir).expect("Failed to create code base");

        // Create sources.jsonl with sample entries
        let sources_content = r#"{"id":"chunk-001","base":"code","branch":"main","filePath":"src/index.ts","startLine":1,"endLine":10,"text":"import { helper } from './utils';\nconsole.log('hello');","vectorId":1,"indexedAt":"2025-01-01T00:00:00Z","revisionId":"rev-001","sourceId":"src-001"}
{"id":"chunk-002","base":"code","branch":"main","filePath":"src/utils.ts","startLine":1,"endLine":5,"text":"export function helper() { return 42; }","vectorId":2,"indexedAt":"2025-01-01T00:00:00Z","revisionId":"rev-001","sourceId":"src-002"}"#;

        fs::write(code_base_dir.join("sources.jsonl"), sources_content)
            .expect("Failed to write sources");

        (temp_dir, workspace)
    }

    /// Sync with symbols and endpoints disabled for tests that check exact node counts
    fn sync_without_symbols(workspace: &Workspace, branch: &str) -> KgSyncResult {
        let cfg = KgExtractionConfig::default()
            .without_symbols()
            .without_endpoints();
        let extractor = DefaultKgExtractor::new();
        sync_branch_kg(workspace, branch, &extractor, &cfg).unwrap()
    }

    #[test]
    fn test_sync_branch_kg_empty_workspace() {
        let (_temp_dir, workspace) = create_test_workspace();

        let result = sync_branch_kg_default(&workspace, "main").unwrap();

        // No bases = no nodes = no KG created
        assert_eq!(result.nodes_written, 0);
        assert_eq!(result.edges_written, 0);
        assert!(!kg_exists(&workspace, "main"));
    }

    #[test]
    fn test_sync_branch_kg_with_code_base() {
        let (_temp_dir, workspace) = create_test_workspace_with_code_base();

        let result = sync_without_symbols(&workspace, "main");

        // Should have created 2 file nodes
        assert_eq!(result.nodes_written, 2);
        assert!(result.full_rebuild);
        assert!(kg_exists(&workspace, "main"));

        // Verify nodes
        let nodes = read_all_nodes(&workspace, "main").unwrap();
        assert_eq!(nodes.len(), 2);

        let node_ids: Vec<_> = nodes.iter().map(|n| n.id.as_str()).collect();
        assert!(node_ids.contains(&"file:src/index.ts"));
        assert!(node_ids.contains(&"file:src/utils.ts"));

        // Verify stats
        let stats = read_stats(&workspace, "main").unwrap();
        assert_eq!(stats.node_count, 2);
        assert_eq!(stats.version, crate::kg::KG_VERSION);
    }

    #[test]
    fn test_sync_branch_kg_with_imports() {
        let (_temp_dir, workspace) = create_test_workspace_with_code_base();

        let result = sync_branch_kg_default(&workspace, "main").unwrap();

        // Should have import edge from index.ts to utils.ts
        assert!(result.edges_written > 0);

        let edges = read_all_edges(&workspace, "main").unwrap();
        assert!(!edges.is_empty());

        // Find the import edge
        let import_edge = edges.iter().find(|e| e.kind == "imports");
        assert!(import_edge.is_some());

        let edge = import_edge.unwrap();
        assert_eq!(edge.from, "file:src/index.ts");
        assert_eq!(edge.to, "file:src/utils.ts");
    }

    #[test]
    fn test_sync_branch_kg_full_rebuild() {
        let (_temp_dir, workspace) = create_test_workspace_with_code_base();

        // First sync (without symbols to get consistent counts)
        let result1 = sync_without_symbols(&workspace, "main");
        assert_eq!(result1.nodes_written, 2);

        // Second sync should clear and rebuild
        let result2 = sync_without_symbols(&workspace, "main");
        assert_eq!(result2.nodes_written, 2);
        assert!(result2.full_rebuild);

        // Should still have exactly 2 nodes (not 4)
        let nodes = read_all_nodes(&workspace, "main").unwrap();
        assert_eq!(nodes.len(), 2);
    }

    #[test]
    fn test_clear_branch_kg() {
        let (_temp_dir, workspace) = create_test_workspace_with_code_base();

        // Sync first
        sync_branch_kg_default(&workspace, "main").unwrap();
        assert!(kg_exists(&workspace, "main"));

        // Clear
        clear_branch_kg(&workspace, "main").unwrap();
        assert!(!kg_exists(&workspace, "main"));
    }

    #[test]
    fn test_clear_branch_kg_nonexistent() {
        let (_temp_dir, workspace) = create_test_workspace();

        // Should not error on nonexistent KG
        let result = clear_branch_kg(&workspace, "main");
        assert!(result.is_ok());
    }

    #[test]
    fn test_sync_result_default() {
        let result = KgSyncResult::new();
        assert_eq!(result.nodes_written, 0);
        assert_eq!(result.edges_written, 0);
        assert!(!result.full_rebuild);
    }
}
