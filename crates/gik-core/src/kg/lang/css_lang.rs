//! CSS and Tailwind extractor for KG symbol extraction.
//!
//! Extracts style-related symbols from CSS, SCSS, SASS, and PostCSS files:
//!
//! - **styleClass**: CSS class selectors (`.btn-primary`)
//! - **styleId**: CSS ID selectors (`#main-header`)
//! - **cssVariable**: CSS custom properties (`--primary-color`)
//!
//! Also captures Tailwind-specific patterns like `@apply` directives.

use regex::Regex;

use super::{
    FrameworkHint, KgRelationCandidate, KgSymbolCandidate, LanguageExtractor, LanguageKind,
};

/// Extractor for CSS, SCSS, and Tailwind files.
pub struct CssTailwindExtractor {
    // Compiled regex patterns
    css_variable_re: Regex,
    apply_directive_re: Regex,
    tailwind_layer_re: Regex,
}

impl CssTailwindExtractor {
    /// Create a new CSS/Tailwind extractor.
    pub fn new() -> Self {
        Self {
            // Match CSS custom properties --variable-name
            css_variable_re: Regex::new(r"--([a-zA-Z][a-zA-Z0-9_-]*)\s*:")
                .expect("Invalid CSS variable regex"),
            // Match @apply directive with class list
            apply_directive_re: Regex::new(r"@apply\s+([^;]+);").expect("Invalid @apply regex"),
            // Match @layer directive
            tailwind_layer_re: Regex::new(r"@layer\s+(base|components|utilities)")
                .expect("Invalid @layer regex"),
        }
    }

    /// Detect if Tailwind is being used in this file.
    fn detect_tailwind(&self, text: &str) -> bool {
        // Check for Tailwind-specific directives
        text.contains("@tailwind")
            || text.contains("@apply")
            || self.tailwind_layer_re.is_match(text)
    }

    /// Extract CSS class selectors.
    fn extract_classes(
        &self,
        file_path: &str,
        text: &str,
        framework: FrameworkHint,
    ) -> Vec<KgSymbolCandidate> {
        let mut symbols = Vec::new();
        let mut seen = std::collections::HashSet::new();

        // Match classes in various contexts: .class, .class:hover, .class > .other, .class.class2
        let complex_class_re = Regex::new(r"\.([a-zA-Z_][a-zA-Z0-9_-]*)").expect("Invalid regex");

        for cap in complex_class_re.captures_iter(text) {
            if let Some(class_name) = cap.get(1) {
                let name = class_name.as_str();
                // Skip if already seen
                if seen.contains(name) {
                    continue;
                }
                seen.insert(name.to_string());

                // Skip pseudo-elements and pseudo-classes that look like class names
                if name == "hover"
                    || name == "focus"
                    || name == "active"
                    || name == "before"
                    || name == "after"
                    || name == "first-child"
                    || name == "last-child"
                    || name == "not"
                {
                    continue;
                }

                let sym = KgSymbolCandidate::new("styleClass", name, LanguageKind::Css, file_path)
                    .with_framework(framework)
                    .with_prop("selector".to_string(), format!(".{}", name))
                    .with_prop("selectorType".to_string(), "class".to_string());
                symbols.push(sym);
            }
        }

        symbols
    }

    /// Extract CSS ID selectors.
    fn extract_ids(
        &self,
        file_path: &str,
        text: &str,
        framework: FrameworkHint,
    ) -> Vec<KgSymbolCandidate> {
        let mut symbols = Vec::new();
        let mut seen = std::collections::HashSet::new();

        // Match #id in various contexts
        let id_re =
            Regex::new(r"#([a-zA-Z_][a-zA-Z0-9_-]*)(?:\s*[:{>,\[\s]|$)").expect("Invalid regex");

        for cap in id_re.captures_iter(text) {
            if let Some(id_name) = cap.get(1) {
                let name = id_name.as_str();
                if seen.contains(name) {
                    continue;
                }
                seen.insert(name.to_string());

                let sym = KgSymbolCandidate::new("styleId", name, LanguageKind::Css, file_path)
                    .with_framework(framework)
                    .with_prop("selector".to_string(), format!("#{}", name))
                    .with_prop("selectorType".to_string(), "id".to_string());
                symbols.push(sym);
            }
        }

        symbols
    }

