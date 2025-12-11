//! Language-specific KG extraction module.
//!
//! This module provides per-language extractors for deriving symbol-level
//! nodes and relations from source code.
//!
//! ## Phase 9.2.1 Scope
//!
//! - **Shallow symbol extraction**: top-level functions, classes, namespaces
//! - **Structural relations only**: no call graphs or deep inter-symbol edges
//! - **Best-effort regex heuristics**: not full AST parsing
//!
//! ## Supported Languages
//!
//! | Language | Extension(s) | Symbols | Relations |
//! |----------|--------------|---------|-----------|
//! | JavaScript/TypeScript | `.js`, `.jsx`, `.ts`, `.tsx` | functions, classes, namespaces | imports, defines |
//! | Python | `.py` | functions, classes, modules | imports, defines |
//! | Ruby | `.rb` | classes, modules, methods | requires, defines |
//! | C# | `.cs` | namespaces, classes, methods | usings, defines |
//! | Java | `.java` | packages, classes, interfaces, methods | imports, defines |
//! | Markdown | `.md` | sections, links | mentions |
//! | Rust | `.rs` | modules, functions, structs, enums, traits | uses, defines |
//! | C | `.c`, `.h` | functions | includes |
//! | C++ | `.cpp`, `.hpp`, `.cc`, `.cxx` | namespaces, classes, functions | includes, defines |
//! | SQL | `.sql` | tables, views, functions | declares |
//! | PHP | `.php` | namespaces, classes, functions | uses, requires |
//! | Go | `.go` | packages, types, functions | imports, defines |
//! | Kotlin | `.kt`, `.kts` | packages, classes, functions | imports, defines |
//!
//! ## Symbol ID Convention
//!
//! Symbol IDs follow the pattern:
//! ```text
//! sym:<lang>:<normalizedFilePath>:<kind>:<name>[#<index>]
//! ```
//!
//! - `lang`: Language tag (js, ts, py, rs, etc.)
//! - `normalizedFilePath`: POSIX-style path (same as file nodes)
//! - `kind`: Symbol kind (function, class, namespace, etc.)
//! - `name`: Symbol name as seen in code
//! - `#<index>`: Optional suffix for duplicates in the same file
//!
//! ## Future (TODO)
//!
//! - Phase 9.4+: Full call graph extraction
//! - Phase 9.4+: Framework-specific endpoint detection beyond Next.js

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;

// Language-specific modules
pub mod c_lang;
pub mod cpp_lang;
pub mod csharp_lang;
pub mod css_lang;
pub mod go_lang;
pub mod html_lang;
pub mod java_lang;
pub mod js_ts_lang;
pub mod kotlin_lang;
pub mod markdown_lang;
pub mod php_lang;
pub mod python_lang;
pub mod ruby_lang;
pub mod rust_lang;
pub mod sql_lang;

// ============================================================================
// LanguageKind
// ============================================================================

/// Enumeration of supported programming languages.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum LanguageKind {
    /// JavaScript and TypeScript
    JsTs,
    /// Python
    Python,
    /// Ruby
    Ruby,
    /// C#
    CSharp,
    /// Java
    Java,
    /// Markdown documentation
    Markdown,
    /// Rust
    Rust,
    /// C
    C,
    /// C++
    Cpp,
    /// SQL
    Sql,
    /// PHP
    Php,
    /// Go
    Go,
    /// Kotlin
    Kotlin,
    /// CSS / SCSS / Tailwind stylesheets
    Css,
    /// HTML templates
    Html,
    /// Unknown or unsupported language
    Unknown,
}

