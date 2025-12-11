//! Vector index traits and core types.
//!
//! This module defines the core abstraction for vector storage backends.

use crate::error::DbResult;
use serde::{Deserialize, Serialize};

// Re-import VectorSearchFilter for trait signature
use super::metadata::VectorSearchFilter;

// ============================================================================
// VectorId
// ============================================================================

/// Unique identifier for a vector in the index.
///
/// Uses a u64 for efficiency and compatibility with gik-core.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct VectorId(pub u64);

impl VectorId {
    /// Create a new vector ID.
    pub fn new(id: u64) -> Self {
        VectorId(id)
    }

    /// Get the underlying ID value.
    pub fn value(&self) -> u64 {
        self.0
    }
}

impl From<u64> for VectorId {
    fn from(id: u64) -> Self {
        VectorId(id)
    }
}

impl From<i64> for VectorId {
    fn from(id: i64) -> Self {
        VectorId(id as u64)
    }
}

impl std::fmt::Display for VectorId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

// ============================================================================
// VectorMetric
// ============================================================================

/// Distance metric for vector similarity search.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum VectorMetric {
    /// Cosine similarity (default).
    #[default]
    Cosine,
    /// Dot product.
    Dot,
    /// Euclidean (L2) distance.
    L2,
}

impl VectorMetric {
    /// Get the metric name as a string.
    pub fn as_str(&self) -> &'static str {
        match self {
            VectorMetric::Cosine => "cosine",
            VectorMetric::Dot => "dot",
            VectorMetric::L2 => "l2",
        }
    }
}

impl std::fmt::Display for VectorMetric {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

// ============================================================================
// VectorInsert
// ============================================================================

/// A vector to insert or update in the index.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VectorInsert {
    /// Unique identifier for this vector.
    pub id: VectorId,

    /// The embedding vector.
    pub vector: Vec<f32>,

    /// JSON payload with metadata.
    pub payload: serde_json::Value,

    /// Knowledge base name (e.g., "code", "docs", "memory").
    #[serde(default)]
    pub base: String,

    /// Branch name (optional).
    #[serde(default)]
    pub branch: Option<String>,

    /// Source type: "file", "memory", "url", etc.
    #[serde(default = "default_source_type")]
    pub source_type: String,

    /// File path or logical key.
    #[serde(default)]
    pub path: Option<String>,

    /// User-defined tags.
    #[serde(default)]
    pub tags: Vec<String>,

    /// Timeline revision ID.
    #[serde(default)]
    pub revision_id: Option<String>,
}

fn default_source_type() -> String {
    "file".to_string()
}

impl VectorInsert {
    /// Create a new vector insert with required fields.
    pub fn new(id: impl Into<VectorId>, vector: Vec<f32>, payload: serde_json::Value) -> Self {
        Self {
            id: id.into(),
            vector,
            payload,
            base: String::new(),
            branch: None,
            source_type: "file".to_string(),
            path: None,
            tags: Vec::new(),
            revision_id: None,
        }
    }

    /// Set the base name.
    pub fn with_base(mut self, base: impl Into<String>) -> Self {
        self.base = base.into();
        self
    }

    /// Set the branch name.
    pub fn with_branch(mut self, branch: impl Into<String>) -> Self {
        self.branch = Some(branch.into());
        self
    }

    /// Set the source type.
    pub fn with_source_type(mut self, source_type: impl Into<String>) -> Self {
        self.source_type = source_type.into();
        self
    }

    /// Set the path.
    pub fn with_path(mut self, path: impl Into<String>) -> Self {
        self.path = Some(path.into());
        self
    }

    /// Set the tags.
    pub fn with_tags(mut self, tags: Vec<String>) -> Self {
        self.tags = tags;
        self
    }

    /// Set the revision ID.
    pub fn with_revision_id(mut self, revision_id: impl Into<String>) -> Self {
        self.revision_id = Some(revision_id.into());
        self
    }
}

// ============================================================================
// VectorSearchResult
// ============================================================================

/// A single result from a vector similarity search.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VectorSearchResult {
    /// Unique identifier of the matched vector.
    pub id: VectorId,

    /// Similarity score (higher is better for cosine/dot, lower for L2).
    pub score: f32,

    /// JSON payload associated with this vector.
    pub payload: serde_json::Value,

    /// The embedding vector (optional, may not be returned by all backends).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub vector: Option<Vec<f32>>,
}