    /// Extract CSS custom properties (variables).
    fn extract_variables(
        &self,
        file_path: &str,
        text: &str,
        framework: FrameworkHint,
    ) -> Vec<KgSymbolCandidate> {
        let mut symbols = Vec::new();
        let mut seen = std::collections::HashSet::new();

        for cap in self.css_variable_re.captures_iter(text) {
            if let Some(var_name) = cap.get(1) {
                let name = var_name.as_str();
                if seen.contains(name) {
                    continue;
                }
                seen.insert(name.to_string());

                let sym = KgSymbolCandidate::new("cssVariable", name, LanguageKind::Css, file_path)
                    .with_framework(framework)
                    .with_prop("variable".to_string(), format!("--{}", name));
                symbols.push(sym);
            }
        }

        symbols
    }

    /// Extract @apply directives and their classes (Tailwind-specific).
    fn extract_apply_classes(&self, file_path: &str, text: &str) -> Vec<(String, Vec<String>)> {
        let mut applies = Vec::new();

        for cap in self.apply_directive_re.captures_iter(text) {
            if let Some(classes_str) = cap.get(1) {
                let classes: Vec<String> = classes_str
                    .as_str()
                    .split_whitespace()
                    .map(|s| s.to_string())
                    .collect();
                if !classes.is_empty() {
                    applies.push((file_path.to_string(), classes));
                }
            }
        }

        applies
    }

    /// Extract @tailwind directives.
    fn extract_tailwind_directives(&self, file_path: &str, text: &str) -> Vec<KgSymbolCandidate> {
        let mut symbols = Vec::new();
        let mut seen = std::collections::HashSet::new();

        // Match @tailwind base|components|utilities|variants
        let directive_re = Regex::new(r"@tailwind\s+([a-zA-Z]+)").expect("Invalid regex");

        for cap in directive_re.captures_iter(text) {
            if let Some(directive) = cap.get(1) {
                let name = directive.as_str();
                if seen.contains(name) {
                    continue;
                }
                seen.insert(name.to_string());

                let sym =
                    KgSymbolCandidate::new("tailwindDirective", name, LanguageKind::Css, file_path)
                        .with_framework(FrameworkHint::Tailwind)
                        .with_prop("directive".to_string(), format!("@tailwind {}", name));
                symbols.push(sym);
            }
        }

        symbols
    }
}

impl Default for CssTailwindExtractor {
    fn default() -> Self {
        Self::new()
    }
}

impl LanguageExtractor for CssTailwindExtractor {
    fn language(&self) -> LanguageKind {
        LanguageKind::Css
    }

    fn detect_framework(&self, _file_path: &str, text: &str) -> FrameworkHint {
        if self.detect_tailwind(text) {
            FrameworkHint::Tailwind
        } else {
            FrameworkHint::None
        }
    }

    fn extract_symbols(&self, file_path: &str, text: &str) -> Vec<KgSymbolCandidate> {
        let mut symbols = Vec::new();
        let framework = self.detect_framework(file_path, text);

        // Extract class selectors
        symbols.extend(self.extract_classes(file_path, text, framework));

        // Extract ID selectors
        symbols.extend(self.extract_ids(file_path, text, framework));

        // Extract CSS variables
        symbols.extend(self.extract_variables(file_path, text, framework));

        // If Tailwind is detected, extract @tailwind directives
        if framework == FrameworkHint::Tailwind {
            symbols.extend(self.extract_tailwind_directives(file_path, text));

            // Add @apply class info to props
            let applies = self.extract_apply_classes(file_path, text);
            for sym in &mut symbols {
                if sym.kind == "styleClass" {
                    // Check if this class uses @apply
                    for (_, classes) in &applies {
                        if classes.iter().any(|c| c == &sym.name) {
                            sym.props = serde_json::json!({
                                "selector": format!(".{}", sym.name),
                                "selectorType": "class",
                                "usedInApply": true
                            });
                        }
                    }
                }
            }
        }

        symbols
    }

