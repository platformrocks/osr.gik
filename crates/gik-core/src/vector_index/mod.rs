//! Vector index abstraction for GIK.
//!
//! This module provides:
//! - [`VectorIndexBackendKind`] - enum of supported vector index backends
//! - [`VectorMetric`] - distance/similarity metrics for vector search
//! - [`VectorId`] - identifier for vectors in the index
//! - [`VectorIndexConfig`] - configuration for a vector index
//! - [`VectorIndexMeta`] - on-disk metadata for a base's vector index
//! - [`VectorIndexCompatibility`] - result of comparing config vs stored index metadata
//! - [`VectorIndexBackend`] - trait for vector index implementations
//!
//! ## Architecture
//!
//! GIK uses **LanceDB** via `gik-db` for vector storage. This module provides:
//! - Domain types and traits (stable API for gik-core)
//! - Factory function `open_vector_index()` that delegates to `db_adapter::DbVectorIndex`
//!
//! The actual LanceDB implementation lives in `gik-db`, keeping heavy dependencies
//! (arrow, lancedb, tokio) out of `gik-core`.
//!
//! ## Sync Design
//!
//! The `VectorIndexBackend` trait is **synchronous**. The LanceDB client is async,
//! but `gik-db` wraps it using `tokio::runtime::Runtime::block_on()` internally.

mod metadata;

use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};
use std::str::FromStr;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::embedding::EmbeddingConfig;
use crate::errors::GikError;

// Re-export submodule types
pub use metadata::{
    VectorMetadata, VectorSearchFilter, SOURCE_TYPE_ARCHIVE, SOURCE_TYPE_FILE, SOURCE_TYPE_MEMORY,
    SOURCE_TYPE_URL,
};

// ============================================================================
// Constants
// ============================================================================

/// Default vector index backend.
pub const DEFAULT_BACKEND: &str = "lancedb";

/// Default similarity metric.
pub const DEFAULT_METRIC: &str = "cosine";

/// Index metadata filename.
pub const INDEX_META_FILENAME: &str = "meta.json";

/// Index records filename (JSONL format, legacy SimpleFile backend).
pub const INDEX_RECORDS_FILENAME: &str = "records.jsonl";

/// LanceDB table name within the index directory.
pub const LANCEDB_TABLE_NAME: &str = "vectors";

// ============================================================================
// VectorIndexBackendKind
// ============================================================================

/// Supported vector index backend types.
///
/// As of Phase 8.1, `LanceDb` is the default backend.
/// `SimpleFile` is deprecated and should only be used for legacy indexes.
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VectorIndexBackendKind {
    /// LanceDB backend (default, recommended).
    #[default]
    #[serde(rename = "lancedb")]
    LanceDb,

    /// Simple file-based backend with linear scan search (deprecated).
    SimpleFile,

    /// Other/custom backend (for extensibility).
    #[serde(untagged)]
    Other(String),
}

impl fmt::Display for VectorIndexBackendKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::LanceDb => write!(f, "lancedb"),
            Self::SimpleFile => write!(f, "simple_file"),
            Self::Other(s) => write!(f, "{}", s),
        }
    }
}

impl FromStr for VectorIndexBackendKind {
    type Err = std::convert::Infallible;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(match s.to_lowercase().as_str() {
            "lancedb" | "lance_db" | "lance" => Self::LanceDb,
            "simple_file" | "simplefile" => Self::SimpleFile,
            other => Self::Other(other.to_string()),
        })
    }
}

// ============================================================================
// VectorMetric
// ============================================================================

/// Distance/similarity metric for vector search.
#[derive(Debug, Clone, Default, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum VectorMetric {
    /// Cosine similarity (default).
    #[default]
    Cosine,

    /// Dot product similarity.
    Dot,

    /// Euclidean (L2) distance.
    L2,
}

impl fmt::Display for VectorMetric {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Cosine => write!(f, "cosine"),
            Self::Dot => write!(f, "dot"),
            Self::L2 => write!(f, "l2"),
        }
    }
}

