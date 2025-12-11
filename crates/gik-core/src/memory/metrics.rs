//! Memory metrics module for GIK.
//!
//! This module provides functionality for computing memory-specific metrics,
//! including entry counts and token estimation. These metrics are used for:
//! - Context budget monitoring
//! - Pruning threshold evaluation
//! - Usage analytics and reporting
//!
//! **Key design decisions:**
//! - `estimated_token_count` is memory-specific (NOT part of generic BaseStats)
//! - Token estimation uses ~chars/4 heuristic (approximates GPT tokenization)
//! - Metrics are computed on-demand by reading sources.jsonl

use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::base::{sources_path, BaseSourceEntry};
use crate::errors::GikError;

// ============================================================================
// Constants
// ============================================================================

/// Default divisor for estimating tokens from character count.
/// ~4 characters per token is a reasonable approximation for English text
/// and code in GPT-style tokenizers.
pub const CHARS_PER_TOKEN_ESTIMATE: usize = 4;

// ============================================================================
// MemoryMetrics
// ============================================================================

/// Metrics specific to the memory knowledge base.
///
/// This structure holds memory-specific stats that are not part of the
/// generic `BaseStats`. In particular, `estimated_token_count` is needed
/// for context budget management but doesn't apply to code bases.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MemoryMetrics {
    /// Number of memory entries in the base.
    pub entry_count: u64,

    /// Estimated token count across all entries.
    ///
    /// This is an approximation using ~chars/4 heuristic. The actual token
    /// count varies by tokenizer, but this provides a useful upper bound
    /// for context budget planning.
    pub estimated_token_count: u64,

    /// Total character count across all entries.
    ///
    /// This is the raw character count before token estimation.
    pub total_chars: u64,
}

impl MemoryMetrics {
    /// Create empty metrics.
    pub fn new() -> Self {
        Self::default()
    }

    /// Create metrics with specific values.
    pub fn with_values(entry_count: u64, estimated_token_count: u64, total_chars: u64) -> Self {
        Self {
            entry_count,
            estimated_token_count,
            total_chars,
        }
    }

    /// Add a single entry's contribution to the metrics.
    pub fn add_entry(&mut self, char_count: usize) {
        self.entry_count += 1;
        self.total_chars += char_count as u64;
        self.estimated_token_count += estimate_tokens(char_count) as u64;
    }

    /// Subtract a single entry's contribution from the metrics.
    ///
    /// Used when pruning entries. Uses saturating subtraction to avoid underflow.
    pub fn remove_entry(&mut self, char_count: usize) {
        self.entry_count = self.entry_count.saturating_sub(1);
        self.total_chars = self.total_chars.saturating_sub(char_count as u64);
        self.estimated_token_count = self
            .estimated_token_count
            .saturating_sub(estimate_tokens(char_count) as u64);
    }

    /// Check if metrics are empty (no entries).
    pub fn is_empty(&self) -> bool {
        self.entry_count == 0
    }
}

// ============================================================================
// Token Estimation
// ============================================================================

/// Estimate the number of tokens from a character count.
///
/// Uses the ~chars/4 heuristic which approximates GPT-style tokenization
/// for mixed English text and code. The result is rounded up to be conservative.
///
/// # Examples
///
/// ```
/// use gik_core::memory::metrics::estimate_tokens;
///
/// assert_eq!(estimate_tokens(100), 25);
/// assert_eq!(estimate_tokens(101), 26); // rounds up
/// assert_eq!(estimate_tokens(0), 0);
/// ```
pub fn estimate_tokens(char_count: usize) -> usize {
    char_count.div_ceil(CHARS_PER_TOKEN_ESTIMATE)
}

/// Estimate tokens from a text string.
///
/// Convenience wrapper that takes the string length and estimates tokens.
pub fn estimate_tokens_from_text(text: &str) -> usize {
    estimate_tokens(text.len())
}

// ============================================================================
// Metrics Computation
// ============================================================================

