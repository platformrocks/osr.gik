//! Query expansion module for improving semantic search recall.
//!
//! This module provides strategies to expand abstract or high-level queries
//! into more concrete terms that are likely to match indexed code.
//!
//! ## Strategies
//!
//! 1. **Abstract Term Mapping**: Maps conceptual terms like "architecture"
//!    to concrete code patterns like "structure", "layout", "design".
//!
//! 2. **Stack-Aware Expansion**: Uses detected stack technologies to add
//!    framework-specific terms (e.g., "components" → "tsx", "react" for React projects).
//!
//! 3. **Multi-Query Embedding**: Generates multiple query variants and
//!    averages their embeddings for better coverage.
//!
//! ## Example
//!
//! ```ignore
//! use gik_core::query_expansion::{QueryExpander, ExpansionConfig};
//! use gik_core::ask::StackSummary;
//!
//! let config = ExpansionConfig::default();
//! let expander = QueryExpander::new(config);
//!
//! // Without stack context
//! let variants = expander.expand("How is the project architecture organized?");
//! // Returns: ["How is the project architecture organized?",
//! //          "How is the project structure organized?",
//! //          "How is the project layout organized?"]
//!
//! // With stack context
//! let stack = StackSummary { languages: vec!["TypeScript".into()], frameworks: vec!["React".into()], .. };
//! let variants = expander.expand_with_stack("Where are the components?", &stack);
//! // For React project, returns: ["Where are the components?",
//! //                              "Where are the tsx files?",
//! //                              "Where are the react components?"]
//! ```

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::ask::StackSummary;

// ============================================================================
// Configuration
// ============================================================================

/// Configuration for query expansion behavior.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExpansionConfig {
    /// Maximum number of query variants to generate.
    pub max_variants: usize,

    /// Whether to include the original query in variants.
    pub include_original: bool,

    /// Whether to use stack-aware expansion.
    pub use_stack_context: bool,

    /// Whether to expand abstract terms.
    pub expand_abstract_terms: bool,
}

impl Default for ExpansionConfig {
    fn default() -> Self {
        Self {
            max_variants: 4,
            include_original: true,
            use_stack_context: true,
            expand_abstract_terms: true,
        }
    }
}

// ============================================================================
// Abstract Term Mappings (Hardcoded Phase 1)
// ============================================================================

/// Returns the hardcoded mapping of abstract terms to concrete alternatives.
///
/// These mappings are based on common patterns observed in software projects
/// and are designed to improve recall for high-level conceptual queries.
fn abstract_term_mappings() -> HashMap<&'static str, Vec<&'static str>> {
    let mut mappings = HashMap::new();

    // Architecture & Structure
    mappings.insert(
        "architecture",
        vec!["structure", "layout", "design", "organization", "folder"],
    );
    mappings.insert("structure", vec!["layout", "organization", "hierarchy"]);
    mappings.insert("organization", vec!["structure", "layout", "folder"]);
    mappings.insert("design", vec!["architecture", "pattern", "structure"]);

    // Code patterns
    mappings.insert("pattern", vec!["design", "approach", "method", "strategy"]);
    mappings.insert(
        "approach",
        vec!["pattern", "method", "strategy", "technique"],
    );
    mappings.insert("strategy", vec!["pattern", "approach", "method"]);

    // Data flow
    mappings.insert("flow", vec!["pipeline", "process", "sequence", "stream"]);
    mappings.insert("pipeline", vec!["flow", "process", "chain", "workflow"]);
    mappings.insert("workflow", vec!["flow", "process", "pipeline", "sequence"]);

    // Components & Modules
    mappings.insert(
        "components",
        vec!["widgets", "elements", "ui", "views", "parts"],
    );
    mappings.insert("modules", vec!["packages", "libraries", "crates", "units"]);
    mappings.insert("features", vec!["functionality", "capabilities", "modules"]);

    // Connections & Relations
    mappings.insert(
        "connections",
        vec![
            "relationships",
            "links",
            "dependencies",
            "imports",
            "references",
        ],
    );
    mappings.insert(
        "relationships",
        vec!["connections", "links", "associations", "bindings"],
    );
    mappings.insert(
        "dependencies",
        vec!["imports", "requires", "packages", "libraries"],
    );

    // Styling
    mappings.insert(
        "styling",
        vec!["styles", "css", "theme", "appearance", "visual", "tailwind"],
    );
    mappings.insert("styles", vec!["css", "theme", "classes", "design"]);
    mappings.insert("theme", vec!["styles", "colors", "appearance", "design"]);
    mappings.insert(
        "animations",
        vec!["transitions", "effects", "motion", "keyframes", "animate"],
    );

    // Configuration
    mappings.insert(
        "configuration",
        vec!["config", "settings", "options", "env"],
    );
    mappings.insert("settings", vec!["config", "options", "preferences"]);
    mappings.insert("config", vec!["configuration", "settings", "options"]);

    // Authentication
    mappings.insert(
        "authentication",
        vec!["auth", "login", "session", "credentials", "jwt"],
    );
    mappings.insert("auth", vec!["authentication", "login", "session", "token"]);
    mappings.insert(
        "authorization",
        vec!["permissions", "roles", "access", "rbac"],
    );

    // Database
    mappings.insert("database", vec!["db", "storage", "persistence", "data"]);
    mappings.insert("schema", vec!["model", "entity", "table", "structure"]);
    mappings.insert("model", vec!["schema", "entity", "type", "interface"]);

    // API
    mappings.insert("api", vec!["endpoints", "routes", "handlers", "services"]);
    mappings.insert("endpoints", vec!["api", "routes", "handlers", "paths"]);
    mappings.insert("routes", vec!["endpoints", "paths", "handlers", "api"]);

    // Testing
    mappings.insert("tests", vec!["testing", "specs", "unit", "integration"]);
    mappings.insert("testing", vec!["tests", "specs", "assertions", "mocks"]);

    // Types & Interfaces
    mappings.insert(
        "types",
        vec!["interfaces", "schemas", "definitions", "models"],
    );
    mappings.insert(
        "interfaces",
        vec!["types", "contracts", "protocols", "apis"],
    );

    // State management
    mappings.insert(
        "state",
        vec!["store", "context", "redux", "zustand", "data"],
    );
    mappings.insert("store", vec!["state", "context", "data", "cache"]);

    mappings
}