impl FromStr for VectorMetric {
    type Err = std::convert::Infallible;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(match s.to_lowercase().as_str() {
            "cosine" => Self::Cosine,
            "dot" => Self::Dot,
            "l2" | "euclidean" => Self::L2,
            _ => Self::Cosine, // Default to cosine for unknown metrics
        })
    }
}

// ============================================================================
// VectorId
// ============================================================================

/// Identifier for a vector in the index.
///
/// Uses a simple unsigned 64-bit integer for efficiency.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct VectorId(pub u64);

impl VectorId {
    /// Create a new vector ID.
    pub fn new(id: u64) -> Self {
        Self(id)
    }

    /// Get the underlying ID value.
    pub fn value(&self) -> u64 {
        self.0
    }
}

impl fmt::Display for VectorId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl From<u64> for VectorId {
    fn from(id: u64) -> Self {
        Self(id)
    }
}

// ============================================================================
// VectorIndexConfig
// ============================================================================

/// Configuration for a vector index.
///
/// This is the resolved configuration used to create or open an index.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VectorIndexConfig {
    /// Backend type (e.g., "lancedb", "simple_file").
    pub backend: VectorIndexBackendKind,

    /// Similarity metric for search.
    pub metric: VectorMetric,

    /// Vector dimension (must match embedding dimension).
    pub dimension: u32,

    /// Knowledge base name (e.g., "code", "docs").
    pub base: String,
}

impl VectorIndexConfig {
    /// Create a new vector index config.
    pub fn new(
        backend: VectorIndexBackendKind,
        metric: VectorMetric,
        dimension: u32,
        base: impl Into<String>,
    ) -> Self {
        Self {
            backend,
            metric,
            dimension,
            base: base.into(),
        }
    }

    /// Create a default config for a base with the given dimension.
    ///
    /// Uses LanceDB as the default backend.
    pub fn default_for_base(base: impl Into<String>, dimension: u32) -> Self {
        Self {
            backend: VectorIndexBackendKind::LanceDb,
            metric: VectorMetric::Cosine,
            dimension,
            base: base.into(),
        }
    }
}

// ============================================================================
// VectorIndexMeta
// ============================================================================

/// On-disk metadata for a vector index.
///
/// This is stored in `<base>/index/meta.json` and describes the index configuration
/// and embedding model used to create the vectors.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VectorIndexMeta {
    /// Backend type as string.
    pub backend: String,

    /// Similarity metric as string.
    pub metric: String,

    /// Vector dimension.
    pub dimension: u32,

    /// Knowledge base name.
    pub base: String,

    /// Embedding provider type used to create vectors.
    pub embedding_provider: String,

    /// Embedding model ID used to create vectors.
    pub embedding_model_id: String,

    /// Timestamp when the index was created.
    pub created_at: DateTime<Utc>,

    /// Timestamp of the last index update.
    pub last_updated_at: DateTime<Utc>,
}

impl VectorIndexMeta {
    /// Create metadata from config and embedding info.
    pub fn from_config(config: &VectorIndexConfig, embedding: &EmbeddingConfig) -> Self {
        let now = Utc::now();
        Self {
            backend: config.backend.to_string(),
            metric: config.metric.to_string(),
            dimension: config.dimension,
            base: config.base.clone(),
            embedding_provider: embedding.provider.to_string(),
            embedding_model_id: embedding.model_id.to_string(),
            created_at: now,
            last_updated_at: now,
        }
    }

    /// Update the last_updated_at timestamp to now.
    pub fn touch(&mut self) {
        self.last_updated_at = Utc::now();
    }
}

// ============================================================================
// VectorInsert
// ============================================================================

/// A vector to be inserted into the index.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VectorInsert {
    /// Unique identifier for this vector.
    pub id: VectorId,

    /// The embedding vector.
    pub embedding: Vec<f32>,

    /// Metadata payload (e.g., doc ID, chunk ID, source path).
    pub payload: Value,
}

