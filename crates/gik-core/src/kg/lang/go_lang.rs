//! Go language extractor.
//!
//! Extracts symbols from Go files:
//! - Functions
//! - Structs
//! - Interfaces
//! - Types
//! - Constants
//! - Variables

use regex::Regex;

use super::{
    FrameworkHint, KgRelationCandidate, KgSymbolCandidate, LanguageExtractor, LanguageKind,
};

/// Go extractor.
#[derive(Debug, Clone, Default)]
pub struct GoExtractor;

impl GoExtractor {
    /// Create a new Go extractor.
    pub fn new() -> Self {
        Self
    }
}

impl LanguageExtractor for GoExtractor {
    fn language(&self) -> LanguageKind {
        LanguageKind::Go
    }

    fn detect_framework(&self, _file_path: &str, text: &str) -> FrameworkHint {
        // Gin detection
        if text.contains("github.com/gin-gonic/gin") {
            return FrameworkHint::Gin;
        }

        // Fiber detection
        if text.contains("github.com/gofiber/fiber") {
            return FrameworkHint::Fiber;
        }

        FrameworkHint::None
    }

    fn extract_symbols(&self, file_path: &str, text: &str) -> Vec<KgSymbolCandidate> {
        let mut symbols = Vec::new();
        let framework = self.detect_framework(file_path, text);

        symbols.extend(extract_functions(file_path, text, framework));
        symbols.extend(extract_structs(file_path, text, framework));
        symbols.extend(extract_interfaces(file_path, text, framework));
        symbols.extend(extract_types(file_path, text, framework));
        symbols.extend(extract_constants(file_path, text, framework));

        symbols
    }

    fn extract_relations(&self, file_path: &str, _text: &str) -> Vec<KgRelationCandidate> {
        let _ = file_path;
        Vec::new()
    }
}

/// Extract function definitions.
fn extract_functions(
    file_path: &str,
    text: &str,
    framework: FrameworkHint,
) -> Vec<KgSymbolCandidate> {
    let mut symbols = Vec::new();

    // Pattern: func name(...) or func (receiver) name(...)
    let fn_re = Regex::new(r"func\s+(?:\([^)]+\)\s+)?([a-zA-Z_][a-zA-Z0-9_]*)\s*\(")
        .expect("Invalid regex");

    for cap in fn_re.captures_iter(text) {
        if let Some(name) = cap.get(1) {
            let sym =
                KgSymbolCandidate::new("function", name.as_str(), LanguageKind::Go, file_path)
                    .with_framework(framework);
            symbols.push(sym);
        }
    }

    symbols
}

/// Extract struct definitions.
fn extract_structs(
    file_path: &str,
    text: &str,
    framework: FrameworkHint,
) -> Vec<KgSymbolCandidate> {
    let mut symbols = Vec::new();

    let struct_re = Regex::new(r"type\s+([A-Z][a-zA-Z0-9_]*)\s+struct\b").expect("Invalid regex");

    for cap in struct_re.captures_iter(text) {
        if let Some(name) = cap.get(1) {
            let sym = KgSymbolCandidate::new("struct", name.as_str(), LanguageKind::Go, file_path)
                .with_framework(framework);
            symbols.push(sym);
        }
    }

    symbols
}

/// Extract interface definitions.
fn extract_interfaces(
    file_path: &str,
    text: &str,
    framework: FrameworkHint,
) -> Vec<KgSymbolCandidate> {
    let mut symbols = Vec::new();

    let iface_re = Regex::new(r"type\s+([A-Z][a-zA-Z0-9_]*)\s+interface\b").expect("Invalid regex");

    for cap in iface_re.captures_iter(text) {
        if let Some(name) = cap.get(1) {
            let sym =
                KgSymbolCandidate::new("interface", name.as_str(), LanguageKind::Go, file_path)
                    .with_framework(framework);
            symbols.push(sym);
        }
    }

    symbols
}

/// Extract type aliases.
fn extract_types(file_path: &str, text: &str, framework: FrameworkHint) -> Vec<KgSymbolCandidate> {
    let mut symbols = Vec::new();

    // Type aliases: type Name OtherType (single identifier, not struct/interface)
    // We'll match all type definitions, then filter out struct/interface
    let type_re = Regex::new(r"type\s+([A-Z][a-zA-Z0-9_]*)\s+([a-zA-Z][a-zA-Z0-9_\[\]\.]*)")
        .expect("Invalid regex");

    for cap in type_re.captures_iter(text) {
        if let (Some(name), Some(target)) = (cap.get(1), cap.get(2)) {
            // Skip struct and interface definitions (they're extracted separately)
            if target.as_str() == "struct" || target.as_str() == "interface" {
                continue;
            }
            let sym = KgSymbolCandidate::new("type", name.as_str(), LanguageKind::Go, file_path)
                .with_framework(framework);
            symbols.push(sym);
        }
    }

    symbols
}

/// Extract constant definitions.
fn extract_constants(
    file_path: &str,
    text: &str,
    framework: FrameworkHint,
) -> Vec<KgSymbolCandidate> {
    let mut symbols = Vec::new();

    // Single const: const Name = ...
    let const_re = Regex::new(r"const\s+([A-Z][a-zA-Z0-9_]*)\s*=").expect("Invalid regex");

    for cap in const_re.captures_iter(text) {
        if let Some(name) = cap.get(1) {
            let sym =
                KgSymbolCandidate::new("constant", name.as_str(), LanguageKind::Go, file_path)
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
    fn test_extract_functions() {
        let code = r#"
package main

func main() {
    // ...
}

func processData(data []byte) error {
    return nil
}

func (s *Service) HandleRequest(r *Request) Response {
    return Response{}
}
"#;
        let symbols = extract_functions("main.go", code, FrameworkHint::None);
        let names: Vec<&str> = symbols.iter().map(|s| s.name.as_str()).collect();

        assert!(names.contains(&"main"));
        assert!(names.contains(&"processData"));
        assert!(names.contains(&"HandleRequest"));
    }

    #[test]
    fn test_extract_structs() {
        let code = r#"
package models

type User struct {
    ID   int
    Name string
}

type Config struct {
    Debug bool
}
"#;
        let symbols = extract_structs("models/user.go", code, FrameworkHint::None);
        let names: Vec<&str> = symbols.iter().map(|s| s.name.as_str()).collect();

        assert!(names.contains(&"User"));
        assert!(names.contains(&"Config"));
    }

    #[test]
    fn test_extract_interfaces() {
        let code = r#"
package services

type UserRepository interface {
    Find(id int) (*User, error)
    Save(user *User) error
}

type Logger interface {
    Log(msg string)
}
"#;
        let symbols = extract_interfaces("services/repo.go", code, FrameworkHint::None);
        let names: Vec<&str> = symbols.iter().map(|s| s.name.as_str()).collect();

        assert!(names.contains(&"UserRepository"));
        assert!(names.contains(&"Logger"));
    }

    #[test]
    fn test_detect_gin() {
        let extractor = GoExtractor::new();

        assert_eq!(
            extractor.detect_framework("main.go", r#"import "github.com/gin-gonic/gin""#),
            FrameworkHint::Gin
        );
    }

    #[test]
    fn test_detect_fiber() {
        let extractor = GoExtractor::new();

        assert_eq!(
            extractor.detect_framework("main.go", r#"import "github.com/gofiber/fiber/v2""#),
            FrameworkHint::Fiber
        );
    }
}
