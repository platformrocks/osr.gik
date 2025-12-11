//! Reindex module for GIK.
//!
//! This module provides the reindex functionality for rebuilding vector indexes
//! when the embedding model changes. The reindex process:
//!
//! 1. Loads all sources from the base's `sources.jsonl`
//! 2. For each source, retrieves the text (from entry or re-reads from file)
//! 3. Re-embeds all chunks with the current embedding backend
//! 4. Rebuilds the vector index
//! 5. Updates `model-info.json` with the new model
//! 6. Optionally records a timeline revision
//!
//! ## Phase 8.1 Performance Optimizations
//!
//! - **Batched embeddings**: Texts are embedded in batches using `embed_batch()`.
//! - **Warm-up**: A small dummy embedding is run before the main loop.
//!
//! ## Dry Run Mode
//!
//! When `dry_run` is true, the reindex process computes what would change
//! without writing to disk or creating timeline entries.
//!
//! ## Error Handling
//!
//! Sources that fail to re-read (when `text` is `None`) are skipped and
//! recorded in `ReindexBaseResult.errors` rather than failing the entire operation.

use std::fs;
use std::path::Path;

use chrono::Utc;

use crate::base::{load_base_sources, sources_path, BaseSourceEntry};
use crate::bm25::{save_bm25_index, Bm25Index};
use crate::config::{DevicePreference, PerformanceConfig};
use crate::embedding::{
    check_model_compatibility, create_backend, read_model_info, write_model_info, EmbeddingBackend,
    EmbeddingConfig, ModelInfo,
};
use crate::errors::GikError;
use crate::timeline::{RevisionId, RevisionOperation};
use crate::types::{ReindexBaseResult, ReindexOptions, ReindexResult};
use crate::vector_index::{
    default_vector_index_config_for_base, index_meta_path, open_vector_index, write_index_meta,
    VectorId, VectorIndexBackend, VectorIndexMeta, VectorInsert,
};
use crate::workspace::Workspace;

// ============================================================================
// Constants
// ============================================================================

/// Model info filename.
const MODEL_INFO_FILENAME: &str = "model-info.json";

// ============================================================================
// Public Functions
// ============================================================================

/// Reindex a single base.
///
/// Re-embeds all sources and rebuilds the vector index for the specified base.
/// If `dry_run` is true, reports what would change without writing.
///
/// # Arguments
///
/// * `workspace` - The workspace containing the base
/// * `branch` - The branch to reindex on
/// * `base` - The base name to reindex
/// * `embedding_config` - The embedding configuration to use
/// * `force` - Force reindex even if model hasn't changed
/// * `dry_run` - If true, don't write changes
///
/// # Returns
///
/// A [`ReindexBaseResult`] with details of the operation.
pub fn reindex_base(
    workspace: &Workspace,
    branch: &str,
    base: &str,
    embedding_config: &EmbeddingConfig,
    force: bool,
    dry_run: bool,
    device_pref: DevicePreference,
) -> Result<ReindexBaseResult, GikError> {
    let base_root = crate::base::base_root(workspace.knowledge_root(), branch, base);

    // Check if base directory exists
    if !base_root.exists() {
        return Err(GikError::BaseNotFound(base.to_string()));
    }

    // Check if base has indexed content (sources.jsonl with data)
    if !crate::base::is_base_indexed(workspace.knowledge_root(), branch, base) {
        return Err(GikError::BaseNotIndexed {
            base: base.to_string(),
        });
    }

    // Load model-info
    let model_info_path = base_root.join(MODEL_INFO_FILENAME);
    let existing_model_info = read_model_info(&model_info_path)?;

    // Get model IDs
    let from_model_id = existing_model_info.as_ref().map(|m| m.model_id.clone());
    let to_model_id = embedding_config.model_id.to_string();

    // Check if reindex is needed
    let compatibility = check_model_compatibility(embedding_config, existing_model_info.as_ref());
    let needs_reindex = force || compatibility.is_mismatch();

    if !needs_reindex {
        // No reindex needed
        return Ok(ReindexBaseResult {
            base: base.to_string(),
            from_model_id,
            to_model_id,
            reindexed: false,
            sources_processed: 0,
            chunks_reembedded: 0,
            errors: Vec::new(),
        });
    }

    // Load sources
    let sources_file = sources_path(&base_root);
    let sources = load_base_sources(&sources_file)?;

    if sources.is_empty() {
        return Err(GikError::ReindexNoSources {
            base: base.to_string(),
        });
    }

    // If dry run, compute stats without writing
    if dry_run {
        let mut errors = Vec::new();
        let mut valid_sources = 0;

        for source in &sources {
            if source.text.is_some() {
                valid_sources += 1;
            } else {
                // Check if we could read the file
                let full_path = workspace.root().join(&source.file_path);
                if !full_path.exists() {
                    errors.push(format!(
                        "Source {}: file not found at {}",
                        source.id.as_str(),
                        source.file_path
                    ));
                } else {
                    valid_sources += 1;
                }
            }
        }

        return Ok(ReindexBaseResult {
            base: base.to_string(),
            from_model_id,
            to_model_id,
            reindexed: true, // Would reindex
            sources_processed: valid_sources,
            chunks_reembedded: valid_sources, // 1 chunk per source in current impl
            errors,
        });
    }

    // Create embedding backend
    let backend = create_backend(embedding_config, device_pref)?;

    // Use default performance config for reindex
    let perf_config = PerformanceConfig::default();

    // Perform actual reindex
    do_reindex(
        workspace,
        branch,
        base,
        &base_root,
        &sources,
        backend.as_ref(),
        embedding_config,
        from_model_id,
        to_model_id,
        &perf_config,
    )
}

