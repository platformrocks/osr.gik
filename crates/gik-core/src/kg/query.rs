//! Knowledge Graph query helpers for the ask pipeline.
//!
//! This module provides query capabilities for enriching ask responses with
//! KG-derived structural context.
//!
//! ## Phase 9.3 Scope
//!
//! - Map RAG chunks to KG nodes via base/path matching
//! - Bounded graph traversal (configurable hops, node/edge limits)
//! - Endpoint-aware heuristics (detect `/api/` patterns in question)
//! - Produce `AskKgResult` subgraphs for `AskContextBundle`
//!
//! ## Exhaustive Query Support (Phase 9.3.1)
//!
//! When a question asks for "all", "every", "list of" something, this module
//! performs a **structured KG search** to find ALL matching entities, not just
//! those semantically similar to the query.
//!
//! Supported patterns:
//! - "all routes", "all endpoints", "all GET/POST/PUT/DELETE handlers"
//! - "all functions", "all methods", "all classes"
//! - "todas as rotas", "todos os métodos", "liste todas as funções"
//!
//! ## Strategy
//!
//! 1. For each RAG chunk, find corresponding KG node (file/doc) by path
//! 2. From root nodes, traverse edges (imports, definesEndpoint, etc.)
//! 3. Collect bounded subgraphs with configurable limits
//! 4. If question mentions endpoints, also seed with endpoint nodes
//! 5. If question asks for exhaustive listing, search KG by node kind/props
//!
//! ## Future (TODO)
//!
//! - Phase 9.4+: Symbol-level traversal (functions, classes, methods)
//! - Phase 9.4+: Incremental graph updates

use std::collections::{HashMap, HashSet, VecDeque};

use serde::{Deserialize, Serialize};

use crate::errors::GikError;
use crate::workspace::Workspace;
use tracing::{debug, trace};

use super::entities::{KgEdge, KgNode};
use super::store::{kg_dir_for_branch, read_all_edges, read_all_nodes};

/// Check if KG exists for a branch (local helper to avoid circular imports).
fn kg_exists(workspace: &Workspace, branch: &str) -> bool {
    kg_dir_for_branch(workspace, branch).exists()
}

// ============================================================================
// AskKgResult (response type for ask)
// ============================================================================

/// Knowledge graph result for ask queries.
///
/// Represents a subgraph of related nodes and edges relevant to the query.
/// This is the *response format* returned by `gik ask`, containing actual
/// graph data extracted from the KG store.
///
/// ## Example JSON
///
/// ```json
/// {
///   "reason": "Related to RAG chunk: src/api/users.ts",
///   "rootNodeIds": ["file:src/api/users.ts"],
///   "nodes": [...],
///   "edges": [...]
/// }
/// ```
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct AskKgResult {
    /// Why this subgraph is relevant to the query.
    pub reason: String,

    /// The root node IDs that seeded this subgraph.
    pub root_node_ids: Vec<String>,

    /// Nodes in this subgraph.
    pub nodes: Vec<KgNode>,

    /// Edges in this subgraph.
    pub edges: Vec<KgEdge>,
}

impl AskKgResult {
    /// Create an empty result.
    pub fn new() -> Self {
        Self::default()
    }

    /// Create a result with a reason and roots.
    pub fn with_reason(reason: impl Into<String>, roots: Vec<String>) -> Self {
        Self {
            reason: reason.into(),
            root_node_ids: roots,
            nodes: Vec::new(),
            edges: Vec::new(),
        }
    }

    /// Check if the result is empty (no nodes).
    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }
}

// ============================================================================
// KgQueryConfig
// ============================================================================

/// Configuration for KG queries in the ask pipeline.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KgQueryConfig {
    /// Maximum number of subgraphs to return.
    #[serde(default = "default_max_subgraphs")]
    pub max_subgraphs: usize,

    /// Maximum nodes per subgraph.
    #[serde(default = "default_max_nodes_per_subgraph")]
    pub max_nodes_per_subgraph: usize,

    /// Maximum edges per subgraph.
    #[serde(default = "default_max_edges_per_subgraph")]
    pub max_edges_per_subgraph: usize,

    /// Maximum hops from root nodes during traversal.
    #[serde(default = "default_max_hops")]
    pub max_hops: usize,

    /// Whether to include endpoint-aware heuristics.
    #[serde(default = "default_endpoint_heuristics")]
    pub endpoint_heuristics: bool,
}

fn default_max_subgraphs() -> usize {
    3
}

fn default_max_nodes_per_subgraph() -> usize {
    32
}

fn default_max_edges_per_subgraph() -> usize {
    48
}

fn default_max_hops() -> usize {
    2
}

