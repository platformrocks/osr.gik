//! Commit pipeline for GIK knowledge indexing.
//!
//! This module provides the core commit functionality that:
//! 1. Reads pending sources from staging
//! 2. Processes each source (read file → chunk → embed → upsert)
//! 3. Updates base sources and stats
//! 4. Creates a new revision in the timeline
//! 5. Cleans up indexed/failed sources from staging
//!
//! ## Limitations
//!
//! - **Single chunk per file**: No advanced semantic chunking yet. Each file
//!   is treated as one chunk. Large files (>1MB or >10k lines) are marked failed.
//! - **URL support**: Web pages can be fetched and indexed. HTML is parsed and
//!   cleaned to extract only text from main content areas (article, main, etc.),
//!   removing CSS, JavaScript, navigation, and other noise. 30-second timeout.
//! - **Archive support**: Archive sources (ZIP, tar, etc.) are not yet supported.
//! - **No runtime mock**: The real Candle embedding backend is required. If the
//!   model is not downloaded, commit fails with an actionable error message.
//!   Mock backends are only used in tests via `#[cfg(test)]`.
//!
//! ## Phase 8.1 Performance Optimizations
//!
//! - **Batched embeddings**: Texts are accumulated and embedded in batches
//!   (default 32) using `embed_batch()` instead of one-by-one `embed()`.
//! - **Warm-up**: A small dummy embedding is run before the main loop to
//!   pay initialization costs upfront.
//! - **Parallel file reading**: Files are read and validated concurrently
//!   using rayon before the embedding phase.

use std::collections::hash_map::DefaultHasher;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};

use rayon::prelude::*;
use serde::{Deserialize, Serialize};

use crate::base::{
    append_base_sources, base_root, load_base_stats, save_base_stats, sources_path, stats_path,
    BaseSourceEntry, BaseStats, ChunkId,
};
use crate::bm25::{load_bm25_index, save_bm25_index, Bm25Config, Bm25Index};
use crate::config::{DevicePreference, GlobalConfig};
#[cfg(test)]
use crate::embedding::create_mock_backend;
use crate::embedding::{
    check_model_compatibility, create_backend, read_model_info, write_model_info, EmbeddingBackend,
    EmbeddingConfig, ModelCompatibility, ModelInfo,
};

use crate::errors::GikError;
use crate::stack::{
    scan_stack, write_dependencies_jsonl, write_files_jsonl, write_stats_json, write_tech_jsonl,
};
use crate::staging::{
    clear_indexed_sources, list_pending_sources, update_source_status, PendingSource,
    PendingSourceId, PendingSourceKind, PendingSourceStatus,
};
use crate::timeline::{append_revision, read_head, write_head, Revision, RevisionOperation};
use crate::types::CommitOptions;
use crate::vector_index::{
    check_index_compatibility, index_meta_path, load_index_meta, open_vector_index,
    write_index_meta, VectorId, VectorIndexBackend, VectorIndexConfig, VectorIndexMeta,
    VectorInsert,
};
use crate::workspace::{BranchName, Workspace};

// ============================================================================
// CommitSummary Types
// ============================================================================

/// Summary of a single base's commit results.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CommitSummaryBase {
    /// The base name.
    pub base: String,

    /// Number of sources indexed successfully.
    pub indexed_count: u64,

    /// Number of sources that failed.
    pub failed_count: u64,

    /// Number of chunks created.
    pub chunk_count: u64,

    /// Number of files processed.
    pub file_count: u64,
}

impl CommitSummaryBase {
    /// Create an empty summary for a base.
    pub fn new(base: impl Into<String>) -> Self {
        Self {
            base: base.into(),
            indexed_count: 0,
            failed_count: 0,
            chunk_count: 0,
            file_count: 0,
        }
    }
}

/// Complete summary of a commit operation.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CommitSummary {
    /// The revision ID created.
    pub revision_id: String,

    /// Total sources indexed across all bases.
    pub total_indexed: u64,

    /// Total sources failed across all bases.
    pub total_failed: u64,

    /// Per-base summaries.
    pub bases: Vec<CommitSummaryBase>,

    /// Bases that were touched.
    pub touched_bases: Vec<String>,
}

// ============================================================================
// Path Helpers
// ============================================================================

/// Get the path to the staging pending.jsonl file.
fn pending_path(knowledge_root: &Path, branch: &str) -> PathBuf {
    knowledge_root
        .join(branch)
        .join("staging")
        .join("pending.jsonl")
}

/// Get the path to the staging summary.json file.
fn summary_path(knowledge_root: &Path, branch: &str) -> PathBuf {
    knowledge_root
        .join(branch)
        .join("staging")
        .join("summary.json")
}

