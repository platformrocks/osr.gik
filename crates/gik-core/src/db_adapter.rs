//! Adapter layer for gik-db infrastructure.
//!
//! This module bridges gik-db implementations with gik-core's domain types.
//! It provides:
//!
//! - Error conversion from `DbError` to `GikError`
//! - Type conversion utilities between gik-db and gik-core types
//! - Wrapper types that implement gik-core traits using gik-db backends
//!
//! ## Architecture
//!
//! ```text
//! gik-core domain code (engine, reindex, ask, commit)
//!        ↓
//!   db_adapter (this module) - wrappers + conversions
//!        ↓
//!     gik-db implementations (LanceDB vector storage, KG persistence)
//!
//!   model_adapter (separate module) - embedding/reranker
//!        ↓
//!     gik-model implementations (Candle ML inference)
//! ```
//!
//! ## Wrappers Provided
//!
//! - `DbVectorIndex` - wraps `gik_db::VectorIndexBackend` for gik-core
//! - `DbKgStore` - wraps `gik_db::KgStoreBackend` for gik-core
//!
//! Note: Embedding backends are now provided by `model_adapter` using gik-model.

use std::sync::Arc;

use crate::errors::GikError;
use crate::vector_index::{
    VectorId as CoreVectorId, VectorIndexBackendKind, VectorIndexConfig as CoreVectorIndexConfig,
    VectorIndexStats, VectorInsert as CoreVectorInsert, VectorMetric as CoreVectorMetric,
    VectorSearchFilter, VectorSearchResult as CoreVectorSearchResult,
};

// ============================================================================
// Error Conversion
// ============================================================================

/// Convert a gik-db error to a gik-core error.
///
/// This function maps DbError variants to appropriate GikError variants.
pub fn from_db_error(err: gik_db::DbError) -> GikError {
    use gik_db::DbError;

    match err {
        DbError::Io(io_err) => GikError::Io(io_err),

        DbError::ModelLoad { model, message } => GikError::EmbeddingProviderUnavailable {
            provider: model,
            reason: message,
        },

        DbError::EmbeddingFailed { message } => GikError::EmbeddingProviderUnavailable {
            provider: "unknown".to_string(),
            reason: message,
        },

        DbError::ModelNotFound { model } => GikError::EmbeddingProviderUnavailable {
            provider: model,
            reason: "Model not found".to_string(),
        },

        DbError::Tokenization { message } => GikError::EmbeddingProviderUnavailable {
            provider: "tokenizer".to_string(),
            reason: message,
        },

        DbError::VectorIo { path, message } => GikError::VectorIndexIo { path, message },

        DbError::VectorParse { path, message } => GikError::VectorIndexParse { path, message },

        DbError::DimensionMismatch { expected, actual } => GikError::VectorIndexIncompatible {
            base: "unknown".to_string(),
            reason: format!("Dimension mismatch: expected {}, got {}", expected, actual),
        },

        DbError::IndexNotFound { path } => GikError::VectorIndexIo {
            path: path.clone(),
            message: format!("Index not found at {:?}", path),
        },

        DbError::IndexIncompatible { base, reason } => {
            GikError::VectorIndexIncompatible { base, reason }
        }

        DbError::LanceDb { message } => GikError::VectorIndexBackendUnavailable {
            backend: "lancedb".to_string(),
            reason: message,
        },

        DbError::KgIo { message } => GikError::BaseStoreIo {
            path: std::path::PathBuf::new(),
            message,
        },

        DbError::KgQuery { message } => GikError::BaseStoreParse {
            path: std::path::PathBuf::new(),
            message,
        },

        DbError::Config { message } => GikError::EmbeddingConfigError { message },

        DbError::Json(json_err) => GikError::StagingParse(json_err.to_string()),

        DbError::Internal { message } => GikError::EmbeddingConfigError { message },
    }
}

/// Extension trait to convert DbResult to Result<T, GikError>.
pub trait IntoGikResult<T> {
    /// Convert a DbResult to a GikError result.
    fn into_gik_result(self) -> Result<T, GikError>;
}

impl<T> IntoGikResult<T> for gik_db::DbResult<T> {
    fn into_gik_result(self) -> Result<T, GikError> {
        self.map_err(from_db_error)
    }
}

// ============================================================================
// KG Type Conversion
// ============================================================================