fn default_endpoint_heuristics() -> bool {
    true
}

impl Default for KgQueryConfig {
    fn default() -> Self {
        Self {
            max_subgraphs: default_max_subgraphs(),
            max_nodes_per_subgraph: default_max_nodes_per_subgraph(),
            max_edges_per_subgraph: default_max_edges_per_subgraph(),
            max_hops: default_max_hops(),
            endpoint_heuristics: default_endpoint_heuristics(),
        }
    }
}

impl KgQueryConfig {
    /// Create a new config with default settings.
    pub fn new() -> Self {
        Self::default()
    }

    /// Set maximum subgraphs.
    pub fn with_max_subgraphs(mut self, max: usize) -> Self {
        self.max_subgraphs = max;
        self
    }

    /// Set maximum nodes per subgraph.
    pub fn with_max_nodes(mut self, max: usize) -> Self {
        self.max_nodes_per_subgraph = max;
        self
    }

    /// Set maximum hops.
    pub fn with_max_hops(mut self, max: usize) -> Self {
        self.max_hops = max;
        self
    }

    /// Disable endpoint heuristics.
    pub fn without_endpoint_heuristics(mut self) -> Self {
        self.endpoint_heuristics = false;
        self
    }
}

// ============================================================================
// RagChunkRef (minimal info for KG mapping)
// ============================================================================

/// Minimal reference to a RAG chunk for KG mapping.
///
/// This avoids coupling to the full RagChunk type from ask.rs.
#[derive(Debug, Clone)]
pub struct RagChunkRef {
    /// Base name (code, docs, memory).
    pub base: String,

    /// File path (workspace-relative).
    pub path: String,
}

impl RagChunkRef {
    /// Create a new chunk reference.
    pub fn new(base: impl Into<String>, path: impl Into<String>) -> Self {
        Self {
            base: base.into(),
            path: path.into(),
        }
    }
}

// ============================================================================
// Query Functions
// ============================================================================

