//! Python language extractor.
//!
//! Extracts symbols from Python files:
//! - Functions (def)
//! - Classes
//! - Methods (within classes)
//! - Module-level constants (UPPER_CASE assignments)

use regex::Regex;

use super::{
    FrameworkHint, KgRelationCandidate, KgSymbolCandidate, LanguageExtractor, LanguageKind,
};

/// Python extractor.
#[derive(Debug, Clone, Default)]
pub struct PythonExtractor;

impl PythonExtractor {
    /// Create a new Python extractor.
    pub fn new() -> Self {
        Self
    }
}

impl LanguageExtractor for PythonExtractor {
    fn language(&self) -> LanguageKind {
        LanguageKind::Python
    }

    fn detect_framework(&self, file_path: &str, text: &str) -> FrameworkHint {
        // Django detection
        if file_path.contains("/views.py")
            || file_path.contains("/models.py")
            || file_path.contains("/urls.py")
        {
            return FrameworkHint::Django;
        }
        if text.contains("from django") || text.contains("import django") {
            return FrameworkHint::Django;
        }

        // Flask detection
        if text.contains("from flask import") || text.contains("Flask(__name__)") {
            return FrameworkHint::Flask;
        }

        FrameworkHint::None
    }

    fn extract_symbols(&self, file_path: &str, text: &str) -> Vec<KgSymbolCandidate> {
        let mut symbols = Vec::new();
        let framework = self.detect_framework(file_path, text);

        // Extract classes first
        symbols.extend(extract_classes(file_path, text, framework));

        // Extract functions
        symbols.extend(extract_functions(file_path, text, framework));

        // Extract constants
        symbols.extend(extract_constants(file_path, text, framework));

        symbols
    }

    fn extract_relations(&self, file_path: &str, text: &str) -> Vec<KgRelationCandidate> {
        let mut relations = Vec::new();

        // Extract function call relations
        relations.extend(extract_function_calls(file_path, text));

        relations
    }
}

