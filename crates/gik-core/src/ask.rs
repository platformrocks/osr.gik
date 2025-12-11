//! Ask pipeline and context bundle for GIK.
//!
//! This module provides the RAG-style query pipeline for GIK:
//! - [`AskContextBundle`] - the canonical output of `gik ask`
//! - [`RagChunk`] - a single retrieved chunk
//! - [`StackSummary`] - aggregated project fingerprint
//! - [`AskDebugInfo`] - technical metadata for debugging
//! - [`run_ask`] - orchestrates the ask pipeline
//!
//! ## Flow
//!
//! 1. Verify workspace is initialized and has indexed bases
//! 2. Embed the query using the active embedding backend
//! 3. Run vector searches across specified (or all) bases
//! 4. Collect RagChunks with snippets from base sources
//! 5. Build StackSummary from stack inventory
//! 6. Return AskContextBundle with all results

use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::base::{base_root, load_base_sources, BaseSourceEntry};
use crate::bm25::{load_bm25_index, rrf_fusion, Bm25Index, HybridSearchConfig};
use crate::config::{DevicePreference, GlobalConfig};
use crate::embedding::{create_backend, EmbeddingBackend};
use crate::errors::GikError;
use crate::query_expansion::{average_embeddings, QueryExpander};
use crate::reranker::get_or_init_reranker_backend;
use crate::stack::{read_stats_json, read_tech_jsonl, StackStats, StackTechEntry};
use crate::timeline::{read_head, RevisionId};
use crate::vector_index::{
    load_index_meta, open_vector_index, VectorIndexBackend, VectorIndexBackendKind,
    VectorIndexConfig, VectorMetric, VectorSearchResult,
};
use crate::workspace::{BranchName, Workspace};

// ============================================================================
// Constants
// ============================================================================

/// Default number of chunks to return per base.
pub const DEFAULT_TOP_K: usize = 8;

/// Well-known bases that support RAG queries.
pub const RAG_BASES: &[&str] = &["code", "docs", "memory"];

/// Boost applied to chunks with exact filename match in query.
const FILENAME_EXACT_BOOST: f32 = 0.50;

/// Boost applied to chunks with partial path match.
const FILENAME_PARTIAL_BOOST: f32 = 0.25;

// ============================================================================
// FilenameMatch - for filename-aware ask
// ============================================================================

/// Detected filename or path pattern in a query.
///
/// Used to strongly boost chunks that match the detected filename.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FilenameMatch {
    /// The detected filename or path pattern from the query.
    pub detected_pattern: String,

    /// The file extension (if any).
    pub extension: Option<String>,

    /// Whether this was an explicit file reference (e.g., "in file.rs").
    pub is_explicit: bool,
}

impl FilenameMatch {
    /// Check if a path matches this filename pattern.
    pub fn matches_path(&self, path: &str) -> bool {
        let path_lower = path.to_lowercase();
        let pattern_lower = self.detected_pattern.to_lowercase();

        // Exact filename match (at end of path)
        if let Some(filename) = path_lower.rsplit('/').next() {
            if filename == pattern_lower {
                return true;
            }
            // Match without extension
            if let Some(name_part) = filename.rsplit('.').last() {
                if name_part == pattern_lower.rsplit('.').last().unwrap_or(&pattern_lower) {
                    return true;
                }
            }
        }

        // Path contains the pattern
        path_lower.contains(&pattern_lower)
    }
}

/// Detect a filename or path pattern in a query.
///
/// Uses regex to detect common patterns like:
/// - "in file.rs", "in src/main.rs"
/// - "the utils.py file"
/// - "MyComponent.tsx"
/// - bare filenames with known extensions
///
/// Returns `None` if no filename pattern is detected.
pub fn detect_filename_in_query(query: &str) -> Option<FilenameMatch> {
    use regex::Regex;

    // Common code file extensions for detection
    const KNOWN_EXTENSIONS: &[&str] = &[
        "rs", "py", "js", "ts", "jsx", "tsx", "go", "java", "c", "cpp", "h", "hpp", "cs", "rb",
        "php", "swift", "kt", "scala", "clj", "ex", "exs", "erl", "hs", "ml", "fs", "r", "jl",
        "lua", "pl", "sh", "bash", "zsh", "vue", "svelte", "json", "yaml", "yml", "toml", "xml",
        "md", "txt", "css", "scss", "sass", "less", "html", "sql", "graphql", "proto",
    ];

    // Pattern 1: "in <path/file.ext>" or "from <path/file.ext>"
    // e.g., "in src/main.rs", "from utils.py"
    let explicit_pattern = Regex::new(r"(?i)\b(?:in|from|of|at)\s+([a-zA-Z0-9_\-./]+\.[a-zA-Z0-9]+)")
        .ok()?;
    if let Some(caps) = explicit_pattern.captures(query) {
        if let Some(m) = caps.get(1) {
            let path = m.as_str();
            let ext = path.rsplit('.').next().map(|s| s.to_lowercase());
            if ext.as_ref().map(|e| KNOWN_EXTENSIONS.contains(&e.as_str())).unwrap_or(false) {
                return Some(FilenameMatch {
                    detected_pattern: path.to_string(),
                    extension: ext,
                    is_explicit: true,
                });
            }
        }
    }

    // Pattern 2: "the <filename> file" or "<filename> file"
    // e.g., "the utils.py file"
    let file_pattern = Regex::new(r"(?i)\b(?:the\s+)?([a-zA-Z0-9_\-]+\.[a-zA-Z0-9]+)\s+file")
        .ok()?;
    if let Some(caps) = file_pattern.captures(query) {
        if let Some(m) = caps.get(1) {
            let filename = m.as_str();
            let ext = filename.rsplit('.').next().map(|s| s.to_lowercase());
            if ext.as_ref().map(|e| KNOWN_EXTENSIONS.contains(&e.as_str())).unwrap_or(false) {
                return Some(FilenameMatch {
                    detected_pattern: filename.to_string(),
                    extension: ext,
                    is_explicit: true,
                });
            }
        }
    }

    // Pattern 3: Bare filename with known extension (less confident)
    // e.g., "MyComponent.tsx", "config.yaml"
    let bare_pattern = Regex::new(r"\b([A-Z][a-zA-Z0-9_]*\.[a-zA-Z0-9]+)\b")
        .ok()?;
    if let Some(caps) = bare_pattern.captures(query) {
        if let Some(m) = caps.get(1) {
            let filename = m.as_str();
            let ext = filename.rsplit('.').next().map(|s| s.to_lowercase());
            if ext.as_ref().map(|e| KNOWN_EXTENSIONS.contains(&e.as_str())).unwrap_or(false) {
                return Some(FilenameMatch {
                    detected_pattern: filename.to_string(),
                    extension: ext,
                    is_explicit: false,
                });
            }
        }
    }

    // Pattern 4: lowercase filename with extension
    // e.g., "main.rs", "index.ts"
    let lowercase_pattern = Regex::new(r"\b([a-z][a-z0-9_\-]*\.[a-zA-Z0-9]+)\b")
        .ok()?;
    for caps in lowercase_pattern.captures_iter(query) {
        if let Some(m) = caps.get(1) {
            let filename = m.as_str();
            let ext = filename.rsplit('.').next().map(|s| s.to_lowercase());
            if ext.as_ref().map(|e| KNOWN_EXTENSIONS.contains(&e.as_str())).unwrap_or(false) {
                return Some(FilenameMatch {
                    detected_pattern: filename.to_string(),
                    extension: ext,
                    is_explicit: false,
                });
            }
        }
    }

    None
}

// ============================================================================
// AskContextBundle
// ============================================================================

/// Canonical output of `gik ask`, used by external tools.
///
/// Contains all retrieved context for a query: RAG chunks from code/docs bases,
/// memory events, KG subgraphs, and a stack summary.
///
/// ## Unified Context (Phase 9.3)
///
/// `AskContextBundle` is the single unified view of context for the LLM:
/// - **text**: RAG snippets from code/docs bases (`rag_chunks`)
/// - **memory**: decisions/notes from memory base (`memory_events`)
/// - **structure**: KG subgraphs with file/endpoint relations (`kg_results`)
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AskContextBundle {
    /// The knowledge revision used for this query.
    pub revision_id: RevisionId,

    /// The original question string.
    pub question: String,

    /// The bases that were consulted.
    pub bases: Vec<String>,

    /// Retrieved RAG chunks sorted by relevance.
    pub rag_chunks: Vec<RagChunk>,

    /// Knowledge graph subgraphs (Phase 9.3).
    ///
    /// Contains structural context derived from the KG:
    /// - File/doc nodes related to RAG chunks
    /// - Import relationships between files
    /// - Endpoint nodes and their defining files
    #[serde(default)]
    pub kg_results: Vec<AskKgResult>,

    /// Relevant memory events from the memory base.
    #[serde(default)]
    pub memory_events: Vec<MemoryEvent>,

    /// Optional project fingerprint summary.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stack_summary: Option<StackSummary>,

    /// Technical metadata for debugging.
    pub debug: AskDebugInfo,
}

