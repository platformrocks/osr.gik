//! Stack scan and inventory for GIK workspaces.
//!
//! This module provides functionality to scan a workspace and collect
//! information about files, dependencies, and technologies.
//!
//! # Performance
//!
//! The stack scanner uses parallel directory walking via `ignore::WalkParallel`
//! for improved performance on large repositories. Manifest files are also
//! parsed in parallel using `rayon`. The final output is sorted by path for
//! deterministic results.

use std::collections::{HashMap, HashSet};
use std::fs::{self, File};
use std::io::{BufRead, BufReader, BufWriter, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use chrono::{DateTime, Utc};
use ignore::{WalkBuilder, WalkState};
use rayon::prelude::*;
use serde::{Deserialize, Serialize};

use crate::constants::{should_ignore_dir, GIK_IGNORE_FILENAME};
use crate::errors::GikError;

// ============================================================================
// Types
// ============================================================================

/// Kind of stack file entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub enum StackFileKind {
    /// A regular file.
    File,
    /// A directory.
    Dir,
}

/// A file or directory entry in the stack inventory.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StackFileEntry {
    /// Relative path from workspace root.
    pub path: String,
    /// Whether this is a file or directory.
    pub kind: StackFileKind,
    /// Detected programming languages.
    pub languages: Vec<String>,
    /// Number of files (only for directories).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub file_count: Option<u64>,
}

/// A dependency entry detected from manifest files.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StackDependencyEntry {
    /// Package manager (e.g., "cargo", "npm", "pip").
    pub manager: String,
    /// Dependency name.
    pub name: String,
    /// Declared version (raw string).
    pub version: String,
    /// Dependency scope ("runtime", "dev", "build").
    pub scope: String,
    /// Path to manifest file (relative to workspace root).
    pub manifest_path: String,
}

/// A technology tag detected from the workspace.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StackTechEntry {
    /// Kind of technology ("framework", "language", "infra", "tool").
    pub kind: String,
    /// Technology name (e.g., "Rust", "Next.js").
    pub name: String,
    /// How this was inferred (e.g., "dependency:next", "files:*.rs").
    pub source: String,
    /// Confidence score (0.0–1.0).
    pub confidence: f32,
}

/// Statistics about the stack inventory.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StackStats {
    /// Total number of files.
    pub total_files: u64,
    /// Per-language file counts.
    pub languages: HashMap<String, u64>,
    /// Detected package managers.
    pub managers: Vec<String>,
    /// When this snapshot was generated.
    pub generated_at: DateTime<Utc>,
}

/// Complete stack inventory for a workspace.
#[derive(Debug, Clone)]
pub struct StackInventory {
    /// File entries.
    pub files: Vec<StackFileEntry>,
    /// Dependency entries.
    pub dependencies: Vec<StackDependencyEntry>,
    /// Technology tags.
    pub tech: Vec<StackTechEntry>,
    /// Summary statistics.
    pub stats: StackStats,
}

// ============================================================================
// Lockfile-based Package Manager Detection
// ============================================================================

/// Mapping of lockfiles to package managers with priority.
/// Priority order for JavaScript: pnpm > yarn > bun > npm
///
/// Returns (lockfile_name, manager_name, priority) tuples.
/// Lower priority number = higher priority.
fn lockfile_manager_mappings() -> &'static [(&'static str, &'static str, u8)] {
    &[
        // JavaScript/Node.js (priority: pnpm > yarn > bun > npm)
        ("pnpm-lock.yaml", "pnpm", 1),
        ("yarn.lock", "yarn", 2),
        ("bun.lockb", "bun", 3),
        ("bun.lock", "bun", 3),
        ("package-lock.json", "npm", 4),
        // Rust
        ("Cargo.lock", "cargo", 1),
        // Python
        ("poetry.lock", "poetry", 1),
        ("Pipfile.lock", "pipenv", 2),
        ("pdm.lock", "pdm", 3),
        ("uv.lock", "uv", 4),
        // PHP
        ("composer.lock", "composer", 1),
        // Go
        ("go.sum", "go", 1),
        // Ruby
        ("Gemfile.lock", "bundler", 1),
        // .NET
        ("packages.lock.json", "nuget", 1),
    ]
}

/// Detect package manager from a lockfile name.
/// Returns (manager_name, priority) if recognized.
fn detect_manager_from_lockfile(file_name: &str) -> Option<(&'static str, u8)> {
    for &(lockfile, manager, priority) in lockfile_manager_mappings() {
        if file_name == lockfile {
            return Some((manager, priority));
        }
    }
    None
}

/// Categories of package managers for manifest association.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum ManagerCategory {
    JavaScript,
    Rust,
    Python,
    Php,
    Go,
    Ruby,
    DotNet,
}

/// Get the category of a package manager.
fn manager_category(manager: &str) -> Option<ManagerCategory> {
    match manager {
        "npm" | "pnpm" | "yarn" | "bun" => Some(ManagerCategory::JavaScript),
        "cargo" => Some(ManagerCategory::Rust),
        "poetry" | "pipenv" | "pdm" | "uv" | "pip" => Some(ManagerCategory::Python),
        "composer" => Some(ManagerCategory::Php),
        "go" => Some(ManagerCategory::Go),
        "bundler" => Some(ManagerCategory::Ruby),
        "nuget" => Some(ManagerCategory::DotNet),
        _ => None,
    }
}

// ============================================================================
// Language Detection
// ============================================================================

/// Map file extension to programming language.
fn extension_to_language(ext: &str) -> Option<&'static str> {
    match ext.to_lowercase().as_str() {
        // Rust
        "rs" => Some("rust"),
        // JavaScript/TypeScript
        "js" | "mjs" | "cjs" => Some("javascript"),
        "ts" | "mts" | "cts" => Some("typescript"),
        "jsx" => Some("javascript"),
        "tsx" => Some("typescript"),
        // Modern web frameworks (SFCs)
        "vue" => Some("vue"),
        "svelte" => Some("svelte"),
        "astro" => Some("astro"),
        "mdx" => Some("mdx"),
        // Python
        "py" | "pyi" | "pyw" => Some("python"),
        // Go
        "go" => Some("go"),
        // Java/Kotlin
        "java" => Some("java"),
        "kt" | "kts" => Some("kotlin"),
        // C/C++
        "c" | "h" => Some("c"),
        "cpp" | "cc" | "cxx" | "hpp" | "hh" | "hxx" => Some("cpp"),
        // C#
        "cs" => Some("csharp"),
        // Ruby
        "rb" | "rake" | "gemspec" => Some("ruby"),
        // PHP
        "php" => Some("php"),
        // Swift
        "swift" => Some("swift"),
        // Dart/Flutter
        "dart" => Some("dart"),
        // Shell
        "sh" | "bash" | "zsh" | "fish" => Some("shell"),
        // Web
        "html" | "htm" => Some("html"),
        "css" | "scss" | "sass" | "less" => Some("css"),
        // Data/Config
        "json" | "jsonc" | "json5" => Some("json"),
        "yaml" | "yml" => Some("yaml"),
        "toml" => Some("toml"),
        "xml" => Some("xml"),
        // API/Schema
        "graphql" | "gql" => Some("graphql"),
        "proto" => Some("protobuf"),
        "prisma" => Some("prisma"),
        // Markdown/Docs
        "md" | "markdown" => Some("markdown"),
        // SQL
        "sql" => Some("sql"),
        // Other languages
        "lua" => Some("lua"),
        "r" => Some("r"),
        "scala" => Some("scala"),
        "clj" | "cljs" | "cljc" => Some("clojure"),
        "ex" | "exs" => Some("elixir"),
        "erl" | "hrl" => Some("erlang"),
        "hs" | "lhs" => Some("haskell"),
        "ml" | "mli" => Some("ocaml"),
        "fs" | "fsi" | "fsx" => Some("fsharp"),
        "vim" => Some("vim"),
        "el" => Some("elisp"),
        "zig" => Some("zig"),
        "nim" => Some("nim"),
        "jl" => Some("julia"),
        "pl" | "pm" => Some("perl"),
        "groovy" | "gradle" => Some("groovy"),
        // Infrastructure
        "dockerfile" => Some("dockerfile"),
        "tf" | "tfvars" => Some("terraform"),
        "hcl" => Some("hcl"),
        _ => None,
    }
}

/// Check if a file is likely binary based on extension.
fn is_binary_extension(ext: &str) -> bool {
    matches!(
        ext.to_lowercase().as_str(),
        // Executables and libraries
        "exe"
            | "dll"
            | "so"
            | "dylib"
            | "a"
            | "o"
            | "obj"
            | "bin"
            // Databases
            | "dat"
            | "db"
            | "sqlite"
            | "sqlite3"
            | "mdb"
            // Images
            | "png"
            | "jpg"
            | "jpeg"
            | "gif"
            | "bmp"
            | "ico"
            | "webp"
            | "svg"
            | "tiff"
            | "tif"
            | "heic"
            | "heif"
            | "avif"
            | "raw"
            | "cr2"
            | "nef"
            // Audio
            | "mp3"
            | "wav"
            | "ogg"
            | "flac"
            | "aac"
            | "m4a"
            | "wma"
            // Video
            | "mp4"
            | "webm"
            | "avi"
            | "mov"
            | "mkv"
            | "flv"
            | "wmv"
            | "m4v"
            // Documents
            | "pdf"
            | "doc"
            | "docx"
            | "xls"
            | "xlsx"
            | "ppt"
            | "pptx"
            | "odt"
            | "ods"
            | "odp"
            // Archives
            | "zip"
            | "tar"
            | "gz"
            | "bz2"
            | "xz"
            | "7z"
            | "rar"
            | "lz4"
            | "zst"
            // Disk images
            | "dmg"
            | "iso"
            // Compiled/bytecode
            | "wasm"
            | "class"
            | "pyc"
            | "pyo"
            | "beam"
            // Fonts
            | "ttf"
            | "otf"
            | "woff"
            | "woff2"
            | "eot"
            // Design files
            | "psd"
            | "ai"
            | "eps"
            | "sketch"
            | "fig"
            | "xd"
            // 3D/CAD
            | "blend"
            | "fbx"
            | "stl"
            | "gltf"
            | "glb"
    )
}

// ============================================================================
// Manifest Parsing
// ============================================================================

/// Parse Cargo.toml for dependencies using the `toml` crate.
///
/// Handles:
/// - Simple versions: `serde = "1.0"`
/// - Inline tables: `tokio = { version = "1.0", features = ["full"] }`
/// - Workspace dependencies: `serde.workspace = true` (marked as version "*")
/// - Path dependencies: `my-lib = { path = "../lib" }` (included with version "*")
/// - Git dependencies: `foo = { git = "https://..." }` (included with version "*")
fn parse_cargo_toml(path: &Path, manifest_rel: &str) -> Vec<StackDependencyEntry> {
    let content = match fs::read_to_string(path) {
        Ok(c) => c,
        Err(e) => {
            tracing::debug!("Failed to read Cargo.toml at {}: {}", path.display(), e);
            return Vec::new();
        }
    };

    let parsed: toml::Value = match toml::from_str(&content) {
        Ok(v) => v,
        Err(e) => {
            tracing::debug!("Failed to parse Cargo.toml at {}: {}", path.display(), e);
            return Vec::new();
        }
    };

    let mut deps = Vec::new();

    // Parse each dependency section
    let sections = [
        ("dependencies", "runtime"),
        ("dev-dependencies", "dev"),
        ("build-dependencies", "build"),
    ];

    for (section_name, scope) in sections {
        if let Some(section) = parsed.get(section_name).and_then(|v| v.as_table()) {
            for (name, value) in section {
                let version = extract_cargo_version(value);
                deps.push(StackDependencyEntry {
                    manager: "cargo".to_string(),
                    name: name.clone(),
                    version,
                    scope: scope.to_string(),
                    manifest_path: manifest_rel.to_string(),
                });
            }
        }
    }

    // Also parse target-specific dependencies: [target.'cfg(...)'.dependencies]
    if let Some(targets) = parsed.get("target").and_then(|v| v.as_table()) {
        for (_target_spec, target_table) in targets {
            if let Some(target_table) = target_table.as_table() {
                for (section_name, scope) in &sections {
                    if let Some(section) = target_table.get(*section_name).and_then(|v| v.as_table())
                    {
                        for (name, value) in section {
                            let version = extract_cargo_version(value);
                            // Avoid duplicates from main sections
                            if !deps.iter().any(|d| d.name == *name && d.scope == *scope) {
                                deps.push(StackDependencyEntry {
                                    manager: "cargo".to_string(),
                                    name: name.clone(),
                                    version,
                                    scope: scope.to_string(),
                                    manifest_path: manifest_rel.to_string(),
                                });
                            }
                        }
                    }
                }
            }
        }
    }

    deps
}

/// Extract version string from a Cargo dependency value.
///
/// Handles:
/// - String: `"1.0"` -> `"1.0"`
/// - Table with version: `{ version = "1.0" }` -> `"1.0"`
/// - Workspace: `{ workspace = true }` -> `"*"`
/// - Path/git only: `{ path = "..." }` -> `"*"`
fn extract_cargo_version(value: &toml::Value) -> String {
    match value {
        toml::Value::String(s) => s.clone(),
        toml::Value::Table(t) => {
            // Check for explicit version first
            if let Some(v) = t.get("version").and_then(|v| v.as_str()) {
                return v.to_string();
            }
            // Workspace, path, or git dependency without explicit version
            "*".to_string()
        }
        _ => "*".to_string(),
    }
}

/// Parse package.json for dependencies.
///
/// Handles:
/// - dependencies (runtime)
/// - devDependencies (dev)
/// - peerDependencies (peer)
/// - optionalDependencies (optional)
fn parse_package_json(path: &Path, manifest_rel: &str, manager: &str) -> Vec<StackDependencyEntry> {
    let content = match fs::read_to_string(path) {
        Ok(c) => c,
        Err(e) => {
            tracing::debug!("Failed to read package.json at {}: {}", path.display(), e);
            return Vec::new();
        }
    };

    let json: serde_json::Value = match serde_json::from_str(&content) {
        Ok(v) => v,
        Err(e) => {
            tracing::debug!("Failed to parse package.json at {}: {}", path.display(), e);
            return Vec::new();
        }
    };

    let mut deps = Vec::new();

    let sections = [
        ("dependencies", "runtime"),
        ("devDependencies", "dev"),
        ("peerDependencies", "peer"),
        ("optionalDependencies", "optional"),
    ];

    for (section_name, scope) in sections {
        if let Some(section) = json.get(section_name).and_then(|v| v.as_object()) {
            for (name, version) in section {
                deps.push(StackDependencyEntry {
                    manager: manager.to_string(),
                    name: name.clone(),
                    version: version.as_str().unwrap_or("*").to_string(),
                    scope: scope.to_string(),
                    manifest_path: manifest_rel.to_string(),
                });
            }
        }
    }

    deps
}

