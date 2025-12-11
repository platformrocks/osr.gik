//! # gik-model
//!
//! ML inference layer for GIK - embeddings and reranking.
//!
//! This crate is the **single source of truth** for ML model inference in GIK.
//! It provides:
//!
//! - **Embedding models**: Bi-encoder models for generating text embeddings
//! - **Reranker models**: Cross-encoder models for relevance scoring
//! - **Model locator**: Runtime path resolution for bundled models
//! - **Unified config**: Single source of truth for embedding/reranker configuration
//!
//! ## Design Principles
//!
//! 1. **Production-only**: No mock implementations. Test doubles live in consuming crates.
//! 2. **Local-first**: Default is embedded Candle inference with disk-based models.
//! 3. **Provider-agnostic**: Traits don't leak Candle internals.
//! 4. **Models as disk assets**: Models are shipped with the release, not embedded in binary.
//!
//! ## Model Location
//!
//! Models are searched in this order:
//! 1. `$GIK_MODELS_DIR` environment variable
//! 2. `~/.gik/models` user directory
//! 3. `{exe_dir}/models` next to the binary
//!
//! ## Features
//!
//! - `embedded` (default): Local Candle inference with disk-based models
//! - `ollama`: Remote inference via Ollama API (future)
//!
//! ## Usage
//!
//! ```ignore
//! use gik_model::{EmbeddingModel, create_embedding_model, EmbeddingConfig};
//!
//! let config = EmbeddingConfig::default();
//! let model = create_embedding_model(&config)?;
//!
//! let embeddings = model.embed(&["Hello, world!"])?;
//! assert_eq!(embeddings[0].len(), model.dimension());
//! ```

pub mod config;
pub mod error;
pub mod model_locator;

#[cfg(feature = "embedded")]
mod embedding;

#[cfg(feature = "embedded")]
mod reranker;

// Re-export error types
pub use error::{ModelError, ModelResult};

// Re-export config types (canonical source of truth)
pub use config::{
    DevicePreference, EmbeddingConfig, EmbeddingProviderKind, HuggingFaceModelConfig,
    ModelArchitecture, ModelInfo, RerankerConfig,
};

// Re-export model locator
pub use model_locator::{
    default_locator, ModelLocator, DEFAULT_EMBEDDING_MODEL_NAME, DEFAULT_RERANKER_MODEL_NAME,
    EMBEDDINGS_SUBDIR, GIK_MODELS_DIR_ENV, REQUIRED_MODEL_FILES, RERANKERS_SUBDIR,
};

// Default model IDs (full HuggingFace identifiers)
pub const DEFAULT_EMBEDDING_MODEL_ID: &str = "sentence-transformers/all-MiniLM-L6-v2";
pub const DEFAULT_RERANKER_MODEL_ID: &str = "cross-encoder/ms-marco-MiniLM-L6-v2";

// ============================================================================
// Embedding Model Trait
// ============================================================================

/// Trait for embedding models (bi-encoders).
///
/// Generates dense vector embeddings from text inputs. These embeddings
/// can be used for semantic search via cosine similarity.
///
/// # Thread Safety
///
/// Implementations must be `Send + Sync` to allow use across threads.
pub trait EmbeddingModel: Send + Sync + std::fmt::Debug {
    /// Generate embeddings for a batch of texts.
    ///
    /// # Arguments
    ///
    /// * `texts` - Slice of text strings to embed
    ///
    /// # Returns
    ///
    /// A vector of embeddings, one per input text.
    /// Each embedding is a normalized f32 vector of length `dimension()`.
    fn embed(&self, texts: &[&str]) -> ModelResult<Vec<Vec<f32>>>;

    /// Generate embeddings for owned strings.
    ///
    /// Convenience method that calls `embed` with string slices.
    fn embed_batch(&self, texts: &[String]) -> ModelResult<Vec<Vec<f32>>> {
        let refs: Vec<&str> = texts.iter().map(|s| s.as_str()).collect();
        self.embed(&refs)
    }

    /// Warm up the model by running a dummy inference.
    ///
    /// This pre-loads weights and triggers any JIT compilation.
    fn warm_up(&self) -> ModelResult<()> {
        let _ = self.embed(&["warmup"])?;
        Ok(())
    }

