//! PHP language extractor.
//!
//! Extracts symbols from PHP files:
//! - Classes
//! - Interfaces
//! - Traits
//! - Functions
//! - Constants

use regex::Regex;

use super::{
    FrameworkHint, KgRelationCandidate, KgSymbolCandidate, LanguageExtractor, LanguageKind,
};

/// PHP extractor.
#[derive(Debug, Clone, Default)]
pub struct PhpExtractor;

impl PhpExtractor {
    /// Create a new PHP extractor.
    pub fn new() -> Self {
        Self
    }
}

impl LanguageExtractor for PhpExtractor {
    fn language(&self) -> LanguageKind {
        LanguageKind::Php
    }

    fn detect_framework(&self, _file_path: &str, text: &str) -> FrameworkHint {
        // Laravel detection
        if text.contains("Illuminate\\") || text.contains("extends Controller") {
            return FrameworkHint::Laravel;
        }

        FrameworkHint::None
    }

    fn extract_symbols(&self, file_path: &str, text: &str) -> Vec<KgSymbolCandidate> {
        let mut symbols = Vec::new();
        let framework = self.detect_framework(file_path, text);

        symbols.extend(extract_classes(file_path, text, framework));
        symbols.extend(extract_interfaces(file_path, text, framework));
        symbols.extend(extract_traits(file_path, text, framework));
        symbols.extend(extract_functions(file_path, text, framework));

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

    let class_re = Regex::new(r"(?:abstract\s+)?(?:final\s+)?class\s+([A-Z][a-zA-Z0-9_]*)")
        .expect("Invalid regex");

    for cap in class_re.captures_iter(text) {
        if let Some(name) = cap.get(1) {
            let sym = KgSymbolCandidate::new("class", name.as_str(), LanguageKind::Php, file_path)
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

    let iface_re = Regex::new(r"interface\s+([A-Z][a-zA-Z0-9_]*)").expect("Invalid regex");

    for cap in iface_re.captures_iter(text) {
        if let Some(name) = cap.get(1) {
            let sym =
                KgSymbolCandidate::new("interface", name.as_str(), LanguageKind::Php, file_path)
                    .with_framework(framework);
            symbols.push(sym);
        }
    }

    symbols
}

/// Extract trait definitions.
fn extract_traits(file_path: &str, text: &str, framework: FrameworkHint) -> Vec<KgSymbolCandidate> {
    let mut symbols = Vec::new();

    let trait_re = Regex::new(r"trait\s+([A-Z][a-zA-Z0-9_]*)").expect("Invalid regex");

    for cap in trait_re.captures_iter(text) {
        if let Some(name) = cap.get(1) {
            let sym = KgSymbolCandidate::new("trait", name.as_str(), LanguageKind::Php, file_path)
                .with_framework(framework);
            symbols.push(sym);
        }
    }

    symbols
}

/// Extract function definitions.
fn extract_functions(
    file_path: &str,
    text: &str,
    framework: FrameworkHint,
) -> Vec<KgSymbolCandidate> {
    let mut symbols = Vec::new();

    let fn_re = Regex::new(
        r"(?:public\s+|private\s+|protected\s+)?(?:static\s+)?function\s+([a-zA-Z_][a-zA-Z0-9_]*)\s*\(",
    )
    .expect("Invalid regex");

    for cap in fn_re.captures_iter(text) {
        if let Some(name) = cap.get(1) {
            let name_str = name.as_str();
            // Skip magic methods except __construct
            if name_str.starts_with("__") && name_str != "__construct" {
                continue;
            }

            let sym = KgSymbolCandidate::new("function", name_str, LanguageKind::Php, file_path)
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
<?php

class UserService
{
    // ...
}

abstract class BaseController
{
    // ...
}

final class Constants
{
    // ...
}
"#;
        let symbols = extract_classes("app/Services/UserService.php", code, FrameworkHint::None);
        let names: Vec<&str> = symbols.iter().map(|s| s.name.as_str()).collect();

        assert!(names.contains(&"UserService"));
        assert!(names.contains(&"BaseController"));
        assert!(names.contains(&"Constants"));
    }

    #[test]
    fn test_extract_interfaces() {
        let code = r#"
<?php

interface UserRepositoryInterface
{
    public function find(int $id): ?User;
}

interface Loggable
{
    public function log(string $message): void;
}
"#;
        let symbols = extract_interfaces(
            "app/Contracts/UserRepository.php",
            code,
            FrameworkHint::None,
        );
        let names: Vec<&str> = symbols.iter().map(|s| s.name.as_str()).collect();

        assert!(names.contains(&"UserRepositoryInterface"));
        assert!(names.contains(&"Loggable"));
    }

    #[test]
    fn test_extract_functions() {
        let code = r#"
<?php

function helper_function($data)
{
    return $data;
}

class UserService
{
    public function getUser(int $id)
    {
        // ...
    }

    private static function validate($data)
    {
        // ...
    }

    public function __construct()
    {
        // ...
    }

    public function __toString()
    {
        return "";
    }
}
"#;
        let symbols = extract_functions("app/helpers.php", code, FrameworkHint::None);
        let names: Vec<&str> = symbols.iter().map(|s| s.name.as_str()).collect();

        assert!(names.contains(&"helper_function"));
        assert!(names.contains(&"getUser"));
        assert!(names.contains(&"validate"));
        assert!(names.contains(&"__construct"));
        assert!(!names.contains(&"__toString")); // magic method skipped
    }

    #[test]
    fn test_detect_laravel() {
        let extractor = PhpExtractor::new();

        assert_eq!(
            extractor.detect_framework(
                "app/Http/Controllers/UserController.php",
                "use Illuminate\\Http\\Request;"
            ),
            FrameworkHint::Laravel
        );
    }
}