impl VectorInsert {
    /// Create a new vector insert.
    pub fn new(id: impl Into<VectorId>, embedding: Vec<f32>, payload: Value) -> Self {
        Self {
            id: id.into(),
            embedding,
            payload,
        }
    }
}

// ============================================================================
// VectorSearchResult
// ============================================================================

/// Result of a vector search query.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VectorSearchResult {
    /// The vector ID.
    pub id: VectorId,

    /// Similarity/distance score.
    pub score: f32,

    /// Metadata payload associated with this vector.
    pub payload: Value,
}

// ============================================================================
// VectorIndexStats
// ============================================================================

/// Statistics for a vector index.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VectorIndexStats {
    /// Total number of vectors in the index.
    pub count: u64,

    /// Vector dimension.
    pub dimension: u32,

    /// Backend type as string.
    pub backend: String,

    /// Similarity metric as string.
    pub metric: String,
}

// ============================================================================
// VectorIndexCompatibility
// ============================================================================

/// Result of checking vector index compatibility.
///
/// Used to determine if an index can be used with the current configuration
/// or if reindexing is required.
#[derive(Debug, Clone)]
pub enum VectorIndexCompatibility {
    /// Index is compatible with current configuration.
    Compatible,

    /// Index metadata file is missing (index not created yet).
    MissingMeta,

    /// Vector dimension mismatch between config and stored index.
    DimensionMismatch {
        /// Dimension from current configuration.
        config: u32,
        /// Dimension stored in index metadata.
        meta: u32,
    },

    /// Backend type mismatch.
    BackendMismatch {
        /// Backend from current configuration.
        config_backend: String,
        /// Backend stored in index metadata.
        meta_backend: String,
    },

    /// Embedding model mismatch (checked first per user request).
    EmbeddingMismatch {
        /// Model ID from current embedding configuration.
        config_model: String,
        /// Model ID stored in index metadata.
        meta_model: String,
    },

    /// Legacy index format detected (requires `gik reindex`).
    LegacyFormat {
        /// Description of the legacy format.
        message: String,
    },
}

impl VectorIndexCompatibility {
    /// Check if the index is compatible.
    pub fn is_compatible(&self) -> bool {
        matches!(self, Self::Compatible)
    }

    /// Check if metadata is missing (fresh index needed).
    pub fn is_missing(&self) -> bool {
        matches!(self, Self::MissingMeta)
    }

    /// Check if this is a legacy format issue.
    pub fn is_legacy(&self) -> bool {
        matches!(self, Self::LegacyFormat { .. })
    }
}

// ============================================================================
// VectorIndexBackend trait
// ============================================================================

/// Trait for vector index backend implementations.
///
/// Implementors provide storage and search capabilities for embedding vectors.
///
/// ## Sync Design
///
/// This trait is synchronous. Async backends (like LanceDB) wrap their async
/// operations using `tokio::runtime::Runtime::block_on()` internally.
pub trait VectorIndexBackend: Send + Sync {
    /// Get the backend kind.
    fn backend_kind(&self) -> VectorIndexBackendKind;

    /// Get the index configuration.
    fn config(&self) -> &VectorIndexConfig;

    /// Get index statistics.
    fn stats(&self) -> Result<VectorIndexStats, GikError>;

    /// Insert or update vectors in the index.
    ///
    /// If a vector with the same ID already exists, it is replaced.
    fn upsert(&mut self, items: &[VectorInsert]) -> Result<(), GikError>;

    /// Search for the most similar vectors.
    ///
    /// # Arguments
    ///
    /// * `query` - Query vector (must have the same dimension as indexed vectors).
    /// * `top_k` - Maximum number of results to return.
    ///
    /// # Returns
    ///
    /// Results sorted by similarity score (highest first for cosine/dot, lowest first for L2).
    fn query(&self, query: &[f32], top_k: u32) -> Result<Vec<VectorSearchResult>, GikError>;

