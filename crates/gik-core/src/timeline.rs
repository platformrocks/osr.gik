//! Timeline and revision management for GIK.
//!
//! This module provides types and functions for managing the revision timeline
//! and HEAD pointer for each branch. The timeline is stored as a JSONL file
//! with one revision per line.

use std::fs::{self, File, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use std::str::FromStr;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::errors::GikError;

// ============================================================================
// RevisionId
// ============================================================================

/// A unique identifier for a revision.
///
/// Revision IDs are UUIDs stored as strings.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct RevisionId(String);

impl RevisionId {
    /// Create a new revision ID from a string.
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    /// Generate a new unique revision ID using UUID v4.
    pub fn generate() -> Self {
        Self(Uuid::new_v4().to_string())
    }

    /// Get the revision ID as a string slice.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for RevisionId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl FromStr for RevisionId {
    type Err = std::convert::Infallible;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(Self(s.to_string()))
    }
}

impl AsRef<str> for RevisionId {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

// ============================================================================
// RevisionOperation
// ============================================================================

/// An operation recorded in a revision.
///
/// Each revision can contain multiple operations that describe what
/// changes were made in that revision.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum RevisionOperation {
    /// Workspace initialization.
    Init,

    /// Commit of staged sources and memory.
    Commit {
        /// Bases that were updated.
        bases: Vec<String>,
        /// Number of source files indexed.
        #[serde(rename = "sourceCount")]
        source_count: usize,
    },

    /// Memory ingestion operation.
    ///
    /// Records the addition of memory entries (decisions, notes, observations)
    /// to the `memory` base. This is distinct from `Commit` to allow filtering
    /// and grouping memory events in logs and reports.
    MemoryIngest {
        /// Number of memory entries successfully persisted.
        count: usize,
    },

    /// Memory pruning operation.
    ///
    /// Records the removal or archival of memory entries based on a pruning
    /// policy. This is tracked separately from `MemoryIngest` for audit and
    /// changelog purposes.
    MemoryPrune {
        /// Total number of entries that were pruned.
        count: usize,
        /// Number of entries that were archived (mode = Archive).
        #[serde(rename = "archivedCount")]
        archived_count: usize,
        /// Number of entries that were permanently deleted (mode = Delete).
        #[serde(rename = "deletedCount")]
        deleted_count: usize,
    },

    /// Reindex of a base with a different embedding model.
    Reindex {
        /// The base that was reindexed.
        base: String,
        /// Previous embedding model ID.
        #[serde(rename = "fromModelId")]
        from_model_id: String,
        /// New embedding model ID.
        #[serde(rename = "toModelId")]
        to_model_id: String,
    },

    /// Release revision with optional tag.
    Release {
        /// Release tag (e.g., "v1.0.0").
        tag: Option<String>,
    },

    /// Custom operation for extensibility.
    Custom {
        /// Operation name.
        name: String,
        /// Additional data as JSON.
        data: Option<serde_json::Value>,
    },
}

// ============================================================================
// Revision
// ============================================================================

/// A revision in the timeline.
///
/// Revisions form a linked list via `parent_id`, representing the history
/// of operations performed on a branch.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Revision {
    /// Unique revision ID.
    pub id: RevisionId,

    /// Parent revision ID (None for the first revision).
    #[serde(rename = "parentId")]
    pub parent_id: Option<RevisionId>,

    /// Branch this revision belongs to.
    pub branch: String,

    /// Associated Git commit hash (if available).
    #[serde(rename = "gitCommit")]
    pub git_commit: Option<String>,

    /// Timestamp of the revision.
    pub timestamp: DateTime<Utc>,

    /// Human-readable message describing the revision.
    pub message: String,

    /// Operations performed in this revision.
    pub operations: Vec<RevisionOperation>,
}

impl Revision {
    /// Create a new revision with the current timestamp.
    pub fn new(
        branch: impl Into<String>,
        parent_id: Option<RevisionId>,
        message: impl Into<String>,
        operations: Vec<RevisionOperation>,
    ) -> Self {
        Self {
            id: RevisionId::generate(),
            parent_id,
            branch: branch.into(),
            git_commit: None,
            timestamp: Utc::now(),
            message: message.into(),
            operations,
        }
    }