impl LanguageKind {
    /// Get a human-readable name for the language.
    pub fn name(&self) -> &'static str {
        match self {
            LanguageKind::JsTs => "JavaScript/TypeScript",
            LanguageKind::Python => "Python",
            LanguageKind::Ruby => "Ruby",
            LanguageKind::CSharp => "C#",
            LanguageKind::Java => "Java",
            LanguageKind::Markdown => "Markdown",
            LanguageKind::Rust => "Rust",
            LanguageKind::C => "C",
            LanguageKind::Cpp => "C++",
            LanguageKind::Sql => "SQL",
            LanguageKind::Php => "PHP",
            LanguageKind::Go => "Go",
            LanguageKind::Kotlin => "Kotlin",
            LanguageKind::Css => "CSS/SCSS",
            LanguageKind::Html => "HTML",
            LanguageKind::Unknown => "Unknown",
        }
    }

    /// Get a short tag for use in IDs.
    pub fn tag(&self) -> &'static str {
        match self {
            LanguageKind::JsTs => "js",
            LanguageKind::Python => "py",
            LanguageKind::Ruby => "rb",
            LanguageKind::CSharp => "cs",
            LanguageKind::Java => "java",
            LanguageKind::Markdown => "md",
            LanguageKind::Rust => "rs",
            LanguageKind::C => "c",
            LanguageKind::Cpp => "cpp",
            LanguageKind::Sql => "sql",
            LanguageKind::Php => "php",
            LanguageKind::Go => "go",
            LanguageKind::Kotlin => "kt",
            LanguageKind::Css => "css",
            LanguageKind::Html => "html",
            LanguageKind::Unknown => "unknown",
        }
    }
}

// ============================================================================
// FrameworkHint
// ============================================================================

/// Hints about detected web frameworks.
///
/// Used to enable framework-specific extraction logic (e.g., route detection).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum FrameworkHint {
    /// Next.js (React framework)
    NextJs,
    /// Express.js (Node.js web framework)
    Express,
    /// Django (Python web framework)
    Django,
    /// Flask (Python micro-framework)
    Flask,
    /// Ruby on Rails
    Rails,
    /// Spring (Java framework)
    Spring,
    /// ASP.NET (C# framework)
    AspNet,
    /// Laravel (PHP framework)
    Laravel,
    /// Gin (Go web framework)
    Gin,
    /// Fiber (Go web framework)
    Fiber,
    /// React (generic React, not Next.js)
    React,
    /// shadcn/ui component library
    Shadcn,
    /// Angular framework
    Angular,
    /// Tailwind CSS utility framework
    Tailwind,
    /// Generic framework (detected but not specifically identified)
    Generic,
    /// No framework detected
    #[default]
    None,
}

// ============================================================================
// KgSymbolCandidate
// ============================================================================

/// A symbol candidate extracted from source code.
///
/// Represents a named entity like a function, class, namespace, or table
/// that may be turned into a KgNode.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KgSymbolCandidate {
    /// Unique identifier for this symbol.
    /// Convention: `sym:<lang>:<filePath>:<kind>:<name>[#<index>]`
    pub id: String,

    /// Symbol kind (e.g., "function", "class", "namespace", "table", "endpoint").
    pub kind: String,

    /// Symbol name (e.g., function name, class name).
    pub name: String,

    /// The programming language this symbol belongs to.
    pub language: LanguageKind,

    /// Detected framework hint (if any).
    pub framework: FrameworkHint,

    /// File path where this symbol is defined.
    pub file_path: String,

    /// Optional byte or line span in the source file (start, end).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub span: Option<(u32, u32)>,

    /// Additional properties specific to this symbol.
    #[serde(default)]
    pub props: serde_json::Value,
}

impl KgSymbolCandidate {
    /// Create a new symbol candidate.
    pub fn new(kind: &str, name: &str, language: LanguageKind, file_path: &str) -> Self {
        // Use tag_from_path for accurate language distinction (e.g., ts vs js)
        let lang_tag = tag_from_path(file_path);
        let id = format!("sym:{}:{}:{}:{}", lang_tag, file_path, kind, name);
        Self {
            id,
            kind: kind.to_string(),
            name: name.to_string(),
            language,
            framework: FrameworkHint::None,
            file_path: file_path.to_string(),
            span: None,
            props: serde_json::Value::Null,
        }
    }

    /// Set the framework hint.
    pub fn with_framework(mut self, framework: FrameworkHint) -> Self {
        self.framework = framework;
        self
    }

    /// Set the span.
    pub fn with_span(mut self, start: u32, end: u32) -> Self {
        self.span = Some((start, end));
        self
    }

    /// Set additional properties.
    pub fn with_props(mut self, props: serde_json::Value) -> Self {
        self.props = props;
        self
    }

