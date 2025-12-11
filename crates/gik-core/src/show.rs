//! Show command implementation for GIK.
//!
//! This module provides the core logic for `gik show`, which inspects a single
//! knowledge revision similar to `git show`. It displays revision metadata,
//! base impacts, KG summaries, and sources.
//!
//! ## Usage
//!
//! ```ignore
//! use gik_core::{GikEngine, ShowOptions};
//!
//! let engine = GikEngine::with_defaults()?;
//! let workspace = engine.resolve_workspace(Path::new("."))?;
//!
//! let opts = ShowOptions::default();
//! let report = engine.show(&workspace, opts)?;
//! println!("{}", report.render_text());
//! ```

use std::fmt;

use serde::{Deserialize, Serialize};

use crate::base::{base_root, load_base_sources, sources_path};
use crate::errors::GikError;
use crate::kg::{kg_exists, read_stats as kg_read_stats};
use crate::memory::MEMORY_BASE_NAME;
use crate::timeline::{get_revision, resolve_revision_ref, Revision, RevisionOperation};
use crate::workspace::Workspace;

// ============================================================================
// ShowOptions
// ============================================================================

/// Options for the show command.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ShowOptions {
    /// Optional explicit knowledge branch (uses current branch if None).
    pub branch: Option<String>,

    /// Revision reference to show (e.g., "HEAD", "HEAD~1", or explicit id).
    /// Defaults to HEAD if None.
    pub revision_ref: Option<String>,

    /// Maximum number of source paths to include in the report.
    pub max_sources: Option<usize>,

    /// Maximum number of memory entry summaries to include.
    pub max_memory_entries: Option<usize>,

    /// Maximum number of KG nodes to include in export (for --kg-dot).
    pub max_kg_nodes: Option<usize>,

    /// Maximum number of KG edges to include in export (for --kg-dot).
    pub max_kg_edges: Option<usize>,
}

impl ShowOptions {
    /// Create new ShowOptions with default values.
    pub fn new() -> Self {
        Self::default()
    }

    /// Set the branch to inspect.
    pub fn with_branch(mut self, branch: impl Into<String>) -> Self {
        self.branch = Some(branch.into());
        self
    }

    /// Set the revision reference to inspect.
    pub fn with_revision_ref(mut self, rev_ref: impl Into<String>) -> Self {
        self.revision_ref = Some(rev_ref.into());
        self
    }

    /// Set the maximum number of sources to include.
    pub fn with_max_sources(mut self, max: usize) -> Self {
        self.max_sources = Some(max);
        self
    }

    /// Set the maximum number of memory entries to include.
    pub fn with_max_memory_entries(mut self, max: usize) -> Self {
        self.max_memory_entries = Some(max);
        self
    }

    /// Get the effective max_sources limit (default: 20).
    pub fn effective_max_sources(&self) -> usize {
        self.max_sources.unwrap_or(20)
    }

    /// Get the effective max_memory_entries limit (default: 10).
    pub fn effective_max_memory_entries(&self) -> usize {
        self.max_memory_entries.unwrap_or(10)
    }
}

// ============================================================================
// BaseImpact
// ============================================================================

/// Impact summary for a single base in a revision.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BaseImpact {
    /// Base name (e.g., "code", "docs", "memory").
    pub base: String,

    /// Number of sources added in this revision (if known).
    pub sources_added: Option<u64>,

    /// Number of sources updated in this revision (if known).
    pub sources_updated: Option<u64>,

    /// Number of vectors/chunks added (if known).
    pub vectors_added: Option<u64>,

    /// Number of memory entries added (for memory base).
    pub memory_entries_added: Option<u64>,
}

impl BaseImpact {
    /// Create a new BaseImpact for a base.
    pub fn new(base: impl Into<String>) -> Self {
        Self {
            base: base.into(),
            sources_added: None,
            sources_updated: None,
            vectors_added: None,
            memory_entries_added: None,
        }
    }

    /// Set sources added count.
    pub fn with_sources_added(mut self, count: u64) -> Self {
        self.sources_added = Some(count);
        self
    }

    /// Set memory entries added count.
    pub fn with_memory_entries_added(mut self, count: u64) -> Self {
        self.memory_entries_added = Some(count);
        self
    }
}

// ============================================================================
// KgImpactSummary
// ============================================================================

/// Knowledge Graph impact summary for a revision.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct KgImpactSummary {
    /// Number of nodes added in this revision (approximate or None).
    pub nodes_added: Option<u64>,

    /// Number of edges added in this revision (approximate or None).
    pub edges_added: Option<u64>,

    /// Total nodes in KG (cumulative snapshot).
    pub total_nodes: Option<u64>,

    /// Total edges in KG (cumulative snapshot).
    pub total_edges: Option<u64>,
}

