//! Log and timeline query for GIK.
//!
//! This module provides read-only access to GIK's knowledge history:
//! - Timeline entries (Commit, Reindex, Init, Release operations)
//! - Ask log entries (query history)
//!
//! The log query pipeline supports filtering by:
//! - Branch
//! - Operation kind (Commit, Reindex, etc.)
//! - Base names
//! - Time range
//! - Entry limit
//!
//! ## Usage
//!
//! ```ignore
//! use gik_core::log::{LogQueryScope, run_log_query, LogKind};
//!
//! let scope = LogQueryScope::default()
//!     .with_kind(LogKind::Timeline)
//!     .with_limit(10);
//!
//! let result = engine.log_query(&workspace, scope)?;
//! for entry in result.entries {
//!     println!("{:?}", entry);
//! }
//! ```

use std::fs::File;
use std::io::{BufRead, BufReader, Write};
use std::path::Path;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::engine::GikEngine;
use crate::errors::GikError;
use crate::timeline::{read_timeline, Revision, RevisionOperation};
use crate::workspace::Workspace;

// ============================================================================
// LogKind
// ============================================================================

/// The kind of log to query.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LogKind {
    /// Branch timeline (Commit, Reindex, Init, Release revisions).
    #[default]
    Timeline,
    /// Ask log entries (query history).
    Ask,
}

impl std::fmt::Display for LogKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Timeline => write!(f, "timeline"),
            Self::Ask => write!(f, "ask"),
        }
    }
}

impl std::str::FromStr for LogKind {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "timeline" => Ok(Self::Timeline),
            "ask" => Ok(Self::Ask),
            other => Err(format!("Unknown log kind: {}", other)),
        }
    }
}

// ============================================================================
// TimelineOperationKind
// ============================================================================

/// The kind of timeline operation to filter by.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TimelineOperationKind {
    /// Workspace initialization.
    Init,
    /// Commit of staged sources.
    Commit,
    /// Memory ingestion (decisions, notes, observations).
    MemoryIngest,
    /// Memory pruning (removal or archival of entries).
    MemoryPrune,
    /// Reindex of a base.
    Reindex,
    /// Release with optional tag.
    Release,
    /// Custom or unknown operation.
    Other(String),
}

impl TimelineOperationKind {
    /// Convert from a RevisionOperation to TimelineOperationKind.
    pub fn from_revision_operation(op: &RevisionOperation) -> Self {
        match op {
            RevisionOperation::Init => Self::Init,
            RevisionOperation::Commit { .. } => Self::Commit,
            RevisionOperation::MemoryIngest { .. } => Self::MemoryIngest,
            RevisionOperation::MemoryPrune { .. } => Self::MemoryPrune,
            RevisionOperation::Reindex { .. } => Self::Reindex,
            RevisionOperation::Release { .. } => Self::Release,
            RevisionOperation::Custom { name, .. } => Self::Other(name.clone()),
        }
    }

    /// Check if this kind matches a RevisionOperation.
    pub fn matches(&self, op: &RevisionOperation) -> bool {
        match (self, op) {
            (Self::Init, RevisionOperation::Init) => true,
            (Self::Commit, RevisionOperation::Commit { .. }) => true,
            (Self::MemoryIngest, RevisionOperation::MemoryIngest { .. }) => true,
            (Self::MemoryPrune, RevisionOperation::MemoryPrune { .. }) => true,
            (Self::Reindex, RevisionOperation::Reindex { .. }) => true,
            (Self::Release, RevisionOperation::Release { .. }) => true,
            (Self::Other(name), RevisionOperation::Custom { name: op_name, .. }) => name == op_name,
            _ => false,
        }
    }
}

impl std::fmt::Display for TimelineOperationKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Init => write!(f, "init"),
            Self::Commit => write!(f, "commit"),
            Self::MemoryIngest => write!(f, "memory_ingest"),
            Self::MemoryPrune => write!(f, "memory_prune"),
            Self::Reindex => write!(f, "reindex"),
            Self::Release => write!(f, "release"),
            Self::Other(name) => write!(f, "{}", name),
        }
    }
}

