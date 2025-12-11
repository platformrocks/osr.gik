//! Knowledge Graph extraction from existing bases.
//!
//! This module provides the extraction logic for deriving KG nodes and edges
//! from existing knowledge bases (`code`, `docs`, etc.).
//!
//! ## Phase 9.2 Scope
//!
//! - Nodes of kind `"file"` for code sources
//! - Nodes of kind `"doc"` for docs sources
//! - Edges of kind `"imports"` representing file→file import relationships
//!
//! ## Phase 9.2.1 Scope (Hardening & Multi-Language Symbols)
//!
//! - **Extension trimming fix**: `.tsx`/`.jsx` stripped before `.ts`/`.js`
//! - **Path normalization**: Windows backslashes converted to forward slashes
//! - **Warnings infrastructure**: Missing text, no HTTP methods, import failures
//! - **Shallow symbol extraction**: Functions, classes, namespaces via `kg::lang` module
//! - **Multi-language support**: JS/TS, Python, Ruby, C#, Java, Markdown, Rust, C, C++, SQL, PHP, Go, Kotlin
//!
//! ## Phase 9.3 Scope
//!
//! - Nodes of kind `"endpoint"` for detected API routes
//! - Edges of kind `"definesEndpoint"` from file→endpoint
//! - Endpoint detection for Next.js patterns:
//!   - `app/api/**/route.ts` (App Router)
//!   - `pages/api/**.ts` (Pages Router)
//!
//! **NOT in scope**:
//! - Full call-graph extraction (deferred to 9.4+)
//! - Incremental extraction (full rebuild per sync)
//!
//! ## Import Detection
//!
//! Best-effort regex-based heuristics for common languages:
//! - **JavaScript/TypeScript**: `import ... from '...'`, `require('...')`
//! - **Rust**: `use crate::...`, `mod ...`
//! - **Python**: `import ...`, `from ... import ...`
//!
//! ## Endpoint Detection (Phase 9.3)
//!
//! Best-effort detection for Next.js route files:
//! - **App Router**: Files matching `app/api/**/route.{ts,tsx,js,jsx}`
//! - **Pages Router**: Files matching `pages/api/**.{ts,tsx,js,jsx}`
//! - **HTTP Methods**: Extracted from exported handler functions (`GET`, `POST`, etc.)
//!
//! ## Future Extensions (TODO)
//!
//! - Phase 9.4+: Full call-graph extraction with function→function edges
//! - Phase 9.4+: Framework-specific endpoint detection (Express, FastAPI, Rails, etc.)

use std::collections::HashMap;
use std::path::Path;

use regex::Regex;
use serde::{Deserialize, Serialize};

use crate::base::{load_base_sources, sources_path, BaseSourceEntry};
use crate::errors::GikError;
use crate::workspace::Workspace;

use super::entities::{KgEdge, KgNode};

// ============================================================================
// Constants
// ============================================================================

/// Default bases to extract KG from.
pub const DEFAULT_KG_BASES: &[&str] = &["code", "docs"];

// ============================================================================
// KgExtractionConfig
// ============================================================================

/// Configuration for KG extraction.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KgExtractionConfig {
    /// Bases to extract KG from. Default: ["code", "docs"].
    #[serde(default = "default_enabled_bases")]
    pub enabled_bases: Vec<String>,

    /// Maximum files to process per base. None = no limit.
    /// Useful as a safety limit for very large repositories.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_files: Option<usize>,

    /// Maximum import edges to create per file. None = no limit.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_edges_per_file: Option<usize>,

    /// Maximum symbols to extract per file. None = no limit. (Phase 9.2.1)
    /// When truncation occurs, a warning is added to KgExtractionResult.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_symbols_per_file: Option<usize>,

    /// Whether to include docs base in extraction.
    #[serde(default = "default_include_docs")]
    pub include_docs: bool,

    /// Whether to extract endpoint nodes from route files.
    /// Default: true (Phase 9.3)
    #[serde(default = "default_extract_endpoints")]
    pub extract_endpoints: bool,

    /// Whether to extract symbol-level nodes (functions, classes, etc.).
    /// Default: true (Phase 9.2.1)
    #[serde(default = "default_extract_symbols")]
    pub extract_symbols: bool,
}

fn default_enabled_bases() -> Vec<String> {
    DEFAULT_KG_BASES.iter().map(|s| (*s).to_string()).collect()
}

fn default_include_docs() -> bool {
    true
}

fn default_extract_endpoints() -> bool {
    true
}

fn default_extract_symbols() -> bool {
    true
}

impl Default for KgExtractionConfig {
    fn default() -> Self {
        Self {
            enabled_bases: default_enabled_bases(),
            max_files: None,
            max_edges_per_file: None,
            max_symbols_per_file: None,
            include_docs: true,
            extract_endpoints: true,
            extract_symbols: true,
        }
    }
}

impl KgExtractionConfig {
    /// Create a new config with default settings.
    pub fn new() -> Self {
        Self::default()
    }

    /// Set maximum files per base.
    pub fn with_max_files(mut self, max: usize) -> Self {
        self.max_files = Some(max);
        self
    }

    /// Set maximum edges per file.
    pub fn with_max_edges_per_file(mut self, max: usize) -> Self {
        self.max_edges_per_file = Some(max);
        self
    }

    /// Disable docs extraction.
    pub fn without_docs(mut self) -> Self {
        self.include_docs = false;
        self
    }

    /// Disable endpoint extraction.
    pub fn without_endpoints(mut self) -> Self {
        self.extract_endpoints = false;
        self
    }

