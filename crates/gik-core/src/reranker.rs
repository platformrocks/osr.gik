//! Reranker module for cross-encoder based document reranking.
//!
//! This module provides a two-stage retrieval enhancement:
//! 1. Dense retrieval per-base using embeddings (existing)
//! 2. Global reranking using cross-encoder models (this module)
//!
//! # Stability
//!
//! **⚠️ EXPERIMENTAL / INTERNAL** — This module is not part of the stable public API.
//!
//! # Supported Models
//!
//! Currently supports:
//! - `cross-encoder/ms-marco-MiniLM-L6-v2` (default, via gik-model)
//!
//! # Architecture Note
//!
//! As of gik-model migration, the actual Candle implementation lives in the
//! `gik-model` crate. This module provides:
//! - Domain types and traits (stable API)
//! - `CandleRerankerBackend` as a wrapper around `gik-model::CandleRerankerModel`
//! - Factory functions for creating reranker backends

use std::fmt;
use std::str::FromStr;
use std::sync::OnceLock;

use serde::{Deserialize, Serialize};
use tracing::{info, warn};

use crate::config::{DevicePreference, RerankerConfig};
use crate::errors::GikError;

// ============================================================================
// RerankerProviderKind
// ============================================================================

/// Enum representing the available reranker providers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum RerankerProviderKind {
    /// Local Candle-based cross-encoder.
    #[default]
    Candle,
    /// Other (custom/future) providers.
    #[serde(untagged)]
    Other(String),
}

impl fmt::Display for RerankerProviderKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Candle => write!(f, "candle"),
            Self::Other(name) => write!(f, "{}", name),
        }
    }
}

impl FromStr for RerankerProviderKind {
    type Err = std::convert::Infallible;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "candle" => Ok(Self::Candle),
            other => Ok(Self::Other(other.to_string())),
        }
    }
}

// ============================================================================
// RerankerModelId
// ============================================================================

/// Newtype wrapper for reranker model identifiers.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct RerankerModelId(pub String);

impl Default for RerankerModelId {
    fn default() -> Self {
        Self("cross-encoder/ms-marco-MiniLM-L6-v2".to_string())
    }
}

impl fmt::Display for RerankerModelId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl FromStr for RerankerModelId {
    type Err = std::convert::Infallible;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(Self(s.to_string()))
    }
}

impl From<String> for RerankerModelId {
    fn from(s: String) -> Self {
        Self(s)
    }
}

impl From<&str> for RerankerModelId {
    fn from(s: &str) -> Self {
        Self(s.to_string())
    }
}

impl AsRef<str> for RerankerModelId {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

// ============================================================================
// RerankerBackend Trait
// ============================================================================

/// Trait for reranker backends.
///
/// Implementations score query-document pairs using cross-encoder models.
pub trait RerankerBackend: Send + Sync {
    /// Get the provider kind for this backend.
    fn provider_kind(&self) -> RerankerProviderKind;

    /// Get the model ID this backend uses.
    fn model_id(&self) -> &RerankerModelId;

    /// Score a batch of documents against a query.
    fn score_batch(&self, query: &str, documents: &[String]) -> Result<Vec<f32>, GikError>;

    /// Rerank documents and return sorted indices with scores.
    fn rerank(&self, query: &str, documents: &[String]) -> Result<Vec<(usize, f32)>, GikError> {
        let scores = self.score_batch(query, documents)?;
        let mut indexed: Vec<_> = scores.into_iter().enumerate().collect();
        indexed.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        Ok(indexed)
    }

    /// Warm up the backend by running a dummy inference.
    fn warm_up(&self) -> Result<(), GikError> {
        let _ = self.score_batch("warmup query", &["warmup document".to_string()])?;
        Ok(())
    }
}

// ============================================================================
// CandleRerankerBackend (wrapper around gik-model)
// ============================================================================

/// Candle-based cross-encoder reranker backend.
///
/// This is a wrapper around `gik_model::CandleRerankerModel` that implements
/// the gik-core `RerankerBackend` trait. The actual ML implementation is in
/// the `gik-model` crate.
pub struct CandleRerankerBackend {
    inner: crate::model_adapter::ModelRerankerBackend,
    #[allow(dead_code)]
    config: RerankerConfig,
}

impl CandleRerankerBackend {
    /// Create a new Candle reranker backend.
    pub fn new(config: RerankerConfig, device_pref: DevicePreference) -> Result<Self, GikError> {
        let inner =
            crate::model_adapter::ModelRerankerBackend::from_core_config(&config, device_pref)?;
        Ok(Self { inner, config })
    }
}

impl RerankerBackend for CandleRerankerBackend {
    fn provider_kind(&self) -> RerankerProviderKind {
        self.inner.provider_kind()
    }

