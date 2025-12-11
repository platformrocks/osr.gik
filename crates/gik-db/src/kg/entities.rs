//! Knowledge Graph entity definitions.
//!
//! This module defines the core entities for the Knowledge Graph (KG):
//! - [`KgNode`] - A node in the knowledge graph
//! - [`KgEdge`] - An edge representing a relationship between two nodes
//! - [`KgStats`] - Aggregate statistics for the knowledge graph
//!
//! ## Storage Format
//!
//! Entities are designed to be stored in various backends:
//! - LanceDB: Stored as Arrow record batches with efficient querying
//! - JSONL: One JSON object per line (legacy/testing)
//!
//! ## JSON Field Names
//!
//! All structs use camelCase for JSON serialization to maintain consistency
//! with the rest of the GIK contracts.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

// ============================================================================
// Constants
// ============================================================================

/// The current KG schema version.
pub const KG_VERSION: &str = "kg-v1";

// ============================================================================
// KgNode
// ============================================================================

/// A node in the knowledge graph.
///
/// Nodes represent entities such as files, modules, functions, classes, concepts,
/// or any other identifiable element in the codebase or documentation.
///
/// ## Fields
///
/// - `id` - Unique identifier for the node (e.g., "file:src/main.rs", "fn:main")
/// - `kind` - Type of node (e.g., "file", "module", "function", "class", "concept")
/// - `label` - Human-readable label for display
/// - `props` - Arbitrary properties as a JSON object
/// - `branch` - Optional branch name if this node is branch-specific
/// - `created_at` - Timestamp when the node was created
/// - `updated_at` - Timestamp when the node was last updated
///
/// ## Example
///
/// ```json
/// {
///   "id": "file:src/main.rs",
///   "kind": "file",
///   "label": "src/main.rs",
///   "props": {"language": "rust", "lines": 150},
///   "branch": "main",
///   "createdAt": "2025-11-28T10:00:00Z",
///   "updatedAt": "2025-11-28T10:00:00Z"
/// }
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KgNode {
    /// Unique node identifier.
    ///
    /// Convention: `<type>:<path>` (e.g., "file:src/main.rs", "fn:lib::parse")
    pub id: String,

    /// Node type/category.
    ///
    /// Common values: "file", "module", "function", "class", "struct", "trait",
    /// "concept", "dependency", "service", "endpoint".
    pub kind: String,

    /// Human-readable label for display.
    pub label: String,

    /// Arbitrary properties as a JSON object.
    ///
    /// Can include language, line count, complexity metrics, tags, etc.
    #[serde(default)]
    pub props: serde_json::Value,

    /// Optional branch name if this node is branch-specific.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,

    /// Timestamp when the node was created.
    pub created_at: DateTime<Utc>,

    /// Timestamp when the node was last updated.
    pub updated_at: DateTime<Utc>,
}

impl KgNode {
    /// Create a new node with the given id, kind, and label.
    ///
    /// Sets `created_at` and `updated_at` to the current time.
    /// Props default to an empty JSON object.
    pub fn new(id: impl Into<String>, kind: impl Into<String>, label: impl Into<String>) -> Self {
        let now = Utc::now();
        Self {
            id: id.into(),
            kind: kind.into(),
            label: label.into(),
            props: serde_json::Value::Object(serde_json::Map::new()),
            branch: None,
            created_at: now,
            updated_at: now,
        }
    }

    /// Set the properties for this node.
    pub fn with_props(mut self, props: serde_json::Value) -> Self {
        self.props = props;
        self
    }

    /// Set the branch for this node.
    pub fn with_branch(mut self, branch: impl Into<String>) -> Self {
        self.branch = Some(branch.into());
        self
    }

    /// Update the `updated_at` timestamp to now.
    pub fn touch(&mut self) {
        self.updated_at = Utc::now();
    }
}

// ============================================================================
// KgEdge
// ============================================================================