/// Convert a gik-core KgNode to a gik-db KgNode.
pub fn to_db_kg_node(node: &crate::kg::KgNode) -> gik_db::kg::KgNode {
    gik_db::kg::KgNode {
        id: node.id.clone(),
        kind: node.kind.clone(),
        label: node.label.clone(),
        props: node.props.clone(),
        branch: node.branch.clone(),
        created_at: node.created_at,
        updated_at: node.updated_at,
    }
}

/// Convert a gik-db KgNode to a gik-core KgNode.
pub fn from_db_kg_node(node: gik_db::kg::KgNode) -> crate::kg::KgNode {
    crate::kg::KgNode {
        id: node.id,
        kind: node.kind,
        label: node.label,
        props: node.props,
        branch: node.branch,
        created_at: node.created_at,
        updated_at: node.updated_at,
    }
}

/// Convert a gik-core KgEdge to a gik-db KgEdge.
pub fn to_db_kg_edge(edge: &crate::kg::KgEdge) -> gik_db::kg::KgEdge {
    gik_db::kg::KgEdge {
        id: edge.id.clone(),
        from: edge.from.clone(),
        to: edge.to.clone(),
        kind: edge.kind.clone(),
        props: edge.props.clone(),
        branch: edge.branch.clone(),
        created_at: edge.created_at,
        updated_at: edge.updated_at,
    }
}

/// Convert a gik-db KgEdge to a gik-core KgEdge.
pub fn from_db_kg_edge(edge: gik_db::kg::KgEdge) -> crate::kg::KgEdge {
    crate::kg::KgEdge {
        id: edge.id,
        from: edge.from,
        to: edge.to,
        kind: edge.kind,
        props: edge.props,
        branch: edge.branch,
        created_at: edge.created_at,
        updated_at: edge.updated_at,
    }
}

/// Convert a gik-db KgStats to a gik-core KgStats.
pub fn from_db_kg_stats(stats: gik_db::kg::KgStats) -> crate::kg::KgStats {
    crate::kg::KgStats {
        node_count: stats.node_count,
        edge_count: stats.edge_count,
        last_updated: stats.last_updated,
        version: stats.version,
    }
}

// ============================================================================
// KG Store Wrapper
// ============================================================================

/// Wrapper around gik-db KG store that provides a simpler interface.
///
/// This provides access to the LanceDB-based KG store while keeping
/// the existing gik-core JSONL store as the default.
pub struct DbKgStore {
    inner: std::sync::Arc<dyn gik_db::kg::KgStoreBackend>,
}

impl DbKgStore {
    /// Open a KG store from gik-db configuration.
    pub fn open(config: &gik_db::kg::KgStoreConfig) -> Result<Self, GikError> {
        let store = gik_db::kg::open_kg_store(config).into_gik_result()?;
        Ok(Self {
            inner: store.into(),
        })
    }

    /// Upsert nodes to the store.
    pub fn upsert_nodes(&self, nodes: &[crate::kg::KgNode]) -> Result<usize, GikError> {
        let db_nodes: Vec<_> = nodes.iter().map(to_db_kg_node).collect();
        self.inner.upsert_nodes(&db_nodes).into_gik_result()
    }

    /// Upsert edges to the store.
    pub fn upsert_edges(&self, edges: &[crate::kg::KgEdge]) -> Result<usize, GikError> {
        let db_edges: Vec<_> = edges.iter().map(to_db_kg_edge).collect();
        self.inner.upsert_edges(&db_edges).into_gik_result()
    }

    /// Get all nodes from the store.
    pub fn get_all_nodes(&self) -> Result<Vec<crate::kg::KgNode>, GikError> {
        let db_nodes = self.inner.get_all_nodes().into_gik_result()?;
        Ok(db_nodes.into_iter().map(from_db_kg_node).collect())
    }

    /// Get all edges from the store.
    pub fn get_all_edges(&self) -> Result<Vec<crate::kg::KgEdge>, GikError> {
        let db_edges = self.inner.get_all_edges().into_gik_result()?;
        Ok(db_edges.into_iter().map(from_db_kg_edge).collect())
    }

    /// Get nodes by kind.
    pub fn get_nodes_by_kind(&self, kind: &str) -> Result<Vec<crate::kg::KgNode>, GikError> {
        let db_nodes = self.inner.get_nodes_by_kind(kind).into_gik_result()?;
        Ok(db_nodes.into_iter().map(from_db_kg_node).collect())
    }