/// Parse requirements.txt for Python dependencies.
///
/// Handles:
/// - Simple: `requests`
/// - Versioned: `requests==2.28.0`, `flask>=2.0,<3.0`
/// - Extras: `requests[security]>=2.0`
/// - Comments: lines starting with `#`
/// - Skips: `-r`, `-e`, `--index-url`, etc.
fn parse_requirements_txt(path: &Path, manifest_rel: &str, manager: &str) -> Vec<StackDependencyEntry> {
    let content = match fs::read_to_string(path) {
        Ok(c) => c,
        Err(e) => {
            tracing::debug!("Failed to read requirements.txt at {}: {}", path.display(), e);
            return Vec::new();
        }
    };

    let mut deps = Vec::new();

    // Regex to parse package specs: name[extras]version_spec
    // Examples: requests, requests==2.0, flask>=1.0,<2.0, django[argon2]>=3.2
    let re = regex::Regex::new(r"^([a-zA-Z0-9_-]+)(?:\[[^\]]*\])?(.*)?$").unwrap();

    for line in content.lines() {
        let line = line.trim();

        // Skip empty lines and comments
        if line.is_empty() || line.starts_with('#') {
            continue;
        }

        // Skip options and references
        if line.starts_with('-') || line.starts_with("--") {
            continue;
        }

        // Skip URLs and editable installs
        if line.contains("://") || line.starts_with('.') {
            continue;
        }

        if let Some(caps) = re.captures(line) {
            let name = caps.get(1).map(|m| m.as_str()).unwrap_or("");
            let version = caps.get(2).map(|m| m.as_str().trim()).unwrap_or("*");

            if !name.is_empty() {
                deps.push(StackDependencyEntry {
                    manager: manager.to_string(),
                    name: name.to_string(),
                    version: if version.is_empty() {
                        "*".to_string()
                    } else {
                        version.to_string()
                    },
                    scope: "runtime".to_string(),
                    manifest_path: manifest_rel.to_string(),
                });
            }
        }
    }

    deps
}

/// Parse pyproject.toml for Python dependencies.
///
/// Handles:
/// - PEP 621: `[project].dependencies` and `[project.optional-dependencies]`
/// - Poetry: `[tool.poetry.dependencies]` and `[tool.poetry.dev-dependencies]`
/// - PDM: `[tool.pdm.dependencies]` and `[tool.pdm.dev-dependencies]`
fn parse_pyproject_toml(path: &Path, manifest_rel: &str, manager: &str) -> Vec<StackDependencyEntry> {
    let content = match fs::read_to_string(path) {
        Ok(c) => c,
        Err(e) => {
            tracing::debug!("Failed to read pyproject.toml at {}: {}", path.display(), e);
            return Vec::new();
        }
    };

    let parsed: toml::Value = match toml::from_str(&content) {
        Ok(v) => v,
        Err(e) => {
            tracing::debug!("Failed to parse pyproject.toml at {}: {}", path.display(), e);
            return Vec::new();
        }
    };

    let mut deps = Vec::new();

    // PEP 621: [project].dependencies (array of PEP 508 strings)
    if let Some(project_deps) = parsed
        .get("project")
        .and_then(|p| p.get("dependencies"))
        .and_then(|d| d.as_array())
    {
        for dep in project_deps {
            if let Some(dep_str) = dep.as_str() {
                if let Some((name, version)) = parse_pep508_spec(dep_str) {
                    deps.push(StackDependencyEntry {
                        manager: manager.to_string(),
                        name,
                        version,
                        scope: "runtime".to_string(),
                        manifest_path: manifest_rel.to_string(),
                    });
                }
            }
        }
    }

    // PEP 621: [project.optional-dependencies] (table of arrays)
    if let Some(opt_deps) = parsed
        .get("project")
        .and_then(|p| p.get("optional-dependencies"))
        .and_then(|d| d.as_table())
    {
        for (_group, group_deps) in opt_deps {
            if let Some(group_arr) = group_deps.as_array() {
                for dep in group_arr {
                    if let Some(dep_str) = dep.as_str() {
                        if let Some((name, version)) = parse_pep508_spec(dep_str) {
                            // Avoid duplicates
                            if !deps.iter().any(|d| d.name == name) {
                                deps.push(StackDependencyEntry {
                                    manager: manager.to_string(),
                                    name,
                                    version,
                                    scope: "optional".to_string(),
                                    manifest_path: manifest_rel.to_string(),
                                });
                            }
                        }
                    }
                }
            }
        }
    }

    // Poetry: [tool.poetry.dependencies] (table)
    if let Some(poetry_deps) = parsed
        .get("tool")
        .and_then(|t| t.get("poetry"))
        .and_then(|p| p.get("dependencies"))
        .and_then(|d| d.as_table())
    {
        for (name, value) in poetry_deps {
            // Skip python version constraint
            if name == "python" {
                continue;
            }
            let version = extract_poetry_version(value);
            if !deps.iter().any(|d| d.name == *name) {
                deps.push(StackDependencyEntry {
                    manager: manager.to_string(),
                    name: name.clone(),
                    version,
                    scope: "runtime".to_string(),
                    manifest_path: manifest_rel.to_string(),
                });
            }
        }
    }

    // Poetry: [tool.poetry.dev-dependencies] or [tool.poetry.group.dev.dependencies]
    if let Some(poetry_dev) = parsed
        .get("tool")
        .and_then(|t| t.get("poetry"))
        .and_then(|p| p.get("dev-dependencies"))
        .and_then(|d| d.as_table())
    {
        for (name, value) in poetry_dev {
            let version = extract_poetry_version(value);
            if !deps.iter().any(|d| d.name == *name) {
                deps.push(StackDependencyEntry {
                    manager: manager.to_string(),
                    name: name.clone(),
                    version,
                    scope: "dev".to_string(),
                    manifest_path: manifest_rel.to_string(),
                });
            }
        }
    }

    deps
}

/// Extract version from Poetry dependency value.
fn extract_poetry_version(value: &toml::Value) -> String {
    match value {
        toml::Value::String(s) => s.clone(),
        toml::Value::Table(t) => {
            if let Some(v) = t.get("version").and_then(|v| v.as_str()) {
                return v.to_string();
            }
            "*".to_string()
        }
        _ => "*".to_string(),
    }
}

/// Parse a PEP 508 dependency specification.
///
/// Examples:
/// - `requests` -> ("requests", "*")
/// - `requests>=2.0` -> ("requests", ">=2.0")
/// - `flask[async]>=2.0,<3.0` -> ("flask", ">=2.0,<3.0")
/// - `django>=3.2; python_version>="3.8"` -> ("django", ">=3.2")
fn parse_pep508_spec(spec: &str) -> Option<(String, String)> {
    let spec = spec.trim();
    if spec.is_empty() {
        return None;
    }

    // Remove environment markers (everything after `;`)
    let spec = spec.split(';').next().unwrap_or(spec).trim();

    // Regex to extract name and version: name[extras]version_constraints
    let re = regex::Regex::new(r"^([a-zA-Z0-9_-]+)(?:\[[^\]]*\])?\s*(.*)$").ok()?;

    if let Some(caps) = re.captures(spec) {
        let name = caps.get(1)?.as_str().to_string();
        let version = caps
            .get(2)
            .map(|m| m.as_str().trim())
            .filter(|s| !s.is_empty())
            .unwrap_or("*")
            .to_string();
        return Some((name, version));
    }

    None
}

/// Parse composer.json for PHP dependencies.
fn parse_composer_json(path: &Path, manifest_rel: &str) -> Vec<StackDependencyEntry> {
    let content = match fs::read_to_string(path) {
        Ok(c) => c,
        Err(e) => {
            tracing::debug!("Failed to read composer.json at {}: {}", path.display(), e);
            return Vec::new();
        }
    };

    let json: serde_json::Value = match serde_json::from_str(&content) {
        Ok(v) => v,
        Err(e) => {
            tracing::debug!("Failed to parse composer.json at {}: {}", path.display(), e);
            return Vec::new();
        }
    };

    let mut deps = Vec::new();

    // Parse require (runtime)
    if let Some(require) = json.get("require").and_then(|v| v.as_object()) {
        for (name, version) in require {
            // Skip PHP and extension requirements
            if name == "php" || name.starts_with("ext-") {
                continue;
            }
            deps.push(StackDependencyEntry {
                manager: "composer".to_string(),
                name: name.clone(),
                version: version.as_str().unwrap_or("*").to_string(),
                scope: "runtime".to_string(),
                manifest_path: manifest_rel.to_string(),
            });
        }
    }

    // Parse require-dev
    if let Some(require_dev) = json.get("require-dev").and_then(|v| v.as_object()) {
        for (name, version) in require_dev {
            deps.push(StackDependencyEntry {
                manager: "composer".to_string(),
                name: name.clone(),
                version: version.as_str().unwrap_or("*").to_string(),
                scope: "dev".to_string(),
                manifest_path: manifest_rel.to_string(),
            });
        }
    }

    deps
}

/// Parse go.mod for Go dependencies.
fn parse_go_mod(path: &Path, manifest_rel: &str) -> Vec<StackDependencyEntry> {
    let content = match fs::read_to_string(path) {
        Ok(c) => c,
        Err(e) => {
            tracing::debug!("Failed to read go.mod at {}: {}", path.display(), e);
            return Vec::new();
        }
    };

    let mut deps = Vec::new();
    let mut in_require_block = false;

    // Regex for require lines: module/path version [// indirect]
    let require_re =
        regex::Regex::new(r"^\s*([^\s]+)\s+v?([^\s]+)(?:\s*//\s*indirect)?$").unwrap();
    let single_require_re =
        regex::Regex::new(r"^\s*require\s+([^\s]+)\s+v?([^\s]+)(?:\s*//\s*indirect)?$").unwrap();

    for line in content.lines() {
        let line = line.trim();

        // Skip empty lines and comments
        if line.is_empty() || line.starts_with("//") {
            continue;
        }

        // Handle single-line require
        if let Some(caps) = single_require_re.captures(line) {
            let name = caps.get(1).map(|m| m.as_str()).unwrap_or("");
            let version = caps.get(2).map(|m| m.as_str()).unwrap_or("*");
            if !name.is_empty() {
                deps.push(StackDependencyEntry {
                    manager: "go".to_string(),
                    name: name.to_string(),
                    version: version.to_string(),
                    scope: "runtime".to_string(),
                    manifest_path: manifest_rel.to_string(),
                });
            }
            continue;
        }

        // Start of require block
        if line == "require (" {
            in_require_block = true;
            continue;
        }

        // End of require block
        if line == ")" {
            in_require_block = false;
            continue;
        }

        // Inside require block
        if in_require_block {
            if let Some(caps) = require_re.captures(line) {
                let name = caps.get(1).map(|m| m.as_str()).unwrap_or("");
                let version = caps.get(2).map(|m| m.as_str()).unwrap_or("*");
                if !name.is_empty() {
                    deps.push(StackDependencyEntry {
                        manager: "go".to_string(),
                        name: name.to_string(),
                        version: version.to_string(),
                        scope: "runtime".to_string(),
                        manifest_path: manifest_rel.to_string(),
                    });
                }
            }
        }
    }

    deps
}

