//! Ruby language extractor.
//!
//! Extracts symbols from Ruby files:
//! - Classes
//! - Modules
//! - Methods (def)
//! - Constants

use regex::Regex;

use super::{
    FrameworkHint, KgRelationCandidate, KgSymbolCandidate, LanguageExtractor, LanguageKind,
};

/// Ruby extractor.
#[derive(Debug, Clone, Default)]
pub struct RubyExtractor;

impl RubyExtractor {
    /// Create a new Ruby extractor.
    pub fn new() -> Self {
        Self
    }
}

impl LanguageExtractor for RubyExtractor {
    fn language(&self) -> LanguageKind {
        LanguageKind::Ruby
    }

    fn detect_framework(&self, file_path: &str, text: &str) -> FrameworkHint {
        // Rails detection
        if file_path.contains("/app/controllers/")
            || file_path.contains("/app/models/")
            || file_path.contains("/app/views/")
        {
            return FrameworkHint::Rails;
        }
        if text.contains("< ApplicationController")
            || text.contains("< ActiveRecord::Base")
            || text.contains("Rails.application")
        {
            return FrameworkHint::Rails;
        }

        FrameworkHint::None
    }

    fn extract_symbols(&self, file_path: &str, text: &str) -> Vec<KgSymbolCandidate> {
        let mut symbols = Vec::new();
        let framework = self.detect_framework(file_path, text);

        symbols.extend(extract_classes(file_path, text, framework));
        symbols.extend(extract_modules(file_path, text, framework));
        symbols.extend(extract_methods(file_path, text, framework));
        symbols.extend(extract_constants(file_path, text, framework));

        symbols
    }

    fn extract_relations(&self, file_path: &str, _text: &str) -> Vec<KgRelationCandidate> {
        let _ = file_path;
        Vec::new()
    }
}

/// Extract class definitions.
fn extract_classes(
    file_path: &str,
    text: &str,
    framework: FrameworkHint,
) -> Vec<KgSymbolCandidate> {
    let mut symbols = Vec::new();

    let class_re = Regex::new(r"class\s+([A-Z][a-zA-Z0-9_]*)").expect("Invalid regex");

    for cap in class_re.captures_iter(text) {
        if let Some(name) = cap.get(1) {
            let sym = KgSymbolCandidate::new("class", name.as_str(), LanguageKind::Ruby, file_path)
                .with_framework(framework);
            symbols.push(sym);
        }
    }

    symbols
}

/// Extract module definitions.
fn extract_modules(
    file_path: &str,
    text: &str,
    framework: FrameworkHint,
) -> Vec<KgSymbolCandidate> {
    let mut symbols = Vec::new();

    let module_re = Regex::new(r"module\s+([A-Z][a-zA-Z0-9_]*)").expect("Invalid regex");

    for cap in module_re.captures_iter(text) {
        if let Some(name) = cap.get(1) {
            let sym =
                KgSymbolCandidate::new("module", name.as_str(), LanguageKind::Ruby, file_path)
                    .with_framework(framework);
            symbols.push(sym);
        }
    }

    symbols
}

/// Extract method definitions.
fn extract_methods(
    file_path: &str,
    text: &str,
    framework: FrameworkHint,
) -> Vec<KgSymbolCandidate> {
    let mut symbols = Vec::new();

    // Pattern: def method_name or def self.method_name
    let method_re =
        Regex::new(r"def\s+(?:self\.)?([a-zA-Z_][a-zA-Z0-9_]*[!?]?)").expect("Invalid regex");

    for cap in method_re.captures_iter(text) {
        if let Some(name) = cap.get(1) {
            let sym =
                KgSymbolCandidate::new("method", name.as_str(), LanguageKind::Ruby, file_path)
                    .with_framework(framework);
            symbols.push(sym);
        }
    }

    symbols
}

/// Extract constants.
fn extract_constants(
    file_path: &str,
    text: &str,
    framework: FrameworkHint,
) -> Vec<KgSymbolCandidate> {
    let mut symbols = Vec::new();

    // Pattern: CONSTANT_NAME = ...
    let const_re = Regex::new(r"([A-Z][A-Z0-9_]*)\s*=").expect("Invalid regex");

    for cap in const_re.captures_iter(text) {
        if let Some(name) = cap.get(1) {
            let sym =
                KgSymbolCandidate::new("constant", name.as_str(), LanguageKind::Ruby, file_path)
                    .with_framework(framework);
            symbols.push(sym);
        }
    }

    symbols
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_classes() {
        let code = r#"
class User
  attr_accessor :name
end

class AdminUser < User
end
"#;
        let symbols = extract_classes("app/models/user.rb", code, FrameworkHint::None);
        let names: Vec<&str> = symbols.iter().map(|s| s.name.as_str()).collect();

        assert!(names.contains(&"User"));
        assert!(names.contains(&"AdminUser"));
    }

    #[test]
    fn test_extract_methods() {
        let code = r#"
def process_data
  # ...
end

def self.class_method
  # ...
end

def valid?
  true
end

def save!
  # ...
end
"#;
        let symbols = extract_methods("lib/utils.rb", code, FrameworkHint::None);
        let names: Vec<&str> = symbols.iter().map(|s| s.name.as_str()).collect();

        assert!(names.contains(&"process_data"));
        assert!(names.contains(&"class_method"));
        assert!(names.contains(&"valid?"));
        assert!(names.contains(&"save!"));
    }

    #[test]
    fn test_detect_rails() {
        let extractor = RubyExtractor::new();

        // Note: detect_framework primarily looks at text content
        assert_eq!(
            extractor.detect_framework(
                "lib/utils.rb",
                "class UsersController < ApplicationController"
            ),
            FrameworkHint::Rails
        );
        assert_eq!(
            extractor.detect_framework("lib/model.rb", "class User < ActiveRecord::Base"),
            FrameworkHint::Rails
        );
    }
}
