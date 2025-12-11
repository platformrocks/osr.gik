//! Common types used throughout GIK.
//!
//! This module contains domain types, option structs, and result types
//! used by the engine API and CLI.

use serde::{Deserialize, Serialize};

// ============================================================================
// BaseName
// ============================================================================

/// A knowledge base name (e.g., "code", "docs", "memory", "stack").
///
/// Knowledge bases are the primary organizational unit for indexed content.
/// Each base has its own vector index and metadata.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct BaseName(pub String);

impl BaseName {
    /// Create a new base name.
    pub fn new(name: impl Into<String>) -> Self {
        Self(name.into())
    }

    /// Get the base name as a string slice.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// The default code base name.
    pub fn code() -> Self {
        Self("code".to_string())
    }

    /// The default docs base name.
    pub fn docs() -> Self {
        Self("docs".to_string())
    }

    /// The default memory base name.
    pub fn memory() -> Self {
        Self("memory".to_string())
    }

    /// The default stack base name.
    pub fn stack() -> Self {
        Self("stack".to_string())
    }
}

impl std::fmt::Display for BaseName {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl From<String> for BaseName {
    fn from(s: String) -> Self {
        Self(s)
    }
}

impl From<&str> for BaseName {
    fn from(s: &str) -> Self {
        Self(s.to_string())
    }
}

// ============================================================================
// Command Options
// ============================================================================

/// Options for the `add` command.
#[derive(Debug, Default, Clone)]
pub struct AddOptions {
    /// Paths, URLs, or archive references to stage.
    pub targets: Vec<String>,
    /// Optional explicit knowledge base (e.g., "code", "docs").
    /// If not provided, base is inferred from source kind/extension.
    pub base: Option<String>,
}

/// Information about a skipped source during add.
#[derive(Debug, Clone, Serialize)]
pub struct AddSourceSkip {
    /// The raw input that was skipped.
    pub raw: String,
    /// Reason for skipping.
    pub reason: String,
}

impl AddSourceSkip {
    /// Create a new skip record.
    pub fn new(raw: impl Into<String>, reason: impl Into<String>) -> Self {
        Self {
            raw: raw.into(),
            reason: reason.into(),
        }
    }
}

/// Options for the `unstage` command.
#[derive(Debug, Default, Clone)]
pub struct UnstageOptions {
    /// Files to unstage (paths relative to workspace root).
    pub targets: Vec<String>,
}

/// Options for the `commit` command.
#[derive(Debug, Default, Clone)]
pub struct CommitOptions {
    /// Commit message.
    pub message: Option<String>,
    /// Use mock embedding backend (test-only).
    ///
    /// This field is only effective in test builds (`#[cfg(test)]`).
    /// In production builds, the real Candle backend is always used,
    /// and commit will fail if the model is not available.
    #[doc(hidden)]
    pub use_mock_backend: bool,
}

/// Options for the `ask` command.
///
/// Note: The full AskOptions with builder methods is in ask.rs.
/// This simplified version is kept for backward compatibility.
#[derive(Debug, Default, Clone)]
pub struct AskOptions {
    /// The query string.
    pub query: String,
    /// Restrict query to specific bases (None = auto-detect).
    pub bases: Option<Vec<String>>,
    /// Maximum chunks to return per base.
    pub top_k: Option<usize>,
    /// Include stack summary in results.
    pub include_stack: bool,
}

/// Options for the `reindex` command.
#[derive(Debug, Default, Clone)]
pub struct ReindexOptions {
    /// The base to reindex.
    pub base: String,
    /// Which branch to reindex on (defaults to current branch if None).
    pub branch: Option<String>,
    /// Force reindex even if model hasn't changed.
    pub force: bool,
    /// Dry run: report what would change without writing.
    pub dry_run: bool,
}

/// Query for the `stats` command.
#[derive(Debug, Default, Clone)]
pub struct StatsQuery {
    /// Specific base to query (None = all bases).
    pub base: Option<String>,
}

// Note: ReleaseOptions is defined in release.rs

// ============================================================================
// Command Results
// ============================================================================

/// Result of the `add` command.
#[derive(Debug, Clone, Serialize)]
pub struct AddResult {
    /// IDs of successfully created pending sources.
    pub created: Vec<String>,
    /// Sources that were skipped (with reasons).
    pub skipped: Vec<AddSourceSkip>,
    /// Stack statistics after rescan (if performed).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stack_stats: Option<crate::stack::StackStats>,
}

/// Result of the `unstage` command.
#[derive(Debug, Clone, Serialize)]
pub struct UnstageResult {
    /// Files that were successfully unstaged.
    pub unstaged: Vec<String>,
    /// Files that were not found in staging (with reasons).
    pub not_found: Vec<UnstageSourceSkip>,
}

/// A source that was not found during unstage.
#[derive(Debug, Clone, Serialize)]
pub struct UnstageSourceSkip {
    /// The raw target string provided by the user.
    pub raw: String,
    /// Reason why the source was not found.
    pub reason: String,
}

impl UnstageSourceSkip {
    /// Create a new unstage source skip.
    pub fn new(raw: impl Into<String>, reason: impl Into<String>) -> Self {
        Self {
            raw: raw.into(),
            reason: reason.into(),
        }
    }
}

/// Result of the `commit` command.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CommitResult {
    /// The ID of the created revision.
    pub revision_id: String,

