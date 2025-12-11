//! Vector index backend implementations.
//!
//! This module provides different backend implementations for vector storage.
//!
//! ## Available Backends
//!
//! - `lancedb` (default): Production-ready LanceDB with ANN search
//! - `simple`: File-based backend for testing/small indexes

#[cfg(feature = "lancedb")]
mod lancedb;

#[cfg(feature = "simple")]
mod simple;

#[cfg(feature = "lancedb")]
pub use self::lancedb::LanceDbVectorIndex;

#[cfg(feature = "simple")]
pub use simple::SimpleFileVectorIndex;

use super::config::{
    check_index_compatibility, write_index_meta, VectorIndexCompatibility, VectorIndexConfig,
    VectorIndexMeta, DEFAULT_BACKEND,
};
use super::traits::VectorIndexBackend;
use crate::error::{DbError, DbResult};
use std::sync::Arc;
use tracing::{debug, info};

/// Open a vector index with the given configuration.
///
/// This is the main factory function for creating vector index instances.
/// It will:
/// 1. Check if an existing index is compatible
/// 2. Create a new index if needed (and `create_if_missing` is true)
/// 3. Open the appropriate backend
///
/// # Errors
///
/// Returns an error if:
/// - The index exists but is incompatible
/// - The backend is not supported
/// - The index cannot be created or opened
pub fn open_vector_index(config: &VectorIndexConfig) -> DbResult<Arc<dyn VectorIndexBackend>> {
    debug!("Opening vector index at {:?}", config.path);

    // Check compatibility
    let compat = check_index_compatibility(config);

    match compat {
        VectorIndexCompatibility::Compatible => {
            debug!("Index is compatible, opening...");
        }
        VectorIndexCompatibility::NotFound => {
            if config.create_if_missing {
                info!("Index not found, creating new index at {:?}", config.path);
                // Create directory and metadata
                std::fs::create_dir_all(&config.path)?;
                let meta = VectorIndexMeta::new(&config.backend, config.dimension, config.metric);
                write_index_meta(&config.path, &meta)?;
            } else {
                return Err(DbError::IndexNotFound {
                    path: config.path.clone(),
                });
            }
        }
        VectorIndexCompatibility::IncompatibleDimension { expected, actual } => {
            return Err(DbError::DimensionMismatch { expected, actual });
        }
        VectorIndexCompatibility::IncompatibleBackend { expected, actual } => {
            return Err(DbError::IndexIncompatible {
                base: "".to_string(),
                reason: format!(
                    "Backend mismatch: expected '{}', found '{}'",
                    expected, actual
                ),
            });
        }
        VectorIndexCompatibility::IncompatibleMetric { expected, actual } => {
            return Err(DbError::IndexIncompatible {
                base: "".to_string(),
                reason: format!(
                    "Metric mismatch: expected '{}', found '{}'",
                    expected, actual
                ),
            });
        }
        VectorIndexCompatibility::Corrupted(msg) => {
            return Err(DbError::IndexIncompatible {
                base: "".to_string(),
                reason: format!("Index corrupted: {}", msg),
            });
        }
    }

    // Open the appropriate backend
    match config.backend.as_str() {
        #[cfg(feature = "lancedb")]
        "lancedb" => {
            let index = LanceDbVectorIndex::open(config)?;
            Ok(Arc::new(index))
        }

        #[cfg(feature = "simple")]
        "simple" => {
            let index = SimpleFileVectorIndex::open(config)?;
            Ok(Arc::new(index))
        }

        // Handle case where default backend is requested but feature is disabled
        backend if backend == DEFAULT_BACKEND => {
            #[cfg(feature = "lancedb")]
            {
                let index = LanceDbVectorIndex::open(config)?;
                Ok(Arc::new(index))
            }

            #[cfg(not(feature = "lancedb"))]
            {
                Err(DbError::Internal {
                    message: format!(
                        "Backend '{}' is not available (feature not enabled)",
                        backend
                    ),
                })
            }
        }

        backend => Err(DbError::Internal {
            message: format!(
                "Unknown backend: '{}'. Available backends: {}",
                backend,
                available_backends().join(", ")
            ),
        }),
    }
}

/// Get a list of available backend names.
#[allow(clippy::vec_init_then_push)]
pub fn available_backends() -> Vec<&'static str> {
    let mut backends = Vec::new();

    #[cfg(feature = "lancedb")]
    backends.push("lancedb");

    #[cfg(feature = "simple")]
    backends.push("simple");

    backends
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_available_backends() {
        let backends = available_backends();
        // At least one backend should be available
        assert!(!backends.is_empty() || cfg!(not(any(feature = "lancedb", feature = "simple"))));
    }
}
