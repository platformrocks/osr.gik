//! Base-level entities and storage for GIK knowledge bases.
//!
//! This module provides types and I/O helpers for per-base data:
//! - [`BaseSourceEntry`] - a single indexed source/chunk record
//! - [`BaseStats`] - aggregate statistics for a base
//! - [`BaseHealthState`] - health indicator for a base
//! - [`BaseStatsReport`] - extended stats with compatibility and health info
//! - [`ChunkId`] - unique identifier for a chunk
//!
//! ## On-Disk Format
//!
//! Per-base data is stored under `.guided/knowledge/<branch>/bases/<base>/`:
//! - `sources.jsonl` - One [`BaseSourceEntry`] per line (JSONL)
//! - `stats.json` - [`BaseStats`] (JSON)

use std::collections::hash_map::DefaultHasher;
use std::fs::{self, File, OpenOptions};
use std::hash::{Hash, Hasher};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::errors::GikError;

// ============================================================================
// Constants
// ============================================================================

/// Filename for the sources JSONL file.
pub const SOURCES_FILENAME: &str = "sources.jsonl";

/// Filename for the stats JSON file.
pub const STATS_FILENAME: &str = "stats.json";

/// Maximum file size (bytes) for single-chunk ingestion in Phase 4.3.
/// Files larger than this are marked as failed.
pub const MAX_FILE_SIZE_BYTES: u64 = 1_000_000; // 1 MB

/// Maximum line count for single-chunk ingestion in Phase 4.3.
pub const MAX_FILE_LINES: usize = 10_000;

// ============================================================================
// ChunkId
// ============================================================================

/// Unique identifier for a chunk within a knowledge base.
///
/// Generated as a hash of the base, branch, file path, and content hash.
/// This allows deduplication and stable IDs across re-indexing.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ChunkId(pub String);

impl ChunkId {
    /// Create a new chunk ID from a string.
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    /// Generate a chunk ID from base, branch, file path, and content.
    ///
    /// Uses a hash-based approach to ensure stable, content-addressable IDs.
    pub fn generate(base: &str, branch: &str, file_path: &str, content_hash: u64) -> Self {
        let mut hasher = DefaultHasher::new();
        base.hash(&mut hasher);
        branch.hash(&mut hasher);
        file_path.hash(&mut hasher);
        content_hash.hash(&mut hasher);
        let hash = hasher.finish();
        Self(format!("chunk-{:016x}", hash))
    }

    /// Get the chunk ID as a string slice.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for ChunkId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl From<String> for ChunkId {
    fn from(s: String) -> Self {
        Self(s)
    }
}

impl From<&str> for ChunkId {
    fn from(s: &str) -> Self {
        Self(s.to_string())
    }
}

// ============================================================================
// BaseSourceEntry
// ============================================================================

/// A single indexed source/chunk record in a knowledge base.
///
/// Represents a chunk of content that has been indexed into the vector index.
/// One file may produce one or more chunks (in Phase 4.3, one file = one chunk).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BaseSourceEntry {
    /// Unique identifier for this chunk.
    pub id: ChunkId,

    /// The knowledge base this chunk belongs to.
    pub base: String,

    /// The branch this chunk belongs to.
    pub branch: String,

    /// Path to the source file (workspace-relative).
    pub file_path: String,

    /// Starting line number in the source file (1-based).
    pub start_line: u32,

    /// Ending line number in the source file (1-based, inclusive).
    pub end_line: u32,

    /// The text content of this chunk (optional but recommended).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,

    /// The corresponding vector ID in the index.
    pub vector_id: u64,

    /// When this chunk was first indexed.
    pub indexed_at: DateTime<Utc>,

    /// The revision ID when this chunk was indexed.
    pub revision_id: String,

    /// The pending source ID that produced this chunk.
    pub source_id: String,

    /// File modification time (Unix timestamp) when indexed.
    /// Used for incremental add to detect modified files.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub indexed_mtime: Option<u64>,

    /// File size in bytes when indexed.
    /// Used alongside mtime for change detection.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub indexed_size: Option<u64>,

    /// Additional metadata (language, tags, etc.).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extra: Option<serde_json::Value>,
}