/// An edge in the knowledge graph representing a relationship between two nodes.
///
/// Edges are directed and connect a source node (`from`) to a target node (`to`).
///
/// ## Fields
///
/// - `id` - Unique identifier for the edge
/// - `from` - Source node ID
/// - `to` - Target node ID
/// - `kind` - Relationship type (e.g., "dependsOn", "calls", "contains", "imports")
/// - `props` - Arbitrary properties as a JSON object
/// - `branch` - Optional branch name if this edge is branch-specific
/// - `created_at` - Timestamp when the edge was created
/// - `updated_at` - Timestamp when the edge was last updated
///
/// ## Example
///
/// ```json
/// {
///   "id": "edge:file:src/main.rs->file:src/lib.rs:imports",
///   "from": "file:src/main.rs",
///   "to": "file:src/lib.rs",
///   "kind": "imports",
///   "props": {"count": 3},
///   "createdAt": "2025-11-28T10:00:00Z",
///   "updatedAt": "2025-11-28T10:00:00Z"
/// }
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KgEdge {
    /// Unique edge identifier.
    ///
    /// Convention: `edge:<from>-><to>:<kind>` or a generated UUID.
    pub id: String,

    /// Source node ID.
    pub from: String,

    /// Target node ID.
    pub to: String,

    /// Relationship type.
    ///
    /// Common values: "dependsOn", "calls", "contains", "imports", "extends",
    /// "implements", "uses", "ownedBy", "relatedTo".
    pub kind: String,

    /// Arbitrary properties as a JSON object.
    ///
    /// Can include weight, count, confidence score, etc.
    #[serde(default)]
    pub props: serde_json::Value,

    /// Optional branch name if this edge is branch-specific.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,

    /// Timestamp when the edge was created.
    pub created_at: DateTime<Utc>,

    /// Timestamp when the edge was last updated.
    pub updated_at: DateTime<Utc>,
}

impl KgEdge {
    /// Create a new edge with the given from, to, and kind.
    ///
    /// Generates an ID based on the edge components.
    /// Sets `created_at` and `updated_at` to the current time.
    /// Props default to an empty JSON object.
    pub fn new(from: impl Into<String>, to: impl Into<String>, kind: impl Into<String>) -> Self {
        let now = Utc::now();
        let from = from.into();
        let to = to.into();
        let kind = kind.into();
        let id = format!("edge:{}->{kind}", Self::edge_id_component(&from, &to));

        Self {
            id,
            from,
            to,
            kind,
            props: serde_json::Value::Object(serde_json::Map::new()),
            branch: None,
            created_at: now,
            updated_at: now,
        }
    }

    /// Create a new edge with an explicit ID.
    pub fn with_id(
        id: impl Into<String>,
        from: impl Into<String>,
        to: impl Into<String>,
        kind: impl Into<String>,
    ) -> Self {
        let now = Utc::now();
        Self {
            id: id.into(),
            from: from.into(),
            to: to.into(),
            kind: kind.into(),
            props: serde_json::Value::Object(serde_json::Map::new()),
            branch: None,
            created_at: now,
            updated_at: now,
        }
    }

    /// Set the properties for this edge.
    pub fn with_props(mut self, props: serde_json::Value) -> Self {
        self.props = props;
        self
    }

    /// Set the branch for this edge.
    pub fn with_branch(mut self, branch: impl Into<String>) -> Self {
        self.branch = Some(branch.into());
        self
    }

    /// Update the `updated_at` timestamp to now.
    pub fn touch(&mut self) {
        self.updated_at = Utc::now();
    }

    /// Generate a unique ID component from the from and to node IDs.
    fn edge_id_component(from: &str, to: &str) -> String {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};

        let mut hasher = DefaultHasher::new();
        from.hash(&mut hasher);
        to.hash(&mut hasher);
        let hash = hasher.finish();
        format!("{:016x}", hash)
    }
}

// ============================================================================
// KgStats
// ============================================================================

/// Aggregate statistics for the knowledge graph.
///
/// ## Fields
///
/// - `node_count` - Total number of nodes in the graph
/// - `edge_count` - Total number of edges in the graph
/// - `last_updated` - Timestamp when the stats were last computed
/// - `version` - Schema version string (e.g., "kg-v1")
///
/// ## Example
///
/// ```json
/// {
///   "nodeCount": 200,
///   "edgeCount": 500,
///   "lastUpdated": "2025-11-28T10:00:00Z",
///   "version": "kg-v1"
/// }
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KgStats {
    /// Total number of nodes in the graph.
    pub node_count: u64,

    /// Total number of edges in the graph.
    pub edge_count: u64,

    /// Timestamp when the stats were last computed.
    pub last_updated: DateTime<Utc>,

    /// Schema version string.
    ///
    /// Initial value: "kg-v1".
    /// Used for future migrations when KG schema changes.
    pub version: String,
}

impl Default for KgStats {
    fn default() -> Self {
        Self {
            node_count: 0,
            edge_count: 0,
            last_updated: Utc::now(),
            version: KG_VERSION.to_string(),
        }
    }
}