// ============================================================================
// ShowReport
// ============================================================================

/// Complete report for a single revision inspection.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ShowReport {
    /// The revision ID being shown.
    pub revision_id: String,

    /// The kind of revision (e.g., "Commit", "Reindex", "MemoryIngest").
    pub revision_kind: String,

    /// The branch this revision belongs to.
    pub branch: String,

    /// Author of the revision (if available).
    pub author: Option<String>,

    /// Timestamp of the revision.
    pub timestamp: String,

    /// Human-readable message for the revision.
    pub message: String,

    /// Git commit hash (if associated).
    pub git_commit: Option<String>,

    /// Parent revision ID (if not root).
    pub parent_id: Option<String>,

    /// Raw operation summary as JSON (for detailed inspection).
    pub operation_summary: serde_json::Value,

    /// Per-base impact summaries.
    pub bases: Vec<BaseImpact>,

    /// KG impact summary (if KG exists).
    pub kg_impact: Option<KgImpactSummary>,

    /// Truncated list of source paths associated with this revision.
    pub sources: Vec<String>,

    /// Truncated list of memory entry summaries (title or first line).
    pub memory_summaries: Vec<String>,
}

impl ShowReport {
    /// Create a ShowReport from a Revision.
    pub fn from_revision(revision: &Revision) -> Self {
        let revision_kind = revision
            .operations
            .first()
            .map(operation_kind_name)
            .unwrap_or_else(|| "Unknown".to_string());

        let operation_summary = revision
            .operations
            .first()
            .map(|op| serde_json::to_value(op).unwrap_or(serde_json::Value::Null))
            .unwrap_or(serde_json::Value::Null);

        Self {
            revision_id: revision.id.to_string(),
            revision_kind,
            branch: revision.branch.clone(),
            author: None, // GIK doesn't track author yet
            timestamp: revision.timestamp.to_rfc3339(),
            message: revision.message.clone(),
            git_commit: revision.git_commit.clone(),
            parent_id: revision.parent_id.as_ref().map(|id| id.to_string()),
            operation_summary,
            bases: Vec::new(),
            kg_impact: None,
            sources: Vec::new(),
            memory_summaries: Vec::new(),
        }
    }

    /// Render the report as human-readable text.
    pub fn render_text(&self) -> String {
        let mut lines = Vec::new();

        // Header
        lines.push(format!("Revision: {}", self.revision_id));
        lines.push(format!("Type:     {}", self.revision_kind));
        lines.push(format!("Branch:   {}", self.branch));
        if let Some(author) = &self.author {
            lines.push(format!("Author:   {}", author));
        }
        lines.push(format!("Time:     {}", self.timestamp));
        if let Some(git) = &self.git_commit {
            lines.push(format!("Git:      {}", git));
        }
        if let Some(parent) = &self.parent_id {
            lines.push(format!("Parent:   {}", &parent[..8.min(parent.len())]));
        }

        // Message
        if !self.message.is_empty() {
            lines.push(String::new());
            lines.push(format!("    {}", self.message));
        }

        // Base impacts
        if !self.bases.is_empty() {
            lines.push(String::new());
            lines.push("Base Impact:".to_string());
            for base in &self.bases {
                let mut parts = vec![base.base.clone()];
                if let Some(n) = base.sources_added {
                    parts.push(format!("+{} sources", n));
                }
                if let Some(n) = base.vectors_added {
                    parts.push(format!("{} vectors", n));
                }
                if let Some(n) = base.memory_entries_added {
                    parts.push(format!("+{} entries", n));
                }
                lines.push(format!("  - {}", parts.join(", ")));
            }
        }

        // KG impact
        if let Some(kg) = &self.kg_impact {
            lines.push(String::new());
            lines.push("KG Impact:".to_string());
            if let Some(n) = kg.nodes_added {
                lines.push(format!("  Nodes added: {}", n));
            }
            if let Some(n) = kg.edges_added {
                lines.push(format!("  Edges added: {}", n));
            }
            if let Some(n) = kg.total_nodes {
                lines.push(format!("  Total nodes: {}", n));
            }
            if let Some(n) = kg.total_edges {
                lines.push(format!("  Total edges: {}", n));
            }
        }

        // Sources
        if !self.sources.is_empty() {
            lines.push(String::new());
            lines.push(format!("Sources ({}):", self.sources.len()));
            for src in &self.sources {
                lines.push(format!("  {}", src));
            }
        }

        // Memory summaries
        if !self.memory_summaries.is_empty() {
            lines.push(String::new());
            lines.push(format!("Memory Entries ({}):", self.memory_summaries.len()));
            for mem in &self.memory_summaries {
                lines.push(format!("  - {}", mem));
            }
        }

        lines.join("\n")
    }
}