// ============================================================================
// Stack-Aware Expansions
// ============================================================================

/// Returns stack-specific term expansions based on detected technologies.
fn stack_term_expansions(stack: &StackSummary) -> HashMap<&'static str, Vec<&'static str>> {
    let mut expansions = HashMap::new();

    // Collect all tech indicators (languages + frameworks + managers)
    let mut techs: Vec<String> = Vec::new();
    techs.extend(stack.languages.iter().map(|s| s.to_lowercase()));
    techs.extend(stack.frameworks.iter().map(|s| s.to_lowercase()));
    techs.extend(stack.managers.iter().map(|s| s.to_lowercase()));

    // React/Next.js specific
    if techs
        .iter()
        .any(|t| t.contains("react") || t.contains("next"))
    {
        expansions.insert("components", vec!["tsx", "jsx", "react", "component"]);
        expansions.insert("pages", vec!["app", "routes", "page.tsx", "layout"]);
        expansions.insert("hooks", vec!["useState", "useEffect", "use", "custom"]);
        expansions.insert("state", vec!["context", "provider", "store", "zustand"]);
    }

    // Vue specific
    if techs.iter().any(|t| t.contains("vue")) {
        expansions.insert("components", vec!["vue", "component", "template"]);
        expansions.insert("state", vec!["pinia", "vuex", "store", "composable"]);
    }

    // Angular specific
    if techs.iter().any(|t| t.contains("angular")) {
        expansions.insert("components", vec!["component.ts", "template", "module"]);
        expansions.insert("services", vec!["service.ts", "injectable", "provider"]);
        expansions.insert("state", vec!["ngrx", "store", "service", "observable"]);
    }

    // Tailwind specific
    if techs.iter().any(|t| t.contains("tailwind")) {
        expansions.insert("styling", vec!["tailwind", "className", "tw", "classes"]);
        expansions.insert("styles", vec!["tailwind.config", "className", "utility"]);
        expansions.insert(
            "animations",
            vec!["animate", "transition", "duration", "ease"],
        );
    }

    // Prisma specific
    if techs.iter().any(|t| t.contains("prisma")) {
        expansions.insert("database", vec!["prisma", "schema.prisma", "model", "db"]);
        expansions.insert("schema", vec!["prisma", "model", "relation", "@@"]);
    }

    // TypeScript specific
    if techs.iter().any(|t| t.contains("typescript")) {
        expansions.insert("types", vec!["interface", "type", ".d.ts", "generic"]);
    }

    // Rust specific
    if techs.iter().any(|t| t.contains("rust")) {
        expansions.insert("modules", vec!["mod.rs", "crate", "pub mod"]);
        expansions.insert("types", vec!["struct", "enum", "trait", "impl"]);
    }

    // Python specific
    if techs.iter().any(|t| t.contains("python")) {
        expansions.insert("modules", vec!["__init__.py", "import", "package"]);
        expansions.insert("types", vec!["class", "dataclass", "TypedDict", "Protocol"]);
    }

    expansions
}

