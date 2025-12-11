//! KG export utilities for generating DOT and Mermaid output.
//!
//! This module provides helpers to export KG subgraphs in visualization formats:
//! - DOT (Graphviz) for detailed graph visualization
//! - Mermaid for embedding in Markdown documentation
//!
//! ## Usage
//!
//! ```ignore
//! use gik_core::kg::export::{export_to_dot, export_to_mermaid, KgExportOptions};
//!
//! let nodes = vec![...];
//! let edges = vec![...];
//!
//! let dot = export_to_dot(&nodes, &edges, KgExportOptions::default());
//! let mermaid = export_to_mermaid(&nodes, &edges, KgExportOptions::default());
//! ```

use super::entities::{KgEdge, KgNode};

// ============================================================================
// KgExportFormat
// ============================================================================

/// Output format for KG export.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum KgExportFormat {
    /// DOT format (Graphviz)
    #[default]
    Dot,
    /// Mermaid flowchart format
    Mermaid,
    /// JSON format (nodes and edges as JSON)
    Json,
}

impl std::fmt::Display for KgExportFormat {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Dot => write!(f, "dot"),
            Self::Mermaid => write!(f, "mermaid"),
            Self::Json => write!(f, "json"),
        }
    }
}

impl std::str::FromStr for KgExportFormat {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "dot" | "graphviz" => Ok(Self::Dot),
            "mermaid" | "md" => Ok(Self::Mermaid),
            "json" => Ok(Self::Json),
            other => Err(format!("Unknown export format: {}", other)),
        }
    }
}

// ============================================================================
// KgExportOptions
// ============================================================================

/// Options for KG export.
#[derive(Debug, Clone, Default)]
pub struct KgExportOptions {
    /// Maximum number of nodes to include (None = unlimited).
    pub max_nodes: Option<usize>,
    /// Maximum number of edges to include (None = unlimited).
    pub max_edges: Option<usize>,
    /// Graph title/label.
    pub title: Option<String>,
    /// Whether to include node properties as labels.
    pub include_props: bool,
}

impl KgExportOptions {
    /// Create new export options with defaults.
    pub fn new() -> Self {
        Self::default()
    }

    /// Set maximum nodes.
    pub fn with_max_nodes(mut self, max: usize) -> Self {
        self.max_nodes = Some(max);
        self
    }

    /// Set maximum edges.
    pub fn with_max_edges(mut self, max: usize) -> Self {
        self.max_edges = Some(max);
        self
    }

    /// Set graph title.
    pub fn with_title(mut self, title: impl Into<String>) -> Self {
        self.title = Some(title.into());
        self
    }
}

// ============================================================================
// Export Functions
// ============================================================================

/// Export KG nodes and edges to DOT format (Graphviz).
///
/// # Arguments
///
/// * `nodes` - The nodes to export.
/// * `edges` - The edges to export.
/// * `opts` - Export options.
///
/// # Returns
///
/// A DOT format string suitable for rendering with Graphviz.
pub fn export_to_dot(nodes: &[KgNode], edges: &[KgEdge], opts: KgExportOptions) -> String {
    let mut lines = Vec::new();

    // Header
    let title = opts.title.as_deref().unwrap_or("GIK Knowledge Graph");
    lines.push(format!("digraph \"{}\" {{", escape_dot_string(title)));
    lines.push("  rankdir=LR;".to_string());
    lines.push("  node [shape=box, style=rounded];".to_string());
    lines.push(String::new());

    // Truncate nodes if needed
    let max_nodes = opts.max_nodes.unwrap_or(usize::MAX);
    let truncated_nodes: Vec<_> = nodes.iter().take(max_nodes).collect();

    // Build a set of node IDs for filtering edges
    let node_ids: std::collections::HashSet<&str> =
        truncated_nodes.iter().map(|n| n.id.as_str()).collect();

    // Nodes
    for node in &truncated_nodes {
        let label = if opts.include_props {
            format!("{}\\n[{}]", escape_dot_string(&node.label), node.kind)
        } else {
            escape_dot_string(&node.label)
        };
        let node_id = escape_dot_id(&node.id);
        lines.push(format!("  {} [label=\"{}\"];", node_id, label));
    }

    lines.push(String::new());

    // Truncate edges if needed
    let max_edges = opts.max_edges.unwrap_or(usize::MAX);

    // Edges (only include edges where both endpoints are in the node set)
    let mut edge_count = 0;
    for edge in edges {
        if edge_count >= max_edges {
            break;
        }
        if node_ids.contains(edge.from.as_str()) && node_ids.contains(edge.to.as_str()) {
            let from_id = escape_dot_id(&edge.from);
            let to_id = escape_dot_id(&edge.to);
            lines.push(format!(
                "  {} -> {} [label=\"{}\"];",
                from_id, to_id, edge.kind
            ));
            edge_count += 1;
        }
    }

    lines.push("}".to_string());

    lines.join("\n")
}

