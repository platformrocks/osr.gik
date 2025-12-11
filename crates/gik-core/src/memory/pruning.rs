//! Memory pruning module for GIK.
//!
//! This module provides functionality for pruning (removing or archiving)
//! memory entries based on configurable policies. Pruning is essential for
//! managing context budget and keeping the memory base relevant.
//!
//! **Key design decisions:**
//! - Pruning is EXPLICIT only (no auto-pruning in `gik release`)
//! - Two modes: `Delete` (permanent) and `Archive` (audit trail)
//! - Archived entries are NOT searchable (removed from index)
//! - Policy is stored in `config.json` within the memory base directory
//!
//! ## Policy Configuration
//!
//! The pruning policy is defined in:
//! `.guided/knowledge/<branch>/bases/memory/config.json`
//!
//! ```json
//! {
//!   "pruningPolicy": {
//!     "maxEntries": 1000,
//!     "maxEstimatedTokens": 100000,
//!     "maxAgeDays": 365,
//!     "obsoleteTags": ["deprecated", "obsolete"],
//!     "mode": "archive"
//!   }
//! }
//! ```

use std::fs::{self, File, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};

use chrono::{Duration, Utc};
use serde::{Deserialize, Serialize};

use crate::base::{sources_path, BaseSourceEntry};
use crate::errors::GikError;
use crate::memory::metrics::{compute_memory_metrics, estimate_tokens, MemoryMetrics};
use crate::vector_index::{VectorId, VectorIndexBackend};

// ============================================================================
// Constants
// ============================================================================

/// Filename for the memory base configuration.
pub const CONFIG_FILENAME: &str = "config.json";

/// Filename for the archived entries file.
pub const ARCHIVE_FILENAME: &str = "archive.jsonl";

// ============================================================================
// MemoryPruneMode
// ============================================================================

/// Mode for handling pruned entries.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MemoryPruneMode {
    /// Permanently delete pruned entries.
    /// Use this when storage is a concern or entries are truly obsolete.
    Delete,

    /// Move pruned entries to an archive file.
    /// Archived entries are NOT searchable but preserved for audit.
    /// This is the default mode.
    #[default]
    Archive,
}

impl std::fmt::Display for MemoryPruneMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Delete => write!(f, "delete"),
            Self::Archive => write!(f, "archive"),
        }
    }
}

// ============================================================================
// MemoryPruningPolicy
// ============================================================================

/// Policy for pruning memory entries.
///
/// Multiple criteria can be specified. Entries are pruned if they match ANY of:
/// - Exceeding `max_entries` count (oldest entries first)
/// - Exceeding `max_estimated_tokens` (oldest entries first)
/// - Older than `max_age_days`
/// - Tagged with any of `obsolete_tags`
///
/// If all thresholds are `None`, pruning is effectively disabled.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MemoryPruningPolicy {
    /// Maximum number of entries to keep.
    /// If exceeded, oldest entries are pruned first.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_entries: Option<u64>,

    /// Maximum estimated token count.
    /// If exceeded, oldest entries are pruned first.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_estimated_tokens: Option<u64>,

    /// Maximum age in days.
    /// Entries older than this are pruned.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_age_days: Option<u32>,

    /// Tags that mark entries for pruning.
    /// Entries with any of these tags are pruned.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub obsolete_tags: Vec<String>,

    /// Mode for handling pruned entries.
    #[serde(default)]
    pub mode: MemoryPruneMode,
}

impl MemoryPruningPolicy {
    /// Create a new empty policy (effectively disabled).
    pub fn new() -> Self {
        Self::default()
    }

    /// Create a policy with a maximum entry count.
    pub fn with_max_entries(max_entries: u64) -> Self {
        Self {
            max_entries: Some(max_entries),
            ..Default::default()
        }
    }

    /// Create a policy with a maximum token count.
    pub fn with_max_tokens(max_tokens: u64) -> Self {
        Self {
            max_estimated_tokens: Some(max_tokens),
            ..Default::default()
        }
    }

    /// Create a policy with a maximum age.
    pub fn with_max_age_days(days: u32) -> Self {
        Self {
            max_age_days: Some(days),
            ..Default::default()
        }
    }