    fn model_id(&self) -> &RerankerModelId {
        self.inner.model_id()
    }

    fn score_batch(&self, query: &str, documents: &[String]) -> Result<Vec<f32>, GikError> {
        self.inner.score_batch(query, documents)
    }
}

// ============================================================================
// Factory Functions
// ============================================================================

/// Create a reranker backend from configuration.
pub fn create_reranker_backend(
    config: &RerankerConfig,
    device_pref: DevicePreference,
) -> Result<Box<dyn RerankerBackend>, GikError> {
    let backend = CandleRerankerBackend::new(config.clone(), device_pref)?;
    Ok(Box::new(backend))
}

// ============================================================================
// Lazy Singleton
// ============================================================================

/// Global singleton for the reranker backend.
static RERANKER_BACKEND: OnceLock<Option<Box<dyn RerankerBackend>>> = OnceLock::new();

/// Initialize or get the global reranker backend.
pub fn get_or_init_reranker_backend(
    config: &RerankerConfig,
    device_pref: DevicePreference,
) -> Option<&'static dyn RerankerBackend> {
    let backend = RERANKER_BACKEND.get_or_init(|| {
        if !config.enabled {
            info!("Reranker disabled in configuration");
            return None;
        }

        match create_reranker_backend(config, device_pref) {
            Ok(backend) => {
                info!("Reranker backend initialized: {}", backend.model_id());
                Some(backend)
            }
            Err(e) => {
                warn!(
                    "Failed to initialize reranker backend (falling back to dense-only): {}",
                    e
                );
                None
            }
        }
    });

    backend.as_ref().map(|b| b.as_ref())
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_reranker_provider_kind_default() {
        let kind = RerankerProviderKind::default();
        assert_eq!(kind, RerankerProviderKind::Candle);
    }

    #[test]
    fn test_reranker_provider_kind_from_str() {
        assert_eq!(
            RerankerProviderKind::from_str("candle").unwrap(),
            RerankerProviderKind::Candle
        );
        assert_eq!(
            RerankerProviderKind::from_str("CANDLE").unwrap(),
            RerankerProviderKind::Candle
        );
        assert_eq!(
            RerankerProviderKind::from_str("custom").unwrap(),
            RerankerProviderKind::Other("custom".to_string())
        );
    }

    #[test]
    fn test_reranker_model_id_default() {
        let id = RerankerModelId::default();
        assert_eq!(id.0, "cross-encoder/ms-marco-MiniLM-L6-v2");
    }

    #[test]
    fn test_reranker_config_defaults() {
        let config = RerankerConfig::default();
        assert!(config.enabled);
        assert_eq!(config.model_id, "cross-encoder/ms-marco-MiniLM-L6-v2");
        assert_eq!(config.top_k, 30);
        assert_eq!(config.final_k, 5);
    }

    /// Mock reranker backend for testing.
    pub struct MockRerankerBackend {
        model_id: RerankerModelId,
    }

    impl Default for MockRerankerBackend {
        fn default() -> Self {
            Self::new()
        }
    }

    impl MockRerankerBackend {
        pub fn new() -> Self {
            Self {
                model_id: RerankerModelId::default(),
            }
        }
    }

    impl RerankerBackend for MockRerankerBackend {
        fn provider_kind(&self) -> RerankerProviderKind {
            RerankerProviderKind::Other("mock".to_string())
        }

        fn model_id(&self) -> &RerankerModelId {
            &self.model_id
        }

        fn score_batch(&self, query: &str, documents: &[String]) -> Result<Vec<f32>, GikError> {
            // Simple mock: score based on query-document overlap
            let query_lower = query.to_lowercase();
            let query_words: std::collections::HashSet<&str> =
                query_lower.split_whitespace().collect();

            let scores: Vec<f32> = documents
                .iter()
                .map(|doc| {
                    let doc_lower = doc.to_lowercase();
                    let doc_words: std::collections::HashSet<&str> =
                        doc_lower.split_whitespace().collect();
                    let overlap = query_words
                        .iter()
                        .filter(|w| doc_words.contains(*w))
                        .count();
                    overlap as f32 / (query_words.len().max(1) as f32)
                })
                .collect();

            Ok(scores)
        }
    }

    #[allow(dead_code)]
    pub fn create_mock_reranker_backend() -> Box<dyn RerankerBackend> {
        Box::new(MockRerankerBackend::new())
    }
}