/// Extract function definitions.
fn extract_functions(
    file_path: &str,
    text: &str,
    framework: FrameworkHint,
) -> Vec<KgSymbolCandidate> {
    let mut symbols = Vec::new();

    // Pattern: def function_name(...)
    // This regex captures both module-level and class methods
    let fn_re = Regex::new(r"(?:^|\n)([ \t]*)(?:async\s+)?def\s+([a-zA-Z_][a-zA-Z0-9_]*)\s*\(")
        .expect("Invalid regex");

    for cap in fn_re.captures_iter(text) {
        let indent = cap.get(1).map(|m| m.as_str()).unwrap_or("");
        if let Some(name) = cap.get(2) {
            let name_str = name.as_str();
            // Skip dunder methods (they're usually implementation details)
            if name_str.starts_with("__") && name_str.ends_with("__") && name_str != "__init__" {
                continue;
            }

            // Determine if this is a method (indented) or module-level function
            let kind = if indent.is_empty() {
                "function"
            } else {
                "method"
            };

            let sym = KgSymbolCandidate::new(kind, name_str, LanguageKind::Python, file_path)
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

    // Pattern: class ClassName or class ClassName(...)
    let class_re =
        Regex::new(r"(?:^|\n)class\s+([a-zA-Z_][a-zA-Z0-9_]*)\s*[:\(]").expect("Invalid regex");

    for cap in class_re.captures_iter(text) {
        if let Some(name) = cap.get(1) {
            let sym =
                KgSymbolCandidate::new("class", name.as_str(), LanguageKind::Python, file_path)
                    .with_framework(framework);
            symbols.push(sym);
        }
    }

    symbols
}

/// Extract module-level constants (UPPER_CASE assignments).
fn extract_constants(
    file_path: &str,
    text: &str,
    framework: FrameworkHint,
) -> Vec<KgSymbolCandidate> {
    let mut symbols = Vec::new();

    // Pattern: CONSTANT_NAME = ... at module level (no leading whitespace)
    let const_re = Regex::new(r"(?:^|\n)([A-Z][A-Z0-9_]*)\s*=").expect("Invalid regex");

    for cap in const_re.captures_iter(text) {
        if let Some(name) = cap.get(1) {
            let name_str = name.as_str();
            // Skip common non-constant patterns
            if name_str == "T" || name_str == "K" || name_str == "V" {
                continue;
            }

            let sym = KgSymbolCandidate::new("constant", name_str, LanguageKind::Python, file_path)
                .with_framework(framework);
            symbols.push(sym);
        }
    }

    symbols
}

// ============================================================================
// Function Call Extraction
// ============================================================================

/// Common built-in functions/methods to skip (too noisy).
const BUILTIN_FUNCTIONS: &[&str] = &[
    // Built-in functions
    "print", "len", "range", "str", "int", "float", "bool", "list", "dict", "set", "tuple",
    "type", "isinstance", "issubclass", "hasattr", "getattr", "setattr", "delattr",
    "open", "input", "format", "repr", "id", "hash", "dir", "vars", "globals", "locals",
    "iter", "next", "enumerate", "zip", "map", "filter", "sorted", "reversed", "min", "max",
    "sum", "abs", "round", "pow", "divmod", "all", "any", "callable", "super", "property",
    "staticmethod", "classmethod", "ord", "chr", "hex", "bin", "oct", "bytes", "bytearray",
    // Common methods
    "append", "extend", "insert", "remove", "pop", "clear", "index", "count", "sort", "reverse",
    "copy", "get", "keys", "values", "items", "update", "setdefault",
    "split", "join", "strip", "lstrip", "rstrip", "lower", "upper", "title", "capitalize",
    "replace", "find", "rfind", "startswith", "endswith", "encode", "decode",
    "read", "write", "readline", "readlines", "close", "flush", "seek", "tell",
    "add", "discard", "union", "intersection", "difference",
    // Dunder methods
    "__init__", "__new__", "__del__", "__repr__", "__str__", "__call__",
];

/// Extract function call relations from Python code.
fn extract_function_calls(file_path: &str, text: &str) -> Vec<KgRelationCandidate> {
    let mut relations = Vec::new();
    let file_node_id = format!("file:{}", file_path);
    let mut seen_calls = std::collections::HashSet::new();

    // Pattern 1: function_name(...) - direct function calls
    let direct_call_re = Regex::new(r"\b([a-zA-Z_][a-zA-Z0-9_]*)\s*\(").expect("Invalid regex");

    for cap in direct_call_re.captures_iter(text) {
        if let Some(name) = cap.get(1) {
            let name_str = name.as_str();

            // Skip built-in functions
            if BUILTIN_FUNCTIONS.contains(&name_str) {
                continue;
            }

            // Skip keywords
            if matches!(
                name_str,
                "if" | "for" | "while" | "with" | "try" | "except" | "finally"
                    | "def" | "class" | "lambda" | "return" | "yield" | "raise"
                    | "import" | "from" | "as" | "pass" | "break" | "continue"
                    | "and" | "or" | "not" | "in" | "is" | "True" | "False" | "None"
            ) {
                continue;
            }

            // Skip if already seen
            if seen_calls.contains(name_str) {
                continue;
            }
            seen_calls.insert(name_str.to_string());

            // Create unresolved call relation
            let callee_id = format!("sym:py:*:function:{}", name_str);
            let rel = KgRelationCandidate::new(&file_node_id, &callee_id, "calls")
                .with_props(serde_json::json!({
                    "callee": name_str,
                    "callType": "direct",
                    "unresolved": true
                }));
            relations.push(rel);
        }
    }

    // Pattern 2: object.method(...) - method calls
    let method_call_re =
        Regex::new(r"\b([a-zA-Z_][a-zA-Z0-9_]*)\.([a-zA-Z_][a-zA-Z0-9_]*)\s*\(")
            .expect("Invalid regex");

    for cap in method_call_re.captures_iter(text) {
        if let (Some(obj), Some(method)) = (cap.get(1), cap.get(2)) {
            let obj_str = obj.as_str();
            let method_str = method.as_str();

            // Skip built-in methods
            if BUILTIN_FUNCTIONS.contains(&method_str) {
                continue;
            }

            // Skip common module prefixes
            if matches!(obj_str, "os" | "sys" | "re" | "json" | "math" | "random" | "datetime" | "time") {
                continue;
            }

            // Skip self/cls method calls on built-ins
            if matches!(obj_str, "self" | "cls") && BUILTIN_FUNCTIONS.contains(&method_str) {
                continue;
            }

            // Skip if already seen
            let call_key = format!("{}.{}", obj_str, method_str);
            if seen_calls.contains(&call_key) {
                continue;
            }
            seen_calls.insert(call_key);

            // Create unresolved method call relation
            let callee_id = format!("sym:py:*:method:{}#{}", obj_str, method_str);
            let rel = KgRelationCandidate::new(&file_node_id, &callee_id, "calls")
                .with_props(serde_json::json!({
                    "callee": method_str,
                    "receiver": obj_str,
                    "callType": "method",
                    "unresolved": true
                }));
            relations.push(rel);
        }
    }

    relations
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_functions() {
        let code = r#"
def process_data(items):
    return [i for i in items]

async def fetch_user(user_id):
    return await db.get(user_id)

def _private_helper():
    pass
"#;

        let symbols = extract_functions("src/utils.py", code, FrameworkHint::None);
        let names: Vec<&str> = symbols.iter().map(|s| s.name.as_str()).collect();

        assert!(names.contains(&"process_data"));
        assert!(names.contains(&"fetch_user"));
        assert!(names.contains(&"_private_helper"));
    }

    #[test]
    fn test_extract_classes() {
        let code = r#"
class UserService:
    def __init__(self):
        pass

class ApiClient(BaseClient):
    pass

class DataProcessor:
    """Processes data."""
    pass
"#;

        let symbols = extract_classes("src/services.py", code, FrameworkHint::None);
        let names: Vec<&str> = symbols.iter().map(|s| s.name.as_str()).collect();

        assert!(names.contains(&"UserService"));
        assert!(names.contains(&"ApiClient"));
        assert!(names.contains(&"DataProcessor"));
    }

    #[test]
    fn test_extract_methods() {
        let code = r#"
class UserService:
    def __init__(self, db):
        self.db = db

    def get_user(self, user_id):
        return self.db.get(user_id)

    async def create_user(self, data):
        return await self.db.create(data)
"#;

        let symbols = extract_functions("src/services.py", code, FrameworkHint::None);
        let methods: Vec<&KgSymbolCandidate> =
            symbols.iter().filter(|s| s.kind == "method").collect();

        assert_eq!(methods.len(), 3); // __init__, get_user, create_user
    }

    #[test]
    fn test_extract_constants() {
        let code = r#"
MAX_RETRIES = 3
DEFAULT_TIMEOUT = 30
API_BASE_URL = "https://api.example.com"

# These should not be extracted
some_variable = "value"
"#;

        let symbols = extract_constants("src/config.py", code, FrameworkHint::None);
        let names: Vec<&str> = symbols.iter().map(|s| s.name.as_str()).collect();

        assert!(names.contains(&"MAX_RETRIES"));
        assert!(names.contains(&"DEFAULT_TIMEOUT"));
        assert!(names.contains(&"API_BASE_URL"));
        assert!(!names.contains(&"some_variable"));
    }

    #[test]
    fn test_detect_django() {
        let extractor = PythonExtractor::new();

        assert_eq!(
            extractor.detect_framework("myapp/views.py", ""),
            FrameworkHint::Django
        );
        assert_eq!(
            extractor.detect_framework("myapp/models.py", ""),
            FrameworkHint::Django
        );
        assert_eq!(
            extractor.detect_framework("src/api.py", "from django.db import models"),
            FrameworkHint::Django
        );
    }

    #[test]
    fn test_detect_flask() {
        let extractor = PythonExtractor::new();

        assert_eq!(
            extractor.detect_framework("app.py", "from flask import Flask"),
            FrameworkHint::Flask
        );
        assert_eq!(
            extractor.detect_framework("app.py", "app = Flask(__name__)"),
            FrameworkHint::Flask
        );
    }

    #[test]
    fn test_skip_dunder_methods() {
        let code = r#"
class MyClass:
    def __init__(self):
        pass

    def __str__(self):
        return "MyClass"

    def __repr__(self):
        return "MyClass()"

    def normal_method(self):
        pass
"#;

        let symbols = extract_functions("src/model.py", code, FrameworkHint::None);
        let names: Vec<&str> = symbols.iter().map(|s| s.name.as_str()).collect();

        // __init__ is kept, others are skipped
        assert!(names.contains(&"__init__"));
        assert!(!names.contains(&"__str__"));
        assert!(!names.contains(&"__repr__"));
        assert!(names.contains(&"normal_method"));
    }

    #[test]
    fn test_extract_function_calls() {
        let code = r#"
def process_order(order_id):
    order = fetch_order(order_id)
    validate_order(order)
    order.calculate_total()
    payment_service.process_payment(order.total)
    print("Order processed")
    return order

class OrderService:
    def handle(self):
        data = self.prepare_data()
        result = external_api.send_request(data)
        return result
"#;

        let relations = extract_function_calls("src/orders.py", code);

        // Should find direct function calls
        assert!(relations.iter().any(|r| r.props["callee"] == "fetch_order"));
        assert!(relations.iter().any(|r| r.props["callee"] == "validate_order"));

        // Should find method calls
        assert!(relations
            .iter()
            .any(|r| r.props["callee"] == "calculate_total" && r.props["receiver"] == "order"));
        assert!(relations.iter().any(|r| r.props["callee"] == "process_payment"
            && r.props["receiver"] == "payment_service"));
        assert!(relations.iter().any(|r| r.props["callee"] == "send_request"
            && r.props["receiver"] == "external_api"));

        // Should NOT find built-in calls
        assert!(!relations.iter().any(|r| r.props["callee"] == "print"));

        // All should be calls kind
        assert!(relations.iter().all(|r| r.kind == "calls"));
    }
}