    /// Total sources indexed across all bases.
    pub total_indexed: u64,

    /// Total sources that failed during indexing.
    pub total_failed: u64,

    /// Bases that were touched during the commit.
    pub touched_bases: Vec<String>,

    /// Per-base commit summaries.
    pub bases: Vec<CommitResultBase>,
}

/// Per-base commit result details.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CommitResultBase {
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

// Note: AskContextBundle, RagChunk, and related types are defined in ask.rs
// and re-exported from lib.rs for public API consistency.

/// Result of the `reindex` command.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReindexResult {
    /// The revision created (None if dry_run or nothing reindexed).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub revision: Option<crate::timeline::Revision>,

    /// Total number of chunks re-embedded across all bases.
    pub reembedded_chunks: usize,

    /// Per-base reindex results.
    pub bases: Vec<ReindexBaseResult>,

    /// Whether this was a dry run (no writes).
    pub dry_run: bool,
}

/// Per-base result of reindex operation.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReindexBaseResult {
    /// The base name.
    pub base: String,

    /// The model ID the base was indexed with before reindex.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub from_model_id: Option<String>,

    /// The model ID used for reindexing.
    pub to_model_id: String,

    /// Whether the base was actually reindexed (model changed or forced).
    pub reindexed: bool,

    /// Number of sources processed.
    pub sources_processed: usize,

    /// Number of chunks re-embedded.
    pub chunks_reembedded: usize,

    /// Errors encountered during reindex (e.g., failed to read source file).
    pub errors: Vec<String>,
}

/// Result of memory ingestion via [`GikEngine::ingest_memory`].
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MemoryIngestResult {
    /// The revision ID created (None if nothing was ingested).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub revision_id: Option<String>,

    /// Detailed ingestion result from the memory module.
    pub result: crate::memory::MemoryIngestionResult,
}

/// Result of memory pruning via [`GikEngine::prune_memory`].
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MemoryPruneEngineResult {
    /// The revision ID created (None if nothing was pruned).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub revision_id: Option<String>,

    /// Detailed pruning result from the memory module.
    pub result: crate::memory::pruning::MemoryPruneResult,
}

/// Result of memory metrics query via [`GikEngine::memory_metrics`].
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MemoryMetricsResult {
    /// The branch these metrics are for.
    pub branch: String,

    /// Memory-specific metrics.
    pub metrics: crate::memory::metrics::MemoryMetrics,

    /// Pruning policy (if configured).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pruning_policy: Option<crate::memory::pruning::MemoryPruningPolicy>,
}