    fn extract_relations(&self, _file_path: &str, _text: &str) -> Vec<KgRelationCandidate> {
        // For CSS files, relations are primarily file → symbol (defines).
        // Cross-file usages (component → styleClass) are handled by JS/TS/HTML extractors.
        Vec::new()
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_class_selectors() {
        let css = r#"
.btn-primary {
    background-color: blue;
}

.card {
    padding: 1rem;
}

.btn-primary:hover {
    background-color: darkblue;
}
"#;

        let extractor = CssTailwindExtractor::new();
        let symbols = extractor.extract_symbols("styles/main.css", css);

        let names: Vec<&str> = symbols.iter().map(|s| s.name.as_str()).collect();
        assert!(names.contains(&"btn-primary"), "Should extract btn-primary");
        assert!(names.contains(&"card"), "Should extract card");
    }

    #[test]
    fn test_extract_id_selectors() {
        let css = r#"
#main-header {
    position: fixed;
}

#sidebar {
    width: 250px;
}
"#;

        let extractor = CssTailwindExtractor::new();
        let symbols = extractor.extract_symbols("styles/layout.css", css);

        let ids: Vec<&str> = symbols
            .iter()
            .filter(|s| s.kind == "styleId")
            .map(|s| s.name.as_str())
            .collect();
        assert!(ids.contains(&"main-header"), "Should extract main-header");
        assert!(ids.contains(&"sidebar"), "Should extract sidebar");
    }

    #[test]
    fn test_extract_css_variables() {
        let css = r#"
:root {
    --primary-color: #3b82f6;
    --secondary-color: #10b981;
    --font-size-base: 16px;
}
"#;

        let extractor = CssTailwindExtractor::new();
        let symbols = extractor.extract_symbols("styles/variables.css", css);

        let vars: Vec<&str> = symbols
            .iter()
            .filter(|s| s.kind == "cssVariable")
            .map(|s| s.name.as_str())
            .collect();
        assert!(
            vars.contains(&"primary-color"),
            "Should extract primary-color"
        );
        assert!(
            vars.contains(&"secondary-color"),
            "Should extract secondary-color"
        );
    }

    #[test]
    fn test_detect_tailwind() {
        let tailwind_css = r#"
@tailwind base;
@tailwind components;
@tailwind utilities;

.btn {
    @apply px-4 py-2 rounded bg-blue-500 text-white;
}
"#;

        let plain_css = r#"
.btn {
    padding: 0.5rem 1rem;
    border-radius: 0.25rem;
}
"#;

        let extractor = CssTailwindExtractor::new();
        assert_eq!(
            extractor.detect_framework("styles.css", tailwind_css),
            FrameworkHint::Tailwind
        );
        assert_eq!(
            extractor.detect_framework("styles.css", plain_css),
            FrameworkHint::None
        );
    }

    #[test]
    fn test_extract_apply_classes() {
        let css = r#"
.btn-custom {
    @apply px-4 py-2 rounded-lg bg-blue-500 hover:bg-blue-600;
}

.card-header {
    @apply text-lg font-bold mb-2;
}
"#;

        let extractor = CssTailwindExtractor::new();
        let applies = extractor.extract_apply_classes("styles.css", css);

        assert_eq!(applies.len(), 2);
        assert!(applies[0].1.contains(&"px-4".to_string()));
        assert!(applies[0].1.contains(&"bg-blue-500".to_string()));
        assert!(applies[1].1.contains(&"text-lg".to_string()));
    }

    #[test]
    fn test_complex_selectors() {
        let css = r#"
.container > .row {
    display: flex;
}

.nav-item, .menu-item {
    padding: 0.5rem;
}

.btn.active {
    border: 2px solid blue;
}
"#;

        let extractor = CssTailwindExtractor::new();
        let symbols = extractor.extract_symbols("styles.css", css);

        let names: Vec<&str> = symbols.iter().map(|s| s.name.as_str()).collect();
        assert!(names.contains(&"container"));
        assert!(names.contains(&"row"));
        assert!(names.contains(&"nav-item"));
        assert!(names.contains(&"menu-item"));
        assert!(names.contains(&"btn"));
    }

    #[test]
    fn test_scss_syntax() {
        let scss = r#"
$primary: #3b82f6;

.btn {
    &-primary {
        background: $primary;
    }
    
    &:hover {
        opacity: 0.9;
    }
}

#app {
    .sidebar {
        width: 250px;
    }
}
"#;

        let extractor = CssTailwindExtractor::new();
        let symbols = extractor.extract_symbols("styles.scss", scss);

        // Should extract top-level selectors
        let names: Vec<&str> = symbols.iter().map(|s| s.name.as_str()).collect();
        assert!(names.contains(&"btn"));
        // Note: Nested SCSS selectors like &-primary are not fully expanded
        // This is expected for regex-based extraction
    }
}