    /// Search for the most similar vectors with filtering.
    ///
    /// # Arguments
    ///
    /// * `query` - Query vector (must have the same dimension as indexed vectors).
    /// * `top_k` - Maximum number of results to return.
    /// * `filter` - Optional filter to apply before searching.
    ///
    /// # Returns
    ///
    /// Results sorted by similarity score (highest first for cosine/dot, lowest first for L2).
    ///
    /// # Default Implementation
    ///
    /// The default implementation ignores the filter and delegates to `query()`.
    /// Backends that support filtering should override this method.
    fn query_filtered(
        &self,
        query: &[f32],
        top_k: u32,
        _filter: Option<&VectorSearchFilter>,
    ) -> Result<Vec<VectorSearchResult>, GikError> {
        // Default: ignore filter, delegate to query()
        self.query(query, top_k)
    }

    /// Delete vectors by ID.
    fn delete(&mut self, ids: &[VectorId]) -> Result<(), GikError>;

    /// Flush any pending changes to disk.
    fn flush(&mut self) -> Result<(), GikError>;
}

// ============================================================================
// Helper functions (crate-internal)
// ============================================================================

/// Get the path to the index metadata file.
pub(crate) fn index_meta_path(base_root: &Path) -> PathBuf {
    base_root.join("index").join(INDEX_META_FILENAME)
}

/// Load index metadata from disk.
///
/// Returns `Ok(None)` if the file does not exist.
pub fn load_index_meta(path: &Path) -> Result<Option<VectorIndexMeta>, GikError> {
    if !path.exists() {
        return Ok(None);
    }

    let content = fs::read_to_string(path).map_err(|e| GikError::VectorIndexIo {
        path: path.to_path_buf(),
        message: e.to_string(),
    })?;

    let meta: VectorIndexMeta =
        serde_json::from_str(&content).map_err(|e| GikError::VectorIndexParse {
            path: path.to_path_buf(),
            message: e.to_string(),
        })?;

    Ok(Some(meta))
}

/// Write index metadata to disk.
pub fn write_index_meta(path: &Path, meta: &VectorIndexMeta) -> Result<(), GikError> {
    // Ensure parent directory exists
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| GikError::VectorIndexIo {
            path: path.to_path_buf(),
            message: format!("Failed to create directory: {}", e),
        })?;
    }

    let content = serde_json::to_string_pretty(meta).map_err(|e| GikError::VectorIndexParse {
        path: path.to_path_buf(),
        message: e.to_string(),
    })?;

    fs::write(path, content).map_err(|e| GikError::VectorIndexIo {
        path: path.to_path_buf(),
        message: e.to_string(),
    })?;

    Ok(())
}

/// Check index compatibility with embedding configuration.
///
/// Checks embedding model first, then dimension, then backend kind.
pub fn check_index_compatibility(
    config: &VectorIndexConfig,
    embedding: &EmbeddingConfig,
    meta: Option<&VectorIndexMeta>,
) -> VectorIndexCompatibility {
    let Some(meta) = meta else {
        return VectorIndexCompatibility::MissingMeta;
    };

    // Check embedding model first (per user request)
    let config_model = embedding.model_id.to_string();
    if meta.embedding_model_id != config_model {
        return VectorIndexCompatibility::EmbeddingMismatch {
            config_model,
            meta_model: meta.embedding_model_id.clone(),
        };
    }

    // Check dimension
    if meta.dimension != config.dimension {
        return VectorIndexCompatibility::DimensionMismatch {
            config: config.dimension,
            meta: meta.dimension,
        };
    }

    // Check backend - if meta says simple_file but config wants lancedb, that's ok
    // (we can upgrade), but warn about legacy format
    if meta.backend == "simple_file" && config.backend == VectorIndexBackendKind::LanceDb {
        return VectorIndexCompatibility::LegacyFormat {
            message: "Legacy SimpleFile index detected. Run `gik reindex` to migrate to LanceDB."
                .to_string(),
        };
    }

    // Different backend that we can't handle
    let config_backend = config.backend.to_string();
    if meta.backend != config_backend && meta.backend != "simple_file" {
        return VectorIndexCompatibility::BackendMismatch {
            config_backend,
            meta_backend: meta.backend.clone(),
        };
    }

    VectorIndexCompatibility::Compatible
}

