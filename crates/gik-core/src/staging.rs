//! Staging model and pending sources for GIK.
//!
//! This module provides the staging infrastructure for GIK, which tracks
//! pending sources (files, directories, URLs, archives) that are queued
//! for ingestion into knowledge bases.
//!
//! ## Key Types
//!
//! - [`PendingSourceId`] - Unique identifier for a pending source
//! - [`PendingSourceKind`] - Type of source (file, directory, URL, archive)
//! - [`PendingSourceStatus`] - Processing status of a pending source
//! - [`PendingSource`] - Full pending source record
//! - [`StagingSummary`] - Aggregate statistics about staging
//! - [`NewPendingSource`] - Input type for adding new sources
//!
//! ## On-Disk Format
//!
//! Staging data is stored under `<branch>/staging/`:
//! - `pending.jsonl` - One `PendingSource` per line (JSONL)
//! - `summary.json` - Aggregate `StagingSummary` (JSON)

use std::collections::HashMap;
use std::fmt;
use std::fs::{self, File, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use std::str::FromStr;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::errors::GikError;

// ============================================================================
// ChangeType - for incremental add
// ============================================================================

/// Type of change detected for a file during `gik add`.
///
/// Used to determine whether a file should be staged and to provide
/// git-like status output.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ChangeType {
    /// File is new (not previously indexed).
    New,
    /// File has been modified since last index (mtime or size changed).
    Modified,
    /// File is unchanged since last index (same mtime and size).
    Unchanged,
}

impl std::fmt::Display for ChangeType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::New => write!(f, "new"),
            Self::Modified => write!(f, "modified"),
            Self::Unchanged => write!(f, "unchanged"),
        }
    }
}

// ============================================================================
// PendingSourceId
// ============================================================================

/// Unique identifier for a pending source.
///
/// Generated as a UUID when a source is added to staging.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct PendingSourceId(pub String);

impl PendingSourceId {
    /// Generate a new unique pending source ID.
    pub fn generate() -> Self {
        Self(Uuid::new_v4().to_string())
    }

    /// Create a PendingSourceId from a string without validation.
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    /// Get the inner string value.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for PendingSourceId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl FromStr for PendingSourceId {
    type Err = std::convert::Infallible;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(Self(s.to_string()))
    }
}

// ============================================================================
// PendingSourceKind
// ============================================================================

/// Type of pending source.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PendingSourceKind {
    /// A single file path.
    FilePath,
    /// A directory (will be scanned recursively).
    Directory,
    /// A URL (web page, API endpoint, etc.).
    Url,
    /// An archive file (ZIP, tar, etc.).
    Archive,
    /// Other source type with custom identifier.
    Other(String),
}

impl PendingSourceKind {
    /// Infer the source kind from a URI string.
    ///
    /// - URLs starting with `http://` or `https://` → `Url`
    /// - Paths ending with `.zip`, `.tar`, `.tar.gz`, `.tgz` → `Archive`
    /// - Paths to directories → `Directory`
    /// - Paths to files → `FilePath`
    /// - Unknown → `Other`
    pub fn infer(uri: &str, workspace_root: Option<&Path>) -> Self {
        // Check for URL
        if uri.starts_with("http://") || uri.starts_with("https://") {
            return Self::Url;
        }

        // Check for archive extensions
        let lower = uri.to_lowercase();
        if lower.ends_with(".zip")
            || lower.ends_with(".tar")
            || lower.ends_with(".tar.gz")
            || lower.ends_with(".tgz")
            || lower.ends_with(".tar.bz2")
            || lower.ends_with(".tar.xz")
        {
            return Self::Archive;
        }

        // Check filesystem if workspace root is provided
        if let Some(root) = workspace_root {
            let path = if Path::new(uri).is_absolute() {
                Path::new(uri).to_path_buf()
            } else {
                root.join(uri)
            };

            if path.is_dir() {
                return Self::Directory;
            }
            if path.is_file() {
                return Self::FilePath;
            }
        }

        // Default based on path-like structure or file extension
        // If it has a file extension (contains a dot after any path separators),
        // assume it's meant to be a file path
        let has_extension = uri
            .rsplit('/')
            .next()
            .or_else(|| uri.rsplit('\\').next())
            .unwrap_or(uri)
            .contains('.');

        if uri.contains('/') || uri.contains('\\') || has_extension {
            // Looks like a path, assume file
            Self::FilePath
        } else {
            Self::Other("unknown".to_string())
        }
    }
}

impl fmt::Display for PendingSourceKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::FilePath => write!(f, "filePath"),
            Self::Directory => write!(f, "directory"),
            Self::Url => write!(f, "url"),
            Self::Archive => write!(f, "archive"),
            Self::Other(s) => write!(f, "other:{}", s),
        }
    }
}

// ============================================================================
// PendingSourceStatus
// ============================================================================

/// Processing status of a pending source.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum PendingSourceStatus {
    /// Source is pending processing.
    #[default]
    Pending,
    /// Source is currently being processed.
    Processing,
    /// Source has been successfully indexed.
    Indexed,
    /// Source processing failed.
    Failed,
}

impl fmt::Display for PendingSourceStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Pending => write!(f, "pending"),
            Self::Processing => write!(f, "processing"),
            Self::Indexed => write!(f, "indexed"),
            Self::Failed => write!(f, "failed"),
        }
    }
}

// ============================================================================
// PendingSource
// ============================================================================

/// A pending source in the staging area.
///
/// Represents a file, directory, URL, or archive that has been staged
/// for ingestion into a knowledge base.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PendingSource {
    /// Unique identifier for this pending source.
    pub id: PendingSourceId,

    /// The branch this source belongs to.
    pub branch: String,

    /// Target knowledge base (e.g., "code", "docs", "memory", "kg").
    pub base: String,

    /// Type of source.
    pub kind: PendingSourceKind,

    /// Normalized path or URL.
    pub uri: String,

    /// When the source was added to staging.
    pub added_at: DateTime<Utc>,

    /// Current processing status.
    pub status: PendingSourceStatus,

    /// Type of change (new, modified, unchanged) for incremental staging.
    /// Only set for file sources during `gik add`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub change_type: Option<ChangeType>,

    /// Last error message if status is Failed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_error: Option<String>,

    /// Additional metadata for extensibility.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<serde_json::Value>,
}

// ============================================================================
// NewPendingSource
// ============================================================================

