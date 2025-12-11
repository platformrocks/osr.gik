//! Vector index configuration and metadata.

use super::traits::VectorMetric;
use crate::error::{DbError, DbResult};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use tracing::debug;

// ============================================================================
// Constants
// ============================================================================

/// Default backend name.
pub const DEFAULT_BACKEND: &str = "lancedb";

/// Filename for index metadata.
pub const INDEX_META_FILENAME: &str = "index.meta.json";

/// LanceDB table name.
pub const LANCEDB_TABLE_NAME: &str = "vectors";

// ============================================================================
// VectorIndexConfig
// ============================================================================

/// Configuration for creating or opening a vector index.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VectorIndexConfig {
    /// Dimension of vectors in the index.
    pub dimension: usize,

    /// Path to the index directory.
    pub path: PathBuf,

    /// Backend to use (e.g., "lancedb", "simple").
    #[serde(default = "default_backend")]
    pub backend: String,

    /// Distance metric for similarity search.
    #[serde(default)]
    pub metric: VectorMetric,

    /// Whether to create the index if it doesn't exist.
    #[serde(default = "default_create_if_missing")]
    pub create_if_missing: bool,
}

fn default_backend() -> String {
    DEFAULT_BACKEND.to_string()
}

fn default_create_if_missing() -> bool {
    true
}

impl VectorIndexConfig {
    /// Create a new config with required fields.
    pub fn new(dimension: usize, path: impl Into<PathBuf>) -> Self {
        Self {
            dimension,
            path: path.into(),
            backend: DEFAULT_BACKEND.to_string(),
            metric: VectorMetric::Cosine,
            create_if_missing: true,
        }
    }

    /// Set the backend.
    pub fn with_backend(mut self, backend: impl Into<String>) -> Self {
        self.backend = backend.into();
        self
    }

    /// Set the distance metric.
    pub fn with_metric(mut self, metric: VectorMetric) -> Self {
        self.metric = metric;
        self
    }

    /// Set whether to create the index if missing.
    pub fn with_create_if_missing(mut self, create: bool) -> Self {
        self.create_if_missing = create;
        self
    }
}

// ============================================================================
// VectorIndexMeta
// ============================================================================

/// Metadata for a persisted vector index.
///
/// This is stored in `index.meta.json` alongside the index data.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VectorIndexMeta {
    /// Backend used for this index.
    pub backend: String,

    /// Dimension of vectors.
    pub dimension: usize,

    /// Distance metric.
    pub metric: VectorMetric,

    /// Number of vectors (approximate, may be stale).
    #[serde(default)]
    pub count: usize,

    /// Schema version for future migrations.
    #[serde(default = "default_schema_version")]
    pub schema_version: u32,

    /// Creation timestamp (ISO 8601).
    #[serde(default)]
    pub created_at: Option<String>,

    /// Last update timestamp (ISO 8601).
    #[serde(default)]
    pub updated_at: Option<String>,
}

fn default_schema_version() -> u32 {
    1
}

impl VectorIndexMeta {
    /// Create new metadata.
    pub fn new(backend: impl Into<String>, dimension: usize, metric: VectorMetric) -> Self {
        let now = chrono::Utc::now().to_rfc3339();
        Self {
            backend: backend.into(),
            dimension,
            metric,
            count: 0,
            schema_version: 1,
            created_at: Some(now.clone()),
            updated_at: Some(now),
        }
    }

    /// Update the count and timestamp.
    pub fn update_count(&mut self, count: usize) {
        self.count = count;
        self.updated_at = Some(chrono::Utc::now().to_rfc3339());
    }
}

// ============================================================================
// VectorIndexCompatibility
// ============================================================================

/// Result of checking index compatibility.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VectorIndexCompatibility {
    /// Index is compatible and can be opened.
    Compatible,

    /// Index doesn't exist and should be created.
    NotFound,

    /// Index exists but has incompatible dimension.
    IncompatibleDimension { expected: usize, actual: usize },

    /// Index exists but uses a different backend.
    IncompatibleBackend { expected: String, actual: String },

    /// Index exists but uses a different metric.
    IncompatibleMetric {
        expected: VectorMetric,
        actual: VectorMetric,
    },

    /// Index metadata is corrupted or unreadable.
    Corrupted(String),
}

