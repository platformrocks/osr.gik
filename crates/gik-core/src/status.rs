//! Status reporting types for GIK.
//!
//! This module provides types for representing the current state of a GIK
//! workspace, including HEAD information, staging summary, stack stats,
//! and per-base statistics with health indicators.
//!
//! ## Key Types
//!
//! - [`HeadInfo`] - Information about the current HEAD revision
//! - [`StatusReport`] - Complete status report for a workspace/branch
//! - [`compute_branch_stats`] - Aggregates per-base stats from on-disk contracts

use std::fs;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::ask::StackSummary;
use crate::base::{BaseHealthState, BaseStatsReport};
use crate::embedding::ModelCompatibility;
use crate::stack::StackStats;
use crate::staging::{ChangeType, StagingSummary};
use crate::timeline::RevisionOperation;
use crate::vector_index::VectorIndexCompatibility;
use crate::workspace::BranchName;

// ============================================================================
// HeadInfo
// ============================================================================

/// Information about the current HEAD revision.
///
/// Contains essential metadata from the HEAD revision without the full
/// revision payload.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HeadInfo {
    /// The revision ID pointed to by HEAD.
    pub revision_id: String,

    /// The primary operation in this revision.
    pub operation: RevisionOperation,

    /// When the revision was created.
    pub timestamp: DateTime<Utc>,

    /// Human-readable message for the revision.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

// ============================================================================
// StatusReport
// ============================================================================

/// Complete status report for a workspace and branch.
///
/// Aggregates information from:
/// - Workspace detection (root path, initialization state)
/// - Timeline (HEAD revision info)
/// - Staging (pending/indexed/failed sources)
/// - Stack (file inventory stats)
/// - Per-base stats with health indicators (Phase 6.2)
/// - Git-like working tree status (Phase 8.x)
///
/// Fields may be `None` when:
/// - The workspace/branch is not initialized
/// - The corresponding data file is missing or empty
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StatusReport {
    /// Absolute path to the workspace root.
    pub workspace_root: PathBuf,

    /// Current branch name.
    pub branch: BranchName,

    /// Whether the branch is initialized for GIK.
    pub is_initialized: bool,

    /// HEAD revision information (if available).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub head: Option<HeadInfo>,

    /// Staging summary (if available).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub staging: Option<StagingSummary>,

    /// Stack statistics (if available).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stack: Option<StackStats>,

    /// Stack summary with frameworks and services (if available).
    ///
    /// Derived from stack stats and tech entries. Provides a concise
    /// fingerprint for display and LLM context.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stack_summary: Option<StackSummary>,

    /// Per-base statistics with health indicators (Phase 6.2).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bases: Option<Vec<BaseStatsReport>>,

    // -------------------------------------------------------------------------
    // Git-like working tree status (Phase 8.x)
    // -------------------------------------------------------------------------

    /// Files staged for next commit, with their change type (new/modified).
    /// Format: Vec of (file_path, change_type).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub staged_files: Option<Vec<StagedFile>>,

    /// Indexed files that have been modified since last commit.
    /// These are files that exist in sources.jsonl but have changed on disk.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modified_files: Option<Vec<String>>,

    /// Whether the working tree is clean (no staged or modified files).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub working_tree_clean: Option<bool>,
}

/// A staged file with its change type.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StagedFile {
    /// Workspace-relative path to the file.
    pub path: String,
    /// The type of change (new or modified).
    pub change_type: ChangeType,
}

impl StatusReport {
    /// Create a status report for an uninitialized workspace/branch.
    pub fn uninitialized(workspace_root: PathBuf, branch: BranchName) -> Self {
        Self {
            workspace_root,
            branch,
            is_initialized: false,
            head: None,
            staging: None,
            stack: None,
            stack_summary: None,
            bases: None,
            staged_files: None,
            modified_files: None,
            working_tree_clean: None,
        }
    }
}

// ============================================================================
// Per-Base Stats Computation (Phase 6.2)
// ============================================================================

