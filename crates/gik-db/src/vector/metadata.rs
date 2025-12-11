//! Vector index metadata types for rich filtering.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

// ============================================================================
// VectorMetadata
// ============================================================================

/// Canonical metadata stored with each vector in the index.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VectorMetadata {
    /// Stable identifier for this chunk.
    pub id: String,

    /// Knowledge base name (e.g., "code", "docs", "memory").
    pub base: String,

    /// Branch name (optional).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,

    /// Source type: "file", "memory", "url", "archive".
    pub source_type: String,

    /// File path or logical key.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,

    /// User-defined tags for filtering.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,

    /// Timeline revision ID that created this vector.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub revision_id: Option<String>,

    /// Timestamp when the vector was first indexed.
    pub created_at: DateTime<Utc>,

    /// Timestamp when the vector was last updated.
    pub updated_at: DateTime<Utc>,
}

impl VectorMetadata {
    /// Create new metadata with required fields.
    pub fn new(
        id: impl Into<String>,
        base: impl Into<String>,
        source_type: impl Into<String>,
    ) -> Self {
        let now = Utc::now();
        Self {
            id: id.into(),
            base: base.into(),
            branch: None,
            source_type: source_type.into(),
            path: None,
            tags: Vec::new(),
            revision_id: None,
            created_at: now,
            updated_at: now,
        }
    }

    /// Set the branch name.
    pub fn with_branch(mut self, branch: impl Into<String>) -> Self {
        self.branch = Some(branch.into());
        self
    }

    /// Set the file path or logical key.
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

    /// Set the created_at timestamp.
    pub fn with_created_at(mut self, created_at: DateTime<Utc>) -> Self {
        self.created_at = created_at;
        self
    }

    /// Update the updated_at timestamp to now.
    pub fn touch(&mut self) {
        self.updated_at = Utc::now();
    }
}

impl Default for VectorMetadata {
    fn default() -> Self {
        Self::new("", "", "file")
    }
}

// ============================================================================
// VectorSearchFilter
// ============================================================================

/// Filter criteria for vector search queries.
///
/// All fields are optional. When multiple fields are specified, they are
/// combined with AND logic. Empty/None fields are ignored.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VectorSearchFilter {
    /// Filter by exact base name.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub base: Option<String>,

    /// Filter by exact branch name.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,

    /// Filter by source type (e.g., "file", "memory").
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_type: Option<String>,

    /// Filter by path prefix (e.g., "src/").
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path_prefix: Option<String>,

    /// Filter to entries that have ALL of these tags.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,

    /// Filter by revision ID.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub revision_id: Option<String>,
}

impl VectorSearchFilter {
    /// Create an empty filter (matches all).
    pub fn new() -> Self {
        Self::default()
    }

    /// Filter by base name.
    pub fn with_base(mut self, base: impl Into<String>) -> Self {
        self.base = Some(base.into());
        self
    }

    /// Filter by branch name.
    pub fn with_branch(mut self, branch: impl Into<String>) -> Self {
        self.branch = Some(branch.into());
        self
    }

    /// Filter by source type.
    pub fn with_source_type(mut self, source_type: impl Into<String>) -> Self {
        self.source_type = Some(source_type.into());
        self
    }

    /// Filter by path prefix.
    pub fn with_path_prefix(mut self, prefix: impl Into<String>) -> Self {
        self.path_prefix = Some(prefix.into());
        self
    }

    /// Filter by tags (must have ALL specified tags).
    pub fn with_tags(mut self, tags: Vec<String>) -> Self {
        self.tags = tags;
        self
    }

    /// Filter by revision ID.
    pub fn with_revision_id(mut self, revision_id: impl Into<String>) -> Self {
        self.revision_id = Some(revision_id.into());
        self
    }

    /// Check if the filter is empty (matches all).
    pub fn is_empty(&self) -> bool {
        self.base.is_none()
            && self.branch.is_none()
            && self.source_type.is_none()
            && self.path_prefix.is_none()
            && self.tags.is_empty()
            && self.revision_id.is_none()
    }