/// Build KG context for the ask pipeline.
///
/// Maps RAG chunks to KG nodes and performs bounded graph traversal to
/// produce relevant subgraphs. Also performs exhaustive searches when the
/// question asks for "all", "every", or "list of" something.
///
/// # Arguments
///
/// * `workspace` - The workspace to query
/// * `branch` - The branch name
/// * `rag_chunks` - References to RAG chunks for mapping to KG nodes
/// * `question` - The original question (used for endpoint heuristics and exhaustive search)
/// * `cfg` - Query configuration
///
/// # Returns
///
/// A vector of [`AskKgResult`] subgraphs, or an empty vector if KG doesn't exist.
pub fn build_ask_kg_context(
    workspace: &Workspace,
    branch: &str,
    rag_chunks: &[RagChunkRef],
    question: &str,
    cfg: &KgQueryConfig,
) -> Result<Vec<AskKgResult>, GikError> {
    // Check if KG exists for this branch
    if !kg_exists(workspace, branch) {
        debug!("KG does not exist for branch '{}'", branch);
        return Ok(Vec::new());
    }

    // Load all nodes and edges
    let all_nodes = read_all_nodes(workspace, branch)?;
    let all_edges = read_all_edges(workspace, branch)?;

    debug!(
        "KG loaded: {} nodes, {} edges for branch '{}'",
        all_nodes.len(),
        all_edges.len(),
        branch
    );

    if all_nodes.is_empty() {
        return Ok(Vec::new());
    }

    let mut results: Vec<AskKgResult> = Vec::new();

    // Phase 9.3.1: Check for exhaustive query intent FIRST
    // This handles questions like "all routes", "todas as funções GET", etc.
    let exhaustive_intent = detect_exhaustive_intent(question);
    if exhaustive_intent.is_exhaustive {
        debug!(
            "Exhaustive query detected: {:?} (kinds: {:?}, http_method: {:?})",
            exhaustive_intent.reason,
            exhaustive_intent.target_kinds,
            exhaustive_intent.http_method_filter
        );

        // Use a higher limit for exhaustive searches (configurable via cfg)
        let exhaustive_max_nodes = cfg.max_nodes_per_subgraph * 4; // 128 by default
        if let Some(exhaustive_result) = search_kg_exhaustive(
            &all_nodes,
            &all_edges,
            &exhaustive_intent,
            exhaustive_max_nodes,
        ) {
            debug!(
                "Exhaustive search found {} nodes, {} edges",
                exhaustive_result.nodes.len(),
                exhaustive_result.edges.len()
            );
            results.push(exhaustive_result);
        }
    }

    // Build lookup maps
    let node_by_id: HashMap<String, &KgNode> =
        all_nodes.iter().map(|n| (n.id.clone(), n)).collect();

    let node_by_path: HashMap<String, &KgNode> = all_nodes
        .iter()
        .filter_map(|n| {
            n.props
                .get("path")
                .and_then(|v| v.as_str())
                .map(|p| (p.to_string(), n))
        })
        .collect();

    debug!("Built node_by_path with {} entries", node_by_path.len());

    // Build adjacency lists for traversal
    let mut outgoing: HashMap<String, Vec<&KgEdge>> = HashMap::new();
    let mut incoming: HashMap<String, Vec<&KgEdge>> = HashMap::new();

    for edge in &all_edges {
        outgoing.entry(edge.from.clone()).or_default().push(edge);
        incoming.entry(edge.to.clone()).or_default().push(edge);
    }

    // Collect root node IDs from RAG chunks
    let mut root_nodes: Vec<(String, String)> = Vec::new(); // (node_id, reason)

    debug!("Processing {} RAG chunks for KG mapping", rag_chunks.len());
    for chunk in rag_chunks {
        trace!("Looking for KG node with path: {}", chunk.path);
        if let Some(node) = node_by_path.get(&chunk.path) {
            let reason = format!("Related to RAG chunk: {}", chunk.path);
            root_nodes.push((node.id.clone(), reason));
            trace!("Found KG node: {}", node.id);
        } else {
            trace!("No KG node found for path: {}", chunk.path);
        }
    }

    // Add endpoint roots if question mentions API patterns (and not already handled by exhaustive)
    if cfg.endpoint_heuristics
        && looks_like_endpoint_question(question)
        && !exhaustive_intent.is_exhaustive
    {
        debug!("Question looks like endpoint query, adding endpoint nodes");
        for node in &all_nodes {
            if node.kind == "endpoint" {
                let route = node
                    .props
                    .get("route")
                    .and_then(|v| v.as_str())
                    .unwrap_or("unknown");
                let reason = format!("Endpoint matching question: {}", route);
                root_nodes.push((node.id.clone(), reason));
            }
        }
    }

    // Deduplicate roots
    let mut seen_roots: HashSet<String> = HashSet::new();
    root_nodes.retain(|(id, _)| seen_roots.insert(id.clone()));

    debug!("Found {} root nodes after deduplication", root_nodes.len());

    // Limit to max_subgraphs
    root_nodes.truncate(cfg.max_subgraphs);

    // Build subgraphs for each root
    for (root_id, reason) in &root_nodes {
        debug!("Building subgraph for root: {} ({})", root_id, reason);
        let subgraph = build_subgraph(
            root_id,
            &node_by_id,
            &outgoing,
            &incoming,
            cfg.max_hops,
            cfg.max_nodes_per_subgraph,
            cfg.max_edges_per_subgraph,
        );

        debug!(
            "Subgraph for {}: {} nodes, {} edges",
            root_id,
            subgraph.nodes.len(),
            subgraph.edges.len()
        );

        if !subgraph.nodes.is_empty() {
            results.push(AskKgResult {
                reason: reason.clone(),
                root_node_ids: vec![root_id.clone()],
                nodes: subgraph.nodes,
                edges: subgraph.edges,
            });
        }
    }

    debug!("Built {} KG subgraphs total", results.len());
    Ok(results)
}

/// Check if a question looks like it's about endpoints/APIs.
fn looks_like_endpoint_question(question: &str) -> bool {
    let q = question.to_lowercase();

    // Check for API-related terms
    let api_terms = [
        "/api/",
        "endpoint",
        "route",
        "api route",
        "http",
        "get ",
        "post ",
        "put ",
        "delete ",
        "patch ",
        "rest ",
        "handler",
    ];

    api_terms.iter().any(|term| q.contains(term))
}

/// Collected subgraph during traversal.
struct CollectedSubgraph {
    nodes: Vec<KgNode>,
    edges: Vec<KgEdge>,
}