    /// Set the pruning mode.
    pub fn mode(mut self, mode: MemoryPruneMode) -> Self {
        self.mode = mode;
        self
    }

    /// Add obsolete tags.
    pub fn with_obsolete_tags(mut self, tags: Vec<String>) -> Self {
        self.obsolete_tags = tags;
        self
    }

    /// Check if the policy is effectively disabled (no criteria set).
    pub fn is_disabled(&self) -> bool {
        self.max_entries.is_none()
            && self.max_estimated_tokens.is_none()
            && self.max_age_days.is_none()
            && self.obsolete_tags.is_empty()
    }
}

// ============================================================================
// MemoryBaseConfig
// ============================================================================

/// Configuration for the memory base.
///
/// Stored in `config.json` within the memory base directory.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MemoryBaseConfig {
    /// Pruning policy for the memory base.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pruning_policy: Option<MemoryPruningPolicy>,
}

impl MemoryBaseConfig {
    /// Create a new config with default values.
    pub fn new() -> Self {
        Self::default()
    }

    /// Create a config with a pruning policy.
    pub fn with_pruning_policy(policy: MemoryPruningPolicy) -> Self {
        Self {
            pruning_policy: Some(policy),
        }
    }
}

// ============================================================================
// PruneResult
// ============================================================================

/// Result of a memory pruning operation.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MemoryPruneResult {
    /// Number of entries that were pruned.
    pub pruned_count: u64,

    /// Number of entries that were archived (if mode is Archive).
    pub archived_count: u64,

    /// Number of entries that were deleted (if mode is Delete).
    pub deleted_count: u64,

    /// IDs of entries that were pruned.
    pub pruned_ids: Vec<String>,

    /// Metrics before pruning.
    pub metrics_before: MemoryMetrics,

    /// Metrics after pruning.
    pub metrics_after: MemoryMetrics,

    /// The mode that was used for pruning.
    pub mode: MemoryPruneMode,

    /// Reason(s) why entries were pruned.
    pub reasons: Vec<String>,
}

impl MemoryPruneResult {
    /// Create a new empty result.
    pub fn new() -> Self {
        Self::default()
    }

    /// Check if any entries were pruned.
    pub fn is_empty(&self) -> bool {
        self.pruned_count == 0
    }
}

// ============================================================================
// Config I/O
// ============================================================================

/// Get the path to the config file for a memory base.
pub fn config_path(base_dir: &Path) -> PathBuf {
    base_dir.join(CONFIG_FILENAME)
}

/// Get the path to the archive file for a memory base.
pub fn archive_path(base_dir: &Path) -> PathBuf {
    base_dir.join(ARCHIVE_FILENAME)
}

/// Load the memory base configuration.
///
/// Returns `None` if the config file doesn't exist.
pub fn load_config(base_dir: &Path) -> Result<Option<MemoryBaseConfig>, GikError> {
    let path = config_path(base_dir);

    if !path.exists() {
        return Ok(None);
    }

    let content = fs::read_to_string(&path).map_err(|e| GikError::BaseStoreIo {
        path: path.clone(),
        message: format!("Failed to read config: {}", e),
    })?;

    let config: MemoryBaseConfig =
        serde_json::from_str(&content).map_err(|e| GikError::BaseStoreParse {
            path,
            message: format!("Failed to parse config: {}", e),
        })?;

    Ok(Some(config))
}

/// Save the memory base configuration.
pub fn save_config(base_dir: &Path, config: &MemoryBaseConfig) -> Result<(), GikError> {
    let path = config_path(base_dir);

    // Ensure directory exists
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| GikError::BaseStoreIo {
            path: path.clone(),
            message: format!("Failed to create config directory: {}", e),
        })?;
    }

    let content = serde_json::to_string_pretty(config).map_err(|e| GikError::BaseStoreParse {
        path: path.clone(),
        message: format!("Failed to serialize config: {}", e),
    })?;

    fs::write(&path, content).map_err(|e| GikError::BaseStoreIo {
        path,
        message: format!("Failed to write config: {}", e),
    })?;

    Ok(())
}

/// Load the pruning policy from the memory base config.
///
/// Returns `None` if config doesn't exist or has no pruning policy.
pub fn load_pruning_policy(base_dir: &Path) -> Result<Option<MemoryPruningPolicy>, GikError> {
    Ok(load_config(base_dir)?.and_then(|c| c.pruning_policy))
}

