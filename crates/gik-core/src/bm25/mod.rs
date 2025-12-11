//! BM25 Sparse Retrieval Module for Hybrid Search.
//!
//! This module provides BM25-based lexical (sparse) retrieval to complement
//! dense semantic retrieval. Together they enable **Hybrid Search** via
//! Reciprocal Rank Fusion (RRF).
//!
//! ## Architecture
//!
//! ```text
//! Query
//!   │
//!   ├──► Dense Retrieval (embeddings + vector index)
//!   │        └──► top_k_dense candidates
//!   │
//!   └──► Sparse Retrieval (BM25 inverted index)
//!            └──► top_k_sparse candidates
//!                      │
//!                      ▼
//!              RRF Score Fusion
//!                      │
//!                      ▼
//!              Cross-Encoder Reranker
//!                      │
//!                      ▼
//!                 final_k results
//! ```
//!
//! ## Key Components
//!
//! - [`tokenizer`]: Unicode-aware tokenization with Porter stemmer
//! - [`index`]: BM25 inverted index and scoring
//! - [`scorer`]: BM25 scoring algorithm (k1=1.2, b=0.75)
//! - [`storage`]: Serialization/deserialization with bincode
//!
//! ## Usage
//!
//! ```ignore
//! use gik_core::bm25::{Bm25Index, Bm25Config, Tokenizer};
//!
//! // Build index during commit
//! let mut index = Bm25Index::new(Bm25Config::default());
//! for doc in documents {
//!     index.add_document(doc.id, &doc.text);
//! }
//!
//! // Search during ask
//! let results = index.search("query text", 30);
//! ```

mod index;
mod scorer;
mod storage;
mod tokenizer;

pub use index::{Bm25Index, DocumentStats};
pub use scorer::{bm25_score, Bm25Params};
pub use storage::{load_bm25_index, save_bm25_index, BM25_DIR_NAME};
pub use tokenizer::{Tokenizer, TokenizerConfig};

use serde::{Deserialize, Serialize};

// ============================================================================
// Configuration
// ============================================================================

/// BM25 configuration.
///
/// Controls tokenization and scoring parameters.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Bm25Config {
    /// BM25 k1 parameter - term frequency saturation.
    /// Higher values give more weight to term frequency.
    /// Default: 1.2
    #[serde(default = "default_k1")]
    pub k1: f32,

    /// BM25 b parameter - document length normalization.
    /// 0 = no length normalization, 1 = full normalization.
    /// Default: 0.75
    #[serde(default = "default_b")]
    pub b: f32,

    /// Whether to apply Porter stemming to tokens.
    /// Default: true
    #[serde(default = "default_stemming")]
    pub stemming: bool,

    /// Whether to remove stop words during tokenization.
    /// Default: true
    #[serde(default = "default_remove_stopwords")]
    pub remove_stopwords: bool,

    /// Minimum token length to include.
    /// Default: 2
    #[serde(default = "default_min_token_length")]
    pub min_token_length: usize,
}

fn default_k1() -> f32 {
    1.2
}

fn default_b() -> f32 {
    0.75
}

fn default_stemming() -> bool {
    true
}

fn default_remove_stopwords() -> bool {
    true
}

fn default_min_token_length() -> usize {
    2
}

impl Default for Bm25Config {
    fn default() -> Self {
        Self {
            k1: default_k1(),
            b: default_b(),
            stemming: default_stemming(),
            remove_stopwords: default_remove_stopwords(),
            min_token_length: default_min_token_length(),
        }
    }
}

// ============================================================================
// Hybrid Search Configuration
// ============================================================================

/// Hybrid search configuration combining dense and sparse retrieval.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HybridSearchConfig {
    /// Whether hybrid search is enabled.
    /// When false, only dense retrieval is used.
    /// Default: true
    #[serde(default = "default_hybrid_enabled")]
    pub enabled: bool,

    /// RRF k parameter for score fusion.
    /// Higher values reduce the impact of rank differences.
    /// Formula: RRF(d) = Σ 1/(k + rank)
    /// Default: 60
    #[serde(default = "default_rrf_k")]
    pub rrf_k: f32,

    /// Weight for dense (semantic) retrieval scores in fusion.
    /// Default: 0.5
    #[serde(default = "default_dense_weight")]
    pub dense_weight: f32,

    /// Weight for sparse (BM25) retrieval scores in fusion.
    /// Default: 0.5
    #[serde(default = "default_sparse_weight")]
    pub sparse_weight: f32,

    /// Number of candidates to retrieve from dense search before fusion.
    /// Default: 50
    #[serde(default = "default_dense_top_k")]
    pub dense_top_k: usize,

    /// Number of candidates to retrieve from sparse search before fusion.
    /// Default: 50
    #[serde(default = "default_sparse_top_k")]
    pub sparse_top_k: usize,

    /// BM25 configuration.
    #[serde(default)]
    pub bm25: Bm25Config,
}