/// Input type for adding a new pending source.
///
/// Callers provide this minimal information; the system generates
/// the ID and timestamps automatically.
#[derive(Debug, Clone)]
pub struct NewPendingSource {
    /// Target knowledge base (e.g., "code", "docs", "memory").
    /// If None, will be inferred from the source kind/extension.
    pub base: Option<String>,

    /// The URI (path or URL) of the source.
    pub uri: String,

    /// Optional explicit kind. If None, will be inferred from URI.
    pub kind: Option<PendingSourceKind>,

    /// Optional change type for incremental staging.
    pub change_type: Option<ChangeType>,

    /// Optional metadata.
    pub metadata: Option<serde_json::Value>,
}

impl NewPendingSource {
    /// Create a new pending source with just a URI.
    ///
    /// Base and kind will be inferred automatically.
    pub fn from_uri(uri: impl Into<String>) -> Self {
        Self {
            base: None,
            uri: uri.into(),
            kind: None,
            change_type: None,
            metadata: None,
        }
    }

    /// Create a new pending source with explicit base and URI.
    pub fn new(base: impl Into<String>, uri: impl Into<String>) -> Self {
        Self {
            base: Some(base.into()),
            uri: uri.into(),
            kind: None,
            change_type: None,
            metadata: None,
        }
    }

    /// Set the source kind explicitly.
    pub fn with_kind(mut self, kind: PendingSourceKind) -> Self {
        self.kind = Some(kind);
        self
    }

    /// Set the change type for incremental staging.
    pub fn with_change_type(mut self, change_type: ChangeType) -> Self {
        self.change_type = Some(change_type);
        self
    }

    /// Set metadata.
    pub fn with_metadata(mut self, metadata: serde_json::Value) -> Self {
        self.metadata = Some(metadata);
        self
    }
}

// ============================================================================
// StagingSummary
// ============================================================================

/// Aggregate statistics about the staging area.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StagingSummary {
    /// Number of sources with status Pending.
    pub pending_count: u64,

    /// Number of sources with status Indexed.
    pub indexed_count: u64,

    /// Number of sources with status Failed.
    pub failed_count: u64,

    /// Count of pending sources per knowledge base.
    pub by_base: HashMap<String, u64>,

    /// When this summary was last computed.
    pub last_updated_at: DateTime<Utc>,
}

impl Default for StagingSummary {
    fn default() -> Self {
        Self {
            pending_count: 0,
            indexed_count: 0,
            failed_count: 0,
            by_base: HashMap::new(),
            last_updated_at: Utc::now(),
        }
    }
}

// ============================================================================
// Base Inference
// ============================================================================

/// Well-known knowledge base names.
pub const BASE_CODE: &str = "code";
pub const BASE_DOCS: &str = "docs";
pub const BASE_MEMORY: &str = "memory";
pub const BASE_KG: &str = "kg";

/// Infer the target knowledge base from a file extension.
///
/// Returns the base name ("code", "docs", etc.) based on common patterns.
pub fn infer_base_from_extension(uri: &str) -> &'static str {
    let lower = uri.to_lowercase();

    // Extract extension
    let ext = lower.rsplit('.').next().unwrap_or("");

    match ext {
        // Code files
        "rs" | "py" | "js" | "ts" | "jsx" | "tsx" | "go" | "java" | "c" | "cpp" | "h" | "hpp"
        | "cs" | "rb" | "php" | "swift" | "kt" | "scala" | "clj" | "ex" | "exs" | "erl" | "hs"
        | "ml" | "fs" | "r" | "jl" | "lua" | "pl" | "pm" | "sh" | "bash" | "zsh" | "fish"
        | "ps1" | "psm1" | "vue" | "svelte" => BASE_CODE,

        // Documentation files
        "md" | "markdown" | "rst" | "txt" | "adoc" | "asciidoc" | "tex" | "latex" | "org"
        | "wiki" | "rtf" | "docx" | "doc" | "odt" | "pdf" => BASE_DOCS,

        // Config/data files (typically go to docs for context)
        "json" | "yaml" | "yml" | "toml" | "xml" | "ini" | "cfg" | "conf" | "env" => BASE_DOCS,

        // Default to code for unknown extensions
        _ => BASE_CODE,
    }
}

/// Infer the target knowledge base from a URI.
///
/// Uses file extension for local paths, defaults to "docs" for URLs.
pub fn infer_base(uri: &str, kind: &PendingSourceKind) -> String {
    match kind {
        PendingSourceKind::Url => BASE_DOCS.to_string(),
        PendingSourceKind::Directory => BASE_CODE.to_string(),
        PendingSourceKind::FilePath | PendingSourceKind::Archive => {
            infer_base_from_extension(uri).to_string()
        }
        PendingSourceKind::Other(_) => BASE_CODE.to_string(),
    }
}

// ============================================================================
// Change Detection for Incremental Add
// ============================================================================

/// Indexed file metadata used for change detection.
#[derive(Debug, Clone)]
pub struct IndexedFileInfo {
    /// File path (workspace-relative).
    pub file_path: String,
    /// Modification time when indexed (Unix timestamp).
    pub indexed_mtime: Option<u64>,
    /// File size when indexed.
    pub indexed_size: Option<u64>,
}

/// Detect whether a file has changed since it was last indexed.
///
/// Compares current file metadata (mtime, size) against stored indexed metadata.
/// Returns `ChangeType::New` if the file wasn't previously indexed.
/// Returns `ChangeType::Modified` if mtime or size differs.
/// Returns `ChangeType::Unchanged` if both match.
///
/// # Arguments
///
/// * `file_path` - Path to the file to check
/// * `indexed_info` - Optional indexed metadata from sources.jsonl
///
/// # Returns
///
/// The detected change type.
pub fn detect_file_change(
    file_path: &Path,
    indexed_info: Option<&IndexedFileInfo>,
) -> std::io::Result<ChangeType> {
    let Some(info) = indexed_info else {
        return Ok(ChangeType::New);
    };

    // Get current file metadata
    let metadata = std::fs::metadata(file_path)?;
    let current_size = metadata.len();
    let current_mtime = metadata
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs());

    // Compare with indexed metadata
    let size_matches = info.indexed_size.map_or(true, |s| s == current_size);
    let mtime_matches = match (current_mtime, info.indexed_mtime) {
        (Some(curr), Some(idx)) => curr == idx,
        _ => true, // If we can't compare mtime, assume unchanged
    };

    if size_matches && mtime_matches {
        Ok(ChangeType::Unchanged)
    } else {
        Ok(ChangeType::Modified)
    }
}