    /// Get the embedding dimension.
    fn dimension(&self) -> usize;

    /// Get the maximum sequence length supported.
    fn max_sequence_length(&self) -> usize;

    /// Get model information (ID, dimension, architecture).
    fn model_info(&self) -> &ModelInfo;

    /// Get the model ID.
    fn model_id(&self) -> &str {
        &self.model_info().model_id
    }
}

// ============================================================================
// Reranker Model Trait
// ============================================================================

/// Trait for reranker models (cross-encoders).
///
/// Scores query-document pairs using a cross-encoder architecture.
/// Higher scores indicate more relevant documents.
pub trait RerankerModel: Send + Sync + std::fmt::Debug {
    /// Score a batch of documents against a query.
    ///
    /// # Arguments
    ///
    /// * `query` - The search query
    /// * `documents` - Documents to score
    ///
    /// # Returns
    ///
    /// Relevance scores in the same order as documents.
    /// Higher scores = more relevant.
    fn score_batch(&self, query: &str, documents: &[String]) -> ModelResult<Vec<f32>>;

    /// Rerank documents and return sorted indices with scores.
    ///
    /// # Returns
    ///
    /// `Vec<(original_index, score)>` sorted by score descending.
    fn rerank(&self, query: &str, documents: &[String]) -> ModelResult<Vec<(usize, f32)>> {
        let scores = self.score_batch(query, documents)?;
        let mut indexed: Vec<_> = scores.into_iter().enumerate().collect();
        indexed.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        Ok(indexed)
    }

    /// Warm up the model.
    fn warm_up(&self) -> ModelResult<()> {
        let _ = self.score_batch("warmup", &["warmup doc".to_string()])?;
        Ok(())
    }

    /// Get the model ID.
    fn model_id(&self) -> &str;
}

// ============================================================================
// Factory Functions
// ============================================================================

/// Create an embedding model from configuration.
///
/// # Features
///
/// - With `embedded` feature: Creates `CandleEmbeddingModel`
///
/// # Note
///
/// Ollama provider is reserved for future use but not yet implemented.
///
/// # Errors
///
/// Returns `ModelError` if model loading fails.
#[cfg(feature = "embedded")]
pub fn create_embedding_model(config: &EmbeddingConfig) -> ModelResult<Box<dyn EmbeddingModel>> {
    match config.provider {
        EmbeddingProviderKind::Candle => {
            let model = embedding::CandleEmbeddingModel::new(config)?;
            Ok(Box::new(model))
        }
        EmbeddingProviderKind::Ollama => Err(ModelError::ProviderNotAvailable {
            provider: "ollama".to_string(),
            reason: "Ollama provider is planned for a future release. Use 'candle' for now."
                .to_string(),
        }),
    }
}

#[cfg(not(feature = "embedded"))]
pub fn create_embedding_model(config: &EmbeddingConfig) -> ModelResult<Box<dyn EmbeddingModel>> {
    Err(ModelError::ProviderNotAvailable {
        provider: config.provider.to_string(),
        reason: "No embedding providers available. Enable 'embedded' or 'ollama' feature."
            .to_string(),
    })
}

/// Create a reranker model from configuration.
///
/// # Features
///
/// - With `embedded` feature: Creates `CandleRerankerModel`
///
/// # Errors
///
/// Returns `ModelError` if model loading fails.
#[cfg(feature = "embedded")]
pub fn create_reranker_model(config: &RerankerConfig) -> ModelResult<Box<dyn RerankerModel>> {
    let model = reranker::CandleRerankerModel::new(config)?;
    Ok(Box::new(model))
}

#[cfg(not(feature = "embedded"))]
pub fn create_reranker_model(_config: &RerankerConfig) -> ModelResult<Box<dyn RerankerModel>> {
    Err(ModelError::ProviderNotAvailable {
        provider: "candle".to_string(),
        reason: "No reranker providers available. Enable 'embedded' feature.".to_string(),
    })
}

// ============================================================================
// Re-export implementations (feature-gated)
// ============================================================================

#[cfg(feature = "embedded")]
pub use embedding::CandleEmbeddingModel;

#[cfg(feature = "embedded")]
pub use reranker::CandleRerankerModel;