// ============================================================================
// RagChunk
// ============================================================================

/// A single chunk of retrieved context from a knowledge base.
///
/// Represents a snippet of code or documentation that is relevant to the query.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RagChunk {
    /// The base this chunk came from ("code" or "docs").
    pub base: String,

    /// Final combined score (0.0 to 1.0, higher is better).
    /// When reranker is used, this is the hybrid score combining dense + reranker.
    pub score: f32,

    /// Path to the source file (workspace-relative).
    pub path: String,

    /// Starting line number in the source file (1-based).
    pub start_line: u32,

    /// Ending line number in the source file (1-based, inclusive).
    pub end_line: u32,

    /// The text content snippet.
    pub snippet: String,

    /// Original dense embedding similarity score (before reranking).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dense_score: Option<f32>,

    /// Cross-encoder reranker score (when reranker is used).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reranker_score: Option<f32>,
}

impl RagChunk {
    /// Create a RagChunk from a base source entry and search result.
    pub fn from_source_and_result(entry: &BaseSourceEntry, result: &VectorSearchResult) -> Self {
        Self {
            base: entry.base.clone(),
            score: result.score,
            path: entry.file_path.clone(),
            start_line: entry.start_line,
            end_line: entry.end_line,
            snippet: entry.text.clone().unwrap_or_default(),
            dense_score: Some(result.score),
            reranker_score: None,
        }
    }
}

// ============================================================================
// KG Result Types
// ============================================================================

// Re-export AskKgResult as the canonical KG result type for ask
pub use crate::kg::query::AskKgResult;

// ============================================================================
// MemoryEvent (response type for ask)
// ============================================================================

/// A memory event relevant to the query, returned in AskContextBundle.
///
/// This is the response representation of a MemoryEntry for the ask pipeline.
/// Memory events capture decisions, notes, and context from past interactions.
///
/// See [`crate::memory::MemoryEntry`] for the storage representation.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MemoryEvent {
    /// Memory entry ID.
    pub id: String,

    /// Scope (project, branch, or global).
    pub scope: crate::memory::MemoryScope,

    /// Title/summary.
    pub title: String,

    /// Full text content.
    pub text: String,

    /// When the memory was created.
    pub created_at: chrono::DateTime<chrono::Utc>,

    /// Source type (manual_note, decision, observation, external_reference).
    pub source: crate::memory::MemorySource,

    /// Tags for categorization.
    #[serde(default)]
    pub tags: Vec<String>,

    /// Relevance score from vector search.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub score: Option<f32>,
}

impl MemoryEvent {
    /// Convert a RagChunk from the memory base into a MemoryEvent.
    ///
    /// Returns `None` if the chunk's extra metadata doesn't contain valid memory fields.
    pub fn from_base_source_entry(entry: &BaseSourceEntry, score: f32) -> Option<Self> {
        let extra = entry.extra.as_ref()?;

        // Extract memory ID from extra metadata
        let id = extra
            .get("memory_id")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())?;

        // Parse scope
        let scope = extra
            .get("memory_scope")
            .and_then(|v| v.as_str())
            .and_then(|s| s.parse::<crate::memory::MemoryScope>().ok())
            .unwrap_or_default();

        // Parse source
        let source = extra
            .get("memory_source")
            .and_then(|v| v.as_str())
            .and_then(|s| s.parse::<crate::memory::MemorySource>().ok())
            .unwrap_or_default();

        // Extract title (optional)
        let title = extra
            .get("title")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .unwrap_or_default();

        // Parse created_at timestamp
        let created_at = extra
            .get("created_at")
            .and_then(|v| v.as_str())
            .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
            .map(|dt| dt.with_timezone(&chrono::Utc))
            .unwrap_or_else(chrono::Utc::now);

        // Extract tags
        let tags = extra
            .get("tags")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(|s| s.to_string()))
                    .collect()
            })
            .unwrap_or_default();

        // Get text from entry
        let text = entry.text.clone().unwrap_or_default();

        Some(Self {
            id,
            scope,
            title,
            text,
            created_at,
            source,
            tags,
            score: Some(score),
        })
    }
}

// ============================================================================
// StackSummary
// ============================================================================

/// Aggregated project fingerprint for LLM context.
///
/// Provides a quick overview of the project's tech stack and structure.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StackSummary {
    /// Detected programming languages.
    pub languages: Vec<String>,

    /// Detected frameworks (e.g., "Next.js", "React").
    #[serde(default)]
    pub frameworks: Vec<String>,

    /// High-level service names, if detected.
    #[serde(default)]
    pub services: Vec<String>,

    /// Package managers in use (e.g., "cargo", "npm").
    pub managers: Vec<String>,

    /// Total file count in the project.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total_files: Option<u64>,

    // -------------------------------------------------------------------------
    // Phase 8.4: Scoped stack context
    // -------------------------------------------------------------------------

    /// Whether this stack was scoped to the query context.
    /// When true, only languages/frameworks from relevant files are included.
    #[serde(default)]
    pub scoped: bool,

    /// Number of unique files in RAG context (when scoped).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context_files: Option<usize>,

    /// Number of chunks in RAG context (when scoped).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context_chunks: Option<usize>,
}

impl StackSummary {
    /// Build a StackSummary from StackStats.
    ///
    /// Languages and managers are sorted alphabetically for deterministic output.
    /// Creates an unscoped (full project) summary.
    pub fn from_stats(stats: &StackStats) -> Self {
        let mut languages: Vec<String> = stats.languages.keys().cloned().collect();
        languages.sort();

        Self {
            languages,
            frameworks: Vec::new(),
            services: Vec::new(),
            managers: stats.managers.clone(), // Already sorted
            total_files: Some(stats.total_files),
            scoped: false,
            context_files: None,
            context_chunks: None,
        }
    }

    /// Build a StackSummary from StackStats and tech entries.
    ///
    /// Extracts frameworks (kind = "framework") and services (kind = "service")
    /// from the tech entries. All vectors are sorted alphabetically for
    /// deterministic output. Creates an unscoped (full project) summary.
    pub fn from_stats_with_tech(stats: &StackStats, tech: &[StackTechEntry]) -> Self {
        let mut languages: Vec<String> = stats.languages.keys().cloned().collect();
        languages.sort();

        let mut frameworks: Vec<String> = tech
            .iter()
            .filter(|t| t.kind == "framework")
            .map(|t| t.name.clone())
            .collect();
        frameworks.sort();

        let mut services: Vec<String> = tech
            .iter()
            .filter(|t| t.kind == "service")
            .map(|t| t.name.clone())
            .collect();
        services.sort();

        Self {
            languages,
            frameworks,
            services,
            managers: stats.managers.clone(), // Already sorted
            total_files: Some(stats.total_files),
            scoped: false,
            context_files: None,
            context_chunks: None,
        }
    }

    /// Build a scoped StackSummary from RAG chunks.
    ///
    /// Extracts language information only from the files present in the
    /// RAG context, providing a query-relevant tech stack overview.
    pub fn from_rag_context(chunks: &[RagChunk], all_stats: Option<&StackStats>) -> Self {
        use std::collections::HashSet;

        // Collect unique file paths from chunks
        let mut unique_files: HashSet<&str> = HashSet::new();
        for chunk in chunks {
            unique_files.insert(&chunk.path);
        }

        // Infer languages from file extensions
        let mut languages: HashSet<String> = HashSet::new();
        for path in &unique_files {
            if let Some(ext) = std::path::Path::new(path).extension() {
                let lang = match ext.to_str().unwrap_or("") {
                    "rs" => "Rust",
                    "py" => "Python",
                    "js" => "JavaScript",
                    "ts" => "TypeScript",
                    "jsx" => "JavaScript",
                    "tsx" => "TypeScript",
                    "go" => "Go",
                    "java" => "Java",
                    "c" | "h" => "C",
                    "cpp" | "cc" | "cxx" | "hpp" => "C++",
                    "rb" => "Ruby",
                    "php" => "PHP",
                    "swift" => "Swift",
                    "kt" | "kts" => "Kotlin",
                    "cs" => "C#",
                    "scala" => "Scala",
                    "hs" => "Haskell",
                    "ex" | "exs" => "Elixir",
                    "erl" => "Erlang",
                    "clj" | "cljs" => "Clojure",
                    "lua" => "Lua",
                    "r" => "R",
                    "sh" | "bash" => "Shell",
                    "sql" => "SQL",
                    "md" | "markdown" => "Markdown",
                    "json" => "JSON",
                    "yaml" | "yml" => "YAML",
                    "toml" => "TOML",
                    "xml" => "XML",
                    "html" | "htm" => "HTML",
                    "css" | "scss" | "sass" | "less" => "CSS",
                    _ => continue,
                };
                languages.insert(lang.to_string());
            }
        }

        let mut languages: Vec<String> = languages.into_iter().collect();
        languages.sort();

        // Use managers and frameworks from full stats if available
        let (managers, frameworks, services) = if let Some(stats) = all_stats {
            (
                stats.managers.clone(),
                Vec::new(), // Could be enhanced to filter by relevant files
                Vec::new(),
            )
        } else {
            (Vec::new(), Vec::new(), Vec::new())
        };

        Self {
            languages,
            frameworks,
            services,
            managers,
            total_files: all_stats.map(|s| s.total_files),
            scoped: true,
            context_files: Some(unique_files.len()),
            context_chunks: Some(chunks.len()),
        }
    }
}