impl KgStats {
    /// Create new stats with the given counts.
    ///
    /// Sets `last_updated` to the current time and `version` to the current schema version.
    pub fn new(node_count: u64, edge_count: u64) -> Self {
        Self {
            node_count,
            edge_count,
            last_updated: Utc::now(),
            version: KG_VERSION.to_string(),
        }
    }

    /// Update the stats with new counts and touch `last_updated`.
    pub fn update(&mut self, node_count: u64, edge_count: u64) {
        self.node_count = node_count;
        self.edge_count = edge_count;
        self.last_updated = Utc::now();
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_kg_node_new() {
        let node = KgNode::new("file:src/main.rs", "file", "src/main.rs");

        assert_eq!(node.id, "file:src/main.rs");
        assert_eq!(node.kind, "file");
        assert_eq!(node.label, "src/main.rs");
        assert_eq!(node.props, json!({}));
        assert!(node.branch.is_none());
    }

    #[test]
    fn test_kg_node_with_props_and_branch() {
        let node = KgNode::new("file:src/main.rs", "file", "src/main.rs")
            .with_props(json!({"language": "rust", "lines": 150}))
            .with_branch("main");

        assert_eq!(node.props, json!({"language": "rust", "lines": 150}));
        assert_eq!(node.branch, Some("main".to_string()));
    }

    #[test]
    fn test_kg_node_serialization() {
        let node = KgNode::new("file:src/main.rs", "file", "src/main.rs")
            .with_props(json!({"language": "rust"}));

        let json_str = serde_json::to_string(&node).unwrap();

        // Check camelCase field names
        assert!(json_str.contains("\"id\":"));
        assert!(json_str.contains("\"kind\":"));
        assert!(json_str.contains("\"label\":"));
        assert!(json_str.contains("\"props\":"));
        assert!(json_str.contains("\"createdAt\":"));
        assert!(json_str.contains("\"updatedAt\":"));

        // Branch should be omitted when None
        assert!(!json_str.contains("\"branch\":"));
    }

    #[test]
    fn test_kg_edge_new() {
        let edge = KgEdge::new("file:src/main.rs", "file:src/lib.rs", "imports");

        assert!(edge.id.starts_with("edge:"));
        assert!(edge.id.ends_with("->imports"));
        assert_eq!(edge.from, "file:src/main.rs");
        assert_eq!(edge.to, "file:src/lib.rs");
        assert_eq!(edge.kind, "imports");
        assert_eq!(edge.props, json!({}));
        assert!(edge.branch.is_none());
    }

    #[test]
    fn test_kg_edge_with_explicit_id() {
        let edge = KgEdge::with_id(
            "my-edge-001",
            "file:src/main.rs",
            "file:src/lib.rs",
            "imports",
        );

        assert_eq!(edge.id, "my-edge-001");
    }

    #[test]
    fn test_kg_edge_serialization() {
        let edge = KgEdge::new("file:a.rs", "file:b.rs", "calls").with_props(json!({"count": 5}));

        let json_str = serde_json::to_string(&edge).unwrap();

        // Check camelCase field names
        assert!(json_str.contains("\"id\":"));
        assert!(json_str.contains("\"from\":"));
        assert!(json_str.contains("\"to\":"));
        assert!(json_str.contains("\"kind\":"));
        assert!(json_str.contains("\"props\":"));
        assert!(json_str.contains("\"createdAt\":"));
        assert!(json_str.contains("\"updatedAt\":"));
    }

    #[test]
    fn test_kg_stats_default() {
        let stats = KgStats::default();

        assert_eq!(stats.node_count, 0);
        assert_eq!(stats.edge_count, 0);
        assert_eq!(stats.version, KG_VERSION);
    }

    #[test]
    fn test_kg_stats_new() {
        let stats = KgStats::new(100, 250);

        assert_eq!(stats.node_count, 100);
        assert_eq!(stats.edge_count, 250);
        assert_eq!(stats.version, KG_VERSION);
    }

    #[test]
    fn test_kg_stats_serialization() {
        let stats = KgStats::new(200, 500);

        let json_str = serde_json::to_string(&stats).unwrap();

        // Check camelCase field names
        assert!(json_str.contains("\"nodeCount\":200"));
        assert!(json_str.contains("\"edgeCount\":500"));
        assert!(json_str.contains("\"lastUpdated\":"));
        assert!(json_str.contains("\"version\":\"kg-v1\""));
    }
}