// ============================================================================
// Pruning Implementation
// ============================================================================

/// Entry with metadata for pruning decisions.
#[derive(Debug, Clone)]
struct PruneCandidate {
    /// Line number in sources.jsonl (0-based).
    #[allow(dead_code)]
    line_index: usize,
    /// The source entry.
    entry: BaseSourceEntry,
    /// Character count of the entry's text.
    #[allow(dead_code)]
    char_count: usize,
    /// Token estimate for the entry.
    token_estimate: u64,
    /// Whether this entry should be pruned.
    should_prune: bool,
    /// Reason for pruning (if any).
    prune_reason: Option<String>,
}

/// Read all entries from sources.jsonl with their line indices.
fn read_entries_for_pruning(base_dir: &Path) -> Result<Vec<PruneCandidate>, GikError> {
    let sources_file = sources_path(base_dir);

    if !sources_file.exists() {
        return Ok(Vec::new());
    }

    let file = File::open(&sources_file).map_err(|e| GikError::BaseStoreIo {
        path: sources_file.clone(),
        message: format!("Failed to open sources file: {}", e),
    })?;

    let reader = BufReader::new(file);
    let mut candidates = Vec::new();

    for (line_index, line_result) in reader.lines().enumerate() {
        let line = line_result.map_err(|e| GikError::BaseStoreIo {
            path: sources_file.clone(),
            message: format!("Failed to read line {}: {}", line_index + 1, e),
        })?;

        if line.trim().is_empty() {
            continue;
        }

        let entry: BaseSourceEntry =
            serde_json::from_str(&line).map_err(|e| GikError::BaseStoreParse {
                path: sources_file.clone(),
                message: format!("Failed to parse line {}: {}", line_index + 1, e),
            })?;

        let char_count = entry.text.as_ref().map(|t| t.len()).unwrap_or(0);
        let token_estimate = estimate_tokens(char_count) as u64;

        candidates.push(PruneCandidate {
            line_index,
            entry,
            char_count,
            token_estimate,
            should_prune: false,
            prune_reason: None,
        });
    }

    Ok(candidates)
}

/// Apply the pruning policy to candidates and mark them for pruning.
fn mark_entries_for_pruning(
    candidates: &mut [PruneCandidate],
    policy: &MemoryPruningPolicy,
) -> Vec<String> {
    let mut reasons = Vec::new();

    // Sort by created_at (oldest first) based on the extra field
    // For now, we use line order as a proxy for age (older entries appear first)

    // 1. Check obsolete tags
    if !policy.obsolete_tags.is_empty() {
        for candidate in candidates.iter_mut() {
            if let Some(extra) = &candidate.entry.extra {
                if let Some(tags) = extra.get("tags").and_then(|t| t.as_array()) {
                    for tag in tags {
                        if let Some(tag_str) = tag.as_str() {
                            if policy.obsolete_tags.iter().any(|t| t == tag_str) {
                                candidate.should_prune = true;
                                candidate.prune_reason = Some(format!("obsolete tag: {}", tag_str));
                                break;
                            }
                        }
                    }
                }
            }
        }
        let count = candidates.iter().filter(|c| c.should_prune).count();
        if count > 0 {
            reasons.push(format!(
                "{} entries marked for obsolete tags: {:?}",
                count, policy.obsolete_tags
            ));
        }
    }

    // 2. Check max age
    if let Some(max_days) = policy.max_age_days {
        let cutoff = Utc::now() - Duration::days(max_days as i64);
        let mut age_pruned = 0;

        for candidate in candidates.iter_mut() {
            if candidate.should_prune {
                continue; // Already marked
            }

            if let Some(extra) = &candidate.entry.extra {
                if let Some(created_str) = extra.get("created_at").and_then(|c| c.as_str()) {
                    if let Ok(created) = chrono::DateTime::parse_from_rfc3339(created_str) {
                        if created.with_timezone(&Utc) < cutoff {
                            candidate.should_prune = true;
                            candidate.prune_reason = Some(format!("older than {} days", max_days));
                            age_pruned += 1;
                        }
                    }
                }
            }
        }

        if age_pruned > 0 {
            reasons.push(format!(
                "{} entries pruned for age > {} days",
                age_pruned, max_days
            ));
        }
    }

    // 3. Check max entries (prune oldest to get under limit)
    if let Some(max_entries) = policy.max_entries {
        let current_count = candidates.iter().filter(|c| !c.should_prune).count() as u64;

        if current_count > max_entries {
            let to_prune = (current_count - max_entries) as usize;
            let mut pruned = 0;

            // Prune oldest first (lower line indices)
            for candidate in candidates.iter_mut() {
                if candidate.should_prune {
                    continue;
                }
                if pruned >= to_prune {
                    break;
                }
                candidate.should_prune = true;
                candidate.prune_reason = Some(format!(
                    "exceeds max entries ({}), oldest first",
                    max_entries
                ));
                pruned += 1;
            }

            if pruned > 0 {
                reasons.push(format!(
                    "{} entries pruned to stay under max_entries={}",
                    pruned, max_entries
                ));
            }
        }
    }

    // 4. Check max tokens (prune oldest to get under limit)
    if let Some(max_tokens) = policy.max_estimated_tokens {
        let mut current_tokens: u64 = candidates
            .iter()
            .filter(|c| !c.should_prune)
            .map(|c| c.token_estimate)
            .sum();

        if current_tokens > max_tokens {
            let mut pruned = 0;

            for candidate in candidates.iter_mut() {
                if candidate.should_prune {
                    continue;
                }
                if current_tokens <= max_tokens {
                    break;
                }

                candidate.should_prune = true;
                candidate.prune_reason =
                    Some(format!("exceeds max tokens ({}), oldest first", max_tokens));
                current_tokens = current_tokens.saturating_sub(candidate.token_estimate);
                pruned += 1;
            }

            if pruned > 0 {
                reasons.push(format!(
                    "{} entries pruned to stay under max_tokens={}",
                    pruned, max_tokens
                ));
            }
        }
    }

    reasons
}