/// Parse Gemfile for Ruby dependencies.
fn parse_gemfile(path: &Path, manifest_rel: &str) -> Vec<StackDependencyEntry> {
    let content = match fs::read_to_string(path) {
        Ok(c) => c,
        Err(e) => {
            tracing::debug!("Failed to read Gemfile at {}: {}", path.display(), e);
            return Vec::new();
        }
    };

    let mut deps = Vec::new();
    let mut current_scope = "runtime";

    // Regex for gem lines: gem 'name', 'version' or gem "name", "~> 1.0"
    let gem_re = regex::Regex::new(r#"^\s*gem\s+['"]([^'"]+)['"](?:\s*,\s*['"]([^'"]+)['"])?"#).unwrap();
    // Group detection: group :development do / group :test, :development do
    let group_start_re = regex::Regex::new(r"^\s*group\s+:?([\w,\s:]+)\s+do").unwrap();

    for line in content.lines() {
        let line = line.trim();

        // Skip empty lines and comments
        if line.is_empty() || line.starts_with('#') {
            continue;
        }

        // Detect group blocks
        if let Some(caps) = group_start_re.captures(line) {
            let groups = caps.get(1).map(|m| m.as_str()).unwrap_or("");
            if groups.contains("development") || groups.contains("test") {
                current_scope = "dev";
            }
            continue;
        }

        // End of group block
        if line == "end" {
            current_scope = "runtime";
            continue;
        }

        // Parse gem line
        if let Some(caps) = gem_re.captures(line) {
            let name = caps.get(1).map(|m| m.as_str()).unwrap_or("");
            let version = caps.get(2).map(|m| m.as_str()).unwrap_or("*");
            if !name.is_empty() {
                deps.push(StackDependencyEntry {
                    manager: "bundler".to_string(),
                    name: name.to_string(),
                    version: version.to_string(),
                    scope: current_scope.to_string(),
                    manifest_path: manifest_rel.to_string(),
                });
            }
        }
    }

    deps
}

/// Parse Pipfile for Python dependencies.
fn parse_pipfile(path: &Path, manifest_rel: &str, manager: &str) -> Vec<StackDependencyEntry> {
    let content = match fs::read_to_string(path) {
        Ok(c) => c,
        Err(e) => {
            tracing::debug!("Failed to read Pipfile at {}: {}", path.display(), e);
            return Vec::new();
        }
    };

    let parsed: toml::Value = match toml::from_str(&content) {
        Ok(v) => v,
        Err(e) => {
            tracing::debug!("Failed to parse Pipfile at {}: {}", path.display(), e);
            return Vec::new();
        }
    };

    let mut deps = Vec::new();

    // [packages] section
    if let Some(packages) = parsed.get("packages").and_then(|p| p.as_table()) {
        for (name, value) in packages {
            let version = extract_pipfile_version(value);
            deps.push(StackDependencyEntry {
                manager: manager.to_string(),
                name: name.clone(),
                version,
                scope: "runtime".to_string(),
                manifest_path: manifest_rel.to_string(),
            });
        }
    }

    // [dev-packages] section
    if let Some(dev_packages) = parsed.get("dev-packages").and_then(|p| p.as_table()) {
        for (name, value) in dev_packages {
            let version = extract_pipfile_version(value);
            deps.push(StackDependencyEntry {
                manager: manager.to_string(),
                name: name.clone(),
                version,
                scope: "dev".to_string(),
                manifest_path: manifest_rel.to_string(),
            });
        }
    }

    deps
}

/// Extract version from Pipfile dependency value.
fn extract_pipfile_version(value: &toml::Value) -> String {
    match value {
        toml::Value::String(s) => {
            if s == "*" {
                "*".to_string()
            } else {
                s.clone()
            }
        }
        toml::Value::Table(t) => {
            if let Some(v) = t.get("version").and_then(|v| v.as_str()) {
                return v.to_string();
            }
            "*".to_string()
        }
        _ => "*".to_string(),
    }
}

// ============================================================================
// Tech Detection
// ============================================================================

/// Infer technology tags from dependencies.
fn infer_tech_from_dependencies(dependencies: &[StackDependencyEntry]) -> Vec<StackTechEntry> {
    let mut tags: Vec<StackTechEntry> = Vec::new();

    for dep in dependencies {
        // Normalize JavaScript package managers to a common check
        let is_js_manager = matches!(
            dep.manager.as_str(),
            "npm" | "pnpm" | "yarn" | "bun"
        );
        let is_python_manager = matches!(
            dep.manager.as_str(),
            "poetry" | "pipenv" | "pdm" | "uv" | "pip"
        );

        let tag = if is_js_manager {
            infer_js_tech_tag(&dep.name)
        } else if is_python_manager {
            infer_python_tech_tag(&dep.name)
        } else {
            match dep.manager.as_str() {
                "cargo" => infer_rust_tech_tag(&dep.name),
                "go" => infer_go_tech_tag(&dep.name),
                "bundler" => infer_ruby_tech_tag(&dep.name),
                "composer" => infer_php_tech_tag(&dep.name),
                _ => None,
            }
        };

        if let Some((kind, name)) = tag {
            // Avoid duplicates
            if !tags.iter().any(|t| t.name == name) {
                tags.push(StackTechEntry {
                    kind: kind.to_string(),
                    name: name.to_string(),
                    source: format!("dependency:{}", dep.name),
                    confidence: 0.9,
                });
            }
        }
    }

    tags
}

/// Infer tech tag from JavaScript/TypeScript dependency.
fn infer_js_tech_tag(name: &str) -> Option<(&'static str, &'static str)> {
    match name {
        // Frameworks - Full-stack/SSR
        "next" => Some(("framework", "Next.js")),
        "nuxt" | "nuxt3" => Some(("framework", "Nuxt")),
        "@remix-run/react" | "remix" => Some(("framework", "Remix")),
        "astro" => Some(("framework", "Astro")),
        "@sveltejs/kit" => Some(("framework", "SvelteKit")),
        "gatsby" => Some(("framework", "Gatsby")),
        "@redwoodjs/core" => Some(("framework", "RedwoodJS")),
        
        // Frameworks - Frontend
        "react" => Some(("framework", "React")),
        "vue" => Some(("framework", "Vue.js")),
        "angular" | "@angular/core" => Some(("framework", "Angular")),
        "svelte" => Some(("framework", "Svelte")),
        "solid-js" => Some(("framework", "SolidJS")),
        "preact" => Some(("framework", "Preact")),
        "@builder.io/qwik" => Some(("framework", "Qwik")),
        "lit" | "lit-element" => Some(("framework", "Lit")),
        
        // Frameworks - Backend
        "express" => Some(("framework", "Express")),
        "fastify" => Some(("framework", "Fastify")),
        "@nestjs/core" | "nest" => Some(("framework", "NestJS")),
        "hono" => Some(("framework", "Hono")),
        "koa" => Some(("framework", "Koa")),
        "@hapi/hapi" => Some(("framework", "Hapi")),
        
        // Testing
        "jest" => Some(("tool", "Jest")),
        "vitest" => Some(("tool", "Vitest")),
        "mocha" => Some(("tool", "Mocha")),
        "playwright" | "@playwright/test" => Some(("tool", "Playwright")),
        "cypress" => Some(("tool", "Cypress")),
        "@testing-library/react" => Some(("tool", "Testing Library")),
        
        // Build tools
        "vite" => Some(("tool", "Vite")),
        "webpack" => Some(("tool", "Webpack")),
        "esbuild" => Some(("tool", "esbuild")),
        "turbo" => Some(("tool", "Turborepo")),
        "tsup" => Some(("tool", "tsup")),
        "rollup" => Some(("tool", "Rollup")),
        "parcel" => Some(("tool", "Parcel")),
        
        // State management
        "zustand" => Some(("tool", "Zustand")),
        "jotai" => Some(("tool", "Jotai")),
        "redux" | "@reduxjs/toolkit" => Some(("tool", "Redux")),
        "mobx" => Some(("tool", "MobX")),
        "recoil" => Some(("tool", "Recoil")),
        "@tanstack/react-query" | "react-query" => Some(("tool", "TanStack Query")),
        "swr" => Some(("tool", "SWR")),
        
        // Styling
        "tailwindcss" => Some(("tool", "Tailwind CSS")),
        "styled-components" => Some(("tool", "Styled Components")),
        "@emotion/react" | "@emotion/styled" => Some(("tool", "Emotion")),
        "sass" => Some(("tool", "Sass")),
        "@chakra-ui/react" => Some(("tool", "Chakra UI")),
        "@mui/material" => Some(("tool", "Material UI")),
        "antd" => Some(("tool", "Ant Design")),
        
        // Databases/ORMs
        "prisma" | "@prisma/client" => Some(("tool", "Prisma")),
        "drizzle-orm" => Some(("tool", "Drizzle")),
        "typeorm" => Some(("tool", "TypeORM")),
        "sequelize" => Some(("tool", "Sequelize")),
        "knex" => Some(("tool", "Knex")),
        "mongoose" => Some(("tool", "Mongoose")),
        "kysely" => Some(("tool", "Kysely")),
        
        // API/GraphQL
        "@trpc/server" | "@trpc/client" => Some(("tool", "tRPC")),
        "graphql" => Some(("tool", "GraphQL")),
        "@apollo/server" | "@apollo/client" => Some(("tool", "Apollo")),
        "urql" => Some(("tool", "URQL")),
        
        // Auth
        "next-auth" | "@auth/core" => Some(("tool", "Auth.js")),
        "passport" => Some(("tool", "Passport")),
        "@clerk/nextjs" => Some(("tool", "Clerk")),
        
        // Backend services
        "@supabase/supabase-js" => Some(("service", "Supabase")),
        "firebase" | "firebase-admin" => Some(("service", "Firebase")),
        "@aws-sdk/client-s3" => Some(("service", "AWS S3")),
        "stripe" => Some(("service", "Stripe")),
        
        // Real-time
        "socket.io" | "socket.io-client" => Some(("tool", "Socket.IO")),
        "ws" => Some(("tool", "WebSocket")),
        
        // Monorepo
        "nx" => Some(("tool", "Nx")),
        "lerna" => Some(("tool", "Lerna")),
        
        // Documentation
        "storybook" | "@storybook/react" => Some(("tool", "Storybook")),
        "typedoc" => Some(("tool", "TypeDoc")),
        
        _ => None,
    }
}

/// Infer tech tag from Python dependency.
fn infer_python_tech_tag(name: &str) -> Option<(&'static str, &'static str)> {
    match name {
        // Web frameworks
        "django" => Some(("framework", "Django")),
        "flask" => Some(("framework", "Flask")),
        "fastapi" => Some(("framework", "FastAPI")),
        "starlette" => Some(("framework", "Starlette")),
        "litestar" => Some(("framework", "Litestar")),
        "tornado" => Some(("framework", "Tornado")),
        "aiohttp" => Some(("framework", "aiohttp")),
        "sanic" => Some(("framework", "Sanic")),
        "pyramid" => Some(("framework", "Pyramid")),
        "bottle" => Some(("framework", "Bottle")),
        "quart" => Some(("framework", "Quart")),
        
        // Task queues
        "celery" => Some(("tool", "Celery")),
        "rq" => Some(("tool", "RQ")),
        "dramatiq" => Some(("tool", "Dramatiq")),
        
        // ORMs/Database
        "sqlalchemy" => Some(("tool", "SQLAlchemy")),
        "alembic" => Some(("tool", "Alembic")),
        "peewee" => Some(("tool", "Peewee")),
        "tortoise-orm" => Some(("tool", "Tortoise ORM")),
        "databases" => Some(("tool", "Databases")),
        "psycopg2" | "psycopg" => Some(("tool", "Psycopg")),
        "pymongo" => Some(("tool", "PyMongo")),
        "redis" => Some(("tool", "Redis-Py")),
        
        // Validation/Serialization
        "pydantic" => Some(("tool", "Pydantic")),
        "marshmallow" => Some(("tool", "Marshmallow")),
        "attrs" => Some(("tool", "attrs")),
        
        // Testing
        "pytest" => Some(("tool", "pytest")),
        "unittest" => Some(("tool", "unittest")),
        "hypothesis" => Some(("tool", "Hypothesis")),
        "coverage" => Some(("tool", "Coverage.py")),
        
        // Data science
        "pandas" => Some(("tool", "pandas")),
        "numpy" => Some(("tool", "NumPy")),
        "scipy" => Some(("tool", "SciPy")),
        "matplotlib" => Some(("tool", "Matplotlib")),
        "seaborn" => Some(("tool", "Seaborn")),
        "plotly" => Some(("tool", "Plotly")),
        "polars" => Some(("tool", "Polars")),
        
        // ML/AI
        "tensorflow" | "tensorflow-gpu" => Some(("tool", "TensorFlow")),
        "torch" | "pytorch" => Some(("tool", "PyTorch")),
        "scikit-learn" | "sklearn" => Some(("tool", "scikit-learn")),
        "keras" => Some(("tool", "Keras")),
        "transformers" => Some(("tool", "Transformers")),
        "langchain" | "langchain-core" => Some(("tool", "LangChain")),
        "openai" => Some(("service", "OpenAI")),
        "anthropic" => Some(("service", "Anthropic")),
        "huggingface-hub" => Some(("tool", "Hugging Face")),
        
        // HTTP/API
        "requests" => Some(("tool", "Requests")),
        "httpx" => Some(("tool", "HTTPX")),
        
        // CLI
        "click" => Some(("tool", "Click")),
        "typer" => Some(("tool", "Typer")),
        "rich" => Some(("tool", "Rich")),
        
        _ => None,
    }
}

/// Infer tech tag from Rust dependency.
fn infer_rust_tech_tag(name: &str) -> Option<(&'static str, &'static str)> {
    match name {
        // Web frameworks
        "actix-web" => Some(("framework", "Actix Web")),
        "axum" => Some(("framework", "Axum")),
        "rocket" => Some(("framework", "Rocket")),
        "warp" => Some(("framework", "Warp")),
        "poem" => Some(("framework", "Poem")),
        "tide" => Some(("framework", "Tide")),
        
        // Async runtime
        "tokio" => Some(("tool", "Tokio")),
        "async-std" => Some(("tool", "async-std")),
        
        // Serialization
        "serde" => Some(("tool", "Serde")),
        "serde_json" => Some(("tool", "serde_json")),
        
        // Database
        "diesel" => Some(("tool", "Diesel")),
        "sqlx" => Some(("tool", "SQLx")),
        "sea-orm" => Some(("tool", "SeaORM")),
        "rusqlite" => Some(("tool", "rusqlite")),
        "mongodb" => Some(("tool", "MongoDB Rust")),
        "redis" => Some(("tool", "redis-rs")),
        
        // CLI
        "clap" => Some(("tool", "clap")),
        "structopt" => Some(("tool", "StructOpt")),
        
        // HTTP client
        "reqwest" => Some(("tool", "reqwest")),
        "hyper" => Some(("tool", "Hyper")),
        
        // Templating
        "askama" => Some(("tool", "Askama")),
        "tera" => Some(("tool", "Tera")),
        "handlebars" => Some(("tool", "Handlebars")),
        
        // Testing
        "criterion" => Some(("tool", "Criterion")),
        "proptest" => Some(("tool", "proptest")),
        
        // Tracing/Logging
        "tracing" => Some(("tool", "tracing")),
        "log" => Some(("tool", "log")),
        
        _ => None,
    }
}

/// Infer tech tag from Go dependency.
fn infer_go_tech_tag(name: &str) -> Option<(&'static str, &'static str)> {
    // Go module paths can be long, check contains patterns
    if name.contains("github.com/gin-gonic/gin") {
        return Some(("framework", "Gin"));
    }
    if name.contains("github.com/gofiber/fiber") {
        return Some(("framework", "Fiber"));
    }
    if name.contains("github.com/labstack/echo") {
        return Some(("framework", "Echo"));
    }
    if name.contains("github.com/gorilla/mux") {
        return Some(("framework", "Gorilla Mux"));
    }
    if name.contains("github.com/go-chi/chi") {
        return Some(("framework", "Chi"));
    }
    if name.contains("github.com/beego/beego") {
        return Some(("framework", "Beego"));
    }
    if name.contains("github.com/gohugoio/hugo") {
        return Some(("tool", "Hugo"));
    }
    
    // ORMs/Database
    if name.contains("gorm.io/gorm") {
        return Some(("tool", "GORM"));
    }
    if name.contains("github.com/jmoiron/sqlx") {
        return Some(("tool", "sqlx"));
    }
    if name.contains("github.com/go-pg/pg") {
        return Some(("tool", "go-pg"));
    }
    if name.contains("go.mongodb.org/mongo-driver") {
        return Some(("tool", "MongoDB Go"));
    }
    if name.contains("github.com/redis/go-redis") {
        return Some(("tool", "go-redis"));
    }
    
    // Testing
    if name.contains("github.com/stretchr/testify") {
        return Some(("tool", "Testify"));
    }
    if name.contains("github.com/onsi/ginkgo") {
        return Some(("tool", "Ginkgo"));
    }
    
    // CLI
    if name.contains("github.com/spf13/cobra") {
        return Some(("tool", "Cobra"));
    }
    if name.contains("github.com/urfave/cli") {
        return Some(("tool", "urfave/cli"));
    }
    
    // gRPC
    if name.contains("google.golang.org/grpc") {
        return Some(("tool", "gRPC Go"));
    }
    
    // Logging
    if name.contains("go.uber.org/zap") {
        return Some(("tool", "Zap"));
    }
    if name.contains("github.com/sirupsen/logrus") {
        return Some(("tool", "Logrus"));
    }
    
    None
}

/// Infer tech tag from Ruby dependency.
fn infer_ruby_tech_tag(name: &str) -> Option<(&'static str, &'static str)> {
    match name {
        // Web frameworks
        "rails" => Some(("framework", "Ruby on Rails")),
        "sinatra" => Some(("framework", "Sinatra")),
        "hanami" => Some(("framework", "Hanami")),
        "roda" => Some(("framework", "Roda")),
        "grape" => Some(("framework", "Grape")),
        "padrino" => Some(("framework", "Padrino")),
        
        // Database
        "activerecord" => Some(("tool", "Active Record")),
        "sequel" => Some(("tool", "Sequel")),
        "rom-rb" | "rom" => Some(("tool", "ROM")),
        "mongoid" => Some(("tool", "Mongoid")),
        "redis" => Some(("tool", "Redis Ruby")),
        
        // Background jobs
        "sidekiq" => Some(("tool", "Sidekiq")),
        "resque" => Some(("tool", "Resque")),
        "delayed_job" => Some(("tool", "Delayed Job")),
        "good_job" => Some(("tool", "GoodJob")),
        
        // Testing
        "rspec" | "rspec-rails" => Some(("tool", "RSpec")),
        "minitest" => Some(("tool", "Minitest")),
        "capybara" => Some(("tool", "Capybara")),
        "factory_bot" | "factory_bot_rails" => Some(("tool", "FactoryBot")),
        
        // API
        "graphql-ruby" | "graphql" => Some(("tool", "GraphQL Ruby")),
        "jbuilder" => Some(("tool", "Jbuilder")),
        
        // Auth
        "devise" => Some(("tool", "Devise")),
        "omniauth" => Some(("tool", "OmniAuth")),
        
        // Assets
        "webpacker" => Some(("tool", "Webpacker")),
        "sprockets" => Some(("tool", "Sprockets")),
        
        _ => None,
    }
}

/// Infer tech tag from PHP dependency.
fn infer_php_tech_tag(name: &str) -> Option<(&'static str, &'static str)> {
    match name {
        // Web frameworks
        "laravel/framework" => Some(("framework", "Laravel")),
        "symfony/symfony" | "symfony/framework-bundle" => Some(("framework", "Symfony")),
        "slim/slim" => Some(("framework", "Slim")),
        "cakephp/cakephp" => Some(("framework", "CakePHP")),
        "yiisoft/yii2" => Some(("framework", "Yii")),
        "laminas/laminas-mvc" => Some(("framework", "Laminas")),
        "codeigniter4/framework" => Some(("framework", "CodeIgniter")),
        
        // Database/ORM
        "doctrine/orm" => Some(("tool", "Doctrine")),
        "doctrine/dbal" => Some(("tool", "Doctrine DBAL")),
        "illuminate/database" => Some(("tool", "Eloquent")),
        
        // Testing
        "phpunit/phpunit" => Some(("tool", "PHPUnit")),
        "phpspec/phpspec" => Some(("tool", "PHPSpec")),
        "behat/behat" => Some(("tool", "Behat")),
        "pestphp/pest" => Some(("tool", "Pest")),
        
        // Template engines
        "twig/twig" => Some(("tool", "Twig")),
        "laravel/blade" | "jenssegers/blade" => Some(("tool", "Blade")),
        
        // API
        "api-platform/core" => Some(("framework", "API Platform")),
        "league/fractal" => Some(("tool", "Fractal")),
        
        // Auth
        "laravel/passport" => Some(("tool", "Laravel Passport")),
        "laravel/sanctum" => Some(("tool", "Laravel Sanctum")),
        
        // Queue
        "laravel/horizon" => Some(("tool", "Horizon")),
        "php-amqplib/php-amqplib" => Some(("tool", "php-amqplib")),
        
        // Utilities
        "guzzlehttp/guzzle" => Some(("tool", "Guzzle")),
        "monolog/monolog" => Some(("tool", "Monolog")),
        
        _ => None,
    }
}

/// Infer technology tags from file patterns.
///
/// Detects infrastructure and tooling based on specific files in the workspace.
fn infer_tech_from_files(files: &[StackFileEntry]) -> Vec<StackTechEntry> {
    let mut tags = Vec::new();
    let mut found = std::collections::HashSet::new();

    for file in files {
        let path = &file.path;
        let file_name = path.rsplit('/').next().unwrap_or(path);
        let path_lower = path.to_lowercase();

        // Infrastructure - Docker
        if file_name == "docker-compose.yml"
            || file_name == "docker-compose.yaml"
            || file_name == "compose.yml"
            || file_name == "compose.yaml"
        {
            if found.insert("Docker Compose") {
                tags.push(StackTechEntry {
                    kind: "infra".to_string(),
                    name: "Docker Compose".to_string(),
                    source: format!("file:{}", path),
                    confidence: 0.85,
                });
            }
        }

        // Infrastructure - Kubernetes
        if (path_lower.contains("kubernetes/") || path_lower.contains("k8s/"))
            && (path.ends_with(".yaml") || path.ends_with(".yml"))
        {
            if found.insert("Kubernetes") {
                tags.push(StackTechEntry {
                    kind: "infra".to_string(),
                    name: "Kubernetes".to_string(),
                    source: format!("file:{}", path),
                    confidence: 0.85,
                });
            }
        }

        // Infrastructure - Helm
        if file_name == "Chart.yaml" || file_name == "Chart.yml" {
            if found.insert("Helm") {
                tags.push(StackTechEntry {
                    kind: "infra".to_string(),
                    name: "Helm".to_string(),
                    source: format!("file:{}", path),
                    confidence: 0.85,
                });
            }
        }

        // CI/CD - GitHub Actions
        if path_lower.contains(".github/workflows/")
            && (path.ends_with(".yml") || path.ends_with(".yaml"))
        {
            if found.insert("GitHub Actions") {
                tags.push(StackTechEntry {
                    kind: "tool".to_string(),
                    name: "GitHub Actions".to_string(),
                    source: format!("file:{}", path),
                    confidence: 0.85,
                });
            }
        }

        // CI/CD - GitLab CI
        if file_name == ".gitlab-ci.yml" {
            if found.insert("GitLab CI") {
                tags.push(StackTechEntry {
                    kind: "tool".to_string(),
                    name: "GitLab CI".to_string(),
                    source: format!("file:{}", path),
                    confidence: 0.85,
                });
            }
        }

        // CI/CD - CircleCI
        if path_lower.contains(".circleci/config.yml") {
            if found.insert("CircleCI") {
                tags.push(StackTechEntry {
                    kind: "tool".to_string(),
                    name: "CircleCI".to_string(),
                    source: format!("file:{}", path),
                    confidence: 0.85,
                });
            }
        }

        // CI/CD - Jenkins
        if file_name == "Jenkinsfile" {
            if found.insert("Jenkins") {
                tags.push(StackTechEntry {
                    kind: "tool".to_string(),
                    name: "Jenkins".to_string(),
                    source: format!("file:{}", path),
                    confidence: 0.85,
                });
            }
        }

        // Infrastructure - Terraform
        if path.ends_with(".tf") && !found.contains("Terraform") {
            found.insert("Terraform");
            tags.push(StackTechEntry {
                kind: "infra".to_string(),
                name: "Terraform".to_string(),
                source: format!("file:{}", path),
                confidence: 0.85,
            });
        }

        // Infrastructure - Pulumi
        if file_name == "Pulumi.yaml" || file_name == "Pulumi.yml" {
            if found.insert("Pulumi") {
                tags.push(StackTechEntry {
                    kind: "infra".to_string(),
                    name: "Pulumi".to_string(),
                    source: format!("file:{}", path),
                    confidence: 0.85,
                });
            }
        }

        // Infrastructure - Serverless Framework
        if file_name == "serverless.yml" || file_name == "serverless.yaml" {
            if found.insert("Serverless Framework") {
                tags.push(StackTechEntry {
                    kind: "infra".to_string(),
                    name: "Serverless Framework".to_string(),
                    source: format!("file:{}", path),
                    confidence: 0.85,
                });
            }
        }

        // Deployment - Vercel
        if file_name == "vercel.json" {
            if found.insert("Vercel") {
                tags.push(StackTechEntry {
                    kind: "infra".to_string(),
                    name: "Vercel".to_string(),
                    source: format!("file:{}", path),
                    confidence: 0.85,
                });
            }
        }

        // Deployment - Netlify
        if file_name == "netlify.toml" {
            if found.insert("Netlify") {
                tags.push(StackTechEntry {
                    kind: "infra".to_string(),
                    name: "Netlify".to_string(),
                    source: format!("file:{}", path),
                    confidence: 0.85,
                });
            }
        }

        // Deployment - Fly.io
        if file_name == "fly.toml" {
            if found.insert("Fly.io") {
                tags.push(StackTechEntry {
                    kind: "infra".to_string(),
                    name: "Fly.io".to_string(),
                    source: format!("file:{}", path),
                    confidence: 0.85,
                });
            }
        }

        // Build - Make
        if file_name == "Makefile" || file_name == "makefile" || file_name == "GNUmakefile" {
            if found.insert("Make") {
                tags.push(StackTechEntry {
                    kind: "tool".to_string(),
                    name: "Make".to_string(),
                    source: format!("file:{}", path),
                    confidence: 0.85,
                });
            }
        }

        // Build - Just
        if file_name == "justfile" || file_name == "Justfile" || file_name == ".justfile" {
            if found.insert("Just") {
                tags.push(StackTechEntry {
                    kind: "tool".to_string(),
                    name: "Just".to_string(),
                    source: format!("file:{}", path),
                    confidence: 0.85,
                });
            }
        }

        // Build - Task
        if file_name == "Taskfile.yml" || file_name == "Taskfile.yaml" {
            if found.insert("Task") {
                tags.push(StackTechEntry {
                    kind: "tool".to_string(),
                    name: "Task".to_string(),
                    source: format!("file:{}", path),
                    confidence: 0.85,
                });
            }
        }

        // Monorepo - Nx
        if file_name == "nx.json" {
            if found.insert("Nx") {
                tags.push(StackTechEntry {
                    kind: "tool".to_string(),
                    name: "Nx".to_string(),
                    source: format!("file:{}", path),
                    confidence: 0.85,
                });
            }
        }

        // Monorepo - Turborepo
        if file_name == "turbo.json" {
            if found.insert("Turborepo") {
                tags.push(StackTechEntry {
                    kind: "tool".to_string(),
                    name: "Turborepo".to_string(),
                    source: format!("file:{}", path),
                    confidence: 0.85,
                });
            }
        }

        // Monorepo - Lerna
        if file_name == "lerna.json" {
            if found.insert("Lerna") {
                tags.push(StackTechEntry {
                    kind: "tool".to_string(),
                    name: "Lerna".to_string(),
                    source: format!("file:{}", path),
                    confidence: 0.85,
                });
            }
        }
    }

    tags
}

/// Infer technology tags from dependencies and file patterns.
fn infer_tech_tags(
    dependencies: &[StackDependencyEntry],
    languages: &HashMap<String, u64>,
    files: &[StackFileEntry],
) -> Vec<StackTechEntry> {
    let mut tags = Vec::new();

    // Add language tags
    for lang in languages.keys() {
        tags.push(StackTechEntry {
            kind: "language".to_string(),
            name: capitalize(lang),
            source: format!("files:*.{}", lang),
            confidence: 0.9,
        });
    }

    // Add dependency-based tags
    tags.extend(infer_tech_from_dependencies(dependencies));

    // Add file-based tags
    let file_tags = infer_tech_from_files(files);
    for tag in file_tags {
        // Avoid duplicates (file-based detection may overlap with dependency-based)
        if !tags.iter().any(|t| t.name == tag.name) {
            tags.push(tag);
        }
    }

    tags
}

/// Capitalize first letter.
fn capitalize(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        None => String::new(),
        Some(c) => c.to_uppercase().chain(chars).collect(),
    }
}

