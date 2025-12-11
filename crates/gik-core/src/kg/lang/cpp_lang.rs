//! C++ language extractor.
//!
//! Extracts symbols from C++ files:
//! - Functions
//! - Classes
//! - Structs
//! - Enums
//! - Namespaces
//! - Templates

use regex::Regex;

use super::{
    FrameworkHint, KgRelationCandidate, KgSymbolCandidate, LanguageExtractor, LanguageKind,
};

/// C++ extractor.
#[derive(Debug, Clone, Default)]
pub struct CppExtractor;

impl CppExtractor {
    /// Create a new C++ extractor.
    pub fn new() -> Self {
        Self
    }
}

impl LanguageExtractor for CppExtractor {
    fn language(&self) -> LanguageKind {
        LanguageKind::Cpp
    }

    fn detect_framework(&self, _file_path: &str, _text: &str) -> FrameworkHint {
        FrameworkHint::None
    }

    fn extract_symbols(&self, file_path: &str, text: &str) -> Vec<KgSymbolCandidate> {
        let mut symbols = Vec::new();
        let framework = self.detect_framework(file_path, text);

        symbols.extend(extract_classes(file_path, text, framework));
        symbols.extend(extract_structs(file_path, text, framework));
        symbols.extend(extract_namespaces(file_path, text, framework));
        symbols.extend(extract_functions(file_path, text, framework));
        symbols.extend(extract_enums(file_path, text, framework));

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

    // Pattern for functions (simplified)
    let fn_re = Regex::new(
        r"(?:static\s+)?(?:virtual\s+)?(?:inline\s+)?(?:const\s+)?(?:\w+(?:<[^>]+>)?\s*\*?\s*&?\s+)+([a-zA-Z_][a-zA-Z0-9_]*)\s*\([^)]*\)\s*(?:const\s*)?(?:override\s*)?(?:final\s*)?\{",
    )
    .expect("Invalid regex");

    for cap in fn_re.captures_iter(text) {
        if let Some(name) = cap.get(1) {
            let name_str = name.as_str();
            // Skip keywords
            if name_str == "if" || name_str == "while" || name_str == "for" || name_str == "switch"
            {
                continue;
            }

            let sym = KgSymbolCandidate::new("function", name_str, LanguageKind::Cpp, file_path)
                .with_framework(framework);
            symbols.push(sym);
        }
    }

    symbols
}

/// Extract class definitions.
fn extract_classes(
    file_path: &str,
    text: &str,
    framework: FrameworkHint,
) -> Vec<KgSymbolCandidate> {
    let mut symbols = Vec::new();

    let class_re = Regex::new(r"(?:template\s*<[^>]*>\s*)?class\s+([a-zA-Z_][a-zA-Z0-9_]*)")
        .expect("Invalid regex");

    for cap in class_re.captures_iter(text) {
        if let Some(name) = cap.get(1) {
            let sym = KgSymbolCandidate::new("class", name.as_str(), LanguageKind::Cpp, file_path)
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

    let struct_re = Regex::new(r"(?:template\s*<[^>]*>\s*)?struct\s+([a-zA-Z_][a-zA-Z0-9_]*)")
        .expect("Invalid regex");

    for cap in struct_re.captures_iter(text) {
        if let Some(name) = cap.get(1) {
            let sym = KgSymbolCandidate::new("struct", name.as_str(), LanguageKind::Cpp, file_path)
                .with_framework(framework);
            symbols.push(sym);
        }
    }

    symbols
}

/// Extract namespace definitions.
fn extract_namespaces(
    file_path: &str,
    text: &str,
    framework: FrameworkHint,
) -> Vec<KgSymbolCandidate> {
    let mut symbols = Vec::new();

    let ns_re = Regex::new(r"namespace\s+([a-zA-Z_][a-zA-Z0-9_]*)\s*\{").expect("Invalid regex");

    for cap in ns_re.captures_iter(text) {
        if let Some(name) = cap.get(1) {
            let sym =
                KgSymbolCandidate::new("namespace", name.as_str(), LanguageKind::Cpp, file_path)
                    .with_framework(framework);
            symbols.push(sym);
        }
    }

    symbols
}

/// Extract enum definitions.
fn extract_enums(file_path: &str, text: &str, framework: FrameworkHint) -> Vec<KgSymbolCandidate> {
    let mut symbols = Vec::new();

    // Pattern: enum or enum class
    let enum_re = Regex::new(r"enum\s+(?:class\s+)?([a-zA-Z_][a-zA-Z0-9_]*)\s*(?::\s*\w+\s*)?\{")
        .expect("Invalid regex");

    for cap in enum_re.captures_iter(text) {
        if let Some(name) = cap.get(1) {
            let sym = KgSymbolCandidate::new("enum", name.as_str(), LanguageKind::Cpp, file_path)
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
class UserService {
public:
    void process();
};

template<typename T>
class Container {
    T value;
};
"#;
        let symbols = extract_classes("src/services.hpp", code, FrameworkHint::None);
        let names: Vec<&str> = symbols.iter().map(|s| s.name.as_str()).collect();

        assert!(names.contains(&"UserService"));
        assert!(names.contains(&"Container"));
    }

    #[test]
    fn test_extract_namespaces() {
        let code = r#"
namespace utils {
    void helper();
}

namespace app {
    class Main {};
}
"#;
        let symbols = extract_namespaces("src/main.cpp", code, FrameworkHint::None);
        let names: Vec<&str> = symbols.iter().map(|s| s.name.as_str()).collect();

        assert!(names.contains(&"utils"));
        assert!(names.contains(&"app"));
    }

    #[test]
    fn test_extract_enums() {
        let code = r#"
enum Color {
    Red,
    Green,
    Blue
};

enum class Status : int {
    Active = 1,
    Inactive = 0
};
"#;
        let symbols = extract_enums("src/types.hpp", code, FrameworkHint::None);
        let names: Vec<&str> = symbols.iter().map(|s| s.name.as_str()).collect();

        assert!(names.contains(&"Color"));
        assert!(names.contains(&"Status"));
    }
}