/// Compute per-base statistics for all bases in a branch.
///
/// This function aggregates stats from existing on-disk contracts:
/// - `bases/<base>/stats.json` - for documents, files, vectors, last_updated
/// - `bases/<base>/model-info.json` - for embedding compatibility
/// - `bases/<base>/index/meta.json` - for index compatibility
/// - Filesystem sizes for `on_disk_bytes`
///
/// # Arguments
///
/// * `branch_dir` - Path to the branch directory (e.g., `.guided/knowledge/main/`)
/// * `model_compat_fn` - Closure that checks embedding model compatibility for a base
/// * `index_compat_fn` - Closure that checks vector index compatibility for a base
///
/// # Returns
///
/// A vector of `BaseStatsReport` for each discovered base, sorted by name.
///
/// # Errors
///
/// Returns an empty vector if the bases directory does not exist.
/// Individual base errors are captured in the `BaseStatsReport.health` field.
pub fn compute_branch_stats<F, G>(
    branch_dir: &Path,
    model_compat_fn: F,
    index_compat_fn: G,
) -> Vec<BaseStatsReport>
where
    F: Fn(&str) -> Option<ModelCompatibility>,
    G: Fn(&str) -> Option<VectorIndexCompatibility>,
{
    let bases_dir = branch_dir.join("bases");
    if !bases_dir.is_dir() {
        return Vec::new();
    }

    // Discover all base directories
    let mut base_names: Vec<String> = Vec::new();
    if let Ok(entries) = fs::read_dir(&bases_dir) {
        for entry in entries.flatten() {
            if entry.path().is_dir() {
                if let Some(name) = entry.file_name().to_str() {
                    // Only include known base types
                    if matches!(name, "code" | "docs" | "memory") {
                        base_names.push(name.to_string());
                    }
                }
            }
        }
    }
    base_names.sort();

    // Compute stats for each base
    base_names
        .iter()
        .map(|base_name| {
            compute_single_base_stats(
                &bases_dir.join(base_name),
                base_name,
                &model_compat_fn,
                &index_compat_fn,
            )
        })
        .collect()
}

/// Compute stats for a single base.
fn compute_single_base_stats<F, G>(
    base_dir: &Path,
    base_name: &str,
    model_compat_fn: &F,
    index_compat_fn: &G,
) -> BaseStatsReport
where
    F: Fn(&str) -> Option<ModelCompatibility>,
    G: Fn(&str) -> Option<VectorIndexCompatibility>,
{
    let mut report = BaseStatsReport::new(base_name);

    // 1. Load base stats from stats.json
    let stats_path = base_dir.join("stats.json");
    if let Ok(Some(stats)) = crate::base::load_base_stats(&stats_path) {
        report.documents = stats.chunk_count;
        report.files = stats.file_count;
        report.vectors = stats.vector_count;
        report.last_commit = Some(stats.last_updated);
    }

    // 2. Compute on_disk_bytes from core contract files
    report.on_disk_bytes = compute_on_disk_bytes(base_dir);

    // 3. Check embedding model compatibility
    let model_compat = model_compat_fn(base_name);
    let embedding_status = match &model_compat {
        Some(ModelCompatibility::Compatible) => Some("compatible".to_string()),
        Some(ModelCompatibility::MissingModelInfo) => Some("missing".to_string()),
        Some(ModelCompatibility::Mismatch { .. }) => Some("mismatch".to_string()),
        None => None,
    };
    report.embedding_status = embedding_status;

    // 4. Check vector index compatibility
    let index_compat = index_compat_fn(base_name);
    let index_status = match &index_compat {
        Some(VectorIndexCompatibility::Compatible) => Some("compatible".to_string()),
        Some(VectorIndexCompatibility::MissingMeta) => Some("missing".to_string()),
        Some(VectorIndexCompatibility::DimensionMismatch { .. }) => {
            Some("dimension_mismatch".to_string())
        }
        Some(VectorIndexCompatibility::BackendMismatch { .. }) => {
            Some("backend_mismatch".to_string())
        }
        Some(VectorIndexCompatibility::EmbeddingMismatch { .. }) => {
            Some("embedding_mismatch".to_string())
        }
        Some(VectorIndexCompatibility::LegacyFormat { .. }) => Some("legacy_format".to_string()),
        None => None,
    };
    report.index_status = index_status;

    // 5. Derive health state from compatibility
    report.health = derive_health_state(&model_compat, &index_compat, base_dir);

    report
}