    /// Build a SQL-like WHERE clause for LanceDB.
    ///
    /// Returns `None` if the filter is empty.
    pub fn to_lance_filter(&self) -> Option<String> {
        let mut conditions: Vec<String> = Vec::new();

        if let Some(base) = &self.base {
            conditions.push(format!("base = '{}'", escape_sql_string(base)));
        }

        if let Some(branch) = &self.branch {
            conditions.push(format!("branch = '{}'", escape_sql_string(branch)));
        }

        if let Some(source_type) = &self.source_type {
            conditions.push(format!(
                "source_type = '{}'",
                escape_sql_string(source_type)
            ));
        }

        if let Some(prefix) = &self.path_prefix {
            conditions.push(format!("path LIKE '{}%'", escape_sql_string(prefix)));
        }

        // Tags filtering: check that each tag is in the tags array
        for tag in &self.tags {
            conditions.push(format!(
                "array_contains(tags, '{}')",
                escape_sql_string(tag)
            ));
        }

        if let Some(revision_id) = &self.revision_id {
            conditions.push(format!(
                "revision_id = '{}'",
                escape_sql_string(revision_id)
            ));
        }

        if conditions.is_empty() {
            None
        } else {
            Some(conditions.join(" AND "))
        }
    }
}

/// Escape single quotes in SQL strings.
fn escape_sql_string(s: &str) -> String {
    s.replace('\'', "''")
}

// ============================================================================
// Source Type Constants
// ============================================================================

/// Source type for regular files.
pub const SOURCE_TYPE_FILE: &str = "file";

/// Source type for memory entries.
pub const SOURCE_TYPE_MEMORY: &str = "memory";

/// Source type for URL sources.
pub const SOURCE_TYPE_URL: &str = "url";

/// Source type for archive sources.
pub const SOURCE_TYPE_ARCHIVE: &str = "archive";

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_vector_metadata_new() {
        let meta = VectorMetadata::new("chunk-001", "code", "file");
        assert_eq!(meta.id, "chunk-001");
        assert_eq!(meta.base, "code");
        assert_eq!(meta.source_type, "file");
        assert!(meta.branch.is_none());
        assert!(meta.path.is_none());
        assert!(meta.tags.is_empty());
    }

    #[test]
    fn test_vector_metadata_builder() {
        let meta = VectorMetadata::new("chunk-001", "code", "file")
            .with_branch("main")
            .with_path("src/lib.rs")
            .with_tags(vec!["rust".to_string(), "core".to_string()])
            .with_revision_id("rev-123");

        assert_eq!(meta.branch, Some("main".to_string()));
        assert_eq!(meta.path, Some("src/lib.rs".to_string()));
        assert_eq!(meta.tags, vec!["rust", "core"]);
        assert_eq!(meta.revision_id, Some("rev-123".to_string()));
    }

    #[test]
    fn test_vector_metadata_serialization() {
        let meta = VectorMetadata::new("chunk-001", "code", "file")
            .with_branch("main")
            .with_path("src/lib.rs");

        let json = serde_json::to_string(&meta).unwrap();
        assert!(json.contains("\"id\":\"chunk-001\""));
        assert!(json.contains("\"base\":\"code\""));
        assert!(json.contains("\"sourceType\":\"file\""));
        assert!(json.contains("\"branch\":\"main\""));
    }

    #[test]
    fn test_vector_search_filter_empty() {
        let filter = VectorSearchFilter::new();
        assert!(filter.is_empty());
        assert!(filter.to_lance_filter().is_none());
    }

    #[test]
    fn test_vector_search_filter_single_field() {
        let filter = VectorSearchFilter::new().with_base("code");
        assert!(!filter.is_empty());
        assert_eq!(filter.to_lance_filter(), Some("base = 'code'".to_string()));
    }

    #[test]
    fn test_vector_search_filter_multiple_fields() {
        let filter = VectorSearchFilter::new()
            .with_base("code")
            .with_branch("main")
            .with_path_prefix("src/");

        let lance_filter = filter.to_lance_filter().unwrap();
        assert!(lance_filter.contains("base = 'code'"));
        assert!(lance_filter.contains("branch = 'main'"));
        assert!(lance_filter.contains("path LIKE 'src/%'"));
    }

    #[test]
    fn test_vector_search_filter_with_tags() {
        let filter =
            VectorSearchFilter::new().with_tags(vec!["rust".to_string(), "core".to_string()]);

        let lance_filter = filter.to_lance_filter().unwrap();
        assert!(lance_filter.contains("array_contains(tags, 'rust')"));
        assert!(lance_filter.contains("array_contains(tags, 'core')"));
    }

    #[test]
    fn test_escape_sql_string() {
        assert_eq!(escape_sql_string("hello"), "hello");
        assert_eq!(escape_sql_string("it's"), "it''s");
        assert_eq!(escape_sql_string("a'b'c"), "a''b''c");
    }
}