/// Compute memory metrics from a base directory.
///
/// This reads the `sources.jsonl` file in the base directory and aggregates
/// metrics from all entries. Each entry's `text` field (if present) is used
/// for character counting.
///
/// # Arguments
///
/// * `base_dir` - Path to the memory base directory (e.g., `.guided/knowledge/bases/memory`)
///
/// # Returns
///
/// * `Ok(MemoryMetrics)` - Computed metrics
/// * `Err(GikError)` - If the sources file cannot be read or parsed
///
/// # Note
///
/// If the sources file doesn't exist, returns empty metrics (not an error).
pub fn compute_memory_metrics(base_dir: &Path) -> Result<MemoryMetrics, GikError> {
    let sources_file = sources_path(base_dir);

    if !sources_file.exists() {
        return Ok(MemoryMetrics::new());
    }

    let file = File::open(&sources_file).map_err(|e| GikError::BaseStoreIo {
        path: sources_file.clone(),
        message: format!("Failed to open sources file: {}", e),
    })?;

    let reader = BufReader::new(file);
    let mut metrics = MemoryMetrics::new();

    for (line_num, line_result) in reader.lines().enumerate() {
        let line = line_result.map_err(|e| GikError::BaseStoreIo {
            path: sources_file.clone(),
            message: format!("Failed to read line {}: {}", line_num + 1, e),
        })?;

        // Skip empty lines
        if line.trim().is_empty() {
            continue;
        }

        // Parse the source entry
        let entry: BaseSourceEntry =
            serde_json::from_str(&line).map_err(|e| GikError::BaseStoreParse {
                path: sources_file.clone(),
                message: format!("Failed to parse line {}: {}", line_num + 1, e),
            })?;

        // Get character count from the text field
        let char_count = entry.text.as_ref().map(|t| t.len()).unwrap_or(0);
        metrics.add_entry(char_count);
    }

    Ok(metrics)
}