/// Get the path to the model-info.json file for a base.
fn model_info_path(base_root: &Path) -> PathBuf {
    base_root.join("model-info.json")
}

/// Run a full stack scan and persist to disk.
///
/// This was moved from add to commit to make `gik add` faster.
fn scan_and_persist_stack(workspace: &Workspace, branch: &str) -> Result<(), GikError> {
    // Run the stack scan
    let inventory = scan_stack(workspace.root())?;

    // Persist to disk
    let files_path = workspace.stack_files_path(branch);
    let deps_path = workspace.stack_dependencies_path(branch);
    let tech_path = workspace.stack_tech_path(branch);
    let stats_path = workspace.stack_stats_path(branch);

    write_files_jsonl(&files_path, &inventory.files)
        .map_err(|e| GikError::StackPersistFailed(format!("files.jsonl: {}", e)))?;

    write_dependencies_jsonl(&deps_path, &inventory.dependencies)
        .map_err(|e| GikError::StackPersistFailed(format!("dependencies.jsonl: {}", e)))?;

    write_tech_jsonl(&tech_path, &inventory.tech)
        .map_err(|e| GikError::StackPersistFailed(format!("tech.jsonl: {}", e)))?;

    write_stats_json(&stats_path, &inventory.stats)
        .map_err(|e| GikError::StackPersistFailed(format!("stats.json: {}", e)))?;

    tracing::debug!(
        "Stack scan complete: {} files, {} dependencies, {} tech tags",
        inventory.stats.total_files,
        inventory.dependencies.len(),
        inventory.tech.len()
    );

    Ok(())
}

// ============================================================================
// Internal Types
// ============================================================================

/// Result of reading and validating a single file (Phase 8.1).
/// This is the intermediate result before embedding.
#[derive(Clone)]
struct ValidatedSource {
    /// Source ID.
    source_id: String,
    /// File content (text).
    content: String,
    /// URI/path of the source.
    uri: String,
    /// Number of lines in the content.
    line_count: usize,
    /// Content hash for chunk ID generation.
    content_hash: u64,
    /// File modification time (Unix timestamp) for change detection.
    file_mtime: u64,
    /// File size in bytes for change detection.
    file_size: u64,
}

/// Failure from reading/validating a source.
struct ValidationFailure {
    source_id: String,
    reason: String,
}

/// Collected data for a single base during commit.
struct BaseCommitData {
    /// The base name.
    base: String,
    /// Path to the base directory.
    base_dir: PathBuf,
    /// Embedding backend for this base.
    backend: Box<dyn EmbeddingBackend>,
    /// Vector index for this base.
    index: Box<dyn VectorIndexBackend>,
    /// BM25 index for this base (for hybrid search).
    bm25_index: Bm25Index,
    /// Sources to process.
    sources: Vec<PendingSource>,
    /// New source entries created.
    entries: Vec<BaseSourceEntry>,
    /// Vectors to insert.
    vectors: Vec<VectorInsert>,
    /// Successfully indexed source IDs.
    indexed_ids: Vec<String>,
    /// Failed source IDs with reasons.
    failed: Vec<(String, String)>,
    /// Next vector ID to use.
    next_vector_id: u64,
}

// ============================================================================
// Main Commit Function
// ============================================================================