/// Build a bounded subgraph starting from a root node.
fn build_subgraph(
    root_id: &str,
    node_by_id: &HashMap<String, &KgNode>,
    outgoing: &HashMap<String, Vec<&KgEdge>>,
    incoming: &HashMap<String, Vec<&KgEdge>>,
    max_hops: usize,
    max_nodes: usize,
    max_edges: usize,
) -> CollectedSubgraph {
    let mut visited_nodes: HashSet<String> = HashSet::new();
    let mut visited_edges: HashSet<String> = HashSet::new();
    let mut collected_nodes: Vec<KgNode> = Vec::new();
    let mut collected_edges: Vec<KgEdge> = Vec::new();

    // BFS queue: (node_id, current_depth)
    let mut queue: VecDeque<(String, usize)> = VecDeque::new();

    // Start from root
    if let Some(root_node) = node_by_id.get(root_id) {
        queue.push_back((root_id.to_string(), 0));
        visited_nodes.insert(root_id.to_string());
        collected_nodes.push((*root_node).clone());
    }

    while let Some((node_id, depth)) = queue.pop_front() {
        // Check limits
        if collected_nodes.len() >= max_nodes {
            break;
        }

        if depth >= max_hops {
            continue;
        }

        // Traverse outgoing edges
        if let Some(edges) = outgoing.get(&node_id) {
            for edge in edges {
                if collected_edges.len() >= max_edges {
                    break;
                }

                if visited_edges.insert(edge.id.clone()) {
                    collected_edges.push((*edge).clone());

                    // Visit target node if not seen
                    if visited_nodes.insert(edge.to.clone()) {
                        if let Some(target) = node_by_id.get(&edge.to) {
                            if collected_nodes.len() < max_nodes {
                                collected_nodes.push((*target).clone());
                                queue.push_back((edge.to.clone(), depth + 1));
                            }
                        }
                    }
                }
            }
        }

        // Traverse incoming edges (for bidirectional context)
        if let Some(edges) = incoming.get(&node_id) {
            for edge in edges {
                if collected_edges.len() >= max_edges {
                    break;
                }

                if visited_edges.insert(edge.id.clone()) {
                    collected_edges.push((*edge).clone());

                    // Visit source node if not seen
                    if visited_nodes.insert(edge.from.clone()) {
                        if let Some(source) = node_by_id.get(&edge.from) {
                            if collected_nodes.len() < max_nodes {
                                collected_nodes.push((*source).clone());
                                queue.push_back((edge.from.clone(), depth + 1));
                            }
                        }
                    }
                }
            }
        }
    }

    CollectedSubgraph {
        nodes: collected_nodes,
        edges: collected_edges,
    }
}

// ============================================================================
// Exhaustive Query Support (Phase 9.3.1)
// ============================================================================

/// Result of exhaustive query intent detection.
#[derive(Debug, Clone, Default)]
pub struct ExhaustiveQueryIntent {
    /// Whether the query asks for an exhaustive listing.
    pub is_exhaustive: bool,

    /// Detected entity kinds to search for (e.g., "endpoint", "function", "file").
    pub target_kinds: Vec<String>,

    /// Optional filter on HTTP method (for endpoint queries).
    pub http_method_filter: Option<String>,

    /// Optional filter on props (key-value pairs).
    pub prop_filters: HashMap<String, String>,

    /// Human-readable reason for the intent.
    pub reason: String,
}

/// Detect if a question asks for an exhaustive listing of entities.
///
/// Returns an `ExhaustiveQueryIntent` with details about what to search for.
///
/// # Supported patterns
///
/// English:
/// - "all routes", "all endpoints", "all GET/POST handlers"
/// - "list all functions", "show every method", "what are all the classes"
///
/// Portuguese:
/// - "todas as rotas", "todos os endpoints", "todos os métodos GET"
/// - "liste todas as funções", "mostre todos os métodos"
pub fn detect_exhaustive_intent(question: &str) -> ExhaustiveQueryIntent {
    let q = question.to_lowercase();

    // Patterns that indicate exhaustive search intent
    let exhaustive_markers = [
        "all ",
        "every ",
        "list ",
        "liste ",
        "show all",
        "mostre todos",
        "mostre todas",
        "quais são todos",
        "quais são todas",
        "todas as ",
        "todos os ",
        "what are all",
        "where are all",
        "find all",
        "get all",
        "list of all",
    ];

    let has_exhaustive_marker = exhaustive_markers.iter().any(|m| q.contains(m));

    if !has_exhaustive_marker {
        return ExhaustiveQueryIntent::default();
    }

    let mut intent = ExhaustiveQueryIntent {
        is_exhaustive: true,
        target_kinds: Vec::new(),
        http_method_filter: None,
        prop_filters: HashMap::new(),
        reason: String::new(),
    };

    // Detect HTTP method filters
    let http_methods = [
        ("get", "GET"),
        ("post", "POST"),
        ("put", "PUT"),
        ("delete", "DELETE"),
        ("patch", "PATCH"),
    ];

    for (pattern, method) in &http_methods {
        if q.contains(pattern) {
            intent.http_method_filter = Some(method.to_string());
            break;
        }
    }

    // Detect entity kinds - map query terms to KG node kinds
    // These are generic mappings, not project-specific
    let kind_patterns: &[(&[&str], &str)] = &[
        // Routes/endpoints
        (
            &[
                "route",
                "routes",
                "rota",
                "rotas",
                "endpoint",
                "endpoints",
                "api",
                "handler",
                "handlers",
            ],
            "endpoint",
        ),
        // Functions/methods
        (
            &[
                "function",
                "functions",
                "função",
                "funções",
                "method",
                "methods",
                "método",
                "métodos",
            ],
            "function",
        ),
        // Classes/types
        (
            &[
                "class",
                "classes",
                "classe",
                "classes",
                "type",
                "types",
                "tipo",
                "tipos",
                "interface",
                "interfaces",
            ],
            "class",
        ),
        // Components (React, Vue, etc.)
        (
            &["component", "components", "componente", "componentes"],
            "component",
        ),
        // Files/modules
        (
            &[
                "file", "files", "arquivo", "arquivos", "module", "modules", "módulo", "módulos",
            ],
            "file",
        ),
        // Symbols (generic)
        (
            &[
                "symbol",
                "symbols",
                "símbolo",
                "símbolos",
                "definition",
                "definitions",
                "definição",
                "definições",
            ],
            "symbol",
        ),
    ];

    for (patterns, kind) in kind_patterns {
        if patterns.iter().any(|p| q.contains(p)) {
            intent.target_kinds.push(kind.to_string());
        }
    }

    // If endpoint with HTTP method filter, refine the search
    if intent.http_method_filter.is_some() && intent.target_kinds.is_empty() {
        intent.target_kinds.push("endpoint".to_string());
    }

    // Build reason string
    if !intent.target_kinds.is_empty() {
        let kinds_str = intent.target_kinds.join(", ");
        if let Some(method) = &intent.http_method_filter {
            intent.reason = format!("Exhaustive search for {} {} entities", method, kinds_str);
        } else {
            intent.reason = format!("Exhaustive search for {} entities", kinds_str);
        }
    } else {
        intent.reason = "Exhaustive search (generic)".to_string();
    }

    intent
}