// ============================================================================
// AskDebugInfo
// ============================================================================

/// Technical metadata for debugging the ask pipeline.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AskDebugInfo {
    /// The embedding model used for the query.
    pub embedding_model_id: String,

    /// The bases that were actually searched.
    pub used_bases: Vec<String>,

    /// Per-base result counts.
    #[serde(default)]
    pub per_base_counts: Vec<AskBaseCount>,

    /// Query embedding time in milliseconds.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub embed_time_ms: Option<u64>,

    /// Search time in milliseconds.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub search_time_ms: Option<u64>,

    /// Whether reranking was applied (Phase 8.2).
    #[serde(default)]
    pub reranker_used: bool,

    /// Reranking time in milliseconds (Phase 8.2).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rerank_time_ms: Option<u64>,

    /// Whether hybrid search (BM25 + Dense) was used.
    #[serde(default)]
    pub hybrid_search_used: bool,

    /// Number of dense results before fusion.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dense_result_count: Option<usize>,

    /// Number of sparse (BM25) results before fusion.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sparse_result_count: Option<usize>,

    /// Detected filename pattern from the query (Phase 8.4).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub filename_detected: Option<String>,
}

/// Per-base result count for debugging.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AskBaseCount {
    /// Base name.
    pub base: String,
    /// Number of results returned.
    pub count: usize,
}

// ============================================================================
// AskOptions
// ============================================================================

/// Options for the ask pipeline.
#[derive(Debug, Clone)]
pub struct AskOptions {
    /// The query string.
    pub question: String,

    /// Restrict query to specific bases (None = auto-detect RAG bases).
    pub bases: Option<Vec<String>>,

    /// Maximum chunks to return per base.
    pub top_k: usize,

    /// Include stack summary in results.
    pub include_stack: bool,

    /// Minimum relevance score to include chunks (0.0-1.0).
    pub min_score: Option<f32>,

    /// Override the reranker's finalK config for final chunk count.
    /// When set, this takes precedence over the config file's `retrieval.reranker.finalK`.
    /// This allows `--top-k` to control the final output count.
    pub final_k: Option<usize>,
}

impl Default for AskOptions {
    fn default() -> Self {
        Self {
            question: String::new(),
            bases: None,
            top_k: DEFAULT_TOP_K,
            include_stack: true,
            min_score: None,
            final_k: None,
        }
    }
}

impl AskOptions {
    /// Create new ask options with a question.
    pub fn new(question: impl Into<String>) -> Self {
        Self {
            question: question.into(),
            ..Default::default()
        }
    }

    /// Set the bases to query.
    pub fn with_bases(mut self, bases: Vec<String>) -> Self {
        self.bases = Some(bases);
        self
    }

    /// Set the top_k limit.
    pub fn with_top_k(mut self, top_k: usize) -> Self {
        self.top_k = top_k;
        self
    }

    /// Set whether to include stack summary.
    pub fn with_stack(mut self, include: bool) -> Self {
        self.include_stack = include;
        self
    }

    /// Set minimum relevance score filter (0.0-1.0).
    pub fn with_min_score(mut self, min_score: f32) -> Self {
        self.min_score = Some(min_score);
        self
    }

    /// Set the final number of chunks to return after reranking.
    /// This overrides the `retrieval.reranker.finalK` config value.
    pub fn with_final_k(mut self, final_k: usize) -> Self {
        self.final_k = Some(final_k);
        self
    }
}

// ============================================================================
// Ask Pipeline
// ============================================================================

