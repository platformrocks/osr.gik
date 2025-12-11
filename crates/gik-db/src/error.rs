//! Error types for gik-db.

use std::path::PathBuf;
use thiserror::Error;

/// Result type alias for gik-db operations.
pub type DbResult<T> = Result<T, DbError>;

/// Errors that can occur in gik-db operations.
#[derive(Debug, Error)]
pub enum DbError {
    // ========================================================================
    // Embedding errors
    // ========================================================================
    /// Failed to load embedding model.
    #[error("Failed to load embedding model '{model}': {message}")]
    ModelLoad { model: String, message: String },

    /// Failed to generate embeddings.
    #[error("Embedding generation failed: {message}")]
    EmbeddingFailed { message: String },

    /// Model not found.
    #[error("Embedding model not found: {model}")]
    ModelNotFound { model: String },

    /// Tokenization error.
    #[error("Tokenization failed: {message}")]
    Tokenization { message: String },

    // ========================================================================
    // Vector index errors
    // ========================================================================
    /// Vector index I/O error.
    #[error("Vector index I/O error at {path}: {message}")]
    VectorIo { path: PathBuf, message: String },

    /// Vector index parse error.
    #[error("Vector index parse error at {path}: {message}")]
    VectorParse { path: PathBuf, message: String },

    /// Vector dimension mismatch.
    #[error("Vector dimension mismatch: expected {expected}, got {actual}")]
    DimensionMismatch { expected: usize, actual: usize },

    /// Vector index not found.
    #[error("Vector index not found at {path}")]
    IndexNotFound { path: PathBuf },

    /// Vector index incompatible.
    #[error("Vector index incompatible for base '{base}': {reason}")]
    IndexIncompatible { base: String, reason: String },

    /// LanceDB error.
    #[cfg(feature = "lancedb")]
    #[error("LanceDB error: {message}")]
    LanceDb { message: String },

    // ========================================================================
    // KG store errors
    // ========================================================================
    /// KG store I/O error.
    #[error("KG store I/O error: {message}")]
    KgIo { message: String },

    /// KG store query error.
    #[error("KG query failed: {message}")]
    KgQuery { message: String },

    // ========================================================================
    // General errors
    // ========================================================================
    /// Configuration error.
    #[error("Configuration error: {message}")]
    Config { message: String },

    /// IO error wrapper.
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    /// JSON error wrapper.
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),

    /// Generic internal error.
    #[error("Internal error: {message}")]
    Internal { message: String },
}

impl DbError {
    /// Create a model load error.
    pub fn model_load(model: impl Into<String>, message: impl Into<String>) -> Self {
        Self::ModelLoad {
            model: model.into(),
            message: message.into(),
        }
    }

    /// Create an embedding failed error.
    pub fn embedding_failed(message: impl Into<String>) -> Self {
        Self::EmbeddingFailed {
            message: message.into(),
        }
    }

    /// Create a vector I/O error.
    pub fn vector_io(path: impl Into<PathBuf>, message: impl Into<String>) -> Self {
        Self::VectorIo {
            path: path.into(),
            message: message.into(),
        }
    }

    /// Create a vector parse error.
    pub fn vector_parse(path: impl Into<PathBuf>, message: impl Into<String>) -> Self {
        Self::VectorParse {
            path: path.into(),
            message: message.into(),
        }
    }

    /// Create an index incompatible error.
    pub fn index_incompatible(base: impl Into<String>, reason: impl Into<String>) -> Self {
        Self::IndexIncompatible {
            base: base.into(),
            reason: reason.into(),
        }
    }

    /// Create an internal error.
    pub fn internal(message: impl Into<String>) -> Self {
        Self::Internal {
            message: message.into(),
        }
    }
}

#[cfg(feature = "lancedb")]
impl From<lancedb::Error> for DbError {
    fn from(err: lancedb::Error) -> Self {
        Self::LanceDb {
            message: err.to_string(),
        }
    }
}