/// Get the current file metadata for indexing.
///
/// Returns (mtime, size) tuple for storing in BaseSourceEntry.
pub fn get_file_metadata(file_path: &Path) -> std::io::Result<(u64, u64)> {
    let metadata = std::fs::metadata(file_path)?;
    let size = metadata.len();
    let mtime = metadata
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs())
        .unwrap_or(0);
    Ok((mtime, size))
}

// ============================================================================
// Staging I/O Functions
// ============================================================================

/// Add a new pending source to staging.
///
/// Generates an ID and timestamp, appends to `pending.jsonl`, and
/// recomputes `summary.json`.
///
/// # Arguments
///
/// * `pending_path` - Path to `pending.jsonl`
/// * `summary_path` - Path to `summary.json`
/// * `branch` - Branch name for the source
/// * `new` - The new source to add
/// * `workspace_root` - Optional workspace root for path resolution
///
/// # Returns
///
/// The ID of the newly added pending source.
pub fn add_pending_source(
    pending_path: &Path,
    summary_path: &Path,
    branch: &str,
    new: NewPendingSource,
    workspace_root: Option<&Path>,
) -> Result<PendingSourceId, GikError> {
    // Ensure parent directories exist
    if let Some(parent) = pending_path.parent() {
        fs::create_dir_all(parent).map_err(|e| {
            GikError::StagingIo(format!(
                "Failed to create staging directory {}: {}",
                parent.display(),
                e
            ))
        })?;
    }

    // Generate ID and timestamp
    let id = PendingSourceId::generate();
    let added_at = Utc::now();

    // Infer kind if not provided
    let kind = new
        .kind
        .unwrap_or_else(|| PendingSourceKind::infer(&new.uri, workspace_root));

    // Infer base if not provided
    let base = new.base.unwrap_or_else(|| infer_base(&new.uri, &kind));

    // Create the pending source
    let source = PendingSource {
        id: id.clone(),
        branch: branch.to_string(),
        base,
        kind,
        uri: new.uri,
        added_at,
        status: PendingSourceStatus::Pending,
        change_type: new.change_type,
        last_error: None,
        metadata: new.metadata,
    };

    // Serialize to JSON line
    let json_line = serde_json::to_string(&source)
        .map_err(|e| GikError::StagingIo(format!("Failed to serialize pending source: {}", e)))?;

    // Append to pending.jsonl
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(pending_path)
        .map_err(|e| {
            GikError::StagingIo(format!("Failed to open {}: {}", pending_path.display(), e))
        })?;

    writeln!(file, "{}", json_line).map_err(|e| {
        GikError::StagingIo(format!(
            "Failed to write to {}: {}",
            pending_path.display(),
            e
        ))
    })?;

    // Recompute and write summary
    let summary = recompute_staging_summary(pending_path)?;
    write_staging_summary(summary_path, &summary)?;

    Ok(id)
}

/// List all pending sources from the staging file.
///
/// Returns an empty vector if the file does not exist.
pub fn list_pending_sources(pending_path: &Path) -> Result<Vec<PendingSource>, GikError> {
    if !pending_path.exists() {
        return Ok(Vec::new());
    }

    let file = File::open(pending_path).map_err(|e| {
        GikError::StagingIo(format!("Failed to open {}: {}", pending_path.display(), e))
    })?;

    let reader = BufReader::new(file);
    let mut sources = Vec::new();

    for (line_num, line_result) in reader.lines().enumerate() {
        let line = line_result.map_err(|e| {
            GikError::StagingIo(format!(
                "Failed to read line {} from {}: {}",
                line_num + 1,
                pending_path.display(),
                e
            ))
        })?;

        // Skip empty lines
        if line.trim().is_empty() {
            continue;
        }

        let source: PendingSource = serde_json::from_str(&line).map_err(|e| {
            GikError::StagingParse(format!(
                "Failed to parse line {} in {}: {}",
                line_num + 1,
                pending_path.display(),
                e
            ))
        })?;

        sources.push(source);
    }

    Ok(sources)
}

/// Recompute the staging summary from pending sources.
pub fn recompute_staging_summary(pending_path: &Path) -> Result<StagingSummary, GikError> {
    let sources = list_pending_sources(pending_path)?;

    let mut pending_count = 0u64;
    let mut indexed_count = 0u64;
    let mut failed_count = 0u64;
    let mut by_base: HashMap<String, u64> = HashMap::new();

    for source in sources {
        match source.status {
            PendingSourceStatus::Pending => pending_count += 1,
            PendingSourceStatus::Processing => pending_count += 1, // Count processing as pending
            PendingSourceStatus::Indexed => indexed_count += 1,
            PendingSourceStatus::Failed => failed_count += 1,
        }

        // Only count pending/processing sources in by_base
        if matches!(
            source.status,
            PendingSourceStatus::Pending | PendingSourceStatus::Processing
        ) {
            *by_base.entry(source.base).or_insert(0) += 1;
        }
    }

    Ok(StagingSummary {
        pending_count,
        indexed_count,
        failed_count,
        by_base,
        last_updated_at: Utc::now(),
    })
}

/// Load the staging summary from disk.
///
/// If the file does not exist, recomputes from `pending.jsonl`.
pub fn load_staging_summary(
    summary_path: &Path,
    pending_path: &Path,
) -> Result<StagingSummary, GikError> {
    if summary_path.exists() {
        let content = fs::read_to_string(summary_path).map_err(|e| {
            GikError::StagingIo(format!("Failed to read {}: {}", summary_path.display(), e))
        })?;

        let summary: StagingSummary = serde_json::from_str(&content).map_err(|e| {
            GikError::StagingParse(format!("Failed to parse {}: {}", summary_path.display(), e))
        })?;

        Ok(summary)
    } else {
        // Recompute from pending.jsonl
        recompute_staging_summary(pending_path)
    }
}

/// Write the staging summary to disk.
fn write_staging_summary(summary_path: &Path, summary: &StagingSummary) -> Result<(), GikError> {
    // Ensure parent directories exist
    if let Some(parent) = summary_path.parent() {
        fs::create_dir_all(parent).map_err(|e| {
            GikError::StagingIo(format!(
                "Failed to create directory {}: {}",
                parent.display(),
                e
            ))
        })?;
    }

    let content = serde_json::to_string_pretty(summary)
        .map_err(|e| GikError::StagingIo(format!("Failed to serialize summary: {}", e)))?;

    fs::write(summary_path, content).map_err(|e| {
        GikError::StagingIo(format!("Failed to write {}: {}", summary_path.display(), e))
    })?;

    Ok(())
}