// ============================================================================
// Query Expander
// ============================================================================

/// Expands queries to improve semantic search recall.
///
/// The expander generates multiple query variants by:
/// 1. Detecting abstract terms in the query
/// 2. Substituting them with concrete alternatives
/// 3. Optionally using stack context for framework-specific terms
#[derive(Debug, Clone)]
pub struct QueryExpander {
    config: ExpansionConfig,
    abstract_mappings: HashMap<&'static str, Vec<&'static str>>,
}

impl QueryExpander {
    /// Creates a new query expander with the given configuration.
    pub fn new(config: ExpansionConfig) -> Self {
        Self {
            config,
            abstract_mappings: abstract_term_mappings(),
        }
    }

    /// Creates a query expander with default configuration.
    pub fn with_defaults() -> Self {
        Self::new(ExpansionConfig::default())
    }

    /// Expands a query into multiple variants without stack context.
    pub fn expand(&self, query: &str) -> Vec<String> {
        self.expand_with_stack_opt(query, None)
    }

    /// Expands a query into multiple variants with stack context.
    pub fn expand_with_stack(&self, query: &str, stack: &StackSummary) -> Vec<String> {
        self.expand_with_stack_opt(query, Some(stack))
    }

    /// Internal expansion with optional stack context.
    fn expand_with_stack_opt(&self, query: &str, stack: Option<&StackSummary>) -> Vec<String> {
        let mut variants = Vec::new();

        // Always include original if configured
        if self.config.include_original {
            variants.push(query.to_string());
        }

        // Get combined mappings (abstract + stack-aware)
        let mut mappings = if self.config.expand_abstract_terms {
            self.abstract_mappings.clone()
        } else {
            HashMap::new()
        };

        // Add stack-specific mappings if available
        if self.config.use_stack_context {
            if let Some(s) = stack {
                for (term, expansions) in stack_term_expansions(s) {
                    mappings
                        .entry(term)
                        .or_insert_with(Vec::new)
                        .extend(expansions);
                }
            }
        }

        // Find terms in query that have mappings
        let query_lower = query.to_lowercase();
        let mut found_terms: Vec<(&str, &Vec<&str>)> = Vec::new();

        for (term, alternatives) in &mappings {
            if query_lower.contains(*term) {
                found_terms.push((term, alternatives));
            }
        }

        // Generate variants by substituting found terms
        for (term, alternatives) in found_terms {
            for alt in alternatives.iter().take(self.config.max_variants - 1) {
                // Case-insensitive replacement
                let variant = replace_case_insensitive(query, term, alt);
                if variant != query && !variants.contains(&variant) {
                    variants.push(variant);
                }

                // Stop if we have enough variants
                if variants.len() >= self.config.max_variants {
                    return variants;
                }
            }
        }

        // Ensure we have at least the original
        if variants.is_empty() {
            variants.push(query.to_string());
        }

        variants
    }

    /// Returns the configuration.
    pub fn config(&self) -> &ExpansionConfig {
        &self.config
    }
}

impl Default for QueryExpander {
    fn default() -> Self {
        Self::with_defaults()
    }
}

// ============================================================================
// Helpers
// ============================================================================

/// Replaces a term in a string case-insensitively, preserving surrounding case.
fn replace_case_insensitive(text: &str, from: &str, to: &str) -> String {
    let lower_text = text.to_lowercase();
    let lower_from = from.to_lowercase();

    if let Some(pos) = lower_text.find(&lower_from) {
        let mut result = String::with_capacity(text.len() + to.len() - from.len());
        result.push_str(&text[..pos]);
        result.push_str(to);
        result.push_str(&text[pos + from.len()..]);
        result
    } else {
        text.to_string()
    }
}

// ============================================================================
// Multi-Query Embedding Strategy
// ============================================================================

