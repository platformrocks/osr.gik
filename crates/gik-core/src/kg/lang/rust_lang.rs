//! Rust language extractor.
//!
//! Extracts symbols from Rust files:
//! - Functions (fn)
//! - Structs
//! - Enums
//! - Traits
//! - Impl blocks
//! - Modules (mod)
//! - Constants and statics
//! - Type aliases

use regex::Regex;

use super::{
    FrameworkHint, KgRelationCandidate, KgSymbolCandidate, LanguageExtractor, LanguageKind,
};

/// Rust extractor.
#[derive(Debug, Clone, Default)]
pub struct RustExtractor;

impl RustExtractor {
    /// Create a new Rust extractor.
    pub fn new() -> Self {
        Self
    }
}

impl LanguageExtractor for RustExtractor {
    fn language(&self) -> LanguageKind {
        LanguageKind::Rust
    }

    fn detect_framework(&self, _file_path: &str, text: &str) -> FrameworkHint {
        // Axum detection
        if text.contains("axum::") || text.contains("use axum") {
            return FrameworkHint::Generic;
        }

        // Actix-web detection
        if text.contains("actix_web::") || text.contains("use actix_web") {
            return FrameworkHint::Generic;
        }

        FrameworkHint::None
    }

    fn extract_symbols(&self, file_path: &str, text: &str) -> Vec<KgSymbolCandidate> {
        let mut symbols = Vec::new();
        let framework = self.detect_framework(file_path, text);

        // Extract in order of typical importance
        symbols.extend(extract_structs(file_path, text, framework));
        symbols.extend(extract_enums(file_path, text, framework));
        symbols.extend(extract_traits(file_path, text, framework));
        symbols.extend(extract_functions(file_path, text, framework));
        symbols.extend(extract_modules(file_path, text, framework));
        symbols.extend(extract_type_aliases(file_path, text, framework));
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

    // Pattern: pub? async? fn name(...
    let fn_re = Regex::new(
        r"(?:pub(?:\([^)]*\))?\s+)?(?:async\s+)?(?:unsafe\s+)?fn\s+([a-zA-Z_][a-zA-Z0-9_]*)\s*[<(]",
    )
    .expect("Invalid regex");

    for cap in fn_re.captures_iter(text) {
        if let Some(name) = cap.get(1) {
            let sym =
                KgSymbolCandidate::new("function", name.as_str(), LanguageKind::Rust, file_path)
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

    // Pattern: pub? struct Name or struct Name
    let struct_re = Regex::new(r"(?:pub(?:\([^)]*\))?\s+)?struct\s+([a-zA-Z_][a-zA-Z0-9_]*)")
        .expect("Invalid regex");

    for cap in struct_re.captures_iter(text) {
        if let Some(name) = cap.get(1) {
            let sym =
                KgSymbolCandidate::new("struct", name.as_str(), LanguageKind::Rust, file_path)
                    .with_framework(framework);
            symbols.push(sym);
        }
    }

    symbols
}

/// Extract enum definitions.
fn extract_enums(file_path: &str, text: &str, framework: FrameworkHint) -> Vec<KgSymbolCandidate> {
    let mut symbols = Vec::new();

    // Pattern: pub? enum Name
    let enum_re = Regex::new(r"(?:pub(?:\([^)]*\))?\s+)?enum\s+([a-zA-Z_][a-zA-Z0-9_]*)")
        .expect("Invalid regex");

    for cap in enum_re.captures_iter(text) {
        if let Some(name) = cap.get(1) {
            let sym = KgSymbolCandidate::new("enum", name.as_str(), LanguageKind::Rust, file_path)
                .with_framework(framework);
            symbols.push(sym);
        }
    }

    symbols
}

/// Extract trait definitions.
fn extract_traits(file_path: &str, text: &str, framework: FrameworkHint) -> Vec<KgSymbolCandidate> {
    let mut symbols = Vec::new();

    // Pattern: pub? trait Name
    let trait_re = Regex::new(r"(?:pub(?:\([^)]*\))?\s+)?trait\s+([a-zA-Z_][a-zA-Z0-9_]*)")
        .expect("Invalid regex");

    for cap in trait_re.captures_iter(text) {
        if let Some(name) = cap.get(1) {
            let sym = KgSymbolCandidate::new("trait", name.as_str(), LanguageKind::Rust, file_path)
                .with_framework(framework);
            symbols.push(sym);
        }
    }

    symbols
}

/// Extract module declarations.
fn extract_modules(
    file_path: &str,
    text: &str,
    framework: FrameworkHint,
) -> Vec<KgSymbolCandidate> {
    let mut symbols = Vec::new();

    // Pattern: pub? mod name (but not mod tests)
    let mod_re = Regex::new(r"(?:pub(?:\([^)]*\))?\s+)?mod\s+([a-zA-Z_][a-zA-Z0-9_]*)\s*[{;]")
        .expect("Invalid regex");

    for cap in mod_re.captures_iter(text) {
        if let Some(name) = cap.get(1) {
            let name_str = name.as_str();
            // Skip test modules
            if name_str == "tests" || name_str == "test" {
                continue;
            }

            let sym = KgSymbolCandidate::new("module", name_str, LanguageKind::Rust, file_path)
                .with_framework(framework);
            symbols.push(sym);
        }
    }

    symbols
}

/// Extract type aliases.
fn extract_type_aliases(
    file_path: &str,
    text: &str,
    framework: FrameworkHint,
) -> Vec<KgSymbolCandidate> {
    let mut symbols = Vec::new();

    // Pattern: pub? type Name... = ... (allow generics)
    let type_re =
        Regex::new(r"(?:pub(?:\([^)]*\))?\s+)?type\s+([a-zA-Z_][a-zA-Z0-9_]*)(?:<[^>]+>)?\s*=")
            .expect("Invalid regex");

    for cap in type_re.captures_iter(text) {
        if let Some(name) = cap.get(1) {
            let sym = KgSymbolCandidate::new("type", name.as_str(), LanguageKind::Rust, file_path)
                .with_framework(framework);
            symbols.push(sym);
        }
    }

    symbols
}

/// Extract constants and statics.
fn extract_constants(
    file_path: &str,
    text: &str,
    framework: FrameworkHint,
) -> Vec<KgSymbolCandidate> {
    let mut symbols = Vec::new();

    // Pattern: pub? const NAME or pub? static NAME
    let const_re =
        Regex::new(r"(?:pub(?:\([^)]*\))?\s+)?(?:const|static(?:\s+mut)?)\s+([A-Z][A-Z0-9_]*)\s*:")
            .expect("Invalid regex");

    for cap in const_re.captures_iter(text) {
        if let Some(name) = cap.get(1) {
            let sym =
                KgSymbolCandidate::new("constant", name.as_str(), LanguageKind::Rust, file_path)
                    .with_framework(framework);
            symbols.push(sym);
        }
    }

    symbols
}

// ============================================================================
// Function Call Extraction
// ============================================================================

/// Common built-in functions/macros to skip (too noisy).
const BUILTIN_FUNCTIONS: &[&str] = &[
    // Macros
    "println", "print", "eprintln", "eprint", "format", "write", "writeln",
    "panic", "assert", "assert_eq", "assert_ne", "debug_assert", "debug_assert_eq",
    "todo", "unimplemented", "unreachable", "dbg", "vec", "format_args",
    "include_str", "include_bytes", "concat", "stringify", "env", "option_env",
    "cfg", "column", "file", "line", "module_path",
    // Common std functions
    "new", "default", "clone", "into", "from", "as_ref", "as_mut",
    "unwrap", "expect", "unwrap_or", "unwrap_or_else", "unwrap_or_default",
    "ok", "err", "map", "map_err", "and_then", "or_else", "ok_or", "ok_or_else",
    "is_some", "is_none", "is_ok", "is_err", "take", "replace",
    "len", "is_empty", "push", "pop", "insert", "remove", "contains", "get", "get_mut",
    "iter", "iter_mut", "into_iter", "collect", "filter", "map", "fold", "find",
    "any", "all", "skip", "take", "chain", "zip", "enumerate", "peekable",
    "to_string", "to_owned", "as_str", "as_bytes", "chars", "bytes",
    "parse", "from_str", "to_vec",
    "lock", "read", "write", "try_lock", "try_read", "try_write",
    "spawn", "join", "send", "recv", "try_recv",
    // Trait methods
    "eq", "ne", "lt", "le", "gt", "ge", "cmp", "partial_cmp",
    "add", "sub", "mul", "div", "rem", "neg", "not",
    "deref", "deref_mut", "index", "index_mut",
    "drop", "borrow", "borrow_mut",
];

/// Extract function call relations from Rust code.
fn extract_function_calls(file_path: &str, text: &str) -> Vec<KgRelationCandidate> {
    let mut relations = Vec::new();
    let file_node_id = format!("file:{}", file_path);
    let mut seen_calls = std::collections::HashSet::new();

    // Pattern 1: function_name(...) - direct function calls
    // Also matches: module::function(...)
    let direct_call_re =
        Regex::new(r"\b([a-zA-Z_][a-zA-Z0-9_]*)\s*(?:::<[^>]+>)?\s*\(").expect("Invalid regex");

    for cap in direct_call_re.captures_iter(text) {
        if let Some(name) = cap.get(1) {
            let name_str = name.as_str();

            // Skip built-in functions/macros
            if BUILTIN_FUNCTIONS.contains(&name_str) {
                continue;
            }

            // Skip keywords
            if matches!(
                name_str,
                "if" | "else" | "for" | "while" | "loop" | "match" | "fn"
                    | "struct" | "enum" | "trait" | "impl" | "mod" | "use" | "pub"
                    | "let" | "const" | "static" | "mut" | "ref" | "return"
                    | "break" | "continue" | "move" | "async" | "await"
                    | "where" | "type" | "unsafe" | "extern" | "crate" | "self"
                    | "super" | "dyn" | "as" | "in" | "Some" | "None" | "Ok" | "Err"
            ) {
                continue;
            }

            // Skip if already seen
            if seen_calls.contains(name_str) {
                continue;
            }
            seen_calls.insert(name_str.to_string());

            // Create unresolved call relation
            let callee_id = format!("sym:rs:*:function:{}", name_str);
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
        Regex::new(r"\b([a-zA-Z_][a-zA-Z0-9_]*)\.([a-zA-Z_][a-zA-Z0-9_]*)\s*(?:::<[^>]+>)?\s*\(")
            .expect("Invalid regex");

    for cap in method_call_re.captures_iter(text) {
        if let (Some(obj), Some(method)) = (cap.get(1), cap.get(2)) {
            let obj_str = obj.as_str();
            let method_str = method.as_str();

            // Skip built-in methods
            if BUILTIN_FUNCTIONS.contains(&method_str) {
                continue;
            }

            // Skip common receiver names with built-in methods
            if matches!(obj_str, "self" | "Self") && BUILTIN_FUNCTIONS.contains(&method_str) {
                continue;
            }

            // Skip if already seen
            let call_key = format!("{}.{}", obj_str, method_str);
            if seen_calls.contains(&call_key) {
                continue;
            }
            seen_calls.insert(call_key);

            // Create unresolved method call relation
            let callee_id = format!("sym:rs:*:method:{}#{}", obj_str, method_str);
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

    // Pattern 3: Type::associated_function(...) - associated function calls
    let assoc_call_re =
        Regex::new(r"\b([A-Z][a-zA-Z0-9_]*)::([a-zA-Z_][a-zA-Z0-9_]*)\s*(?:::<[^>]+>)?\s*\(")
            .expect("Invalid regex");

    for cap in assoc_call_re.captures_iter(text) {
        if let (Some(ty), Some(func)) = (cap.get(1), cap.get(2)) {
            let type_str = ty.as_str();
            let func_str = func.as_str();

            // Skip built-in types
            if matches!(
                type_str,
                "Option" | "Result" | "Vec" | "String" | "Box" | "Rc" | "Arc"
                    | "Cell" | "RefCell" | "Mutex" | "RwLock" | "HashMap" | "HashSet"
                    | "BTreeMap" | "BTreeSet" | "Path" | "PathBuf" | "File"
            ) {
                continue;
            }

            // Skip if already seen
            let call_key = format!("{}::{}", type_str, func_str);
            if seen_calls.contains(&call_key) {
                continue;
            }
            seen_calls.insert(call_key);

            // Create unresolved associated function call relation
            let callee_id = format!("sym:rs:*:function:{}::{}", type_str, func_str);
            let rel = KgRelationCandidate::new(&file_node_id, &callee_id, "calls")
                .with_props(serde_json::json!({
                    "callee": func_str,
                    "type": type_str,
                    "callType": "associated",
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
fn process_items(items: Vec<Item>) -> Vec<Item> {
    items
}

pub fn public_function() {
    // ...
}

pub(crate) async fn async_handler() {
    // ...
}

unsafe fn unsafe_operation() {
    // ...
}
"#;

        let symbols = extract_functions("src/lib.rs", code, FrameworkHint::None);
        let names: Vec<&str> = symbols.iter().map(|s| s.name.as_str()).collect();

        assert!(names.contains(&"process_items"));
        assert!(names.contains(&"public_function"));
        assert!(names.contains(&"async_handler"));
        assert!(names.contains(&"unsafe_operation"));
    }

    #[test]
    fn test_extract_structs() {
        let code = r#"
struct Point {
    x: f64,
    y: f64,
}

pub struct User {
    id: u64,
    name: String,
}

pub(crate) struct InternalConfig {
    debug: bool,
}
"#;

        let symbols = extract_structs("src/types.rs", code, FrameworkHint::None);
        let names: Vec<&str> = symbols.iter().map(|s| s.name.as_str()).collect();

        assert!(names.contains(&"Point"));
        assert!(names.contains(&"User"));
        assert!(names.contains(&"InternalConfig"));
    }

    #[test]
    fn test_extract_enums() {
        let code = r#"
enum Status {
    Active,
    Inactive,
}

pub enum Result<T, E> {
    Ok(T),
    Err(E),
}
"#;

        let symbols = extract_enums("src/types.rs", code, FrameworkHint::None);
        let names: Vec<&str> = symbols.iter().map(|s| s.name.as_str()).collect();

        assert!(names.contains(&"Status"));
        assert!(names.contains(&"Result"));
    }

    #[test]
    fn test_extract_traits() {
        let code = r#"
trait Drawable {
    fn draw(&self);
}

pub trait Repository<T> {
    fn find(&self, id: u64) -> Option<T>;
    fn save(&mut self, item: T);
}
"#;

        let symbols = extract_traits("src/traits.rs", code, FrameworkHint::None);
        let names: Vec<&str> = symbols.iter().map(|s| s.name.as_str()).collect();

        assert!(names.contains(&"Drawable"));
        assert!(names.contains(&"Repository"));
    }

    #[test]
    fn test_extract_modules() {
        let code = r#"
mod utils;

pub mod api;

pub(crate) mod internal {
    // ...
}

#[cfg(test)]
mod tests {
    // ...
}
"#;

        let symbols = extract_modules("src/lib.rs", code, FrameworkHint::None);
        let names: Vec<&str> = symbols.iter().map(|s| s.name.as_str()).collect();

        assert!(names.contains(&"utils"));
        assert!(names.contains(&"api"));
        assert!(names.contains(&"internal"));
        assert!(!names.contains(&"tests")); // tests module should be skipped
    }

    #[test]
    fn test_extract_type_aliases() {
        let code = r#"
type MyResult<T> = std::result::Result<T, Error>;

pub type UserId = u64;
"#;

        let symbols = extract_type_aliases("src/lib.rs", code, FrameworkHint::None);
        let names: Vec<&str> = symbols.iter().map(|s| s.name.as_str()).collect();

        assert!(names.contains(&"MyResult"));
        assert!(names.contains(&"UserId"));
    }

    #[test]
    fn test_extract_constants() {
        let code = r#"
const MAX_RETRIES: u32 = 3;

pub const DEFAULT_TIMEOUT: u64 = 30;

static GLOBAL_CONFIG: Config = Config::new();

pub static mut COUNTER: u64 = 0;
"#;

        let symbols = extract_constants("src/config.rs", code, FrameworkHint::None);
        let names: Vec<&str> = symbols.iter().map(|s| s.name.as_str()).collect();

        assert!(names.contains(&"MAX_RETRIES"));
        assert!(names.contains(&"DEFAULT_TIMEOUT"));
        assert!(names.contains(&"GLOBAL_CONFIG"));
        assert!(names.contains(&"COUNTER"));
    }

    #[test]
    fn test_extract_function_calls() {
        let code = r#"
fn process_request(req: Request) -> Response {
    let user = fetch_user(req.user_id);
    validate_request(&req);
    user.authenticate();
    db.save_session(&user);
    UserService::create_session(user.id);
    println!("Done");
    Ok(())
}

fn other_handler() {
    let data = prepare_data();
    api_client.send_request(data);
    Logger::info("Handled");
}
"#;

        let relations = extract_function_calls("src/handlers.rs", code);

        // Should find direct function calls
        assert!(relations.iter().any(|r| r.props["callee"] == "fetch_user"));
        assert!(relations.iter().any(|r| r.props["callee"] == "validate_request"));
        assert!(relations.iter().any(|r| r.props["callee"] == "prepare_data"));

        // Should find method calls
        assert!(relations
            .iter()
            .any(|r| r.props["callee"] == "authenticate" && r.props["receiver"] == "user"));
        assert!(relations
            .iter()
            .any(|r| r.props["callee"] == "save_session" && r.props["receiver"] == "db"));
        assert!(relations
            .iter()
            .any(|r| r.props["callee"] == "send_request" && r.props["receiver"] == "api_client"));

        // Should find associated function calls
        assert!(relations.iter().any(
            |r| r.props["callee"] == "create_session" && r.props["type"] == "UserService"
        ));
        assert!(
            relations
                .iter()
                .any(|r| r.props["callee"] == "info" && r.props["type"] == "Logger")
        );

        // Should NOT find built-in calls (Ok, println are skipped)
        assert!(!relations.iter().any(|r| r.props["callee"] == "println"));

        // All should be calls kind
        assert!(relations.iter().all(|r| r.kind == "calls"));
    }
}