/// Detect if a legacy (SimpleFile) index exists in the given directory.
///
/// Returns `true` if the index directory contains `records.jsonl` (the SimpleFile format).
pub fn is_legacy_index(index_dir: &Path) -> bool {
    index_dir.join(INDEX_RECORDS_FILENAME).exists()
}

/// Create a default vector index config for a base.
///
/// Uses the dimension from the embedding config and LanceDB as the backend.
pub fn default_vector_index_config_for_base(
    base: &str,
    embedding: &EmbeddingConfig,
) -> VectorIndexConfig {
    let dimension = embedding
        .dimension
        .unwrap_or(crate::embedding::DEFAULT_DIMENSION);
    VectorIndexConfig::default_for_base(base, dimension)
}

/// Open a vector index for the given base.
///
/// This is the main factory function for creating vector index backends.
/// It automatically selects the appropriate backend based on configuration
/// and detects legacy indexes that need migration.
///
/// # Arguments
///
/// * `index_root` - Path to the index directory (e.g., `bases/code/index/`)
/// * `config` - Vector index configuration
/// * `_embedding` - Embedding configuration (kept for API compatibility)
///
/// # Returns
///
/// A boxed `VectorIndexBackend` implementation.
///
/// # Errors
///
/// Returns an error if:
/// - A legacy SimpleFile index is detected (must run `gik reindex`)
/// - Index creation fails
pub fn open_vector_index(
    index_root: PathBuf,
    config: VectorIndexConfig,
    _embedding: &EmbeddingConfig,
) -> Result<Box<dyn VectorIndexBackend>, GikError> {
    // Check for legacy index format
    if is_legacy_index(&index_root) {
        // Check if meta.json says it's a simple_file backend
        let meta_path = index_root.join(INDEX_META_FILENAME);
        if let Ok(Some(meta)) = load_index_meta(&meta_path) {
            if meta.backend == "simple_file" && config.backend == VectorIndexBackendKind::LanceDb {
                return Err(GikError::VectorIndexIncompatible {
                    base: config.base.clone(),
                    reason: format!(
                        "Legacy SimpleFile index detected at {}. \
                         The index format is obsolete. \
                         Please run `gik reindex --base {}` to rebuild the index with LanceDB.",
                        index_root.display(),
                        config.base
                    ),
                });
            }
        }
    }

    // Open the appropriate backend
    match config.backend {
        VectorIndexBackendKind::LanceDb => {
            // Use gik-db via the adapter for LanceDB backend
            use crate::db_adapter::DbVectorIndex;

            let db_config =
                gik_db::vector::VectorIndexConfig::new(config.dimension as usize, &index_root)
                    .with_backend("lancedb")
                    .with_metric(gik_db::vector::VectorMetric::Cosine);

            let db_index = DbVectorIndex::open(&db_config)?;

            // Wrap in a boxed trait object with the right config
            let wrapped = DbVectorIndexWithConfig {
                inner: db_index,
                config: config.clone(),
            };
            Ok(Box::new(wrapped))
        }
        VectorIndexBackendKind::SimpleFile => {
            // SimpleFile backend removed in Phase 4 migration
            Err(GikError::VectorIndexIncompatible {
                base: config.base.clone(),
                reason: "SimpleFile backend is no longer supported. Run `gik reindex` to migrate to LanceDB.".to_string(),
            })
        }
        VectorIndexBackendKind::Other(ref name) => Err(GikError::VectorIndexIncompatible {
            base: config.base.clone(),
            reason: format!("Unknown vector index backend: {}", name),
        }),
    }
}