/// Get sort priority for tech tag kinds.
///
/// Lower number = higher priority (appears first).
/// Order: language (0) < framework (1) < tool (2) < service (3) < infra (4)
fn tech_kind_priority(kind: &str) -> u8 {
    match kind {
        "language" => 0,
        "framework" => 1,
        "tool" => 2,
        "service" => 3,
        "infra" => 4,
        _ => 5, // Unknown kinds come last
    }
}

// ============================================================================
// Parallel Stack Scanning Infrastructure
// ============================================================================

/// Per-thread accumulator for parallel directory walking.
///
/// Each thread in the parallel walker gets its own accumulator instance,
/// which is then merged after the walk completes.
#[derive(Default)]
struct ThreadLocalAccumulator {
    /// Collected file entries.
    files: Vec<StackFileEntry>,
    /// Language counts.
    language_counts: HashMap<String, u64>,
    /// Manifest files discovered (path, relative_path).
    manifest_files: Vec<(PathBuf, String)>,
    /// Detected managers per category: category -> (manager, priority).
    detected_managers: HashMap<ManagerCategory, (String, u8)>,
    /// Total file count.
    total_files: u64,
}

impl ThreadLocalAccumulator {
    fn new() -> Self {
        Self::default()
    }
}

/// Process a single file entry during parallel walking.
///
/// This is extracted to be called from the parallel walker callback.
fn process_file_entry(
    path: &Path,
    rel_path_str: String,
    accumulator: &mut ThreadLocalAccumulator,
) {
    let extension = path.extension().and_then(|e| e.to_str()).unwrap_or("");
    let file_name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");

    // Detect language
    let mut languages = Vec::new();
    if let Some(lang) = extension_to_language(extension) {
        languages.push(lang.to_string());
        *accumulator
            .language_counts
            .entry(lang.to_string())
            .or_insert(0) += 1;
    }

    // Handle Dockerfile
    if file_name == "Dockerfile" || file_name.starts_with("Dockerfile.") {
        languages.push("dockerfile".to_string());
        *accumulator
            .language_counts
            .entry("dockerfile".to_string())
            .or_insert(0) += 1;
    }

    // Detect lockfiles for package manager detection
    if let Some((manager, priority)) = detect_manager_from_lockfile(file_name) {
        if let Some(category) = manager_category(manager) {
            let should_update = match accumulator.detected_managers.get(&category) {
                None => true,
                Some((_, existing_priority)) => priority < *existing_priority,
            };
            if should_update {
                accumulator
                    .detected_managers
                    .insert(category, (manager.to_string(), priority));
            }
        }
    }

    // Collect manifest files for later processing
    match file_name {
        "Cargo.toml" | "package.json" | "pyproject.toml" | "requirements.txt" | "Pipfile"
        | "composer.json" | "go.mod" | "Gemfile" => {
            accumulator
                .manifest_files
                .push((path.to_path_buf(), rel_path_str.clone()));
        }
        _ => {}
    }

    // Skip binary files
    if is_binary_extension(extension) {
        return;
    }

    accumulator.total_files += 1;

    accumulator.files.push(StackFileEntry {
        path: rel_path_str,
        kind: StackFileKind::File,
        languages,
        file_count: None,
    });
}

/// Manifest parsing result for parallel processing.
struct ManifestParseResult {
    dependencies: Vec<StackDependencyEntry>,
    manager: Option<String>,
}