impl std::str::FromStr for TimelineOperationKind {
    type Err = std::convert::Infallible;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(match s.to_lowercase().as_str() {
            "init" => Self::Init,
            "commit" => Self::Commit,
            "memory_ingest" | "memoryingest" => Self::MemoryIngest,
            "memory_prune" | "memoryprune" => Self::MemoryPrune,
            "reindex" => Self::Reindex,
            "release" => Self::Release,
            other => Self::Other(other.to_string()),
        })
    }
}

// ============================================================================
// LogQueryScope
// ============================================================================

/// Scope and filters for a log query.
#[derive(Debug, Clone, Default)]
pub struct LogQueryScope {
    /// Branch to query (None = current branch).
    pub branch: Option<String>,
    /// Kind of log to query (None = Timeline by default).
    pub kind: Option<LogKind>,
    /// Filter by timeline operation kinds (None = all operations).
    pub timeline_ops: Option<Vec<TimelineOperationKind>>,
    /// Filter by base names (None = all bases).
    pub bases: Option<Vec<String>>,
    /// Filter entries since this time (inclusive).
    pub since: Option<DateTime<Utc>>,
    /// Filter entries until this time (inclusive).
    pub until: Option<DateTime<Utc>>,
    /// Maximum number of entries to return.
    pub limit: Option<usize>,
}

impl LogQueryScope {
    /// Create a new empty scope (defaults to timeline, current branch).
    pub fn new() -> Self {
        Self::default()
    }

    /// Set the branch to query.
    pub fn with_branch(mut self, branch: impl Into<String>) -> Self {
        self.branch = Some(branch.into());
        self
    }

    /// Set the log kind.
    pub fn with_kind(mut self, kind: LogKind) -> Self {
        self.kind = Some(kind);
        self
    }

    /// Set the timeline operation filter.
    pub fn with_ops(mut self, ops: Vec<TimelineOperationKind>) -> Self {
        self.timeline_ops = Some(ops);
        self
    }

    /// Set the base filter.
    pub fn with_bases(mut self, bases: Vec<String>) -> Self {
        self.bases = Some(bases);
        self
    }

    /// Set the since filter.
    pub fn with_since(mut self, since: DateTime<Utc>) -> Self {
        self.since = Some(since);
        self
    }

    /// Set the until filter.
    pub fn with_until(mut self, until: DateTime<Utc>) -> Self {
        self.until = Some(until);
        self
    }

    /// Set the limit.
    pub fn with_limit(mut self, limit: usize) -> Self {
        self.limit = Some(limit);
        self
    }
}

// ============================================================================
// TimelineLogEntry
// ============================================================================

/// A single entry from the timeline log.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TimelineLogEntry {
    /// Branch this entry belongs to.
    pub branch: String,
    /// Timestamp of the operation.
    pub timestamp: DateTime<Utc>,
    /// The kind of operation.
    pub operation: TimelineOperationKind,
    /// The revision ID.
    pub revision_id: String,
    /// Bases affected by this operation (if any).
    pub bases: Vec<String>,
    /// Commit/revision message (if any).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    /// Extra metadata as JSON.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<serde_json::Value>,
}

impl TimelineLogEntry {
    /// Create a TimelineLogEntry from a Revision.
    pub fn from_revision(revision: &Revision) -> Self {
        let (operation, bases, meta) = Self::extract_operation_info(&revision.operations);

        Self {
            branch: revision.branch.clone(),
            timestamp: revision.timestamp,
            operation,
            revision_id: revision.id.to_string(),
            bases,
            message: if revision.message.is_empty() {
                None
            } else {
                Some(revision.message.clone())
            },
            meta,
        }
    }

