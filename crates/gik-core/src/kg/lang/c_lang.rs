//! C language extractor.
//!
//! Extracts symbols from C files:
//! - Functions
//! - Structs
//! - Enums
//! - Typedefs
//! - Macros

use regex::Regex;

use super::{
    FrameworkHint, KgRelationCandidate, KgSymbolCandidate, LanguageExtractor, LanguageKind,
};

/// C extractor.
#[derive(Debug, Clone, Default)]
pub struct CExtractor;

impl CExtractor {
    /// Create a new C extractor.
    pub fn new() -> Self {
        Self
    }
}

impl LanguageExtractor for CExtractor {
    fn language(&self) -> LanguageKind {
        LanguageKind::C
    }

    fn detect_framework(&self, _file_path: &str, _text: &str) -> FrameworkHint {
        FrameworkHint::None
    }

    fn extract_symbols(&self, file_path: &str, text: &str) -> Vec<KgSymbolCandidate> {
        let mut symbols = Vec::new();
        let framework = self.detect_framework(file_path, text);

        symbols.extend(extract_functions(file_path, text, framework));
        symbols.extend(extract_structs(file_path, text, framework));
        symbols.extend(extract_enums(file_path, text, framework));
        symbols.extend(extract_typedefs(file_path, text, framework));
        symbols.extend(extract_macros(file_path, text, framework));

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

    // Pattern: return_type function_name(...)
    let fn_re = Regex::new(
        r"(?:static\s+)?(?:inline\s+)?(?:const\s+)?(?:\w+\s*\*?\s+)+([a-zA-Z_][a-zA-Z0-9_]*)\s*\([^)]*\)\s*\{",
    )
    .expect("Invalid regex");

    for cap in fn_re.captures_iter(text) {
        if let Some(name) = cap.get(1) {
            let name_str = name.as_str();
            // Skip common C keywords that might be captured
            if name_str == "if"
                || name_str == "while"
                || name_str == "for"
                || name_str == "switch"
                || name_str == "return"
            {
                continue;
            }

            let sym = KgSymbolCandidate::new("function", name_str, LanguageKind::C, file_path)
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

    // Pattern: struct name { or typedef struct { ... } name
    let struct_re = Regex::new(r"struct\s+([a-zA-Z_][a-zA-Z0-9_]*)\s*\{").expect("Invalid regex");

    for cap in struct_re.captures_iter(text) {
        if let Some(name) = cap.get(1) {
            let sym = KgSymbolCandidate::new("struct", name.as_str(), LanguageKind::C, file_path)
                .with_framework(framework);
            symbols.push(sym);
        }
    }

    symbols
}

/// Extract enum definitions.
fn extract_enums(file_path: &str, text: &str, framework: FrameworkHint) -> Vec<KgSymbolCandidate> {
    let mut symbols = Vec::new();

    let enum_re = Regex::new(r"enum\s+([a-zA-Z_][a-zA-Z0-9_]*)\s*\{").expect("Invalid regex");

    for cap in enum_re.captures_iter(text) {
        if let Some(name) = cap.get(1) {
            let sym = KgSymbolCandidate::new("enum", name.as_str(), LanguageKind::C, file_path)
                .with_framework(framework);
            symbols.push(sym);
        }
    }

    symbols
}

/// Extract typedef definitions.
fn extract_typedefs(
    file_path: &str,
    text: &str,
    framework: FrameworkHint,
) -> Vec<KgSymbolCandidate> {
    let mut symbols = Vec::new();

    // Pattern: typedef ... name;
    let typedef_re =
        Regex::new(r"typedef\s+.+?\s+([a-zA-Z_][a-zA-Z0-9_]*)\s*;").expect("Invalid regex");

    for cap in typedef_re.captures_iter(text) {
        if let Some(name) = cap.get(1) {
            let sym = KgSymbolCandidate::new("typedef", name.as_str(), LanguageKind::C, file_path)
                .with_framework(framework);
            symbols.push(sym);
        }
    }

    symbols
}

/// Extract macro definitions.
fn extract_macros(file_path: &str, text: &str, framework: FrameworkHint) -> Vec<KgSymbolCandidate> {
    let mut symbols = Vec::new();

    // Pattern: #define NAME
    let macro_re = Regex::new(r"#define\s+([A-Z_][A-Z0-9_]*)").expect("Invalid regex");

    for cap in macro_re.captures_iter(text) {
        if let Some(name) = cap.get(1) {
            let sym = KgSymbolCandidate::new("macro", name.as_str(), LanguageKind::C, file_path)
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
int main(int argc, char **argv) {
    return 0;
}

static void helper_function(void) {
    // ...
}

void process_data(int *buffer) {
    // ...
}
"#;
        let symbols = extract_functions("src/main.c", code, FrameworkHint::None);
        let names: Vec<&str> = symbols.iter().map(|s| s.name.as_str()).collect();

        assert!(names.contains(&"main"));
        assert!(names.contains(&"helper_function"));
        assert!(names.contains(&"process_data"));
    }

    #[test]
    fn test_extract_structs() {
        let code = r#"
struct Point {
    int x;
    int y;
};

struct User {
    char *name;
    int age;
};
"#;
        let symbols = extract_structs("src/types.h", code, FrameworkHint::None);
        let names: Vec<&str> = symbols.iter().map(|s| s.name.as_str()).collect();

        assert!(names.contains(&"Point"));
        assert!(names.contains(&"User"));
    }

    #[test]
    fn test_extract_macros() {
        let code = r#"
#define MAX_SIZE 1024
#define MIN_VALUE 0
#define DEBUG_MODE
"#;
        let symbols = extract_macros("src/config.h", code, FrameworkHint::None);
        let names: Vec<&str> = symbols.iter().map(|s| s.name.as_str()).collect();

        assert!(names.contains(&"MAX_SIZE"));
        assert!(names.contains(&"MIN_VALUE"));
        assert!(names.contains(&"DEBUG_MODE"));
    }
}