/// Check if a source with the same (branch, base, uri) is already pending.
///
/// This is used to avoid adding duplicate sources to staging.
///
/// # Arguments
///
/// * `pending_path` - Path to `pending.jsonl`
/// * `branch` - Branch name to check
/// * `base` - Knowledge base name to check
/// * `uri` - URI to check for duplicates
///
/// # Returns
///
/// `true` if a matching pending source exists, `false` otherwise.
pub fn is_source_already_pending(
    pending_path: &Path,
    branch: &str,
    base: &str,
    uri: &str,
) -> Result<bool, GikError> {
    let sources = list_pending_sources(pending_path)?;

    // Phase 8.6: Include Failed status to prevent re-adding files that failed to index.
    // Users should resolve the failure cause (e.g., fix empty file) before re-adding.
    Ok(sources.iter().any(|s| {
        s.branch == branch
            && s.base == base
            && s.uri == uri
            && matches!(
                s.status,
                PendingSourceStatus::Pending
                    | PendingSourceStatus::Processing
                    | PendingSourceStatus::Failed
            )
    }))
}

/// Clear the staging area by removing all pending sources.
///
/// This removes `pending.jsonl` and resets `summary.json`.
pub fn clear_staging(pending_path: &Path, summary_path: &Path) -> Result<(), GikError> {
    // Remove pending.jsonl if it exists
    if pending_path.exists() {
        fs::remove_file(pending_path).map_err(|e| {
            GikError::StagingIo(format!(
                "Failed to remove {}: {}",
                pending_path.display(),
                e
            ))
        })?;
    }

    // Write empty summary
    let empty_summary = StagingSummary::default();
    write_staging_summary(summary_path, &empty_summary)?;

    Ok(())
}

/// Update the status of a pending source.
///
/// Rewrites the entire `pending.jsonl` file with the updated status.
/// Also optionally sets `last_error` if provided.
///
/// # Arguments
///
/// * `pending_path` - Path to `pending.jsonl`
/// * `summary_path` - Path to `summary.json`
/// * `source_id` - ID of the source to update
/// * `new_status` - The new status to set
/// * `error` - Optional error message (for Failed status)
///
/// # Returns
///
/// `true` if the source was found and updated, `false` if not found.
pub fn update_source_status(
    pending_path: &Path,
    summary_path: &Path,
    source_id: &PendingSourceId,
    new_status: PendingSourceStatus,
    error: Option<String>,
) -> Result<bool, GikError> {
    let mut sources = list_pending_sources(pending_path)?;

    let mut found = false;
    for source in &mut sources {
        if &source.id == source_id {
            source.status = new_status.clone();
            source.last_error = error.clone();
            found = true;
            break;
        }
    }

    if !found {
        return Ok(false);
    }

    // Rewrite the entire file
    write_pending_sources(pending_path, &sources)?;

    // Recompute and update summary
    let summary = recompute_staging_summary(pending_path)?;
    write_staging_summary(summary_path, &summary)?;

    Ok(true)
}

/// Update the status of multiple pending sources at once.
///
/// More efficient than calling `update_source_status` multiple times.
///
/// # Arguments
///
/// * `pending_path` - Path to `pending.jsonl`
/// * `summary_path` - Path to `summary.json`
/// * `updates` - Vector of (source_id, new_status, optional_error) tuples
///
/// # Returns
///
/// Number of sources actually updated.
pub fn update_sources_status_batch(
    pending_path: &Path,
    summary_path: &Path,
    updates: &[(PendingSourceId, PendingSourceStatus, Option<String>)],
) -> Result<usize, GikError> {
    let mut sources = list_pending_sources(pending_path)?;

    let mut updated_count = 0;
    for source in &mut sources {
        for (id, status, error) in updates {
            if &source.id == id {
                source.status = status.clone();
                source.last_error = error.clone();
                updated_count += 1;
                break;
            }
        }
    }

    if updated_count > 0 {
        // Rewrite the entire file
        write_pending_sources(pending_path, &sources)?;

        // Recompute and update summary
        let summary = recompute_staging_summary(pending_path)?;
        write_staging_summary(summary_path, &summary)?;
    }

    Ok(updated_count)
}

/// Get pending sources filtered by status.
///
/// # Arguments
///
/// * `pending_path` - Path to `pending.jsonl`
/// * `status` - The status to filter by
///
/// # Returns
///
/// Vector of sources with the specified status.
pub fn filter_sources_by_status(
    pending_path: &Path,
    status: &PendingSourceStatus,
) -> Result<Vec<PendingSource>, GikError> {
    let sources = list_pending_sources(pending_path)?;
    Ok(sources
        .into_iter()
        .filter(|s| &s.status == status)
        .collect())
}

/// Get pending sources filtered by base name.
///
/// # Arguments
///
/// * `pending_path` - Path to `pending.jsonl`
/// * `base` - The base name to filter by
///
/// # Returns
///
/// Vector of sources for the specified base.
pub fn filter_sources_by_base(
    pending_path: &Path,
    base: &str,
) -> Result<Vec<PendingSource>, GikError> {
    let sources = list_pending_sources(pending_path)?;
    Ok(sources.into_iter().filter(|s| s.base == base).collect())
}

/// Get pending sources with "pending" status, grouped by base.
///
/// # Arguments
///
/// * `pending_path` - Path to `pending.jsonl`
///
/// # Returns
///
/// HashMap from base name to vector of pending sources.
pub fn get_pending_by_base(
    pending_path: &Path,
) -> Result<HashMap<String, Vec<PendingSource>>, GikError> {
    let sources = list_pending_sources(pending_path)?;

    let mut by_base: HashMap<String, Vec<PendingSource>> = HashMap::new();
    for source in sources {
        if source.status == PendingSourceStatus::Pending {
            by_base.entry(source.base.clone()).or_default().push(source);
        }
    }

    Ok(by_base)
}