    /// Get edges by kind.
    pub fn get_edges_by_kind(&self, kind: &str) -> Result<Vec<crate::kg::KgEdge>, GikError> {
        let db_edges = self.inner.get_edges_by_kind(kind).into_gik_result()?;
        Ok(db_edges.into_iter().map(from_db_kg_edge).collect())
    }

    /// Get stats from the store.
    pub fn get_stats(&self) -> Result<crate::kg::KgStats, GikError> {
        let db_stats = self.inner.get_stats().into_gik_result()?;
        Ok(from_db_kg_stats(db_stats))
    }

    /// Clear all data from the store.
    pub fn clear(&self) -> Result<(), GikError> {
        self.inner.clear().into_gik_result()
    }

    /// Flush changes to disk.
    pub fn flush(&self) -> Result<(), GikError> {
        self.inner.flush().into_gik_result()
    }
}

// ============================================================================
// Vector Index Wrapper
// ============================================================================

/// Wrapper around gik-db vector index for gik-core.
///
/// This adapts the gik-db `VectorIndexBackend` trait to match the gik-core
/// `VectorIndexBackend` trait signature.
///
/// Key differences handled:
/// - gik-db uses `&self` for mutable operations (internal locking)
/// - gik-core uses `&mut self` for mutable operations
/// - Type conversions between VectorInsert/VectorSearchResult
pub struct DbVectorIndex {
    inner: Arc<dyn gik_db::vector::VectorIndexBackend>,
    config: CoreVectorIndexConfig,
}

impl std::fmt::Debug for DbVectorIndex {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DbVectorIndex")
            .field("config", &self.config)
            .finish()
    }
}

impl DbVectorIndex {
    /// Create a new wrapper from a gik-db vector index.
    pub fn new(
        backend: Arc<dyn gik_db::vector::VectorIndexBackend>,
        config: CoreVectorIndexConfig,
    ) -> Self {
        Self {
            inner: backend,
            config,
        }
    }

    /// Open a vector index using gik-db configuration.
    pub fn open(db_config: &gik_db::vector::VectorIndexConfig) -> Result<Self, GikError> {
        let backend = gik_db::vector::open_vector_index(db_config).into_gik_result()?;
        let core_config = CoreVectorIndexConfig {
            backend: VectorIndexBackendKind::LanceDb,
            metric: CoreVectorMetric::Cosine,
            dimension: db_config.dimension as u32,
            base: String::new(), // Base is not stored in gik-db config
        };
        Ok(Self::new(backend, core_config))
    }

    /// Get the underlying gik-db backend.
    pub fn inner(&self) -> &dyn gik_db::vector::VectorIndexBackend {
        self.inner.as_ref()
    }
}

/// Convert a gik-core VectorInsert to a gik-db VectorInsert.
fn to_db_vector_insert(insert: &CoreVectorInsert) -> gik_db::vector::VectorInsert {
    // Extract metadata from payload
    let base = insert
        .payload
        .get("base")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let branch = insert
        .payload
        .get("branch")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let path = insert
        .payload
        .get("file_path")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    gik_db::vector::VectorInsert::new(
        gik_db::vector::VectorId::new(insert.id.0),
        insert.embedding.clone(),
        insert.payload.clone(),
    )
    .with_base(base)
    .with_branch(branch.unwrap_or_default())
    .with_path(path.unwrap_or_default())
}

/// Convert a gik-db VectorSearchResult to a gik-core VectorSearchResult.
fn from_db_search_result(result: gik_db::vector::VectorSearchResult) -> CoreVectorSearchResult {
    CoreVectorSearchResult {
        id: CoreVectorId(result.id.value()),
        score: result.score,
        payload: result.payload,
    }
}

/// Convert a gik-core VectorSearchFilter to a gik-db VectorSearchFilter.
fn to_db_filter(filter: &VectorSearchFilter) -> gik_db::vector::VectorSearchFilter {
    gik_db::vector::VectorSearchFilter {
        base: filter.base.clone(),
        branch: filter.branch.clone(),
        source_type: filter.source_type.clone(),
        path_prefix: filter.path_prefix.clone(),
        tags: filter.tags.clone(),
        revision_id: filter.revision_id.clone(),
    }
}