/// Run the ask pipeline to retrieve context for a query.
///
/// # Arguments
///
/// * `workspace` - The GIK workspace
/// * `branch` - The branch to query
/// * `opts` - Ask options (question, bases, top_k)
/// * `global_config` - Global configuration for embedding settings
///
/// # Returns
///
/// An `AskContextBundle` with retrieved chunks and metadata.
///
/// # Errors
///
/// - [`GikError::NotInitialized`] if workspace is not initialized
/// - [`GikError::AskNoIndexedBases`] if no indexed bases are found
/// - [`GikError::AskEmbeddingError`] if query embedding fails
/// - [`GikError::InvalidArgument`] if the question is empty
pub fn run_ask(
    workspace: &Workspace,
    branch: &BranchName,
    opts: AskOptions,
    global_config: &GlobalConfig,
    retrieval_config: &crate::config::RetrievalConfig,
) -> Result<AskContextBundle, GikError> {
    let start_time = std::time::Instant::now();

    // 0. Validate question is not empty
    if opts.question.trim().is_empty() {
        return Err(GikError::InvalidArgument(
            "Question cannot be empty".to_string(),
        ));
    }

    // 1. Read HEAD to ensure workspace is initialized
    let head_path = workspace.head_path(branch.as_str());
    let revision_id = read_head(&head_path)?.ok_or(GikError::NotInitialized)?;

    // 2. Determine which bases to query
    let bases_to_query = determine_bases_to_query(workspace, branch, &opts)?;

    if bases_to_query.is_empty() {
        return Err(GikError::AskNoIndexedBases {
            branch: branch.as_str().to_string(),
        });
    }

    // 3. Create embedding backend (use first base's config from global config)
    let first_base = &bases_to_query[0];
    let embedding_config = global_config.resolve_embedding_config(first_base);

    // Try to create a real backend, fall back to mock in tests
    let backend = create_ask_backend(&embedding_config, global_config.device)?;
    let embed_start = std::time::Instant::now();

    // 4. Expand query and embed with multi-query strategy
    //    This improves recall for abstract/conceptual queries
    let expander = QueryExpander::with_defaults();
    let query_variants = expander.expand(&opts.question);

    tracing::debug!(
        "Query expansion: {} -> {} variants",
        opts.question,
        query_variants.len()
    );

    // Embed all query variants
    let variant_embeddings =
        backend
            .embed_batch(&query_variants)
            .map_err(|e| GikError::AskEmbeddingError {
                question: opts.question.clone(),
                reason: e.to_string(),
            })?;

    // Average the embeddings for multi-query strategy
    let query_embedding = if variant_embeddings.len() > 1 {
        average_embeddings(&variant_embeddings).ok_or_else(|| GikError::AskEmbeddingError {
            question: opts.question.clone(),
            reason: "Failed to average query embeddings".to_string(),
        })?
    } else {
        variant_embeddings
            .into_iter()
            .next()
            .ok_or_else(|| GikError::AskEmbeddingError {
                question: opts.question.clone(),
                reason: "No embedding returned for query".to_string(),
            })?
    };

    let embed_time_ms = embed_start.elapsed().as_millis() as u64;
    let search_start = std::time::Instant::now();

    // 4b. Detect filename in query early (Phase 8.6: filename pre-filter)
    //     This is used for both pre-filtering search results and boosting scores
    let filename_match = detect_filename_in_query(&opts.question);
    if let Some(ref fm) = filename_match {
        tracing::debug!(
            "Detected filename '{}' in query (explicit: {})",
            fm.detected_pattern,
            fm.is_explicit
        );
    }

    // 5. Search each base and collect results
    // Memory base is handled separately to populate memory_events instead of rag_chunks
    let mut all_chunks: Vec<RagChunk> = Vec::new();
    let mut memory_events: Vec<MemoryEvent> = Vec::new();
    let mut per_base_counts: Vec<AskBaseCount> = Vec::new();
    let mut used_bases: Vec<String> = Vec::new();

    for base_name in &bases_to_query {
        if base_name == crate::memory::MEMORY_BASE_NAME {
            // Search memory base and populate memory_events
            match search_memory_base(
                workspace,
                branch,
                &query_embedding,
                opts.top_k,
                global_config,
            ) {
                Ok(events) => {
                    per_base_counts.push(AskBaseCount {
                        base: base_name.clone(),
                        count: events.len(),
                    });
                    used_bases.push(base_name.clone());
                    memory_events.extend(events);
                }
                Err(e) => {
                    tracing::warn!("Failed to search memory base: {}", e);
                    // Continue with other bases
                }
            }
        } else {
            // Search non-memory bases and populate rag_chunks
            match search_base(
                workspace,
                branch,
                base_name,
                &query_embedding,
                opts.top_k,
                global_config,
                retrieval_config,
                filename_match.as_ref(),
            ) {
                Ok(chunks) => {
                    per_base_counts.push(AskBaseCount {
                        base: base_name.clone(),
                        count: chunks.len(),
                    });
                    used_bases.push(base_name.clone());
                    all_chunks.extend(chunks);
                }
                Err(e) => {
                    tracing::warn!("Failed to search base '{}': {}", base_name, e);
                    // Continue with other bases
                }
            }
        }
    }

    let search_time_ms = search_start.elapsed().as_millis() as u64;

    // 6. Sort all chunks by score (highest first)
    all_chunks.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    // Sort memory events by score (highest first)
    memory_events.sort_by(|a, b| {
        let score_a = a.score.unwrap_or(0.0);
        let score_b = b.score.unwrap_or(0.0);
        score_b
            .partial_cmp(&score_a)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    // 6b. Apply reranker if enabled (Phase 8.2)
    // Use opts.final_k to override config's finalK if specified (e.g., from --top-k CLI flag)
    let (reranker_used, rerank_time_ms, filename_detected) =
        apply_reranker(&mut all_chunks, &opts.question, global_config, retrieval_config, opts.final_k);

    // 6c. Filter by min_score if set
    if let Some(min_score) = opts.min_score {
        let before_count = all_chunks.len();
        all_chunks.retain(|chunk| chunk.score >= min_score);
        if all_chunks.len() < before_count {
            tracing::debug!(
                "Filtered {} chunks below min_score {} (kept {})",
                before_count - all_chunks.len(),
                min_score,
                all_chunks.len()
            );
        }
    }

    // 6d. Deduplicate overlapping chunks (keep highest-scored)
    all_chunks = deduplicate_overlapping_chunks(all_chunks);

    // 7. Build stack summary if requested (Phase 8.4: scoped to RAG context)
    let stack_summary = if opts.include_stack {
        // Try to load full stats for managers/frameworks
        let full_stats = build_stack_summary(workspace, branch).ok();
        
        // If we have RAG chunks, build a scoped summary
        if !all_chunks.is_empty() {
            let stats_ref = full_stats.as_ref().and_then(|_| {
                // We need StackStats, not StackSummary - try to load it
                let stats_path = workspace.stack_stats_path(branch.as_str());
                read_stats_json(&stats_path).ok().flatten()
            });
            Some(StackSummary::from_rag_context(&all_chunks, stats_ref.as_ref()))
        } else {
            full_stats
        }
    } else {
        None
    };

    // 8. Build KG context (Phase 9.3)
    let kg_results =
        build_kg_context_for_ask(workspace, branch.as_str(), &all_chunks, &opts.question);

    // 9. Build debug info
    let debug = AskDebugInfo {
        embedding_model_id: backend.model_id().to_string(),
        used_bases,
        per_base_counts,
        embed_time_ms: Some(embed_time_ms),
        search_time_ms: Some(search_time_ms),
        reranker_used,
        rerank_time_ms,
        hybrid_search_used: retrieval_config.hybrid.enabled,
        dense_result_count: None,  // TODO: aggregate from search results
        sparse_result_count: None, // TODO: aggregate from search results
        filename_detected,
    };

    let total_time_ms = start_time.elapsed().as_millis();
    tracing::info!(
        "Ask completed in {}ms: {} rag_chunks + {} memory_events + {} kg_subgraphs from {} bases",
        total_time_ms,
        all_chunks.len(),
        memory_events.len(),
        kg_results.len(),
        bases_to_query.len()
    );

    Ok(AskContextBundle {
        revision_id,
        question: opts.question,
        bases: bases_to_query,
        rag_chunks: all_chunks,
        kg_results,
        memory_events,
        stack_summary,
        debug,
    })
}

// ============================================================================
// Helper Functions
// ============================================================================

/// Deduplicate chunks that overlap in the same file.
///
/// Two chunks overlap if they share the same file path and their line ranges intersect.
/// When overlaps are found, keeps the highest-scored chunk and removes others.
///
/// This prevents returning multiple chunks that essentially cover the same content,
/// which can happen when different query variants or search strategies find
/// similar regions of a file.
fn deduplicate_overlapping_chunks(mut chunks: Vec<RagChunk>) -> Vec<RagChunk> {
    if chunks.len() <= 1 {
        return chunks;
    }

    // Already sorted by score (highest first), so we keep the first chunk in any overlap
    let mut kept: Vec<RagChunk> = Vec::with_capacity(chunks.len());
    let original_count = chunks.len();

    for chunk in chunks.drain(..) {
        // Check if this chunk overlaps with any already-kept chunk
        let overlaps = kept.iter().any(|existing| {
            existing.path == chunk.path && ranges_overlap(
                existing.start_line,
                existing.end_line,
                chunk.start_line,
                chunk.end_line,
            )
        });

        if !overlaps {
            kept.push(chunk);
        }
    }

    if kept.len() < original_count {
        tracing::debug!(
            "Deduplicated {} overlapping chunks (kept {})",
            original_count - kept.len(),
            kept.len()
        );
    }

    kept
}

/// Check if two line ranges overlap.
///
/// Two ranges [a_start, a_end] and [b_start, b_end] overlap if:
/// - a_start <= b_end AND b_start <= a_end
#[inline]
fn ranges_overlap(a_start: u32, a_end: u32, b_start: u32, b_end: u32) -> bool {
    a_start <= b_end && b_start <= a_end
}

/// Determine which bases to query based on options and available indexed bases.
fn determine_bases_to_query(
    workspace: &Workspace,
    branch: &BranchName,
    opts: &AskOptions,
) -> Result<Vec<String>, GikError> {
    let branch_dir = workspace.branch_dir(branch.as_str());
    let bases_dir = branch_dir.join("bases");

    // If specific bases requested, validate they exist
    if let Some(requested_bases) = &opts.bases {
        let mut valid_bases = Vec::new();
        for base in requested_bases {
            let base_dir = bases_dir.join(base);
            if base_dir.exists() && has_index(&base_dir) {
                valid_bases.push(base.clone());
            } else {
                tracing::warn!("Requested base '{}' not found or not indexed", base);
            }
        }
        return Ok(valid_bases);
    }

    // Auto-detect: check well-known RAG bases
    let mut available_bases = Vec::new();
    for base in RAG_BASES {
        let base_dir = bases_dir.join(base);
        if base_dir.exists() && has_index(&base_dir) {
            available_bases.push((*base).to_string());
        }
    }

    Ok(available_bases)
}

/// Check if a base directory has an index.
fn has_index(base_dir: &Path) -> bool {
    let index_dir = base_dir.join("index");
    let meta_path = index_dir.join(crate::vector_index::INDEX_META_FILENAME);
    meta_path.exists()
}

/// Create an embedding backend for ask queries.
///
/// Uses the real Candle backend if available, otherwise returns an error.
fn create_ask_backend(
    config: &crate::embedding::EmbeddingConfig,
    device_pref: DevicePreference,
) -> Result<Box<dyn EmbeddingBackend>, GikError> {
    create_backend(config, device_pref)
}

/// Search a single base for relevant chunks using hybrid search (dense + BM25).
///
/// When hybrid search is enabled:
/// 1. Run dense (embedding) search to get top candidates
/// 2. Run BM25 (sparse) search on the same query text
/// 3. Combine results using Reciprocal Rank Fusion (RRF)
/// 4. Return fused results
///
/// If BM25 index is not available or hybrid is disabled, falls back to dense-only.
///
/// Phase 8.6: When a `filename_match` is provided, chunks from matching files
/// are guaranteed to be included in results even if they weren't in the top-k
/// from dense search. This ensures filename-specific queries find the right files.
fn search_base(
    workspace: &Workspace,
    branch: &BranchName,
    base_name: &str,
    query_embedding: &[f32],
    top_k: usize,
    global_config: &GlobalConfig,
    retrieval_config: &crate::config::RetrievalConfig,
    filename_match: Option<&FilenameMatch>,
) -> Result<Vec<RagChunk>, GikError> {
    let branch_dir = workspace.branch_dir(branch.as_str());
    let base_dir = branch_dir.join("bases").join(base_name);
    let index_dir = base_dir.join("index");

    // Get embedding config for this base from global config
    let embedding_config = global_config.resolve_embedding_config(base_name);
    let dimension = embedding_config.dimension.unwrap_or(384);

    // Create default vector config with resolved embedding dimension
    let mut vector_config = VectorIndexConfig::new(
        VectorIndexBackendKind::SimpleFile,
        VectorMetric::Cosine,
        dimension,
        base_name,
    );

    // Check existing index metadata to determine actual backend
    let meta_path = index_dir.join("meta.json");
    if let Ok(Some(meta)) = load_index_meta(&meta_path) {
        // Use the backend from existing index metadata
        if meta.backend == "simple_file" {
            vector_config.backend = VectorIndexBackendKind::SimpleFile;
        } else if meta.backend == "lancedb" {
            vector_config.backend = VectorIndexBackendKind::LanceDb;
        }
    }

    // Load the vector index using the unified factory
    let index: Box<dyn VectorIndexBackend> =
        open_vector_index(index_dir, vector_config, &embedding_config)?;

    // Load base sources to get chunk details (needed for both dense and hybrid)
    let sources_path = base_dir.join(crate::base::SOURCES_FILENAME);
    let source_entries = load_base_sources(&sources_path)?;

    // Build maps from vector_id and chunk_id to source entry
    let source_map_by_vector: std::collections::HashMap<u64, &BaseSourceEntry> =
        source_entries.iter().map(|e| (e.vector_id, e)).collect();
    let source_map_by_chunk: std::collections::HashMap<&str, &BaseSourceEntry> =
        source_entries.iter().map(|e| (e.id.as_str(), e)).collect();

    // Check if hybrid search is enabled (uses resolved config with project overrides)
    let hybrid_config = &retrieval_config.hybrid;

    let mut chunks: Vec<RagChunk> = if hybrid_config.enabled {
        // Try to load BM25 index for hybrid search
        let bm25_base_dir = base_root(workspace.knowledge_root(), branch.as_str(), base_name);
        if let Ok(Some(bm25_index)) = load_bm25_index(&bm25_base_dir) {
            search_base_hybrid(
                &*index,
                &bm25_index,
                query_embedding,
                top_k,
                hybrid_config,
                &source_map_by_vector,
                &source_map_by_chunk,
            )?
        } else {
            tracing::debug!(
                "BM25 index not found for base '{}', falling back to dense-only",
                base_name
            );
            // Fall through to dense-only
            let search_results = index.query(query_embedding, top_k as u32)?;
            search_results
                .iter()
                .filter_map(|result| {
                    source_map_by_vector
                        .get(&result.id.0)
                        .map(|entry| RagChunk::from_source_and_result(entry, result))
                })
                .collect()
        }
    } else {
        // Dense-only search
        let search_results = index.query(query_embedding, top_k as u32)?;
        search_results
            .iter()
            .filter_map(|result| {
                source_map_by_vector
                    .get(&result.id.0)
                    .map(|entry| RagChunk::from_source_and_result(entry, result))
            })
            .collect()
    };

    // Phase 8.6: Filename pre-filter - include matching files even if not in top-k
    if let Some(fm) = filename_match {
        // Collect paths already in results
        let existing_paths: std::collections::HashSet<&str> =
            chunks.iter().map(|c| c.path.as_str()).collect();

        // Find source entries matching the filename that aren't already included
        let mut additional_chunks: Vec<RagChunk> = Vec::new();
        for entry in &source_entries {
            if !existing_paths.contains(entry.file_path.as_str())
                && fm.matches_path(&entry.file_path)
            {
                // Create a chunk with a high synthetic score (will be boosted by reranker)
                let chunk = RagChunk {
                    base: entry.base.clone(),
                    path: entry.file_path.clone(),
                    snippet: entry.text.clone().unwrap_or_default(),
                    start_line: entry.start_line,
                    end_line: entry.end_line,
                    // Use a score that places it mid-range so reranker can adjust
                    score: 0.5,
                    dense_score: Some(0.5),
                    reranker_score: None,
                };
                tracing::debug!(
                    "Pre-filter: adding '{}' matching filename pattern '{}'",
                    entry.file_path,
                    fm.detected_pattern
                );
                additional_chunks.push(chunk);
            }
        }

        if !additional_chunks.is_empty() {
            tracing::info!(
                "Filename pre-filter added {} chunks matching '{}'",
                additional_chunks.len(),
                fm.detected_pattern
            );
            chunks.extend(additional_chunks);
        }
    }

    Ok(chunks)
}

/// Perform hybrid search combining dense (embedding) and sparse (BM25) retrieval.
///
/// Uses Reciprocal Rank Fusion (RRF) to combine results from both retrievers.
fn search_base_hybrid(
    dense_index: &dyn VectorIndexBackend,
    _bm25_index: &Bm25Index,
    query_embedding: &[f32],
    top_k: usize,
    config: &HybridSearchConfig,
    source_map_by_vector: &std::collections::HashMap<u64, &BaseSourceEntry>,
    _source_map_by_chunk: &std::collections::HashMap<&str, &BaseSourceEntry>,
) -> Result<Vec<RagChunk>, GikError> {
    // 1. Dense search
    let dense_top_k = config.dense_top_k.max(top_k);
    let dense_results = dense_index.query(query_embedding, dense_top_k as u32)?;

    // Convert dense results to (chunk_id, score) pairs
    let dense_pairs: Vec<(String, f32)> = dense_results
        .iter()
        .filter_map(|result| {
            source_map_by_vector
                .get(&result.id.0)
                .map(|entry| (entry.id.as_str().to_string(), result.score))
        })
        .collect();

    // 2. Sparse (BM25) search
    // We need the query text, but we only have the embedding here.
    // For now, we'll search using the text from the original question.
    // This requires passing the query text through the call chain.
    // As a workaround, we can skip BM25 if we don't have text, or use a placeholder.
    //
    // For the initial implementation, we'll search BM25 with empty query if no text available.
    // The caller should pass the query text for proper BM25 search.
    //
    // TODO: Pass query text through the search chain for proper BM25 search.
    // For now, we just use dense results if no query text is available.

    if dense_pairs.is_empty() {
        return Ok(Vec::new());
    }

    // Construct RagChunks from dense results (no BM25 fusion yet since we don't have query text)
    // This will be improved when we thread the query text through.
    let chunks: Vec<RagChunk> = dense_results
        .iter()
        .take(top_k)
        .filter_map(|result| {
            source_map_by_vector.get(&result.id.0).map(|entry| {
                let mut chunk = RagChunk::from_source_and_result(entry, result);
                // Mark as having gone through hybrid pipeline
                chunk.dense_score = Some(result.score);
                chunk
            })
        })
        .collect();

    tracing::debug!(
        "Hybrid search: {} dense results (BM25 fusion pending query text threading)",
        dense_results.len()
    );

    Ok(chunks)
}

/// Search a single base with hybrid search, using query text for BM25.
///
/// This is the full hybrid search implementation that uses both dense embeddings
/// and BM25 text search, combining results with RRF.
///
/// # Internal
///
/// **Phase 8.2 Infrastructure** — This function is internal infrastructure for
/// the upcoming hybrid search feature (Phase 8.2). It is not wired into the
/// public API yet; the current `search_base_hybrid` uses a simplified path.
/// Retain for future use.
#[allow(dead_code)]
fn search_base_hybrid_with_query(
    workspace: &Workspace,
    branch: &BranchName,
    base_name: &str,
    query_embedding: &[f32],
    query_text: &str,
    top_k: usize,
    global_config: &GlobalConfig,
    retrieval_config: &crate::config::RetrievalConfig,
) -> Result<Vec<RagChunk>, GikError> {
    let branch_dir = workspace.branch_dir(branch.as_str());
    let base_dir = branch_dir.join("bases").join(base_name);
    let index_dir = base_dir.join("index");

    // Get embedding config for this base from global config
    let embedding_config = global_config.resolve_embedding_config(base_name);
    let dimension = embedding_config.dimension.unwrap_or(384);

    // Create default vector config
    let mut vector_config = VectorIndexConfig::new(
        VectorIndexBackendKind::SimpleFile,
        VectorMetric::Cosine,
        dimension,
        base_name,
    );

    // Check existing index metadata
    let meta_path = index_dir.join("meta.json");
    if let Ok(Some(meta)) = load_index_meta(&meta_path) {
        if meta.backend == "simple_file" {
            vector_config.backend = VectorIndexBackendKind::SimpleFile;
        } else if meta.backend == "lancedb" {
            vector_config.backend = VectorIndexBackendKind::LanceDb;
        }
    }

    // Load the vector index using the unified factory
    let index: Box<dyn VectorIndexBackend> =
        open_vector_index(index_dir, vector_config, &embedding_config)?;

    // Load base sources
    let sources_path = base_dir.join(crate::base::SOURCES_FILENAME);
    let source_entries = load_base_sources(&sources_path)?;

    let source_map_by_vector: std::collections::HashMap<u64, &BaseSourceEntry> =
        source_entries.iter().map(|e| (e.vector_id, e)).collect();
    let source_map_by_chunk: std::collections::HashMap<&str, &BaseSourceEntry> =
        source_entries.iter().map(|e| (e.id.as_str(), e)).collect();

    let hybrid_config = &retrieval_config.hybrid;

    // Load BM25 index
    let bm25_base_dir = base_root(workspace.knowledge_root(), branch.as_str(), base_name);
    let bm25_index = match load_bm25_index(&bm25_base_dir)? {
        Some(idx) => idx,
        None => {
            // Fall back to dense-only
            let search_results = index.query(query_embedding, top_k as u32)?;
            let chunks: Vec<RagChunk> = search_results
                .iter()
                .filter_map(|result| {
                    source_map_by_vector
                        .get(&result.id.0)
                        .map(|entry| RagChunk::from_source_and_result(entry, result))
                })
                .collect();
            return Ok(chunks);
        }
    };

    // 1. Dense search
    let dense_top_k = hybrid_config.dense_top_k.max(top_k);
    let dense_results = index.query(query_embedding, dense_top_k as u32)?;

    let dense_pairs: Vec<(String, f32)> = dense_results
        .iter()
        .filter_map(|result| {
            source_map_by_vector
                .get(&result.id.0)
                .map(|entry| (entry.id.as_str().to_string(), result.score))
        })
        .collect();

    // 2. BM25 search
    let sparse_top_k = hybrid_config.sparse_top_k.max(top_k);
    let bm25_results = bm25_index.search(query_text, sparse_top_k);

    tracing::debug!(
        "Hybrid search for '{}': {} dense, {} sparse results",
        query_text,
        dense_pairs.len(),
        bm25_results.len()
    );

    // 3. RRF Fusion (with runtime validation for config values)
    let fused = rrf_fusion(&dense_pairs, &bm25_results, hybrid_config)?;

    // 4. Convert fused results to RagChunks
    let chunks: Vec<RagChunk> = fused
        .into_iter()
        .take(top_k)
        .filter_map(|fused_result| {
            source_map_by_chunk
                .get(fused_result.doc_id.as_str())
                .map(|entry| {
                    RagChunk {
                        base: entry.base.clone(),
                        score: fused_result.rrf_score,
                        path: entry.file_path.clone(),
                        start_line: entry.start_line,
                        end_line: entry.end_line,
                        snippet: entry.text.clone().unwrap_or_default(),
                        dense_score: fused_result.dense_rank.map(|r| {
                            // Approximate dense score from rank
                            1.0 / (r as f32 + 1.0)
                        }),
                        reranker_score: None,
                    }
                })
        })
        .collect();

    Ok(chunks)
}

/// Search the memory base for relevant memory events.
///
/// This is a specialized version of `search_base` that returns `MemoryEvent`
/// instead of `RagChunk`, extracting memory-specific metadata from the source entries.
fn search_memory_base(
    workspace: &Workspace,
    branch: &BranchName,
    query_embedding: &[f32],
    top_k: usize,
    global_config: &GlobalConfig,
) -> Result<Vec<MemoryEvent>, GikError> {
    let branch_dir = workspace.branch_dir(branch.as_str());
    let base_dir = branch_dir
        .join("bases")
        .join(crate::memory::MEMORY_BASE_NAME);
    let index_dir = base_dir.join("index");

    // Get embedding config for memory base from global config
    let embedding_config = global_config.resolve_embedding_config(crate::memory::MEMORY_BASE_NAME);
    let dimension = embedding_config.dimension.unwrap_or(384);

    // Create default vector config with resolved embedding dimension
    let mut vector_config = VectorIndexConfig::new(
        VectorIndexBackendKind::SimpleFile,
        VectorMetric::Cosine,
        dimension,
        crate::memory::MEMORY_BASE_NAME,
    );

    // Check existing index metadata to determine actual backend
    let meta_path = index_dir.join("meta.json");
    if let Ok(Some(meta)) = load_index_meta(&meta_path) {
        // Use the backend from existing index metadata
        if meta.backend == "simple_file" {
            vector_config.backend = VectorIndexBackendKind::SimpleFile;
        } else if meta.backend == "lancedb" {
            vector_config.backend = VectorIndexBackendKind::LanceDb;
        }
    }

    // Load the vector index using the unified factory
    let index: Box<dyn VectorIndexBackend> =
        open_vector_index(index_dir, vector_config, &embedding_config)?;

    // Query the index
    let search_results = index.query(query_embedding, top_k as u32)?;

    if search_results.is_empty() {
        return Ok(Vec::new());
    }

    // Load base sources to get entry details
    let sources_path = base_dir.join(crate::base::SOURCES_FILENAME);
    let source_entries = load_base_sources(&sources_path)?;

    // Build a map from vector_id to source entry
    let source_map: std::collections::HashMap<u64, &BaseSourceEntry> =
        source_entries.iter().map(|e| (e.vector_id, e)).collect();

    // Convert search results to MemoryEvents
    let events: Vec<MemoryEvent> = search_results
        .iter()
        .filter_map(|result| {
            source_map
                .get(&result.id.0)
                .and_then(|entry| MemoryEvent::from_base_source_entry(entry, result.score))
        })
        .collect();

    Ok(events)
}

// ============================================================================
// P0/P1 Reranker Enhancement Helpers
// ============================================================================

/// Detect file type from path for reranker metadata injection (P0.1).
fn detect_file_type(path: &str) -> &'static str {
    let path_lower = path.to_lowercase();
    let filename = path_lower.rsplit('/').next().unwrap_or(&path_lower);

    // Config files
    if filename.contains("config")
        || filename.ends_with(".config.js")
        || filename.ends_with(".config.ts")
        || filename == "vite.config.js"
        || filename == "webpack.config.js"
        || filename == "package.json"
        || filename == "tsconfig.json"
    {
        return "config";
    }

    // Stylesheets
    if filename.ends_with(".css")
        || filename.ends_with(".scss")
        || filename.ends_with(".sass")
        || filename.ends_with(".less")
    {
        return "stylesheet";
    }

    // Documentation
    if filename.ends_with(".md")
        || filename == "readme"
        || filename.contains("readme")
        || filename == "changelog"
    {
        return "documentation";
    }

    // React components
    if filename.ends_with(".jsx") || filename.ends_with(".tsx") {
        return "component";
    }

    // Hooks
    if filename.starts_with("use") && (filename.ends_with(".js") || filename.ends_with(".ts")) {
        return "hook";
    }

    // Utilities/helpers
    if filename.contains("helper")
        || filename.contains("util")
        || filename.contains("utils")
        || filename.contains("lib")
    {
        return "utility";
    }

    // API/services
    if filename.contains("api") || filename.contains("service") || filename.contains("client") {
        return "api";
    }

    // Default based on extension
    if filename.ends_with(".js") || filename.ends_with(".ts") {
        return "script";
    }
    if filename.ends_with(".html") {
        return "markup";
    }
    if filename.ends_with(".json") {
        return "data";
    }

    "code"
}

/// Build reranker input with file metadata prepended (P0.1).
/// Format: "[File: path] [Type: type] [Base: base]\n<snippet>"
fn build_reranker_input(chunk: &RagChunk) -> String {
    let file_type = detect_file_type(&chunk.path);
    format!(
        "[File: {}] [Type: {}] [Base: {}]\n{}",
        chunk.path, file_type, chunk.base, chunk.snippet
    )
}

/// Apply filename-based score boost for queries matching file patterns.
///
/// This function provides two levels of boosting:
/// 1. Strong boost (+0.50) for exact filename matches detected by `detect_filename_in_query`
/// 2. Weak boost (+0.05-0.10) for semantic pattern matching (config, helper, etc.)
///
/// The strong boost ensures that when a user asks about a specific file,
/// that file appears at the top of results.
fn compute_filename_boost(query: &str, path: &str, filename_match: Option<&FilenameMatch>) -> f32 {
    // Strong boost for explicit filename match
    if let Some(fm) = filename_match {
        if fm.matches_path(path) {
            return if fm.is_explicit {
                FILENAME_EXACT_BOOST // +0.50
            } else {
                FILENAME_PARTIAL_BOOST // +0.25
            };
        }
    }

    // Fallback: legacy semantic pattern matching
    let query_lower = query.to_lowercase();
    let path_lower = path.to_lowercase();
    let filename = path_lower.rsplit('/').next().unwrap_or(&path_lower);
    let name_without_ext = filename
        .rsplit('.')
        .next_back()
        .unwrap_or(filename)
        .replace(".jsx", "")
        .replace(".tsx", "")
        .replace(".js", "")
        .replace(".ts", "");

    // CSS/styles queries
    if (query_lower.contains("css") || query_lower.contains("style"))
        && (filename.ends_with(".css") || filename.ends_with(".scss") || filename.contains("style"))
    {
        return 0.10;
    }

    // Config/bundler queries
    if (query_lower.contains("config")
        || query_lower.contains("bundler")
        || query_lower.contains("vite")
        || query_lower.contains("webpack"))
        && (filename.contains("config")
            || filename.contains("vite")
            || filename.contains("webpack"))
    {
        return 0.10;
    }

    // Helper/utility queries
    if (query_lower.contains("helper") || query_lower.contains("util"))
        && (filename.contains("helper") || filename.contains("util"))
    {
        return 0.10;
    }

    // Hook queries
    if query_lower.contains("hook") && filename.starts_with("use") {
        return 0.10;
    }

    // API queries
    if query_lower.contains("api") && filename.contains("api") {
        return 0.10;
    }

    // Generic term matching in filename
    let query_terms: Vec<&str> = query_lower.split_whitespace().collect();
    for term in &query_terms {
        // Skip common stop words
        if [
            "the", "is", "a", "an", "where", "what", "how", "which", "are", "in", "for", "to",
        ]
        .contains(term)
        {
            continue;
        }
        if term.len() >= 3 && (name_without_ext.contains(term) || filename.contains(term)) {
            return 0.05;
        }
    }

    0.0
}

/// Compute hybrid score combining dense and reranker scores (P1).
/// Uses weighted combination: alpha * reranker + beta * dense
fn compute_hybrid_score(dense_score: f32, reranker_score: f32) -> f32 {
    // Weights: favor dense score since reranker is trained on natural language,
    // not code retrieval. These can be tuned based on evaluation.
    const ALPHA: f32 = 0.4; // Reranker weight
    const BETA: f32 = 0.6; // Dense weight

    // Normalize dense score (typically 0.0 to 0.5 range -> 0.0 to 1.0)
    let norm_dense = (dense_score * 2.0).clamp(0.0, 1.0);

    // Normalize reranker score (typically -0.1 to +0.1 -> 0.0 to 1.0)
    // Shift by 0.1 and scale by 5 to map [-0.1, 0.1] to [0.0, 1.0]
    let norm_reranker = ((reranker_score + 0.1) * 5.0).clamp(0.0, 1.0);

    ALPHA * norm_reranker + BETA * norm_dense
}

// ============================================================================

/// Apply cross-encoder reranking to chunks.
///
/// # Arguments
///
/// * `chunks` - Mutable reference to chunks to rerank (will be replaced with reranked results)
/// * `question` - The query string
/// * `global_config` - Global configuration (for device preference)
/// * `retrieval_config` - Resolved retrieval configuration (with project overrides applied)
/// * `final_k_override` - Optional override for the final number of chunks to return.
///   When `Some(n)`, this takes precedence over `retrieval.reranker.finalK` from config.
///   This allows CLI flags like `--top-k` to control the final output count.
///
/// # Returns
///
/// A tuple of (reranker_used: bool, rerank_time_ms: Option<u64>, filename_detected: Option<String>)
fn apply_reranker(
    chunks: &mut Vec<RagChunk>,
    question: &str,
    global_config: &GlobalConfig,
    retrieval_config: &crate::config::RetrievalConfig,
    final_k_override: Option<usize>,
) -> (bool, Option<u64>, Option<String>) {
    // Check if reranker is configured and available
    let reranker_config = &retrieval_config.reranker;

    if !reranker_config.enabled {
        tracing::debug!("Reranker disabled in configuration");
        return (false, None, None);
    }

    if chunks.is_empty() {
        return (false, None, None);
    }

    // Get or initialize the reranker backend (lazy singleton)
    let reranker = match get_or_init_reranker_backend(reranker_config, global_config.device) {
        Some(backend) => backend,
        None => {
            tracing::debug!("Reranker backend not available, using dense-only ordering");
            return (false, None, None);
        }
    };

    let rerank_start = std::time::Instant::now();

    // Limit to top_k candidates for reranking
    let top_k = reranker_config.top_k.min(chunks.len());

    // P0.1: Build reranker input with file metadata prepended
    let candidates: Vec<String> = chunks
        .iter()
        .take(top_k)
        .map(build_reranker_input)
        .collect();

    tracing::debug!(
        "Reranking {} candidates (out of {}) for query: '{}'",
        candidates.len(),
        chunks.len(),
        question
    );

    // P0.2: Detect filename in query once before scoring loop
    let filename_match = detect_filename_in_query(question);
    if let Some(ref fm) = filename_match {
        tracing::debug!(
            "Detected filename '{}' in query (explicit: {})",
            fm.detected_pattern,
            fm.is_explicit
        );
    }

    // Score candidates
    match reranker.rerank(question, &candidates) {
        Ok(ranked_indices) => {
            // Use override if provided, otherwise use config value
            // This allows --top-k to control final output when specified
            let final_k = final_k_override
                .unwrap_or(reranker_config.final_k)
                .min(ranked_indices.len());

            if final_k_override.is_some() {
                tracing::debug!(
                    "Using final_k override: {} (config was: {})",
                    final_k,
                    reranker_config.final_k
                );
            }

            // Create a new order based on reranker results with hybrid scoring
            let mut reranked_chunks: Vec<RagChunk> = Vec::with_capacity(final_k);
            for (original_idx, reranker_score) in ranked_indices.into_iter().take(final_k) {
                let mut chunk = chunks[original_idx].clone();
                let dense_score = chunk.dense_score.unwrap_or(chunk.score);

                // Store the raw reranker score
                chunk.reranker_score = Some(reranker_score);

                // P1: Compute hybrid score combining dense and reranker
                let mut hybrid = compute_hybrid_score(dense_score, reranker_score);

                // P0.2: Apply filename-based boost
                let filename_boost = compute_filename_boost(question, &chunk.path, filename_match.as_ref());
                if filename_boost > 0.0 {
                    tracing::debug!(
                        "Filename boost +{:.2} for '{}' (query: '{}')",
                        filename_boost,
                        chunk.path,
                        question
                    );
                    hybrid += filename_boost;
                }

                chunk.score = hybrid;
                reranked_chunks.push(chunk);
            }

            // Re-sort by final hybrid score (descending)
            reranked_chunks.sort_by(|a, b| {
                b.score
                    .partial_cmp(&a.score)
                    .unwrap_or(std::cmp::Ordering::Equal)
            });

            // Replace original chunks with reranked subset
            *chunks = reranked_chunks;

            let rerank_time_ms = rerank_start.elapsed().as_millis() as u64;
            tracing::info!(
                "Reranking complete: {} -> {} chunks in {}ms (P0/P1 hybrid scoring)",
                top_k,
                chunks.len(),
                rerank_time_ms
            );

            let detected_filename = filename_match.map(|fm| fm.detected_pattern);
            (true, Some(rerank_time_ms), detected_filename)
        }
        Err(e) => {
            tracing::warn!("Reranking failed (falling back to dense-only): {}", e);
            let detected_filename = filename_match.map(|fm| fm.detected_pattern);
            (false, None, detected_filename)
        }
    }
}

/// Build a stack summary from the workspace's stack inventory.
fn build_stack_summary(
    workspace: &Workspace,
    branch: &BranchName,
) -> Result<StackSummary, GikError> {
    let stats_path = workspace.stack_stats_path(branch.as_str());
    let tech_path = workspace.stack_tech_path(branch.as_str());

    let stats = read_stats_json(&stats_path)?;
    let tech = read_tech_jsonl(&tech_path)?;

    match stats {
        Some(s) => Ok(StackSummary::from_stats_with_tech(&s, &tech)),
        None => Err(GikError::StackScanFailed(
            "No stack stats found".to_string(),
        )),
    }
}

/// Build KG context for the ask pipeline (Phase 9.3).
///
/// Maps RAG chunks to KG nodes and performs bounded graph traversal to
/// produce relevant subgraphs enriched with structural information.
///
/// Returns an empty vector if:
/// - KG doesn't exist for the branch
/// - No RAG chunks to map
/// - An error occurs (best-effort, doesn't fail the ask)
fn build_kg_context_for_ask(
    workspace: &Workspace,
    branch: &str,
    rag_chunks: &[RagChunk],
    question: &str,
) -> Vec<AskKgResult> {
    use crate::kg::query::{build_ask_kg_context, KgQueryConfig, RagChunkRef};

    // Convert RagChunks to RagChunkRefs for KG mapping
    let chunk_refs: Vec<RagChunkRef> = rag_chunks
        .iter()
        .map(|c| RagChunkRef::new(&c.base, &c.path))
        .collect();

    // Use default config with reasonable limits
    let cfg = KgQueryConfig::default();

    // Build KG context (best-effort: log errors but don't fail)
    match build_ask_kg_context(workspace, branch, &chunk_refs, question, &cfg) {
        Ok(results) => {
            if !results.is_empty() {
                tracing::debug!("Built {} KG subgraphs for ask query", results.len());
            }
            results
        }
        Err(e) => {
            tracing::warn!(
                "Failed to build KG context for ask: {}. Continuing without KG.",
                e
            );
            Vec::new()
        }
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::timeline::RevisionId;
    use chrono::Utc;

    #[test]
    fn test_ask_options_default() {
        let opts = AskOptions::default();
        assert!(opts.question.is_empty());
        assert!(opts.bases.is_none());
        assert_eq!(opts.top_k, DEFAULT_TOP_K);
        assert!(opts.include_stack);
        assert!(opts.final_k.is_none());
    }

    #[test]
    fn test_ask_options_builder() {
        let opts = AskOptions::new("How does it work?")
            .with_bases(vec!["code".to_string()])
            .with_top_k(5)
            .with_stack(false);

        assert_eq!(opts.question, "How does it work?");
        assert_eq!(opts.bases, Some(vec!["code".to_string()]));
        assert_eq!(opts.top_k, 5);
        assert!(!opts.include_stack);
        assert!(opts.final_k.is_none());
    }

    #[test]
    fn test_ask_options_with_final_k() {
        let opts = AskOptions::new("Query")
            .with_top_k(20)
            .with_final_k(15);

        assert_eq!(opts.top_k, 20);
        assert_eq!(opts.final_k, Some(15));
    }

    #[test]
    fn test_stack_summary_from_stats() {
        use std::collections::HashMap;

        let mut languages = HashMap::new();
        languages.insert("rust".to_string(), 10);
        languages.insert("typescript".to_string(), 5);

        let stats = StackStats {
            total_files: 15,
            languages,
            managers: vec!["cargo".to_string(), "npm".to_string()],
            generated_at: Utc::now(),
        };

        let summary = StackSummary::from_stats(&stats);
        assert_eq!(summary.total_files, Some(15));
        assert!(summary.languages.contains(&"rust".to_string()));
        assert!(summary.languages.contains(&"typescript".to_string()));
        assert!(summary.managers.contains(&"cargo".to_string()));
    }

    #[test]
    fn test_stack_summary_from_stats_with_tech() {
        use crate::stack::StackTechEntry;
        use std::collections::HashMap;

        let mut languages = HashMap::new();
        languages.insert("rust".to_string(), 10);
        languages.insert("typescript".to_string(), 5);

        let stats = StackStats {
            total_files: 15,
            languages,
            managers: vec!["cargo".to_string(), "npm".to_string()],
            generated_at: Utc::now(),
        };

        let tech = vec![
            StackTechEntry {
                kind: "framework".to_string(),
                name: "Next.js".to_string(),
                source: "dependency:next".to_string(),
                confidence: 0.9,
            },
            StackTechEntry {
                kind: "framework".to_string(),
                name: "React".to_string(),
                source: "dependency:react".to_string(),
                confidence: 0.9,
            },
            StackTechEntry {
                kind: "tool".to_string(),
                name: "Tokio".to_string(),
                source: "dependency:tokio".to_string(),
                confidence: 0.9,
            },
            StackTechEntry {
                kind: "language".to_string(),
                name: "Rust".to_string(),
                source: "files:*.rs".to_string(),
                confidence: 0.9,
            },
        ];

        let summary = StackSummary::from_stats_with_tech(&stats, &tech);
        assert_eq!(summary.total_files, Some(15));
        assert!(summary.languages.contains(&"rust".to_string()));
        assert_eq!(summary.frameworks.len(), 2);
        assert!(summary.frameworks.contains(&"Next.js".to_string()));
        assert!(summary.frameworks.contains(&"React".to_string()));
        // services should be empty (no service entries in tech)
        assert!(summary.services.is_empty());
    }

    #[test]
    fn test_rag_chunk_serialization() {
        let chunk = RagChunk {
            base: "code".to_string(),
            score: 0.92,
            path: "src/main.rs".to_string(),
            start_line: 10,
            end_line: 40,
            snippet: "fn main() {}".to_string(),
            dense_score: Some(0.85),
            reranker_score: Some(0.12),
        };

        let json = serde_json::to_string(&chunk).unwrap();
        assert!(json.contains("\"base\":\"code\""));
        assert!(json.contains("\"score\":0.92"));
        assert!(json.contains("\"startLine\":10"));
        assert!(json.contains("\"denseScore\":0.85"));
        assert!(json.contains("\"rerankerScore\":0.12"));
    }

    #[test]
    fn test_rag_chunk_serialization_without_optional_scores() {
        // Test that optional scores are omitted from JSON when None
        let chunk = RagChunk {
            base: "docs".to_string(),
            score: 0.75,
            path: "README.md".to_string(),
            start_line: 1,
            end_line: 20,
            snippet: "# Title".to_string(),
            dense_score: None,
            reranker_score: None,
        };

        let json = serde_json::to_string(&chunk).unwrap();
        assert!(!json.contains("denseScore"));
        assert!(!json.contains("rerankerScore"));
    }

    #[test]
    fn test_ask_context_bundle_serialization() {
        let bundle = AskContextBundle {
            revision_id: RevisionId::new("rev-123"),
            question: "How does it work?".to_string(),
            bases: vec!["code".to_string()],
            rag_chunks: vec![],
            kg_results: vec![],
            memory_events: vec![],
            stack_summary: None,
            debug: AskDebugInfo {
                embedding_model_id: "test-model".to_string(),
                used_bases: vec!["code".to_string()],
                per_base_counts: vec![],
                embed_time_ms: Some(10),
                search_time_ms: Some(20),
                reranker_used: false,
                rerank_time_ms: None,
                hybrid_search_used: false,
                dense_result_count: None,
                sparse_result_count: None,
                filename_detected: None,
            },
        };

        let json = serde_json::to_string_pretty(&bundle).unwrap();
        assert!(json.contains("\"revisionId\":"));
        assert!(json.contains("\"question\":"));
        assert!(json.contains("\"embeddingModelId\":"));
    }

    #[test]
    fn test_memory_event_from_base_source_entry() {
        use crate::base::{BaseSourceEntry, ChunkId};

        // Create a BaseSourceEntry that simulates a memory entry
        let entry = BaseSourceEntry {
            id: ChunkId::new("chunk-001"),
            base: "memory".to_string(),
            branch: "main".to_string(),
            file_path: "memory:mem-abc123".to_string(),
            start_line: 1,
            end_line: 1,
            text: Some("We chose PostgreSQL for ACID compliance.".to_string()),
            vector_id: 42,
            indexed_at: Utc::now(),
            revision_id: "rev-001".to_string(),
            source_id: "src-001".to_string(),
            indexed_mtime: None,
            indexed_size: None,
            extra: Some(serde_json::json!({
                "memory_id": "mem-abc123",
                "memory_scope": "project",
                "memory_source": "decision",
                "title": "Database Selection",
                "tags": ["architecture", "database"],
                "created_at": "2025-01-15T10:30:00Z"
            })),
        };

        let event = MemoryEvent::from_base_source_entry(&entry, 0.95);
        assert!(event.is_some());

        let event = event.unwrap();
        assert_eq!(event.id, "mem-abc123");
        assert_eq!(event.scope, crate::memory::MemoryScope::Project);
        assert_eq!(event.source, crate::memory::MemorySource::Decision);
        assert_eq!(event.title, "Database Selection");
        assert_eq!(event.text, "We chose PostgreSQL for ACID compliance.");
        assert_eq!(event.tags, vec!["architecture", "database"]);
        assert_eq!(event.score, Some(0.95));
    }

    #[test]
    fn test_memory_event_from_base_source_entry_missing_extra() {
        use crate::base::{BaseSourceEntry, ChunkId};

        // Entry without extra metadata should return None
        let entry = BaseSourceEntry {
            id: ChunkId::new("chunk-002"),
            base: "memory".to_string(),
            branch: "main".to_string(),
            file_path: "memory:mem-xyz".to_string(),
            start_line: 1,
            end_line: 1,
            text: Some("Some text".to_string()),
            vector_id: 43,
            indexed_at: Utc::now(),
            revision_id: "rev-002".to_string(),
            source_id: "src-002".to_string(),
            indexed_mtime: None,
            indexed_size: None,
            extra: None,
        };

        let event = MemoryEvent::from_base_source_entry(&entry, 0.80);
        assert!(event.is_none());
    }

    #[test]
    fn test_memory_event_serialization() {
        let event = MemoryEvent {
            id: "mem-test-001".to_string(),
            scope: crate::memory::MemoryScope::Project,
            title: "Test Decision".to_string(),
            text: "We decided to use Rust.".to_string(),
            created_at: chrono::DateTime::parse_from_rfc3339("2025-01-15T10:30:00Z")
                .unwrap()
                .with_timezone(&Utc),
            source: crate::memory::MemorySource::Decision,
            tags: vec!["language".to_string()],
            score: Some(0.88),
        };

        let json = serde_json::to_string(&event).unwrap();
        assert!(json.contains("\"id\":\"mem-test-001\""));
        assert!(json.contains("\"scope\":\"project\""));
        assert!(json.contains("\"source\":\"decision\""));
        assert!(json.contains("\"score\":0.88"));
    }

    #[test]
    fn test_rag_bases_includes_memory() {
        assert!(RAG_BASES.contains(&"memory"));
        assert!(RAG_BASES.contains(&"code"));
        assert!(RAG_BASES.contains(&"docs"));
    }
}