/// Apply the memory pruning policy.
///
/// This function:
/// 1. Reads all entries from sources.jsonl
/// 2. Marks entries for pruning based on the policy
/// 3. Archives (or deletes) pruned entries
/// 4. Rewrites sources.jsonl with remaining entries
/// 5. Removes pruned entries from the vector index
///
/// # Arguments
///
/// * `base_dir` - Path to the memory base directory
/// * `policy` - The pruning policy to apply
/// * `index` - Optional vector index backend for removing pruned vectors
///
/// # Returns
///
/// * `Ok(MemoryPruneResult)` - Summary of the pruning operation
/// * `Err(GikError)` - If an I/O or parsing error occurs
pub fn apply_memory_pruning_policy(
    base_dir: &Path,
    policy: &MemoryPruningPolicy,
    index: Option<&mut dyn VectorIndexBackend>,
) -> Result<MemoryPruneResult, GikError> {
    let mut result = MemoryPruneResult::new();
    result.mode = policy.mode;

    // Compute metrics before
    result.metrics_before = compute_memory_metrics(base_dir)?;

    // If policy is disabled or no entries, return early
    if policy.is_disabled() || result.metrics_before.is_empty() {
        result.metrics_after = result.metrics_before.clone();
        return Ok(result);
    }

    // Read all entries
    let mut candidates = read_entries_for_pruning(base_dir)?;

    if candidates.is_empty() {
        result.metrics_after = MemoryMetrics::new();
        return Ok(result);
    }

    // Mark entries for pruning
    result.reasons = mark_entries_for_pruning(&mut candidates, policy);

    // Separate entries to keep and to prune
    let (to_prune, to_keep): (Vec<_>, Vec<_>) =
        candidates.into_iter().partition(|c| c.should_prune);

    if to_prune.is_empty() {
        result.metrics_after = result.metrics_before.clone();
        return Ok(result);
    }

    // Handle pruned entries based on mode
    let archive_file = archive_path(base_dir);

    match policy.mode {
        MemoryPruneMode::Archive => {
            // Append to archive file
            let mut file = OpenOptions::new()
                .create(true)
                .append(true)
                .open(&archive_file)
                .map_err(|e| GikError::BaseStoreIo {
                    path: archive_file.clone(),
                    message: format!("Failed to open archive file: {}", e),
                })?;

            for candidate in &to_prune {
                let line = serde_json::to_string(&candidate.entry).map_err(|e| {
                    GikError::BaseStoreParse {
                        path: archive_file.clone(),
                        message: format!("Failed to serialize entry: {}", e),
                    }
                })?;
                writeln!(file, "{}", line).map_err(|e| GikError::BaseStoreIo {
                    path: archive_file.clone(),
                    message: format!("Failed to write to archive: {}", e),
                })?;
            }

            file.flush().map_err(|e| GikError::BaseStoreIo {
                path: archive_file,
                message: format!("Failed to flush archive: {}", e),
            })?;

            result.archived_count = to_prune.len() as u64;
        }
        MemoryPruneMode::Delete => {
            result.deleted_count = to_prune.len() as u64;
        }
    }

    // Collect pruned IDs and vector IDs
    let mut pruned_vector_ids: Vec<VectorId> = Vec::new();
    for candidate in &to_prune {
        // Get memory_id from extra if available
        if let Some(extra) = &candidate.entry.extra {
            if let Some(mem_id) = extra.get("memory_id").and_then(|m| m.as_str()) {
                result.pruned_ids.push(mem_id.to_string());
            }
        }

        // Collect vector ID for removal
        pruned_vector_ids.push(VectorId::new(candidate.entry.vector_id));
    }

    result.pruned_count = to_prune.len() as u64;

    // Rewrite sources.jsonl with remaining entries
    let sources_file = sources_path(base_dir);
    let temp_file = sources_file.with_extension("jsonl.tmp");

    {
        let mut file = File::create(&temp_file).map_err(|e| GikError::BaseStoreIo {
            path: temp_file.clone(),
            message: format!("Failed to create temp file: {}", e),
        })?;

        for candidate in &to_keep {
            let line =
                serde_json::to_string(&candidate.entry).map_err(|e| GikError::BaseStoreParse {
                    path: temp_file.clone(),
                    message: format!("Failed to serialize entry: {}", e),
                })?;
            writeln!(file, "{}", line).map_err(|e| GikError::BaseStoreIo {
                path: temp_file.clone(),
                message: format!("Failed to write entry: {}", e),
            })?;
        }

        file.flush().map_err(|e| GikError::BaseStoreIo {
            path: temp_file.clone(),
            message: format!("Failed to flush temp file: {}", e),
        })?;
    }

    // Atomically replace the original file
    fs::rename(&temp_file, &sources_file).map_err(|e| GikError::BaseStoreIo {
        path: sources_file.clone(),
        message: format!("Failed to rename temp file: {}", e),
    })?;

    // Remove vectors from index
    if let Some(index) = index {
        if !pruned_vector_ids.is_empty() {
            index.delete(&pruned_vector_ids)?;
            index.flush()?;
        }
    }

    // Compute metrics after
    result.metrics_after = compute_memory_metrics(base_dir)?;

    Ok(result)
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::base::ChunkId;
    use tempfile::TempDir;

    // ------------------------------------------------------------------------
    // MemoryPruneMode tests
    // ------------------------------------------------------------------------

    #[test]
    fn test_prune_mode_default() {
        let mode = MemoryPruneMode::default();
        assert_eq!(mode, MemoryPruneMode::Archive);
    }

    #[test]
    fn test_prune_mode_serialization() {
        let delete_json = serde_json::to_string(&MemoryPruneMode::Delete).unwrap();
        assert_eq!(delete_json, "\"delete\"");

        let archive_json = serde_json::to_string(&MemoryPruneMode::Archive).unwrap();
        assert_eq!(archive_json, "\"archive\"");
    }

    #[test]
    fn test_prune_mode_display() {
        assert_eq!(format!("{}", MemoryPruneMode::Delete), "delete");
        assert_eq!(format!("{}", MemoryPruneMode::Archive), "archive");
    }

    // ------------------------------------------------------------------------
    // MemoryPruningPolicy tests
    // ------------------------------------------------------------------------

    #[test]
    fn test_pruning_policy_default() {
        let policy = MemoryPruningPolicy::new();
        assert!(policy.is_disabled());
        assert_eq!(policy.mode, MemoryPruneMode::Archive);
    }

    #[test]
    fn test_pruning_policy_with_max_entries() {
        let policy = MemoryPruningPolicy::with_max_entries(100);
        assert!(!policy.is_disabled());
        assert_eq!(policy.max_entries, Some(100));
    }

    #[test]
    fn test_pruning_policy_with_max_tokens() {
        let policy = MemoryPruningPolicy::with_max_tokens(50000);
        assert!(!policy.is_disabled());
        assert_eq!(policy.max_estimated_tokens, Some(50000));
    }

    #[test]
    fn test_pruning_policy_builder() {
        let policy = MemoryPruningPolicy::with_max_entries(100)
            .mode(MemoryPruneMode::Delete)
            .with_obsolete_tags(vec!["deprecated".to_string()]);

        assert_eq!(policy.max_entries, Some(100));
        assert_eq!(policy.mode, MemoryPruneMode::Delete);
        assert_eq!(policy.obsolete_tags, vec!["deprecated"]);
    }

    #[test]
    fn test_pruning_policy_serialization() {
        let policy = MemoryPruningPolicy {
            max_entries: Some(1000),
            max_estimated_tokens: Some(100000),
            max_age_days: Some(365),
            obsolete_tags: vec!["deprecated".to_string()],
            mode: MemoryPruneMode::Archive,
        };

        let json = serde_json::to_string_pretty(&policy).unwrap();
        assert!(json.contains("\"maxEntries\": 1000"));
        assert!(json.contains("\"maxEstimatedTokens\": 100000"));
        assert!(json.contains("\"maxAgeDays\": 365"));

        let parsed: MemoryPruningPolicy = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, policy);
    }

    // ------------------------------------------------------------------------
    // Config I/O tests
    // ------------------------------------------------------------------------

    #[test]
    fn test_load_config_missing() {
        let temp = TempDir::new().unwrap();
        let config = load_config(temp.path()).unwrap();
        assert!(config.is_none());
    }

    #[test]
    fn test_save_and_load_config() {
        let temp = TempDir::new().unwrap();

        let config =
            MemoryBaseConfig::with_pruning_policy(MemoryPruningPolicy::with_max_entries(500));

        save_config(temp.path(), &config).unwrap();

        let loaded = load_config(temp.path()).unwrap().unwrap();
        assert_eq!(loaded.pruning_policy.unwrap().max_entries, Some(500));
    }

    #[test]
    fn test_load_pruning_policy() {
        let temp = TempDir::new().unwrap();

        // No config - should return None
        assert!(load_pruning_policy(temp.path()).unwrap().is_none());

        // Config with policy
        let config =
            MemoryBaseConfig::with_pruning_policy(MemoryPruningPolicy::with_max_tokens(10000));
        save_config(temp.path(), &config).unwrap();

        let policy = load_pruning_policy(temp.path()).unwrap().unwrap();
        assert_eq!(policy.max_estimated_tokens, Some(10000));
    }

    // ------------------------------------------------------------------------
    // Pruning tests
    // ------------------------------------------------------------------------

    #[test]
    fn test_apply_pruning_empty_base() {
        let temp = TempDir::new().unwrap();
        let policy = MemoryPruningPolicy::with_max_entries(10);

        let result = apply_memory_pruning_policy(temp.path(), &policy, None).unwrap();

        assert!(result.is_empty());
        assert!(result.metrics_before.is_empty());
        assert!(result.metrics_after.is_empty());
    }

    #[test]
    fn test_apply_pruning_disabled_policy() {
        let temp = TempDir::new().unwrap();
        let policy = MemoryPruningPolicy::new(); // Disabled

        // Create some entries
        let sources_file = temp.path().join("sources.jsonl");
        let entry = BaseSourceEntry::new(
            ChunkId::new("chunk-1"),
            "memory",
            "main",
            "memory:1",
            1,
            1,
            0,
            "rev",
            "mem",
        )
        .with_text("Test content".to_string());
        fs::write(&sources_file, serde_json::to_string(&entry).unwrap() + "\n").unwrap();

        let result = apply_memory_pruning_policy(temp.path(), &policy, None).unwrap();

        assert!(result.is_empty());
        assert_eq!(result.metrics_before.entry_count, 1);
        assert_eq!(result.metrics_after.entry_count, 1);
    }

    #[test]
    fn test_apply_pruning_max_entries() {
        let temp = TempDir::new().unwrap();

        // Create 5 entries
        let sources_file = temp.path().join("sources.jsonl");
        let mut content = String::new();
        for i in 0..5 {
            let entry = BaseSourceEntry::new(
                ChunkId::new(format!("chunk-{}", i)),
                "memory",
                "main",
                format!("memory:{}", i),
                1,
                1,
                i as u64,
                "rev",
                format!("mem-{}", i),
            )
            .with_text(format!("Content for entry {}", i));

            content.push_str(&serde_json::to_string(&entry).unwrap());
            content.push('\n');
        }
        fs::write(&sources_file, content).unwrap();

        // Prune to max 3 entries
        let policy = MemoryPruningPolicy::with_max_entries(3).mode(MemoryPruneMode::Delete);

        let result = apply_memory_pruning_policy(temp.path(), &policy, None).unwrap();

        assert_eq!(result.pruned_count, 2);
        assert_eq!(result.deleted_count, 2);
        assert_eq!(result.metrics_before.entry_count, 5);
        assert_eq!(result.metrics_after.entry_count, 3);
    }

    #[test]
    fn test_apply_pruning_archive_mode() {
        let temp = TempDir::new().unwrap();

        // Create 3 entries
        let sources_file = temp.path().join("sources.jsonl");
        let mut content = String::new();
        for i in 0..3 {
            let entry = BaseSourceEntry::new(
                ChunkId::new(format!("chunk-{}", i)),
                "memory",
                "main",
                format!("memory:{}", i),
                1,
                1,
                i as u64,
                "rev",
                format!("mem-{}", i),
            )
            .with_text(format!("Content {}", i));

            content.push_str(&serde_json::to_string(&entry).unwrap());
            content.push('\n');
        }
        fs::write(&sources_file, content).unwrap();

        // Prune to max 1 entry with archive mode
        let policy = MemoryPruningPolicy::with_max_entries(1).mode(MemoryPruneMode::Archive);

        let result = apply_memory_pruning_policy(temp.path(), &policy, None).unwrap();

        assert_eq!(result.pruned_count, 2);
        assert_eq!(result.archived_count, 2);
        assert_eq!(result.deleted_count, 0);

        // Check archive file exists
        let archive_file = temp.path().join("archive.jsonl");
        assert!(archive_file.exists());

        // Count lines in archive
        let archive_content = fs::read_to_string(&archive_file).unwrap();
        let archive_lines: Vec<_> = archive_content.lines().collect();
        assert_eq!(archive_lines.len(), 2);
    }

    #[test]
    fn test_apply_pruning_obsolete_tags() {
        let temp = TempDir::new().unwrap();

        // Create entries with tags
        let sources_file = temp.path().join("sources.jsonl");

        let entry1 = BaseSourceEntry::new(
            ChunkId::new("chunk-1"),
            "memory",
            "main",
            "memory:1",
            1,
            1,
            0,
            "rev",
            "mem-1",
        )
        .with_text("Keep this".to_string())
        .with_extra(serde_json::json!({
            "tags": ["important"]
        }));

        let entry2 = BaseSourceEntry::new(
            ChunkId::new("chunk-2"),
            "memory",
            "main",
            "memory:2",
            1,
            1,
            1,
            "rev",
            "mem-2",
        )
        .with_text("Obsolete entry".to_string())
        .with_extra(serde_json::json!({
            "tags": ["deprecated"]
        }));

        let mut content = String::new();
        content.push_str(&serde_json::to_string(&entry1).unwrap());
        content.push('\n');
        content.push_str(&serde_json::to_string(&entry2).unwrap());
        content.push('\n');
        fs::write(&sources_file, content).unwrap();

        // Prune entries with "deprecated" tag
        let policy = MemoryPruningPolicy::new()
            .with_obsolete_tags(vec!["deprecated".to_string()])
            .mode(MemoryPruneMode::Delete);

        let result = apply_memory_pruning_policy(temp.path(), &policy, None).unwrap();

        assert_eq!(result.pruned_count, 1);
        assert_eq!(result.metrics_after.entry_count, 1);
    }
}