impl BaseSourceEntry {
    /// Create a new base source entry.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        id: ChunkId,
        base: impl Into<String>,
        branch: impl Into<String>,
        file_path: impl Into<String>,
        start_line: u32,
        end_line: u32,
        vector_id: u64,
        revision_id: impl Into<String>,
        source_id: impl Into<String>,
    ) -> Self {
        Self {
            id,
            base: base.into(),
            branch: branch.into(),
            file_path: file_path.into(),
            start_line,
            end_line,
            text: None,
            vector_id,
            indexed_at: Utc::now(),
            revision_id: revision_id.into(),
            source_id: source_id.into(),
            indexed_mtime: None,
            indexed_size: None,
            extra: None,
        }
    }

    /// Set the text content.
    pub fn with_text(mut self, text: impl Into<String>) -> Self {
        self.text = Some(text.into());
        self
    }

    /// Set file metadata for change detection.
    pub fn with_file_metadata(mut self, mtime: u64, size: u64) -> Self {
        self.indexed_mtime = Some(mtime);
        self.indexed_size = Some(size);
        self
    }

    /// Set extra metadata.
    pub fn with_extra(mut self, extra: serde_json::Value) -> Self {
        self.extra = Some(extra);
        self
    }
}

// ============================================================================
// BaseStats
// ============================================================================

/// Aggregate statistics for a knowledge base.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BaseStats {
    /// The knowledge base name.
    pub base: String,

    /// Total number of source entries (chunks).
    pub chunk_count: u64,

    /// Total number of unique files indexed.
    pub file_count: u64,

    /// Total number of vectors in the index.
    pub vector_count: u64,

    /// Number of failed sources.
    pub failed_count: u64,

    /// When these stats were last updated.
    pub last_updated: DateTime<Utc>,
}

impl BaseStats {
    /// Create new empty stats for a base.
    pub fn new(base: impl Into<String>) -> Self {
        Self {
            base: base.into(),
            chunk_count: 0,
            file_count: 0,
            vector_count: 0,
            failed_count: 0,
            last_updated: Utc::now(),
        }
    }

    /// Update the last_updated timestamp.
    pub fn touch(&mut self) {
        self.last_updated = Utc::now();
    }
}

impl Default for BaseStats {
    fn default() -> Self {
        Self::new("")
    }
}

// ============================================================================
// BaseHealthState
// ============================================================================

/// Health indicator for a knowledge base.
///
/// Derived from embedding model compatibility and vector index compatibility.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum BaseHealthState {
    /// Base is healthy: model and index are compatible.
    Healthy,

    /// Base requires reindexing due to model or index mismatch.
    NeedsReindex,

    /// Model info is missing (base never indexed or corrupted).
    MissingModel,

    /// Index files are missing (base never indexed or corrupted).
    IndexMissing,

    /// An error occurred while checking health.
    Error,
}

impl std::fmt::Display for BaseHealthState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Healthy => write!(f, "OK"),
            Self::NeedsReindex => write!(f, "NEEDS_REINDEX"),
            Self::MissingModel => write!(f, "MISSING_MODEL"),
            Self::IndexMissing => write!(f, "INDEX_MISSING"),
            Self::Error => write!(f, "ERROR"),
        }
    }
}

// ============================================================================
// BaseStatsReport
// ============================================================================

/// Extended base statistics with compatibility and health information.
///
/// This struct is used in `StatusReport` to provide per-base stats including:
/// - Core counts (documents, vectors, files)
/// - On-disk size
/// - Last commit/update time
/// - Embedding and index compatibility status
/// - Overall health state
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BaseStatsReport {
    /// The knowledge base name.
    pub base: String,

    /// Total number of source entries (chunks/documents).
    pub documents: u64,

    /// Total number of vectors in the index.
    pub vectors: u64,

    /// Total number of unique files indexed.
    pub files: u64,

    /// Approximate on-disk size in bytes (sources + index + meta files).
    pub on_disk_bytes: u64,

    /// When the base was last updated (from stats.json.last_updated).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_commit: Option<DateTime<Utc>>,

    /// Embedding model compatibility status.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub embedding_status: Option<String>,

    /// Vector index compatibility status.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub index_status: Option<String>,

    /// Overall health state for the base.
    pub health: BaseHealthState,
}

