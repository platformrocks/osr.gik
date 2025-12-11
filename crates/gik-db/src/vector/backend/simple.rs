//! Simple file-based vector index backend.
//!
//! This backend stores vectors in a JSONL file and uses linear scan for search.
//! It is intended for testing and small indexes where the overhead of a full
//! vector database is not justified.

use super::super::config::VectorIndexConfig;
use super::super::metadata::VectorSearchFilter;
use super::super::traits::{
    VectorId, VectorIndexBackend, VectorInsert, VectorMetric, VectorSearchResult,
};
use crate::error::{DbError, DbResult};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs::File;
use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::sync::RwLock;
use tracing::{debug, trace};

/// Filename for the JSONL data file.
const DATA_FILENAME: &str = "vectors.jsonl";

/// A stored vector entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct StoredVector {
    id: u64,
    vector: Vec<f32>,
    payload: serde_json::Value,
    base: String,
    branch: Option<String>,
    source_type: String,
    path: Option<String>,
    tags: Vec<String>,
    revision_id: Option<String>,
}

impl From<&VectorInsert> for StoredVector {
    fn from(insert: &VectorInsert) -> Self {
        Self {
            id: insert.id.value(),
            vector: insert.vector.clone(),
            payload: insert.payload.clone(),
            base: insert.base.clone(),
            branch: insert.branch.clone(),
            source_type: insert.source_type.clone(),
            path: insert.path.clone(),
            tags: insert.tags.clone(),
            revision_id: insert.revision_id.clone(),
        }
    }
}

/// Simple file-based vector index.
///
/// Uses JSONL storage and linear scan for search.
pub struct SimpleFileVectorIndex {
    /// Path to the index directory.
    path: PathBuf,

    /// Dimension of vectors.
    dimension: usize,

    /// Distance metric.
    metric: VectorMetric,

    /// In-memory vector store.
    vectors: RwLock<HashMap<u64, StoredVector>>,
}

impl SimpleFileVectorIndex {
    /// Open or create a simple file vector index.
    pub fn open(config: &VectorIndexConfig) -> DbResult<Self> {
        debug!("Opening SimpleFileVectorIndex at {:?}", config.path);

        let index = Self {
            path: config.path.clone(),
            dimension: config.dimension,
            metric: config.metric,
            vectors: RwLock::new(HashMap::new()),
        };

        // Load existing data if present
        let data_path = config.path.join(DATA_FILENAME);
        if data_path.exists() {
            index.load_from_file(&data_path)?;
        }

        Ok(index)
    }

    /// Load vectors from a JSONL file.
    fn load_from_file(&self, path: &PathBuf) -> DbResult<()> {
        debug!("Loading vectors from {:?}", path);

        let file = File::open(path)?;
        let reader = BufReader::new(file);

        let mut vectors = self
            .vectors
            .write()
            .map_err(|e| DbError::internal(format!("Failed to acquire write lock: {}", e)))?;

        for (line_num, line) in reader.lines().enumerate() {
            let line = line?;
            if line.trim().is_empty() {
                continue;
            }

            match serde_json::from_str::<StoredVector>(&line) {
                Ok(stored) => {
                    vectors.insert(stored.id, stored);
                }
                Err(e) => {
                    debug!("Skipping invalid line {}: {}", line_num + 1, e);
                }
            }
        }

        debug!("Loaded {} vectors", vectors.len());
        Ok(())
    }

    /// Save all vectors to the JSONL file.
    fn save_to_file(&self) -> DbResult<()> {
        let data_path = self.path.join(DATA_FILENAME);
        debug!("Saving vectors to {:?}", data_path);

        let vectors = self
            .vectors
            .read()
            .map_err(|e| DbError::internal(format!("Failed to acquire read lock: {}", e)))?;

        let mut file = File::create(&data_path)?;
        for stored in vectors.values() {
            let line = serde_json::to_string(stored)?;
            writeln!(file, "{}", line)?;
        }

        debug!("Saved {} vectors", vectors.len());
        Ok(())
    }

    /// Check if a vector matches the filter.
    fn matches_filter(stored: &StoredVector, filter: &VectorSearchFilter) -> bool {
        // Check base
        if let Some(ref base) = filter.base {
            if &stored.base != base {
                return false;
            }
        }

        // Check branch
        if let Some(ref branch) = filter.branch {
            match &stored.branch {
                Some(b) if b == branch => {}
                _ => return false,
            }
        }

        // Check source_type
        if let Some(ref source_type) = filter.source_type {
            if &stored.source_type != source_type {
                return false;
            }
        }

        // Check path prefix
        if let Some(ref prefix) = filter.path_prefix {
            match &stored.path {
                Some(p) if p.starts_with(prefix) => {}
                _ => return false,
            }
        }

        // Check tags (must have ALL)
        for tag in &filter.tags {
            if !stored.tags.contains(tag) {
                return false;
            }
        }

        // Check revision_id
        if let Some(ref revision_id) = filter.revision_id {
            match &stored.revision_id {
                Some(r) if r == revision_id => {}
                _ => return false,
            }
        }

        true
    }

    /// Compute similarity between two vectors.
    fn compute_similarity(&self, a: &[f32], b: &[f32]) -> f32 {
        match self.metric {
            VectorMetric::Cosine => cosine_similarity(a, b),
            VectorMetric::Dot => dot_product(a, b),
            VectorMetric::L2 => -euclidean_distance(a, b), // Negate so higher is better
        }
    }
}

