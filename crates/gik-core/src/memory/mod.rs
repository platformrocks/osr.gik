//! Memory entities and storage for GIK knowledge bases.
//!
//! This module provides the domain model for the `memory` knowledge base:
//! - [`MemoryScope`] - defines the visibility/scope of a memory entry
//! - [`MemorySource`] - categorizes how the memory was created
//! - [`MemoryEntry`] - a single memory record that can be embedded and indexed
//! - [`MemoryIngestionResult`] - result summary from memory ingestion
//! - [`ingest_memory_entries`] - main function to ingest memory entries into the base
//!
//! ## Submodules
//!
//! - [`metrics`] - Memory-specific metrics (entry count, token estimation)
//! - [`pruning`] - Memory pruning policies and operations (TODO)
//!
//! ## Purpose
//!
//! The `memory` base stores high-level, human-readable knowledge about the project:
//! - Design decisions and their rationale
//! - Observations from experiments, tests, or debugging sessions
//! - Manual notes added by users or agents
//! - Summarized external references
//!
//! Unlike `code` and `docs` bases which index raw files, `memory` entries are
//! structured records that capture contextual knowledge that complements the code.
//!
//! ## Storage
//!
//! Memory entries are stored in `.guided/knowledge/<branch>/bases/memory/sources.jsonl`
//! and indexed using the same embedding/vector index pipeline as other bases.
//!
//! ## Ingestion
//!
//! The [`ingest_memory_entries`] function processes memory entries by:
//! 1. Creating the memory base directory if needed
//! 2. Setting up embedding backend and vector index
//! 3. Generating embeddings for each entry's text
//! 4. Upserting vectors into the index
//! 5. Appending source entries to `sources.jsonl`
//! 6. Updating base statistics

// Submodules
pub mod metrics;
pub mod pruning;

use std::collections::hash_map::DefaultHasher;
use std::fs;
use std::hash::{Hash, Hasher};
use std::path::Path;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::base::{
    append_base_sources, base_root, load_base_stats, save_base_stats, sources_path, stats_path,
    BaseSourceEntry, BaseStats, ChunkId,
};
use crate::config::{DevicePreference, PerformanceConfig};
use crate::embedding::{
    check_model_compatibility, create_backend, default_embedding_config_for_base, read_model_info,
    write_model_info, EmbeddingBackend, ModelCompatibility, ModelInfo,
};
use crate::errors::GikError;
use crate::vector_index::{
    check_index_compatibility, index_meta_path, load_index_meta, open_vector_index,
    write_index_meta, VectorId, VectorIndexBackend, VectorIndexConfig, VectorIndexMeta,
    VectorInsert,
};

#[cfg(test)]
use crate::embedding::create_mock_backend;

// ============================================================================
// Constants
// ============================================================================

/// The well-known name for the memory knowledge base.
pub const MEMORY_BASE_NAME: &str = "memory";

// ============================================================================
// MemoryScope
// ============================================================================

/// Defines the visibility and scope of a memory entry.
///
/// Memory entries can be scoped to different levels of the project hierarchy,
/// allowing for fine-grained organization of knowledge.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum MemoryScope {
    /// Project-wide knowledge that applies across all branches.
    /// Examples: architectural decisions, coding standards, project goals.
    #[default]
    Project,

    /// Branch-specific knowledge that only applies to a particular branch.
    /// Examples: feature-specific decisions, experiment notes.
    Branch,

    /// Global knowledge that could apply across multiple projects.
    /// Reserved for future cross-project memory sharing.
    Global,
}

impl std::fmt::Display for MemoryScope {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MemoryScope::Project => write!(f, "project"),
            MemoryScope::Branch => write!(f, "branch"),
            MemoryScope::Global => write!(f, "global"),
        }
    }
}

impl std::str::FromStr for MemoryScope {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "project" => Ok(MemoryScope::Project),
            "branch" => Ok(MemoryScope::Branch),
            "global" => Ok(MemoryScope::Global),
            _ => Err(format!("Unknown memory scope: {}", s)),
        }
    }
}