impl BaseStatsReport {
    /// Create a new base stats report with minimal data.
    pub fn new(base: impl Into<String>) -> Self {
        Self {
            base: base.into(),
            documents: 0,
            vectors: 0,
            files: 0,
            on_disk_bytes: 0,
            last_commit: None,
            embedding_status: None,
            index_status: None,
            health: BaseHealthState::IndexMissing,
        }
    }

    /// Create a report indicating an error state.
    pub fn with_error(base: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            base: base.into(),
            documents: 0,
            vectors: 0,
            files: 0,
            on_disk_bytes: 0,
            last_commit: None,
            embedding_status: Some(message.into()),
            index_status: None,
            health: BaseHealthState::Error,
        }
    }
}

// ============================================================================
// Path Helpers (crate-internal)
// ============================================================================

/// Get the root directory for a knowledge base.
///
/// Returns `.guided/knowledge/<branch>/bases/<base>/`.
pub(crate) fn base_root(knowledge_root: &Path, branch: &str, base: &str) -> PathBuf {
    knowledge_root.join(branch).join("bases").join(base)
}

/// Get the path to the sources JSONL file for a base.
///
/// Returns `.guided/knowledge/<branch>/bases/<base>/sources.jsonl`.
pub(crate) fn sources_path(base_root: &Path) -> PathBuf {
    base_root.join(SOURCES_FILENAME)
}

/// Get the path to the stats JSON file for a base.
///
/// Returns `.guided/knowledge/<branch>/bases/<base>/stats.json`.
pub(crate) fn stats_path(base_root: &Path) -> PathBuf {
    base_root.join(STATS_FILENAME)
}

/// Check if a base directory exists.
///
/// Returns `true` if the base directory exists (even if empty).
/// This is the first-level check before [`is_base_indexed`].
pub fn base_exists(knowledge_root: &Path, branch: &str, base: &str) -> bool {
    base_root(knowledge_root, branch, base).exists()
}

/// Check if a base has indexed content (sources.jsonl exists and is non-empty).
///
/// This is the canonical check used by commands like `reindex`, `ask`, and `stats`
/// to determine if a base has usable content.
///
/// Returns `true` if:
/// 1. The base directory exists, AND
/// 2. The `sources.jsonl` file exists and is non-empty
///
/// # Arguments
///
/// * `knowledge_root` - Path to `.guided/knowledge/`
/// * `branch` - Branch name
/// * `base` - Base name (e.g., "code", "docs", "memory")
pub fn is_base_indexed(knowledge_root: &Path, branch: &str, base: &str) -> bool {
    let base_dir = base_root(knowledge_root, branch, base);
    if !base_dir.exists() {
        return false;
    }
    let sources_file = sources_path(&base_dir);
    if !sources_file.exists() {
        return false;
    }
    // Check if file has content (non-zero size)
    sources_file
        .metadata()
        .map(|m| m.len() > 0)
        .unwrap_or(false)
}

/// List all indexed bases for a branch.
///
/// Returns the names of bases that have indexed content (sources.jsonl exists).
/// This is used by commands that auto-detect available bases.
///
/// # Arguments
///
/// * `knowledge_root` - Path to `.guided/knowledge/`
/// * `branch` - Branch name
pub fn list_indexed_bases(knowledge_root: &Path, branch: &str) -> Vec<String> {
    let bases_dir = knowledge_root.join(branch).join("bases");
    if !bases_dir.is_dir() {
        return Vec::new();
    }

    let mut indexed_bases = Vec::new();
    if let Ok(entries) = fs::read_dir(&bases_dir) {
        for entry in entries.flatten() {
            if entry.path().is_dir() {
                if let Some(name) = entry.file_name().to_str() {
                    // Only include well-known base types that have sources
                    if matches!(name, "code" | "docs" | "memory") {
                        let sources_file = entry.path().join(SOURCES_FILENAME);
                        if sources_file.exists() {
                            // Check non-empty
                            if sources_file.metadata().map(|m| m.len() > 0).unwrap_or(false) {
                                indexed_bases.push(name.to_string());
                            }
                        }
                    }
                }
            }
        }
    }
    indexed_bases.sort();
    indexed_bases
}

