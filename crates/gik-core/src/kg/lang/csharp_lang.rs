//! C# language extractor.
//!
//! Extracts symbols from C# files:
//! - Classes
//! - Interfaces
//! - Structs
//! - Enums
//! - Methods
//! - Properties

use regex::Regex;

use super::{
    FrameworkHint, KgRelationCandidate, KgSymbolCandidate, LanguageExtractor, LanguageKind,
};

/// C# extractor.
#[derive(Debug, Clone, Default)]
pub struct CSharpExtractor;

impl CSharpExtractor {
    /// Create a new C# extractor.
    pub fn new() -> Self {
        Self
    }
}

impl LanguageExtractor for CSharpExtractor {
    fn language(&self) -> LanguageKind {
        LanguageKind::CSharp
    }

    fn detect_framework(&self, _file_path: &str, text: &str) -> FrameworkHint {
        // ASP.NET detection
        if text.contains("[ApiController]")
            || text.contains("[HttpGet]")
            || text.contains("[HttpPost]")
            || text.contains("Microsoft.AspNetCore")
        {
            return FrameworkHint::AspNet;
        }

        FrameworkHint::None
    }

    fn extract_symbols(&self, file_path: &str, text: &str) -> Vec<KgSymbolCandidate> {
        let mut symbols = Vec::new();
        let framework = self.detect_framework(file_path, text);

        symbols.extend(extract_classes(file_path, text, framework));
        symbols.extend(extract_interfaces(file_path, text, framework));
        symbols.extend(extract_structs(file_path, text, framework));
        symbols.extend(extract_enums(file_path, text, framework));
        symbols.extend(extract_methods(file_path, text, framework));

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

    let class_re = Regex::new(
        r"(?:public|private|protected|internal)?\s*(?:abstract|sealed|static|partial)?\s*class\s+([A-Z][a-zA-Z0-9_]*)",
    )
    .expect("Invalid regex");

    for cap in class_re.captures_iter(text) {
        if let Some(name) = cap.get(1) {
            let sym =
                KgSymbolCandidate::new("class", name.as_str(), LanguageKind::CSharp, file_path)
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

    let iface_re =
        Regex::new(r"(?:public|private|protected|internal)?\s*interface\s+(I[A-Z][a-zA-Z0-9_]*)")
            .expect("Invalid regex");

    for cap in iface_re.captures_iter(text) {
        if let Some(name) = cap.get(1) {
            let sym =
                KgSymbolCandidate::new("interface", name.as_str(), LanguageKind::CSharp, file_path)
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

    let struct_re = Regex::new(
        r"(?:public|private|protected|internal)?\s*(?:readonly)?\s*struct\s+([A-Z][a-zA-Z0-9_]*)",
    )
    .expect("Invalid regex");

    for cap in struct_re.captures_iter(text) {
        if let Some(name) = cap.get(1) {
            let sym =
                KgSymbolCandidate::new("struct", name.as_str(), LanguageKind::CSharp, file_path)
                    .with_framework(framework);
            symbols.push(sym);
        }
    }

    symbols
}

/// Extract enum definitions.
fn extract_enums(file_path: &str, text: &str, framework: FrameworkHint) -> Vec<KgSymbolCandidate> {
    let mut symbols = Vec::new();

    let enum_re =
        Regex::new(r"(?:public|private|protected|internal)?\s*enum\s+([A-Z][a-zA-Z0-9_]*)")
            .expect("Invalid regex");

    for cap in enum_re.captures_iter(text) {
        if let Some(name) = cap.get(1) {
            let sym =
                KgSymbolCandidate::new("enum", name.as_str(), LanguageKind::CSharp, file_path)
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

    // Pattern for methods (simplified)
    let method_re = Regex::new(
        r"(?:public|private|protected|internal)\s+(?:static\s+)?(?:async\s+)?(?:virtual\s+)?(?:override\s+)?(?:\w+(?:<[^>]+>)?)\s+([A-Z][a-zA-Z0-9_]*)\s*\(",
    )
    .expect("Invalid regex");

    for cap in method_re.captures_iter(text) {
        if let Some(name) = cap.get(1) {
            let name_str = name.as_str();
            // Skip properties and common patterns
            if name_str == "Main" || name_str.starts_with("Get") && !name_str.contains("_") {
                let sym =
                    KgSymbolCandidate::new("method", name_str, LanguageKind::CSharp, file_path)
                        .with_framework(framework);
                symbols.push(sym);
            }
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
public class UserService
{
    // ...
}

internal sealed class CacheManager
{
    // ...
}

public abstract class BaseController
{
    // ...
}
"#;
        let symbols = extract_classes("Services/UserService.cs", code, FrameworkHint::None);
        let names: Vec<&str> = symbols.iter().map(|s| s.name.as_str()).collect();

        assert!(names.contains(&"UserService"));
        assert!(names.contains(&"CacheManager"));
        assert!(names.contains(&"BaseController"));
    }

    #[test]
    fn test_extract_interfaces() {
        let code = r#"
public interface IUserRepository
{
    User FindById(int id);
}

internal interface ILogger
{
    void Log(string message);
}
"#;
        let symbols =
            extract_interfaces("Interfaces/IUserRepository.cs", code, FrameworkHint::None);
        let names: Vec<&str> = symbols.iter().map(|s| s.name.as_str()).collect();

        assert!(names.contains(&"IUserRepository"));
        assert!(names.contains(&"ILogger"));
    }

    #[test]
    fn test_detect_aspnet() {
        let extractor = CSharpExtractor::new();

        assert_eq!(
            extractor.detect_framework("Controllers/UsersController.cs", "[ApiController]"),
            FrameworkHint::AspNet
        );
        assert_eq!(
            extractor.detect_framework("Controllers/UsersController.cs", "[HttpGet]"),
            FrameworkHint::AspNet
        );
    }
}