/// Run the commit pipeline.
///
/// This function:
/// 1. Loads pending sources from staging
/// 2. Groups sources by target base
/// 3. For each base:
///    - Checks embedding/index compatibility
///    - Creates/opens the vector index
///    - Processes each source (read → chunk → embed → upsert)
/// 4. Updates base sources and stats
/// 5. Creates a new revision in the timeline
/// 6. Cleans up indexed/failed sources from staging
///
/// # Arguments
///
/// * `workspace` - The workspace to commit in
/// * `branch` - The branch to commit to
/// * `opts` - Commit options (message, etc.)
/// * `global_config` - Global configuration for embedding settings
///
/// # Returns
///
/// A `CommitSummary` describing what was done, or an error.
pub fn run_commit(
    workspace: &Workspace,
    branch: &BranchName,
    opts: &CommitOptions,
    global_config: &GlobalConfig,
) -> Result<CommitSummary, GikError> {
    let knowledge_root = workspace.knowledge_root();
    let branch_str = branch.as_str();

    // 1. Load pending sources
    let pending_file = pending_path(knowledge_root, branch_str);
    let all_sources = list_pending_sources(&pending_file)?;
    let pending_sources: Vec<_> = all_sources
        .into_iter()
        .filter(|s| s.status == PendingSourceStatus::Pending)
        .collect();

    if pending_sources.is_empty() {
        return Err(GikError::CommitNoPendingSources {
            branch: branch_str.to_string(),
        });
    }

    // 2. Group sources by base
    let mut sources_by_base: HashMap<String, Vec<PendingSource>> = HashMap::new();
    for source in pending_sources {
        sources_by_base
            .entry(source.base.clone())
            .or_default()
            .push(source);
    }

    // 3. Process each base
    let mut base_data: Vec<BaseCommitData> = Vec::new();
    let mut touched_bases: Vec<String> = Vec::new();

    for (base_name, sources) in sources_by_base {
        let base_dir = base_root(knowledge_root, branch_str, &base_name);

        // Initialize base data with config
        let embedding_config = global_config.resolve_embedding_config(&base_name);
        let data = prepare_base_for_commit(
            branch_str,
            &base_name,
            &base_dir,
            sources,
            opts.use_mock_backend,
            &embedding_config,
            global_config.device,
        )?;
        touched_bases.push(base_name.clone());
        base_data.push(data);
    }

    // 4. Process sources for each base (with batched embeddings - Phase 8.1)
    let perf_config = &global_config.performance;
    for data in &mut base_data {
        process_base_sources(workspace, branch_str, data, perf_config)?;
    }

    // 5. Upsert vectors and save base data
    for data in &mut base_data {
        finalize_base_commit(data)?;
    }

    // 6. Build revision
    let head_path = knowledge_root.join(branch_str).join("HEAD");
    let head = read_head(&head_path)?;
    let parent_id = head;

    let total_sources: usize = base_data.iter().map(|d| d.indexed_ids.len()).sum();
    let message = opts.message.clone().unwrap_or_else(|| {
        format!(
            "Index {} source{}",
            total_sources,
            if total_sources == 1 { "" } else { "s" }
        )
    });

    let operation = RevisionOperation::Commit {
        bases: touched_bases.clone(),
        source_count: total_sources,
    };

    let revision = Revision::new(branch_str, parent_id, message, vec![operation]);
    let revision_id = revision.id.as_str().to_string();

    // 7. Write revision to timeline and update HEAD
    let timeline_path = knowledge_root.join(branch_str).join("timeline.jsonl");
    append_revision(&timeline_path, &revision)?;
    write_head(&head_path, &revision.id)?;

    // 8. Update source entries with revision ID and save to bases
    for data in &mut base_data {
        // Update revision ID in entries
        for entry in &mut data.entries {
            entry.revision_id = revision_id.clone();
        }

        // Append entries to sources.jsonl
        let sources_file = sources_path(&base_root(knowledge_root, branch_str, &data.base));
        append_base_sources(&sources_file, &data.entries)?;

        // Update and save stats
        let stats_file = stats_path(&base_root(knowledge_root, branch_str, &data.base));
        let mut stats = load_base_stats(&stats_file)?.unwrap_or_else(|| BaseStats::new(&data.base));

        // Collect unique file paths
        let unique_files: HashSet<&str> =
            data.entries.iter().map(|e| e.file_path.as_str()).collect();

        stats.chunk_count += data.entries.len() as u64;
        stats.file_count += unique_files.len() as u64;
        stats.vector_count += data.vectors.len() as u64;
        stats.failed_count += data.failed.len() as u64;
        stats.touch();

        save_base_stats(&stats_file, &stats)?;
    }

    // 8b. Sync Knowledge Graph for the branch (Phase 9.2)
    //
    // KG sync is best-effort: failures are logged but don't fail the commit.
    // This ensures KG issues don't block the primary indexing workflow.
    if let Err(e) = crate::kg::sync_branch_kg_default(workspace, branch_str) {
        // Log warning but continue - KG sync failure shouldn't fail commit
        eprintln!(
            "Warning: KG sync failed for branch '{}': {}. Commit succeeded but KG may be stale.",
            branch_str, e
        );
    }

    // 8c. Rescan stack after commit (moved from add)
    //
    // Stack scanning provides file inventory for status display.
    // Best-effort: failures are logged but don't fail the commit.
    if let Err(e) = scan_and_persist_stack(workspace, branch_str) {
        eprintln!(
            "Warning: Stack scan failed for branch '{}': {}. Commit succeeded but stack may be stale.",
            branch_str, e
        );
    }

    // 9. Update staging - mark indexed sources and remove them
    let summary_file = summary_path(knowledge_root, branch_str);
    for data in &base_data {
        for source_id in &data.indexed_ids {
            update_source_status(
                &pending_file,
                &summary_file,
                &PendingSourceId::new(source_id),
                PendingSourceStatus::Indexed,
                None,
            )?;
        }
        for (source_id, reason) in &data.failed {
            update_source_status(
                &pending_file,
                &summary_file,
                &PendingSourceId::new(source_id),
                PendingSourceStatus::Failed,
                Some(reason.clone()),
            )?;
        }
    }

    // Remove indexed and failed sources from pending
    clear_indexed_sources(&pending_file, &summary_file)?;

    // 10. Build and return summary
    let base_summaries: Vec<CommitSummaryBase> = base_data
        .iter()
        .map(|d| CommitSummaryBase {
            base: d.base.clone(),
            indexed_count: d.indexed_ids.len() as u64,
            failed_count: d.failed.len() as u64,
            chunk_count: d.entries.len() as u64,
            file_count: {
                let files: HashSet<&str> = d.entries.iter().map(|e| e.file_path.as_str()).collect();
                files.len() as u64
            },
        })
        .collect();

    let total_indexed: u64 = base_summaries.iter().map(|s| s.indexed_count).sum();
    let total_failed: u64 = base_summaries.iter().map(|s| s.failed_count).sum();

    Ok(CommitSummary {
        revision_id,
        total_indexed,
        total_failed,
        bases: base_summaries,
        touched_bases,
    })
}