/// Parse a single manifest file and return dependencies.
fn parse_manifest_file(
    manifest_path: &Path,
    rel_path: &str,
    detected_managers: &HashMap<ManagerCategory, (String, u8)>,
) -> ManifestParseResult {
    let file_name = manifest_path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("");

    match file_name {
        "Cargo.toml" => {
            let deps = parse_cargo_toml(manifest_path, rel_path);
            ManifestParseResult {
                manager: if deps.is_empty() {
                    None
                } else {
                    Some("cargo".to_string())
                },
                dependencies: deps,
            }
        }
        "package.json" => {
            let manager = detected_managers
                .get(&ManagerCategory::JavaScript)
                .map(|(m, _)| m.as_str())
                .unwrap_or("npm");

            let deps = parse_package_json(manifest_path, rel_path, manager);
            ManifestParseResult {
                manager: if deps.is_empty() {
                    None
                } else {
                    Some(manager.to_string())
                },
                dependencies: deps,
            }
        }
        "pyproject.toml" => {
            let manager = detected_managers
                .get(&ManagerCategory::Python)
                .map(|(m, _)| m.as_str())
                .unwrap_or("pip");

            let deps = parse_pyproject_toml(manifest_path, rel_path, manager);
            ManifestParseResult {
                manager: if deps.is_empty() {
                    None
                } else {
                    Some(manager.to_string())
                },
                dependencies: deps,
            }
        }
        "requirements.txt" => {
            let manager = detected_managers
                .get(&ManagerCategory::Python)
                .map(|(m, _)| m.as_str())
                .unwrap_or("pip");

            let deps = parse_requirements_txt(manifest_path, rel_path, manager);
            ManifestParseResult {
                manager: if deps.is_empty() {
                    None
                } else {
                    Some(manager.to_string())
                },
                dependencies: deps,
            }
        }
        "Pipfile" => {
            let manager = detected_managers
                .get(&ManagerCategory::Python)
                .map(|(m, _)| m.as_str())
                .unwrap_or("pipenv");

            let deps = parse_pipfile(manifest_path, rel_path, manager);
            ManifestParseResult {
                manager: if deps.is_empty() {
                    None
                } else {
                    Some(manager.to_string())
                },
                dependencies: deps,
            }
        }
        "composer.json" => {
            let deps = parse_composer_json(manifest_path, rel_path);
            ManifestParseResult {
                manager: if deps.is_empty() {
                    None
                } else {
                    Some("composer".to_string())
                },
                dependencies: deps,
            }
        }
        "go.mod" => {
            let deps = parse_go_mod(manifest_path, rel_path);
            ManifestParseResult {
                manager: if deps.is_empty() {
                    None
                } else {
                    Some("go".to_string())
                },
                dependencies: deps,
            }
        }
        "Gemfile" => {
            let deps = parse_gemfile(manifest_path, rel_path);
            ManifestParseResult {
                manager: if deps.is_empty() {
                    None
                } else {
                    Some("bundler".to_string())
                },
                dependencies: deps,
            }
        }
        _ => ManifestParseResult {
            dependencies: Vec::new(),
            manager: None,
        },
    }
}

// ============================================================================
// Stack Scanning
// ============================================================================

/// Scan a workspace and collect stack inventory.
///
/// Uses parallel directory walking via `ignore::WalkParallel` for improved
/// performance on large repositories. Manifest files are also parsed in
/// parallel using `rayon`. The output is sorted by path for deterministic
/// results.
///
/// # Package Manager Detection
///
/// Package managers are detected from lockfiles with the following priority:
/// - JavaScript: pnpm > yarn > bun > npm
/// - Rust: cargo
/// - Python: poetry > pipenv > pdm > uv
/// - PHP: composer
/// - Go: go
/// - Ruby: bundler
///
/// # Arguments
///
/// * `root` - Path to the workspace root.
///
/// # Returns
///
/// A complete `StackInventory` with files, dependencies, tech tags, and stats.
///
/// # Errors
///
/// Returns `GikError::StackScanFailed` if scanning fails.
pub fn scan_stack(root: &Path) -> Result<StackInventory, GikError> {
    // Shared accumulator protected by mutex for parallel walking
    let shared_accumulator = Mutex::new(ThreadLocalAccumulator::new());

    // Capture root as owned PathBuf for thread safety
    let root_path = root.to_path_buf();

    // Build parallel walker with ignore support
    let walker = WalkBuilder::new(&root_path)
        .hidden(true) // Skip hidden files by default
        .git_ignore(true) // Respect .gitignore
        .git_global(true) // Respect global gitignore
        .git_exclude(true) // Respect .git/info/exclude
        .add_custom_ignore_filename(GIK_IGNORE_FILENAME) // Custom GIK ignore
        .follow_links(false) // Don't follow symlinks/junction points (prevents accessing protected system dirs on Windows)
        .threads(0) // Use all available threads (0 = auto-detect)
        .filter_entry(|entry| {
            // Always skip these directories
            let name = entry.file_name().to_string_lossy();
            !should_ignore_dir(&name)
        })
        .build_parallel();

    // Run parallel walk - each entry is processed and immediately added to shared accumulator
    walker.run(|| {
        let root_clone = root_path.clone();
        let shared_ref = &shared_accumulator;

        Box::new(move |result| {
            let entry = match result {
                Ok(entry) => entry,
                Err(e) => {
                    // Log warning but continue on errors
                    eprintln!("Warning: Error walking directory: {}", e);
                    return WalkState::Continue;
                }
            };

            let path = entry.path();

            // Skip the root directory itself
            if path == root_clone.as_path() {
                return WalkState::Continue;
            }

            // Get relative path
            let rel_path = match path.strip_prefix(&root_clone) {
                Ok(p) => p,
                Err(_) => return WalkState::Continue,
            };

            let rel_path_str = rel_path.to_string_lossy().to_string();

            // Skip directories
            if path.is_dir() {
                return WalkState::Continue;
            }

            // Process file entry directly into shared accumulator
            if let Ok(mut guard) = shared_ref.lock() {
                process_file_entry(path, rel_path_str, &mut guard);
            }

            WalkState::Continue
        })
    });

    // Get the accumulated results
    let mut accumulator = shared_accumulator
        .into_inner()
        .map_err(|e| GikError::StackScanFailed(format!("Lock poisoned: {}", e)))?;

    // Sort files by path for deterministic output
    accumulator.files.sort_by(|a, b| a.path.cmp(&b.path));

    // Sort manifest files by path for deterministic processing order
    accumulator.manifest_files.sort_by(|a, b| a.0.cmp(&b.0));

    // Parse manifest files in parallel using rayon
    let parse_results: Vec<ManifestParseResult> = accumulator
        .manifest_files
        .par_iter()
        .map(|(path, rel_path)| parse_manifest_file(path, rel_path, &accumulator.detected_managers))
        .collect();

    // Collect dependencies and managers from parse results
    let mut dependencies = Vec::new();
    let mut managers_set: HashSet<String> = HashSet::new();

    for result in parse_results {
        dependencies.extend(result.dependencies);
        if let Some(manager) = result.manager {
            managers_set.insert(manager);
        }
    }

    // Sort dependencies by (manager, name) for deterministic output
    dependencies.sort_by(|a, b| {
        match a.manager.cmp(&b.manager) {
            std::cmp::Ordering::Equal => a.name.cmp(&b.name),
            other => other,
        }
    });

    // Infer tech tags
    let mut tech = infer_tech_tags(&dependencies, &accumulator.language_counts, &accumulator.files);

    // Sort tech tags by (kind_priority, name) for deterministic output
    tech.sort_by(|a, b| {
        let priority_a = tech_kind_priority(&a.kind);
        let priority_b = tech_kind_priority(&b.kind);
        match priority_a.cmp(&priority_b) {
            std::cmp::Ordering::Equal => a.name.cmp(&b.name),
            other => other,
        }
    });

    // Build stats with sorted managers for determinism
    let mut managers: Vec<String> = managers_set.into_iter().collect();
    managers.sort();

    let stats = StackStats {
        total_files: accumulator.total_files,
        languages: accumulator.language_counts,
        managers,
        generated_at: Utc::now(),
    };

    Ok(StackInventory {
        files: accumulator.files,
        dependencies,
        tech,
        stats,
    })
}

// ============================================================================
// Persistence
// ============================================================================

/// Write file entries to a JSONL file.
///
/// Overwrites any existing file.
pub fn write_files_jsonl(path: &Path, files: &[StackFileEntry]) -> Result<(), GikError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }

    let file = File::create(path)?;
    let mut writer = BufWriter::new(file);

    for entry in files {
        let json = serde_json::to_string(entry)?;
        writeln!(writer, "{}", json)?;
    }

    writer.flush()?;
    Ok(())
}

/// Write dependency entries to a JSONL file.
///
/// Overwrites any existing file.
pub fn write_dependencies_jsonl(
    path: &Path,
    deps: &[StackDependencyEntry],
) -> Result<(), GikError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }

    let file = File::create(path)?;
    let mut writer = BufWriter::new(file);

    for entry in deps {
        let json = serde_json::to_string(entry)?;
        writeln!(writer, "{}", json)?;
    }

    writer.flush()?;
    Ok(())
}

/// Write tech entries to a JSONL file.
///
/// Overwrites any existing file.
pub fn write_tech_jsonl(path: &Path, tech: &[StackTechEntry]) -> Result<(), GikError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }

    let file = File::create(path)?;
    let mut writer = BufWriter::new(file);

    for entry in tech {
        let json = serde_json::to_string(entry)?;
        writeln!(writer, "{}", json)?;
    }

    writer.flush()?;
    Ok(())
}

/// Write stats to a JSON file.
///
/// Overwrites any existing file.
pub fn write_stats_json(path: &Path, stats: &StackStats) -> Result<(), GikError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }

    let json = serde_json::to_string_pretty(stats)?;
    fs::write(path, json)?;
    Ok(())
}

/// Read file entries from a JSONL file.
pub fn read_files_jsonl(path: &Path) -> Result<Vec<StackFileEntry>, GikError> {
    if !path.exists() {
        return Ok(Vec::new());
    }

    let file = File::open(path)?;
    let reader = BufReader::new(file);
    let mut entries = Vec::new();

    for line in reader.lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let entry: StackFileEntry = serde_json::from_str(&line)?;
        entries.push(entry);
    }

    Ok(entries)
}

/// Read stats from a JSON file.
pub fn read_stats_json(path: &Path) -> Result<Option<StackStats>, GikError> {
    if !path.exists() {
        return Ok(None);
    }

    let content = fs::read_to_string(path)?;
    let stats: StackStats = serde_json::from_str(&content)?;
    Ok(Some(stats))
}