/// Export KG nodes and edges to Mermaid flowchart format.
///
/// # Arguments
///
/// * `nodes` - The nodes to export.
/// * `edges` - The edges to export.
/// * `opts` - Export options.
///
/// # Returns
///
/// A Mermaid flowchart string suitable for embedding in Markdown.
pub fn export_to_mermaid(nodes: &[KgNode], edges: &[KgEdge], opts: KgExportOptions) -> String {
    let mut lines = Vec::new();

    // Header
    lines.push("```mermaid".to_string());
    lines.push("flowchart LR".to_string());

    // Truncate nodes if needed
    let max_nodes = opts.max_nodes.unwrap_or(usize::MAX);
    let truncated_nodes: Vec<_> = nodes.iter().take(max_nodes).collect();

    // Build a map of original ID -> mermaid-safe ID
    let mut id_map: std::collections::HashMap<&str, String> = std::collections::HashMap::new();
    for (i, node) in truncated_nodes.iter().enumerate() {
        id_map.insert(&node.id, format!("n{}", i));
    }

    // Nodes
    for node in &truncated_nodes {
        let safe_id = &id_map[node.id.as_str()];
        let label = escape_mermaid_string(&node.label);
        lines.push(format!("  {}[{}]", safe_id, label));
    }

    // Truncate edges if needed
    let max_edges = opts.max_edges.unwrap_or(usize::MAX);

    // Edges (only include edges where both endpoints are in the node set)
    let mut edge_count = 0;
    for edge in edges {
        if edge_count >= max_edges {
            break;
        }
        if let (Some(from_id), Some(to_id)) =
            (id_map.get(edge.from.as_str()), id_map.get(edge.to.as_str()))
        {
            let edge_label = escape_mermaid_string(&edge.kind);
            lines.push(format!("  {} -->|{}| {}", from_id, edge_label, to_id));
            edge_count += 1;
        }
    }

    lines.push("```".to_string());

    lines.join("\n")
}

/// Export KG nodes and edges to JSON format.
pub fn export_to_json(nodes: &[KgNode], edges: &[KgEdge], opts: KgExportOptions) -> String {
    let max_nodes = opts.max_nodes.unwrap_or(usize::MAX);
    let max_edges = opts.max_edges.unwrap_or(usize::MAX);

    let truncated_nodes: Vec<_> = nodes.iter().take(max_nodes).cloned().collect();

    // Build a set of node IDs for filtering edges
    let node_ids: std::collections::HashSet<String> =
        truncated_nodes.iter().map(|n| n.id.clone()).collect();

    let truncated_edges: Vec<_> = edges
        .iter()
        .filter(|e| node_ids.contains(&e.from) && node_ids.contains(&e.to))
        .take(max_edges)
        .cloned()
        .collect();

    let output = serde_json::json!({
        "nodes": truncated_nodes,
        "edges": truncated_edges,
        "title": opts.title,
    });

    serde_json::to_string_pretty(&output).unwrap_or_else(|_| "{}".to_string())
}

/// Export KG in the specified format.
pub fn export_kg(
    nodes: &[KgNode],
    edges: &[KgEdge],
    format: KgExportFormat,
    opts: KgExportOptions,
) -> String {
    match format {
        KgExportFormat::Dot => export_to_dot(nodes, edges, opts),
        KgExportFormat::Mermaid => export_to_mermaid(nodes, edges, opts),
        KgExportFormat::Json => export_to_json(nodes, edges, opts),
    }
}

