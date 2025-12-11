//! Java language extractor.
//!
//! Extracts symbols from Java files:
//! - Classes
//! - Interfaces
//! - Enums
//! - Methods
//! - Records (Java 14+)

use regex::Regex;

use super::{
    FrameworkHint, KgRelationCandidate, KgSymbolCandidate, LanguageExtractor, LanguageKind,
};

/// Java extractor.
#[derive(Debug, Clone, Default)]
pub struct JavaExtractor;

impl JavaExtractor {
    /// Create a new Java extractor.
    pub fn new() -> Self {
        Self
    }
}

impl LanguageExtractor for JavaExtractor {
    fn language(&self) -> LanguageKind {
        LanguageKind::Java
    }

    fn detect_framework(&self, _file_path: &str, text: &str) -> FrameworkHint {
        // Spring detection
        if text.contains("@RestController")
            || text.contains("@Controller")
            || text.contains("@Service")
            || text.contains("@Repository")
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
        symbols.extend(extract_interfaces(file_path, text, framework));
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
        r"(?:public|private|protected)?\s*(?:abstract|final|static)?\s*class\s+([A-Z][a-zA-Z0-9_]*)",
    )
    .expect("Invalid regex");

    for cap in class_re.captures_iter(text) {
        if let Some(name) = cap.get(1) {
            let sym = KgSymbolCandidate::new("class", name.as_str(), LanguageKind::Java, file_path)
                .with_framework(framework);
            symbols.push(sym);
        }
    }

    // Also extract records (Java 14+)
    let record_re = Regex::new(r"(?:public|private|protected)?\s*record\s+([A-Z][a-zA-Z0-9_]*)")
        .expect("Invalid regex");

    for cap in record_re.captures_iter(text) {
        if let Some(name) = cap.get(1) {
            let sym =
                KgSymbolCandidate::new("record", name.as_str(), LanguageKind::Java, file_path)
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

    let iface_re = Regex::new(r"(?:public|private|protected)?\s*interface\s+([A-Z][a-zA-Z0-9_]*)")
        .expect("Invalid regex");

    for cap in iface_re.captures_iter(text) {
        if let Some(name) = cap.get(1) {
            let sym =
                KgSymbolCandidate::new("interface", name.as_str(), LanguageKind::Java, file_path)
                    .with_framework(framework);
            symbols.push(sym);
        }
    }

    symbols
}

/// Extract enum definitions.
fn extract_enums(file_path: &str, text: &str, framework: FrameworkHint) -> Vec<KgSymbolCandidate> {
    let mut symbols = Vec::new();

    let enum_re = Regex::new(r"(?:public|private|protected)?\s*enum\s+([A-Z][a-zA-Z0-9_]*)")
        .expect("Invalid regex");

    for cap in enum_re.captures_iter(text) {
        if let Some(name) = cap.get(1) {
            let sym = KgSymbolCandidate::new("enum", name.as_str(), LanguageKind::Java, file_path)
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

    // Pattern for methods
    let method_re = Regex::new(
        r"(?:public|private|protected)\s+(?:static\s+)?(?:final\s+)?(?:synchronized\s+)?(?:\w+(?:<[^>]+>)?)\s+([a-z][a-zA-Z0-9_]*)\s*\(",
    )
    .expect("Invalid regex");

    for cap in method_re.captures_iter(text) {
        if let Some(name) = cap.get(1) {
            let sym =
                KgSymbolCandidate::new("method", name.as_str(), LanguageKind::Java, file_path)
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
public class UserService {
    // ...
}

abstract class BaseRepository {
    // ...
}

public final class Constants {
    // ...
}

public record UserDto(String name, int age) {}
"#;
        let symbols = extract_classes("src/UserService.java", code, FrameworkHint::None);
        let names: Vec<&str> = symbols.iter().map(|s| s.name.as_str()).collect();

        assert!(names.contains(&"UserService"));
        assert!(names.contains(&"BaseRepository"));
        assert!(names.contains(&"Constants"));
        assert!(names.contains(&"UserDto"));
    }

    #[test]
    fn test_extract_interfaces() {
        let code = r#"
public interface UserRepository {
    User findById(Long id);
}

interface InternalService {
    void process();
}
"#;
        let symbols = extract_interfaces("src/UserRepository.java", code, FrameworkHint::None);
        let names: Vec<&str> = symbols.iter().map(|s| s.name.as_str()).collect();

        assert!(names.contains(&"UserRepository"));
        assert!(names.contains(&"InternalService"));
    }

    #[test]
    fn test_extract_methods() {
        let code = r#"
public class UserService {
    public User findById(Long id) {
        return null;
    }

    private void processInternal() {
        // ...
    }

    public static UserService getInstance() {
        return instance;
    }
}
"#;
        let symbols = extract_methods("src/UserService.java", code, FrameworkHint::None);
        let names: Vec<&str> = symbols.iter().map(|s| s.name.as_str()).collect();

        assert!(names.contains(&"findById"));
        assert!(names.contains(&"processInternal"));
        assert!(names.contains(&"getInstance"));
    }

    #[test]
    fn test_detect_spring() {
        let extractor = JavaExtractor::new();

        assert_eq!(
            extractor.detect_framework("src/UserController.java", "@RestController"),
            FrameworkHint::Spring
        );
        assert_eq!(
            extractor.detect_framework("src/UserService.java", "@Service"),
            FrameworkHint::Spring
        );
    }
}