    /// Add a single property.
    pub fn with_prop(mut self, key: String, value: String) -> Self {
        if self.props.is_null() {
            self.props = serde_json::json!({});
        }
        if let Some(obj) = self.props.as_object_mut() {
            obj.insert(key, serde_json::Value::String(value));
        }
        self
    }
}

// ============================================================================
// KgRelationCandidate
// ============================================================================

/// A relation candidate extracted from source code.
///
/// Represents a structural relationship between symbols (e.g., "defines",
/// "belongsToNamespace", "usesImport") that may be turned into a KgEdge.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KgRelationCandidate {
    /// Source symbol/node ID.
    pub from_id: String,

    /// Target symbol/node ID.
    pub to_id: String,

    /// Relation kind (e.g., "defines", "belongsToNamespace", "usesImport").
    pub kind: String,

    /// Additional properties specific to this relation.
    #[serde(default)]
    pub props: serde_json::Value,
}

impl KgRelationCandidate {
    /// Create a new relation candidate.
    pub fn new(from_id: &str, to_id: &str, kind: &str) -> Self {
        Self {
            from_id: from_id.to_string(),
            to_id: to_id.to_string(),
            kind: kind.to_string(),
            props: serde_json::Value::Null,
        }
    }

    /// Set additional properties.
    pub fn with_props(mut self, props: serde_json::Value) -> Self {
        self.props = props;
        self
    }
}

// ============================================================================
// LanguageExtractor Trait
// ============================================================================

/// Trait for language-specific symbol and relation extraction.
pub trait LanguageExtractor {
    /// Get the language this extractor handles.
    fn language(&self) -> LanguageKind;

    /// Detect if a specific framework is used in this file.
    fn detect_framework(&self, file_path: &str, text: &str) -> FrameworkHint;

    /// Extract symbol candidates from source code.
    fn extract_symbols(&self, file_path: &str, text: &str) -> Vec<KgSymbolCandidate>;

    /// Extract relation candidates from source code.
    fn extract_relations(&self, file_path: &str, text: &str) -> Vec<KgRelationCandidate>;
}

// ============================================================================
// Helper Functions
// ============================================================================

/// Determine the language from a file extension.
pub fn language_from_extension(ext: &str) -> LanguageKind {
    match ext.to_lowercase().as_str() {
        // JavaScript/TypeScript
        "js" | "jsx" | "ts" | "tsx" | "mjs" | "cjs" => LanguageKind::JsTs,
        // Python
        "py" | "pyw" | "pyi" => LanguageKind::Python,
        // Ruby
        "rb" | "rake" | "gemspec" => LanguageKind::Ruby,
        // C#
        "cs" => LanguageKind::CSharp,
        // Java
        "java" => LanguageKind::Java,
        // Markdown
        "md" | "markdown" => LanguageKind::Markdown,
        // Rust
        "rs" => LanguageKind::Rust,
        // C
        "c" | "h" => LanguageKind::C,
        // C++
        "cpp" | "hpp" | "cc" | "cxx" | "hxx" | "hh" => LanguageKind::Cpp,
        // SQL
        "sql" => LanguageKind::Sql,
        // PHP
        "php" | "phtml" => LanguageKind::Php,
        // Go
        "go" => LanguageKind::Go,
        // Kotlin
        "kt" | "kts" => LanguageKind::Kotlin,
        // CSS / SCSS / Tailwind
        "css" | "scss" | "sass" | "less" | "postcss" => LanguageKind::Css,
        // HTML
        "html" | "htm" => LanguageKind::Html,
        // Unknown
        _ => LanguageKind::Unknown,
    }
}

/// Determine language from a file path.
pub fn language_from_path(file_path: &str) -> LanguageKind {
    Path::new(file_path)
        .extension()
        .and_then(|e| e.to_str())
        .map(language_from_extension)
        .unwrap_or(LanguageKind::Unknown)
}