// ============================================================================
// DbVectorIndexWithConfig wrapper
// ============================================================================

/// Wrapper that adds gik-core's VectorIndexConfig to DbVectorIndex.
struct DbVectorIndexWithConfig {
    inner: crate::db_adapter::DbVectorIndex,
    config: VectorIndexConfig,
}

impl VectorIndexBackend for DbVectorIndexWithConfig {
    fn backend_kind(&self) -> VectorIndexBackendKind {
        self.config.backend.clone()
    }

    fn config(&self) -> &VectorIndexConfig {
        &self.config
    }

    fn stats(&self) -> Result<VectorIndexStats, GikError> {
        self.inner.stats()
    }

    fn upsert(&mut self, items: &[VectorInsert]) -> Result<(), GikError> {
        self.inner.upsert(items)
    }

    fn query(&self, query: &[f32], top_k: u32) -> Result<Vec<VectorSearchResult>, GikError> {
        self.inner.query(query, top_k)
    }

    fn query_filtered(
        &self,
        query: &[f32],
        top_k: u32,
        filter: Option<&VectorSearchFilter>,
    ) -> Result<Vec<VectorSearchResult>, GikError> {
        self.inner.query_filtered(query, top_k, filter)
    }

    fn delete(&mut self, ids: &[VectorId]) -> Result<(), GikError> {
        self.inner.delete(ids)
    }

