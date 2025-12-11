//! Kotlin language extractor.
//!
//! Extracts symbols from Kotlin files:
//! - Classes
//! - Data classes
//! - Objects
//! - Interfaces
//! - Functions
//! - Properties

use regex::Regex;

use super::{
    FrameworkHint, KgRelationCandidate, KgSymbolCandidate, LanguageExtractor, LanguageKind,
};

/// Kotlin extractor.
#[derive(Debug, Clone, Default)]
pub struct KotlinExtractor;

impl KotlinExtractor {
    /// Create a new Kotlin extractor.
    pub fn new() -> Self {
        Self
    }
}

impl LanguageExtractor for KotlinExtractor {
    fn language(&self) -> LanguageKind {
        LanguageKind::Kotlin
    }

    fn detect_framework(&self, _file_path: &str, text: &str) -> FrameworkHint {
        // Spring detection (Kotlin is often used with Spring)
        if text.contains("@RestController")
            || text.contains("@Controller")
            || text.contains("@Service")
            || text.contains("org.springframework")
        {
            return FrameworkHint::Spring;
        }

        FrameworkHint::None
    }

    fn extract_symbols(&self, file_path: &str, text: &str) -> Vec<KgSymbolCandidate> {
        let mut symbols = Vec::new();
        let framework = self.detect_framework(file_path, text);

        symbols.extend(extract_classes(file_path, text, framework));
        symbols.extend(extract_objects(file_path, text, framework));
        symbols.extend(extract_interfaces(file_path, text, framework));
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

    // Pattern: class Name, data class Name, sealed class Name, etc.
    let class_re = Regex::new(
        r"(?:abstract\s+|open\s+|sealed\s+|data\s+|inner\s+)?class\s+([A-Z][a-zA-Z0-9_]*)",
    )
    .expect("Invalid regex");

    for cap in class_re.captures_iter(text) {
        if let Some(name) = cap.get(1) {
            let sym =
                KgSymbolCandidate::new("class", name.as_str(), LanguageKind::Kotlin, file_path)
                    .with_framework(framework);
            symbols.push(sym);
        }
    }

    symbols
}

/// Extract object declarations.
fn extract_objects(
    file_path: &str,
    text: &str,
    framework: FrameworkHint,
) -> Vec<KgSymbolCandidate> {
    let mut symbols = Vec::new();

    let object_re = Regex::new(r"object\s+([A-Z][a-zA-Z0-9_]*)").expect("Invalid regex");

    for cap in object_re.captures_iter(text) {
        if let Some(name) = cap.get(1) {
            let sym =
                KgSymbolCandidate::new("object", name.as_str(), LanguageKind::Kotlin, file_path)
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
                KgSymbolCandidate::new("interface", name.as_str(), LanguageKind::Kotlin, file_path)
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
        r"(?:suspend\s+)?(?:private\s+|public\s+|internal\s+|protected\s+)?fun\s+([a-zA-Z_][a-zA-Z0-9_]*)\s*\(",
    )
    .expect("Invalid regex");

    for cap in fn_re.captures_iter(text) {
        if let Some(name) = cap.get(1) {
            let sym =
                KgSymbolCandidate::new("function", name.as_str(), LanguageKind::Kotlin, file_path)
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
    // ...
}

data class User(val id: Long, val name: String)

sealed class Result<out T> {
    data class Success<T>(val data: T) : Result<T>()
    data class Error(val message: String) : Result<Nothing>()
}

abstract class BaseRepository {
    // ...
}
"#;
        let symbols = extract_classes("src/UserService.kt", code, FrameworkHint::None);
        let names: Vec<&str> = symbols.iter().map(|s| s.name.as_str()).collect();

        assert!(names.contains(&"UserService"));
        assert!(names.contains(&"User"));
        assert!(names.contains(&"Result"));
        assert!(names.contains(&"Success"));
        assert!(names.contains(&"Error"));
        assert!(names.contains(&"BaseRepository"));
    }

    #[test]
    fn test_extract_objects() {
        let code = r#"
object Database {
    fun connect() {}
}

object Constants {
    const val MAX_SIZE = 100
}
"#;
        let symbols = extract_objects("src/Database.kt", code, FrameworkHint::None);
        let names: Vec<&str> = symbols.iter().map(|s| s.name.as_str()).collect();

        assert!(names.contains(&"Database"));
        assert!(names.contains(&"Constants"));
    }

    #[test]
    fn test_extract_functions() {
        let code = r#"
fun main() {
    println("Hello")
}

suspend fun fetchData(): Data {
    return Data()
}

private fun helper() {
    // ...
}
"#;
        let symbols = extract_functions("src/main.kt", code, FrameworkHint::None);
        let names: Vec<&str> = symbols.iter().map(|s| s.name.as_str()).collect();

        assert!(names.contains(&"main"));
        assert!(names.contains(&"fetchData"));
        assert!(names.contains(&"helper"));
    }

    #[test]
    fn test_detect_spring() {
        let extractor = KotlinExtractor::new();

        assert_eq!(
            extractor.detect_framework("src/UserController.kt", "@RestController"),
            FrameworkHint::Spring
        );
    }
}