/// Get the language tag from a file extension.
///
/// This differs from `LanguageKind::tag()` in that it returns a more specific
/// tag for TypeScript files (`ts`) vs JavaScript files (`js`), rather than
/// grouping them under the same `JsTs` language kind.
pub fn tag_from_extension(ext: &str) -> &'static str {
    match ext.to_lowercase().as_str() {
        // TypeScript
        "ts" | "tsx" | "mts" | "cts" => "ts",
        // JavaScript
        "js" | "jsx" | "mjs" | "cjs" => "js",
        // Python
        "py" | "pyw" | "pyi" => "py",
        // Ruby
        "rb" | "rake" | "gemspec" => "rb",
        // C#
        "cs" => "cs",
        // Java
        "java" => "java",
        // Markdown
        "md" | "markdown" => "md",
        // Rust
        "rs" => "rs",
        // C
        "c" | "h" => "c",
        // C++
        "cpp" | "hpp" | "cc" | "cxx" | "hxx" | "hh" => "cpp",
        // SQL
        "sql" => "sql",
        // PHP
        "php" | "phtml" => "php",
        // Go
        "go" => "go",
        // Kotlin
        "kt" | "kts" => "kt",
        // CSS / SCSS / Tailwind
        "css" | "scss" | "sass" | "less" | "postcss" => "css",
        // HTML
        "html" | "htm" => "html",
        // Unknown
        _ => "unknown",
    }
}

/// Get the language tag from a file path.
///
/// Returns a specific tag for TypeScript (`ts`) vs JavaScript (`js`),
/// rather than the generic `js` tag from `LanguageKind::tag()`.
pub fn tag_from_path(file_path: &str) -> &'static str {
    Path::new(file_path)
        .extension()
        .and_then(|e| e.to_str())
        .map(tag_from_extension)
        .unwrap_or("unknown")
}

/// Extract symbols and relations for a file based on its extension.
///
/// This is the main entry point for language-specific extraction.
pub fn extract_for_file(
    file_path: &str,
    text: &str,
) -> (Vec<KgSymbolCandidate>, Vec<KgRelationCandidate>) {
    let lang = language_from_path(file_path);

    match lang {
        LanguageKind::JsTs => {
            let extractor = js_ts_lang::JsTsExtractor::new();
            (
                extractor.extract_symbols(file_path, text),
                extractor.extract_relations(file_path, text),
            )
        }
        LanguageKind::Python => {
            let extractor = python_lang::PythonExtractor::new();
            (
                extractor.extract_symbols(file_path, text),
                extractor.extract_relations(file_path, text),
            )
        }
        LanguageKind::Ruby => {
            let extractor = ruby_lang::RubyExtractor::new();
            (
                extractor.extract_symbols(file_path, text),
                extractor.extract_relations(file_path, text),
            )
        }
        LanguageKind::CSharp => {
            let extractor = csharp_lang::CSharpExtractor::new();
            (
                extractor.extract_symbols(file_path, text),
                extractor.extract_relations(file_path, text),
            )
        }
        LanguageKind::Java => {
            let extractor = java_lang::JavaExtractor::new();
            (
                extractor.extract_symbols(file_path, text),
                extractor.extract_relations(file_path, text),
            )
        }
        LanguageKind::Markdown => {
            let extractor = markdown_lang::MarkdownExtractor::new();
            (
                extractor.extract_symbols(file_path, text),
                extractor.extract_relations(file_path, text),
            )
        }
        LanguageKind::Rust => {
            let extractor = rust_lang::RustExtractor::new();
            (
                extractor.extract_symbols(file_path, text),
                extractor.extract_relations(file_path, text),
            )
        }
        LanguageKind::C => {
            let extractor = c_lang::CExtractor::new();
            (
                extractor.extract_symbols(file_path, text),
                extractor.extract_relations(file_path, text),
            )
        }
        LanguageKind::Cpp => {
            let extractor = cpp_lang::CppExtractor::new();
            (
                extractor.extract_symbols(file_path, text),
                extractor.extract_relations(file_path, text),
            )
        }
        LanguageKind::Sql => {
            let extractor = sql_lang::SqlExtractor::new();
            (
                extractor.extract_symbols(file_path, text),
                extractor.extract_relations(file_path, text),
            )
        }
        LanguageKind::Php => {
            let extractor = php_lang::PhpExtractor::new();
            (
                extractor.extract_symbols(file_path, text),
                extractor.extract_relations(file_path, text),
            )
        }
        LanguageKind::Go => {
            let extractor = go_lang::GoExtractor::new();
            (
                extractor.extract_symbols(file_path, text),
                extractor.extract_relations(file_path, text),
            )
        }
        LanguageKind::Kotlin => {
            let extractor = kotlin_lang::KotlinExtractor::new();
            (
                extractor.extract_symbols(file_path, text),
                extractor.extract_relations(file_path, text),
            )
        }
        LanguageKind::Css => {
            let extractor = css_lang::CssTailwindExtractor::new();
            (
                extractor.extract_symbols(file_path, text),
                extractor.extract_relations(file_path, text),
            )
        }
        LanguageKind::Html => {
            let extractor = html_lang::HtmlExtractor::new();
            (
                extractor.extract_symbols(file_path, text),
                extractor.extract_relations(file_path, text),
            )
        }
        LanguageKind::Unknown => (Vec::new(), Vec::new()),
    }
}