/// Remove all indexed sources from staging.
///
/// Keeps only sources with status Pending, Processing, or Failed.
/// Updates the summary after removal.
///
/// # Arguments
///
/// * `pending_path` - Path to `pending.jsonl`
/// * `summary_path` - Path to `summary.json`
///
/// # Returns
///
/// Number of sources removed.
pub fn clear_indexed_sources(pending_path: &Path, summary_path: &Path) -> Result<usize, GikError> {
    let sources = list_pending_sources(pending_path)?;

    let indexed_count = sources
        .iter()
        .filter(|s| s.status == PendingSourceStatus::Indexed)
        .count();

    if indexed_count == 0 {
        return Ok(0);
    }

    // Keep only non-indexed sources
    let remaining: Vec<_> = sources
        .into_iter()
        .filter(|s| s.status != PendingSourceStatus::Indexed)
        .collect();

    // Rewrite the file
    write_pending_sources(pending_path, &remaining)?;

    // Recompute and update summary
    let summary = recompute_staging_summary(pending_path)?;
    write_staging_summary(summary_path, &summary)?;

    Ok(indexed_count)
}

/// Remove pending sources that match the given URIs.
///
/// Only removes sources with status `Pending` or `Failed` (not `Indexed` or `Processing`).
/// URIs are matched exactly against the stored `uri` field.
///
/// # Arguments
///
/// * `pending_path` - Path to `pending.jsonl`
/// * `summary_path` - Path to `summary.json`
/// * `branch` - Branch name to filter by
/// * `uris` - List of URIs to unstage
///
/// # Returns
///
/// Tuple of (removed_uris, not_found_uris) where:
/// - `removed_uris` are the URIs that were successfully removed
/// - `not_found_uris` are the URIs that were not found in pending sources
pub fn unstage_sources(
    pending_path: &Path,
    summary_path: &Path,
    branch: &str,
    uris: &[String],
) -> Result<(Vec<String>, Vec<String>), GikError> {
    let sources = list_pending_sources(pending_path)?;

    let mut removed_uris: Vec<String> = Vec::new();
    let mut not_found_uris: Vec<String> = Vec::new();

    // Find which URIs exist in pending sources for this branch
    for uri in uris {
        let exists = sources.iter().any(|s| {
            s.branch == branch
                && s.uri == *uri
                && matches!(
                    s.status,
                    PendingSourceStatus::Pending | PendingSourceStatus::Failed
                )
        });

        if exists {
            removed_uris.push(uri.clone());
        } else {
            not_found_uris.push(uri.clone());
        }
    }

    // If nothing to remove, return early
    if removed_uris.is_empty() {
        return Ok((removed_uris, not_found_uris));
    }

    // Filter out the sources to remove
    let remaining: Vec<_> = sources
        .into_iter()
        .filter(|s| {
            // Keep sources that don't match removal criteria
            !(s.branch == branch
                && removed_uris.contains(&s.uri)
                && matches!(
                    s.status,
                    PendingSourceStatus::Pending | PendingSourceStatus::Failed
                ))
        })
        .collect();

    // Rewrite the file
    write_pending_sources(pending_path, &remaining)?;

    // Recompute and update summary
    let summary = recompute_staging_summary(pending_path)?;
    write_staging_summary(summary_path, &summary)?;

    Ok((removed_uris, not_found_uris))
}