    /// Extract operation info from revision operations.
    fn extract_operation_info(
        operations: &[RevisionOperation],
    ) -> (
        TimelineOperationKind,
        Vec<String>,
        Option<serde_json::Value>,
    ) {
        // Take the first operation as the primary one
        let op = operations.first();

        match op {
            Some(RevisionOperation::Init) => (TimelineOperationKind::Init, vec![], None),
            Some(RevisionOperation::Commit {
                bases,
                source_count,
            }) => (
                TimelineOperationKind::Commit,
                bases.clone(),
                Some(serde_json::json!({
                    "sourceCount": source_count
                })),
            ),
            Some(RevisionOperation::MemoryIngest { count }) => (
                TimelineOperationKind::MemoryIngest,
                vec!["memory".to_string()],
                Some(serde_json::json!({
                    "count": count
                })),
            ),
            Some(RevisionOperation::MemoryPrune {
                count,
                archived_count,
                deleted_count,
            }) => (
                TimelineOperationKind::MemoryPrune,
                vec!["memory".to_string()],
                Some(serde_json::json!({
                    "count": count,
                    "archivedCount": archived_count,
                    "deletedCount": deleted_count
                })),
            ),
            Some(RevisionOperation::Reindex {
                base,
                from_model_id,
                to_model_id,
            }) => (
                TimelineOperationKind::Reindex,
                vec![base.clone()],
                Some(serde_json::json!({
                    "fromModelId": from_model_id,
                    "toModelId": to_model_id
                })),
            ),
            Some(RevisionOperation::Release { tag }) => (
                TimelineOperationKind::Release,
                vec![],
                tag.as_ref().map(|t| serde_json::json!({ "tag": t })),
            ),
            Some(RevisionOperation::Custom { name, data }) => (
                TimelineOperationKind::Other(name.clone()),
                vec![],
                data.clone(),
            ),
            None => (
                TimelineOperationKind::Other("unknown".to_string()),
                vec![],
                None,
            ),
        }
    }
}

// ============================================================================
// AskLogEntry (stored format)
// ============================================================================

/// An entry in the ask log (stored format in ask.log.jsonl).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AskLogEntry {
    /// Timestamp of the query.
    pub timestamp: DateTime<Utc>,
    /// The branch used for the query.
    pub branch: String,
    /// The question asked.
    pub question: String,
    /// Bases that were queried.
    pub bases: Vec<String>,
    /// Total number of chunks returned.
    pub total_hits: u32,
    /// Path to the full context bundle (if saved).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bundle_path: Option<String>,
}

impl AskLogEntry {
    /// Create a new AskLogEntry.
    pub fn new(
        branch: impl Into<String>,
        question: impl Into<String>,
        bases: Vec<String>,
        total_hits: u32,
    ) -> Self {
        Self {
            timestamp: Utc::now(),
            branch: branch.into(),
            question: question.into(),
            bases,
            total_hits,
            bundle_path: None,
        }
    }

    /// Set the bundle path.
    pub fn with_bundle_path(mut self, path: impl Into<String>) -> Self {
        self.bundle_path = Some(path.into());
        self
    }
}

// ============================================================================
// AskLogView
// ============================================================================

/// A view of an ask log entry for query results.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AskLogView {
    /// Branch the query was run on.
    pub branch: String,
    /// Timestamp of the query.
    pub timestamp: DateTime<Utc>,
    /// The question asked.
    pub question: String,
    /// Bases that were queried.
    pub bases: Vec<String>,
    /// Total number of chunks returned.
    pub total_hits: u32,
    /// Path to the full context bundle (if saved).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bundle_path: Option<String>,
}

impl From<AskLogEntry> for AskLogView {
    fn from(entry: AskLogEntry) -> Self {
        Self {
            branch: entry.branch,
            timestamp: entry.timestamp,
            question: entry.question,
            bases: entry.bases,
            total_hits: entry.total_hits,
            bundle_path: entry.bundle_path,
        }
    }
}

// ============================================================================
// LogEntry
// ============================================================================

/// A unified log entry (timeline or ask).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum LogEntry {
    /// A timeline entry (Commit, Reindex, etc.).
    Timeline(TimelineLogEntry),
    /// An ask log entry.
    Ask(AskLogView),
}

impl LogEntry {
    /// Get the timestamp of this entry.
    pub fn timestamp(&self) -> DateTime<Utc> {
        match self {
            Self::Timeline(e) => e.timestamp,
            Self::Ask(e) => e.timestamp,
        }
    }

    /// Get the branch of this entry.
    pub fn branch(&self) -> &str {
        match self {
            Self::Timeline(e) => &e.branch,
            Self::Ask(e) => &e.branch,
        }
    }
}

// ============================================================================
// LogQueryResult
// ============================================================================

/// Result of a log query.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LogQueryResult {
    /// The entries matching the query.
    pub entries: Vec<LogEntry>,
}

impl LogQueryResult {
    /// Create an empty result.
    pub fn empty() -> Self {
        Self { entries: vec![] }
    }

    /// Create a result with entries.
    pub fn with_entries(entries: Vec<LogEntry>) -> Self {
        Self { entries }
    }
}