// ============================================================================
// Helper Functions
// ============================================================================

/// Prepare a base for commit by creating/loading embedding backend and vector index.
fn prepare_base_for_commit(
    _branch: &str,
    base_name: &str,
    base_dir: &Path,
    sources: Vec<PendingSource>,
    use_mock_backend: bool,
    embedding_config: &EmbeddingConfig,
    device_pref: DevicePreference,
) -> Result<BaseCommitData, GikError> {
    // Create base directory if needed
    fs::create_dir_all(base_dir).map_err(|e| GikError::BaseStoreIo {
        path: base_dir.to_path_buf(),
        message: format!("Failed to create base directory: {}", e),
    })?;

    // Check embedding compatibility
    let model_info_file = model_info_path(base_dir);
    let existing_model_info = read_model_info(&model_info_file)?;
    let compat = check_model_compatibility(embedding_config, existing_model_info.as_ref());

    match compat {
        ModelCompatibility::Compatible => {}
        ModelCompatibility::MissingModelInfo => {
            // First time indexing this base - will write model info after
        }
        ModelCompatibility::Mismatch { configured, stored } => {
            return Err(GikError::CommitEmbeddingIncompatible {
                base: base_name.to_string(),
                reason: format!(
                    "Model mismatch: configured {:?}, stored {:?}",
                    configured.model_id, stored.model_id
                ),
            });
        }
    }

    // Create embedding backend
    // In tests, use_mock_backend allows using MockEmbeddingBackend.
    // In production, we always use the real backend and fail if unavailable.
    #[cfg(test)]
    let backend: Box<dyn EmbeddingBackend> = if use_mock_backend {
        create_mock_backend(embedding_config)
    } else {
        create_backend(embedding_config, device_pref)?
    };

    #[cfg(not(test))]
    let backend: Box<dyn EmbeddingBackend> = {
        let _ = use_mock_backend; // Silence unused warning in production
        create_backend(embedding_config, device_pref)?
    };

    let dimension = backend.dimension();

    // Check/create vector index
    let index_meta_file = index_meta_path(base_dir);
    let index_meta = load_index_meta(&index_meta_file)?;

    let index_config = VectorIndexConfig::default_for_base(base_name, dimension);
    let index_compat =
        check_index_compatibility(&index_config, embedding_config, index_meta.as_ref());

    if !index_compat.is_compatible() && !index_compat.is_missing() {
        return Err(GikError::CommitIndexIncompatible {
            base: base_name.to_string(),
            reason: format!("{:?}", index_compat),
        });
    }

    // Create index directory
    let index_dir = base_dir.join("index");
    fs::create_dir_all(&index_dir).map_err(|e| GikError::VectorIndexIo {
        path: index_dir.clone(),
        message: format!("Failed to create index directory: {}", e),
    })?;

    // Open or create vector index using the unified factory
    let index: Box<dyn VectorIndexBackend> =
        open_vector_index(index_dir.clone(), index_config.clone(), embedding_config)?;

    // Get next vector ID
    let stats = index.stats()?;
    let next_vector_id = stats.count;

    // Write model info if this is first indexing
    if existing_model_info.is_none() {
        let model_info = ModelInfo::from_config(embedding_config);
        write_model_info(&model_info_file, &model_info)?;
    }

    // Write index metadata if new
    if index_meta.is_none() {
        let meta = VectorIndexMeta::from_config(
            &VectorIndexConfig::default_for_base(base_name, dimension),
            embedding_config,
        );
        write_index_meta(&index_meta_file, &meta)?;
    }

    // Load or create BM25 index for hybrid search
    let bm25_index =
        load_bm25_index(base_dir)?.unwrap_or_else(|| Bm25Index::new(Bm25Config::default()));

    Ok(BaseCommitData {
        base: base_name.to_string(),
        base_dir: base_dir.to_path_buf(),
        backend,
        index,
        bm25_index,
        sources,
        entries: Vec::new(),
        vectors: Vec::new(),
        indexed_ids: Vec::new(),
        failed: Vec::new(),
        next_vector_id,
    })
}

