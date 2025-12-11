//! Error types for gik-model.
//!
//! This module provides structured error types with actionable guidance.
//! Errors clearly explain:
//! - What went wrong
//! - Where models are expected
//! - How to fix the issue

use std::path::PathBuf;
use thiserror::Error;

/// Result type alias for gik-model operations.
pub type ModelResult<T> = Result<T, ModelError>;

/// Errors that can occur in gik-model operations.
#[derive(Debug, Error)]
pub enum ModelError {
    // ========================================================================
    // Model discovery errors
    // ========================================================================
    /// No models directory found in any search location.
    #[error("{}", format_models_dir_not_found(.searched))]
    ModelsDirectoryNotFound { searched: Vec<PathBuf> },

    /// Model files not found at expected location.
    #[error("{}", format_model_not_found(.model_id, .path))]
    ModelNotFound { model_id: String, path: PathBuf },

    /// Model directory exists but is missing required files.
    #[error("{}", format_incomplete_model(.path, .missing))]
    IncompleteModelFiles {
        path: PathBuf,
        missing: Vec<&'static str>,
    },

    // ========================================================================
    // Model loading errors
    // ========================================================================
    /// Failed to load model.
    #[error("Failed to load model '{model_id}': {message}")]
    ModelLoad { model_id: String, message: String },

    /// Model configuration invalid or corrupted.
    #[error("Invalid model configuration: {message}\n\nThe model's config.json may be corrupted or incompatible.\nTry re-downloading the model from Hugging Face.")]
    InvalidConfig { message: String },

    // ========================================================================
    // Inference errors
    // ========================================================================
    /// Tokenization failed.
    #[error("Tokenization failed: {message}")]
    Tokenization { message: String },

    /// Embedding generation failed.
    #[error("Embedding failed for model '{model_id}': {message}")]
    EmbeddingFailed { model_id: String, message: String },

    /// Reranking failed.
    #[error("Reranking failed for model '{model_id}': {message}")]
    RerankingFailed { model_id: String, message: String },

    // ========================================================================
    // Provider errors
    // ========================================================================
    /// Provider not available.
    #[error("Provider '{provider}' not available: {reason}")]
    ProviderNotAvailable { provider: String, reason: String },

    /// Device not available.
    #[error("Compute device not available: {reason}\n\nGIK tried to use GPU acceleration but it is not available.\nSet device preference to 'cpu' in ~/.gik/config.yaml to use CPU-only inference.")]
    DeviceNotAvailable { reason: String },

    // ========================================================================
    // I/O errors
    // ========================================================================
    /// File I/O error.
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    /// JSON parsing error.
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),
}

// ============================================================================
// Error message formatters
// ============================================================================

fn format_models_dir_not_found(searched: &[PathBuf]) -> String {
    let list = searched
        .iter()
        .enumerate()
        .map(|(i, p)| format!("  {}. {}", i + 1, p.display()))
        .collect::<Vec<_>>()
        .join("\n");

    format!(
        "Models directory not found.\n\n\
        GIK searched these locations:\n\
        {list}\n\n\
        Models are shipped with the GIK release. To fix:\n\
        1. Set $GIK_MODELS_DIR to your models directory, OR\n\
        2. Copy models to ~/.gik/models/, OR\n\
        3. Ensure models/ exists next to the gik binary."
    )
}

fn format_model_not_found(model_id: &str, path: &std::path::Path) -> String {
    format!(
        "Model not found: {model_id}\n\n\
        Expected at: {}\n\n\
        Models are shipped with the GIK release. Ensure the model directory exists\n\
        and contains config.json, model.safetensors, and tokenizer.json.",
        path.display()
    )
}

fn format_incomplete_model(path: &std::path::Path, missing: &[&str]) -> String {
    let missing_list = missing.join(", ");
    format!(
        "Incomplete model installation at {}\n\n\
        Missing files: {missing_list}\n\n\
        A complete model directory must contain:\n\
        - config.json (model configuration)\n\
        - model.safetensors (model weights)\n\
        - tokenizer.json (tokenizer configuration)",
        path.display()
    )
}

// ============================================================================
// Error constructors
// ============================================================================

impl ModelError {
    /// Create a model load error.
    pub fn model_load(model_id: impl Into<String>, message: impl Into<String>) -> Self {
        Self::ModelLoad {
            model_id: model_id.into(),
            message: message.into(),
        }
    }

    /// Create an embedding failed error.
    pub fn embedding_failed(model_id: impl Into<String>, message: impl Into<String>) -> Self {
        Self::EmbeddingFailed {
            model_id: model_id.into(),
            message: message.into(),
        }
    }

    /// Create a reranking failed error.
    pub fn reranking_failed(model_id: impl Into<String>, message: impl Into<String>) -> Self {
        Self::RerankingFailed {
            model_id: model_id.into(),
            message: message.into(),
        }
    }

    /// Create a tokenization error.
    pub fn tokenization(message: impl Into<String>) -> Self {
        Self::Tokenization {
            message: message.into(),
        }
    }
}