use crate::vector_index::VectorIndexBackend as CoreVectorIndexBackend;

impl CoreVectorIndexBackend for DbVectorIndex {
    fn backend_kind(&self) -> VectorIndexBackendKind {
        self.config.backend.clone()
    }

    fn config(&self) -> &CoreVectorIndexConfig {
        &self.config
    }

    fn stats(&self) -> Result<VectorIndexStats, GikError> {
        let count = self.inner.len().into_gik_result()?;
        Ok(VectorIndexStats {
            count: count as u64,
            dimension: self.config.dimension,
            backend: self.config.backend.to_string(),
            metric: self.config.metric.to_string(),
        })
    }

    fn upsert(&mut self, items: &[CoreVectorInsert]) -> Result<(), GikError> {
        let db_items: Vec<_> = items.iter().map(to_db_vector_insert).collect();
        self.inner.upsert(&db_items).into_gik_result()
    }

    fn query(&self, query: &[f32], top_k: u32) -> Result<Vec<CoreVectorSearchResult>, GikError> {
        let results = self
            .inner
            .query(query, top_k as usize, None)
            .into_gik_result()?;
        Ok(results.into_iter().map(from_db_search_result).collect())
    }

    fn query_filtered(
        &self,
        query: &[f32],
        top_k: u32,
        filter: Option<&VectorSearchFilter>,
    ) -> Result<Vec<CoreVectorSearchResult>, GikError> {
        let db_filter = filter.map(to_db_filter);
        let results = self
            .inner
            .query(query, top_k as usize, db_filter.as_ref())
            .into_gik_result()?;
        Ok(results.into_iter().map(from_db_search_result).collect())
    }

    fn delete(&mut self, ids: &[CoreVectorId]) -> Result<(), GikError> {
        let db_ids: Vec<_> = ids
            .iter()
            .map(|id| gik_db::vector::VectorId::new(id.0))
            .collect();
        self.inner.delete(&db_ids).into_gik_result()
    }

    fn flush(&mut self) -> Result<(), GikError> {
        self.inner.flush().into_gik_result()
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_error_conversion_io() {
        let io_err = std::io::Error::new(std::io::ErrorKind::NotFound, "file not found");
        let db_err = gik_db::DbError::Io(io_err);
        let gik_err = from_db_error(db_err);

        match gik_err {
            GikError::Io(_) => {}
            other => panic!("Expected GikError::Io, got {:?}", other),
        }
    }

    #[test]
    fn test_error_conversion_lancedb() {
        let db_err = gik_db::DbError::LanceDb {
            message: "connection failed".to_string(),
        };
        let gik_err = from_db_error(db_err);

        match gik_err {
            GikError::VectorIndexBackendUnavailable { backend, reason } => {
                assert_eq!(backend, "lancedb");
                assert!(reason.contains("connection failed"));
            }
            other => panic!("Expected VectorIndexBackendUnavailable, got {:?}", other),
        }
    }

    #[test]
    fn test_error_conversion_model_not_found() {
        let db_err = gik_db::DbError::ModelNotFound {
            model: "test-model".to_string(),
        };
        let gik_err = from_db_error(db_err);

        match gik_err {
            GikError::EmbeddingProviderUnavailable { provider, reason } => {
                assert_eq!(provider, "test-model");
                assert!(reason.contains("not found"));
            }
            other => panic!("Expected EmbeddingProviderUnavailable, got {:?}", other),
        }
    }

    #[test]
    fn test_kg_node_conversion() {
        let core_node = crate::kg::KgNode::new("test:id", "file", "test.rs");
        let db_node = to_db_kg_node(&core_node);
        let roundtrip = from_db_kg_node(db_node);

        assert_eq!(core_node.id, roundtrip.id);
        assert_eq!(core_node.kind, roundtrip.kind);
        assert_eq!(core_node.label, roundtrip.label);
    }

    #[test]
    fn test_kg_edge_conversion() {
        let core_edge = crate::kg::KgEdge::new("file:a.rs", "file:b.rs", "imports");
        let db_edge = to_db_kg_edge(&core_edge);
        let roundtrip = from_db_kg_edge(db_edge);

        assert_eq!(core_edge.from, roundtrip.from);
        assert_eq!(core_edge.to, roundtrip.to);
        assert_eq!(core_edge.kind, roundtrip.kind);
    }
}