// ============================================================================
// MemorySource
// ============================================================================

/// Categorizes how a memory entry was created.
///
/// This helps distinguish between different types of knowledge and their origins,
/// which can be useful for filtering, prioritization, and pruning strategies.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum MemorySource {
    /// Directly added by user or UI as a manual note.
    #[default]
    ManualNote,

    /// A design or product decision with rationale.
    Decision,

    /// Findings from tasks, tests, experiments, or debugging.
    Observation,

    /// Summarized content from external documentation or resources.
    ExternalReference,

    /// Generated by an AI agent during conversation or task execution.
    AgentGenerated,

    /// Extracted from commit messages or code review comments.
    CommitContext,
}

impl std::fmt::Display for MemorySource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MemorySource::ManualNote => write!(f, "manual_note"),
            MemorySource::Decision => write!(f, "decision"),
            MemorySource::Observation => write!(f, "observation"),
            MemorySource::ExternalReference => write!(f, "external_reference"),
            MemorySource::AgentGenerated => write!(f, "agent_generated"),
            MemorySource::CommitContext => write!(f, "commit_context"),
        }
    }
}

impl std::str::FromStr for MemorySource {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().replace('-', "_").as_str() {
            "manual_note" | "manualnote" => Ok(MemorySource::ManualNote),
            "decision" => Ok(MemorySource::Decision),
            "observation" => Ok(MemorySource::Observation),
            "external_reference" | "externalreference" => Ok(MemorySource::ExternalReference),
            "agent_generated" | "agentgenerated" => Ok(MemorySource::AgentGenerated),
            "commit_context" | "commitcontext" => Ok(MemorySource::CommitContext),
            _ => Err(format!("Unknown memory source: {}", s)),
        }
    }
}

// ============================================================================
// MemoryEntryId
// ============================================================================

/// Unique identifier for a memory entry.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct MemoryEntryId(pub String);

impl MemoryEntryId {
    /// Create a new memory entry ID from a string.
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    /// Generate a new unique memory entry ID.
    ///
    /// Uses UUID v4 format for uniqueness.
    pub fn generate() -> Self {
        Self(format!("mem-{}", uuid::Uuid::new_v4()))
    }

    /// Generate a content-addressable ID from the entry's key fields.
    ///
    /// Useful for deduplication when re-ingesting the same content.
    pub fn from_content(scope: &MemoryScope, source: &MemorySource, text: &str) -> Self {
        let mut hasher = DefaultHasher::new();
        scope.to_string().hash(&mut hasher);
        source.to_string().hash(&mut hasher);
        text.hash(&mut hasher);
        let hash = hasher.finish();
        Self(format!("mem-{:016x}", hash))
    }

    /// Get the ID as a string slice.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for MemoryEntryId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl From<String> for MemoryEntryId {
    fn from(s: String) -> Self {
        Self(s)
    }
}

impl From<&str> for MemoryEntryId {
    fn from(s: &str) -> Self {
        Self(s.to_string())
    }
}

// ============================================================================
// MemoryEntry
// ============================================================================