    /// Create a new revision with a specific ID (for testing).
    pub fn with_id(
        id: RevisionId,
        branch: impl Into<String>,
        parent_id: Option<RevisionId>,
        message: impl Into<String>,
        operations: Vec<RevisionOperation>,
    ) -> Self {
        Self {
            id,
            parent_id,
            branch: branch.into(),
            git_commit: None,
            timestamp: Utc::now(),
            message: message.into(),
            operations,
        }
    }

    /// Create an Init revision for a new branch.
    pub fn init(branch: impl Into<String>) -> Self {
        Self::new(
            branch,
            None,
            "Initialize GIK workspace",
            vec![RevisionOperation::Init],
        )
    }

    /// Set the Git commit hash for this revision.
    pub fn with_git_commit(mut self, commit: impl Into<String>) -> Self {
        self.git_commit = Some(commit.into());
        self
    }
}

// ============================================================================
// Timeline I/O
// ============================================================================

/// Append a revision to a timeline file.
///
/// Creates the file and parent directories if they don't exist.
/// The revision is serialized as a single JSON line.
///
/// # Errors
///
/// Returns [`GikError::TimelineWrite`] if the file cannot be written.
pub fn append_revision(path: &Path, revision: &Revision) -> Result<(), GikError> {
    // Ensure parent directories exist
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| {
            GikError::TimelineWrite(format!("Failed to create timeline directory: {}", e))
        })?;
    }

    // Open file in append mode
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .map_err(|e| GikError::TimelineWrite(format!("Failed to open timeline: {}", e)))?;

    // Serialize and write
    let line = serde_json::to_string(revision)
        .map_err(|e| GikError::TimelineWrite(format!("Failed to serialize revision: {}", e)))?;

    writeln!(file, "{}", line)
        .map_err(|e| GikError::TimelineWrite(format!("Failed to write revision: {}", e)))?;

    file.flush()
        .map_err(|e| GikError::TimelineWrite(format!("Failed to flush timeline: {}", e)))?;

    Ok(())
}

/// Read all revisions from a timeline file.
///
/// Returns an empty vector if the file doesn't exist.
///
/// # Errors
///
/// Returns [`GikError::TimelineParse`] if a line cannot be parsed.
pub fn read_timeline(path: &Path) -> Result<Vec<Revision>, GikError> {
    if !path.exists() {
        return Ok(Vec::new());
    }

    let file = File::open(path)
        .map_err(|e| GikError::TimelineRead(format!("Failed to open timeline: {}", e)))?;

    let reader = BufReader::new(file);
    let mut revisions = Vec::new();

    for (line_num, line_result) in reader.lines().enumerate() {
        let line = line_result.map_err(|e| {
            GikError::TimelineRead(format!("Failed to read line {}: {}", line_num + 1, e))
        })?;

        // Skip empty lines
        if line.trim().is_empty() {
            continue;
        }

        let revision: Revision = serde_json::from_str(&line).map_err(|e| {
            GikError::TimelineParse(format!("Failed to parse line {}: {}", line_num + 1, e))
        })?;

        revisions.push(revision);
    }

    Ok(revisions)
}

/// Get the last revision from a timeline file.
///
/// Returns `None` if the timeline is empty or doesn't exist.
///
/// # Errors
///
/// Returns an error if the timeline cannot be read or parsed.
pub fn last_revision(path: &Path) -> Result<Option<Revision>, GikError> {
    let revisions = read_timeline(path)?;
    Ok(revisions.into_iter().last())
}

/// Get a specific revision by ID from a timeline file.
///
/// Returns `None` if the revision is not found.
pub fn get_revision(path: &Path, id: &RevisionId) -> Result<Option<Revision>, GikError> {
    let revisions = read_timeline(path)?;
    Ok(revisions.into_iter().find(|r| r.id == *id))
}

// ============================================================================
// HEAD I/O
// ============================================================================