impl VectorSearchResult {
    /// Create a new search result.
    pub fn new(id: impl Into<VectorId>, score: f32, payload: serde_json::Value) -> Self {
        Self {
            id: id.into(),
            score,
            payload,
            vector: None,
        }
    }

    /// Set the vector.
    pub fn with_vector(mut self, vector: Vec<f32>) -> Self {
        self.vector = Some(vector);
        self
    }
}

// ============================================================================
// VectorIndexBackend Trait
// ============================================================================

/// Core trait for vector index backends.
///
/// This trait defines the interface that all vector storage backends must implement.
/// It provides methods for inserting, querying, and deleting vectors.
///
/// ## Implementation Notes
///
/// - Backends should be thread-safe (implement `Send + Sync`).
/// - The `query` method should return results sorted by relevance (best first).
/// - Upsert semantics: if a vector with the same ID exists, it should be replaced.
pub trait VectorIndexBackend: Send + Sync {
    /// Query the index for similar vectors.
    ///
    /// # Arguments
    /// * `embedding` - The query vector.
    /// * `limit` - Maximum number of results to return.
    /// * `filter` - Optional filter criteria.
    ///
    /// # Returns
    /// A list of search results sorted by relevance (best first).
    fn query(
        &self,
        embedding: &[f32],
        limit: usize,
        filter: Option<&VectorSearchFilter>,
    ) -> DbResult<Vec<VectorSearchResult>>;

    /// Insert or update vectors in the index.
    ///
    /// Uses upsert semantics: if a vector with the same ID exists, it is replaced.
    fn upsert(&self, vectors: &[VectorInsert]) -> DbResult<()>;

    /// Delete vectors by their IDs.
    fn delete(&self, ids: &[VectorId]) -> DbResult<()>;

    /// Flush pending writes to persistent storage.
    ///
    /// Some backends may buffer writes for performance. This method ensures
    /// all data is persisted.
    fn flush(&self) -> DbResult<()>;

    /// Get the number of vectors in the index.
    fn len(&self) -> DbResult<usize>;

    /// Check if the index is empty.
    fn is_empty(&self) -> DbResult<bool> {
        Ok(self.len()? == 0)
    }

    /// Get the dimension of vectors in this index.
    fn dimension(&self) -> usize;

    /// Get the distance metric used by this index.
    fn metric(&self) -> VectorMetric;
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_vector_id() {
        let id = VectorId::new(123);
        assert_eq!(id.value(), 123);
        assert_eq!(id.to_string(), "123");

        let id_from_u64: VectorId = 456u64.into();
        assert_eq!(id_from_u64.value(), 456);
    }

    #[test]
    fn test_vector_metric() {
        assert_eq!(VectorMetric::Cosine.as_str(), "cosine");
        assert_eq!(VectorMetric::Dot.as_str(), "dot");
        assert_eq!(VectorMetric::L2.as_str(), "l2");
        assert_eq!(VectorMetric::default(), VectorMetric::Cosine);
    }

    #[test]
    fn test_vector_insert_builder() {
        let insert = VectorInsert::new(
            1u64,
            vec![1.0, 2.0, 3.0],
            serde_json::json!({"key": "value"}),
        )
        .with_base("code")
        .with_branch("main")
        .with_path("src/lib.rs")
        .with_tags(vec!["rust".to_string()])
        .with_revision_id("rev-001");

        assert_eq!(insert.id.value(), 1);
        assert_eq!(insert.base, "code");
        assert_eq!(insert.branch, Some("main".to_string()));
        assert_eq!(insert.path, Some("src/lib.rs".to_string()));
        assert_eq!(insert.tags, vec!["rust"]);
        assert_eq!(insert.revision_id, Some("rev-001".to_string()));
    }

    #[test]
    fn test_vector_search_result() {
        let result = VectorSearchResult::new(1u64, 0.95, serde_json::json!({"text": "hello"}))
            .with_vector(vec![1.0, 2.0, 3.0]);

        assert_eq!(result.id.value(), 1);
        assert_eq!(result.score, 0.95);
        assert!(result.vector.is_some());
    }
}