impl VectorIndexBackend for SimpleFileVectorIndex {
    fn query(
        &self,
        embedding: &[f32],
        limit: usize,
        filter: Option<&VectorSearchFilter>,
    ) -> DbResult<Vec<VectorSearchResult>> {
        trace!("Querying SimpleFileVectorIndex, limit={}", limit);

        let vectors = self
            .vectors
            .read()
            .map_err(|e| DbError::internal(format!("Failed to acquire read lock: {}", e)))?;

        // Compute similarities
        let mut scored: Vec<(f32, &StoredVector)> = vectors
            .values()
            .filter(|v| filter.map(|f| Self::matches_filter(v, f)).unwrap_or(true))
            .map(|v| (self.compute_similarity(embedding, &v.vector), v))
            .collect();

        // Sort by score (descending)
        scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));

        // Take top results
        let results: Vec<VectorSearchResult> = scored
            .into_iter()
            .take(limit)
            .map(|(score, stored)| {
                VectorSearchResult::new(VectorId::new(stored.id), score, stored.payload.clone())
            })
            .collect();

        trace!("Found {} results", results.len());
        Ok(results)
    }

    fn upsert(&self, vectors: &[VectorInsert]) -> DbResult<()> {
        debug!("Upserting {} vectors", vectors.len());

        let mut stored = self
            .vectors
            .write()
            .map_err(|e| DbError::internal(format!("Failed to acquire write lock: {}", e)))?;

        for insert in vectors {
            // Validate dimension
            if insert.vector.len() != self.dimension {
                return Err(DbError::DimensionMismatch {
                    expected: self.dimension,
                    actual: insert.vector.len(),
                });
            }

            let entry = StoredVector::from(insert);
            stored.insert(entry.id, entry);
        }

        // Persist immediately
        drop(stored);
        self.save_to_file()?;

        Ok(())
    }

    fn delete(&self, ids: &[VectorId]) -> DbResult<()> {
        debug!("Deleting {} vectors", ids.len());

        let mut stored = self
            .vectors
            .write()
            .map_err(|e| DbError::internal(format!("Failed to acquire write lock: {}", e)))?;

        for id in ids {
            stored.remove(&id.value());
        }

        // Persist immediately
        drop(stored);
        self.save_to_file()?;

        Ok(())
    }

    fn flush(&self) -> DbResult<()> {
        self.save_to_file()
    }

    fn len(&self) -> DbResult<usize> {
        let stored = self
            .vectors
            .read()
            .map_err(|e| DbError::internal(format!("Failed to acquire read lock: {}", e)))?;
        Ok(stored.len())
    }

    fn dimension(&self) -> usize {
        self.dimension
    }

    fn metric(&self) -> VectorMetric {
        self.metric
    }
}

// ============================================================================
// Similarity Functions
// ============================================================================

/// Compute cosine similarity between two vectors.
fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
    let dot: f32 = a.iter().zip(b.iter()).map(|(x, y)| x * y).sum();
    let norm_a: f32 = a.iter().map(|x| x * x).sum::<f32>().sqrt();
    let norm_b: f32 = b.iter().map(|x| x * x).sum::<f32>().sqrt();

    if norm_a == 0.0 || norm_b == 0.0 {
        return 0.0;
    }

    dot / (norm_a * norm_b)
}

/// Compute dot product between two vectors.
fn dot_product(a: &[f32], b: &[f32]) -> f32 {
    a.iter().zip(b.iter()).map(|(x, y)| x * y).sum()
}

/// Compute Euclidean (L2) distance between two vectors.
fn euclidean_distance(a: &[f32], b: &[f32]) -> f32 {
    a.iter()
        .zip(b.iter())
        .map(|(x, y)| (x - y).powi(2))
        .sum::<f32>()
        .sqrt()
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cosine_similarity() {
        let a = vec![1.0, 0.0, 0.0];
        let b = vec![1.0, 0.0, 0.0];
        assert!((cosine_similarity(&a, &b) - 1.0).abs() < 1e-6);

        let c = vec![0.0, 1.0, 0.0];
        assert!(cosine_similarity(&a, &c).abs() < 1e-6);
    }

    #[test]
    fn test_dot_product() {
        let a = vec![1.0, 2.0, 3.0];
        let b = vec![4.0, 5.0, 6.0];
        assert!((dot_product(&a, &b) - 32.0).abs() < 1e-6);
    }

    #[test]
    fn test_euclidean_distance() {
        let a = vec![0.0, 0.0, 0.0];
        let b = vec![3.0, 4.0, 0.0];
        assert!((euclidean_distance(&a, &b) - 5.0).abs() < 1e-6);
    }

    #[test]
    fn test_filter_matching() {
        let stored = StoredVector {
            id: 123,
            vector: vec![1.0, 2.0, 3.0],
            payload: serde_json::json!({}),
            base: "code".to_string(),
            branch: Some("main".to_string()),
            source_type: "file".to_string(),
            path: Some("src/lib.rs".to_string()),
            tags: vec!["rust".to_string()],
            revision_id: None,
        };

        // Empty filter matches all
        let filter = VectorSearchFilter::new();
        assert!(SimpleFileVectorIndex::matches_filter(&stored, &filter));

        // Matching base
        let filter = VectorSearchFilter::new().with_base("code");
        assert!(SimpleFileVectorIndex::matches_filter(&stored, &filter));

        // Non-matching base
        let filter = VectorSearchFilter::new().with_base("docs");
        assert!(!SimpleFileVectorIndex::matches_filter(&stored, &filter));

        // Matching path prefix
        let filter = VectorSearchFilter::new().with_path_prefix("src/");
        assert!(SimpleFileVectorIndex::matches_filter(&stored, &filter));
    }
}