/// Read the HEAD revision ID from a file.
///
/// Returns `None` if the file doesn't exist or is empty.
///
/// # Errors
///
/// Returns [`GikError::HeadRead`] if the file cannot be read.
pub fn read_head(path: &Path) -> Result<Option<RevisionId>, GikError> {
    if !path.exists() {
        return Ok(None);
    }

    let content = fs::read_to_string(path)
        .map_err(|e| GikError::HeadRead(format!("Failed to read HEAD: {}", e)))?;

    let id = content.trim();
    if id.is_empty() {
        return Ok(None);
    }

    Ok(Some(RevisionId::new(id)))
}

/// Write a revision ID to the HEAD file.
///
/// Creates the file and parent directories if they don't exist.
///
/// # Errors
///
/// Returns [`GikError::HeadWrite`] if the file cannot be written.
pub fn write_head(path: &Path, id: &RevisionId) -> Result<(), GikError> {
    // Ensure parent directories exist
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|e| GikError::HeadWrite(format!("Failed to create HEAD directory: {}", e)))?;
    }

    fs::write(path, format!("{}\n", id))
        .map_err(|e| GikError::HeadWrite(format!("Failed to write HEAD: {}", e)))?;

    Ok(())
}

// ============================================================================
// Revision Reference Resolution
// ============================================================================

/// Resolve a revision reference to a concrete RevisionId.
///
/// Supports the following reference formats:
/// - `HEAD` or `@` – resolves to the current HEAD revision
/// - `HEAD~N` or `@~N` – resolves to the Nth ancestor of HEAD (N >= 1)
/// - Full UUID – looks up the exact revision id
/// - UUID prefix (6+ chars) – looks up the first matching revision
///
/// # Arguments
///
/// * `timeline_path` - Path to the timeline.jsonl file
/// * `head_path` - Path to the HEAD file
/// * `ref_str` - The revision reference string to resolve
///
/// # Returns
///
/// The resolved [`RevisionId`], or an error if not found.
///
/// # Errors
///
/// Returns [`GikError::RevisionNotFound`] if the reference cannot be resolved.
pub fn resolve_revision_ref(
    timeline_path: &Path,
    head_path: &Path,
    ref_str: &str,
) -> Result<RevisionId, GikError> {
    let ref_str = ref_str.trim();

    // Handle HEAD / @ aliases
    if ref_str.eq_ignore_ascii_case("HEAD") || ref_str == "@" {
        return read_head(head_path)?.ok_or_else(|| {
            GikError::RevisionNotFound("HEAD not found (timeline may be empty)".to_string())
        });
    }

    // Handle HEAD~N / @~N ancestor syntax
    if let Some(ancestor_str) = ref_str
        .strip_prefix("HEAD~")
        .or_else(|| ref_str.strip_prefix("@~"))
    {
        let n: usize = ancestor_str.parse().map_err(|_| {
            GikError::RevisionNotFound(format!(
                "Invalid ancestor syntax '{}': expected a number after ~",
                ref_str
            ))
        })?;

        if n == 0 {
            // HEAD~0 is just HEAD
            return read_head(head_path)?.ok_or_else(|| {
                GikError::RevisionNotFound("HEAD not found (timeline may be empty)".to_string())
            });
        }

        // Get HEAD and walk back N steps
        let head_id = read_head(head_path)?.ok_or_else(|| {
            GikError::RevisionNotFound("HEAD not found (timeline may be empty)".to_string())
        })?;

        return resolve_ancestor(timeline_path, &head_id, n);
    }

    // Try exact match first
    let revisions = read_timeline(timeline_path)?;

    // Exact match by full ID
    if let Some(rev) = revisions.iter().find(|r| r.id.as_str() == ref_str) {
        return Ok(rev.id.clone());
    }

    // Prefix match (minimum 6 characters for safety)
    if ref_str.len() >= 6 {
        let matches: Vec<_> = revisions
            .iter()
            .filter(|r| r.id.as_str().starts_with(ref_str))
            .collect();

        match matches.len() {
            0 => {}
            1 => return Ok(matches[0].id.clone()),
            _ => {
                return Err(GikError::RevisionNotFound(format!(
                    "Ambiguous revision prefix '{}': matches {} revisions",
                    ref_str,
                    matches.len()
                )));
            }
        }
    }

    Err(GikError::RevisionNotFound(format!(
        "Revision not found: '{}'",
        ref_str
    )))
}