/// Compute on_disk_bytes by summing sizes of core contract files.
///
/// Sums:
/// - `sources.jsonl`
/// - `stats.json`
/// - `model-info.json`
/// - All files under `index/`
fn compute_on_disk_bytes(base_dir: &Path) -> u64 {
    let mut total: u64 = 0;

    // Sum core files
    for filename in &["sources.jsonl", "stats.json", "model-info.json"] {
        let path = base_dir.join(filename);
        if let Ok(meta) = fs::metadata(&path) {
            total += meta.len();
        }
    }

    // Sum all files under index/
    let index_dir = base_dir.join("index");
    if index_dir.is_dir() {
        if let Ok(entries) = fs::read_dir(&index_dir) {
            for entry in entries.flatten() {
                if entry.path().is_file() {
                    if let Ok(meta) = entry.metadata() {
                        total += meta.len();
                    }
                }
            }
        }
    }

    total
}

/// Derive health state from model and index compatibility.
fn derive_health_state(
    model_compat: &Option<ModelCompatibility>,
    index_compat: &Option<VectorIndexCompatibility>,
    base_dir: &Path,
) -> BaseHealthState {
    // Check for missing model info
    if matches!(model_compat, Some(ModelCompatibility::MissingModelInfo)) {
        // If index also doesn't exist, it's INDEX_MISSING (never indexed)
        let index_dir = base_dir.join("index");
        if !index_dir.exists() {
            return BaseHealthState::IndexMissing;
        }
        return BaseHealthState::MissingModel;
    }

    // Check for model mismatch
    if matches!(model_compat, Some(ModelCompatibility::Mismatch { .. })) {
        return BaseHealthState::NeedsReindex;
    }

    // Check for missing index meta
    if matches!(index_compat, Some(VectorIndexCompatibility::MissingMeta)) {
        return BaseHealthState::IndexMissing;
    }

    // Check for index mismatches
    if matches!(
        index_compat,
        Some(VectorIndexCompatibility::DimensionMismatch { .. })
            | Some(VectorIndexCompatibility::BackendMismatch { .. })
            | Some(VectorIndexCompatibility::EmbeddingMismatch { .. })
    ) {
        return BaseHealthState::NeedsReindex;
    }

    // If we couldn't determine compatibility (None values), check if files exist
    if model_compat.is_none() || index_compat.is_none() {
        let model_info_path = base_dir.join("model-info.json");
        let index_meta_path = base_dir.join("index").join("meta.json");

        if !model_info_path.exists() && !index_meta_path.exists() {
            return BaseHealthState::IndexMissing;
        }

        // Some unknown state
        return BaseHealthState::Error;
    }

    // Both compatible
    BaseHealthState::Healthy
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::timeline::RevisionOperation;

    #[test]
    fn test_head_info_serialization() {
        let head = HeadInfo {
            revision_id: "abc123".to_string(),
            operation: RevisionOperation::Init,
            timestamp: Utc::now(),
            message: Some("Initial commit".to_string()),
        };

        let json = serde_json::to_string(&head).unwrap();
        assert!(json.contains("\"revisionId\""));
        assert!(json.contains("\"operation\""));
        assert!(json.contains("\"timestamp\""));
        assert!(json.contains("\"message\""));
    }

    #[test]
    fn test_status_report_uninitialized() {
        let report = StatusReport::uninitialized(
            PathBuf::from("/test/workspace"),
            BranchName::default_branch(),
        );

        assert!(!report.is_initialized);
        assert!(report.head.is_none());
        assert!(report.staging.is_none());
        assert!(report.stack.is_none());
    }

    #[test]
    fn test_status_report_serialization() {
        let report = StatusReport {
            workspace_root: PathBuf::from("/test"),
            branch: BranchName::default_branch(),
            is_initialized: true,
            head: Some(HeadInfo {
                revision_id: "rev-001".to_string(),
                operation: RevisionOperation::Init,
                timestamp: Utc::now(),
                message: Some("Init".to_string()),
            }),
            staging: None,
            stack: None,
            stack_summary: None,
            bases: None,
            staged_files: None,
            modified_files: None,
            working_tree_clean: None,
        };

        let json = serde_json::to_string(&report).unwrap();
        assert!(json.contains("\"workspaceRoot\""));
        assert!(json.contains("\"branch\""));
        assert!(json.contains("\"isInitialized\""));
        assert!(json.contains("\"head\""));
        // staging, stack, and bases should be omitted when None
        assert!(!json.contains("\"staging\""));
        assert!(!json.contains("\"stack\""));
        assert!(!json.contains("\"bases\""));
    }
}