// ============================================================================
// I/O Functions
// ============================================================================

/// Load all source entries from a base's sources.jsonl file.
///
/// Returns an empty vector if the file does not exist.
pub fn load_base_sources(path: &Path) -> Result<Vec<BaseSourceEntry>, GikError> {
    if !path.exists() {
        return Ok(Vec::new());
    }

    let file = File::open(path).map_err(|e| GikError::BaseStoreIo {
        path: path.to_path_buf(),
        message: format!("Failed to open: {}", e),
    })?;

    let reader = BufReader::new(file);
    let mut entries = Vec::new();

    for (line_num, line_result) in reader.lines().enumerate() {
        let line = line_result.map_err(|e| GikError::BaseStoreIo {
            path: path.to_path_buf(),
            message: format!("Failed to read line {}: {}", line_num + 1, e),
        })?;

        // Skip empty lines
        if line.trim().is_empty() {
            continue;
        }

        let entry: BaseSourceEntry =
            serde_json::from_str(&line).map_err(|e| GikError::BaseStoreParse {
                path: path.to_path_buf(),
                message: format!("Failed to parse line {}: {}", line_num + 1, e),
            })?;

        entries.push(entry);
    }

    Ok(entries)
}

/// Append source entries to a base's sources.jsonl file.
///
/// Creates the file and parent directories if they don't exist.
pub fn append_base_sources(path: &Path, entries: &[BaseSourceEntry]) -> Result<(), GikError> {
    if entries.is_empty() {
        return Ok(());
    }

    // Ensure parent directories exist
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| GikError::BaseStoreIo {
            path: path.to_path_buf(),
            message: format!("Failed to create directory: {}", e),
        })?;
    }

    // Open file in append mode
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .map_err(|e| GikError::BaseStoreIo {
            path: path.to_path_buf(),
            message: format!("Failed to open: {}", e),
        })?;

    for entry in entries {
        let json_line = serde_json::to_string(entry).map_err(|e| GikError::BaseStoreParse {
            path: path.to_path_buf(),
            message: format!("Failed to serialize entry: {}", e),
        })?;
        writeln!(file, "{}", json_line).map_err(|e| GikError::BaseStoreIo {
            path: path.to_path_buf(),
            message: format!("Failed to write: {}", e),
        })?;
    }

    Ok(())
}

/// Load base stats from a stats.json file.
///
/// Returns `Ok(None)` if the file does not exist.
pub fn load_base_stats(path: &Path) -> Result<Option<BaseStats>, GikError> {
    if !path.exists() {
        return Ok(None);
    }

    let content = fs::read_to_string(path).map_err(|e| GikError::BaseStoreIo {
        path: path.to_path_buf(),
        message: format!("Failed to read: {}", e),
    })?;

    let stats: BaseStats =
        serde_json::from_str(&content).map_err(|e| GikError::BaseStoreParse {
            path: path.to_path_buf(),
            message: format!("Failed to parse: {}", e),
        })?;

    Ok(Some(stats))
}

/// Save base stats to a stats.json file.
///
/// Creates parent directories if they don't exist.
pub fn save_base_stats(path: &Path, stats: &BaseStats) -> Result<(), GikError> {
    // Ensure parent directories exist
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| GikError::BaseStoreIo {
            path: path.to_path_buf(),
            message: format!("Failed to create directory: {}", e),
        })?;
    }

    let content = serde_json::to_string_pretty(stats).map_err(|e| GikError::BaseStoreParse {
        path: path.to_path_buf(),
        message: format!("Failed to serialize: {}", e),
    })?;

    fs::write(path, content).map_err(|e| GikError::BaseStoreIo {
        path: path.to_path_buf(),
        message: format!("Failed to write: {}", e),
    })?;

    Ok(())
}