/// Run the full reindex operation.
///
/// This is called by `GikEngine::reindex()` after resolving the branch and
/// validating options.
///
/// # Arguments
///
/// * `workspace` - The workspace
/// * `opts` - Reindex options
/// * `embedding_config` - Embedding configuration to use
/// * `revision_id` - The revision ID to use (if not dry_run and reindex occurs)
/// * `git_commit` - Optional git commit hash
///
/// # Returns
///
/// A [`ReindexResult`] with the operation summary.
pub fn run_reindex(
    workspace: &Workspace,
    opts: &ReindexOptions,
    embedding_config: &EmbeddingConfig,
    revision_id: Option<&RevisionId>,
    git_commit: Option<&str>,
    device_pref: DevicePreference,
) -> Result<ReindexResult, GikError> {
    let branch = opts.branch.as_deref().unwrap_or("main");

    // Reindex the single base
    let base_result = reindex_base(
        workspace,
        branch,
        &opts.base,
        embedding_config,
        opts.force,
        opts.dry_run,
        device_pref,
    )?;

    let reembedded_chunks = base_result.chunks_reembedded;
    let bases = vec![base_result.clone()];

    // Determine if we need to create a revision
    let revision = if !opts.dry_run && base_result.reindexed {
        // Create timeline revision
        if let Some(rev_id) = revision_id {
            let operation = RevisionOperation::Reindex {
                base: opts.base.clone(),
                from_model_id: base_result.from_model_id.clone().unwrap_or_default(),
                to_model_id: base_result.to_model_id.clone(),
            };

            let revision = crate::timeline::Revision {
                id: rev_id.clone(),
                parent_id: None, // Will be set by engine
                branch: branch.to_string(),
                git_commit: git_commit.map(|s| s.to_string()),
                timestamp: Utc::now(),
                message: format!(
                    "Reindex base '{}': {} -> {}",
                    opts.base,
                    base_result.from_model_id.as_deref().unwrap_or("none"),
                    base_result.to_model_id
                ),
                operations: vec![operation],
            };

            Some(revision)
        } else {
            None
        }
    } else {
        None
    };

    Ok(ReindexResult {
        revision,
        reembedded_chunks,
        bases,
        dry_run: opts.dry_run,
    })
}

// ============================================================================
// Private Functions
// ============================================================================