fn default_hybrid_enabled() -> bool {
    true
}

fn default_rrf_k() -> f32 {
    60.0
}

fn default_dense_weight() -> f32 {
    0.5
}

fn default_sparse_weight() -> f32 {
    0.5
}

fn default_dense_top_k() -> usize {
    50
}

fn default_sparse_top_k() -> usize {
    50
}

impl Default for HybridSearchConfig {
    fn default() -> Self {
        Self {
            enabled: default_hybrid_enabled(),
            rrf_k: default_rrf_k(),
            dense_weight: default_dense_weight(),
            sparse_weight: default_sparse_weight(),
            dense_top_k: default_dense_top_k(),
            sparse_top_k: default_sparse_top_k(),
            bm25: Bm25Config::default(),
        }
    }
}

impl HybridSearchConfig {
    /// Validates the hybrid search configuration, returning warnings for questionable values.
    ///
    /// # Errors
    /// Returns an error if:
    /// - `rrf_k` is 0 or negative (would cause division by zero)
    /// - Weights are negative
    /// - `dense_top_k` or `sparse_top_k` is 0
    ///
    /// # Warnings
    /// - Weights don't sum to 1.0 (informational only, still valid)
    /// - Very large rrf_k (> 100) which may over-smooth rankings
    pub fn validate(&self) -> Result<Vec<String>, crate::GikError> {
        let mut warnings = Vec::new();

        // Only validate when enabled
        if !self.enabled {
            return Ok(warnings);
        }

        // Critical: rrf_k must be positive
        if self.rrf_k <= 0.0 {
            return Err(crate::GikError::InvalidConfiguration {
                message: "retrieval.hybrid.rrfK must be positive".to_string(),
                hint: "Set rrfK to a positive value (recommended: 60)".to_string(),
            });
        }

        // Critical: weights must be non-negative
        if self.dense_weight < 0.0 {
            return Err(crate::GikError::InvalidConfiguration {
                message: "retrieval.hybrid.denseWeight cannot be negative".to_string(),
                hint: "Set denseWeight to 0.0 or higher (recommended: 0.5)".to_string(),
            });
        }
        if self.sparse_weight < 0.0 {
            return Err(crate::GikError::InvalidConfiguration {
                message: "retrieval.hybrid.sparseWeight cannot be negative".to_string(),
                hint: "Set sparseWeight to 0.0 or higher (recommended: 0.5)".to_string(),
            });
        }

        // Critical: top_k values must be positive when using that retrieval type
        if self.dense_top_k == 0 && self.dense_weight > 0.0 {
            return Err(crate::GikError::InvalidConfiguration {
                message: "retrieval.hybrid.denseTopK cannot be 0 when denseWeight > 0".to_string(),
                hint: "Set denseTopK to at least 1 (recommended: 50)".to_string(),
            });
        }
        if self.sparse_top_k == 0 && self.sparse_weight > 0.0 {
            return Err(crate::GikError::InvalidConfiguration {
                message: "retrieval.hybrid.sparseTopK cannot be 0 when sparseWeight > 0".to_string(),
                hint: "Set sparseTopK to at least 1 (recommended: 50)".to_string(),
            });
        }

        // Warning: weights don't sum to 1.0
        let weight_sum = self.dense_weight + self.sparse_weight;
        if (weight_sum - 1.0).abs() > 0.01 {
            warnings.push(format!(
                "retrieval.hybrid weights sum to {} (denseWeight={}, sparseWeight={}); \
                 scores will be normalized but weights summing to 1.0 are recommended",
                weight_sum, self.dense_weight, self.sparse_weight
            ));
        }

        // Warning: very large rrf_k
        if self.rrf_k > 100.0 {
            warnings.push(format!(
                "retrieval.hybrid.rrfK={} is very large; rankings will be heavily smoothed (recommended: 60)",
                self.rrf_k
            ));
        }

        // Warning: both weights are 0 (effectively disabling hybrid)
        if self.dense_weight == 0.0 && self.sparse_weight == 0.0 {
            warnings.push(
                "Both denseWeight and sparseWeight are 0; no results will be returned. \
                 Consider setting enabled=false or adjusting weights."
                    .to_string(),
            );
        }

        Ok(warnings)
    }
}

// ============================================================================
// RRF Fusion
// ============================================================================

/// Result from BM25 search.
#[derive(Debug, Clone)]
pub struct Bm25SearchResult {
    /// Document ID (chunk_id).
    pub doc_id: String,
    /// BM25 score.
    pub score: f32,
    /// Rank in the BM25 result list (1-indexed).
    pub rank: usize,
}