// ============================================================================
// Ask Log Path Constants
// ============================================================================

/// The filename for the ask log.
pub const ASK_LOG_FILENAME: &str = "ask.log.jsonl";

/// The directory for ask-related files.
pub const ASKS_DIR: &str = "asks";

// ============================================================================
// Core Query Function
// ============================================================================

/// Run a log query against the workspace.
///
/// # Arguments
///
/// * `engine` - The GIK engine (for branch resolution).
/// * `workspace` - The workspace to query.
/// * `scope` - The query scope and filters.
///
/// # Returns
///
/// A `LogQueryResult` containing matching entries.
pub fn run_log_query(
    engine: &GikEngine,
    workspace: &Workspace,
    scope: LogQueryScope,
) -> Result<LogQueryResult, GikError> {
    if !workspace.is_initialized() {
        return Ok(LogQueryResult::empty());
    }

    // Resolve branch
    let branch = match &scope.branch {
        Some(b) => b.clone(),
        None => engine.current_branch(workspace)?.to_string(),
    };

    // Determine log kind (default to Timeline)
    let kind = scope.kind.unwrap_or(LogKind::Timeline);

    let entries = match kind {
        LogKind::Timeline => query_timeline(workspace, &branch, &scope)?,
        LogKind::Ask => query_ask_log(workspace, &scope)?,
    };

    Ok(LogQueryResult::with_entries(entries))
}

/// Query the timeline for a branch.
fn query_timeline(
    workspace: &Workspace,
    branch: &str,
    scope: &LogQueryScope,
) -> Result<Vec<LogEntry>, GikError> {
    let timeline_path = workspace.timeline_path(branch);

    if !timeline_path.exists() {
        return Ok(vec![]);
    }

    let revisions = read_timeline(&timeline_path)?;

    // Convert to log entries
    let mut entries: Vec<LogEntry> = revisions
        .iter()
        .map(|r| LogEntry::Timeline(TimelineLogEntry::from_revision(r)))
        .collect();

    // Enrich Commit/Reindex entries with per-base stats (Phase 6.2)
    enrich_timeline_entries_with_stats(&mut entries, workspace, branch);

    // Apply filters
    entries = apply_timeline_filters(entries, scope);

    // Sort by timestamp (newest first)
    entries.sort_by_key(|e| std::cmp::Reverse(e.timestamp()));

    // Apply limit
    if let Some(limit) = scope.limit {
        entries.truncate(limit);
    }

    Ok(entries)
}

/// Enrich timeline entries with per-base stats for Commit and Reindex operations.
///
/// For each entry that has bases, this reads the current stats.json and adds
/// a compact summary to the meta field.
fn enrich_timeline_entries_with_stats(
    entries: &mut [LogEntry],
    workspace: &Workspace,
    branch: &str,
) {
    for entry in entries.iter_mut() {
        if let LogEntry::Timeline(timeline_entry) = entry {
            // Only enrich Commit and Reindex operations
            if !matches!(
                timeline_entry.operation,
                TimelineOperationKind::Commit | TimelineOperationKind::Reindex
            ) {
                continue;
            }

            // Skip if no bases
            if timeline_entry.bases.is_empty() {
                continue;
            }

            // Build compact base stats
            let base_stats: Vec<serde_json::Value> = timeline_entry
                .bases
                .iter()
                .map(|base_name| {
                    let base_dir = workspace.branch_dir(branch).join("bases").join(base_name);
                    let stats_path = base_dir.join("stats.json");

                    // Try to load stats
                    if let Ok(Some(stats)) = crate::base::load_base_stats(&stats_path) {
                        serde_json::json!({
                            "base": base_name,
                            "documents": stats.chunk_count,
                            "vectors": stats.vector_count,
                            "files": stats.file_count
                        })
                    } else {
                        // Base stats not available; still include base name
                        serde_json::json!({
                            "base": base_name,
                            "documents": 0,
                            "vectors": 0,
                            "files": 0
                        })
                    }
                })
                .collect();

            // Merge into existing meta or create new
            if let Some(ref mut meta) = timeline_entry.meta {
                if let Some(obj) = meta.as_object_mut() {
                    obj.insert("baseStats".to_string(), serde_json::json!(base_stats));
                }
            } else {
                timeline_entry.meta = Some(serde_json::json!({
                    "baseStats": base_stats
                }));
            }
        }
    }
}