/// Perform the actual reindex operation with batched embeddings (Phase 8.1).
#[allow(clippy::too_many_arguments)]
fn do_reindex(
    workspace: &Workspace,
    branch: &str,
    base: &str,
    base_root: &Path,
    sources: &[BaseSourceEntry],
    backend: &dyn EmbeddingBackend,
    embedding_config: &EmbeddingConfig,
    from_model_id: Option<String>,
    to_model_id: String,
    perf_config: &PerformanceConfig,
) -> Result<ReindexBaseResult, GikError> {
    let mut errors: Vec<String> = Vec::new();

    // Phase 0: Warm-up (if enabled)
    if perf_config.enable_warmup {
        if let Err(e) = backend.warm_up() {
            tracing::warn!("Embedding warm-up failed (non-fatal): {}", e);
        }
    }

    // Phase 1: Read all texts and collect valid sources
    let mut valid_sources: Vec<(&BaseSourceEntry, String)> = Vec::new();
    for source in sources {
        match get_source_text(workspace, source) {
            Ok(text) => valid_sources.push((source, text)),
            Err(e) => {
                errors.push(format!(
                    "Source {}: failed to read text: {}",
                    source.id.as_str(),
                    e
                ));
            }
        };
    }

    if valid_sources.is_empty() {
        return Ok(ReindexBaseResult {
            base: base.to_string(),
            from_model_id,
            to_model_id,
            reindexed: false,
            sources_processed: 0,
            chunks_reembedded: 0,
            errors,
        });
    }

    // Phase 2: Batch embedding
    let texts: Vec<String> = valid_sources.iter().map(|(_, text)| text.clone()).collect();
    let batch_size = perf_config.embedding_batch_size;

    let mut all_embeddings: Vec<Vec<f32>> = Vec::with_capacity(texts.len());
    for chunk in texts.chunks(batch_size) {
        match backend.embed_batch(chunk) {
            Ok(embeddings) => all_embeddings.extend(embeddings),
            Err(_batch_err) => {
                // If batch fails, try individual embedding as fallback
                for text in chunk {
                    match backend.embed(text) {
                        Ok(emb) => all_embeddings.push(emb),
                        Err(e) => {
                            errors.push(format!("Embedding failed: {}", e));
                            // Push a placeholder (will be filtered below)
                            all_embeddings.push(vec![]);
                        }
                    }
                }
            }
        }
    }

    // Phase 3: Create vector inserts
    let mut vector_inserts: Vec<VectorInsert> = Vec::new();
    let mut chunks_reembedded = 0;
    let mut sources_processed = 0;

    for ((source, _), embedding) in valid_sources.into_iter().zip(all_embeddings) {
        if embedding.is_empty() {
            // Skip failed embeddings
            continue;
        }

        vector_inserts.push(VectorInsert {
            id: VectorId(source.vector_id),
            embedding,
            payload: serde_json::json!({
                "chunk_id": source.id.as_str(),
                "file_path": source.file_path,
                "base": base,
                "branch": branch,
            }),
        });

        sources_processed += 1;
        chunks_reembedded += 1;
    }

    // Rebuild vector index
    let index_config = default_vector_index_config_for_base(base, embedding_config);
    let index_root = base_root.join("index");

    // Remove old index files and recreate
    if index_root.exists() {
        fs::remove_dir_all(&index_root).map_err(|e| GikError::VectorIndexIo {
            path: index_root.clone(),
            message: format!("Failed to remove old index: {}", e),
        })?;
    }

    // Create index using the unified factory
    let mut index: Box<dyn VectorIndexBackend> =
        open_vector_index(index_root.clone(), index_config.clone(), embedding_config).map_err(
            |e| GikError::ReindexIndexError {
                base: base.to_string(),
                reason: format!("Failed to create index: {}", e),
            },
        )?;

    // Insert vectors
    if !vector_inserts.is_empty() {
        index
            .upsert(&vector_inserts)
            .map_err(|e| GikError::ReindexIndexError {
                base: base.to_string(),
                reason: format!("Failed to insert vectors: {}", e),
            })?;
    }

    // Flush index
    index.flush().map_err(|e| GikError::ReindexIndexError {
        base: base.to_string(),
        reason: format!("Failed to flush index: {}", e),
    })?;

    // Build BM25 index for hybrid search
    tracing::debug!("Building BM25 index for hybrid search...");
    let mut bm25_index = Bm25Index::new(crate::bm25::Bm25Config::default());
    for source in sources {
        if let Ok(text) = get_source_text(workspace, source) {
            bm25_index.add_document(source.id.as_str().to_string(), &text);
        }
    }
    save_bm25_index(&bm25_index, base_root).map_err(|e| GikError::ReindexIndexError {
        base: base.to_string(),
        reason: format!("Failed to save BM25 index: {}", e),
    })?;
    tracing::debug!(
        "BM25 index built with {} documents",
        bm25_index.num_documents()
    );

    // Update model-info
    let mut model_info = ModelInfo::from_config(embedding_config);
    model_info.touch_reindex();
    let model_info_path = base_root.join(MODEL_INFO_FILENAME);
    write_model_info(&model_info_path, &model_info)?;

    // Update index metadata
    let index_meta_file = index_meta_path(base_root);
    let index_meta = VectorIndexMeta::from_config(
        &default_vector_index_config_for_base(base, embedding_config),
        embedding_config,
    );
    write_index_meta(&index_meta_file, &index_meta)?;

    Ok(ReindexBaseResult {
        base: base.to_string(),
        from_model_id,
        to_model_id,
        reindexed: true,
        sources_processed,
        chunks_reembedded,
        errors,
    })
}