/// Compute memory metrics from a list of source entries.
///
/// This is useful when entries are already loaded in memory.
pub fn compute_metrics_from_entries(entries: &[BaseSourceEntry]) -> MemoryMetrics {
    let mut metrics = MemoryMetrics::new();

    for entry in entries {
        let char_count = entry.text.as_ref().map(|t| t.len()).unwrap_or(0);
        metrics.add_entry(char_count);
    }

    metrics
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
    // Token estimation tests
    // ------------------------------------------------------------------------

    #[test]
    fn test_estimate_tokens_basic() {
        assert_eq!(estimate_tokens(0), 0);
        assert_eq!(estimate_tokens(4), 1);
        assert_eq!(estimate_tokens(8), 2);
        assert_eq!(estimate_tokens(100), 25);
    }

    #[test]
    fn test_estimate_tokens_rounds_up() {
        assert_eq!(estimate_tokens(1), 1);
        assert_eq!(estimate_tokens(5), 2);
        assert_eq!(estimate_tokens(6), 2);
        assert_eq!(estimate_tokens(7), 2);
        assert_eq!(estimate_tokens(9), 3);
    }

    #[test]
    fn test_estimate_tokens_from_text() {
        assert_eq!(estimate_tokens_from_text(""), 0);
        assert_eq!(estimate_tokens_from_text("test"), 1);
        assert_eq!(estimate_tokens_from_text("hello world"), 3); // 11 chars -> 3 tokens
    }

    // ------------------------------------------------------------------------
    // MemoryMetrics tests
    // ------------------------------------------------------------------------

    #[test]
    fn test_memory_metrics_new() {
        let metrics = MemoryMetrics::new();
        assert_eq!(metrics.entry_count, 0);
        assert_eq!(metrics.estimated_token_count, 0);
        assert_eq!(metrics.total_chars, 0);
        assert!(metrics.is_empty());
    }

    #[test]
    fn test_memory_metrics_add_entry() {
        let mut metrics = MemoryMetrics::new();

        metrics.add_entry(100);
        assert_eq!(metrics.entry_count, 1);
        assert_eq!(metrics.total_chars, 100);
        assert_eq!(metrics.estimated_token_count, 25);
        assert!(!metrics.is_empty());

        metrics.add_entry(50);
        assert_eq!(metrics.entry_count, 2);
        assert_eq!(metrics.total_chars, 150);
        assert_eq!(metrics.estimated_token_count, 38); // 25 + 13
    }

    #[test]
    fn test_memory_metrics_remove_entry() {
        let mut metrics = MemoryMetrics::with_values(2, 50, 200);

        metrics.remove_entry(100);
        assert_eq!(metrics.entry_count, 1);
        assert_eq!(metrics.total_chars, 100);
        assert_eq!(metrics.estimated_token_count, 25);

        // Test saturating subtraction
        metrics.remove_entry(200);
        assert_eq!(metrics.entry_count, 0);
        assert_eq!(metrics.total_chars, 0);
        assert_eq!(metrics.estimated_token_count, 0);
    }

    #[test]
    fn test_memory_metrics_serialization() {
        let metrics = MemoryMetrics::with_values(10, 500, 2000);
        let json = serde_json::to_string(&metrics).unwrap();

        assert!(json.contains("\"entryCount\":10"));
        assert!(json.contains("\"estimatedTokenCount\":500"));
        assert!(json.contains("\"totalChars\":2000"));

        let parsed: MemoryMetrics = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, metrics);
    }

    // ------------------------------------------------------------------------
    // Metrics computation tests
    // ------------------------------------------------------------------------

    #[test]
    fn test_compute_memory_metrics_missing_file() {
        let temp = TempDir::new().unwrap();
        let metrics = compute_memory_metrics(temp.path()).unwrap();
        assert!(metrics.is_empty());
    }

    #[test]
    fn test_compute_memory_metrics_empty_file() {
        let temp = TempDir::new().unwrap();
        let sources_file = temp.path().join("sources.jsonl");
        std::fs::write(&sources_file, "").unwrap();

        let metrics = compute_memory_metrics(temp.path()).unwrap();
        assert!(metrics.is_empty());
    }

    #[test]
    fn test_compute_memory_metrics_with_entries() {
        let temp = TempDir::new().unwrap();
        let sources_file = temp.path().join("sources.jsonl");

        // Create test entries
        let entry1 = BaseSourceEntry::new(
            ChunkId::new("chunk-1"),
            "memory",
            "main",
            "memory:entry-1",
            1,
            1,
            0,
            "rev-1",
            "mem-1",
        )
        .with_text("This is test content".to_string());

        let entry2 = BaseSourceEntry::new(
            ChunkId::new("chunk-2"),
            "memory",
            "main",
            "memory:entry-2",
            1,
            1,
            1,
            "rev-1",
            "mem-2",
        )
        .with_text("Another entry with more text here".to_string());

        // Write entries
        let mut content = String::new();
        content.push_str(&serde_json::to_string(&entry1).unwrap());
        content.push('\n');
        content.push_str(&serde_json::to_string(&entry2).unwrap());
        content.push('\n');
        std::fs::write(&sources_file, content).unwrap();

        // Compute metrics
        let metrics = compute_memory_metrics(temp.path()).unwrap();

        assert_eq!(metrics.entry_count, 2);
        // entry1: "This is test content" = 20 chars
        // entry2: "Another entry with more text here" = 33 chars
        assert_eq!(metrics.total_chars, 53);
        // tokens: ceil(20/4) + ceil(33/4) = 5 + 9 = 14
        assert_eq!(metrics.estimated_token_count, 14);
    }

    #[test]
    fn test_compute_metrics_from_entries() {
        let entries = vec![
            BaseSourceEntry::new(
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
            .with_text("Short".to_string()),
            BaseSourceEntry::new(
                ChunkId::new("chunk-2"),
                "memory",
                "main",
                "memory:2",
                1,
                1,
                1,
                "rev",
                "mem",
            )
            .with_text("A longer piece of text".to_string()),
        ];

        let metrics = compute_metrics_from_entries(&entries);

        assert_eq!(metrics.entry_count, 2);
        // "Short" = 5 chars, "A longer piece of text" = 22 chars
        assert_eq!(metrics.total_chars, 27);
        // ceil(5/4) + ceil(22/4) = 2 + 6 = 8
        assert_eq!(metrics.estimated_token_count, 8);
    }
}
