//! Markdown extractor.
//!
//! Extracts symbols from Markdown files:
//! - Headings (h1-h6)
//! - Code block languages
//! - Links (as potential references)

use regex::Regex;

use super::{
    FrameworkHint, KgRelationCandidate, KgSymbolCandidate, LanguageExtractor, LanguageKind,
};

/// Markdown extractor.
#[derive(Debug, Clone, Default)]
pub struct MarkdownExtractor;

impl MarkdownExtractor {
    /// Create a new Markdown extractor.
    pub fn new() -> Self {
        Self
    }
}

impl LanguageExtractor for MarkdownExtractor {
    fn language(&self) -> LanguageKind {
        LanguageKind::Markdown
    }

    fn detect_framework(&self, _file_path: &str, _text: &str) -> FrameworkHint {
        FrameworkHint::None
    }

    fn extract_symbols(&self, file_path: &str, text: &str) -> Vec<KgSymbolCandidate> {
        let mut symbols = Vec::new();

        symbols.extend(extract_headings(file_path, text));

        symbols
    }

    fn extract_relations(&self, file_path: &str, _text: &str) -> Vec<KgRelationCandidate> {
        let _ = file_path;
        Vec::new()
    }
}

/// Extract headings from markdown.
fn extract_headings(file_path: &str, text: &str) -> Vec<KgSymbolCandidate> {
    let mut symbols = Vec::new();

    // Pattern: # Heading or ## Heading etc.
    let heading_re = Regex::new(r"(?m)^(#{1,6})\s+(.+)$").expect("Invalid regex");

    for cap in heading_re.captures_iter(text) {
        let level = cap.get(1).map(|m| m.as_str().len()).unwrap_or(1);
        if let Some(title) = cap.get(2) {
            let title_str = title.as_str().trim();
            // Create a slug-like name from the heading
            let slug = slugify(title_str);

            let kind = format!("h{}", level);
            let sym = KgSymbolCandidate::new(&kind, &slug, LanguageKind::Markdown, file_path)
                .with_prop("title".to_string(), title_str.to_string());
            symbols.push(sym);
        }
    }

    symbols
}

/// Convert a heading to a URL-friendly slug.
fn slugify(text: &str) -> String {
    text.to_lowercase()
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { '-' })
        .collect::<String>()
        .split('-')
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("-")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_headings() {
        let code = r#"
# Main Title

Some intro text.

## Getting Started

Instructions here.

### Installation

```bash
npm install
```

## API Reference

### Functions

#### `processData`

Processes data.
"#;
        let symbols = extract_headings("README.md", code);

        assert_eq!(symbols.len(), 6);
        assert_eq!(symbols[0].kind, "h1");
        assert_eq!(symbols[0].name, "main-title");
        assert_eq!(symbols[1].kind, "h2");
        assert_eq!(symbols[1].name, "getting-started");
    }

    #[test]
    fn test_slugify() {
        assert_eq!(slugify("Getting Started"), "getting-started");
        assert_eq!(slugify("API Reference"), "api-reference");
        assert_eq!(slugify("Hello, World!"), "hello-world");
        assert_eq!(slugify("Version 2.0"), "version-2-0");
    }
}