/// Fused result after RRF combination.
#[derive(Debug, Clone)]
pub struct FusedResult {
    /// Document ID (chunk_id).
    pub doc_id: String,
    /// Combined RRF score.
    pub rrf_score: f32,
    /// Dense rank (None if not found in dense results).
    pub dense_rank: Option<usize>,
    /// Sparse rank (None if not found in sparse results).
    pub sparse_rank: Option<usize>,
}

/// Perform Reciprocal Rank Fusion (RRF) on dense and sparse results.
///
/// Formula: `RRF(d) = w_dense / (k + rank_dense) + w_sparse / (k + rank_sparse)`
///
/// Documents found in only one result set get a penalty (rank = top_k + 1).
///
/// # Errors
///
/// Returns an error if the configuration is invalid:
/// - `rrfK <= 0` (would cause division issues)
/// - `denseWeight < 0` or `sparseWeight < 0` (negative weights invalid)
pub fn rrf_fusion(
    dense_results: &[(String, f32)], // (doc_id, score)
    sparse_results: &[Bm25SearchResult],
    config: &HybridSearchConfig,
) -> Result<Vec<FusedResult>, crate::GikError> {
    use std::collections::HashMap;

    // P0: Defensive runtime checks for config values that could cause issues
    // These complement the config-time validation and protect against bypass
    if config.rrf_k <= 0.0 {
        return Err(crate::GikError::InvalidConfiguration {
            message: "retrieval.hybrid.rrfK must be positive for RRF fusion".to_string(),
            hint: "Set rrfK to a positive value (recommended: 60)".to_string(),
        });
    }
    if config.dense_weight < 0.0 {
        return Err(crate::GikError::InvalidConfiguration {
            message: "retrieval.hybrid.denseWeight cannot be negative".to_string(),
            hint: "Set denseWeight to 0.0 or higher (recommended: 0.5)".to_string(),
        });
    }
    if config.sparse_weight < 0.0 {
        return Err(crate::GikError::InvalidConfiguration {
            message: "retrieval.hybrid.sparseWeight cannot be negative".to_string(),
            hint: "Set sparseWeight to 0.0 or higher (recommended: 0.5)".to_string(),
        });
    }

    let k = config.rrf_k;
    let w_dense = config.dense_weight;
    let w_sparse = config.sparse_weight;

    // Build rank maps (1-indexed)
    let mut dense_ranks: HashMap<&str, usize> = HashMap::new();
    for (rank, (doc_id, _)) in dense_results.iter().enumerate() {
        dense_ranks.insert(doc_id.as_str(), rank + 1);
    }

    let mut sparse_ranks: HashMap<&str, usize> = HashMap::new();
    for result in sparse_results {
        sparse_ranks.insert(result.doc_id.as_str(), result.rank);
    }

    // Collect all unique doc_ids
    let mut all_docs: std::collections::HashSet<&str> = std::collections::HashSet::new();
    for (doc_id, _) in dense_results {
        all_docs.insert(doc_id.as_str());
    }
    for result in sparse_results {
        all_docs.insert(result.doc_id.as_str());
    }

    // Penalty rank for documents not found in a result set
    let penalty_rank = (dense_results.len().max(sparse_results.len()) + 1) as f32;

    // Calculate RRF scores
    let mut fused: Vec<FusedResult> = all_docs
        .into_iter()
        .map(|doc_id| {
            let dense_rank = dense_ranks.get(doc_id).copied();
            let sparse_rank = sparse_ranks.get(doc_id).copied();

            let dense_contrib = w_dense / (k + dense_rank.unwrap_or(penalty_rank as usize) as f32);
            let sparse_contrib =
                w_sparse / (k + sparse_rank.unwrap_or(penalty_rank as usize) as f32);

            FusedResult {
                doc_id: doc_id.to_string(),
                rrf_score: dense_contrib + sparse_contrib,
                dense_rank,
                sparse_rank,
            }
        })
        .collect();

    // Sort by RRF score descending
    fused.sort_by(|a, b| {
        b.rrf_score
            .partial_cmp(&a.rrf_score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    Ok(fused)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = Bm25Config::default();
        assert!((config.k1 - 1.2).abs() < 0.001);
        assert!((config.b - 0.75).abs() < 0.001);
        assert!(config.stemming);
        assert!(config.remove_stopwords);
    }

    #[test]
    fn test_hybrid_config() {
        let config = HybridSearchConfig::default();
        assert!(config.enabled);
        assert!((config.rrf_k - 60.0).abs() < 0.001);
        assert_eq!(config.dense_top_k, 50);
        assert_eq!(config.sparse_top_k, 50);
    }

    #[test]
    fn test_rrf_fusion_basic() {
        let dense = vec![
            ("doc1".to_string(), 0.9),
            ("doc2".to_string(), 0.8),
            ("doc3".to_string(), 0.7),
        ];

        let sparse = vec![
            Bm25SearchResult {
                doc_id: "doc2".to_string(),
                score: 5.0,
                rank: 1,
            },
            Bm25SearchResult {
                doc_id: "doc3".to_string(),
                score: 4.0,
                rank: 2,
            },
            Bm25SearchResult {
                doc_id: "doc4".to_string(),
                score: 3.0,
                rank: 3,
            },
        ];

        let config = HybridSearchConfig::default();
        let fused = rrf_fusion(&dense, &sparse, &config).unwrap();

        assert_eq!(fused.len(), 4); // doc1, doc2, doc3, doc4

        // doc2 should rank high (good in both)
        let doc2 = fused.iter().find(|r| r.doc_id == "doc2").unwrap();
        assert!(doc2.dense_rank.is_some());
        assert!(doc2.sparse_rank.is_some());
    }

    // ========================================================================
    // Validation Tests
    // ========================================================================

    #[test]
    fn test_hybrid_config_validate_default_is_valid() {
        let config = HybridSearchConfig::default();
        let warnings = config.validate().unwrap();
        assert!(warnings.is_empty());
    }

    #[test]
    fn test_hybrid_config_validate_disabled_skips_validation() {
        let config = HybridSearchConfig {
            enabled: false,
            rrf_k: 0.0, // Would be invalid if enabled
            ..Default::default()
        };
        // Should pass validation when disabled
        let result = config.validate();
        assert!(result.is_ok());
    }

    #[test]
    fn test_hybrid_config_validate_rrf_k_zero() {
        let config = HybridSearchConfig {
            enabled: true,
            rrf_k: 0.0,
            ..Default::default()
        };
        let result = config.validate();
        assert!(result.is_err());
    }

    #[test]
    fn test_hybrid_config_validate_negative_weight() {
        let config = HybridSearchConfig {
            enabled: true,
            dense_weight: -0.5,
            ..Default::default()
        };
        let result = config.validate();
        assert!(result.is_err());
    }

    #[test]
    fn test_hybrid_config_validate_weights_not_sum_to_one() {
        let config = HybridSearchConfig {
            enabled: true,
            dense_weight: 0.3,
            sparse_weight: 0.3,
            ..Default::default()
        };
        let warnings = config.validate().unwrap();
        assert!(!warnings.is_empty());
        assert!(warnings[0].contains("sum"));
    }

    #[test]
    fn test_hybrid_config_validate_both_weights_zero() {
        let config = HybridSearchConfig {
            enabled: true,
            dense_weight: 0.0,
            sparse_weight: 0.0,
            ..Default::default()
        };
        let warnings = config.validate().unwrap();
        assert!(!warnings.is_empty());
        assert!(warnings.iter().any(|w| w.contains("no results")));
    }

    // ========================================================================
    // RRF Fusion Runtime Validation Tests
    // ========================================================================

    #[test]
    fn test_rrf_fusion_rejects_zero_rrf_k() {
        let dense = vec![("doc1".to_string(), 0.9)];
        let sparse = vec![];
        let config = HybridSearchConfig {
            enabled: true,
            rrf_k: 0.0, // Invalid
            ..Default::default()
        };
        let result = rrf_fusion(&dense, &sparse, &config);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.to_string().contains("rrfK"));
    }

    #[test]
    fn test_rrf_fusion_rejects_negative_dense_weight() {
        let dense = vec![("doc1".to_string(), 0.9)];
        let sparse = vec![];
        let config = HybridSearchConfig {
            enabled: true,
            dense_weight: -0.5, // Invalid
            ..Default::default()
        };
        let result = rrf_fusion(&dense, &sparse, &config);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.to_string().contains("denseWeight"));
    }

    #[test]
    fn test_rrf_fusion_rejects_negative_sparse_weight() {
        let dense = vec![("doc1".to_string(), 0.9)];
        let sparse = vec![];
        let config = HybridSearchConfig {
            enabled: true,
            sparse_weight: -1.0, // Invalid
            ..Default::default()
        };
        let result = rrf_fusion(&dense, &sparse, &config);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.to_string().contains("sparseWeight"));
    }

    #[test]
    fn test_rrf_fusion_accepts_valid_config() {
        let dense = vec![("doc1".to_string(), 0.9)];
        let sparse = vec![];
        let config = HybridSearchConfig {
            enabled: true,
            rrf_k: 60.0,
            dense_weight: 0.5,
            sparse_weight: 0.5,
            ..Default::default()
        };
        let result = rrf_fusion(&dense, &sparse, &config);
        assert!(result.is_ok());
    }
}