impl VectorIndexCompatibility {
    /// Check if the index is compatible.
    pub fn is_compatible(&self) -> bool {
        matches!(self, VectorIndexCompatibility::Compatible)
    }

    /// Check if the index doesn't exist.
    pub fn is_not_found(&self) -> bool {
        matches!(self, VectorIndexCompatibility::NotFound)
    }
}

// ============================================================================
// Helper Functions
// ============================================================================

/// Check if an existing index is compatible with the given config.
pub fn check_index_compatibility(config: &VectorIndexConfig) -> VectorIndexCompatibility {
    let meta_path = config.path.join(INDEX_META_FILENAME);

    if !meta_path.exists() {
        // Check if the directory exists but has no metadata
        if config.path.exists() && config.path.is_dir() {
            // Directory exists but no metadata - could be empty or corrupted
            let entries = config.path.read_dir().map(|rd| rd.count()).unwrap_or(0);
            if entries == 0 {
                return VectorIndexCompatibility::NotFound;
            }
            // Has files but no metadata - corrupted
            return VectorIndexCompatibility::Corrupted(
                "Index directory exists but has no metadata".to_string(),
            );
        }
        return VectorIndexCompatibility::NotFound;
    }

    // Load and check metadata
    match load_index_meta(&config.path) {
        Ok(meta) => {
            // Check dimension
            if meta.dimension != config.dimension {
                return VectorIndexCompatibility::IncompatibleDimension {
                    expected: config.dimension,
                    actual: meta.dimension,
                };
            }

            // Check backend
            if meta.backend != config.backend {
                return VectorIndexCompatibility::IncompatibleBackend {
                    expected: config.backend.clone(),
                    actual: meta.backend,
                };
            }

            // Check metric
            if meta.metric != config.metric {
                return VectorIndexCompatibility::IncompatibleMetric {
                    expected: config.metric,
                    actual: meta.metric,
                };
            }

            VectorIndexCompatibility::Compatible
        }
        Err(e) => VectorIndexCompatibility::Corrupted(e.to_string()),
    }
}

/// Load index metadata from a directory.
pub fn load_index_meta(path: &Path) -> DbResult<VectorIndexMeta> {
    let meta_path = path.join(INDEX_META_FILENAME);
    debug!("Loading index metadata from {:?}", meta_path);

    let content = fs::read_to_string(&meta_path).map_err(|e| DbError::VectorIo {
        path: meta_path.clone(),
        message: format!("Failed to read index metadata: {}", e),
    })?;

    let meta: VectorIndexMeta =
        serde_json::from_str(&content).map_err(|e| DbError::VectorParse {
            path: meta_path,
            message: format!("Failed to parse index metadata: {}", e),
        })?;

    Ok(meta)
}

/// Write index metadata to a directory.
pub fn write_index_meta(path: &Path, meta: &VectorIndexMeta) -> DbResult<()> {
    let meta_path = path.join(INDEX_META_FILENAME);
    debug!("Writing index metadata to {:?}", meta_path);

    // Ensure directory exists
    if !path.exists() {
        fs::create_dir_all(path)?;
    }

    let content = serde_json::to_string_pretty(meta)?;
    fs::write(&meta_path, content)?;

    Ok(())
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_config_builder() {
        let config = VectorIndexConfig::new(1024, "/tmp/test")
            .with_backend("simple")
            .with_metric(VectorMetric::L2)
            .with_create_if_missing(false);

        assert_eq!(config.dimension, 1024);
        assert_eq!(config.backend, "simple");
        assert_eq!(config.metric, VectorMetric::L2);
        assert!(!config.create_if_missing);
    }

    #[test]
    fn test_meta_serialization() {
        let meta = VectorIndexMeta::new("lancedb", 1024, VectorMetric::Cosine);

        let json = serde_json::to_string(&meta).unwrap();
        assert!(json.contains("\"backend\":\"lancedb\""));
        assert!(json.contains("\"dimension\":1024"));
        assert!(json.contains("\"metric\":\"cosine\""));

        let parsed: VectorIndexMeta = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.backend, "lancedb");
        assert_eq!(parsed.dimension, 1024);
    }

    #[test]
    fn test_compatibility_check_not_found() {
        let config = VectorIndexConfig::new(1024, "/nonexistent/path/xyz123");
        let compat = check_index_compatibility(&config);
        assert!(compat.is_not_found());
    }
}