/// Search the KG for all nodes matching the exhaustive query intent.
///
/// Returns an `AskKgResult` containing all matching nodes and their immediate edges.
pub fn search_kg_exhaustive(
    all_nodes: &[KgNode],
    all_edges: &[KgEdge],
    intent: &ExhaustiveQueryIntent,
    max_nodes: usize,
) -> Option<AskKgResult> {
    if !intent.is_exhaustive || intent.target_kinds.is_empty() {
        return None;
    }

    let mut matching_nodes: Vec<KgNode> = Vec::new();
    let mut matching_node_ids: HashSet<String> = HashSet::new();

    // Find nodes matching the target kinds
    for node in all_nodes {
        if matching_nodes.len() >= max_nodes {
            break;
        }

        // Check if node kind matches any target kind
        // Also check for kind variations (e.g., "function" matches "fn", "method")
        let kind_matches = intent.target_kinds.iter().any(|target| {
            node.kind == *target
                || (target == "endpoint" && (node.kind == "route" || node.kind == "api"))
                || (target == "function" && (node.kind == "fn" || node.kind == "method"))
                || (target == "class"
                    && (node.kind == "type"
                        || node.kind == "interface"
                        || node.kind == "struct"
                        || node.kind == "entity"))
                || (target == "symbol"
                    && (node.kind == "function"
                        || node.kind == "fn"
                        || node.kind == "method"
                        || node.kind == "class"
                        || node.kind == "type"
                        || node.kind == "interface"))
        });

        if !kind_matches {
            continue;
        }

        // Apply HTTP method filter if present
        if let Some(http_method) = &intent.http_method_filter {
            // Check props.method or props.httpMethod
            let node_method = node
                .props
                .get("method")
                .or_else(|| node.props.get("httpMethod"))
                .and_then(|v| v.as_str());

            if let Some(nm) = node_method {
                if !nm.eq_ignore_ascii_case(http_method) {
                    continue;
                }
            } else {
                // If no method prop and filter is set, skip unless kind is exactly "endpoint"
                // This allows endpoints without explicit method to still be included in generic searches
                if node.kind != "endpoint" {
                    continue;
                }
            }
        }

        // Apply prop filters
        let props_match = intent.prop_filters.iter().all(|(key, value)| {
            node.props
                .get(key)
                .and_then(|v| v.as_str())
                .map(|v| v == value)
                .unwrap_or(false)
        });

        if !props_match {
            continue;
        }

        matching_node_ids.insert(node.id.clone());
        matching_nodes.push(node.clone());
    }

    if matching_nodes.is_empty() {
        return None;
    }

    // Build a lookup map for all nodes by ID
    let node_by_id: HashMap<String, &KgNode> = all_nodes.iter().map(|n| (n.id.clone(), n)).collect();

    // Collect edges that connect matching nodes to other nodes
    // Also collect the connected nodes (e.g., file nodes that define endpoints)
    let mut matching_edges: Vec<KgEdge> = Vec::new();
    let mut connected_node_ids: HashSet<String> = HashSet::new();
    let max_edges = max_nodes * 2; // Reasonable limit for edges

    for edge in all_edges {
        if matching_edges.len() >= max_edges {
            break;
        }

        // Include edge if it connects from or to a matching node
        let from_matches = matching_node_ids.contains(&edge.from);
        let to_matches = matching_node_ids.contains(&edge.to);

        if from_matches || to_matches {
            matching_edges.push(edge.clone());

            // Track connected nodes that aren't already in matching_nodes
            if from_matches && !matching_node_ids.contains(&edge.to) {
                connected_node_ids.insert(edge.to.clone());
            }
            if to_matches && !matching_node_ids.contains(&edge.from) {
                connected_node_ids.insert(edge.from.clone());
            }
        }
    }

    // Add connected nodes (e.g., file nodes that define the endpoints)
    for node_id in &connected_node_ids {
        if let Some(node) = node_by_id.get(node_id) {
            matching_nodes.push((*node).clone());
        }
    }

    let root_ids: Vec<String> = matching_node_ids.iter().cloned().collect();

    Some(AskKgResult {
        reason: intent.reason.clone(),
        root_node_ids: root_ids,
        nodes: matching_nodes,
        edges: matching_edges,
    })
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    fn make_node(id: &str, kind: &str, path: Option<&str>) -> KgNode {
        let mut props = serde_json::json!({});
        if let Some(p) = path {
            props["path"] = serde_json::json!(p);
        }
        KgNode {
            id: id.to_string(),
            kind: kind.to_string(),
            label: id.to_string(),
            props,
            branch: Some("main".to_string()),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    fn make_edge(from: &str, to: &str, kind: &str) -> KgEdge {
        KgEdge {
            id: format!("edge:{}->{}:{}", from, to, kind),
            from: from.to_string(),
            to: to.to_string(),
            kind: kind.to_string(),
            props: serde_json::json!({}),
            branch: Some("main".to_string()),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    #[test]
    fn test_looks_like_endpoint_question() {
        assert!(looks_like_endpoint_question("What API endpoints exist?"));
        assert!(looks_like_endpoint_question("Show me the /api/users route"));
        assert!(looks_like_endpoint_question(
            "Which GET handlers are there?"
        ));
        assert!(looks_like_endpoint_question("List all endpoints"));
        assert!(!looks_like_endpoint_question(
            "What is the purpose of this file?"
        ));
        assert!(!looks_like_endpoint_question("How does the function work?"));
    }

    #[test]
    fn test_kg_query_config_defaults() {
        let cfg = KgQueryConfig::default();
        assert_eq!(cfg.max_subgraphs, 3);
        assert_eq!(cfg.max_nodes_per_subgraph, 32);
        assert_eq!(cfg.max_edges_per_subgraph, 48);
        assert_eq!(cfg.max_hops, 2);
        assert!(cfg.endpoint_heuristics);
    }

    #[test]
    fn test_build_subgraph_basic() {
        // Create a simple graph: A -> B -> C
        let nodes = [
            make_node("file:a.ts", "file", Some("a.ts")),
            make_node("file:b.ts", "file", Some("b.ts")),
            make_node("file:c.ts", "file", Some("c.ts")),
        ];
        let edges = [
            make_edge("file:a.ts", "file:b.ts", "imports"),
            make_edge("file:b.ts", "file:c.ts", "imports"),
        ];

        let node_by_id: HashMap<String, &KgNode> =
            nodes.iter().map(|n| (n.id.clone(), n)).collect();

        let mut outgoing: HashMap<String, Vec<&KgEdge>> = HashMap::new();
        let mut incoming: HashMap<String, Vec<&KgEdge>> = HashMap::new();

        for edge in &edges {
            outgoing.entry(edge.from.clone()).or_default().push(edge);
            incoming.entry(edge.to.clone()).or_default().push(edge);
        }

        // Start from A with max_hops=2
        let subgraph = build_subgraph(
            "file:a.ts",
            &node_by_id,
            &outgoing,
            &incoming,
            2,  // max_hops
            10, // max_nodes
            10, // max_edges
        );

        // Should find all 3 nodes and 2 edges
        assert_eq!(subgraph.nodes.len(), 3);
        assert_eq!(subgraph.edges.len(), 2);
    }

    #[test]
    fn test_build_subgraph_respects_limits() {
        // Create a star graph: center -> N nodes
        let mut nodes = vec![make_node("center", "file", Some("center.ts"))];
        let mut edges = vec![];

        for i in 0..10 {
            let id = format!("file:leaf{}.ts", i);
            nodes.push(make_node(&id, "file", Some(&format!("leaf{}.ts", i))));
            edges.push(make_edge("center", &id, "imports"));
        }

        let node_by_id: HashMap<String, &KgNode> =
            nodes.iter().map(|n| (n.id.clone(), n)).collect();

        let mut outgoing: HashMap<String, Vec<&KgEdge>> = HashMap::new();
        let mut incoming: HashMap<String, Vec<&KgEdge>> = HashMap::new();

        for edge in &edges {
            outgoing.entry(edge.from.clone()).or_default().push(edge);
            incoming.entry(edge.to.clone()).or_default().push(edge);
        }

        // Start from center with max_nodes=5
        let subgraph = build_subgraph(
            "center",
            &node_by_id,
            &outgoing,
            &incoming,
            1, // max_hops
            5, // max_nodes (should limit)
            3, // max_edges (should limit)
        );

        // Should be limited
        assert!(subgraph.nodes.len() <= 5);
        assert!(subgraph.edges.len() <= 3);
    }

    #[test]
    fn test_ask_kg_result_serialization() {
        let result = AskKgResult {
            reason: "Test reason".to_string(),
            root_node_ids: vec!["file:test.ts".to_string()],
            nodes: vec![make_node("file:test.ts", "file", Some("test.ts"))],
            edges: vec![],
        };

        let json = serde_json::to_string(&result).unwrap();
        assert!(json.contains("\"reason\""));
        assert!(json.contains("\"rootNodeIds\""));
        assert!(json.contains("\"nodes\""));
        assert!(json.contains("\"edges\""));
    }

    // ====================================================================
    // Exhaustive Query Tests (Phase 9.3.1)
    // ====================================================================

    #[test]
    fn test_detect_exhaustive_intent_english() {
        // "all routes" -> should detect endpoint kind
        let intent = detect_exhaustive_intent("What are all the routes?");
        assert!(intent.is_exhaustive);
        assert!(intent.target_kinds.contains(&"endpoint".to_string()));

        // "list all functions" -> should detect function kind
        let intent = detect_exhaustive_intent("List all functions in the project");
        assert!(intent.is_exhaustive);
        assert!(intent.target_kinds.contains(&"function".to_string()));

        // "every GET handler" -> should detect endpoint + GET filter
        let intent = detect_exhaustive_intent("Show every GET handler");
        assert!(intent.is_exhaustive);
        assert!(intent.target_kinds.contains(&"endpoint".to_string()));
        assert_eq!(intent.http_method_filter, Some("GET".to_string()));

        // Regular question -> not exhaustive
        let intent = detect_exhaustive_intent("How does authentication work?");
        assert!(!intent.is_exhaustive);
    }

    #[test]
    fn test_detect_exhaustive_intent_portuguese() {
        // "todas as rotas" -> should detect endpoint kind
        let intent = detect_exhaustive_intent("Quais são todas as rotas?");
        assert!(intent.is_exhaustive);
        assert!(intent.target_kinds.contains(&"endpoint".to_string()));

        // "todos os métodos GET" -> should detect function + GET filter
        let intent = detect_exhaustive_intent("Liste todos os métodos GET");
        assert!(intent.is_exhaustive);
        assert_eq!(intent.http_method_filter, Some("GET".to_string()));

        // "mostre todas as funções" -> should detect function kind
        let intent = detect_exhaustive_intent("Mostre todas as funções");
        assert!(intent.is_exhaustive);
        assert!(intent.target_kinds.contains(&"function".to_string()));
    }

    #[test]
    fn test_detect_exhaustive_intent_http_methods() {
        let methods = [
            ("all POST endpoints", "POST"),
            ("every PUT handler", "PUT"),
            ("list DELETE routes", "DELETE"),
            ("show all PATCH methods", "PATCH"),
        ];

        for (query, expected_method) in methods {
            let intent = detect_exhaustive_intent(query);
            assert!(
                intent.is_exhaustive,
                "Query '{}' should be exhaustive",
                query
            );
            assert_eq!(
                intent.http_method_filter,
                Some(expected_method.to_string()),
                "Query '{}' should have {} method filter",
                query,
                expected_method
            );
        }
    }

    fn make_endpoint_node(id: &str, route: &str, method: &str) -> KgNode {
        KgNode {
            id: id.to_string(),
            kind: "endpoint".to_string(),
            label: route.to_string(),
            props: serde_json::json!({
                "route": route,
                "method": method
            }),
            branch: Some("main".to_string()),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    fn make_function_node(id: &str, name: &str) -> KgNode {
        KgNode {
            id: id.to_string(),
            kind: "function".to_string(),
            label: name.to_string(),
            props: serde_json::json!({"name": name}),
            branch: Some("main".to_string()),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    #[test]
    fn test_search_kg_exhaustive_all_endpoints() {
        let nodes = vec![
            make_endpoint_node("ep:1", "/api/users", "GET"),
            make_endpoint_node("ep:2", "/api/users", "POST"),
            make_endpoint_node("ep:3", "/api/items", "GET"),
            make_function_node("fn:1", "handleRequest"),
            make_node("file:api.ts", "file", Some("api.ts")),
        ];
        let edges = vec![
            make_edge("ep:1", "file:api.ts", "definedIn"),
            make_edge("ep:2", "file:api.ts", "definedIn"),
        ];

        let intent = detect_exhaustive_intent("List all endpoints");
        let result = search_kg_exhaustive(&nodes, &edges, &intent, 100);

        assert!(result.is_some());
        let result = result.unwrap();

        // Should find all 3 endpoints
        assert_eq!(result.nodes.len(), 3);
        assert!(result.nodes.iter().all(|n| n.kind == "endpoint"));
    }

    #[test]
    fn test_search_kg_exhaustive_get_endpoints_only() {
        let nodes = vec![
            make_endpoint_node("ep:1", "/api/users", "GET"),
            make_endpoint_node("ep:2", "/api/users", "POST"),
            make_endpoint_node("ep:3", "/api/items", "GET"),
            make_endpoint_node("ep:4", "/api/orders", "DELETE"),
        ];
        let edges = vec![];

        let intent = detect_exhaustive_intent("Show all GET routes");
        let result = search_kg_exhaustive(&nodes, &edges, &intent, 100);

        assert!(result.is_some());
        let result = result.unwrap();

        // Should find only GET endpoints
        assert_eq!(result.nodes.len(), 2);
        for node in &result.nodes {
            let method = node.props.get("method").and_then(|v| v.as_str());
            assert_eq!(method, Some("GET"));
        }
    }

    #[test]
    fn test_search_kg_exhaustive_all_functions() {
        let nodes = vec![
            make_function_node("fn:1", "handleRequest"),
            make_function_node("fn:2", "validateInput"),
            make_function_node("fn:3", "processData"),
            make_endpoint_node("ep:1", "/api/test", "GET"),
            make_node("file:utils.ts", "file", Some("utils.ts")),
        ];
        let edges = vec![];

        let intent = detect_exhaustive_intent("What are all the functions?");
        let result = search_kg_exhaustive(&nodes, &edges, &intent, 100);

        assert!(result.is_some());
        let result = result.unwrap();

        // Should find all 3 functions
        assert_eq!(result.nodes.len(), 3);
        assert!(result.nodes.iter().all(|n| n.kind == "function"));
    }

    #[test]
    fn test_search_kg_exhaustive_respects_max_nodes() {
        let mut nodes: Vec<KgNode> = Vec::new();
        for i in 0..50 {
            nodes.push(make_endpoint_node(
                &format!("ep:{}", i),
                &format!("/api/route{}", i),
                "GET",
            ));
        }

        let intent = detect_exhaustive_intent("List all routes");
        let result = search_kg_exhaustive(&nodes, &[], &intent, 10);

        assert!(result.is_some());
        let result = result.unwrap();

        // Should be limited to 10 nodes
        assert_eq!(result.nodes.len(), 10);
    }

    #[test]
    fn test_search_kg_exhaustive_no_match() {
        let nodes = vec![
            make_node("file:a.ts", "file", Some("a.ts")),
            make_node("file:b.ts", "file", Some("b.ts")),
        ];

        // Looking for endpoints but there are none
        let intent = detect_exhaustive_intent("List all endpoints");
        let result = search_kg_exhaustive(&nodes, &[], &intent, 100);

        assert!(result.is_none());
    }

    #[test]
    fn test_search_kg_exhaustive_includes_related_edges() {
        let nodes = vec![
            make_endpoint_node("ep:1", "/api/users", "GET"),
            make_endpoint_node("ep:2", "/api/items", "GET"),
            make_node("file:api.ts", "file", Some("api.ts")),
        ];
        let edges = vec![
            make_edge("ep:1", "file:api.ts", "definedIn"),
            make_edge("ep:2", "file:api.ts", "definedIn"),
            make_edge("file:api.ts", "file:utils.ts", "imports"), // unrelated edge
        ];

        let intent = detect_exhaustive_intent("List all GET endpoints");
        let result = search_kg_exhaustive(&nodes, &edges, &intent, 100);

        assert!(result.is_some());
        let result = result.unwrap();

        // Should include edges connecting to/from the endpoints
        assert_eq!(result.edges.len(), 2);
    }
}