    /// Set maximum symbols per file.
    pub fn with_max_symbols_per_file(mut self, max: usize) -> Self {
        self.max_symbols_per_file = Some(max);
        self
    }

    /// Disable symbol extraction.
    pub fn without_symbols(mut self) -> Self {
        self.extract_symbols = false;
        self
    }
}

// ============================================================================
// KgExtractionResult
// ============================================================================

/// Result of KG extraction for a branch.
#[derive(Debug, Clone, Default)]
pub struct KgExtractionResult {
    /// Extracted nodes.
    pub nodes: Vec<KgNode>,

    /// Extracted edges.
    pub edges: Vec<KgEdge>,

    /// Number of files processed.
    pub files_processed: usize,

    /// Number of import edges created.
    pub import_edges_created: usize,

    /// Number of endpoint nodes created (Phase 9.3).
    pub endpoints_created: usize,

    /// Number of symbol nodes created (Phase 9.2.1).
    pub symbols_created: usize,

    /// Warnings or skipped items.
    pub warnings: Vec<String>,
}

impl KgExtractionResult {
    /// Create an empty result.
    pub fn new() -> Self {
        Self::default()
    }

    /// Merge another result into this one.
    pub fn merge(&mut self, other: KgExtractionResult) {
        self.nodes.extend(other.nodes);
        self.edges.extend(other.edges);
        self.files_processed += other.files_processed;
        self.import_edges_created += other.import_edges_created;
        self.endpoints_created += other.endpoints_created;
        self.symbols_created += other.symbols_created;
        self.warnings.extend(other.warnings);
    }
}

// ============================================================================
// KgExtractor Trait
// ============================================================================

/// Trait for KG extraction from bases.
pub trait KgExtractor {
    /// Extract KG nodes and edges for a branch.
    ///
    /// # Arguments
    ///
    /// * `workspace` - The workspace to extract from
    /// * `branch` - The branch name
    /// * `cfg` - Extraction configuration
    ///
    /// # Returns
    ///
    /// A [`KgExtractionResult`] containing extracted nodes and edges.
    fn extract_for_branch(
        &self,
        workspace: &Workspace,
        branch: &str,
        cfg: &KgExtractionConfig,
    ) -> Result<KgExtractionResult, GikError>;
}

// ============================================================================
// DefaultKgExtractor
// ============================================================================

/// Default implementation of KG extraction.
///
/// Extracts file-level nodes and import edges from code/docs bases.
#[derive(Debug, Clone, Default)]
pub struct DefaultKgExtractor;

impl DefaultKgExtractor {
    /// Create a new default extractor.
    pub fn new() -> Self {
        Self
    }