/// Apply timeline-specific filters.
fn apply_timeline_filters(entries: Vec<LogEntry>, scope: &LogQueryScope) -> Vec<LogEntry> {
    entries
        .into_iter()
        .filter(|entry| {
            if let LogEntry::Timeline(t) = entry {
                // Filter by operation kind
                if let Some(ref ops) = scope.timeline_ops {
                    let matches = ops.iter().any(|op| {
                        std::mem::discriminant(op) == std::mem::discriminant(&t.operation)
                            || matches!(
                                (&t.operation, op),
                                (
                                    TimelineOperationKind::Other(a),
                                    TimelineOperationKind::Other(b)
                                ) if a == b
                            )
                    });
                    if !matches {
                        return false;
                    }
                }

                // Filter by bases
                if let Some(ref bases) = scope.bases {
                    if !t.bases.is_empty() {
                        let has_match = t.bases.iter().any(|b| bases.contains(b));
                        if !has_match {
                            return false;
                        }
                    }
                }

                // Filter by time range
                if let Some(since) = scope.since {
                    if t.timestamp < since {
                        return false;
                    }
                }
                if let Some(until) = scope.until {
                    if t.timestamp > until {
                        return false;
                    }
                }

                true
            } else {
                false
            }
        })
        .collect()
}

/// Query the ask log.
fn query_ask_log(workspace: &Workspace, scope: &LogQueryScope) -> Result<Vec<LogEntry>, GikError> {
    let ask_log_path = workspace
        .knowledge_root()
        .join(ASKS_DIR)
        .join(ASK_LOG_FILENAME);

    if !ask_log_path.exists() {
        return Ok(vec![]);
    }

    let entries = load_ask_log(&ask_log_path)?;

    // Convert to log entries
    let mut log_entries: Vec<LogEntry> = entries
        .into_iter()
        .map(|e| LogEntry::Ask(AskLogView::from(e)))
        .collect();

    // Apply filters
    log_entries = apply_ask_filters(log_entries, scope);

    // Sort by timestamp (newest first)
    log_entries.sort_by_key(|e| std::cmp::Reverse(e.timestamp()));

    // Apply limit
    if let Some(limit) = scope.limit {
        log_entries.truncate(limit);
    }

    Ok(log_entries)
}

/// Load ask log entries from file.
fn load_ask_log(path: &Path) -> Result<Vec<AskLogEntry>, GikError> {
    let file = File::open(path).map_err(|e| GikError::LogIoError {
        path: path.to_path_buf(),
        reason: e.to_string(),
    })?;

    let reader = BufReader::new(file);
    let mut entries = Vec::new();

    for (line_num, line_result) in reader.lines().enumerate() {
        let line = line_result.map_err(|e| GikError::LogIoError {
            path: path.to_path_buf(),
            reason: format!("Failed to read line {}: {}", line_num + 1, e),
        })?;

        if line.trim().is_empty() {
            continue;
        }

        let entry: AskLogEntry = serde_json::from_str(&line).map_err(|e| GikError::LogIoError {
            path: path.to_path_buf(),
            reason: format!("Failed to parse line {}: {}", line_num + 1, e),
        })?;

        entries.push(entry);
    }

    Ok(entries)
}

/// Apply ask-specific filters.
fn apply_ask_filters(entries: Vec<LogEntry>, scope: &LogQueryScope) -> Vec<LogEntry> {
    entries
        .into_iter()
        .filter(|entry| {
            if let LogEntry::Ask(a) = entry {
                // Filter by branch
                if let Some(ref branch) = scope.branch {
                    if a.branch != *branch {
                        return false;
                    }
                }

                // Filter by bases
                if let Some(ref bases) = scope.bases {
                    let has_match = a.bases.iter().any(|b| bases.contains(b));
                    if !has_match {
                        return false;
                    }
                }

                // Filter by time range
                if let Some(since) = scope.since {
                    if a.timestamp < since {
                        return false;
                    }
                }
                if let Some(until) = scope.until {
                    if a.timestamp > until {
                        return false;
                    }
                }

                true
            } else {
                false
            }
        })
        .collect()
}