/// Read tech entries from a JSONL file.
pub fn read_tech_jsonl(path: &Path) -> Result<Vec<StackTechEntry>, GikError> {
    if !path.exists() {
        return Ok(Vec::new());
    }

    let file = File::open(path)?;
    let reader = BufReader::new(file);
    let mut entries = Vec::new();

    for line in reader.lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let entry: StackTechEntry = serde_json::from_str(&line)?;
        entries.push(entry);
    }

    Ok(entries)
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn test_extension_to_language() {
        assert_eq!(extension_to_language("rs"), Some("rust"));
        assert_eq!(extension_to_language("ts"), Some("typescript"));
        assert_eq!(extension_to_language("py"), Some("python"));
        assert_eq!(extension_to_language("unknown"), None);
    }

    #[test]
    fn test_is_binary_extension() {
        assert!(is_binary_extension("exe"));
        assert!(is_binary_extension("png"));
        assert!(is_binary_extension("zip"));
        assert!(!is_binary_extension("rs"));
        assert!(!is_binary_extension("txt"));
    }

    #[test]
    fn test_scan_stack_empty_dir() {
        let temp = TempDir::new().unwrap();
        let inventory = scan_stack(temp.path()).unwrap();

        assert_eq!(inventory.files.len(), 0);
        assert_eq!(inventory.dependencies.len(), 0);
        assert_eq!(inventory.stats.total_files, 0);
    }

    #[test]
    fn test_scan_stack_with_files() {
        let temp = TempDir::new().unwrap();

        // Create some files
        fs::write(temp.path().join("main.rs"), "fn main() {}").unwrap();
        fs::write(temp.path().join("lib.rs"), "pub mod foo;").unwrap();
        fs::write(temp.path().join("README.md"), "# Hello").unwrap();

        let inventory = scan_stack(temp.path()).unwrap();

        assert_eq!(inventory.stats.total_files, 3);
        assert_eq!(inventory.stats.languages.get("rust"), Some(&2));
        assert_eq!(inventory.stats.languages.get("markdown"), Some(&1));
    }

    #[test]
    fn test_scan_stack_skips_hidden() {
        let temp = TempDir::new().unwrap();

        fs::write(temp.path().join("main.rs"), "fn main() {}").unwrap();
        fs::create_dir(temp.path().join(".hidden")).unwrap();
        fs::write(temp.path().join(".hidden/secret.rs"), "// hidden").unwrap();

        let inventory = scan_stack(temp.path()).unwrap();

        // Should only find main.rs
        assert_eq!(inventory.stats.total_files, 1);
    }

    #[test]
    fn test_scan_stack_skips_target() {
        let temp = TempDir::new().unwrap();

        fs::write(temp.path().join("main.rs"), "fn main() {}").unwrap();
        fs::create_dir(temp.path().join("target")).unwrap();
        fs::write(temp.path().join("target/debug.rs"), "// build output").unwrap();

        let inventory = scan_stack(temp.path()).unwrap();

        // Should only find main.rs
        assert_eq!(inventory.stats.total_files, 1);
    }

    #[test]
    fn test_scan_stack_respects_gitignore() {
        let temp = TempDir::new().unwrap();

        fs::write(temp.path().join(".gitignore"), "ignored.rs\n").unwrap();
        fs::write(temp.path().join("main.rs"), "fn main() {}").unwrap();
        fs::write(temp.path().join("ignored.rs"), "// ignored").unwrap();

        let inventory = scan_stack(temp.path()).unwrap();

        // Should find .gitignore and main.rs, but not ignored.rs
        // Note: .gitignore itself is not ignored (hidden=true skips dot files by default,
        // but .gitignore is special and typically included)
        assert_eq!(inventory.stats.total_files, 2);
    }

    #[test]
    fn test_parse_cargo_toml() {
        let temp = TempDir::new().unwrap();
        let cargo_path = temp.path().join("Cargo.toml");

        fs::write(
            &cargo_path,
            r#"
[package]
name = "test"
version = "0.1.0"

[dependencies]
serde = "1.0"
tokio = { version = "1.0", features = ["full"] }

[dev-dependencies]
tempfile = "3"
"#,
        )
        .unwrap();

        let deps = parse_cargo_toml(&cargo_path, "Cargo.toml");

        assert_eq!(deps.len(), 3);

        let serde_dep = deps.iter().find(|d| d.name == "serde").unwrap();
        assert_eq!(serde_dep.version, "1.0");
        assert_eq!(serde_dep.scope, "runtime");
        assert_eq!(serde_dep.manager, "cargo");

        let tokio_dep = deps.iter().find(|d| d.name == "tokio").unwrap();
        assert_eq!(tokio_dep.version, "1.0");

        let tempfile_dep = deps.iter().find(|d| d.name == "tempfile").unwrap();
        assert_eq!(tempfile_dep.scope, "dev");
    }

    #[test]
    fn test_parse_package_json() {
        let temp = TempDir::new().unwrap();
        let package_path = temp.path().join("package.json");

        fs::write(
            &package_path,
            r#"{
  "name": "test",
  "dependencies": {
    "react": "^18.0.0",
    "next": "14.0.0"
  },
  "devDependencies": {
    "typescript": "^5.0.0"
  }
}"#,
        )
        .unwrap();

        let deps = parse_package_json(&package_path, "package.json", "npm");

        assert_eq!(deps.len(), 3);

        let react_dep = deps.iter().find(|d| d.name == "react").unwrap();
        assert_eq!(react_dep.version, "^18.0.0");
        assert_eq!(react_dep.scope, "runtime");
        assert_eq!(react_dep.manager, "npm");

        let ts_dep = deps.iter().find(|d| d.name == "typescript").unwrap();
        assert_eq!(ts_dep.scope, "dev");
    }

    #[test]
    fn test_parse_package_json_with_pnpm() {
        let temp = TempDir::new().unwrap();
        let package_path = temp.path().join("package.json");

        fs::write(
            &package_path,
            r#"{
  "name": "test",
  "dependencies": {
    "react": "^18.0.0"
  }
}"#,
        )
        .unwrap();

        let deps = parse_package_json(&package_path, "package.json", "pnpm");

        assert_eq!(deps.len(), 1);
        assert_eq!(deps[0].manager, "pnpm");
    }

    #[test]
    fn test_write_and_read_files_jsonl() {
        let temp = TempDir::new().unwrap();
        let path = temp.path().join("files.jsonl");

        let files = vec![
            StackFileEntry {
                path: "main.rs".to_string(),
                kind: StackFileKind::File,
                languages: vec!["rust".to_string()],
                file_count: None,
            },
            StackFileEntry {
                path: "lib.rs".to_string(),
                kind: StackFileKind::File,
                languages: vec!["rust".to_string()],
                file_count: None,
            },
        ];

        write_files_jsonl(&path, &files).unwrap();
        let read_back = read_files_jsonl(&path).unwrap();

        assert_eq!(read_back.len(), 2);
        assert_eq!(read_back[0].path, "main.rs");
        assert_eq!(read_back[1].path, "lib.rs");
    }

    #[test]
    fn test_write_and_read_stats_json() {
        let temp = TempDir::new().unwrap();
        let path = temp.path().join("stats.json");

        let mut languages = HashMap::new();
        languages.insert("rust".to_string(), 10);
        languages.insert("typescript".to_string(), 5);

        let stats = StackStats {
            total_files: 15,
            languages,
            managers: vec!["cargo".to_string(), "npm".to_string()],
            generated_at: Utc::now(),
        };

        write_stats_json(&path, &stats).unwrap();
        let read_back = read_stats_json(&path).unwrap().unwrap();

        assert_eq!(read_back.total_files, 15);
        assert_eq!(read_back.languages.get("rust"), Some(&10));
        assert_eq!(read_back.managers.len(), 2);
    }

    #[test]
    fn test_infer_tech_tags() {
        let dependencies = vec![
            StackDependencyEntry {
                manager: "npm".to_string(),
                name: "next".to_string(),
                version: "14.0.0".to_string(),
                scope: "runtime".to_string(),
                manifest_path: "package.json".to_string(),
            },
            StackDependencyEntry {
                manager: "cargo".to_string(),
                name: "tokio".to_string(),
                version: "1.0".to_string(),
                scope: "runtime".to_string(),
                manifest_path: "Cargo.toml".to_string(),
            },
        ];

        let mut languages = HashMap::new();
        languages.insert("rust".to_string(), 10);

        let tags = infer_tech_tags(&dependencies, &languages, &[]);

        // Should have: Rust (language), Next.js (framework), Tokio (tool)
        assert!(tags
            .iter()
            .any(|t| t.name == "Rust" && t.kind == "language"));
        assert!(tags
            .iter()
            .any(|t| t.name == "Next.js" && t.kind == "framework"));
        assert!(tags.iter().any(|t| t.name == "Tokio" && t.kind == "tool"));
    }

    // ========================================================================
    // Lockfile Detection Tests
    // ========================================================================

    #[test]
    fn test_detect_manager_from_lockfile_javascript() {
        // pnpm
        let (manager, priority) = detect_manager_from_lockfile("pnpm-lock.yaml").unwrap();
        assert_eq!(manager, "pnpm");
        assert_eq!(priority, 1);

        // yarn
        let (manager, priority) = detect_manager_from_lockfile("yarn.lock").unwrap();
        assert_eq!(manager, "yarn");
        assert_eq!(priority, 2);

        // bun
        let (manager, priority) = detect_manager_from_lockfile("bun.lockb").unwrap();
        assert_eq!(manager, "bun");
        assert_eq!(priority, 3);

        // npm
        let (manager, priority) = detect_manager_from_lockfile("package-lock.json").unwrap();
        assert_eq!(manager, "npm");
        assert_eq!(priority, 4);
    }

    #[test]
    fn test_detect_manager_from_lockfile_other_languages() {
        // Rust
        let (manager, _) = detect_manager_from_lockfile("Cargo.lock").unwrap();
        assert_eq!(manager, "cargo");

        // Python
        let (manager, _) = detect_manager_from_lockfile("poetry.lock").unwrap();
        assert_eq!(manager, "poetry");
        let (manager, _) = detect_manager_from_lockfile("Pipfile.lock").unwrap();
        assert_eq!(manager, "pipenv");

        // Go
        let (manager, _) = detect_manager_from_lockfile("go.sum").unwrap();
        assert_eq!(manager, "go");

        // Ruby
        let (manager, _) = detect_manager_from_lockfile("Gemfile.lock").unwrap();
        assert_eq!(manager, "bundler");

        // PHP
        let (manager, _) = detect_manager_from_lockfile("composer.lock").unwrap();
        assert_eq!(manager, "composer");
    }

    #[test]
    fn test_detect_manager_from_lockfile_unknown() {
        assert!(detect_manager_from_lockfile("random.lock").is_none());
        assert!(detect_manager_from_lockfile("package.json").is_none());
    }

    #[test]
    fn test_manager_category() {
        assert_eq!(manager_category("npm"), Some(ManagerCategory::JavaScript));
        assert_eq!(manager_category("pnpm"), Some(ManagerCategory::JavaScript));
        assert_eq!(manager_category("yarn"), Some(ManagerCategory::JavaScript));
        assert_eq!(manager_category("bun"), Some(ManagerCategory::JavaScript));
        assert_eq!(manager_category("cargo"), Some(ManagerCategory::Rust));
        assert_eq!(manager_category("poetry"), Some(ManagerCategory::Python));
        assert_eq!(manager_category("go"), Some(ManagerCategory::Go));
        assert_eq!(manager_category("bundler"), Some(ManagerCategory::Ruby));
        assert_eq!(manager_category("unknown"), None);
    }

    #[test]
    fn test_scan_stack_detects_pnpm_from_lockfile() {
        let temp = TempDir::new().unwrap();

        // Create package.json
        fs::write(
            temp.path().join("package.json"),
            r#"{"name": "test", "dependencies": {"react": "^18.0.0"}}"#,
        )
        .unwrap();

        // Create pnpm lockfile
        fs::write(temp.path().join("pnpm-lock.yaml"), "lockfileVersion: 6.0\n").unwrap();

        let inventory = scan_stack(temp.path()).unwrap();

        // Should detect pnpm, not npm
        assert!(inventory.stats.managers.contains(&"pnpm".to_string()));
        assert!(!inventory.stats.managers.contains(&"npm".to_string()));

        // Dependencies should have pnpm as manager
        let react_dep = inventory
            .dependencies
            .iter()
            .find(|d| d.name == "react")
            .unwrap();
        assert_eq!(react_dep.manager, "pnpm");
    }

    #[test]
    fn test_scan_stack_detects_yarn_from_lockfile() {
        let temp = TempDir::new().unwrap();

        fs::write(
            temp.path().join("package.json"),
            r#"{"name": "test", "dependencies": {"vue": "^3.0.0"}}"#,
        )
        .unwrap();

        fs::write(temp.path().join("yarn.lock"), "# yarn lockfile v1\n").unwrap();

        let inventory = scan_stack(temp.path()).unwrap();

        assert!(inventory.stats.managers.contains(&"yarn".to_string()));
        assert!(!inventory.stats.managers.contains(&"npm".to_string()));

        let vue_dep = inventory
            .dependencies
            .iter()
            .find(|d| d.name == "vue")
            .unwrap();
        assert_eq!(vue_dep.manager, "yarn");
    }

    #[test]
    fn test_scan_stack_pnpm_priority_over_npm() {
        let temp = TempDir::new().unwrap();

        fs::write(
            temp.path().join("package.json"),
            r#"{"name": "test", "dependencies": {"next": "^14.0.0"}}"#,
        )
        .unwrap();

        // Both lockfiles present - pnpm should win
        fs::write(temp.path().join("pnpm-lock.yaml"), "lockfileVersion: 6.0\n").unwrap();
        fs::write(temp.path().join("package-lock.json"), "{}").unwrap();

        let inventory = scan_stack(temp.path()).unwrap();

        // pnpm should be detected (higher priority)
        assert!(inventory.stats.managers.contains(&"pnpm".to_string()));
        assert!(!inventory.stats.managers.contains(&"npm".to_string()));

        let next_dep = inventory
            .dependencies
            .iter()
            .find(|d| d.name == "next")
            .unwrap();
        assert_eq!(next_dep.manager, "pnpm");
    }

    #[test]
    fn test_scan_stack_fallback_to_npm() {
        let temp = TempDir::new().unwrap();

        // Only package.json, no lockfile
        fs::write(
            temp.path().join("package.json"),
            r#"{"name": "test", "dependencies": {"express": "^4.0.0"}}"#,
        )
        .unwrap();

        let inventory = scan_stack(temp.path()).unwrap();

        // Should fallback to npm
        assert!(inventory.stats.managers.contains(&"npm".to_string()));

        let express_dep = inventory
            .dependencies
            .iter()
            .find(|d| d.name == "express")
            .unwrap();
        assert_eq!(express_dep.manager, "npm");
    }

    // ========================================================================
    // New Parser Tests
    // ========================================================================

    #[test]
    fn test_parse_cargo_toml_workspace_deps() {
        let temp = TempDir::new().unwrap();
        let cargo_path = temp.path().join("Cargo.toml");

        fs::write(
            &cargo_path,
            r#"
[package]
name = "test"
version = "0.1.0"

[dependencies]
serde.workspace = true
tokio = { workspace = true }
local-lib = { path = "../lib" }
remote = { git = "https://github.com/example/repo" }
versioned = "1.0"
"#,
        )
        .unwrap();

        let deps = parse_cargo_toml(&cargo_path, "Cargo.toml");

        assert_eq!(deps.len(), 5);

        // Workspace deps should have version "*"
        let serde_dep = deps.iter().find(|d| d.name == "serde").unwrap();
        assert_eq!(serde_dep.version, "*");

        let tokio_dep = deps.iter().find(|d| d.name == "tokio").unwrap();
        assert_eq!(tokio_dep.version, "*");

        // Path and git deps should have version "*"
        let local_dep = deps.iter().find(|d| d.name == "local-lib").unwrap();
        assert_eq!(local_dep.version, "*");

        let remote_dep = deps.iter().find(|d| d.name == "remote").unwrap();
        assert_eq!(remote_dep.version, "*");

        // Regular version should be preserved
        let versioned_dep = deps.iter().find(|d| d.name == "versioned").unwrap();
        assert_eq!(versioned_dep.version, "1.0");
    }

    #[test]
    fn test_parse_cargo_toml_build_deps() {
        let temp = TempDir::new().unwrap();
        let cargo_path = temp.path().join("Cargo.toml");

        fs::write(
            &cargo_path,
            r#"
[package]
name = "test"

[build-dependencies]
cc = "1.0"
"#,
        )
        .unwrap();

        let deps = parse_cargo_toml(&cargo_path, "Cargo.toml");

        assert_eq!(deps.len(), 1);
        let cc_dep = deps.iter().find(|d| d.name == "cc").unwrap();
        assert_eq!(cc_dep.scope, "build");
    }

    #[test]
    fn test_parse_package_json_peer_deps() {
        let temp = TempDir::new().unwrap();
        let package_path = temp.path().join("package.json");

        fs::write(
            &package_path,
            r#"{
  "name": "my-lib",
  "dependencies": { "lodash": "^4.0" },
  "devDependencies": { "jest": "^29" },
  "peerDependencies": { "react": "^18" },
  "optionalDependencies": { "fsevents": "^2" }
}"#,
        )
        .unwrap();

        let deps = parse_package_json(&package_path, "package.json", "npm");

        assert_eq!(deps.len(), 4);

        let lodash = deps.iter().find(|d| d.name == "lodash").unwrap();
        assert_eq!(lodash.scope, "runtime");

        let jest = deps.iter().find(|d| d.name == "jest").unwrap();
        assert_eq!(jest.scope, "dev");

        let react = deps.iter().find(|d| d.name == "react").unwrap();
        assert_eq!(react.scope, "peer");

        let fsevents = deps.iter().find(|d| d.name == "fsevents").unwrap();
        assert_eq!(fsevents.scope, "optional");
    }

    #[test]
    fn test_parse_requirements_txt() {
        let temp = TempDir::new().unwrap();
        let req_path = temp.path().join("requirements.txt");

        fs::write(
            &req_path,
            r#"
# Core dependencies
requests==2.28.0
flask>=2.0,<3.0
django[argon2]>=3.2

# Skip these
-r other-requirements.txt
--index-url https://pypi.org/simple
-e git+https://github.com/...

# Simple deps
numpy
pandas
"#,
        )
        .unwrap();

        let deps = parse_requirements_txt(&req_path, "requirements.txt", "pip");

        assert_eq!(deps.len(), 5);

        let requests = deps.iter().find(|d| d.name == "requests").unwrap();
        assert_eq!(requests.version, "==2.28.0");

        let flask = deps.iter().find(|d| d.name == "flask").unwrap();
        assert_eq!(flask.version, ">=2.0,<3.0");

        let django = deps.iter().find(|d| d.name == "django").unwrap();
        assert_eq!(django.version, ">=3.2");

        let numpy = deps.iter().find(|d| d.name == "numpy").unwrap();
        assert_eq!(numpy.version, "*");
    }

    #[test]
    fn test_parse_pyproject_toml_pep621() {
        let temp = TempDir::new().unwrap();
        let pyproject_path = temp.path().join("pyproject.toml");

        fs::write(
            &pyproject_path,
            r#"
[project]
name = "myapp"
dependencies = [
    "requests>=2.0",
    "flask[async]>=2.0",
]

[project.optional-dependencies]
dev = ["pytest>=7.0"]
"#,
        )
        .unwrap();

        let deps = parse_pyproject_toml(&pyproject_path, "pyproject.toml", "pip");

        assert_eq!(deps.len(), 3);

        let requests = deps.iter().find(|d| d.name == "requests").unwrap();
        assert_eq!(requests.version, ">=2.0");
        assert_eq!(requests.scope, "runtime");

        let flask = deps.iter().find(|d| d.name == "flask").unwrap();
        assert_eq!(flask.version, ">=2.0");

        let pytest = deps.iter().find(|d| d.name == "pytest").unwrap();
        assert_eq!(pytest.scope, "optional");
    }

    #[test]
    fn test_parse_pyproject_toml_poetry() {
        let temp = TempDir::new().unwrap();
        let pyproject_path = temp.path().join("pyproject.toml");

        fs::write(
            &pyproject_path,
            r#"
[tool.poetry]
name = "myapp"

[tool.poetry.dependencies]
python = "^3.9"
requests = "^2.28"
django = { version = "^4.0", optional = true }

[tool.poetry.dev-dependencies]
pytest = "^7.0"
"#,
        )
        .unwrap();

        let deps = parse_pyproject_toml(&pyproject_path, "pyproject.toml", "poetry");

        // Should not include python
        assert_eq!(deps.len(), 3);

        let requests = deps.iter().find(|d| d.name == "requests").unwrap();
        assert_eq!(requests.version, "^2.28");
        assert_eq!(requests.manager, "poetry");

        let django = deps.iter().find(|d| d.name == "django").unwrap();
        assert_eq!(django.version, "^4.0");

        let pytest = deps.iter().find(|d| d.name == "pytest").unwrap();
        assert_eq!(pytest.scope, "dev");
    }

    #[test]
    fn test_parse_pipfile() {
        let temp = TempDir::new().unwrap();
        let pipfile_path = temp.path().join("Pipfile");

        fs::write(
            &pipfile_path,
            r#"
[[source]]
url = "https://pypi.org/simple"

[packages]
requests = "*"
flask = ">=2.0"
django = {version = ">=3.2"}

[dev-packages]
pytest = "*"
"#,
        )
        .unwrap();

        let deps = parse_pipfile(&pipfile_path, "Pipfile", "pipenv");

        assert_eq!(deps.len(), 4);

        let requests = deps.iter().find(|d| d.name == "requests").unwrap();
        assert_eq!(requests.version, "*");
        assert_eq!(requests.scope, "runtime");

        let flask = deps.iter().find(|d| d.name == "flask").unwrap();
        assert_eq!(flask.version, ">=2.0");

        let django = deps.iter().find(|d| d.name == "django").unwrap();
        assert_eq!(django.version, ">=3.2");

        let pytest = deps.iter().find(|d| d.name == "pytest").unwrap();
        assert_eq!(pytest.scope, "dev");
    }

    #[test]
    fn test_parse_composer_json() {
        let temp = TempDir::new().unwrap();
        let composer_path = temp.path().join("composer.json");

        fs::write(
            &composer_path,
            r#"{
    "require": {
        "php": "^8.0",
        "ext-json": "*",
        "laravel/framework": "^10.0",
        "guzzlehttp/guzzle": "^7.0"
    },
    "require-dev": {
        "phpunit/phpunit": "^10.0"
    }
}"#,
        )
        .unwrap();

        let deps = parse_composer_json(&composer_path, "composer.json");

        // Should skip php and ext-json
        assert_eq!(deps.len(), 3);

        let laravel = deps.iter().find(|d| d.name == "laravel/framework").unwrap();
        assert_eq!(laravel.version, "^10.0");
        assert_eq!(laravel.scope, "runtime");

        let guzzle = deps.iter().find(|d| d.name == "guzzlehttp/guzzle").unwrap();
        assert_eq!(guzzle.version, "^7.0");

        let phpunit = deps.iter().find(|d| d.name == "phpunit/phpunit").unwrap();
        assert_eq!(phpunit.scope, "dev");
    }

    #[test]
    fn test_parse_go_mod() {
        let temp = TempDir::new().unwrap();
        let go_mod_path = temp.path().join("go.mod");

        fs::write(
            &go_mod_path,
            r#"
module github.com/example/myapp

go 1.21

require github.com/pkg/errors v0.9.1

require (
	github.com/gin-gonic/gin v1.9.0
	github.com/stretchr/testify v1.8.0 // indirect
)
"#,
        )
        .unwrap();

        let deps = parse_go_mod(&go_mod_path, "go.mod");

        assert_eq!(deps.len(), 3);

        let errors = deps
            .iter()
            .find(|d| d.name == "github.com/pkg/errors")
            .unwrap();
        assert_eq!(errors.version, "0.9.1");

        let gin = deps
            .iter()
            .find(|d| d.name == "github.com/gin-gonic/gin")
            .unwrap();
        assert_eq!(gin.version, "1.9.0");

        let testify = deps
            .iter()
            .find(|d| d.name == "github.com/stretchr/testify")
            .unwrap();
        assert_eq!(testify.version, "1.8.0");
    }

    #[test]
    fn test_parse_gemfile() {
        let temp = TempDir::new().unwrap();
        let gemfile_path = temp.path().join("Gemfile");

        fs::write(
            &gemfile_path,
            r#"
source 'https://rubygems.org'

gem 'rails', '~> 7.0'
gem 'pg', '>= 1.0'
gem 'puma'

group :development, :test do
  gem 'rspec-rails', '~> 6.0'
  gem 'factory_bot_rails'
end
"#,
        )
        .unwrap();

        let deps = parse_gemfile(&gemfile_path, "Gemfile");

        assert_eq!(deps.len(), 5);

        let rails = deps.iter().find(|d| d.name == "rails").unwrap();
        assert_eq!(rails.version, "~> 7.0");
        assert_eq!(rails.scope, "runtime");
        assert_eq!(rails.manager, "bundler");

        let puma = deps.iter().find(|d| d.name == "puma").unwrap();
        assert_eq!(puma.version, "*");

        let rspec = deps.iter().find(|d| d.name == "rspec-rails").unwrap();
        assert_eq!(rspec.scope, "dev");
    }

    #[test]
    fn test_parse_malformed_manifest_returns_empty() {
        let temp = TempDir::new().unwrap();

        // Malformed Cargo.toml
        let cargo_path = temp.path().join("Cargo.toml");
        fs::write(&cargo_path, "this is not valid toml [[[").unwrap();
        let deps = parse_cargo_toml(&cargo_path, "Cargo.toml");
        assert!(deps.is_empty());

        // Malformed package.json
        let pkg_path = temp.path().join("package.json");
        fs::write(&pkg_path, "{ invalid json }").unwrap();
        let deps = parse_package_json(&pkg_path, "package.json", "npm");
        assert!(deps.is_empty());

        // Malformed pyproject.toml
        let pyproject_path = temp.path().join("pyproject.toml");
        fs::write(&pyproject_path, "not valid [[[ toml").unwrap();
        let deps = parse_pyproject_toml(&pyproject_path, "pyproject.toml", "pip");
        assert!(deps.is_empty());
    }

    #[test]
    fn test_scan_stack_with_python_project() {
        let temp = TempDir::new().unwrap();

        // Create pyproject.toml
        fs::write(
            temp.path().join("pyproject.toml"),
            r#"
[project]
dependencies = ["requests>=2.0", "flask"]
"#,
        )
        .unwrap();

        // Create poetry.lock to set manager
        fs::write(temp.path().join("poetry.lock"), "# poetry lock").unwrap();

        let inventory = scan_stack(temp.path()).unwrap();

        assert!(inventory.stats.managers.contains(&"poetry".to_string()));

        let requests = inventory
            .dependencies
            .iter()
            .find(|d| d.name == "requests")
            .unwrap();
        assert_eq!(requests.manager, "poetry");
    }

    #[test]
    fn test_scan_stack_with_go_project() {
        let temp = TempDir::new().unwrap();

        fs::write(temp.path().join("main.go"), "package main").unwrap();
        fs::write(
            temp.path().join("go.mod"),
            r#"
module example.com/app
require github.com/gin-gonic/gin v1.9.0
"#,
        )
        .unwrap();
        fs::write(temp.path().join("go.sum"), "# checksums").unwrap();

        let inventory = scan_stack(temp.path()).unwrap();

        assert!(inventory.stats.managers.contains(&"go".to_string()));
        assert!(inventory
            .dependencies
            .iter()
            .any(|d| d.name == "github.com/gin-gonic/gin"));
    }

    // ========================================================================
    // New Language Extension Tests
    // ========================================================================

    #[test]
    fn test_extension_to_language_modern_web() {
        // Modern web framework extensions
        assert_eq!(extension_to_language("vue"), Some("vue"));
        assert_eq!(extension_to_language("svelte"), Some("svelte"));
        assert_eq!(extension_to_language("astro"), Some("astro"));
        assert_eq!(extension_to_language("mdx"), Some("mdx"));
    }

    #[test]
    fn test_extension_to_language_api_schema() {
        // API/Schema file extensions
        assert_eq!(extension_to_language("graphql"), Some("graphql"));
        assert_eq!(extension_to_language("gql"), Some("graphql"));
        assert_eq!(extension_to_language("proto"), Some("protobuf"));
        assert_eq!(extension_to_language("prisma"), Some("prisma"));
    }

    #[test]
    fn test_extension_to_language_additional_languages() {
        // Additional languages
        assert_eq!(extension_to_language("dart"), Some("dart"));
        assert_eq!(extension_to_language("zig"), Some("zig"));
        assert_eq!(extension_to_language("nim"), Some("nim"));
        assert_eq!(extension_to_language("jl"), Some("julia"));
        assert_eq!(extension_to_language("pl"), Some("perl"));
        assert_eq!(extension_to_language("groovy"), Some("groovy"));
        assert_eq!(extension_to_language("gradle"), Some("groovy"));
        assert_eq!(extension_to_language("hcl"), Some("hcl"));
    }

    #[test]
    fn test_extension_to_language_json_variants() {
        // JSON variants
        assert_eq!(extension_to_language("json"), Some("json"));
        assert_eq!(extension_to_language("jsonc"), Some("json"));
        assert_eq!(extension_to_language("json5"), Some("json"));
    }

    // ========================================================================
    // New Binary Extension Tests
    // ========================================================================

    #[test]
    fn test_is_binary_extension_modern_formats() {
        // Modern image formats
        assert!(is_binary_extension("heic"));
        assert!(is_binary_extension("heif"));
        assert!(is_binary_extension("avif"));
        assert!(is_binary_extension("tiff"));
        assert!(is_binary_extension("tif"));

        // Modern video/audio
        assert!(is_binary_extension("mkv"));
        assert!(is_binary_extension("flac"));
        assert!(is_binary_extension("m4a"));
        assert!(is_binary_extension("aac"));

        // Design files
        assert!(is_binary_extension("psd"));
        assert!(is_binary_extension("sketch"));
        assert!(is_binary_extension("fig"));
        assert!(is_binary_extension("xd"));

        // 3D files
        assert!(is_binary_extension("blend"));
        assert!(is_binary_extension("fbx"));
        assert!(is_binary_extension("gltf"));
        assert!(is_binary_extension("glb"));

        // Modern archives
        assert!(is_binary_extension("lz4"));
        assert!(is_binary_extension("zst"));
        assert!(is_binary_extension("dmg"));
        assert!(is_binary_extension("iso"));
    }

    // ========================================================================
    // Tech Tag Detection Tests
    // ========================================================================

    #[test]
    fn test_infer_js_tech_tag_frameworks() {
        // Full-stack frameworks
        assert_eq!(infer_js_tech_tag("next"), Some(("framework", "Next.js")));
        assert_eq!(infer_js_tech_tag("@sveltejs/kit"), Some(("framework", "SvelteKit")));
        assert_eq!(infer_js_tech_tag("astro"), Some(("framework", "Astro")));
        assert_eq!(infer_js_tech_tag("gatsby"), Some(("framework", "Gatsby")));

        // Frontend frameworks
        assert_eq!(infer_js_tech_tag("react"), Some(("framework", "React")));
        assert_eq!(infer_js_tech_tag("vue"), Some(("framework", "Vue.js")));
        assert_eq!(infer_js_tech_tag("svelte"), Some(("framework", "Svelte")));
        assert_eq!(infer_js_tech_tag("preact"), Some(("framework", "Preact")));
    }

    #[test]
    fn test_infer_js_tech_tag_tools() {
        // State management
        assert_eq!(infer_js_tech_tag("zustand"), Some(("tool", "Zustand")));
        assert_eq!(infer_js_tech_tag("jotai"), Some(("tool", "Jotai")));
        assert_eq!(infer_js_tech_tag("@tanstack/react-query"), Some(("tool", "TanStack Query")));

        // Styling
        assert_eq!(infer_js_tech_tag("tailwindcss"), Some(("tool", "Tailwind CSS")));
        assert_eq!(infer_js_tech_tag("styled-components"), Some(("tool", "Styled Components")));

        // API
        assert_eq!(infer_js_tech_tag("@trpc/server"), Some(("tool", "tRPC")));
        assert_eq!(infer_js_tech_tag("graphql"), Some(("tool", "GraphQL")));

        // Database
        assert_eq!(infer_js_tech_tag("mongoose"), Some(("tool", "Mongoose")));
        assert_eq!(infer_js_tech_tag("drizzle-orm"), Some(("tool", "Drizzle")));
    }

    #[test]
    fn test_infer_python_tech_tag() {
        // Web frameworks
        assert_eq!(infer_python_tech_tag("django"), Some(("framework", "Django")));
        assert_eq!(infer_python_tech_tag("flask"), Some(("framework", "Flask")));
        assert_eq!(infer_python_tech_tag("fastapi"), Some(("framework", "FastAPI")));
        assert_eq!(infer_python_tech_tag("starlette"), Some(("framework", "Starlette")));

        // Data science
        assert_eq!(infer_python_tech_tag("pandas"), Some(("tool", "pandas")));
        assert_eq!(infer_python_tech_tag("numpy"), Some(("tool", "NumPy")));

        // ML/AI
        assert_eq!(infer_python_tech_tag("tensorflow"), Some(("tool", "TensorFlow")));
        assert_eq!(infer_python_tech_tag("torch"), Some(("tool", "PyTorch")));
        assert_eq!(infer_python_tech_tag("langchain"), Some(("tool", "LangChain")));

        // Tools
        assert_eq!(infer_python_tech_tag("celery"), Some(("tool", "Celery")));
        assert_eq!(infer_python_tech_tag("sqlalchemy"), Some(("tool", "SQLAlchemy")));
        assert_eq!(infer_python_tech_tag("pydantic"), Some(("tool", "Pydantic")));
        assert_eq!(infer_python_tech_tag("pytest"), Some(("tool", "pytest")));
    }

    #[test]
    fn test_infer_go_tech_tag() {
        // Web frameworks
        assert_eq!(
            infer_go_tech_tag("github.com/gin-gonic/gin"),
            Some(("framework", "Gin"))
        );
        assert_eq!(
            infer_go_tech_tag("github.com/gofiber/fiber/v2"),
            Some(("framework", "Fiber"))
        );
        assert_eq!(
            infer_go_tech_tag("github.com/labstack/echo/v4"),
            Some(("framework", "Echo"))
        );
        assert_eq!(
            infer_go_tech_tag("github.com/go-chi/chi/v5"),
            Some(("framework", "Chi"))
        );

        // Database
        assert_eq!(infer_go_tech_tag("gorm.io/gorm"), Some(("tool", "GORM")));
        assert_eq!(
            infer_go_tech_tag("github.com/stretchr/testify"),
            Some(("tool", "Testify"))
        );
    }

    #[test]
    fn test_infer_ruby_tech_tag() {
        // Web frameworks
        assert_eq!(infer_ruby_tech_tag("rails"), Some(("framework", "Ruby on Rails")));
        assert_eq!(infer_ruby_tech_tag("sinatra"), Some(("framework", "Sinatra")));
        assert_eq!(infer_ruby_tech_tag("hanami"), Some(("framework", "Hanami")));

        // Background jobs
        assert_eq!(infer_ruby_tech_tag("sidekiq"), Some(("tool", "Sidekiq")));

        // Testing
        assert_eq!(infer_ruby_tech_tag("rspec"), Some(("tool", "RSpec")));
        assert_eq!(infer_ruby_tech_tag("rspec-rails"), Some(("tool", "RSpec")));
    }

    #[test]
    fn test_infer_php_tech_tag() {
        // Web frameworks
        assert_eq!(
            infer_php_tech_tag("laravel/framework"),
            Some(("framework", "Laravel"))
        );
        assert_eq!(
            infer_php_tech_tag("symfony/symfony"),
            Some(("framework", "Symfony"))
        );
        assert_eq!(infer_php_tech_tag("slim/slim"), Some(("framework", "Slim")));

        // Database
        assert_eq!(infer_php_tech_tag("doctrine/orm"), Some(("tool", "Doctrine")));

        // Testing
        assert_eq!(infer_php_tech_tag("phpunit/phpunit"), Some(("tool", "PHPUnit")));
    }

    #[test]
    fn test_infer_rust_tech_tag() {
        // Web frameworks
        assert_eq!(infer_rust_tech_tag("actix-web"), Some(("framework", "Actix Web")));
        assert_eq!(infer_rust_tech_tag("axum"), Some(("framework", "Axum")));
        assert_eq!(infer_rust_tech_tag("rocket"), Some(("framework", "Rocket")));

        // Async runtime
        assert_eq!(infer_rust_tech_tag("tokio"), Some(("tool", "Tokio")));

        // Database
        assert_eq!(infer_rust_tech_tag("sqlx"), Some(("tool", "SQLx")));
        assert_eq!(infer_rust_tech_tag("diesel"), Some(("tool", "Diesel")));
        assert_eq!(infer_rust_tech_tag("sea-orm"), Some(("tool", "SeaORM")));
    }

    // ========================================================================
    // File-Based Tech Detection Tests
    // ========================================================================

    #[test]
    fn test_infer_tech_from_files_docker_compose() {
        let files = vec![
            StackFileEntry {
                path: "docker-compose.yml".to_string(),
                kind: StackFileKind::File,
                languages: vec!["yaml".to_string()],
                file_count: None,
            },
        ];

        let tags = infer_tech_from_files(&files);

        assert!(tags.iter().any(|t| t.name == "Docker Compose" && t.kind == "infra"));
    }

    #[test]
    fn test_infer_tech_from_files_github_actions() {
        let files = vec![
            StackFileEntry {
                path: ".github/workflows/ci.yml".to_string(),
                kind: StackFileKind::File,
                languages: vec!["yaml".to_string()],
                file_count: None,
            },
        ];

        let tags = infer_tech_from_files(&files);

        assert!(tags.iter().any(|t| t.name == "GitHub Actions" && t.kind == "tool"));
    }

    #[test]
    fn test_infer_tech_from_files_kubernetes() {
        let files = vec![
            StackFileEntry {
                path: "kubernetes/deployment.yaml".to_string(),
                kind: StackFileKind::File,
                languages: vec!["yaml".to_string()],
                file_count: None,
            },
        ];

        let tags = infer_tech_from_files(&files);

        assert!(tags.iter().any(|t| t.name == "Kubernetes" && t.kind == "infra"));
    }

    #[test]
    fn test_infer_tech_from_files_helm() {
        let files = vec![
            StackFileEntry {
                path: "charts/myapp/Chart.yaml".to_string(),
                kind: StackFileKind::File,
                languages: vec!["yaml".to_string()],
                file_count: None,
            },
        ];

        let tags = infer_tech_from_files(&files);

        assert!(tags.iter().any(|t| t.name == "Helm" && t.kind == "infra"));
    }

    #[test]
    fn test_infer_tech_from_files_terraform() {
        let files = vec![
            StackFileEntry {
                path: "infra/main.tf".to_string(),
                kind: StackFileKind::File,
                languages: vec!["terraform".to_string()],
                file_count: None,
            },
        ];

        let tags = infer_tech_from_files(&files);

        assert!(tags.iter().any(|t| t.name == "Terraform" && t.kind == "infra"));
    }

    #[test]
    fn test_infer_tech_from_files_makefile() {
        let files = vec![
            StackFileEntry {
                path: "Makefile".to_string(),
                kind: StackFileKind::File,
                languages: vec![],
                file_count: None,
            },
        ];

        let tags = infer_tech_from_files(&files);

        assert!(tags.iter().any(|t| t.name == "Make" && t.kind == "tool"));
    }

    #[test]
    fn test_infer_tech_from_files_monorepo_tools() {
        let files = vec![
            StackFileEntry {
                path: "turbo.json".to_string(),
                kind: StackFileKind::File,
                languages: vec!["json".to_string()],
                file_count: None,
            },
            StackFileEntry {
                path: "nx.json".to_string(),
                kind: StackFileKind::File,
                languages: vec!["json".to_string()],
                file_count: None,
            },
        ];

        let tags = infer_tech_from_files(&files);

        assert!(tags.iter().any(|t| t.name == "Turborepo" && t.kind == "tool"));
        assert!(tags.iter().any(|t| t.name == "Nx" && t.kind == "tool"));
    }

    #[test]
    fn test_infer_tech_from_files_deployment_platforms() {
        let files = vec![
            StackFileEntry {
                path: "vercel.json".to_string(),
                kind: StackFileKind::File,
                languages: vec!["json".to_string()],
                file_count: None,
            },
            StackFileEntry {
                path: "netlify.toml".to_string(),
                kind: StackFileKind::File,
                languages: vec!["toml".to_string()],
                file_count: None,
            },
            StackFileEntry {
                path: "fly.toml".to_string(),
                kind: StackFileKind::File,
                languages: vec!["toml".to_string()],
                file_count: None,
            },
        ];

        let tags = infer_tech_from_files(&files);

        assert!(tags.iter().any(|t| t.name == "Vercel" && t.kind == "infra"));
        assert!(tags.iter().any(|t| t.name == "Netlify" && t.kind == "infra"));
        assert!(tags.iter().any(|t| t.name == "Fly.io" && t.kind == "infra"));
    }

    #[test]
    fn test_scan_stack_detects_file_based_tech() {
        let temp = TempDir::new().unwrap();

        // Create docker-compose.yml
        fs::write(
            temp.path().join("docker-compose.yml"),
            "version: '3'\nservices:\n  app:\n    image: node",
        )
        .unwrap();

        // Create Makefile
        fs::write(temp.path().join("Makefile"), "build:\n\techo build").unwrap();

        // Create a source file to ensure files vec is populated
        fs::write(temp.path().join("main.rs"), "fn main() {}").unwrap();

        let inventory = scan_stack(temp.path()).unwrap();

        assert!(inventory.tech.iter().any(|t| t.name == "Docker Compose"));
        assert!(inventory.tech.iter().any(|t| t.name == "Make"));
    }

    #[test]
    fn test_scan_stack_deterministic_output() {
        let temp = TempDir::new().unwrap();

        // Create multiple files in various directories
        let src = temp.path().join("src");
        fs::create_dir_all(&src).unwrap();
        fs::write(src.join("main.rs"), "fn main() {}").unwrap();
        fs::write(src.join("lib.rs"), "pub fn foo() {}").unwrap();
        fs::write(src.join("utils.rs"), "pub fn bar() {}").unwrap();

        let tests = temp.path().join("tests");
        fs::create_dir_all(&tests).unwrap();
        fs::write(tests.join("integration.rs"), "#[test] fn test() {}").unwrap();
        fs::write(tests.join("unit.rs"), "#[test] fn unit() {}").unwrap();

        // Create package.json with deps
        fs::write(
            temp.path().join("package.json"),
            r#"{"dependencies": {"react": "^18.0.0", "lodash": "^4.0.0", "axios": "^1.0.0"}}"#,
        )
        .unwrap();

        // Create Cargo.toml
        fs::write(
            temp.path().join("Cargo.toml"),
            r#"[package]
name = "test"
[dependencies]
serde = "1.0"
tokio = "1.0"
"#,
        )
        .unwrap();

        // Run scan multiple times and verify identical output
        let mut results = Vec::new();
        for _ in 0..5 {
            let inventory = scan_stack(temp.path()).unwrap();
            results.push(inventory);
        }

        // Verify all file paths are identical across runs
        let first_files: Vec<&str> = results[0].files.iter().map(|f| f.path.as_str()).collect();
        for (i, inv) in results.iter().enumerate().skip(1) {
            let file_paths: Vec<&str> = inv.files.iter().map(|f| f.path.as_str()).collect();
            assert_eq!(
                first_files, file_paths,
                "File order differs on run {}",
                i + 1
            );
        }

        // Verify file paths are sorted
        let mut sorted_paths = first_files.clone();
        sorted_paths.sort();
        assert_eq!(
            first_files, sorted_paths,
            "Files should be sorted by path"
        );

        // Verify managers are sorted
        let first_managers = &results[0].stats.managers;
        let mut sorted_managers = first_managers.clone();
        sorted_managers.sort();
        assert_eq!(
            first_managers, &sorted_managers,
            "Managers should be sorted"
        );

        // Verify dependencies are consistent
        let first_deps: Vec<&str> = results[0]
            .dependencies
            .iter()
            .map(|d| d.name.as_str())
            .collect();
        for (i, inv) in results.iter().enumerate().skip(1) {
            let dep_names: Vec<&str> = inv.dependencies.iter().map(|d| d.name.as_str()).collect();
            assert_eq!(
                first_deps, dep_names,
                "Dependency order differs on run {}",
                i + 1
            );
        }
    }

    #[test]
    fn test_dependencies_sorted_by_manager_then_name() {
        let temp = TempDir::new().unwrap();

        // Create multiple manifest files with various dependencies
        // The order in the file should not affect the output order

        // package.json with deps in reverse alphabetical order
        fs::write(
            temp.path().join("package.json"),
            r#"{"dependencies": {"zod": "^3.0.0", "axios": "^1.0.0", "next": "^14.0.0"}}"#,
        )
        .unwrap();

        // Cargo.toml with deps in reverse order
        fs::write(
            temp.path().join("Cargo.toml"),
            r#"[package]
name = "test"
[dependencies]
tokio = "1.0"
serde = "1.0"
anyhow = "1.0"
"#,
        )
        .unwrap();

        let inventory = scan_stack(temp.path()).unwrap();

        // Extract deps as (manager, name) tuples
        let dep_tuples: Vec<(&str, &str)> = inventory
            .dependencies
            .iter()
            .map(|d| (d.manager.as_str(), d.name.as_str()))
            .collect();

        // Verify sorted by manager first
        let mut prev_manager = "";
        let mut prev_name = "";
        for (manager, name) in &dep_tuples {
            if *manager == prev_manager {
                assert!(
                    name >= &prev_name,
                    "Dependencies not sorted by name within manager '{}': {} should come before {}",
                    manager,
                    prev_name,
                    name
                );
            } else {
                assert!(
                    manager >= &prev_manager,
                    "Dependencies not sorted by manager: {} should come before {}",
                    prev_manager,
                    manager
                );
            }
            prev_manager = manager;
            prev_name = name;
        }

        // Verify cargo deps come before npm (alphabetically)
        let cargo_idx = dep_tuples.iter().position(|(m, _)| *m == "cargo");
        let npm_idx = dep_tuples.iter().position(|(m, _)| *m == "npm");
        if let (Some(ci), Some(ni)) = (cargo_idx, npm_idx) {
            assert!(ci < ni, "cargo deps should come before npm deps");
        }
    }

    #[test]
    fn test_tech_tags_sorted_by_kind_priority_then_name() {
        let temp = TempDir::new().unwrap();

        // Create files that will trigger various tech tags
        fs::write(temp.path().join("main.rs"), "fn main() {}").unwrap();
        fs::write(temp.path().join("index.ts"), "console.log('hello');").unwrap();

        // Create package.json with various framework and tool deps
        fs::write(
            temp.path().join("package.json"),
            r#"{
                "dependencies": {
                    "next": "^14.0.0",
                    "react": "^18.0.0",
                    "eslint": "^8.0.0",
                    "typescript": "^5.0.0"
                }
            }"#,
        )
        .unwrap();

        // Create docker-compose.yml for infra tag
        fs::write(
            temp.path().join("docker-compose.yml"),
            "version: '3'\nservices:\n  app:\n    image: node",
        )
        .unwrap();

        let inventory = scan_stack(temp.path()).unwrap();

        // Extract tech tags as (kind, name) tuples
        let tech_tuples: Vec<(&str, &str)> = inventory
            .tech
            .iter()
            .map(|t| (t.kind.as_str(), t.name.as_str()))
            .collect();

        // Define expected kind order
        let kind_priority = |k: &str| -> u8 {
            match k {
                "language" => 0,
                "framework" => 1,
                "tool" => 2,
                "service" => 3,
                "infra" => 4,
                _ => 5,
            }
        };

        // Verify sorting
        let mut prev_priority = 0u8;
        let mut prev_name = "";
        for (kind, name) in &tech_tuples {
            let priority = kind_priority(kind);
            if priority == prev_priority {
                assert!(
                    name >= &prev_name,
                    "Tech tags not sorted by name within kind '{}': {} should come before {}",
                    kind,
                    prev_name,
                    name
                );
            } else {
                assert!(
                    priority >= prev_priority,
                    "Tech tags not sorted by kind priority: {} (priority {}) should come before {} (priority {})",
                    kind,
                    priority,
                    "",
                    prev_priority
                );
            }
            prev_priority = priority;
            prev_name = name;
        }

        // Verify languages come before frameworks come before tools come before infra
        let lang_idx = tech_tuples.iter().position(|(k, _)| *k == "language");
        let framework_idx = tech_tuples.iter().position(|(k, _)| *k == "framework");
        let tool_idx = tech_tuples.iter().position(|(k, _)| *k == "tool");
        let infra_idx = tech_tuples.iter().position(|(k, _)| *k == "infra");

        if let (Some(li), Some(fi)) = (lang_idx, framework_idx) {
            assert!(li < fi, "languages should come before frameworks");
        }
        if let (Some(fi), Some(ti)) = (framework_idx, tool_idx) {
            assert!(fi < ti, "frameworks should come before tools");
        }
        if let (Some(ti), Some(ii)) = (tool_idx, infra_idx) {
            assert!(ti < ii, "tools should come before infra");
        }
    }

    #[test]
    fn test_tech_kind_priority() {
        assert_eq!(tech_kind_priority("language"), 0);
        assert_eq!(tech_kind_priority("framework"), 1);
        assert_eq!(tech_kind_priority("tool"), 2);
        assert_eq!(tech_kind_priority("service"), 3);
        assert_eq!(tech_kind_priority("infra"), 4);
        assert_eq!(tech_kind_priority("unknown"), 5);
    }
}