    /// Extract from a single base.
    fn extract_from_base(
        &self,
        workspace: &Workspace,
        branch: &str,
        base: &str,
        cfg: &KgExtractionConfig,
    ) -> Result<KgExtractionResult, GikError> {
        let base_root = crate::base::base_root(workspace.knowledge_root(), branch, base);

        // Check if base exists
        if !base_root.exists() {
            return Ok(KgExtractionResult::new());
        }

        // Load sources
        let sources_file = sources_path(&base_root);
        let sources = load_base_sources(&sources_file)?;

        if sources.is_empty() {
            return Ok(KgExtractionResult::new());
        }

        // Deduplicate by file path (a file can have multiple chunks)
        let unique_files: HashMap<String, &BaseSourceEntry> =
            sources.iter().map(|s| (s.file_path.clone(), s)).collect();

        let mut result = KgExtractionResult::new();

        // Apply max_files limit if set
        let files_to_process: Vec<_> = unique_files
            .values()
            .take(cfg.max_files.unwrap_or(usize::MAX))
            .collect();

        // Map of file paths to node IDs for edge resolution
        let mut file_to_node_id: HashMap<String, String> = HashMap::new();

        // Pass 1: Create file/doc nodes
        for source in &files_to_process {
            let (node_kind, node_id) = if base == "docs" {
                let id = format!("doc:{}", source.file_path);
                ("doc", id)
            } else {
                let id = format!("file:{}", source.file_path);
                ("file", id)
            };

            file_to_node_id.insert(source.file_path.clone(), node_id.clone());

            // Generate a disambiguated label: include parent folder for common filenames
            let label = generate_disambiguated_label(&source.file_path);

            let node = KgNode::new(&node_id, node_kind, &label)
                .with_props(serde_json::json!({
                    "base": base,
                    "path": source.file_path,
                }))
                .with_branch(branch);

            result.nodes.push(node);
            result.files_processed += 1;
        }

        // Pass 2: Extract import edges (only for code base)
        if base == "code" {
            for source in &files_to_process {
                if let Some(text) = &source.text {
                    let imports = extract_imports(text, &source.file_path, cfg.max_edges_per_file);

                    for import_path in imports {
                        // Try to resolve import to a known file
                        if let Some(resolved) =
                            resolve_import(&import_path, &source.file_path, &file_to_node_id)
                        {
                            let from_id = file_to_node_id
                                .get(&source.file_path)
                                .cloned()
                                .unwrap_or_else(|| format!("file:{}", source.file_path));

                            let edge = KgEdge::new(&from_id, &resolved, "imports")
                                .with_props(serde_json::json!({
                                    "rawImport": import_path,
                                }))
                                .with_branch(branch);

                            result.edges.push(edge);
                            result.import_edges_created += 1;
                        }
                    }
                }
            }
        }

        // Pass 3: Extract endpoint nodes (Phase 9.3)
        if base == "code" && cfg.extract_endpoints {
            for source in &files_to_process {
                if let Some(endpoint_info) =
                    detect_endpoint(&source.file_path, source.text.as_deref())
                {
                    let file_node_id = file_to_node_id
                        .get(&source.file_path)
                        .cloned()
                        .unwrap_or_else(|| format!("file:{}", source.file_path));

                    // Create endpoint node for each HTTP method
                    for method in &endpoint_info.methods {
                        let endpoint_id =
                            format!("endpoint:{}:{}", method.to_uppercase(), endpoint_info.route);

                        let endpoint_node =
                            KgNode::new(&endpoint_id, "endpoint", &endpoint_info.route)
                                .with_props(serde_json::json!({
                                    "base": base,
                                    "path": source.file_path,
                                    "route": endpoint_info.route,
                                    "httpMethod": method.to_uppercase(),
                                }))
                                .with_branch(branch);

                        result.nodes.push(endpoint_node);
                        result.endpoints_created += 1;

                        // Create edge from file to endpoint
                        let edge = KgEdge::new(&file_node_id, &endpoint_id, "definesEndpoint")
                            .with_props(serde_json::json!({
                                "httpMethod": method.to_uppercase(),
                            }))
                            .with_branch(branch);

                        result.edges.push(edge);
                    }
                }
            }
        }

        // Pass 4: Extract symbol-level nodes (Phase 9.2.1)
        if base == "code" && cfg.extract_symbols {
            for source in &files_to_process {
                if let Some(text) = &source.text {
                    let file_node_id = file_to_node_id
                        .get(&source.file_path)
                        .cloned()
                        .unwrap_or_else(|| format!("file:{}", source.file_path));

                    // Extract symbols using the lang module
                    let (mut symbols, relations) =
                        crate::kg::lang::extract_for_file(&source.file_path, text);

                    // Deduplicate symbol IDs
                    crate::kg::lang::deduplicate_symbol_ids(&mut symbols);

                    // Apply max_symbols_per_file limit if set
                    let limit = cfg.max_symbols_per_file.unwrap_or(usize::MAX);
                    if symbols.len() > limit {
                        result.warnings.push(format!(
                            "File '{}' had {} symbols, truncated to {}",
                            source.file_path,
                            symbols.len(),
                            limit
                        ));
                        symbols.truncate(limit);
                    }

                    // Create symbol nodes and file→symbol edges
                    for sym in symbols {
                        // Create symbol node
                        // Use tag_from_path for accurate language distinction (e.g., ts vs js)
                        let lang_tag = crate::kg::lang::tag_from_path(&source.file_path);
                        let node = KgNode::new(&sym.id, &sym.kind, &sym.name)
                            .with_props(serde_json::json!({
                                "base": base,
                                "path": source.file_path,
                                "language": lang_tag,
                                "symbolKind": sym.kind,
                            }))
                            .with_branch(branch);

                        result.nodes.push(node);
                        result.symbols_created += 1;

                        // Create edge from file to symbol
                        let edge = KgEdge::new(&file_node_id, &sym.id, "defines")
                            .with_props(serde_json::json!({
                                "symbolKind": sym.kind,
                            }))
                            .with_branch(branch);

                        result.edges.push(edge);
                    }

                    // Pass 4b: Process relation candidates (Phase 9.2.2)
                    // Creates edges for usesClass, usesUiComponent, belongsToModule, etc.
                    for rel in relations {
                        // Build props from the relation candidate
                        let mut edge_props = rel.props.clone();
                        // Add source file info if not present
                        if !edge_props
                            .as_object()
                            .is_some_and(|o| o.contains_key("sourceFile"))
                        {
                            if let Some(obj) = edge_props.as_object_mut() {
                                obj.insert(
                                    "sourceFile".to_string(),
                                    serde_json::json!(source.file_path),
                                );
                            }
                        }

                        let edge = KgEdge::new(&rel.from_id, &rel.to_id, &rel.kind)
                            .with_props(edge_props)
                            .with_branch(branch);

                        result.edges.push(edge);
                    }
                } else {
                    // Warn about missing text
                    result.warnings.push(format!(
                        "File '{}' has no text content for symbol extraction",
                        source.file_path
                    ));
                }
            }
        }

        // TODO(gik.phase-9.3+): Extract doc→file "mentions" edges
        // For docs base, we could scan for relative paths like "./src/..." and
        // create edges of kind "mentions" or "documents" to code files.

        Ok(result)
    }
}

impl KgExtractor for DefaultKgExtractor {
    fn extract_for_branch(
        &self,
        workspace: &Workspace,
        branch: &str,
        cfg: &KgExtractionConfig,
    ) -> Result<KgExtractionResult, GikError> {
        let mut result = KgExtractionResult::new();

        for base in &cfg.enabled_bases {
            // Skip docs if disabled
            if base == "docs" && !cfg.include_docs {
                continue;
            }

            // Skip memory base (not suitable for structural KG)
            if base == "memory" {
                continue;
            }

            let base_result = self.extract_from_base(workspace, branch, base, cfg)?;
            result.merge(base_result);
        }

        Ok(result)
    }
}

// ============================================================================
// Import Extraction Helpers
// ============================================================================