/// Process all sources for a base using batched embeddings (Phase 8.1).
///
/// This function implements a three-phase pipeline:
/// 1. **Read phase**: Read and validate files in parallel using rayon
/// 2. **Embed phase**: Group texts into batches and call embed_batch()
/// 3. **Finalize phase**: Create entries and vectors from embeddings
fn process_base_sources(
    workspace: &Workspace,
    branch: &str,
    data: &mut BaseCommitData,
    perf_config: &crate::config::PerformanceConfig,
) -> Result<(), GikError> {
    let sources: Vec<_> = data.sources.clone();
    let base = data.base.clone();
    let workspace_root = workspace.root().to_path_buf();

    // Phase 0: Warm-up (if enabled)
    if perf_config.enable_warmup {
        if let Err(e) = data.backend.warm_up() {
            tracing::warn!("Embedding warm-up failed (non-fatal): {}", e);
        }
    }

    // Phase 1: Read and validate files in parallel
    let validation_results: Vec<Result<ValidatedSource, ValidationFailure>> =
        if perf_config.parallel_file_reading {
            sources
                .par_iter()
                .map(|source| validate_source(&workspace_root, source, perf_config))
                .collect()
        } else {
            sources
                .iter()
                .map(|source| validate_source(&workspace_root, source, perf_config))
                .collect()
        };

    // Separate validated sources from failures
    let mut validated: Vec<ValidatedSource> = Vec::new();
    for result in validation_results {
        match result {
            Ok(v) => validated.push(v),
            Err(f) => data.failed.push((f.source_id, f.reason)),
        }
    }

    if validated.is_empty() {
        return Ok(());
    }

    // Phase 2: Batch embedding
    let batch_size = perf_config.embedding_batch_size;
    let texts: Vec<String> = validated.iter().map(|v| v.content.clone()).collect();

    // Embed in batches
    let mut all_embeddings: Vec<Vec<f32>> = Vec::with_capacity(texts.len());
    for chunk in texts.chunks(batch_size) {
        let batch_embeddings = data.backend.embed_batch(chunk)?;
        all_embeddings.extend(batch_embeddings);
    }

    // Phase 3: Create entries and vectors
    for (validated_source, embedding) in validated.into_iter().zip(all_embeddings) {
        let chunk_id = ChunkId::generate(
            &base,
            branch,
            &validated_source.uri,
            validated_source.content_hash,
        );
        let vector_id = data.next_vector_id;
        data.next_vector_id += 1;

        // Create vector insert
        let payload = serde_json::json!({
            "chunk_id": chunk_id.as_str(),
            "file_path": validated_source.uri,
            "base": base,
            "branch": branch,
            "start_line": 1,
            "end_line": validated_source.line_count,
        });

        let vector = VectorInsert::new(VectorId::new(vector_id), embedding, payload);

        // Create source entry with file metadata for incremental add
        let entry = BaseSourceEntry::new(
            chunk_id.clone(),
            &base,
            branch,
            &validated_source.uri,
            1,
            validated_source.line_count as u32,
            vector_id,
            "", // Will be filled with revision ID later
            &validated_source.source_id,
        )
        .with_text(validated_source.content.clone())
        .with_file_metadata(validated_source.file_mtime, validated_source.file_size);

        // Add document to BM25 index for hybrid search
        data.bm25_index
            .add_document(chunk_id.as_str().to_string(), &validated_source.content);

        data.indexed_ids.push(validated_source.source_id);
        data.entries.push(entry);
        data.vectors.push(vector);
    }

    Ok(())
}