/// Append an ask log entry to the ask log file.
///
/// Creates the asks directory and file if they don't exist.
pub fn append_ask_log(workspace: &Workspace, entry: &AskLogEntry) -> Result<(), GikError> {
    let asks_dir = workspace.knowledge_root().join(ASKS_DIR);
    std::fs::create_dir_all(&asks_dir).map_err(|e| GikError::LogIoError {
        path: asks_dir.clone(),
        reason: format!("Failed to create asks directory: {}", e),
    })?;

    let ask_log_path = asks_dir.join(ASK_LOG_FILENAME);
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&ask_log_path)
        .map_err(|e| GikError::LogIoError {
            path: ask_log_path.clone(),
            reason: format!("Failed to open ask log: {}", e),
        })?;

    let json = serde_json::to_string(entry).map_err(|e| GikError::LogIoError {
        path: ask_log_path.clone(),
        reason: format!("Failed to serialize ask log entry: {}", e),
    })?;

    writeln!(file, "{}", json).map_err(|e| GikError::LogIoError {
        path: ask_log_path,
        reason: format!("Failed to write ask log entry: {}", e),
    })?;

    Ok(())
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::timeline::{append_revision, Revision, RevisionId, RevisionOperation};
    use tempfile::TempDir;

    fn create_test_workspace() -> (TempDir, Workspace) {
        let temp_dir = TempDir::new().unwrap();
        let workspace = Workspace::from_root(temp_dir.path()).unwrap();

        // Create knowledge structure
        let branch_dir = workspace.knowledge_root().join("main");
        std::fs::create_dir_all(&branch_dir).unwrap();

        (temp_dir, workspace)
    }

    fn create_test_revision(branch: &str, message: &str, ops: Vec<RevisionOperation>) -> Revision {
        Revision {
            id: RevisionId::generate(),
            parent_id: None,
            branch: branch.to_string(),
            git_commit: None,
            timestamp: Utc::now(),
            message: message.to_string(),
            operations: ops,
        }
    }

    #[test]
    fn test_log_kind_from_str() {
        assert_eq!("timeline".parse::<LogKind>().unwrap(), LogKind::Timeline);
        assert_eq!("ask".parse::<LogKind>().unwrap(), LogKind::Ask);
        assert!("invalid".parse::<LogKind>().is_err());
    }

    #[test]
    fn test_timeline_operation_kind_from_str() {
        assert_eq!(
            "commit".parse::<TimelineOperationKind>().unwrap(),
            TimelineOperationKind::Commit
        );
        assert_eq!(
            "REINDEX".parse::<TimelineOperationKind>().unwrap(),
            TimelineOperationKind::Reindex
        );
        assert_eq!(
            "custom_op".parse::<TimelineOperationKind>().unwrap(),
            TimelineOperationKind::Other("custom_op".to_string())
        );
    }

    #[test]
    fn test_timeline_log_entry_from_revision() {
        let revision = create_test_revision(
            "main",
            "Initial commit",
            vec![RevisionOperation::Commit {
                bases: vec!["code".to_string()],
                source_count: 5,
            }],
        );

        let entry = TimelineLogEntry::from_revision(&revision);
        assert_eq!(entry.branch, "main");
        assert_eq!(entry.operation, TimelineOperationKind::Commit);
        assert_eq!(entry.bases, vec!["code".to_string()]);
        assert_eq!(entry.message, Some("Initial commit".to_string()));
    }

    #[test]
    fn test_log_query_scope_builder() {
        let scope = LogQueryScope::new()
            .with_branch("main")
            .with_kind(LogKind::Timeline)
            .with_ops(vec![TimelineOperationKind::Commit])
            .with_limit(10);

        assert_eq!(scope.branch, Some("main".to_string()));
        assert_eq!(scope.kind, Some(LogKind::Timeline));
        assert_eq!(scope.limit, Some(10));
    }

    #[test]
    fn test_query_empty_timeline() {
        let (_temp_dir, workspace) = create_test_workspace();
        let scope = LogQueryScope::new().with_branch("main");

        let entries = query_timeline(&workspace, "main", &scope).unwrap();
        assert!(entries.is_empty());
    }

    #[test]
    fn test_query_timeline_with_revisions() {
        let (_temp_dir, workspace) = create_test_workspace();
        let timeline_path = workspace.timeline_path("main");

        // Add some revisions
        let rev1 = create_test_revision("main", "Init", vec![RevisionOperation::Init]);
        let rev2 = create_test_revision(
            "main",
            "Commit 1",
            vec![RevisionOperation::Commit {
                bases: vec!["code".to_string()],
                source_count: 3,
            }],
        );

        append_revision(&timeline_path, &rev1).unwrap();
        append_revision(&timeline_path, &rev2).unwrap();

        let scope = LogQueryScope::new().with_branch("main");
        let entries = query_timeline(&workspace, "main", &scope).unwrap();

        assert_eq!(entries.len(), 2);
    }

    #[test]
    fn test_query_timeline_with_op_filter() {
        let (_temp_dir, workspace) = create_test_workspace();
        let timeline_path = workspace.timeline_path("main");

        let rev1 = create_test_revision("main", "Init", vec![RevisionOperation::Init]);
        let rev2 = create_test_revision(
            "main",
            "Commit 1",
            vec![RevisionOperation::Commit {
                bases: vec!["code".to_string()],
                source_count: 3,
            }],
        );

        append_revision(&timeline_path, &rev1).unwrap();
        append_revision(&timeline_path, &rev2).unwrap();

        // Filter by Commit only
        let scope = LogQueryScope::new()
            .with_branch("main")
            .with_ops(vec![TimelineOperationKind::Commit]);

        let entries = query_timeline(&workspace, "main", &scope).unwrap();

        assert_eq!(entries.len(), 1);
        if let LogEntry::Timeline(t) = &entries[0] {
            assert_eq!(t.operation, TimelineOperationKind::Commit);
        } else {
            panic!("Expected Timeline entry");
        }
    }

    #[test]
    fn test_query_timeline_with_limit() {
        let (_temp_dir, workspace) = create_test_workspace();
        let timeline_path = workspace.timeline_path("main");

        for i in 0..5 {
            let rev = create_test_revision(
                "main",
                &format!("Commit {}", i),
                vec![RevisionOperation::Commit {
                    bases: vec!["code".to_string()],
                    source_count: i,
                }],
            );
            append_revision(&timeline_path, &rev).unwrap();
        }

        let scope = LogQueryScope::new().with_branch("main").with_limit(2);

        let entries = query_timeline(&workspace, "main", &scope).unwrap();
        assert_eq!(entries.len(), 2);
    }

    #[test]
    fn test_ask_log_entry_serialization() {
        let entry = AskLogEntry {
            timestamp: Utc::now(),
            branch: "main".to_string(),
            question: "What is this?".to_string(),
            bases: vec!["code".to_string()],
            total_hits: 5,
            bundle_path: Some("/path/to/bundle.json".to_string()),
        };

        let json = serde_json::to_string(&entry).unwrap();
        assert!(json.contains("question"));
        assert!(json.contains("What is this?"));

        let parsed: AskLogEntry = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.question, entry.question);
    }

    #[test]
    fn test_append_and_load_ask_log() {
        let (_temp_dir, workspace) = create_test_workspace();

        let entry1 = AskLogEntry::new("main", "Question 1?", vec!["code".to_string()], 3);
        let entry2 = AskLogEntry::new("main", "Question 2?", vec!["docs".to_string()], 5);

        append_ask_log(&workspace, &entry1).unwrap();
        append_ask_log(&workspace, &entry2).unwrap();

        let ask_log_path = workspace
            .knowledge_root()
            .join(ASKS_DIR)
            .join(ASK_LOG_FILENAME);
        let entries = load_ask_log(&ask_log_path).unwrap();

        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].question, "Question 1?");
        assert_eq!(entries[1].question, "Question 2?");
    }

    #[test]
    fn test_log_entry_unified_type() {
        let timeline_entry = LogEntry::Timeline(TimelineLogEntry {
            branch: "main".to_string(),
            timestamp: Utc::now(),
            operation: TimelineOperationKind::Commit,
            revision_id: "rev-123".to_string(),
            bases: vec!["code".to_string()],
            message: Some("Test".to_string()),
            meta: None,
        });

        let ask_entry = LogEntry::Ask(AskLogView {
            branch: "main".to_string(),
            timestamp: Utc::now(),
            question: "Test?".to_string(),
            bases: vec!["docs".to_string()],
            total_hits: 3,
            bundle_path: None,
        });

        assert_eq!(timeline_entry.branch(), "main");
        assert_eq!(ask_entry.branch(), "main");
    }
}