/// Get the text content for a source entry.
///
/// If `source.text` is `Some`, returns it directly.
/// Otherwise, reads the file from `source.file_path`.
///
/// For memory entries (file_path starts with "memory:"), the text MUST be stored
/// in the source entry since there is no physical file to read from.
fn get_source_text(workspace: &Workspace, source: &BaseSourceEntry) -> Result<String, String> {
    if let Some(text) = &source.text {
        return Ok(text.clone());
    }

    // Memory entries are virtual and have no physical file
    if source.file_path.starts_with("memory:") {
        return Err(format!(
            "memory entry {} has no stored text (corrupted data)",
            source.file_path
        ));
    }

    // Re-read from file
    let full_path = workspace.root().join(&source.file_path);

    if !full_path.exists() {
        return Err(format!("file not found: {}", source.file_path));
    }

    let content = fs::read_to_string(&full_path)
        .map_err(|e| format!("failed to read {}: {}", source.file_path, e))?;

    // Extract the specific lines if start_line/end_line are set
    if source.start_line > 0 && source.end_line > 0 {
        let lines: Vec<&str> = content.lines().collect();
        let start = (source.start_line - 1) as usize;
        let end = source.end_line as usize;

        if start < lines.len() {
            let end = end.min(lines.len());
            return Ok(lines[start..end].join("\n"));
        }
    }

    Ok(content)
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_get_source_text_with_text() {
        // Mock workspace (not used when text is present)
        use tempfile::TempDir;
        let temp_dir = TempDir::new().unwrap();
        let workspace = Workspace::from_root(temp_dir.path()).unwrap();

        let source = BaseSourceEntry {
            id: crate::base::ChunkId::new("test-001"),
            base: "code".to_string(),
            branch: "main".to_string(),
            file_path: "src/main.rs".to_string(),
            start_line: 1,
            end_line: 10,
            text: Some("fn main() {}".to_string()),
            vector_id: 1,
            indexed_at: Utc::now(),
            revision_id: "rev-001".to_string(),
            source_id: "src-001".to_string(),
            indexed_mtime: None,
            indexed_size: None,
            extra: None,
        };

        let result = get_source_text(&workspace, &source);
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), "fn main() {}");
    }

    #[test]
    fn test_get_source_text_file_not_found() {
        use tempfile::TempDir;
        let temp_dir = TempDir::new().unwrap();
        let workspace = Workspace::from_root(temp_dir.path()).unwrap();

        let source = BaseSourceEntry {
            id: crate::base::ChunkId::new("test-001"),
            base: "code".to_string(),
            branch: "main".to_string(),
            file_path: "nonexistent.rs".to_string(),
            start_line: 1,
            end_line: 10,
            text: None,
            vector_id: 1,
            indexed_at: Utc::now(),
            revision_id: "rev-001".to_string(),
            source_id: "src-001".to_string(),
            indexed_mtime: None,
            indexed_size: None,
            extra: None,
        };

        let result = get_source_text(&workspace, &source);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("file not found"));
    }
}