/// Extract import statements from source text.
///
/// Uses regex-based heuristics for common languages. Returns raw import
/// targets (not yet resolved to file paths).
fn extract_imports(text: &str, file_path: &str, max_edges: Option<usize>) -> Vec<String> {
    let ext = Path::new(file_path)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("");

    let imports = match ext {
        "js" | "jsx" | "ts" | "tsx" | "mjs" | "cjs" => extract_js_imports(text),
        "rs" => extract_rust_imports(text),
        "py" => extract_python_imports(text),
        _ => Vec::new(),
    };

    // Apply max_edges limit
    if let Some(max) = max_edges {
        imports.into_iter().take(max).collect()
    } else {
        imports
    }
}

/// Extract JavaScript/TypeScript imports.
fn extract_js_imports(text: &str) -> Vec<String> {
    let mut imports = Vec::new();

    // Match: import ... from 'path' or import ... from "path"
    // Also: import 'path' (side-effect imports)
    let import_re =
        Regex::new(r#"(?:import\s+(?:.*?\s+from\s+)?['"]([^'"]+)['"])|(?:require\s*\(\s*['"]([^'"]+)['"]\s*\))"#)
            .expect("Invalid regex");

    for cap in import_re.captures_iter(text) {
        if let Some(m) = cap.get(1).or_else(|| cap.get(2)) {
            imports.push(m.as_str().to_string());
        }
    }

    imports
}

/// Extract Rust imports.
fn extract_rust_imports(text: &str) -> Vec<String> {
    let mut imports = Vec::new();

    // Match: use crate::path or use super::path or mod path
    let use_re = Regex::new(r#"(?:use\s+(?:crate|super|self)::([a-zA-Z_][a-zA-Z0-9_:]*)|mod\s+([a-zA-Z_][a-zA-Z0-9_]*))"#)
        .expect("Invalid regex");

    for cap in use_re.captures_iter(text) {
        if let Some(m) = cap.get(1).or_else(|| cap.get(2)) {
            imports.push(m.as_str().to_string());
        }
    }

    imports
}

/// Extract Python imports.
fn extract_python_imports(text: &str) -> Vec<String> {
    let mut imports = Vec::new();

    // Match: import module or from module import ...
    let import_re = Regex::new(
        r#"(?:^|\n)\s*(?:from\s+([a-zA-Z_][a-zA-Z0-9_.]*)|import\s+([a-zA-Z_][a-zA-Z0-9_.]*))"#,
    )
    .expect("Invalid regex");

    for cap in import_re.captures_iter(text) {
        if let Some(m) = cap.get(1).or_else(|| cap.get(2)) {
            imports.push(m.as_str().to_string());
        }
    }

    imports
}

/// Generate a disambiguated label for a file path.
///
/// For common filenames like `page.tsx`, `index.ts`, `layout.tsx`, includes
/// parent folder(s) to make the label unique and meaningful.
///
/// Examples:
/// - `apps/web/app/home/page.tsx` → `home/page.tsx`
/// - `packages/ui/src/button/index.ts` → `button/index.ts`
/// - `utils/format.ts` → `format.ts`
fn generate_disambiguated_label(file_path: &str) -> String {
    let path = Path::new(file_path);
    let filename = path
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| file_path.to_string());

    // Common filenames that need disambiguation
    let common_names = [
        "page.tsx",
        "page.ts",
        "page.jsx",
        "page.js",
        "index.tsx",
        "index.ts",
        "index.jsx",
        "index.js",
        "layout.tsx",
        "layout.ts",
        "route.tsx",
        "route.ts",
        "types.ts",
        "types.d.ts",
        "utils.ts",
        "helpers.ts",
        "constants.ts",
        "config.ts",
        "schema.ts",
        "mod.rs",
        "lib.rs",
        "main.rs",
        "__init__.py",
    ];

    if common_names.contains(&filename.as_str()) {
        // Get parent folder name
        if let Some(parent) = path.parent() {
            if let Some(parent_name) = parent.file_name() {
                let parent_str = parent_name.to_string_lossy();
                // Skip generic folder names
                if !["src", "lib", "app", "pages", "components"].contains(&parent_str.as_ref()) {
                    return format!("{}/{}", parent_str, filename);
                }
                // Try grandparent
                if let Some(grandparent) = parent.parent() {
                    if let Some(gp_name) = grandparent.file_name() {
                        let gp_str = gp_name.to_string_lossy();
                        if !["src", "lib", "app", "pages", "components"].contains(&gp_str.as_ref())
                        {
                            return format!("{}/{}", gp_str, filename);
                        }
                    }
                }
            }
        }
    }

    filename
}

/// Try to resolve an import path to a known file node ID.
///
/// This is a best-effort resolution. Returns `None` if the import
/// cannot be resolved to a known file.
fn resolve_import(
    import_path: &str,
    source_file: &str,
    known_files: &HashMap<String, String>,
) -> Option<String> {
    // Skip external packages (no relative path marker and not in known files)
    if !import_path.starts_with('.') && !import_path.starts_with('/') {
        // Could be an npm package or standard library - skip
        return None;
    }

    // Get the directory of the source file
    let source_dir = Path::new(source_file).parent().unwrap_or(Path::new(""));

    // Resolve relative path
    let resolved = source_dir.join(import_path);

    // Normalize the path
    let normalized = normalize_path(&resolved);

    // Try common extensions
    let candidates = generate_import_candidates(&normalized);

    for candidate in candidates {
        if let Some(node_id) = known_files.get(&candidate) {
            return Some(node_id.clone());
        }
    }

    None
}

/// Normalize a path by resolving .. and . components.
fn normalize_path(path: &Path) -> String {
    let mut parts: Vec<&str> = Vec::new();

    for component in path.components() {
        match component {
            std::path::Component::Normal(s) => {
                if let Some(s_str) = s.to_str() {
                    parts.push(s_str);
                }
            }
            std::path::Component::ParentDir => {
                parts.pop();
            }
            std::path::Component::CurDir => {}
            _ => {}
        }
    }

    parts.join("/")
}

/// Generate candidate file paths for an import.
fn generate_import_candidates(base_path: &str) -> Vec<String> {
    let mut candidates = vec![base_path.to_string()];

    // Add common extensions
    let extensions = [
        ".ts",
        ".tsx",
        ".js",
        ".jsx",
        ".mjs",
        ".cjs",
        ".rs",
        ".py",
        "/index.ts",
        "/index.tsx",
        "/index.js",
        "/index.jsx",
        "/mod.rs",
    ];

    for ext in extensions {
        candidates.push(format!("{}{}", base_path, ext));
    }

    candidates
}

// ============================================================================
// Endpoint Detection Helpers (Phase 9.3)
// ============================================================================

/// Information about a detected API endpoint.
#[derive(Debug, Clone)]
struct EndpointInfo {
    /// The API route path (e.g., "/api/users").
    route: String,

    /// HTTP methods handled by this endpoint.
    methods: Vec<String>,
}

/// Detect if a file defines an API endpoint.
///
/// Supports Next.js patterns:
/// - App Router: `app/api/**/route.{ts,tsx,js,jsx}`
/// - Pages Router: `pages/api/**.{ts,tsx,js,jsx}`
///
/// Returns `None` if the file is not a route file.
fn detect_endpoint(file_path: &str, source_text: Option<&str>) -> Option<EndpointInfo> {
    // Check if file matches route patterns
    let route = extract_route_from_path(file_path)?;

    // Extract HTTP methods from source code (if available)
    let methods = if let Some(text) = source_text {
        extract_http_methods(text)
    } else {
        // Default to common methods if source not available
        vec!["GET".to_string()]
    };

    if methods.is_empty() {
        return None;
    }

    Some(EndpointInfo { route, methods })
}

/// Extract API route from file path based on Next.js conventions.
///
/// Examples:
/// - `app/api/users/route.ts` → `/api/users`
/// - `pages/api/users.ts` → `/api/users`
/// - `packages/web/src/app/api/health/route.ts` → `/api/health`
/// - `packages/web/src/app/(app)/api/items/[id]/route.ts` → `/api/items/[id]`
///
/// ## Path Normalization (Phase 9.2.1)
///
/// Windows-style backslashes are normalized to forward slashes before matching.
fn extract_route_from_path(file_path: &str) -> Option<String> {
    // Normalize Windows-style paths to POSIX-style (Phase 9.2.1)
    let file_path = file_path.replace('\\', "/");
    let path = file_path.to_lowercase();

    // App Router pattern: .../app/api/**/route.ts
    if let Some(idx) = path.find("/app/api/") {
        let after_app = &file_path[idx..];
        // Remove the leading /app and trailing /route.{ext}
        // NOTE: Must strip .tsx/.jsx BEFORE .ts/.js to handle compound extensions correctly
        let route_part = after_app
            .strip_prefix("/app")?
            .trim_end_matches(".tsx")
            .trim_end_matches(".jsx")
            .trim_end_matches(".ts")
            .trim_end_matches(".js")
            .trim_end_matches("/route");
        return Some(route_part.to_string());
    }

    // App Router with route groups: .../app/(group)/api/**/route.ts
    if path.contains("/app/") && path.contains("/api/") && is_route_file(&path) {
        // Find the /api/ part and extract from there
        if let Some(api_idx) = path.find("/api/") {
            let after_api = &file_path[api_idx..];
            let route = strip_route_extension(after_api).trim_end_matches("/route");
            return Some(route.to_string());
        }
    }

    // Pages Router pattern: .../pages/api/**.ts
    if let Some(idx) = path.find("/pages/api/") {
        let after_pages = &file_path[idx..];
        let route_part = after_pages.strip_prefix("/pages")?;
        let route_part = strip_route_extension(route_part).trim_end_matches("/index");
        return Some(route_part.to_string());
    }

    // Direct pattern for standalone files: app/api/.../route.ts or pages/api/...
    if (path.starts_with("app/api/") || path.starts_with("src/app/api/")) && is_route_file(&path) {
        let route = file_path
            .trim_start_matches("src/")
            .trim_start_matches("app");
        let route = strip_route_extension(route).trim_end_matches("/route");
        return Some(route.to_string());
    }

    if (path.starts_with("pages/api/") || path.starts_with("src/pages/api/"))
        && is_js_ts_file(&path)
    {
        let route = file_path
            .trim_start_matches("src/")
            .trim_start_matches("pages");
        let route = strip_route_extension(route).trim_end_matches("/index");
        return Some(route.to_string());
    }

    None
}

/// Check if a path is a Next.js route file (route.ts/tsx/js/jsx).
fn is_route_file(path: &str) -> bool {
    path.ends_with("/route.ts")
        || path.ends_with("/route.tsx")
        || path.ends_with("/route.js")
        || path.ends_with("/route.jsx")
}

/// Check if a path is a JS/TS file.
fn is_js_ts_file(path: &str) -> bool {
    path.ends_with(".ts")
        || path.ends_with(".tsx")
        || path.ends_with(".js")
        || path.ends_with(".jsx")
}

/// Strip JS/TS file extensions in the correct order.
///
/// IMPORTANT: Must strip .tsx/.jsx BEFORE .ts/.js to handle compound extensions.
/// Example: "route.tsx".trim_end_matches(".ts") would incorrectly produce "route.x"
fn strip_route_extension(path: &str) -> &str {
    path.trim_end_matches(".tsx")
        .trim_end_matches(".jsx")
        .trim_end_matches(".ts")
        .trim_end_matches(".js")
}

/// Extract HTTP methods from Next.js route handler source code.
///
/// Looks for exported functions like:
/// - `export async function GET(...)`
/// - `export function POST(...)`
/// - `export const GET = ...`
fn extract_http_methods(source_text: &str) -> Vec<String> {
    let mut methods = Vec::new();

    // Common HTTP methods in Next.js App Router
    let http_methods = ["GET", "POST", "PUT", "PATCH", "DELETE", "HEAD", "OPTIONS"];

    // Pattern: export async function METHOD or export function METHOD or export const METHOD
    let export_fn_re = Regex::new(
        r"export\s+(?:async\s+)?(?:function|const)\s+(GET|POST|PUT|PATCH|DELETE|HEAD|OPTIONS)\b",
    )
    .expect("Invalid regex");

    for cap in export_fn_re.captures_iter(source_text) {
        if let Some(m) = cap.get(1) {
            let method = m.as_str().to_uppercase();
            if http_methods.contains(&method.as_str()) && !methods.contains(&method) {
                methods.push(method);
            }
        }
    }

    methods
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extraction_config_default() {
        let cfg = KgExtractionConfig::default();
        assert_eq!(cfg.enabled_bases, vec!["code", "docs"]);
        assert!(cfg.include_docs);
        assert!(cfg.max_files.is_none());
        assert!(cfg.max_edges_per_file.is_none());
    }

    #[test]
    fn test_extraction_config_builder() {
        let cfg = KgExtractionConfig::new()
            .with_max_files(100)
            .with_max_edges_per_file(50)
            .without_docs();

        assert_eq!(cfg.max_files, Some(100));
        assert_eq!(cfg.max_edges_per_file, Some(50));
        assert!(!cfg.include_docs);
    }

    #[test]
    fn test_extract_js_imports() {
        let code = r#"
import React from 'react';
import { useState } from 'react';
import './styles.css';
import Component from './components/Component';
const utils = require('./utils');
const fs = require('fs');
"#;

        let imports = extract_js_imports(code);
        assert!(imports.contains(&"react".to_string()));
        assert!(imports.contains(&"./styles.css".to_string()));
        assert!(imports.contains(&"./components/Component".to_string()));
        assert!(imports.contains(&"./utils".to_string()));
        assert!(imports.contains(&"fs".to_string()));
    }

    #[test]
    fn test_extract_rust_imports() {
        let code = r#"
use crate::config::Config;
use super::utils;
use self::types::*;
mod parser;
mod lexer;
"#;

        let imports = extract_rust_imports(code);
        assert!(imports.contains(&"config::Config".to_string()));
        assert!(imports.contains(&"utils".to_string()));
        assert!(imports.contains(&"parser".to_string()));
        assert!(imports.contains(&"lexer".to_string()));
    }

    #[test]
    fn test_extract_python_imports() {
        let code = r#"
import os
import sys
from pathlib import Path
from . import utils
from ..models import User
"#;

        let imports = extract_python_imports(code);
        assert!(imports.contains(&"os".to_string()));
        assert!(imports.contains(&"sys".to_string()));
        assert!(imports.contains(&"pathlib".to_string()));
    }

    #[test]
    fn test_resolve_import_relative() {
        let mut known_files = HashMap::new();
        known_files.insert("src/utils.ts".to_string(), "file:src/utils.ts".to_string());
        known_files.insert(
            "src/components/Button.tsx".to_string(),
            "file:src/components/Button.tsx".to_string(),
        );

        // Resolve relative import from src/index.ts
        let resolved = resolve_import("./utils", "src/index.ts", &known_files);
        assert_eq!(resolved, Some("file:src/utils.ts".to_string()));

        // External packages should not resolve
        let resolved = resolve_import("react", "src/index.ts", &known_files);
        assert!(resolved.is_none());
    }

    #[test]
    fn test_normalize_path() {
        assert_eq!(normalize_path(Path::new("src/../lib/utils")), "lib/utils");
        assert_eq!(
            normalize_path(Path::new("./components/Button")),
            "components/Button"
        );
        assert_eq!(
            normalize_path(Path::new("src/utils/../config")),
            "src/config"
        );
    }

    #[test]
    fn test_generate_import_candidates() {
        let candidates = generate_import_candidates("src/utils");
        assert!(candidates.contains(&"src/utils".to_string()));
        assert!(candidates.contains(&"src/utils.ts".to_string()));
        assert!(candidates.contains(&"src/utils.js".to_string()));
        assert!(candidates.contains(&"src/utils/index.ts".to_string()));
    }

    #[test]
    fn test_extraction_result_merge() {
        let mut r1 = KgExtractionResult {
            nodes: vec![KgNode::new("file:a.ts", "file", "a.ts")],
            edges: vec![],
            files_processed: 1,
            import_edges_created: 0,
            endpoints_created: 0,
            symbols_created: 0,
            warnings: vec![],
        };

        let r2 = KgExtractionResult {
            nodes: vec![KgNode::new("file:b.ts", "file", "b.ts")],
            edges: vec![KgEdge::new("file:a.ts", "file:b.ts", "imports")],
            files_processed: 1,
            import_edges_created: 1,
            endpoints_created: 1,
            symbols_created: 2,
            warnings: vec!["Warning 1".to_string()],
        };

        r1.merge(r2);

        assert_eq!(r1.nodes.len(), 2);
        assert_eq!(r1.edges.len(), 1);
        assert_eq!(r1.files_processed, 2);
        assert_eq!(r1.import_edges_created, 1);
        assert_eq!(r1.endpoints_created, 1);
        assert_eq!(r1.symbols_created, 2);
        assert_eq!(r1.warnings.len(), 1);
    }

    #[test]
    fn test_extract_route_from_path_app_router() {
        // Basic app router pattern
        assert_eq!(
            extract_route_from_path("app/api/users/route.ts"),
            Some("/api/users".to_string())
        );

        // Nested route
        assert_eq!(
            extract_route_from_path("app/api/users/[id]/route.ts"),
            Some("/api/users/[id]".to_string())
        );

        // With src prefix
        assert_eq!(
            extract_route_from_path("src/app/api/health/route.ts"),
            Some("/api/health".to_string())
        );

        // Monorepo pattern
        assert_eq!(
            extract_route_from_path("packages/web/src/app/api/items/route.ts"),
            Some("/api/items".to_string())
        );

        // With route group
        assert_eq!(
            extract_route_from_path("packages/web/src/app/(app)/api/items/[id]/route.ts"),
            Some("/api/items/[id]".to_string())
        );
    }

    #[test]
    fn test_extract_route_from_path_pages_router() {
        // Basic pages router pattern
        assert_eq!(
            extract_route_from_path("pages/api/users.ts"),
            Some("/api/users".to_string())
        );

        // Index pattern
        assert_eq!(
            extract_route_from_path("pages/api/users/index.ts"),
            Some("/api/users".to_string())
        );

        // With src prefix
        assert_eq!(
            extract_route_from_path("src/pages/api/auth/login.ts"),
            Some("/api/auth/login".to_string())
        );
    }

    #[test]
    fn test_extract_route_from_path_non_route() {
        // Regular file, not a route
        assert!(extract_route_from_path("src/components/Button.tsx").is_none());

        // Page but not API
        assert!(extract_route_from_path("app/users/page.tsx").is_none());

        // Utils file in API folder
        assert!(extract_route_from_path("app/api/utils.ts").is_none());
    }

    #[test]
    fn test_extract_http_methods() {
        let code = r#"
export async function GET(request: Request) {
    return Response.json({ message: "Hello" });
}

export async function POST(request: Request) {
    const body = await request.json();
    return Response.json({ created: true });
}
"#;

        let methods = extract_http_methods(code);
        assert!(methods.contains(&"GET".to_string()));
        assert!(methods.contains(&"POST".to_string()));
        assert_eq!(methods.len(), 2);
    }

    #[test]
    fn test_extract_http_methods_const_export() {
        let code = r#"
export const GET = async (request: Request) => {
    return Response.json({ message: "Hello" });
};

export const DELETE = async (request: Request) => {
    return new Response(null, { status: 204 });
};
"#;

        let methods = extract_http_methods(code);
        assert!(methods.contains(&"GET".to_string()));
        assert!(methods.contains(&"DELETE".to_string()));
    }

    #[test]
    fn test_detect_endpoint() {
        let code = r#"
export async function GET(request: Request) {
    return Response.json({ users: [] });
}

export async function POST(request: Request) {
    return Response.json({ created: true });
}
"#;

        let endpoint = detect_endpoint("packages/web/src/app/api/users/route.ts", Some(code));
        assert!(endpoint.is_some());

        let info = endpoint.unwrap();
        assert_eq!(info.route, "/api/users");
        assert!(info.methods.contains(&"GET".to_string()));
        assert!(info.methods.contains(&"POST".to_string()));
    }

    // ========================================================================
    // Phase 9.2.1: Extension trimming and path normalization tests
    // ========================================================================

    #[test]
    fn test_extract_route_tsx_extension() {
        // Test .tsx files are handled correctly (must strip .tsx before .ts)
        assert_eq!(
            extract_route_from_path("app/api/users/route.tsx"),
            Some("/api/users".to_string())
        );
        assert_eq!(
            extract_route_from_path("packages/web/src/app/api/items/route.tsx"),
            Some("/api/items".to_string())
        );
    }

    #[test]
    fn test_extract_route_jsx_extension() {
        // Test .jsx files are handled correctly (must strip .jsx before .js)
        assert_eq!(
            extract_route_from_path("app/api/users/route.jsx"),
            Some("/api/users".to_string())
        );
        assert_eq!(
            extract_route_from_path("pages/api/users.jsx"),
            Some("/api/users".to_string())
        );
    }

    #[test]
    fn test_extract_route_windows_path() {
        // Test Windows-style paths are normalized
        assert_eq!(
            extract_route_from_path("packages\\web\\src\\app\\api\\users\\route.ts"),
            Some("/api/users".to_string())
        );
        assert_eq!(
            extract_route_from_path("app\\api\\health\\route.tsx"),
            Some("/api/health".to_string())
        );
    }

    #[test]
    fn test_strip_route_extension() {
        // Unit test for the helper function
        assert_eq!(strip_route_extension("route.tsx"), "route");
        assert_eq!(strip_route_extension("route.jsx"), "route");
        assert_eq!(strip_route_extension("route.ts"), "route");
        assert_eq!(strip_route_extension("route.js"), "route");
        // Ensure .tsx is stripped correctly and doesn't leave ".x"
        assert_eq!(strip_route_extension("foo.tsx"), "foo");
        assert_eq!(strip_route_extension("bar.jsx"), "bar");
    }

    #[test]
    fn test_is_route_file() {
        assert!(is_route_file("app/api/users/route.ts"));
        assert!(is_route_file("app/api/users/route.tsx"));
        assert!(is_route_file("app/api/users/route.js"));
        assert!(is_route_file("app/api/users/route.jsx"));
        assert!(!is_route_file("app/api/users/page.tsx"));
        assert!(!is_route_file("app/api/utils.ts"));
    }

    #[test]
    fn test_is_js_ts_file() {
        assert!(is_js_ts_file("src/utils.ts"));
        assert!(is_js_ts_file("src/Button.tsx"));
        assert!(is_js_ts_file("lib/index.js"));
        assert!(is_js_ts_file("components/Card.jsx"));
        assert!(!is_js_ts_file("README.md"));
        assert!(!is_js_ts_file("config.json"));
    }

    // ========================================================================
    // Phase 9.2.1: Symbol extraction integration tests
    // ========================================================================

    #[test]
    fn test_symbol_extraction_from_js_ts() {
        // Use the lang module directly to test symbol extraction
        let code = r#"
export function handleClick() {
    console.log("clicked");
}

export class UserService {
    async getUser(id: string) {
        return { id };
    }
}

interface User {
    id: string;
    name: string;
}

type UserId = string;
"#;

        let (symbols, _relations) = crate::kg::lang::extract_for_file("src/utils.ts", code);

        // Check we got functions
        let fn_names: Vec<&str> = symbols
            .iter()
            .filter(|s| s.kind == "function")
            .map(|s| s.name.as_str())
            .collect();
        assert!(fn_names.contains(&"handleClick"));

        // Check we got classes
        let class_names: Vec<&str> = symbols
            .iter()
            .filter(|s| s.kind == "class")
            .map(|s| s.name.as_str())
            .collect();
        assert!(class_names.contains(&"UserService"));

        // Check we got interfaces (TypeScript)
        let iface_names: Vec<&str> = symbols
            .iter()
            .filter(|s| s.kind == "interface")
            .map(|s| s.name.as_str())
            .collect();
        assert!(iface_names.contains(&"User"));

        // Check we got type aliases (TypeScript)
        let type_names: Vec<&str> = symbols
            .iter()
            .filter(|s| s.kind == "type")
            .map(|s| s.name.as_str())
            .collect();
        assert!(type_names.contains(&"UserId"));
    }

    #[test]
    fn test_symbol_extraction_from_python() {
        let code = r#"
class UserService:
    def __init__(self, db):
        self.db = db

    def get_user(self, user_id):
        return self.db.get(user_id)

def process_data(items):
    return [item for item in items]

MAX_RETRIES = 3
"#;

        let (symbols, _relations) = crate::kg::lang::extract_for_file("src/services.py", code);

        // Check we got classes
        let class_names: Vec<&str> = symbols
            .iter()
            .filter(|s| s.kind == "class")
            .map(|s| s.name.as_str())
            .collect();
        assert!(class_names.contains(&"UserService"));

        // Check we got functions
        let fn_names: Vec<&str> = symbols
            .iter()
            .filter(|s| s.kind == "function")
            .map(|s| s.name.as_str())
            .collect();
        assert!(fn_names.contains(&"process_data"));

        // Check we got constants
        let const_names: Vec<&str> = symbols
            .iter()
            .filter(|s| s.kind == "constant")
            .map(|s| s.name.as_str())
            .collect();
        assert!(const_names.contains(&"MAX_RETRIES"));
    }

    #[test]
    fn test_symbol_extraction_from_rust() {
        let code = r#"
pub struct User {
    pub id: u64,
    pub name: String,
}

pub enum Status {
    Active,
    Inactive,
}

pub trait Repository {
    fn find(&self, id: u64) -> Option<User>;
}

pub fn process_items(items: Vec<Item>) -> Vec<Item> {
    items
}

pub mod utils {
    pub fn helper() {}
}
"#;

        let (symbols, _relations) = crate::kg::lang::extract_for_file("src/lib.rs", code);

        // Check we got structs
        let struct_names: Vec<&str> = symbols
            .iter()
            .filter(|s| s.kind == "struct")
            .map(|s| s.name.as_str())
            .collect();
        assert!(struct_names.contains(&"User"));

        // Check we got enums
        let enum_names: Vec<&str> = symbols
            .iter()
            .filter(|s| s.kind == "enum")
            .map(|s| s.name.as_str())
            .collect();
        assert!(enum_names.contains(&"Status"));

        // Check we got traits
        let trait_names: Vec<&str> = symbols
            .iter()
            .filter(|s| s.kind == "trait")
            .map(|s| s.name.as_str())
            .collect();
        assert!(trait_names.contains(&"Repository"));

        // Check we got functions
        let fn_names: Vec<&str> = symbols
            .iter()
            .filter(|s| s.kind == "function")
            .map(|s| s.name.as_str())
            .collect();
        assert!(fn_names.contains(&"process_items"));

        // Check we got modules
        let mod_names: Vec<&str> = symbols
            .iter()
            .filter(|s| s.kind == "module")
            .map(|s| s.name.as_str())
            .collect();
        assert!(mod_names.contains(&"utils"));
    }
}