/// A single memory record in the `memory` knowledge base.
///
/// Memory entries are the primary unit of storage for project knowledge that
/// isn't directly derived from code or documentation files. They capture
/// decisions, notes, observations, and other contextual information.
///
/// ## Embedding
///
/// The `text` field is the primary content that gets embedded into the vector
/// index. The `title` (if present) is prepended to provide additional context.
///
/// ## Example
///
/// ```ignore
/// let entry = MemoryEntry::new(
///     MemoryScope::Project,
///     MemorySource::Decision,
///     "We chose PostgreSQL over MongoDB because our data model is highly relational \
///      and we need strong ACID guarantees for financial transactions.",
/// )
/// .with_title("Database Selection Decision")
/// .with_tags(vec!["architecture", "database", "decision"]);
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MemoryEntry {
    /// Unique identifier for this memory entry.
    pub id: MemoryEntryId,

    /// When this entry was first created.
    pub created_at: DateTime<Utc>,

    /// When this entry was last updated.
    pub updated_at: DateTime<Utc>,

    /// The scope/visibility of this memory.
    pub scope: MemoryScope,

    /// How this memory was created/sourced.
    pub source: MemorySource,

    /// Optional short title or summary.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,

    /// The main memory content that will be embedded.
    pub text: String,

    /// Tags for categorization and filtering.
    #[serde(default)]
    pub tags: Vec<String>,

    /// Branch name if scope == Branch.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,

    /// Link to a timeline revision when the memory is associated with a commit.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub origin_revision: Option<String>,

    /// Additional metadata as key-value pairs.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extra: Option<serde_json::Value>,
}

impl MemoryEntry {
    /// Create a new memory entry with required fields.
    pub fn new(scope: MemoryScope, source: MemorySource, text: impl Into<String>) -> Self {
        let now = Utc::now();
        Self {
            id: MemoryEntryId::generate(),
            created_at: now,
            updated_at: now,
            scope,
            source,
            title: None,
            text: text.into(),
            tags: Vec::new(),
            branch: None,
            origin_revision: None,
            extra: None,
        }
    }

    /// Create a memory entry with a content-addressable ID for deduplication.
    pub fn new_dedup(scope: MemoryScope, source: MemorySource, text: impl Into<String>) -> Self {
        let text_str = text.into();
        let now = Utc::now();
        Self {
            id: MemoryEntryId::from_content(&scope, &source, &text_str),
            created_at: now,
            updated_at: now,
            scope,
            source,
            title: None,
            text: text_str,
            tags: Vec::new(),
            branch: None,
            origin_revision: None,
            extra: None,
        }
    }

    /// Set the title.
    pub fn with_title(mut self, title: impl Into<String>) -> Self {
        self.title = Some(title.into());
        self
    }

    /// Set tags.
    pub fn with_tags(mut self, tags: Vec<String>) -> Self {
        self.tags = tags;
        self
    }

    /// Set the branch (for Branch-scoped entries).
    pub fn with_branch(mut self, branch: impl Into<String>) -> Self {
        self.branch = Some(branch.into());
        self
    }

    /// Set the origin revision.
    pub fn with_origin_revision(mut self, revision: impl Into<String>) -> Self {
        self.origin_revision = Some(revision.into());
        self
    }

    /// Set extra metadata.
    pub fn with_extra(mut self, extra: serde_json::Value) -> Self {
        self.extra = Some(extra);
        self
    }

    /// Get the text to embed (title + text if title exists).
    pub fn embeddable_text(&self) -> String {
        match &self.title {
            Some(title) => format!("{}\n\n{}", title, self.text),
            None => self.text.clone(),
        }
    }

    /// Check if this entry matches the given branch scope.
    pub fn matches_branch(&self, branch: &str) -> bool {
        match self.scope {
            MemoryScope::Project | MemoryScope::Global => true,
            MemoryScope::Branch => self.branch.as_deref() == Some(branch),
        }
    }
}

// ============================================================================
// MemoryIngestionResult
// ============================================================================

/// Result summary from memory ingestion.
///
/// Returned by [`ingest_memory_entries`] to indicate how many entries were
/// successfully ingested and which (if any) failed.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MemoryIngestionResult {
    /// Number of entries successfully ingested.
    pub ingested_count: usize,

    /// Number of entries that failed to ingest.
    pub failed_count: usize,

    /// IDs of successfully ingested entries.
    pub ingested_ids: Vec<String>,

    /// IDs and error messages for failed entries.
    pub failed: Vec<(String, String)>,

    /// Number of vectors created.
    pub vector_count: u64,
}