/// Compute a hash of the file content for deduplication.
pub fn content_hash(content: &str) -> u64 {
    let mut hasher = DefaultHasher::new();
    content.hash(&mut hasher);
    hasher.finish()
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_chunk_id_generate() {
        let id1 = ChunkId::generate("code", "main", "src/main.rs", 12345);
        let id2 = ChunkId::generate("code", "main", "src/main.rs", 12345);
        let id3 = ChunkId::generate("code", "main", "src/lib.rs", 12345);

        assert_eq!(id1, id2, "Same inputs should produce same ID");
        assert_ne!(id1, id3, "Different paths should produce different IDs");
        assert!(id1.as_str().starts_with("chunk-"));
    }

    #[test]
    fn test_base_source_entry_serialization() {
        let entry = BaseSourceEntry::new(
            ChunkId::new("chunk-001"),
            "code",
            "main",
            "src/main.rs",
            1,
            50,
            1,
            "rev-001",
            "src-001",
        )
        .with_text("fn main() { }")
        .with_extra(serde_json::json!({"language": "rust"}));

        let json = serde_json::to_string(&entry).unwrap();
        assert!(json.contains("\"id\":\"chunk-001\""));
        assert!(json.contains("\"filePath\":\"src/main.rs\""));
        assert!(json.contains("\"startLine\":1"));

        let parsed: BaseSourceEntry = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.id.as_str(), "chunk-001");
        assert_eq!(parsed.file_path, "src/main.rs");
    }

    #[test]
    fn test_base_stats_serialization() {
        let mut stats = BaseStats::new("code");
        stats.chunk_count = 100;
        stats.file_count = 10;
        stats.vector_count = 100;

        let json = serde_json::to_string_pretty(&stats).unwrap();
        assert!(json.contains("\"base\": \"code\""));
        assert!(json.contains("\"chunkCount\": 100"));

        let parsed: BaseStats = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.base, "code");
        assert_eq!(parsed.chunk_count, 100);
    }

    #[test]
    fn test_path_helpers() {
        let knowledge_root = PathBuf::from("/workspace/.guided/knowledge");
        let root = base_root(&knowledge_root, "main", "code");
        assert_eq!(
            root,
            PathBuf::from("/workspace/.guided/knowledge/main/bases/code")
        );

        let sources = sources_path(&root);
        assert!(sources.ends_with("sources.jsonl"));

        let stats = stats_path(&root);
        assert!(stats.ends_with("stats.json"));
    }

    #[test]
    fn test_load_base_sources_missing() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("nonexistent.jsonl");

        let sources = load_base_sources(&path).unwrap();
        assert!(sources.is_empty());
    }

    #[test]
    fn test_append_and_load_base_sources() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("sources.jsonl");

        let entry1 = BaseSourceEntry::new(
            ChunkId::new("chunk-001"),
            "code",
            "main",
            "src/main.rs",
            1,
            10,
            1,
            "rev-001",
            "src-001",
        );
        let entry2 = BaseSourceEntry::new(
            ChunkId::new("chunk-002"),
            "code",
            "main",
            "src/lib.rs",
            1,
            20,
            2,
            "rev-001",
            "src-002",
        );

        append_base_sources(&path, &[entry1, entry2]).unwrap();

        let loaded = load_base_sources(&path).unwrap();
        assert_eq!(loaded.len(), 2);
        assert_eq!(loaded[0].id.as_str(), "chunk-001");
        assert_eq!(loaded[1].id.as_str(), "chunk-002");
    }

    #[test]
    fn test_load_base_stats_missing() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("stats.json");

        let stats = load_base_stats(&path).unwrap();
        assert!(stats.is_none());
    }

    #[test]
    fn test_save_and_load_base_stats() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("stats.json");

        let mut stats = BaseStats::new("code");
        stats.chunk_count = 50;
        stats.file_count = 5;
        stats.vector_count = 50;

        save_base_stats(&path, &stats).unwrap();

        let loaded = load_base_stats(&path).unwrap().unwrap();
        assert_eq!(loaded.base, "code");
        assert_eq!(loaded.chunk_count, 50);
        assert_eq!(loaded.file_count, 5);
    }

    #[test]
    fn test_content_hash() {
        let hash1 = content_hash("hello world");
        let hash2 = content_hash("hello world");
        let hash3 = content_hash("different content");

        assert_eq!(hash1, hash2);
        assert_ne!(hash1, hash3);
    }
}