impl fmt::Display for ShowReport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.render_text())
    }
}

// ============================================================================
// Helper Functions
// ============================================================================

/// Get a human-readable name for a RevisionOperation.
fn operation_kind_name(op: &RevisionOperation) -> String {
    match op {
        RevisionOperation::Init => "Init".to_string(),
        RevisionOperation::Commit { .. } => "Commit".to_string(),
        RevisionOperation::MemoryIngest { .. } => "MemoryIngest".to_string(),
        RevisionOperation::MemoryPrune { .. } => "MemoryPrune".to_string(),
        RevisionOperation::Reindex { .. } => "Reindex".to_string(),
        RevisionOperation::Release { .. } => "Release".to_string(),
        RevisionOperation::Custom { name, .. } => format!("Custom({})", name),
    }
}

/// Extract base names from a revision's operations.
fn extract_bases_from_operations(ops: &[RevisionOperation]) -> Vec<String> {
    let mut bases = Vec::new();
    for op in ops {
        match op {
            RevisionOperation::Commit {
                bases: op_bases, ..
            } => {
                bases.extend(op_bases.iter().cloned());
            }
            RevisionOperation::Reindex { base, .. } => {
                bases.push(base.clone());
            }
            RevisionOperation::MemoryIngest { .. } | RevisionOperation::MemoryPrune { .. } => {
                if !bases.contains(&MEMORY_BASE_NAME.to_string()) {
                    bases.push(MEMORY_BASE_NAME.to_string());
                }
            }
            _ => {}
        }
    }
    bases.sort();
    bases.dedup();
    bases
}

// ============================================================================
// Engine Integration
// ============================================================================

/// Run the show command for a workspace.
///
/// This is the core implementation that the GikEngine delegates to.
pub fn run_show(
    workspace: &Workspace,
    branch: &str,
    opts: ShowOptions,
) -> Result<ShowReport, GikError> {
    // Check initialization
    if !workspace.is_initialized() {
        return Err(GikError::NotInitialized);
    }

    let timeline_path = workspace.timeline_path(branch);
    let head_path = workspace.head_path(branch);

    // Resolve revision reference
    let ref_str = opts.revision_ref.as_deref().unwrap_or("HEAD");
    let revision_id = resolve_revision_ref(&timeline_path, &head_path, ref_str)?;

    // Load the revision
    let revision = get_revision(&timeline_path, &revision_id)?.ok_or_else(|| {
        GikError::RevisionNotFound(format!("Revision '{}' not found in timeline", revision_id))
    })?;

    // Build initial report from revision
    let mut report = ShowReport::from_revision(&revision);

    // Extract bases from operations
    let bases = extract_bases_from_operations(&revision.operations);

    // Compute base impacts from operations
    for base_name in &bases {
        let mut impact = BaseImpact::new(base_name);

        // Extract counts directly from the revision's operations
        for op in &revision.operations {
            match op {
                RevisionOperation::Commit {
                    bases: op_bases,
                    source_count,
                } => {
                    if op_bases.contains(base_name) {
                        impact.sources_added = Some(*source_count as u64);
                    }
                }
                RevisionOperation::MemoryIngest { count } => {
                    if base_name == MEMORY_BASE_NAME {
                        impact.memory_entries_added = Some(*count as u64);
                    }
                }
                RevisionOperation::Reindex { base, .. } => {
                    if base == base_name {
                        // Reindex doesn't add sources, but we note the base was touched
                        impact.sources_updated = Some(0); // Marker that it was reindexed
                    }
                }
                _ => {}
            }
        }

        report.bases.push(impact);
    }

    // Compute KG impact (if KG exists)
    if kg_exists(workspace, branch) {
        if let Ok(stats) = kg_read_stats(workspace, branch) {
            // Note: These are cumulative stats, not per-revision deltas.
            // Per-revision tracking would require KG to store revision provenance.
            report.kg_impact = Some(KgImpactSummary {
                nodes_added: None, // TODO: Track per-revision KG changes
                edges_added: None,
                total_nodes: Some(stats.node_count),
                total_edges: Some(stats.edge_count),
            });
        }
    }

    // Collect sample sources (truncated) - filter by revision_id
    let max_sources = opts.effective_max_sources();
    let knowledge_root = workspace.knowledge_root();
    for base_name in &bases {
        let base_dir = base_root(knowledge_root, branch, base_name);
        let src_path = sources_path(&base_dir);
        if let Ok(entries) = load_base_sources(&src_path) {
            // Filter entries that belong to this revision
            let rev_sources: Vec<_> = entries
                .iter()
                .filter(|e| e.revision_id == revision_id.as_str())
                .collect();

            for entry in rev_sources
                .iter()
                .take(max_sources.saturating_sub(report.sources.len()))
            {
                // For memory base, strip the "memory://" prefix
                let path = if base_name == MEMORY_BASE_NAME {
                    entry
                        .file_path
                        .strip_prefix("memory://")
                        .unwrap_or(&entry.file_path)
                        .to_string()
                } else {
                    entry.file_path.clone()
                };
                report.sources.push(path);
            }
        }
    }
    report.sources.truncate(max_sources);

    // Collect memory summaries (truncated) - from extra metadata or text
    let max_memory = opts.effective_max_memory_entries();
    if bases.contains(&MEMORY_BASE_NAME.to_string()) {
        let base_dir = base_root(knowledge_root, branch, MEMORY_BASE_NAME);
        let src_path = sources_path(&base_dir);
        if let Ok(entries) = load_base_sources(&src_path) {
            let memory_entries: Vec<_> = entries
                .iter()
                .filter(|e| e.revision_id == revision_id.as_str())
                .take(max_memory)
                .collect();

            for entry in memory_entries {
                // Try to get title from extra metadata, fallback to truncated text
                let summary = entry
                    .extra
                    .as_ref()
                    .and_then(|ex| ex.get("title"))
                    .and_then(|v| v.as_str())
                    .map(String::from)
                    .or_else(|| entry.text.as_ref().map(|t| truncate_str(t, 60)))
                    .unwrap_or_else(|| entry.file_path.clone());
                report.memory_summaries.push(summary);
            }
        }
    }

    Ok(report)
}