// ============================================================================
// Helper Functions
// ============================================================================

/// Escape a string for use in DOT format.
fn escape_dot_string(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
}

/// Escape an ID for use as a DOT node identifier.
fn escape_dot_id(s: &str) -> String {
    // DOT IDs need to be quoted if they contain special characters
    format!("\"{}\"", escape_dot_string(s))
}

/// Escape a string for use in Mermaid format.
fn escape_mermaid_string(s: &str) -> String {
    // Mermaid has issues with certain characters in node labels
    // - Square brackets define node shape, so escape to parentheses
    // - Pipes are used for edge labels, escape to slashes
    // - Quotes can break parsing
    // - Slashes and parentheses in text content need to be escaped
    //   when inside square brackets (node definitions)
    s.replace('"', "'")
        .replace('[', "&#91;")
        .replace(']', "&#93;")
        .replace('(', "&#40;")
        .replace(')', "&#41;")
        .replace('/', "&#47;")
        .replace('|', "&#124;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    fn sample_nodes() -> Vec<KgNode> {
        vec![
            KgNode {
                id: "file:src/main.rs".to_string(),
                kind: "file".to_string(),
                label: "main.rs".to_string(),
                props: serde_json::Value::Null,
                branch: None,
                created_at: Utc::now(),
                updated_at: Utc::now(),
            },
            KgNode {
                id: "file:src/lib.rs".to_string(),
                kind: "file".to_string(),
                label: "lib.rs".to_string(),
                props: serde_json::Value::Null,
                branch: None,
                created_at: Utc::now(),
                updated_at: Utc::now(),
            },
        ]
    }

    fn sample_edges() -> Vec<KgEdge> {
        vec![KgEdge {
            id: "edge-1".to_string(),
            from: "file:src/main.rs".to_string(),
            to: "file:src/lib.rs".to_string(),
            kind: "imports".to_string(),
            props: serde_json::Value::Null,
            branch: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }]
    }

    #[test]
    fn test_export_to_dot() {
        let nodes = sample_nodes();
        let edges = sample_edges();
        let opts = KgExportOptions::new().with_title("Test Graph");

        let dot = export_to_dot(&nodes, &edges, opts);

        assert!(dot.contains("digraph \"Test Graph\""));
        assert!(dot.contains("main.rs"));
        assert!(dot.contains("lib.rs"));
        assert!(dot.contains("imports"));
    }

    #[test]
    fn test_export_to_mermaid() {
        let nodes = sample_nodes();
        let edges = sample_edges();
        let opts = KgExportOptions::default();

        let mermaid = export_to_mermaid(&nodes, &edges, opts);

        assert!(mermaid.contains("```mermaid"));
        assert!(mermaid.contains("flowchart LR"));
        assert!(mermaid.contains("main.rs"));
        assert!(mermaid.contains("imports"));
        assert!(mermaid.contains("```"));
    }

    #[test]
    fn test_export_to_json() {
        let nodes = sample_nodes();
        let edges = sample_edges();
        let opts = KgExportOptions::default();

        let json = export_to_json(&nodes, &edges, opts);

        assert!(json.contains("\"nodes\""));
        assert!(json.contains("\"edges\""));
        assert!(json.contains("main.rs"));
    }

    #[test]
    fn test_export_format_from_str() {
        assert_eq!(
            "dot".parse::<KgExportFormat>().unwrap(),
            KgExportFormat::Dot
        );
        assert_eq!(
            "mermaid".parse::<KgExportFormat>().unwrap(),
            KgExportFormat::Mermaid
        );
        assert_eq!(
            "json".parse::<KgExportFormat>().unwrap(),
            KgExportFormat::Json
        );
    }

    #[test]
    fn test_max_nodes_truncation() {
        let nodes = sample_nodes();
        let edges = sample_edges();
        let opts = KgExportOptions::new().with_max_nodes(1);

        let dot = export_to_dot(&nodes, &edges, opts);

        // Should only have one node (main.rs) and no edges (since lib.rs is truncated)
        assert!(dot.contains("main.rs"));
        // lib.rs should be truncated
        assert!(!dot.contains("lib.rs"));
    }
}