    fn flush(&mut self) -> Result<(), GikError> {
        self.inner.flush()
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::embedding::{EmbeddingModelId, EmbeddingProviderKind};
    use tempfile::tempdir;

    fn test_embedding_config() -> EmbeddingConfig {
        EmbeddingConfig {
            provider: EmbeddingProviderKind::Candle,
            model_id: EmbeddingModelId::new("test-model"),
            architecture: None,
            dimension: Some(4),
            max_tokens: Some(256),
            local_path: None,
        }
    }

    #[test]
    fn test_vector_index_backend_kind_display() {
        assert_eq!(VectorIndexBackendKind::LanceDb.to_string(), "lancedb");
        assert_eq!(
            VectorIndexBackendKind::SimpleFile.to_string(),
            "simple_file"
        );
        assert_eq!(
            VectorIndexBackendKind::Other("custom".to_string()).to_string(),
            "custom"
        );
    }

    #[test]
    fn test_vector_index_backend_kind_from_str() {
        assert_eq!(
            VectorIndexBackendKind::from_str("lancedb").unwrap(),
            VectorIndexBackendKind::LanceDb
        );
        assert_eq!(
            VectorIndexBackendKind::from_str("lance").unwrap(),
            VectorIndexBackendKind::LanceDb
        );
        assert_eq!(
            VectorIndexBackendKind::from_str("simple_file").unwrap(),
            VectorIndexBackendKind::SimpleFile
        );
    }

    #[test]
    fn test_vector_index_backend_kind_default() {
        assert_eq!(
            VectorIndexBackendKind::default(),
            VectorIndexBackendKind::LanceDb
        );
    }

    #[test]
    fn test_vector_metric_display() {
        assert_eq!(VectorMetric::Cosine.to_string(), "cosine");
        assert_eq!(VectorMetric::Dot.to_string(), "dot");
        assert_eq!(VectorMetric::L2.to_string(), "l2");
    }

    #[test]
    fn test_vector_id() {
        let id = VectorId::new(42);
        assert_eq!(id.value(), 42);
        assert_eq!(id.to_string(), "42");
    }

    #[test]
    fn test_vector_index_config_serialization() {
        let config = VectorIndexConfig::default_for_base("code", 384);
        let json = serde_json::to_string(&config).unwrap();
        assert!(json.contains("\"backend\":\"lancedb\""));
        assert!(json.contains("\"metric\":\"cosine\""));
        assert!(json.contains("\"dimension\":384"));
    }

    #[test]
    fn test_vector_index_meta_from_config() {
        let config = VectorIndexConfig::default_for_base("code", 384);
        let embedding = test_embedding_config();
        let meta = VectorIndexMeta::from_config(&config, &embedding);

        assert_eq!(meta.backend, "lancedb");
        assert_eq!(meta.metric, "cosine");
        assert_eq!(meta.dimension, 384);
        assert_eq!(meta.base, "code");
        assert_eq!(meta.embedding_provider, "candle");
        assert_eq!(meta.embedding_model_id, "test-model");
    }

    #[test]
    fn test_check_index_compatibility_missing() {
        let config = VectorIndexConfig::default_for_base("code", 384);
        let embedding = test_embedding_config();
        let result = check_index_compatibility(&config, &embedding, None);
        assert!(result.is_missing());
    }

    #[test]
    fn test_check_index_compatibility_compatible() {
        let config = VectorIndexConfig::default_for_base("code", 4);
        let embedding = test_embedding_config();
        let meta = VectorIndexMeta::from_config(&config, &embedding);
        let result = check_index_compatibility(&config, &embedding, Some(&meta));
        assert!(result.is_compatible());
    }

    #[test]
    fn test_check_index_compatibility_embedding_mismatch() {
        let config = VectorIndexConfig::default_for_base("code", 4);
        let embedding = test_embedding_config();
        let mut meta = VectorIndexMeta::from_config(&config, &embedding);
        meta.embedding_model_id = "different-model".to_string();

        let result = check_index_compatibility(&config, &embedding, Some(&meta));
        match result {
            VectorIndexCompatibility::EmbeddingMismatch {
                config_model,
                meta_model,
            } => {
                assert_eq!(config_model, "test-model");
                assert_eq!(meta_model, "different-model");
            }
            _ => panic!("Expected EmbeddingMismatch"),
        }
    }

    #[test]
    fn test_check_index_compatibility_dimension_mismatch() {
        let config = VectorIndexConfig::default_for_base("code", 512);
        let embedding = test_embedding_config();
        let meta = VectorIndexMeta::from_config(
            &VectorIndexConfig::default_for_base("code", 4),
            &embedding,
        );

        let result = check_index_compatibility(&config, &embedding, Some(&meta));
        match result {
            VectorIndexCompatibility::DimensionMismatch { config: c, meta: m } => {
                assert_eq!(c, 512);
                assert_eq!(m, 4);
            }
            _ => panic!("Expected DimensionMismatch"),
        }
    }

    #[test]
    fn test_check_index_compatibility_legacy_format() {
        let config = VectorIndexConfig::default_for_base("code", 4);
        let embedding = test_embedding_config();

        // Create a meta with simple_file backend
        let mut meta = VectorIndexMeta::from_config(&config, &embedding);
        meta.backend = "simple_file".to_string();

        let result = check_index_compatibility(&config, &embedding, Some(&meta));
        assert!(result.is_legacy());
    }

    #[test]
    fn test_load_write_index_meta() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("meta.json");
        let config = VectorIndexConfig::default_for_base("code", 384);
        let embedding = test_embedding_config();
        let meta = VectorIndexMeta::from_config(&config, &embedding);

        // Write
        write_index_meta(&path, &meta).unwrap();

        // Read
        let loaded = load_index_meta(&path).unwrap().unwrap();
        assert_eq!(loaded.backend, meta.backend);
        assert_eq!(loaded.dimension, meta.dimension);
        assert_eq!(loaded.embedding_model_id, meta.embedding_model_id);
    }

    #[test]
    fn test_is_legacy_index() {
        let dir = tempdir().unwrap();
        let index_dir = dir.path().join("index");
        fs::create_dir_all(&index_dir).unwrap();

        // No records.jsonl = not legacy
        assert!(!is_legacy_index(&index_dir));

        // Create records.jsonl = legacy
        fs::write(index_dir.join(INDEX_RECORDS_FILENAME), "").unwrap();
        assert!(is_legacy_index(&index_dir));
    }
}