/// Truncate a string to a maximum length, adding "..." if truncated.
fn truncate_str(s: &str, max_len: usize) -> String {
    if s.len() <= max_len {
        s.to_string()
    } else {
        format!("{}...", &s[..max_len.saturating_sub(3)])
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_show_options_defaults() {
        let opts = ShowOptions::default();
        assert!(opts.branch.is_none());
        assert!(opts.revision_ref.is_none());
        assert_eq!(opts.effective_max_sources(), 20);
        assert_eq!(opts.effective_max_memory_entries(), 10);
    }

    #[test]
    fn test_show_options_builder() {
        let opts = ShowOptions::new()
            .with_branch("feature")
            .with_revision_ref("HEAD~1")
            .with_max_sources(5);

        assert_eq!(opts.branch, Some("feature".to_string()));
        assert_eq!(opts.revision_ref, Some("HEAD~1".to_string()));
        assert_eq!(opts.effective_max_sources(), 5);
    }

    #[test]
    fn test_base_impact_builder() {
        let impact = BaseImpact::new("code")
            .with_sources_added(10)
            .with_memory_entries_added(5);

        assert_eq!(impact.base, "code");
        assert_eq!(impact.sources_added, Some(10));
        assert_eq!(impact.memory_entries_added, Some(5));
    }

    #[test]
    fn test_operation_kind_name() {
        assert_eq!(operation_kind_name(&RevisionOperation::Init), "Init");
        assert_eq!(
            operation_kind_name(&RevisionOperation::Commit {
                bases: vec![],
                source_count: 0
            }),
            "Commit"
        );
        assert_eq!(
            operation_kind_name(&RevisionOperation::MemoryIngest { count: 5 }),
            "MemoryIngest"
        );
    }

    #[test]
    fn test_show_report_render_text() {
        let report = ShowReport {
            revision_id: "abc12345-def6-7890".to_string(),
            revision_kind: "Commit".to_string(),
            branch: "main".to_string(),
            author: None,
            timestamp: "2025-11-28T10:00:00Z".to_string(),
            message: "Add initial code".to_string(),
            git_commit: Some("abcdef1234567890".to_string()),
            parent_id: Some("parent123".to_string()),
            operation_summary: serde_json::json!({"type": "Commit"}),
            bases: vec![BaseImpact::new("code").with_sources_added(5)],
            kg_impact: None,
            sources: vec!["src/main.rs".to_string()],
            memory_summaries: vec![],
        };

        let text = report.render_text();
        assert!(text.contains("Revision: abc12345-def6-7890"));
        assert!(text.contains("Type:     Commit"));
        assert!(text.contains("Branch:   main"));
        assert!(text.contains("Add initial code"));
        assert!(text.contains("code, +5 sources"));
        assert!(text.contains("src/main.rs"));
    }

    #[test]
    fn test_truncate_str() {
        assert_eq!(truncate_str("short", 10), "short");
        assert_eq!(truncate_str("this is a long string", 10), "this is...");
    }
}