/// Result of the `stats` command.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StatsReport {
    /// The branch these stats are for.
    pub branch: String,

    /// Stats per base (using full BaseStatsReport for rich info).
    pub bases: Vec<crate::base::BaseStatsReport>,

    /// Stack statistics (optional).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stack: Option<crate::stack::StackStats>,

    /// Total documents across all bases.
    pub total_documents: u64,

    /// Total vectors across all bases.
    pub total_vectors: u64,

    /// Total on-disk size in bytes.
    pub total_on_disk_bytes: u64,
}

// Note: ReleaseResult is defined in release.rs

// ============================================================================
// Traits (placeholders for future implementation)
// ============================================================================

/// Trait for embedding providers.
///
/// Implementations generate vector embeddings from text inputs.
/// Concrete implementations will be added in phase 0.4.
pub trait EmbeddingProvider: Send + Sync {
    /// Unique identifier for the model (e.g., "all-MiniLM-L6-v2").
    fn model_id(&self) -> &str;

    /// Dimensionality of the embedding vectors.
    fn dim(&self) -> usize;

    /// Generate embeddings for a batch of text inputs.
    fn embed_texts(&self, inputs: &[String]) -> anyhow::Result<Vec<Vec<f32>>>;
}

/// Trait for vector index backends.
///
/// Implementations store and search vector embeddings.
/// Concrete implementations will be added in phase 0.4.
pub trait VectorIndex: Send + Sync {
    /// Add embeddings to the index.
    fn add_embeddings(
        &mut self,
        base: &BaseName,
        vectors: &[Vec<f32>],
        metadata: &[serde_json::Value],
    ) -> anyhow::Result<()>;

    /// Search for similar vectors.
    fn search(
        &self,
        base: &BaseName,
        query: &[f32],
        top_k: usize,
    ) -> anyhow::Result<Vec<SearchResult>>;

    /// Rebuild the index for a base from scratch.
    fn rebuild(&mut self, base: &BaseName, entries: &[ReindexEntry]) -> anyhow::Result<()>;
}

/// Result of a vector search.
#[derive(Debug, Clone)]
pub struct SearchResult {
    /// Similarity score.
    pub score: f32,
    /// Associated metadata.
    pub metadata: serde_json::Value,
}

/// Entry for reindexing.
#[derive(Debug, Clone)]
pub struct ReindexEntry {
    /// The embedding vector.
    pub vector: Vec<f32>,
    /// Associated metadata.
    pub metadata: serde_json::Value,
}

// ============================================================================
// Config Validation
// ============================================================================

/// Information about a configuration source.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConfigSourceInfo {
    /// Name of the config source (e.g., "global", "project").
    pub name: String,
    /// Path to the configuration file.
    pub path: std::path::PathBuf,
    /// Whether the file exists.
    pub exists: bool,
    /// Whether the file is valid (parseable).
    pub valid: bool,
    /// Parse error, if any.
    pub error: Option<String>,
}

/// Result of configuration validation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConfigValidationResult {
    /// Configuration sources checked.
    pub sources: Vec<ConfigSourceInfo>,
    /// Validation warnings (non-fatal issues).
    pub warnings: Vec<String>,
    /// Validation errors (fatal issues).
    pub errors: Vec<String>,
}

impl ConfigValidationResult {
    /// Create a new empty validation result.
    pub fn new() -> Self {
        Self {
            sources: Vec::new(),
            warnings: Vec::new(),
            errors: Vec::new(),
        }
    }

    /// Check if the configuration is valid (no errors).
    pub fn is_valid(&self) -> bool {
        self.errors.is_empty()
    }
}

impl Default for ConfigValidationResult {
    fn default() -> Self {
        Self::new()
    }
}

/// Resolved configuration from all sources.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResolvedConfig {
    /// Device preference.
    pub device: String,
    /// Embedding configuration.
    pub embedding: serde_json::Value,
    /// Retrieval configuration.
    pub retrieval: serde_json::Value,
    /// Model paths.
    pub model_paths: serde_json::Value,
    /// Project-specific overrides applied.
    pub project_overrides: serde_json::Value,
}