/// Validate a single source by reading and checking constraints.
/// This function is designed to be called in parallel.
fn validate_source(
    workspace_root: &Path,
    source: &PendingSource,
    perf_config: &crate::config::PerformanceConfig,
) -> Result<ValidatedSource, ValidationFailure> {
    let source_id = source.id.as_str().to_string();

    // Handle unsupported source kinds
    match &source.kind {
        PendingSourceKind::Url => {
            // Fetch URL content using gik-utils
            let content = gik_utils::url::fetch_url_as_markdown(&source.uri).map_err(|e| ValidationFailure {
                source_id: source_id.clone(),
                reason: format!("Failed to fetch URL: {}", e),
            })?;

            // Check content size
            let content_size = content.len() as u64;
            if content_size > perf_config.max_file_size_bytes {
                return Err(ValidationFailure {
                    source_id,
                    reason: format!(
                        "URL content too large: {} bytes (max: {} bytes)",
                        content_size, perf_config.max_file_size_bytes
                    ),
                });
            }

            if content_size == 0 {
                return Err(ValidationFailure {
                    source_id,
                    reason: "URL returned empty content".to_string(),
                });
            }

            // For URLs, use current timestamp as mtime
            let mtime = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_secs();

            // Count lines
            let line_count = content.lines().count();

            // Compute content hash
            let content_hash = {
                let mut hasher = DefaultHasher::new();
                content.hash(&mut hasher);
                hasher.finish()
            };

            return Ok(ValidatedSource {
                source_id,
                content,
                uri: source.uri.clone(),
                line_count,
                content_hash,
                file_mtime: mtime,
                file_size: content_size,
            });
        }
        PendingSourceKind::Archive => {
            return Err(ValidationFailure {
                source_id,
                reason: "Archive sources not supported yet".to_string(),
            });
        }
        PendingSourceKind::Other(kind) => {
            return Err(ValidationFailure {
                source_id,
                reason: format!("Unsupported source kind: {}", kind),
            });
        }
        PendingSourceKind::Directory => {
            return Err(ValidationFailure {
                source_id,
                reason: "Directory sources should be expanded during add".to_string(),
            });
        }
        PendingSourceKind::FilePath => {
            // Continue processing files below
        }
    }

    // Read file content
    let file_path = workspace_root.join(&source.uri);

    if !file_path.exists() {
        return Err(ValidationFailure {
            source_id,
            reason: format!("File not found: {}", source.uri),
        });
    }

    // Check file size
    let metadata = fs::metadata(&file_path).map_err(|e| ValidationFailure {
        source_id: source_id.clone(),
        reason: format!("Failed to read metadata: {}", e),
    })?;

    if metadata.len() > perf_config.max_file_size_bytes {
        return Err(ValidationFailure {
            source_id,
            reason: format!(
                "File too large ({} bytes, max {})",
                metadata.len(),
                perf_config.max_file_size_bytes
            ),
        });
    }

    // Read file content
    let content = fs::read_to_string(&file_path).map_err(|e| ValidationFailure {
        source_id: source_id.clone(),
        reason: format!("Failed to read file: {}", e),
    })?;

    // Check line count
    let line_count = content.lines().count();
    if line_count > perf_config.max_file_lines {
        return Err(ValidationFailure {
            source_id,
            reason: format!(
                "File has too many lines ({}, max {})",
                line_count, perf_config.max_file_lines
            ),
        });
    }

    // Skip empty files
    if content.trim().is_empty() {
        return Err(ValidationFailure {
            source_id,
            reason: "File is empty".to_string(),
        });
    }

    // Capture file metadata for incremental add detection
    let file_size = metadata.len();
    let file_mtime = metadata
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs())
        .unwrap_or(0);

    // Compute content hash
    let content_hash = {
        let mut hasher = DefaultHasher::new();
        content.hash(&mut hasher);
        hasher.finish()
    };

    Ok(ValidatedSource {
        source_id,
        content,
        uri: source.uri.clone(),
        line_count,
        content_hash,
        file_mtime,
        file_size,
    })
}