/// Deduplicate symbol IDs by appending #<index> suffixes.
///
/// When multiple symbols have the same ID (same lang, file, kind, name),
/// this function appends #1, #2, etc. to make them unique.
pub fn deduplicate_symbol_ids(symbols: &mut [KgSymbolCandidate]) {
    let mut id_counts: HashMap<String, usize> = HashMap::new();

    for symbol in symbols.iter_mut() {
        let count = id_counts.entry(symbol.id.clone()).or_insert(0);
        *count += 1;

        if *count > 1 {
            symbol.id = format!("{}#{}", symbol.id, count);
        }
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_language_from_extension() {
        assert_eq!(language_from_extension("ts"), LanguageKind::JsTs);
        assert_eq!(language_from_extension("tsx"), LanguageKind::JsTs);
        assert_eq!(language_from_extension("js"), LanguageKind::JsTs);
        assert_eq!(language_from_extension("py"), LanguageKind::Python);
        assert_eq!(language_from_extension("rb"), LanguageKind::Ruby);
        assert_eq!(language_from_extension("cs"), LanguageKind::CSharp);
        assert_eq!(language_from_extension("java"), LanguageKind::Java);
        assert_eq!(language_from_extension("md"), LanguageKind::Markdown);
        assert_eq!(language_from_extension("rs"), LanguageKind::Rust);
        assert_eq!(language_from_extension("c"), LanguageKind::C);
        assert_eq!(language_from_extension("cpp"), LanguageKind::Cpp);
        assert_eq!(language_from_extension("sql"), LanguageKind::Sql);
        assert_eq!(language_from_extension("php"), LanguageKind::Php);
        assert_eq!(language_from_extension("go"), LanguageKind::Go);
        assert_eq!(language_from_extension("kt"), LanguageKind::Kotlin);
        assert_eq!(language_from_extension("css"), LanguageKind::Css);
        assert_eq!(language_from_extension("scss"), LanguageKind::Css);
        assert_eq!(language_from_extension("html"), LanguageKind::Html);
        assert_eq!(language_from_extension("htm"), LanguageKind::Html);
        assert_eq!(language_from_extension("xyz"), LanguageKind::Unknown);
    }

    #[test]
    fn test_language_from_path() {
        assert_eq!(
            language_from_path("src/components/Button.tsx"),
            LanguageKind::JsTs
        );
        assert_eq!(language_from_path("lib/utils.py"), LanguageKind::Python);
        assert_eq!(language_from_path("docs/README.md"), LanguageKind::Markdown);
    }

    #[test]
    fn test_language_kind_tag() {
        assert_eq!(LanguageKind::JsTs.tag(), "js");
        assert_eq!(LanguageKind::Python.tag(), "py");
        assert_eq!(LanguageKind::Rust.tag(), "rs");
    }

    #[test]
    fn test_tag_from_extension() {
        // TypeScript extensions should return "ts"
        assert_eq!(tag_from_extension("ts"), "ts");
        assert_eq!(tag_from_extension("tsx"), "ts");
        assert_eq!(tag_from_extension("mts"), "ts");
        assert_eq!(tag_from_extension("cts"), "ts");
        // JavaScript extensions should return "js"
        assert_eq!(tag_from_extension("js"), "js");
        assert_eq!(tag_from_extension("jsx"), "js");
        assert_eq!(tag_from_extension("mjs"), "js");
        assert_eq!(tag_from_extension("cjs"), "js");
        // Other languages
        assert_eq!(tag_from_extension("py"), "py");
        assert_eq!(tag_from_extension("rs"), "rs");
        assert_eq!(tag_from_extension("unknown_ext"), "unknown");
    }

    #[test]
    fn test_tag_from_path() {
        // TypeScript files should return "ts"
        assert_eq!(tag_from_path("src/components/Button.tsx"), "ts");
        assert_eq!(tag_from_path("src/data/plans.ts"), "ts");
        // JavaScript files should return "js"
        assert_eq!(tag_from_path("src/utils.js"), "js");
        assert_eq!(tag_from_path("src/App.jsx"), "js");
        // Other languages
        assert_eq!(tag_from_path("lib/utils.py"), "py");
        assert_eq!(tag_from_path("src/main.rs"), "rs");
    }

    #[test]
    fn test_symbol_candidate_creation() {
        // TypeScript file should use 'ts' tag
        let sym = KgSymbolCandidate::new(
            "function",
            "handleClick",
            LanguageKind::JsTs,
            "src/Button.tsx",
        );
        assert_eq!(sym.kind, "function");
        assert_eq!(sym.name, "handleClick");
        assert_eq!(sym.language, LanguageKind::JsTs);
        assert!(sym.id.starts_with("sym:ts:"), "Expected 'sym:ts:' prefix for .tsx file, got: {}", sym.id);
        assert!(sym.id.contains("function:handleClick"));

        // JavaScript file should use 'js' tag
        let sym_js = KgSymbolCandidate::new(
            "function",
            "handleClick",
            LanguageKind::JsTs,
            "src/Button.jsx",
        );
        assert!(sym_js.id.starts_with("sym:js:"), "Expected 'sym:js:' prefix for .jsx file, got: {}", sym_js.id);
    }

    #[test]
    fn test_relation_candidate_creation() {
        // Use ts tag for .ts files
        let rel = KgRelationCandidate::new("file:a.ts", "sym:ts:a.ts:function:foo", "defines");
        assert_eq!(rel.from_id, "file:a.ts");
        assert_eq!(rel.to_id, "sym:ts:a.ts:function:foo");
        assert_eq!(rel.kind, "defines");
    }

    #[test]
    fn test_deduplicate_symbol_ids() {
        let mut symbols = vec![
            KgSymbolCandidate::new("function", "foo", LanguageKind::JsTs, "src/utils.ts"),
            KgSymbolCandidate::new("function", "foo", LanguageKind::JsTs, "src/utils.ts"),
            KgSymbolCandidate::new("function", "foo", LanguageKind::JsTs, "src/utils.ts"),
        ];

        deduplicate_symbol_ids(&mut symbols);

        // TypeScript files should use 'ts' tag
        assert_eq!(symbols[0].id, "sym:ts:src/utils.ts:function:foo");
        assert_eq!(symbols[1].id, "sym:ts:src/utils.ts:function:foo#2");
        assert_eq!(symbols[2].id, "sym:ts:src/utils.ts:function:foo#3");
    }

    // ========================================================================
    // Phase 9.2.2 Frontend Integration Tests
    // ========================================================================

    #[test]
    fn test_extract_for_file_css() {
        let css = r#"
.btn { padding: 1rem; }
#header { height: 60px; }
--primary: blue;
"#;

        let (symbols, relations) = extract_for_file("styles.css", css);

        // Should have CSS symbols
        assert!(!symbols.is_empty(), "Should extract CSS symbols");
        assert!(
            symbols.iter().any(|s| s.kind == "styleClass"),
            "Should have styleClass symbols"
        );

        // CSS doesn't produce relations
        assert!(relations.is_empty(), "CSS should not produce relations");

        // Check language
        assert!(
            symbols.iter().all(|s| s.language == LanguageKind::Css),
            "All symbols should be CSS"
        );
    }

    #[test]
    fn test_extract_for_file_html() {
        let html = r##"
<!DOCTYPE html>
<html>
<body>
    <header id="main-header" class="flex">
        <a href="#about">About</a>
    </header>
    <section id="content" class="container">
        <p class="text-lg">Hello</p>
        <div id="inner-div">Some content</div>
    </section>
</body>
</html>
"##;

        let (symbols, relations) = extract_for_file("page.html", html);

        // Should have HTML symbols
        assert!(!symbols.is_empty(), "Should extract HTML symbols");
        assert!(
            symbols.iter().any(|s| s.kind == "htmlTemplate"),
            "Should have htmlTemplate"
        );

        // Section elements with IDs become htmlSection
        assert!(
            symbols
                .iter()
                .any(|s| s.kind == "htmlSection" && s.name == "main-header"),
            "Should have htmlSection for main-header"
        );
        assert!(
            symbols
                .iter()
                .any(|s| s.kind == "htmlSection" && s.name == "content"),
            "Should have htmlSection for content"
        );

        // Non-section elements with IDs become htmlAnchor
        assert!(
            symbols
                .iter()
                .any(|s| s.kind == "htmlAnchor" && s.name == "inner-div"),
            "Should have htmlAnchor for inner-div"
        );

        // Should have usesClass relations
        assert!(
            !relations.is_empty(),
            "HTML should produce usesClass relations"
        );
        assert!(
            relations.iter().all(|r| r.kind == "usesClass"),
            "All HTML relations should be usesClass"
        );

        // Check language
        assert!(
            symbols.iter().all(|s| s.language == LanguageKind::Html),
            "All symbols should be HTML"
        );
    }

    #[test]
    fn test_extract_for_file_react_tsx() {
        let tsx = r#"
import { Button } from "@/components/ui/button";

export function MyComponent() {
    return (
        <div className="flex items-center">
            <Button>Click me</Button>
        </div>
    );
}
"#;

        let (symbols, relations) = extract_for_file("components/MyComponent.tsx", tsx);

        // Should have JS/TS symbols
        assert!(!symbols.is_empty(), "Should extract JS/TS symbols");

        // Check for react component
        assert!(
            symbols.iter().any(|s| s.kind == "reactComponent"),
            "Should have reactComponent"
        );

        // Check for shadcn import
        assert!(
            symbols.iter().any(|s| s.kind == "uiComponent"),
            "Should have uiComponent"
        );

        // Should have className relations
        assert!(
            relations.iter().any(|r| r.kind == "usesClass"),
            "Should have usesClass relations"
        );

        // Should have usesUiComponent relations
        assert!(
            relations.iter().any(|r| r.kind == "usesUiComponent"),
            "Should have usesUiComponent relations"
        );
    }

    #[test]
    fn test_extract_for_file_angular() {
        let ts = r#"
import { Component } from '@angular/core';

@Component({
    selector: 'app-header',
    template: '<header>Header</header>'
})
export class HeaderComponent {
    title = 'My App';
}
"#;

        let (symbols, _relations) = extract_for_file("header.component.ts", ts);

        // Should have Angular component
        assert!(!symbols.is_empty(), "Should extract Angular symbols");
        assert!(
            symbols.iter().any(|s| s.kind == "ngComponent"),
            "Should have ngComponent"
        );
        assert!(
            symbols.iter().any(|s| s.name == "HeaderComponent"),
            "Should have HeaderComponent"
        );
    }

    #[test]
    fn test_extract_css_with_tailwind() {
        let css = r#"
@tailwind base;
@tailwind components;
@tailwind utilities;

.btn {
    @apply px-4 py-2 rounded-md;
}
"#;

        let (symbols, _relations) = extract_for_file("styles/globals.css", css);

        // Check for Tailwind directives
        assert!(
            symbols.iter().any(|s| s.kind == "tailwindDirective"),
            "Should extract @tailwind directives"
        );

        // Check for class selectors
        assert!(
            symbols
                .iter()
                .any(|s| s.kind == "styleClass" && s.name == "btn"),
            "Should extract .btn class"
        );
    }
}