impl MemoryIngestionResult {
    /// Create an empty result.
    pub fn new() -> Self {
        Self {
            ingested_count: 0,
            failed_count: 0,
            ingested_ids: Vec::new(),
            failed: Vec::new(),
            vector_count: 0,
        }
    }

    /// Returns true if all entries were ingested successfully.
    pub fn is_success(&self) -> bool {
        self.failed_count == 0
    }
}

impl Default for MemoryIngestionResult {
    fn default() -> Self {
        Self::new()
    }
}

// ============================================================================
// Memory Ingestion Options
// ============================================================================

/// Options for memory ingestion.
#[derive(Debug, Clone, Default)]
pub struct MemoryIngestionOptions {
    /// If true, use mock embedding backend (test only).
    #[allow(dead_code)]
    pub use_mock_backend: bool,
    /// Device preference for embedding generation.
    pub device_pref: DevicePreference,
}

// ============================================================================
// Memory Ingestion Functions
// ============================================================================

/// Ingest memory entries into the memory knowledge base.
///
/// This function:
/// 1. Creates the memory base directory if needed
/// 2. Sets up embedding backend and vector index (with compatibility checks)
/// 3. Generates embeddings for each entry's embeddable text
/// 4. Upsererts vectors into the index
/// 5. Appends source entries to `sources.jsonl`
/// 6. Updates base statistics in `stats.json`
///
/// # Arguments
///
/// * `knowledge_root` - Path to `.guided/knowledge/`
/// * `branch` - The branch name to ingest into
/// * `entries` - The memory entries to ingest
/// * `revision_id` - The revision ID to associate with these entries
/// * `opts` - Ingestion options
///
/// # Returns
///
/// A [`MemoryIngestionResult`] summarizing the ingestion, or an error.
///
/// # Errors
///
/// Returns `GikError` if:
/// - Failed to create directories
/// - Embedding model not available
/// - Embedding/index compatibility mismatch
/// - Failed to write to storage
pub fn ingest_memory_entries(
    knowledge_root: &Path,
    branch: &str,
    entries: Vec<MemoryEntry>,
    revision_id: &str,
    opts: &MemoryIngestionOptions,
) -> Result<MemoryIngestionResult, GikError> {
    if entries.is_empty() {
        return Ok(MemoryIngestionResult::new());
    }

    let base_dir = base_root(knowledge_root, branch, MEMORY_BASE_NAME);

    // Create base directory
    fs::create_dir_all(&base_dir).map_err(|e| GikError::BaseStoreIo {
        path: base_dir.clone(),
        message: format!("Failed to create memory base directory: {}", e),
    })?;

    // Get embedding config for memory base
    let embedding_config = default_embedding_config_for_base(MEMORY_BASE_NAME);

    // Check embedding model compatibility
    let model_info_file = base_dir.join("model-info.json");
    let existing_model_info = read_model_info(&model_info_file)?;
    let compat = check_model_compatibility(&embedding_config, existing_model_info.as_ref());

    match compat {
        ModelCompatibility::Compatible => {}
        ModelCompatibility::MissingModelInfo => {
            // First time indexing this base - will write model info after
        }
        ModelCompatibility::Mismatch { configured, stored } => {
            return Err(GikError::CommitEmbeddingIncompatible {
                base: MEMORY_BASE_NAME.to_string(),
                reason: format!(
                    "Model mismatch: configured {:?}, stored {:?}",
                    configured.model_id, stored.model_id
                ),
            });
        }
    }

    // Create embedding backend
    #[cfg(test)]
    let backend: Box<dyn EmbeddingBackend> = if opts.use_mock_backend {
        create_mock_backend(&embedding_config)
    } else {
        create_backend(&embedding_config, opts.device_pref)?
    };

    #[cfg(not(test))]
    let backend: Box<dyn EmbeddingBackend> = {
        let _ = opts; // Silence unused warning in production
        create_backend(&embedding_config, opts.device_pref)?
    };

    let dimension = backend.dimension();

    // Check/create vector index
    let index_meta_file = index_meta_path(&base_dir);
    let index_meta = load_index_meta(&index_meta_file)?;

    let index_config = VectorIndexConfig::default_for_base(MEMORY_BASE_NAME, dimension);
    let index_compat =
        check_index_compatibility(&index_config, &embedding_config, index_meta.as_ref());

    if !index_compat.is_compatible() && !index_compat.is_missing() {
        return Err(GikError::CommitIndexIncompatible {
            base: MEMORY_BASE_NAME.to_string(),
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
    let mut index: Box<dyn VectorIndexBackend> =
        open_vector_index(index_dir.clone(), index_config.clone(), &embedding_config)?;

    // Get next vector ID
    let stats = index.stats()?;
    let mut next_vector_id = stats.count;

    // Write model info if this is first indexing
    if existing_model_info.is_none() {
        let model_info = ModelInfo::from_config(&embedding_config);
        write_model_info(&model_info_file, &model_info)?;
    }

    // Write index metadata if new
    if index_meta.is_none() {
        let meta = VectorIndexMeta::from_config(
            &VectorIndexConfig::default_for_base(MEMORY_BASE_NAME, dimension),
            &embedding_config,
        );
        write_index_meta(&index_meta_file, &meta)?;
    }

    // Process entries - Phase 8.1 batched embedding approach
    // Phase 1: Validate and prepare entries
    let mut result = MemoryIngestionResult::new();
    let mut valid_entries: Vec<(MemoryEntry, String)> = Vec::new(); // (entry, embeddable_text)

    for entry in entries {
        let text = entry.embeddable_text();
        if text.trim().is_empty() {
            result.failed.push((
                entry.id.as_str().to_string(),
                "Empty text content".to_string(),
            ));
            result.failed_count += 1;
            continue;
        }
        valid_entries.push((entry, text));
    }

    if valid_entries.is_empty() {
        return Ok(result);
    }

    // Phase 2: Batch embedding (using PerformanceConfig)
    let perf_config = PerformanceConfig::default();
    let batch_size = perf_config.embedding_batch_size;

    // Warm up the embedding backend
    let _ = backend.warm_up();

    let texts: Vec<String> = valid_entries.iter().map(|(_, text)| text.clone()).collect();
    let mut all_embeddings: Vec<Vec<f32>> = Vec::with_capacity(texts.len());

    for chunk in texts.chunks(batch_size) {
        match backend.embed_batch(chunk) {
            Ok(embeddings) => all_embeddings.extend(embeddings),
            Err(_batch_err) => {
                // Fallback: embed individually if batch fails
                for text in chunk {
                    match backend.embed(text) {
                        Ok(emb) => all_embeddings.push(emb),
                        Err(_) => {
                            // Push empty embedding, will be filtered below
                            all_embeddings.push(vec![]);
                        }
                    }
                }
            }
        }
    }

    // Phase 3: Create vectors and source entries
    let mut source_entries: Vec<BaseSourceEntry> = Vec::new();
    let mut vectors: Vec<VectorInsert> = Vec::new();

    for ((entry, text), embedding) in valid_entries.into_iter().zip(all_embeddings) {
        let entry_id = entry.id.as_str().to_string();

        // Skip entries with empty embeddings (failed to embed)
        if embedding.is_empty() {
            result
                .failed
                .push((entry_id, "Embedding failed".to_string()));
            result.failed_count += 1;
            continue;
        }

        // Generate chunk ID (content-addressable for memory)
        let content_hash = {
            let mut hasher = DefaultHasher::new();
            text.hash(&mut hasher);
            hasher.finish()
        };

        // Use "memory:" prefix for file_path to distinguish from real files
        let virtual_path = format!("memory:{}", entry.id.as_str());
        let chunk_id = ChunkId::generate(MEMORY_BASE_NAME, branch, &virtual_path, content_hash);
        let vector_id = next_vector_id;
        next_vector_id += 1;

        // Create vector insert with payload
        let payload = serde_json::json!({
            "chunk_id": chunk_id.as_str(),
            "file_path": virtual_path,
            "base": MEMORY_BASE_NAME,
            "branch": branch,
            "start_line": 1,
            "end_line": 1,
            "memory_id": entry.id.as_str(),
            "memory_scope": entry.scope.to_string(),
            "memory_source": entry.source.to_string(),
            "tags": entry.tags,
        });

        let vector = VectorInsert::new(VectorId::new(vector_id), embedding, payload);
        vectors.push(vector);

        // Create source entry
        let source_entry = BaseSourceEntry::new(
            chunk_id,
            MEMORY_BASE_NAME,
            branch,
            &virtual_path,
            1, // start_line
            1, // end_line (memory entries are logical units, not line-based)
            vector_id,
            revision_id,
            &entry_id,
        )
        .with_text(text)
        .with_extra(serde_json::json!({
            "memory_id": entry.id.as_str(),
            "memory_scope": entry.scope.to_string(),
            "memory_source": entry.source.to_string(),
            "title": entry.title,
            "tags": entry.tags,
            "created_at": entry.created_at.to_rfc3339(),
            "updated_at": entry.updated_at.to_rfc3339(),
        }));

        source_entries.push(source_entry);
        result.ingested_ids.push(entry_id);
        result.ingested_count += 1;
    }

    // Upsert vectors
    if !vectors.is_empty() {
        index.upsert(&vectors)?;
        index.flush()?;
        result.vector_count = vectors.len() as u64;
    }

    // Append source entries to sources.jsonl
    let sources_file = sources_path(&base_dir);
    append_base_sources(&sources_file, &source_entries)?;

    // Update stats
    let stats_file = stats_path(&base_dir);
    let mut base_stats =
        load_base_stats(&stats_file)?.unwrap_or_else(|| BaseStats::new(MEMORY_BASE_NAME));

    base_stats.chunk_count += source_entries.len() as u64;
    base_stats.file_count += source_entries.len() as u64; // Each memory entry is a logical "file"
    base_stats.vector_count += result.vector_count;
    base_stats.failed_count += result.failed_count as u64;
    base_stats.touch();

    save_base_stats(&stats_file, &base_stats)?;

    Ok(result)
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_memory_scope_serialization() {
        assert_eq!(
            serde_json::to_string(&MemoryScope::Project).unwrap(),
            "\"project\""
        );
        assert_eq!(
            serde_json::to_string(&MemoryScope::Branch).unwrap(),
            "\"branch\""
        );
        assert_eq!(
            serde_json::to_string(&MemoryScope::Global).unwrap(),
            "\"global\""
        );
    }

    #[test]
    fn test_memory_scope_from_str() {
        assert_eq!(
            "project".parse::<MemoryScope>().unwrap(),
            MemoryScope::Project
        );
        assert_eq!(
            "Branch".parse::<MemoryScope>().unwrap(),
            MemoryScope::Branch
        );
        assert_eq!(
            "GLOBAL".parse::<MemoryScope>().unwrap(),
            MemoryScope::Global
        );
        assert!("invalid".parse::<MemoryScope>().is_err());
    }

    #[test]
    fn test_memory_source_serialization() {
        assert_eq!(
            serde_json::to_string(&MemorySource::ManualNote).unwrap(),
            "\"manualNote\""
        );
        assert_eq!(
            serde_json::to_string(&MemorySource::Decision).unwrap(),
            "\"decision\""
        );
    }

    #[test]
    fn test_memory_source_from_str() {
        assert_eq!(
            "manual_note".parse::<MemorySource>().unwrap(),
            MemorySource::ManualNote
        );
        assert_eq!(
            "decision".parse::<MemorySource>().unwrap(),
            MemorySource::Decision
        );
        assert_eq!(
            "agent_generated".parse::<MemorySource>().unwrap(),
            MemorySource::AgentGenerated
        );
    }

    #[test]
    fn test_memory_entry_id_generate() {
        let id1 = MemoryEntryId::generate();
        let id2 = MemoryEntryId::generate();
        assert!(id1.as_str().starts_with("mem-"));
        assert!(id2.as_str().starts_with("mem-"));
        assert_ne!(id1, id2);
    }

    #[test]
    fn test_memory_entry_id_from_content() {
        let id1 = MemoryEntryId::from_content(
            &MemoryScope::Project,
            &MemorySource::Decision,
            "Same text",
        );
        let id2 = MemoryEntryId::from_content(
            &MemoryScope::Project,
            &MemorySource::Decision,
            "Same text",
        );
        assert_eq!(id1, id2);

        let id3 =
            MemoryEntryId::from_content(&MemoryScope::Branch, &MemorySource::Decision, "Same text");
        assert_ne!(id1, id3);
    }

    #[test]
    fn test_memory_entry_new() {
        let entry = MemoryEntry::new(
            MemoryScope::Project,
            MemorySource::Decision,
            "We chose Rust for performance",
        );

        assert!(entry.id.as_str().starts_with("mem-"));
        assert_eq!(entry.scope, MemoryScope::Project);
        assert_eq!(entry.source, MemorySource::Decision);
        assert_eq!(entry.text, "We chose Rust for performance");
        assert!(entry.title.is_none());
        assert!(entry.tags.is_empty());
    }

    #[test]
    fn test_memory_entry_builder() {
        let entry = MemoryEntry::new(
            MemoryScope::Branch,
            MemorySource::Observation,
            "Performance improved by 50%",
        )
        .with_title("Benchmark Results")
        .with_tags(vec!["performance".to_string(), "benchmark".to_string()])
        .with_branch("feature-x")
        .with_origin_revision("abc123");

        assert_eq!(entry.title.as_deref(), Some("Benchmark Results"));
        assert_eq!(entry.tags, vec!["performance", "benchmark"]);
        assert_eq!(entry.branch.as_deref(), Some("feature-x"));
        assert_eq!(entry.origin_revision.as_deref(), Some("abc123"));
    }

    #[test]
    fn test_memory_entry_embeddable_text() {
        let entry_no_title = MemoryEntry::new(
            MemoryScope::Project,
            MemorySource::ManualNote,
            "Just the text",
        );
        assert_eq!(entry_no_title.embeddable_text(), "Just the text");

        let entry_with_title = MemoryEntry::new(
            MemoryScope::Project,
            MemorySource::ManualNote,
            "The body content",
        )
        .with_title("The Title");
        assert_eq!(
            entry_with_title.embeddable_text(),
            "The Title\n\nThe body content"
        );
    }

    #[test]
    fn test_memory_entry_matches_branch() {
        let project_entry =
            MemoryEntry::new(MemoryScope::Project, MemorySource::Decision, "Project wide");
        assert!(project_entry.matches_branch("main"));
        assert!(project_entry.matches_branch("develop"));

        let branch_entry = MemoryEntry::new(
            MemoryScope::Branch,
            MemorySource::Observation,
            "Branch specific",
        )
        .with_branch("feature-x");
        assert!(branch_entry.matches_branch("feature-x"));
        assert!(!branch_entry.matches_branch("main"));
    }

    #[test]
    fn test_memory_entry_serialization_roundtrip() {
        let entry = MemoryEntry::new(MemoryScope::Project, MemorySource::Decision, "Test content")
            .with_title("Test Title")
            .with_tags(vec!["tag1".to_string(), "tag2".to_string()]);

        let json = serde_json::to_string(&entry).unwrap();
        let parsed: MemoryEntry = serde_json::from_str(&json).unwrap();

        assert_eq!(parsed.id, entry.id);
        assert_eq!(parsed.scope, entry.scope);
        assert_eq!(parsed.source, entry.source);
        assert_eq!(parsed.title, entry.title);
        assert_eq!(parsed.text, entry.text);
        assert_eq!(parsed.tags, entry.tags);
    }

    #[test]
    fn test_memory_ingestion_result_default() {
        let result = MemoryIngestionResult::default();
        assert_eq!(result.ingested_count, 0);
        assert_eq!(result.failed_count, 0);
        assert!(result.ingested_ids.is_empty());
        assert!(result.failed.is_empty());
        assert_eq!(result.vector_count, 0);
        assert!(result.is_success());
    }

    #[test]
    fn test_memory_ingestion_empty_entries() {
        use tempfile::TempDir;

        let temp_dir = TempDir::new().unwrap();
        let knowledge_root = temp_dir.path().join("knowledge");
        std::fs::create_dir_all(&knowledge_root).unwrap();

        let opts = MemoryIngestionOptions::default();
        let result = ingest_memory_entries(&knowledge_root, "main", vec![], "rev-001", &opts);

        assert!(result.is_ok());
        let result = result.unwrap();
        assert_eq!(result.ingested_count, 0);
        assert!(result.is_success());
    }

    #[test]
    fn test_memory_ingestion_with_mock_backend() {
        use tempfile::TempDir;

        let temp_dir = TempDir::new().unwrap();
        let knowledge_root = temp_dir.path().join("knowledge");
        std::fs::create_dir_all(&knowledge_root).unwrap();

        let entries = vec![
            MemoryEntry::new(
                MemoryScope::Project,
                MemorySource::Decision,
                "We chose Rust for safety and performance.",
            )
            .with_title("Language Decision")
            .with_tags(vec!["architecture".to_string()]),
            MemoryEntry::new(
                MemoryScope::Project,
                MemorySource::Observation,
                "The codebase follows a modular structure.",
            ),
        ];

        let opts = MemoryIngestionOptions {
            use_mock_backend: true,
            device_pref: DevicePreference::Auto,
        };
        let result = ingest_memory_entries(&knowledge_root, "main", entries, "rev-002", &opts);

        assert!(result.is_ok());
        let result = result.unwrap();
        assert_eq!(result.ingested_count, 2);
        assert_eq!(result.failed_count, 0);
        assert_eq!(result.vector_count, 2);
        assert!(result.is_success());
        assert_eq!(result.ingested_ids.len(), 2);

        // Verify sources.jsonl was created
        let sources_path = knowledge_root
            .join("main")
            .join("bases")
            .join("memory")
            .join("sources.jsonl");
        assert!(sources_path.exists());

        // Verify stats.json was created
        let stats_path = knowledge_root
            .join("main")
            .join("bases")
            .join("memory")
            .join("stats.json");
        assert!(stats_path.exists());

        // Verify index directory was created
        let index_dir = knowledge_root
            .join("main")
            .join("bases")
            .join("memory")
            .join("index");
        assert!(index_dir.exists());
    }

    #[test]
    fn test_memory_ingestion_empty_text_fails() {
        use tempfile::TempDir;

        let temp_dir = TempDir::new().unwrap();
        let knowledge_root = temp_dir.path().join("knowledge");
        std::fs::create_dir_all(&knowledge_root).unwrap();

        let entries = vec![
            MemoryEntry::new(MemoryScope::Project, MemorySource::ManualNote, "Valid text"),
            MemoryEntry::new(
                MemoryScope::Project,
                MemorySource::ManualNote,
                "   ", // Empty after trim
            ),
        ];

        let opts = MemoryIngestionOptions {
            use_mock_backend: true,
            device_pref: DevicePreference::Auto,
        };
        let result = ingest_memory_entries(&knowledge_root, "main", entries, "rev-003", &opts);

        assert!(result.is_ok());
        let result = result.unwrap();
        assert_eq!(result.ingested_count, 1);
        assert_eq!(result.failed_count, 1);
        assert!(!result.is_success());
    }
}