/// Write all pending sources to a file (overwrites existing content).
fn write_pending_sources(pending_path: &Path, sources: &[PendingSource]) -> Result<(), GikError> {
    // Ensure parent directories exist
    if let Some(parent) = pending_path.parent() {
        fs::create_dir_all(parent).map_err(|e| {
            GikError::StagingIo(format!(
                "Failed to create directory {}: {}",
                parent.display(),
                e
            ))
        })?;
    }

    let mut file = File::create(pending_path).map_err(|e| {
        GikError::StagingIo(format!(
            "Failed to create {}: {}",
            pending_path.display(),
            e
        ))
    })?;

    for source in sources {
        let json_line = serde_json::to_string(source)
            .map_err(|e| GikError::StagingIo(format!("Failed to serialize source: {}", e)))?;
        writeln!(file, "{}", json_line).map_err(|e| {
            GikError::StagingIo(format!(
                "Failed to write to {}: {}",
                pending_path.display(),
                e
            ))
        })?;
    }

    Ok(())
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_pending_source_id_generate() {
        let id1 = PendingSourceId::generate();
        let id2 = PendingSourceId::generate();

        assert_ne!(id1.as_str(), id2.as_str());
        assert!(!id1.as_str().is_empty());
    }

    #[test]
    fn test_pending_source_id_display_and_from_str() {
        let id = PendingSourceId::new("test-id-123");
        assert_eq!(id.to_string(), "test-id-123");

        let parsed: PendingSourceId = "test-id-123".parse().unwrap();
        assert_eq!(parsed.as_str(), "test-id-123");
    }

    #[test]
    fn test_pending_source_kind_infer_url() {
        assert_eq!(
            PendingSourceKind::infer("https://example.com/docs", None),
            PendingSourceKind::Url
        );
        assert_eq!(
            PendingSourceKind::infer("http://localhost:8080/api", None),
            PendingSourceKind::Url
        );
    }

    #[test]
    fn test_pending_source_kind_infer_archive() {
        assert_eq!(
            PendingSourceKind::infer("backup.zip", None),
            PendingSourceKind::Archive
        );
        assert_eq!(
            PendingSourceKind::infer("data.tar.gz", None),
            PendingSourceKind::Archive
        );
        assert_eq!(
            PendingSourceKind::infer("archive.tgz", None),
            PendingSourceKind::Archive
        );
    }

    #[test]
    fn test_pending_source_kind_infer_with_filesystem() {
        let temp = TempDir::new().unwrap();

        // Create a file
        let file_path = temp.path().join("test.rs");
        fs::write(&file_path, "fn main() {}").unwrap();

        // Create a directory
        let dir_path = temp.path().join("src");
        fs::create_dir(&dir_path).unwrap();

        assert_eq!(
            PendingSourceKind::infer("test.rs", Some(temp.path())),
            PendingSourceKind::FilePath
        );
        assert_eq!(
            PendingSourceKind::infer("src", Some(temp.path())),
            PendingSourceKind::Directory
        );
    }

    #[test]
    fn test_infer_base_from_extension() {
        // Code files
        assert_eq!(infer_base_from_extension("main.rs"), BASE_CODE);
        assert_eq!(infer_base_from_extension("app.py"), BASE_CODE);
        assert_eq!(infer_base_from_extension("index.ts"), BASE_CODE);

        // Documentation files
        assert_eq!(infer_base_from_extension("README.md"), BASE_DOCS);
        assert_eq!(infer_base_from_extension("guide.txt"), BASE_DOCS);
        assert_eq!(infer_base_from_extension("spec.pdf"), BASE_DOCS);

        // Config files go to docs
        assert_eq!(infer_base_from_extension("config.yaml"), BASE_DOCS);
        assert_eq!(infer_base_from_extension("settings.json"), BASE_DOCS);
    }

    #[test]
    fn test_infer_base() {
        // URLs default to docs
        assert_eq!(
            infer_base("https://example.com", &PendingSourceKind::Url),
            BASE_DOCS
        );

        // Directories default to code
        assert_eq!(infer_base("src/", &PendingSourceKind::Directory), BASE_CODE);

        // Files use extension
        assert_eq!(
            infer_base("main.rs", &PendingSourceKind::FilePath),
            BASE_CODE
        );
        assert_eq!(
            infer_base("README.md", &PendingSourceKind::FilePath),
            BASE_DOCS
        );
    }

    #[test]
    fn test_add_and_list_pending_sources() {
        let temp = TempDir::new().unwrap();
        let pending_path = temp.path().join("staging/pending.jsonl");
        let summary_path = temp.path().join("staging/summary.json");

        // Add first source
        let new1 = NewPendingSource::from_uri("src/main.rs");
        let id1 = add_pending_source(&pending_path, &summary_path, "main", new1, None).unwrap();

        // Add second source
        let new2 = NewPendingSource::new("docs", "README.md");
        let id2 = add_pending_source(&pending_path, &summary_path, "main", new2, None).unwrap();

        // List sources
        let sources = list_pending_sources(&pending_path).unwrap();
        assert_eq!(sources.len(), 2);

        assert_eq!(sources[0].id.as_str(), id1.as_str());
        assert_eq!(sources[0].uri, "src/main.rs");
        assert_eq!(sources[0].base, BASE_CODE);
        assert_eq!(sources[0].status, PendingSourceStatus::Pending);

        assert_eq!(sources[1].id.as_str(), id2.as_str());
        assert_eq!(sources[1].uri, "README.md");
        assert_eq!(sources[1].base, BASE_DOCS);
    }

    #[test]
    fn test_staging_summary() {
        let temp = TempDir::new().unwrap();
        let pending_path = temp.path().join("staging/pending.jsonl");
        let summary_path = temp.path().join("staging/summary.json");

        // Add sources to different bases
        add_pending_source(
            &pending_path,
            &summary_path,
            "main",
            NewPendingSource::new("code", "src/lib.rs"),
            None,
        )
        .unwrap();

        add_pending_source(
            &pending_path,
            &summary_path,
            "main",
            NewPendingSource::new("code", "src/main.rs"),
            None,
        )
        .unwrap();

        add_pending_source(
            &pending_path,
            &summary_path,
            "main",
            NewPendingSource::new("docs", "README.md"),
            None,
        )
        .unwrap();

        // Load summary
        let summary = load_staging_summary(&summary_path, &pending_path).unwrap();

        assert_eq!(summary.pending_count, 3);
        assert_eq!(summary.indexed_count, 0);
        assert_eq!(summary.failed_count, 0);
        assert_eq!(summary.by_base.get("code"), Some(&2));
        assert_eq!(summary.by_base.get("docs"), Some(&1));
    }

    #[test]
    fn test_list_pending_sources_empty() {
        let temp = TempDir::new().unwrap();
        let pending_path = temp.path().join("staging/pending.jsonl");

        // Should return empty vec when file doesn't exist
        let sources = list_pending_sources(&pending_path).unwrap();
        assert!(sources.is_empty());
    }

    #[test]
    fn test_clear_staging() {
        let temp = TempDir::new().unwrap();
        let pending_path = temp.path().join("staging/pending.jsonl");
        let summary_path = temp.path().join("staging/summary.json");

        // Add a source
        add_pending_source(
            &pending_path,
            &summary_path,
            "main",
            NewPendingSource::from_uri("test.rs"),
            None,
        )
        .unwrap();

        // Verify it exists
        let sources = list_pending_sources(&pending_path).unwrap();
        assert_eq!(sources.len(), 1);

        // Clear staging
        clear_staging(&pending_path, &summary_path).unwrap();

        // Verify it's empty
        let sources = list_pending_sources(&pending_path).unwrap();
        assert!(sources.is_empty());

        // Summary should be reset
        let summary = load_staging_summary(&summary_path, &pending_path).unwrap();
        assert_eq!(summary.pending_count, 0);
    }

    #[test]
    fn test_new_pending_source_builder() {
        let new = NewPendingSource::from_uri("test.rs")
            .with_kind(PendingSourceKind::FilePath)
            .with_metadata(serde_json::json!({"custom": "value"}));

        assert_eq!(new.uri, "test.rs");
        assert_eq!(new.kind, Some(PendingSourceKind::FilePath));
        assert!(new.metadata.is_some());
    }

    #[test]
    fn test_pending_source_serialization() {
        let source = PendingSource {
            id: PendingSourceId::new("test-123"),
            branch: "main".to_string(),
            base: "code".to_string(),
            kind: PendingSourceKind::FilePath,
            uri: "src/main.rs".to_string(),
            added_at: Utc::now(),
            status: PendingSourceStatus::Pending,
            change_type: None,
            last_error: None,
            metadata: None,
        };

        let json = serde_json::to_string(&source).unwrap();
        assert!(json.contains("\"id\":\"test-123\""));
        assert!(json.contains("\"kind\":\"filePath\""));
        assert!(json.contains("\"status\":\"pending\""));

        // Roundtrip
        let parsed: PendingSource = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.id.as_str(), "test-123");
        assert_eq!(parsed.kind, PendingSourceKind::FilePath);
    }

    #[test]
    fn test_update_source_status() {
        let temp = TempDir::new().unwrap();
        let pending_path = temp.path().join("staging/pending.jsonl");
        let summary_path = temp.path().join("staging/summary.json");

        // Add a source
        let id = add_pending_source(
            &pending_path,
            &summary_path,
            "main",
            NewPendingSource::from_uri("test.rs"),
            None,
        )
        .unwrap();

        // Update status to Indexed
        let updated = update_source_status(
            &pending_path,
            &summary_path,
            &id,
            PendingSourceStatus::Indexed,
            None,
        )
        .unwrap();
        assert!(updated);

        // Verify update
        let sources = list_pending_sources(&pending_path).unwrap();
        assert_eq!(sources[0].status, PendingSourceStatus::Indexed);

        // Summary should reflect the change
        let summary = load_staging_summary(&summary_path, &pending_path).unwrap();
        assert_eq!(summary.pending_count, 0);
        assert_eq!(summary.indexed_count, 1);
    }

    #[test]
    fn test_update_source_status_with_error() {
        let temp = TempDir::new().unwrap();
        let pending_path = temp.path().join("staging/pending.jsonl");
        let summary_path = temp.path().join("staging/summary.json");

        // Add a source
        let id = add_pending_source(
            &pending_path,
            &summary_path,
            "main",
            NewPendingSource::from_uri("test.rs"),
            None,
        )
        .unwrap();

        // Update status to Failed with error
        update_source_status(
            &pending_path,
            &summary_path,
            &id,
            PendingSourceStatus::Failed,
            Some("File too large".to_string()),
        )
        .unwrap();

        // Verify update
        let sources = list_pending_sources(&pending_path).unwrap();
        assert_eq!(sources[0].status, PendingSourceStatus::Failed);
        assert_eq!(sources[0].last_error, Some("File too large".to_string()));
    }

    #[test]
    fn test_filter_sources_by_status() {
        let temp = TempDir::new().unwrap();
        let pending_path = temp.path().join("staging/pending.jsonl");
        let summary_path = temp.path().join("staging/summary.json");

        // Add multiple sources
        let id1 = add_pending_source(
            &pending_path,
            &summary_path,
            "main",
            NewPendingSource::from_uri("test1.rs"),
            None,
        )
        .unwrap();
        add_pending_source(
            &pending_path,
            &summary_path,
            "main",
            NewPendingSource::from_uri("test2.rs"),
            None,
        )
        .unwrap();

        // Mark one as indexed
        update_source_status(
            &pending_path,
            &summary_path,
            &id1,
            PendingSourceStatus::Indexed,
            None,
        )
        .unwrap();

        // Filter by pending status
        let pending =
            filter_sources_by_status(&pending_path, &PendingSourceStatus::Pending).unwrap();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].uri, "test2.rs");

        // Filter by indexed status
        let indexed =
            filter_sources_by_status(&pending_path, &PendingSourceStatus::Indexed).unwrap();
        assert_eq!(indexed.len(), 1);
        assert_eq!(indexed[0].uri, "test1.rs");
    }

    #[test]
    fn test_get_pending_by_base() {
        let temp = TempDir::new().unwrap();
        let pending_path = temp.path().join("staging/pending.jsonl");
        let summary_path = temp.path().join("staging/summary.json");

        // Add sources to different bases
        add_pending_source(
            &pending_path,
            &summary_path,
            "main",
            NewPendingSource::new("code", "src/main.rs"),
            None,
        )
        .unwrap();
        add_pending_source(
            &pending_path,
            &summary_path,
            "main",
            NewPendingSource::new("code", "src/lib.rs"),
            None,
        )
        .unwrap();
        add_pending_source(
            &pending_path,
            &summary_path,
            "main",
            NewPendingSource::new("docs", "README.md"),
            None,
        )
        .unwrap();

        // Get pending by base
        let by_base = get_pending_by_base(&pending_path).unwrap();
        assert_eq!(by_base.get("code").map(|v| v.len()), Some(2));
        assert_eq!(by_base.get("docs").map(|v| v.len()), Some(1));
    }

    #[test]
    fn test_clear_indexed_sources() {
        let temp = TempDir::new().unwrap();
        let pending_path = temp.path().join("staging/pending.jsonl");
        let summary_path = temp.path().join("staging/summary.json");

        // Add multiple sources
        let id1 = add_pending_source(
            &pending_path,
            &summary_path,
            "main",
            NewPendingSource::from_uri("test1.rs"),
            None,
        )
        .unwrap();
        add_pending_source(
            &pending_path,
            &summary_path,
            "main",
            NewPendingSource::from_uri("test2.rs"),
            None,
        )
        .unwrap();

        // Mark one as indexed
        update_source_status(
            &pending_path,
            &summary_path,
            &id1,
            PendingSourceStatus::Indexed,
            None,
        )
        .unwrap();

        // Clear indexed sources
        let removed = clear_indexed_sources(&pending_path, &summary_path).unwrap();
        assert_eq!(removed, 1);

        // Only one source should remain
        let sources = list_pending_sources(&pending_path).unwrap();
        assert_eq!(sources.len(), 1);
        assert_eq!(sources[0].uri, "test2.rs");
    }

    #[test]
    fn test_unstage_sources_basic() {
        let temp = TempDir::new().unwrap();
        let pending_path = temp.path().join("staging/pending.jsonl");
        let summary_path = temp.path().join("staging/summary.json");

        // Add multiple sources
        add_pending_source(
            &pending_path,
            &summary_path,
            "main",
            NewPendingSource::from_uri("src/main.rs"),
            None,
        )
        .unwrap();
        add_pending_source(
            &pending_path,
            &summary_path,
            "main",
            NewPendingSource::from_uri("src/lib.rs"),
            None,
        )
        .unwrap();
        add_pending_source(
            &pending_path,
            &summary_path,
            "main",
            NewPendingSource::from_uri("README.md"),
            None,
        )
        .unwrap();

        // Unstage one source
        let (removed, not_found) = unstage_sources(
            &pending_path,
            &summary_path,
            "main",
            &["src/main.rs".to_string()],
        )
        .unwrap();

        assert_eq!(removed.len(), 1);
        assert_eq!(removed[0], "src/main.rs");
        assert!(not_found.is_empty());

        // Verify only two sources remain
        let sources = list_pending_sources(&pending_path).unwrap();
        assert_eq!(sources.len(), 2);
        assert!(sources.iter().all(|s| s.uri != "src/main.rs"));
    }

    #[test]
    fn test_unstage_sources_multiple() {
        let temp = TempDir::new().unwrap();
        let pending_path = temp.path().join("staging/pending.jsonl");
        let summary_path = temp.path().join("staging/summary.json");

        // Add sources
        add_pending_source(
            &pending_path,
            &summary_path,
            "main",
            NewPendingSource::from_uri("a.rs"),
            None,
        )
        .unwrap();
        add_pending_source(
            &pending_path,
            &summary_path,
            "main",
            NewPendingSource::from_uri("b.rs"),
            None,
        )
        .unwrap();
        add_pending_source(
            &pending_path,
            &summary_path,
            "main",
            NewPendingSource::from_uri("c.rs"),
            None,
        )
        .unwrap();

        // Unstage two sources
        let (removed, not_found) = unstage_sources(
            &pending_path,
            &summary_path,
            "main",
            &["a.rs".to_string(), "c.rs".to_string()],
        )
        .unwrap();

        assert_eq!(removed.len(), 2);
        assert!(removed.contains(&"a.rs".to_string()));
        assert!(removed.contains(&"c.rs".to_string()));
        assert!(not_found.is_empty());

        // Verify only one source remains
        let sources = list_pending_sources(&pending_path).unwrap();
        assert_eq!(sources.len(), 1);
        assert_eq!(sources[0].uri, "b.rs");
    }

    #[test]
    fn test_unstage_sources_not_found() {
        let temp = TempDir::new().unwrap();
        let pending_path = temp.path().join("staging/pending.jsonl");
        let summary_path = temp.path().join("staging/summary.json");

        // Add one source
        add_pending_source(
            &pending_path,
            &summary_path,
            "main",
            NewPendingSource::from_uri("exists.rs"),
            None,
        )
        .unwrap();

        // Try to unstage a source that doesn't exist
        let (removed, not_found) = unstage_sources(
            &pending_path,
            &summary_path,
            "main",
            &["not_exists.rs".to_string()],
        )
        .unwrap();

        assert!(removed.is_empty());
        assert_eq!(not_found.len(), 1);
        assert_eq!(not_found[0], "not_exists.rs");

        // Original source should still be there
        let sources = list_pending_sources(&pending_path).unwrap();
        assert_eq!(sources.len(), 1);
        assert_eq!(sources[0].uri, "exists.rs");
    }

    #[test]
    fn test_unstage_sources_mixed() {
        let temp = TempDir::new().unwrap();
        let pending_path = temp.path().join("staging/pending.jsonl");
        let summary_path = temp.path().join("staging/summary.json");

        // Add sources
        add_pending_source(
            &pending_path,
            &summary_path,
            "main",
            NewPendingSource::from_uri("exists.rs"),
            None,
        )
        .unwrap();

        // Unstage one that exists and one that doesn't
        let (removed, not_found) = unstage_sources(
            &pending_path,
            &summary_path,
            "main",
            &["exists.rs".to_string(), "not_exists.rs".to_string()],
        )
        .unwrap();

        assert_eq!(removed.len(), 1);
        assert_eq!(removed[0], "exists.rs");
        assert_eq!(not_found.len(), 1);
        assert_eq!(not_found[0], "not_exists.rs");

        // Should be empty now
        let sources = list_pending_sources(&pending_path).unwrap();
        assert!(sources.is_empty());
    }

    #[test]
    fn test_unstage_sources_ignores_indexed() {
        let temp = TempDir::new().unwrap();
        let pending_path = temp.path().join("staging/pending.jsonl");
        let summary_path = temp.path().join("staging/summary.json");

        // Add source
        let id = add_pending_source(
            &pending_path,
            &summary_path,
            "main",
            NewPendingSource::from_uri("indexed.rs"),
            None,
        )
        .unwrap();

        // Mark as indexed
        update_source_status(
            &pending_path,
            &summary_path,
            &id,
            PendingSourceStatus::Indexed,
            None,
        )
        .unwrap();

        // Try to unstage - should not find it since it's Indexed
        let (removed, not_found) = unstage_sources(
            &pending_path,
            &summary_path,
            "main",
            &["indexed.rs".to_string()],
        )
        .unwrap();

        assert!(removed.is_empty());
        assert_eq!(not_found.len(), 1);
        assert_eq!(not_found[0], "indexed.rs");

        // Source should still be there (just not removable via unstage)
        let sources = list_pending_sources(&pending_path).unwrap();
        assert_eq!(sources.len(), 1);
    }

    #[test]
    fn test_unstage_sources_removes_failed() {
        let temp = TempDir::new().unwrap();
        let pending_path = temp.path().join("staging/pending.jsonl");
        let summary_path = temp.path().join("staging/summary.json");

        // Add source
        let id = add_pending_source(
            &pending_path,
            &summary_path,
            "main",
            NewPendingSource::from_uri("failed.rs"),
            None,
        )
        .unwrap();

        // Mark as failed
        update_source_status(
            &pending_path,
            &summary_path,
            &id,
            PendingSourceStatus::Failed,
            Some("embedding error".to_string()),
        )
        .unwrap();

        // Unstage - should work for Failed status
        let (removed, not_found) = unstage_sources(
            &pending_path,
            &summary_path,
            "main",
            &["failed.rs".to_string()],
        )
        .unwrap();

        assert_eq!(removed.len(), 1);
        assert_eq!(removed[0], "failed.rs");
        assert!(not_found.is_empty());

        // Source should be removed
        let sources = list_pending_sources(&pending_path).unwrap();
        assert!(sources.is_empty());
    }

    #[test]
    fn test_unstage_sources_different_branch() {
        let temp = TempDir::new().unwrap();
        let pending_path = temp.path().join("staging/pending.jsonl");
        let summary_path = temp.path().join("staging/summary.json");

        // Add sources on different branches
        add_pending_source(
            &pending_path,
            &summary_path,
            "main",
            NewPendingSource::from_uri("main.rs"),
            None,
        )
        .unwrap();
        add_pending_source(
            &pending_path,
            &summary_path,
            "feature",
            NewPendingSource::from_uri("feature.rs"),
            None,
        )
        .unwrap();

        // Unstage from main branch
        let (removed, not_found) = unstage_sources(
            &pending_path,
            &summary_path,
            "main",
            &["main.rs".to_string(), "feature.rs".to_string()],
        )
        .unwrap();

        // Only main.rs should be removed (feature.rs is on different branch)
        assert_eq!(removed.len(), 1);
        assert_eq!(removed[0], "main.rs");
        assert_eq!(not_found.len(), 1);
        assert_eq!(not_found[0], "feature.rs");

        // feature.rs should still be there
        let sources = list_pending_sources(&pending_path).unwrap();
        assert_eq!(sources.len(), 1);
        assert_eq!(sources[0].uri, "feature.rs");
        assert_eq!(sources[0].branch, "feature");
    }
}