/// Resolve the Nth ancestor of a given revision.
///
/// Walks back the parent chain N steps.
fn resolve_ancestor(
    timeline_path: &Path,
    start_id: &RevisionId,
    steps: usize,
) -> Result<RevisionId, GikError> {
    let revisions = read_timeline(timeline_path)?;

    // Build a lookup map: id -> revision
    let rev_map: std::collections::HashMap<&str, &Revision> =
        revisions.iter().map(|r| (r.id.as_str(), r)).collect();

    let mut current_id = start_id.as_str();

    for step in 0..steps {
        let rev = rev_map.get(current_id).ok_or_else(|| {
            GikError::RevisionNotFound(format!(
                "Revision '{}' not found while resolving ancestor",
                current_id
            ))
        })?;

        current_id = rev.parent_id.as_ref().map(|id| id.as_str()).ok_or_else(|| {
            GikError::RevisionNotFound(format!(
                "Cannot resolve ancestor ~{}: revision '{}' has no parent (reached root after {} steps)",
                steps,
                current_id,
                step
            ))
        })?;
    }

    Ok(RevisionId::new(current_id))
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    // ------------------------------------------------------------------------
    // RevisionId tests
    // ------------------------------------------------------------------------

    #[test]
    fn test_revision_id_generate() {
        let id1 = RevisionId::generate();
        let id2 = RevisionId::generate();
        assert_ne!(id1, id2);
        assert!(!id1.as_str().is_empty());
    }

    #[test]
    fn test_revision_id_from_str() {
        let id: RevisionId = "abc123".parse().unwrap();
        assert_eq!(id.as_str(), "abc123");
    }

    #[test]
    fn test_revision_id_display() {
        let id = RevisionId::new("test-id");
        assert_eq!(format!("{}", id), "test-id");
    }

    // ------------------------------------------------------------------------
    // RevisionOperation tests
    // ------------------------------------------------------------------------

    #[test]
    fn test_revision_operation_init_serialization() {
        let op = RevisionOperation::Init;
        let json = serde_json::to_string(&op).unwrap();
        assert_eq!(json, r#"{"type":"Init"}"#);
    }

    #[test]
    fn test_revision_operation_commit_serialization() {
        let op = RevisionOperation::Commit {
            bases: vec!["code".to_string(), "docs".to_string()],
            source_count: 42,
        };
        let json = serde_json::to_string(&op).unwrap();
        assert!(json.contains(r#""type":"Commit""#));
        assert!(json.contains(r#""sourceCount":42"#));
    }

    #[test]
    fn test_revision_operation_reindex_serialization() {
        let op = RevisionOperation::Reindex {
            base: "code".to_string(),
            from_model_id: "old-model".to_string(),
            to_model_id: "new-model".to_string(),
        };
        let json = serde_json::to_string(&op).unwrap();
        assert!(json.contains(r#""type":"Reindex""#));
        assert!(json.contains(r#""fromModelId":"old-model""#));
        assert!(json.contains(r#""toModelId":"new-model""#));
    }

    #[test]
    fn test_revision_operation_release_serialization() {
        let op = RevisionOperation::Release {
            tag: Some("v1.0.0".to_string()),
        };
        let json = serde_json::to_string(&op).unwrap();
        assert!(json.contains(r#""type":"Release""#));
        assert!(json.contains(r#""tag":"v1.0.0""#));
    }

    #[test]
    fn test_revision_operation_memory_ingest_serialization() {
        let op = RevisionOperation::MemoryIngest { count: 5 };
        let json = serde_json::to_string(&op).unwrap();
        assert!(json.contains(r#""type":"MemoryIngest""#));
        assert!(json.contains(r#""count":5"#));

        let parsed: RevisionOperation = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, op);
    }

    #[test]
    fn test_revision_operation_memory_prune_serialization() {
        let op = RevisionOperation::MemoryPrune {
            count: 10,
            archived_count: 7,
            deleted_count: 3,
        };
        let json = serde_json::to_string(&op).unwrap();
        assert!(json.contains(r#""type":"MemoryPrune""#));
        assert!(json.contains(r#""count":10"#));
        assert!(json.contains(r#""archivedCount":7"#));
        assert!(json.contains(r#""deletedCount":3"#));

        let parsed: RevisionOperation = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, op);
    }

    // ------------------------------------------------------------------------
    // Revision tests
    // ------------------------------------------------------------------------

    #[test]
    fn test_revision_new() {
        let rev = Revision::new("main", None, "Test message", vec![RevisionOperation::Init]);
        assert_eq!(rev.branch, "main");
        assert!(rev.parent_id.is_none());
        assert_eq!(rev.message, "Test message");
        assert_eq!(rev.operations.len(), 1);
    }

    #[test]
    fn test_revision_init() {
        let rev = Revision::init("main");
        assert_eq!(rev.branch, "main");
        assert!(rev.parent_id.is_none());
        assert!(rev.message.contains("Initialize"));
        assert_eq!(rev.operations, vec![RevisionOperation::Init]);
    }

    #[test]
    fn test_revision_with_git_commit() {
        let rev = Revision::init("main").with_git_commit("abc123");
        assert_eq!(rev.git_commit, Some("abc123".to_string()));
    }

    #[test]
    fn test_revision_serialization_roundtrip() {
        let rev = Revision::new(
            "feature/test",
            Some(RevisionId::new("parent-id")),
            "Commit changes",
            vec![RevisionOperation::Commit {
                bases: vec!["code".to_string()],
                source_count: 10,
            }],
        );

        let json = serde_json::to_string(&rev).unwrap();
        let parsed: Revision = serde_json::from_str(&json).unwrap();

        assert_eq!(parsed.branch, rev.branch);
        assert_eq!(parsed.parent_id, rev.parent_id);
        assert_eq!(parsed.message, rev.message);
        assert_eq!(parsed.operations.len(), 1);
    }

    // ------------------------------------------------------------------------
    // Timeline I/O tests
    // ------------------------------------------------------------------------

    #[test]
    fn test_timeline_empty() {
        let temp = TempDir::new().unwrap();
        let path = temp.path().join("timeline.jsonl");

        let revisions = read_timeline(&path).unwrap();
        assert!(revisions.is_empty());
    }

    #[test]
    fn test_timeline_append_and_read() {
        let temp = TempDir::new().unwrap();
        let path = temp.path().join("timeline.jsonl");

        let rev1 = Revision::init("main");
        let rev2 = Revision::new(
            "main",
            Some(rev1.id.clone()),
            "Second revision",
            vec![RevisionOperation::Commit {
                bases: vec!["code".to_string()],
                source_count: 5,
            }],
        );

        append_revision(&path, &rev1).unwrap();
        append_revision(&path, &rev2).unwrap();

        let revisions = read_timeline(&path).unwrap();
        assert_eq!(revisions.len(), 2);
        assert_eq!(revisions[0].id, rev1.id);
        assert_eq!(revisions[1].id, rev2.id);
        assert_eq!(revisions[1].parent_id, Some(rev1.id.clone()));
    }

    #[test]
    fn test_timeline_last_revision() {
        let temp = TempDir::new().unwrap();
        let path = temp.path().join("timeline.jsonl");

        // Empty timeline
        assert!(last_revision(&path).unwrap().is_none());

        // Add revisions
        let rev1 = Revision::init("main");
        let rev2 = Revision::new(
            "main",
            Some(rev1.id.clone()),
            "Second",
            vec![RevisionOperation::Init],
        );

        append_revision(&path, &rev1).unwrap();
        append_revision(&path, &rev2).unwrap();

        let last = last_revision(&path).unwrap().unwrap();
        assert_eq!(last.id, rev2.id);
    }

    #[test]
    fn test_timeline_get_revision() {
        let temp = TempDir::new().unwrap();
        let path = temp.path().join("timeline.jsonl");

        let rev = Revision::init("main");
        append_revision(&path, &rev).unwrap();

        let found = get_revision(&path, &rev.id).unwrap();
        assert!(found.is_some());
        assert_eq!(found.unwrap().id, rev.id);

        let not_found = get_revision(&path, &RevisionId::new("nonexistent")).unwrap();
        assert!(not_found.is_none());
    }

    #[test]
    fn test_timeline_parse_error() {
        let temp = TempDir::new().unwrap();
        let path = temp.path().join("timeline.jsonl");

        // Write invalid JSON
        fs::write(&path, "not valid json\n").unwrap();

        let result = read_timeline(&path);
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), GikError::TimelineParse(_)));
    }

    // ------------------------------------------------------------------------
    // HEAD I/O tests
    // ------------------------------------------------------------------------

    #[test]
    fn test_head_missing() {
        let temp = TempDir::new().unwrap();
        let path = temp.path().join("HEAD");

        let head = read_head(&path).unwrap();
        assert!(head.is_none());
    }

    #[test]
    fn test_head_write_and_read() {
        let temp = TempDir::new().unwrap();
        let path = temp.path().join("HEAD");

        let id = RevisionId::new("test-revision-id");
        write_head(&path, &id).unwrap();

        let read_id = read_head(&path).unwrap().unwrap();
        assert_eq!(read_id, id);
    }

    #[test]
    fn test_head_empty_file() {
        let temp = TempDir::new().unwrap();
        let path = temp.path().join("HEAD");

        fs::write(&path, "  \n").unwrap();

        let head = read_head(&path).unwrap();
        assert!(head.is_none());
    }

    #[test]
    fn test_head_creates_parent_dirs() {
        let temp = TempDir::new().unwrap();
        let path = temp.path().join("deep/nested/HEAD");

        let id = RevisionId::new("test-id");
        write_head(&path, &id).unwrap();

        assert!(path.exists());
        let read_id = read_head(&path).unwrap().unwrap();
        assert_eq!(read_id, id);
    }

    // ----------------------------------------------------------------
    // resolve_revision_ref tests
    // ----------------------------------------------------------------

    /// Helper to create a chain of revisions for testing
    fn setup_revision_chain(temp: &TempDir, count: usize) -> Vec<RevisionId> {
        let timeline_path = temp.path().join("timeline.jsonl");
        let head_path = temp.path().join("HEAD");
        let mut ids = Vec::new();
        let mut prev: Option<RevisionId> = None;

        for i in 0..count {
            let id = RevisionId::new(format!("rev-{:04}-{}", i, uuid::Uuid::new_v4()));
            let rev = Revision {
                id: id.clone(),
                parent_id: prev.clone(),
                branch: "main".to_string(),
                git_commit: None,
                timestamp: Utc::now(),
                message: format!("Commit {}", i),
                operations: vec![RevisionOperation::Commit {
                    bases: vec!["sources".to_string()],
                    source_count: 1,
                }],
            };
            append_revision(&timeline_path, &rev).unwrap();
            write_head(&head_path, &id).unwrap();
            ids.push(id.clone());
            prev = Some(id);
        }
        ids
    }

    #[test]
    fn test_resolve_head() {
        let temp = TempDir::new().unwrap();
        let ids = setup_revision_chain(&temp, 3);
        let timeline_path = temp.path().join("timeline.jsonl");
        let head_path = temp.path().join("HEAD");

        let resolved = resolve_revision_ref(&timeline_path, &head_path, "HEAD").unwrap();
        assert_eq!(resolved, ids[2]); // HEAD is the last commit
    }

    #[test]
    fn test_resolve_head_ancestor_tilde_1() {
        let temp = TempDir::new().unwrap();
        let ids = setup_revision_chain(&temp, 5);
        let timeline_path = temp.path().join("timeline.jsonl");
        let head_path = temp.path().join("HEAD");

        let resolved = resolve_revision_ref(&timeline_path, &head_path, "HEAD~1").unwrap();
        assert_eq!(resolved, ids[3]); // parent of HEAD (index 4)
    }

    #[test]
    fn test_resolve_head_ancestor_tilde_3() {
        let temp = TempDir::new().unwrap();
        let ids = setup_revision_chain(&temp, 5);
        let timeline_path = temp.path().join("timeline.jsonl");
        let head_path = temp.path().join("HEAD");

        let resolved = resolve_revision_ref(&timeline_path, &head_path, "HEAD~3").unwrap();
        assert_eq!(resolved, ids[1]); // 3 ancestors back from HEAD (index 4)
    }

    #[test]
    fn test_resolve_head_ancestor_beyond_root() {
        let temp = TempDir::new().unwrap();
        let _ids = setup_revision_chain(&temp, 3);
        let timeline_path = temp.path().join("timeline.jsonl");
        let head_path = temp.path().join("HEAD");

        // HEAD~10 on a chain of 3 revisions should fail
        let result = resolve_revision_ref(&timeline_path, &head_path, "HEAD~10");
        assert!(result.is_err());
        let err_msg = format!("{}", result.unwrap_err());
        assert!(err_msg.contains("has no parent"));
    }

    #[test]
    fn test_resolve_full_id() {
        let temp = TempDir::new().unwrap();
        let ids = setup_revision_chain(&temp, 3);
        let timeline_path = temp.path().join("timeline.jsonl");
        let head_path = temp.path().join("HEAD");

        // Resolve by full ID
        let target_id = &ids[1];
        let resolved =
            resolve_revision_ref(&timeline_path, &head_path, target_id.as_str()).unwrap();
        assert_eq!(resolved, *target_id);
    }

    #[test]
    fn test_resolve_uuid_prefix() {
        let temp = TempDir::new().unwrap();
        let ids = setup_revision_chain(&temp, 3);
        let timeline_path = temp.path().join("timeline.jsonl");
        let head_path = temp.path().join("HEAD");

        // Use first 12 chars of the second revision ID as prefix
        let target_id = &ids[1];
        let prefix = &target_id.as_str()[..12];
        let resolved = resolve_revision_ref(&timeline_path, &head_path, prefix).unwrap();
        assert_eq!(resolved, *target_id);
    }

    #[test]
    fn test_resolve_nonexistent_id() {
        let temp = TempDir::new().unwrap();
        let _ids = setup_revision_chain(&temp, 3);
        let timeline_path = temp.path().join("timeline.jsonl");
        let head_path = temp.path().join("HEAD");

        let result = resolve_revision_ref(&timeline_path, &head_path, "nonexistent-revision-id");
        assert!(result.is_err());
        let err_msg = format!("{}", result.unwrap_err());
        assert!(err_msg.contains("not found") || err_msg.contains("No revision"));
    }

    #[test]
    fn test_resolve_ambiguous_prefix() {
        let temp = TempDir::new().unwrap();
        let timeline_path = temp.path().join("timeline.jsonl");
        let head_path = temp.path().join("HEAD");

        // Create revisions with same prefix
        let mut prev: Option<RevisionId> = None;
        for i in 0..3 {
            let id = RevisionId::new(format!("same-prefix-{}", i));
            let rev = Revision {
                id: id.clone(),
                parent_id: prev.clone(),
                branch: "main".to_string(),
                git_commit: None,
                timestamp: Utc::now(),
                message: format!("Commit {}", i),
                operations: vec![RevisionOperation::Commit {
                    bases: vec!["sources".to_string()],
                    source_count: 1,
                }],
            };
            append_revision(&timeline_path, &rev).unwrap();
            write_head(&head_path, &id).unwrap();
            prev = Some(id);
        }

        // "same-prefix" matches multiple revisions
        let result = resolve_revision_ref(&timeline_path, &head_path, "same-prefix");
        assert!(result.is_err());
        let err_msg = format!("{}", result.unwrap_err());
        assert!(err_msg.contains("Ambiguous"));
    }

    #[test]
    fn test_resolve_head_no_head_file() {
        let temp = TempDir::new().unwrap();
        let timeline_path = temp.path().join("timeline.jsonl");
        let head_path = temp.path().join("HEAD");

        // Create timeline but no HEAD
        let id = RevisionId::new("rev-001");
        let rev = Revision {
            id: id.clone(),
            parent_id: None,
            branch: "main".to_string(),
            git_commit: None,
            timestamp: Utc::now(),
            message: "Init".to_string(),
            operations: vec![RevisionOperation::Init],
        };
        append_revision(&timeline_path, &rev).unwrap();
        // Don't write HEAD

        let result = resolve_revision_ref(&timeline_path, &head_path, "HEAD");
        assert!(result.is_err());
        let err_msg = format!("{}", result.unwrap_err());
        assert!(err_msg.contains("HEAD") || err_msg.contains("not set"));
    }
}
