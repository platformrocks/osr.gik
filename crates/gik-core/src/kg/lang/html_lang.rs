//! HTML extractor for KG symbol extraction.
//!
//! Extracts structure and style references from HTML files:
//!
//! - **htmlTemplate**: Root document structure (`<html>`, `<body>`, layout files)
//! - **htmlSection**: Section elements with IDs (`<section id="hero">`)
//! - **htmlAnchor**: Elements with ID attributes for linking
//!
//! Also extracts class and ID usage for linking to CSS symbols.

use regex::Regex;

use super::{
    FrameworkHint, KgRelationCandidate, KgSymbolCandidate, LanguageExtractor, LanguageKind,
};

/// Extractor for HTML and HTM files.
pub struct HtmlExtractor {
    // Compiled regex patterns
    section_id_re: Regex,
    element_id_re: Regex,
    class_attr_re: Regex,
    template_markers_re: Regex,
}

impl HtmlExtractor {
    /// Create a new HTML extractor.
    pub fn new() -> Self {
        Self {
            // Match <section id="..."> or <div id="..."> etc.
            section_id_re: Regex::new(
                r#"<(section|article|aside|header|footer|main|nav)\s+[^>]*id\s*=\s*["']([^"']+)["']"#,
            )
            .expect("Invalid section ID regex"),

            // Match any element with id="..."
            element_id_re: Regex::new(r#"<[a-zA-Z][a-zA-Z0-9]*\s+[^>]*id\s*=\s*["']([^"']+)["']"#)
                .expect("Invalid element ID regex"),

            // Match class="..." attribute
            class_attr_re: Regex::new(r#"class\s*=\s*["']([^"']+)["']"#)
                .expect("Invalid class attribute regex"),

            // Match template engine markers (EJS, Handlebars, etc.)
            template_markers_re: Regex::new(r"<%|%>|\{\{|\}\}|\{%|%\}")
                .expect("Invalid template markers regex"),
        }
    }

    /// Detect if this is a template file (EJS, Handlebars, etc.)
    fn is_template_file(&self, file_path: &str, text: &str) -> bool {
        // Check filename patterns
        if file_path.contains(".ejs")
            || file_path.contains(".hbs")
            || file_path.contains(".handlebars")
            || file_path.contains(".njk")
            || file_path.contains(".twig")
        {
            return true;
        }

        // Check for template markers in content
        self.template_markers_re.is_match(text)
    }

    /// Detect if this is a partial/layout file.
    fn is_partial_or_layout(&self, file_path: &str) -> bool {
        let lower = file_path.to_lowercase();
        lower.contains("partial")
            || lower.contains("layout")
            || lower.contains("template")
            || lower.contains("_")  // Common partial naming convention
            || lower.contains("component")
    }

    /// Extract section symbols (sections, articles, etc. with IDs).
    fn extract_sections(
        &self,
        file_path: &str,
        text: &str,
        framework: FrameworkHint,
    ) -> Vec<KgSymbolCandidate> {
        let mut symbols = Vec::new();
        let mut seen = std::collections::HashSet::new();

        for cap in self.section_id_re.captures_iter(text) {
            if let (Some(tag), Some(id)) = (cap.get(1), cap.get(2)) {
                let id_str = id.as_str();
                if seen.contains(id_str) {
                    continue;
                }
                seen.insert(id_str.to_string());

                let sym =
                    KgSymbolCandidate::new("htmlSection", id_str, LanguageKind::Html, file_path)
                        .with_framework(framework)
                        .with_prop("tagName".to_string(), tag.as_str().to_string())
                        .with_prop("elementId".to_string(), id_str.to_string());
                symbols.push(sym);
            }
        }

        symbols
    }

    /// Extract anchor/ID symbols (any element with an ID).
    fn extract_anchors(
        &self,
        file_path: &str,
        text: &str,
        framework: FrameworkHint,
    ) -> Vec<KgSymbolCandidate> {
        let mut symbols = Vec::new();
        let mut seen = std::collections::HashSet::new();

        for cap in self.element_id_re.captures_iter(text) {
            if let Some(id) = cap.get(1) {
                let id_str = id.as_str();
                if seen.contains(id_str) {
                    continue;
                }
                seen.insert(id_str.to_string());

                let sym =
                    KgSymbolCandidate::new("htmlAnchor", id_str, LanguageKind::Html, file_path)
                        .with_framework(framework)
                        .with_prop("elementId".to_string(), id_str.to_string());
                symbols.push(sym);
            }
        }

        symbols
    }

    /// Extract template root symbol.
    fn extract_template_symbol(
        &self,
        file_path: &str,
        text: &str,
        framework: FrameworkHint,
    ) -> Option<KgSymbolCandidate> {
        // Check if it's a full HTML document
        let has_html_tag = text.contains("<html") || text.contains("<!DOCTYPE");
        let has_body = text.contains("<body");

        let kind = if self.is_partial_or_layout(file_path) {
            "htmlPartial"
        } else if has_html_tag || has_body {
            "htmlTemplate"
        } else {
            // Small fragments without html/body are likely partials
            "htmlPartial"
        };

        // Extract a name from the file path
        let name = std::path::Path::new(file_path)
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("index");

        let sym = KgSymbolCandidate::new(kind, name, LanguageKind::Html, file_path)
            .with_framework(framework)
            .with_prop("isDocument".to_string(), has_html_tag.to_string())
            .with_prop("hasBody".to_string(), has_body.to_string());

        Some(sym)
    }

    /// Extract all class names used in the HTML.
    fn extract_class_usages(&self, text: &str) -> Vec<String> {
        let mut classes = Vec::new();
        let mut seen = std::collections::HashSet::new();

        for cap in self.class_attr_re.captures_iter(text) {
            if let Some(class_list) = cap.get(1) {
                for class_name in class_list.as_str().split_whitespace() {
                    let trimmed = class_name.trim();
                    if !trimmed.is_empty() && !seen.contains(trimmed) {
                        seen.insert(trimmed.to_string());
                        classes.push(trimmed.to_string());
                    }
                }
            }
        }

        classes
    }
}

impl Default for HtmlExtractor {
    fn default() -> Self {
        Self::new()
    }
}

impl LanguageExtractor for HtmlExtractor {
    fn language(&self) -> LanguageKind {
        LanguageKind::Html
    }

    fn detect_framework(&self, file_path: &str, text: &str) -> FrameworkHint {
        // Check for Angular template markers
        if text.contains("*ngIf") || text.contains("*ngFor") || text.contains("[(ngModel)]") {
            return FrameworkHint::Angular;
        }

        // Check for template engine usage
        if self.is_template_file(file_path, text) {
            return FrameworkHint::Generic;
        }

        FrameworkHint::None
    }

    fn extract_symbols(&self, file_path: &str, text: &str) -> Vec<KgSymbolCandidate> {
        let mut symbols = Vec::new();
        let framework = self.detect_framework(file_path, text);

        // Extract template/partial root symbol
        if let Some(template_sym) = self.extract_template_symbol(file_path, text, framework) {
            symbols.push(template_sym);
        }

        // Extract semantic sections with IDs
        symbols.extend(self.extract_sections(file_path, text, framework));

        // Extract anchor IDs (deduplicated from sections)
        let section_ids: std::collections::HashSet<_> = symbols
            .iter()
            .filter(|s| s.kind == "htmlSection")
            .map(|s| s.name.clone())
            .collect();

        for anchor in self.extract_anchors(file_path, text, framework) {
            // Don't duplicate IDs already captured as sections
            if !section_ids.contains(&anchor.name) {
                symbols.push(anchor);
            }
        }

        symbols
    }

    fn extract_relations(&self, file_path: &str, text: &str) -> Vec<KgRelationCandidate> {
        let mut relations = Vec::new();

        // Create usesClass relations for each class found in the HTML
        let file_node_id = format!("file:{}", file_path);

        for class_name in self.extract_class_usages(text) {
            // Reference to a potentially virtual CSS symbol
            // Using a normalized path "*" to indicate it could come from any CSS file
            let style_symbol_id = format!("sym:css:*:styleClass:{}", class_name);

            let rel = KgRelationCandidate::new(&file_node_id, &style_symbol_id, "usesClass")
                .with_props(serde_json::json!({
                    "className": class_name,
                    "unresolved": true  // Mark as potentially unresolved
                }));
            relations.push(rel);
        }

        relations
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_html_sections() {
        let html = r#"
<!DOCTYPE html>
<html>
<body>
    <header id="main-header">
        <nav>Navigation</nav>
    </header>
    <section id="hero">
        <h1>Welcome</h1>
    </section>
    <main id="content">
        <article id="post-1">
            <p>Content here</p>
        </article>
    </main>
    <footer id="main-footer">
        <p>Footer</p>
    </footer>
</body>
</html>
"#;

        let extractor = HtmlExtractor::new();
        let symbols = extractor.extract_symbols("index.html", html);

        let sections: Vec<&str> = symbols
            .iter()
            .filter(|s| s.kind == "htmlSection")
            .map(|s| s.name.as_str())
            .collect();

        assert!(sections.contains(&"main-header"));
        assert!(sections.contains(&"hero"));
        assert!(sections.contains(&"content"));
        assert!(sections.contains(&"main-footer"));
    }

    #[test]
    fn test_extract_html_template() {
        let html = r#"
<!DOCTYPE html>
<html lang="en">
<head><title>Test</title></head>
<body>
    <div id="app"></div>
</body>
</html>
"#;

        let extractor = HtmlExtractor::new();
        let symbols = extractor.extract_symbols("templates/index.html", html);

        let templates: Vec<&str> = symbols
            .iter()
            .filter(|s| s.kind == "htmlTemplate" || s.kind == "htmlPartial")
            .map(|s| s.name.as_str())
            .collect();

        assert!(!templates.is_empty());
        assert!(templates.contains(&"index"));
    }

    #[test]
    fn test_extract_class_usages() {
        let html = r#"
<div class="container mx-auto px-4">
    <button class="btn btn-primary hover:bg-blue-600">
        Click me
    </button>
    <span class="text-sm text-gray-500">Helper text</span>
</div>
"#;

        let extractor = HtmlExtractor::new();
        let classes = extractor.extract_class_usages(html);

        assert!(classes.contains(&"container".to_string()));
        assert!(classes.contains(&"mx-auto".to_string()));
        assert!(classes.contains(&"btn".to_string()));
        assert!(classes.contains(&"btn-primary".to_string()));
        assert!(classes.contains(&"hover:bg-blue-600".to_string()));
        assert!(classes.contains(&"text-sm".to_string()));
    }

    #[test]
    fn test_extract_uses_class_relations() {
        let html = r#"
<div class="hero bg-blue-500">
    <h1 class="title">Hello</h1>
</div>
"#;

        let extractor = HtmlExtractor::new();
        let relations = extractor.extract_relations("page.html", html);

        assert!(!relations.is_empty());

        let class_names: Vec<String> = relations
            .iter()
            .filter(|r| r.kind == "usesClass")
            .map(|r| {
                r.props
                    .get("className")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string()
            })
            .collect();

        assert!(class_names.contains(&"hero".to_string()));
        assert!(class_names.contains(&"bg-blue-500".to_string()));
        assert!(class_names.contains(&"title".to_string()));
    }

    #[test]
    fn test_detect_angular_template() {
        let angular_html = r#"
<div *ngIf="showContent">
    <ul>
        <li *ngFor="let item of items">{{ item.name }}</li>
    </ul>
    <input [(ngModel)]="searchTerm">
</div>
"#;

        let plain_html = r#"
<div>
    <p>Regular HTML</p>
</div>
"#;

        let extractor = HtmlExtractor::new();
        assert_eq!(
            extractor.detect_framework("app.component.html", angular_html),
            FrameworkHint::Angular
        );
        assert_eq!(
            extractor.detect_framework("page.html", plain_html),
            FrameworkHint::None
        );
    }

    #[test]
    fn test_partial_detection() {
        let extractor = HtmlExtractor::new();

        assert!(extractor.is_partial_or_layout("partials/header.html"));
        assert!(extractor.is_partial_or_layout("layouts/main.html"));
        assert!(extractor.is_partial_or_layout("templates/base.html"));
        assert!(extractor.is_partial_or_layout("_sidebar.html"));
        assert!(!extractor.is_partial_or_layout("index.html"));
    }

    #[test]
    fn test_extract_anchors() {
        let html = r#"
<div id="app">
    <span id="user-name">John</span>
    <input id="search-input" type="text">
</div>
"#;

        let extractor = HtmlExtractor::new();
        let symbols = extractor.extract_symbols("page.html", html);

        let anchors: Vec<&str> = symbols
            .iter()
            .filter(|s| s.kind == "htmlAnchor")
            .map(|s| s.name.as_str())
            .collect();

        assert!(anchors.contains(&"app"));
        assert!(anchors.contains(&"user-name"));
        assert!(anchors.contains(&"search-input"));
    }

    #[test]
    fn test_template_engine_detection() {
        let ejs = r#"
<html>
<body>
    <h1><%= title %></h1>
    <% if (showContent) { %>
        <p>Content</p>
    <% } %>
</body>
</html>
"#;

        let handlebars = r#"
<html>
<body>
    <h1>{{title}}</h1>
    {{#if showContent}}
        <p>Content</p>
    {{/if}}
</body>
</html>
"#;

        let extractor = HtmlExtractor::new();
        assert!(extractor.is_template_file("view.ejs", ejs));
        assert!(extractor.is_template_file("view.html", ejs)); // Contains EJS markers
        assert!(extractor.is_template_file("view.html", handlebars)); // Contains Handlebars markers
    }
}