/// Finalize commit for a base by upserting vectors, saving BM25 index, and flushing.
fn finalize_base_commit(data: &mut BaseCommitData) -> Result<(), GikError> {
    if !data.vectors.is_empty() {
        // Upsert vectors to dense index
        data.index.upsert(&data.vectors)?;
        data.index.flush()?;

        // Save BM25 index for hybrid search
        save_bm25_index(&data.bm25_index, &data.base_dir)?;

        let bm25_stats = data.bm25_index.stats();
        tracing::debug!(
            "BM25 index for base '{}': {} docs, {} terms",
            data.base,
            bm25_stats.num_documents,
            bm25_stats.vocabulary_size
        );
    }
    Ok(())
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::staging::{add_pending_source, NewPendingSource, PendingSourceKind};
    use std::io::Write;
    use tempfile::TempDir;

    fn setup_test_workspace() -> (TempDir, Workspace) {
        let temp = TempDir::new().unwrap();
        let root = temp.path().to_path_buf();
        let knowledge_root = root.join(".guided").join("knowledge");
        fs::create_dir_all(&knowledge_root).unwrap();

        // Create workspace using the from_root method
        let workspace = Workspace::from_root(&root).unwrap();

        (temp, workspace)
    }

    fn test_global_config() -> GlobalConfig {
        GlobalConfig::default_for_testing()
    }

    fn setup_initialized_branch(workspace: &Workspace, branch: &str) {
        let branch_dir = workspace.knowledge_root().join(branch);
        fs::create_dir_all(branch_dir.join("staging")).unwrap();
        fs::create_dir_all(branch_dir.join("bases")).unwrap();

        // Write HEAD
        fs::write(branch_dir.join("HEAD"), "init-rev").unwrap();

        // Write empty timeline
        fs::write(branch_dir.join("timeline.jsonl"), "").unwrap();
    }

    fn create_test_file(workspace: &Workspace, path: &str, content: &str) {
        let file_path = workspace.root().join(path);
        if let Some(parent) = file_path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        let mut file = fs::File::create(file_path).unwrap();
        file.write_all(content.as_bytes()).unwrap();
    }

    fn staging_pending_path_helper(knowledge_root: &Path, branch: &str) -> PathBuf {
        knowledge_root
            .join(branch)
            .join("staging")
            .join("pending.jsonl")
    }

    fn staging_summary_path_helper(knowledge_root: &Path, branch: &str) -> PathBuf {
        knowledge_root
            .join(branch)
            .join("staging")
            .join("summary.json")
    }

    fn add_source(workspace: &Workspace, branch: &str, new: NewPendingSource) -> PendingSourceId {
        let pending = staging_pending_path_helper(workspace.knowledge_root(), branch);
        let summary = staging_summary_path_helper(workspace.knowledge_root(), branch);
        add_pending_source(&pending, &summary, branch, new, Some(workspace.root())).unwrap()
    }

    #[test]
    fn test_commit_no_pending_sources() {
        let (_temp, workspace) = setup_test_workspace();
        let branch = BranchName::new_unchecked("main");
        setup_initialized_branch(&workspace, "main");

        let opts = CommitOptions::default();
        let config = test_global_config();
        let result = run_commit(&workspace, &branch, &opts, &config);

        assert!(result.is_err());
        match result.unwrap_err() {
            GikError::CommitNoPendingSources { branch: b } => {
                assert_eq!(b, "main");
            }
            e => panic!("Unexpected error: {:?}", e),
        }
    }

    #[test]
    fn test_commit_single_file() {
        let (_temp, workspace) = setup_test_workspace();
        let branch = BranchName::new_unchecked("main");
        setup_initialized_branch(&workspace, "main");

        // Create a test file
        create_test_file(
            &workspace,
            "src/main.rs",
            "fn main() {\n    println!(\"Hello\");\n}\n",
        );

        // Add to staging
        let new_source =
            NewPendingSource::new("code", "src/main.rs").with_kind(PendingSourceKind::FilePath);
        add_source(&workspace, "main", new_source);

        // Run commit
        let opts = CommitOptions {
            message: Some("Test commit".to_string()),
            use_mock_backend: true,
        };
        let config = test_global_config();
        let result = run_commit(&workspace, &branch, &opts, &config);

        assert!(result.is_ok(), "Commit failed: {:?}", result.err());

        let commit_summary = result.unwrap();
        assert_eq!(commit_summary.total_indexed, 1);
        assert_eq!(commit_summary.total_failed, 0);
        assert_eq!(commit_summary.touched_bases, vec!["code"]);
        assert_eq!(commit_summary.bases.len(), 1);
        assert_eq!(commit_summary.bases[0].base, "code");
        assert_eq!(commit_summary.bases[0].indexed_count, 1);
        assert_eq!(commit_summary.bases[0].chunk_count, 1);
    }

    #[test]
    fn test_commit_url_fails() {
        let (_temp, workspace) = setup_test_workspace();
        let branch = BranchName::new_unchecked("main");
        setup_initialized_branch(&workspace, "main");

        // Add URL source to staging (should fail during commit due to network/invalid URL)
        // Using an invalid URL that will fail to fetch
        let new_source =
            NewPendingSource::new("docs", "https://this-domain-does-not-exist-12345.com").with_kind(PendingSourceKind::Url);
        add_source(&workspace, "main", new_source);

        // Also add a valid file so commit doesn't fail entirely
        create_test_file(&workspace, "README.md", "# Test\n");
        let file_source =
            NewPendingSource::new("docs", "README.md").with_kind(PendingSourceKind::FilePath);
        add_source(&workspace, "main", file_source);

        // Run commit
        let opts = CommitOptions {
            message: None,
            use_mock_backend: true,
        };
        let config = test_global_config();
        let result = run_commit(&workspace, &branch, &opts, &config);

        assert!(result.is_ok());
        let commit_summary = result.unwrap();
        assert_eq!(commit_summary.total_indexed, 1);
        assert_eq!(commit_summary.total_failed, 1);
    }

    #[test]
    fn test_commit_missing_file_fails() {
        let (_temp, workspace) = setup_test_workspace();
        let branch = BranchName::new_unchecked("main");
        setup_initialized_branch(&workspace, "main");

        // Add nonexistent file to staging
        let new_source =
            NewPendingSource::new("code", "nonexistent.rs").with_kind(PendingSourceKind::FilePath);
        add_source(&workspace, "main", new_source);

        // Also add a valid file
        create_test_file(&workspace, "exists.rs", "fn test() {}\n");
        let file_source =
            NewPendingSource::new("code", "exists.rs").with_kind(PendingSourceKind::FilePath);
        add_source(&workspace, "main", file_source);

        // Run commit
        let opts = CommitOptions {
            message: None,
            use_mock_backend: true,
        };
        let config = test_global_config();
        let result = run_commit(&workspace, &branch, &opts, &config);

        assert!(result.is_ok());
        let commit_summary = result.unwrap();
        assert_eq!(commit_summary.total_indexed, 1);
        assert_eq!(commit_summary.total_failed, 1);
    }

    #[test]
    fn test_commit_empty_file_fails() {
        let (_temp, workspace) = setup_test_workspace();
        let branch = BranchName::new_unchecked("main");
        setup_initialized_branch(&workspace, "main");

        // Create empty file
        create_test_file(&workspace, "empty.rs", "   \n\n  ");

        // Add to staging
        let new_source =
            NewPendingSource::new("code", "empty.rs").with_kind(PendingSourceKind::FilePath);
        add_source(&workspace, "main", new_source);

        // Also add valid file
        create_test_file(&workspace, "valid.rs", "fn valid() {}\n");
        let valid_source =
            NewPendingSource::new("code", "valid.rs").with_kind(PendingSourceKind::FilePath);
        add_source(&workspace, "main", valid_source);

        // Run commit
        let opts = CommitOptions {
            message: None,
            use_mock_backend: true,
        };
        let config = test_global_config();
        let result = run_commit(&workspace, &branch, &opts, &config);

        assert!(result.is_ok());
        let commit_summary = result.unwrap();
        assert_eq!(commit_summary.total_indexed, 1);
        assert_eq!(commit_summary.total_failed, 1);
    }

    #[test]
    fn test_commit_multiple_files() {
        let (_temp, workspace) = setup_test_workspace();
        let branch = BranchName::new_unchecked("main");
        setup_initialized_branch(&workspace, "main");

        // Create multiple test files
        create_test_file(&workspace, "src/lib.rs", "pub mod utils;\n");
        create_test_file(&workspace, "src/utils.rs", "pub fn helper() {}\n");
        create_test_file(&workspace, "README.md", "# Project\n\nDescription here.\n");

        // Add all to staging
        for (base, path) in [
            ("code", "src/lib.rs"),
            ("code", "src/utils.rs"),
            ("docs", "README.md"),
        ] {
            let source = NewPendingSource::new(base, path).with_kind(PendingSourceKind::FilePath);
            add_source(&workspace, "main", source);
        }

        // Run commit
        let opts = CommitOptions {
            message: Some("Add multiple files".to_string()),
            use_mock_backend: true,
        };
        let config = test_global_config();
        let result = run_commit(&workspace, &branch, &opts, &config);

        assert!(result.is_ok());
        let commit_summary = result.unwrap();
        assert_eq!(commit_summary.total_indexed, 3);
        assert_eq!(commit_summary.total_failed, 0);
        assert_eq!(commit_summary.touched_bases.len(), 2);
        assert!(commit_summary.touched_bases.contains(&"code".to_string()));
        assert!(commit_summary.touched_bases.contains(&"docs".to_string()));
    }

    #[test]
    fn test_commit_summary_base() {
        let summary = CommitSummaryBase::new("test");
        assert_eq!(summary.base, "test");
        assert_eq!(summary.indexed_count, 0);
        assert_eq!(summary.failed_count, 0);
    }
}