/// Averages multiple embedding vectors into a single vector.
///
/// This is used for multi-query expansion where we embed multiple
/// query variants and average their embeddings for better coverage.
pub fn average_embeddings(embeddings: &[Vec<f32>]) -> Option<Vec<f32>> {
    if embeddings.is_empty() {
        return None;
    }

    let dim = embeddings[0].len();
    if dim == 0 {
        return None;
    }

    // Verify all embeddings have the same dimension
    if !embeddings.iter().all(|e| e.len() == dim) {
        return None;
    }

    let count = embeddings.len() as f32;
    let mut avg = vec![0.0f32; dim];

    for embedding in embeddings {
        for (i, &val) in embedding.iter().enumerate() {
            avg[i] += val;
        }
    }

    for val in &mut avg {
        *val /= count;
    }

    // L2 normalize the result
    let norm: f32 = avg.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm > 0.0 {
        for val in &mut avg {
            *val /= norm;
        }
    }

    Some(avg)
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = ExpansionConfig::default();
        assert_eq!(config.max_variants, 4);
        assert!(config.include_original);
        assert!(config.use_stack_context);
        assert!(config.expand_abstract_terms);
    }

    #[test]
    fn test_expand_architecture() {
        let expander = QueryExpander::with_defaults();
        let variants = expander.expand("How is the project architecture organized?");

        assert!(!variants.is_empty());
        assert!(variants.contains(&"How is the project architecture organized?".to_string()));
        // Should have variants with "structure", "layout", etc.
        assert!(variants.len() > 1);

        // Check that at least one alternative was generated
        let has_alternative = variants
            .iter()
            .any(|v| v.contains("structure") || v.contains("layout") || v.contains("design"));
        assert!(
            has_alternative,
            "Should have alternative terms for 'architecture'"
        );
    }

    #[test]
    fn test_expand_components() {
        let expander = QueryExpander::with_defaults();
        let variants = expander.expand("Where are the components defined?");

        assert!(!variants.is_empty());
        assert!(variants.contains(&"Where are the components defined?".to_string()));
    }

    #[test]
    fn test_expand_animations() {
        let expander = QueryExpander::with_defaults();
        let variants = expander.expand("What custom animations are defined?");

        assert!(!variants.is_empty());
        // Should have variants with "transitions", "effects", etc.
        let has_alternative = variants.iter().any(|v| {
            v.contains("transitions")
                || v.contains("effects")
                || v.contains("motion")
                || v.contains("keyframes")
        });
        assert!(
            has_alternative,
            "Should have alternative terms for 'animations'"
        );
    }

    #[test]
    fn test_expand_no_match() {
        let expander = QueryExpander::with_defaults();
        let variants = expander.expand("What is 2 + 2?");

        // Should still return at least the original
        assert_eq!(variants.len(), 1);
        assert_eq!(variants[0], "What is 2 + 2?");
    }

    #[test]
    fn test_expand_without_original() {
        let config = ExpansionConfig {
            include_original: false,
            ..Default::default()
        };
        let expander = QueryExpander::new(config);
        let variants = expander.expand("How is the architecture?");

        // Should not contain original (unless there's no expansion)
        // If expansion happened, original should not be first
        if variants.len() > 1 {
            assert!(!variants.iter().any(|v| v == "How is the architecture?"));
        }
    }

    #[test]
    fn test_replace_case_insensitive() {
        assert_eq!(
            replace_case_insensitive("Hello World", "world", "universe"),
            "Hello universe"
        );
        assert_eq!(
            replace_case_insensitive("ARCHITECTURE is good", "architecture", "structure"),
            "structure is good"
        );
        assert_eq!(
            replace_case_insensitive("no match here", "xyz", "abc"),
            "no match here"
        );
    }

    #[test]
    fn test_average_embeddings() {
        let embeddings = vec![
            vec![1.0, 0.0, 0.0],
            vec![0.0, 1.0, 0.0],
            vec![0.0, 0.0, 1.0],
        ];

        let avg = average_embeddings(&embeddings).unwrap();
        assert_eq!(avg.len(), 3);

        // After averaging and normalizing, should be close to [0.577, 0.577, 0.577]
        let expected = 1.0 / 3.0_f32.sqrt();
        for val in &avg {
            assert!((val - expected).abs() < 0.01);
        }
    }

    #[test]
    fn test_average_embeddings_empty() {
        assert!(average_embeddings(&[]).is_none());
    }

    #[test]
    fn test_average_embeddings_single() {
        let embeddings = vec![vec![0.6, 0.8, 0.0]];
        let avg = average_embeddings(&embeddings).unwrap();

        // Should be normalized version of input
        let norm = (0.6_f32.powi(2) + 0.8_f32.powi(2)).sqrt();
        assert!((avg[0] - 0.6 / norm).abs() < 0.01);
        assert!((avg[1] - 0.8 / norm).abs() < 0.01);
    }

    #[test]
    fn test_max_variants_limit() {
        let config = ExpansionConfig {
            max_variants: 2,
            include_original: true,
            ..Default::default()
        };
        let expander = QueryExpander::new(config);
        let variants = expander.expand("architecture and components and styling");

        // Should be limited to max_variants
        assert!(variants.len() <= 2);
    }
}
