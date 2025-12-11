//! JavaScript/TypeScript language extractor.
//!
//! Extracts symbols from JS/TS files:
//! - Functions (function declarations, arrow functions, exports)
//! - Classes
//! - Interfaces (TypeScript)
//! - Type aliases (TypeScript)
//! - Namespaces (TypeScript)
//! - React components (function and class components)
//! - shadcn/ui component imports and usage
//! - Angular decorators (@Component, @NgModule, @Injectable)
//! - Tailwind/CSS className usage in JSX

use regex::Regex;

use super::{
    FrameworkHint, KgRelationCandidate, KgSymbolCandidate, LanguageExtractor, LanguageKind,
};

/// JavaScript/TypeScript extractor.
#[derive(Debug, Clone, Default)]
pub struct JsTsExtractor;

impl JsTsExtractor {
    /// Create a new JS/TS extractor.
    pub fn new() -> Self {
        Self
    }
}

impl LanguageExtractor for JsTsExtractor {
    fn language(&self) -> LanguageKind {
        LanguageKind::JsTs
    }

    fn detect_framework(&self, file_path: &str, text: &str) -> FrameworkHint {
        // Angular detection (check first as it's most specific)
        if text.contains("@Component(") || text.contains("@NgModule(") {
            return FrameworkHint::Angular;
        }

        // Next.js detection (path-based and import-based)
        if file_path.contains("/app/") && file_path.contains("/api/") {
            return FrameworkHint::NextJs;
        }
        if file_path.contains("/pages/api/") {
            return FrameworkHint::NextJs;
        }
        if text.contains("from 'next") || text.contains("from \"next") {
            return FrameworkHint::NextJs;
        }

        // shadcn/ui detection
        if text.contains("@/components/ui/") || text.contains("from '@/components/ui") {
            return FrameworkHint::Shadcn;
        }

        // Generic React detection
        if text.contains("from 'react'")
            || text.contains("from \"react\"")
            || text.contains("import React")
            || text.contains("React.Component")
            || text.contains("useState")
            || text.contains("useEffect")
            || (file_path.ends_with(".jsx") || file_path.ends_with(".tsx"))
                && text.contains("return (")
        {
            return FrameworkHint::React;
        }

        // Express detection
        if text.contains("express()") || text.contains("from 'express'") {
            return FrameworkHint::Express;
        }

        // Angular detection
        if text.contains("@angular/core")
            || text.contains("@angular/common")
            || text.contains("@Component(")
            || text.contains("@NgModule(")
            || text.contains("@Injectable(")
        {
            return FrameworkHint::Angular;
        }

        FrameworkHint::None
    }

    fn extract_symbols(&self, file_path: &str, text: &str) -> Vec<KgSymbolCandidate> {
        let mut symbols = Vec::new();
        let framework = self.detect_framework(file_path, text);

        // Extract React components (before functions to capture PascalCase names)
        if matches!(
            framework,
            FrameworkHint::React | FrameworkHint::NextJs | FrameworkHint::Shadcn
        ) || is_jsx_tsx(file_path)
        {
            symbols.extend(extract_react_components(file_path, text, framework));
        }

        // Extract shadcn/ui component imports
        if framework == FrameworkHint::Shadcn || text.contains("@/components/ui") {
            symbols.extend(extract_shadcn_components(file_path, text));
        }

        // Extract Angular decorators
        if framework == FrameworkHint::Angular || text.contains("@Component(") {
            symbols.extend(extract_angular_symbols(file_path, text));
        }

        // Extract Angular routes (from routing modules)
        if framework == FrameworkHint::Angular
            || file_path.contains("routing")
            || text.contains("Routes")
        {
            symbols.extend(extract_angular_routes(file_path, text));
        }

        // Extract functions (excluding React components already captured)
        symbols.extend(extract_functions(file_path, text, framework));

        // Extract classes (excluding React class components already captured)
        symbols.extend(extract_classes(file_path, text, framework));

        // Extract interfaces (TypeScript)
        if is_typescript(file_path) {
            symbols.extend(extract_interfaces(file_path, text, framework));
            symbols.extend(extract_type_aliases(file_path, text, framework));
        }

        symbols
    }

    fn extract_relations(&self, file_path: &str, text: &str) -> Vec<KgRelationCandidate> {
        let mut relations = Vec::new();
        let framework = self.detect_framework(file_path, text);

        // Extract className/class usage from JSX (usesClass relations)
        if is_jsx_tsx(file_path)
            || matches!(
                framework,
                FrameworkHint::React | FrameworkHint::NextJs | FrameworkHint::Shadcn
            )
        {
            relations.extend(extract_classname_relations(file_path, text));
        }

        // Extract shadcn/ui component usage relations
        if framework == FrameworkHint::Shadcn || text.contains("@/components/ui") {
            relations.extend(extract_shadcn_usage_relations(file_path, text));
        }

        // Extract Angular module membership relations
        if framework == FrameworkHint::Angular {
            relations.extend(extract_angular_relations(file_path, text));
        }

        // Extract function call relations
        relations.extend(extract_function_calls(file_path, text));

        relations
    }
}

/// Check if a file is TypeScript.
fn is_typescript(file_path: &str) -> bool {
    file_path.ends_with(".ts") || file_path.ends_with(".tsx")
}

/// Check if a file is JSX/TSX.
fn is_jsx_tsx(file_path: &str) -> bool {
    file_path.ends_with(".jsx") || file_path.ends_with(".tsx")
}

/// Check if a name is PascalCase (likely a React component).
fn is_pascal_case(name: &str) -> bool {
    if name.is_empty() {
        return false;
    }
    let first_char = name.chars().next().unwrap();
    first_char.is_uppercase() && name.chars().skip(1).any(|c| c.is_lowercase())
}

/// HTTP methods that should not be classified as React components.
/// These are Next.js App Router route handler exports.
const HTTP_METHODS: &[&str] = &["GET", "POST", "PUT", "PATCH", "DELETE", "HEAD", "OPTIONS"];

/// Check if a name is an HTTP method (Next.js route handler).
fn is_http_method(name: &str) -> bool {
    HTTP_METHODS.contains(&name)
}

// ============================================================================
// React Component Extraction
// ============================================================================

/// Extract React components (function and class components).
fn extract_react_components(
    file_path: &str,
    text: &str,
    framework: FrameworkHint,
) -> Vec<KgSymbolCandidate> {
    let mut symbols = Vec::new();
    let mut seen = std::collections::HashSet::new();

    // Pattern: function ComponentName(...) { return <JSX /> }
    let fn_component_re = Regex::new(
        r"(?:export\s+)?(?:default\s+)?function\s+([A-Z][a-zA-Z0-9_$]*)\s*\([^)]*\)\s*(?::\s*[^{]+)?\s*\{"
    ).expect("Invalid regex");

    for cap in fn_component_re.captures_iter(text) {
        if let Some(name) = cap.get(1) {
            let name_str = name.as_str();

            // Skip HTTP methods (GET, POST, etc.) - these are route handlers, not React components
            if is_http_method(name_str) {
                continue;
            }

            if seen.contains(name_str) {
                continue;
            }
            seen.insert(name_str.to_string());

            let is_default = cap
                .get(0)
                .map(|m| m.as_str().contains("default"))
                .unwrap_or(false);
            let sym =
                KgSymbolCandidate::new("reactComponent", name_str, LanguageKind::JsTs, file_path)
                    .with_framework(framework)
                    .with_prop("componentType".to_string(), "function".to_string())
                    .with_prop("isDefaultExport".to_string(), is_default.to_string());
            symbols.push(sym);
        }
    }

    // Pattern: const ComponentName = (...) => or const ComponentName = function(...)
    let arrow_component_re = Regex::new(
        r"(?:export\s+)?(?:default\s+)?const\s+([A-Z][a-zA-Z0-9_$]*)\s*(?::\s*[^=]+)?\s*=\s*(?:React\.)?(?:memo|forwardRef)?\s*\(?\s*(?:async\s+)?(?:\([^)]*\)|[a-zA-Z_$][a-zA-Z0-9_$]*)\s*=>"
    ).expect("Invalid regex");

    for cap in arrow_component_re.captures_iter(text) {
        if let Some(name) = cap.get(1) {
            let name_str = name.as_str();

            // Skip HTTP methods (GET, POST, etc.) - these are route handlers, not React components
            if is_http_method(name_str) {
                continue;
            }

            if seen.contains(name_str) {
                continue;
            }
            seen.insert(name_str.to_string());

            let is_default = cap
                .get(0)
                .map(|m| m.as_str().contains("default"))
                .unwrap_or(false);
            let sym =
                KgSymbolCandidate::new("reactComponent", name_str, LanguageKind::JsTs, file_path)
                    .with_framework(framework)
                    .with_prop("componentType".to_string(), "arrow".to_string())
                    .with_prop("isDefaultExport".to_string(), is_default.to_string());
            symbols.push(sym);
        }
    }

    // Pattern: class ComponentName extends React.Component or Component
    let class_component_re = Regex::new(
        r"(?:export\s+)?(?:default\s+)?class\s+([A-Z][a-zA-Z0-9_$]*)\s+extends\s+(?:React\.)?(?:Component|PureComponent)"
    ).expect("Invalid regex");

    for cap in class_component_re.captures_iter(text) {
        if let Some(name) = cap.get(1) {
            let name_str = name.as_str();
            if seen.contains(name_str) {
                continue;
            }
            seen.insert(name_str.to_string());

            let is_default = cap
                .get(0)
                .map(|m| m.as_str().contains("default"))
                .unwrap_or(false);
            let sym =
                KgSymbolCandidate::new("reactComponent", name_str, LanguageKind::JsTs, file_path)
                    .with_framework(framework)
                    .with_prop("componentType".to_string(), "class".to_string())
                    .with_prop("isDefaultExport".to_string(), is_default.to_string());
            symbols.push(sym);
        }
    }

    symbols
}

// ============================================================================
// shadcn/ui Component Extraction
// ============================================================================

/// Extract shadcn/ui component imports.
fn extract_shadcn_components(file_path: &str, text: &str) -> Vec<KgSymbolCandidate> {
    let mut symbols = Vec::new();
    let mut seen = std::collections::HashSet::new();

    // Pattern: import { Button, Card } from "@/components/ui/button"
    let shadcn_import_re =
        Regex::new(r#"import\s*\{\s*([^}]+)\s*\}\s*from\s*["']@/components/ui/([^"']+)["']"#)
            .expect("Invalid regex");

    for cap in shadcn_import_re.captures_iter(text) {
        if let (Some(imports), Some(source)) = (cap.get(1), cap.get(2)) {
            let source_name = source.as_str();

            for import_name in imports.as_str().split(',') {
                let name = import_name.trim();
                // Skip renamed imports for now
                if name.contains(" as ") || name.is_empty() {
                    continue;
                }

                if seen.contains(name) {
                    continue;
                }
                seen.insert(name.to_string());

                let sym =
                    KgSymbolCandidate::new("uiComponent", name, LanguageKind::JsTs, file_path)
                        .with_framework(FrameworkHint::Shadcn)
                        .with_prop("library".to_string(), "shadcn/ui".to_string())
                        .with_prop(
                            "sourceModule".to_string(),
                            format!("@/components/ui/{}", source_name),
                        );
                symbols.push(sym);
            }
        }
    }

    symbols
}

/// Extract shadcn/ui component usage relations.
fn extract_shadcn_usage_relations(file_path: &str, text: &str) -> Vec<KgRelationCandidate> {
    let mut relations = Vec::new();
    let file_node_id = format!("file:{}", file_path);

    // Pattern: import { Button } from "@/components/ui/button"
    let shadcn_import_re =
        Regex::new(r#"import\s*\{\s*([^}]+)\s*\}\s*from\s*["']@/components/ui/([^"']+)["']"#)
            .expect("Invalid regex");

    for cap in shadcn_import_re.captures_iter(text) {
        if let (Some(imports), Some(source)) = (cap.get(1), cap.get(2)) {
            for import_name in imports.as_str().split(',') {
                let name = import_name.trim();
                if name.contains(" as ") || name.is_empty() {
                    continue;
                }

                // Create relation from file to the shadcn component symbol
                let component_id = format!(
                    "sym:js:@/components/ui/{}:uiComponent:{}",
                    source.as_str(),
                    name
                );

                let rel = KgRelationCandidate::new(&file_node_id, &component_id, "usesUiComponent")
                    .with_props(serde_json::json!({
                        "componentName": name,
                        "library": "shadcn/ui"
                    }));
                relations.push(rel);
            }
        }
    }

    relations
}

// ============================================================================
// Angular Extraction
// ============================================================================

/// Extract Angular symbols (@Component, @NgModule, @Injectable).
fn extract_angular_symbols(file_path: &str, text: &str) -> Vec<KgSymbolCandidate> {
    let mut symbols = Vec::new();
    let mut seen = std::collections::HashSet::new();

    // Pattern: @Component({ ... }) export class ComponentName
    let component_re = Regex::new(
        r"@Component\s*\(\s*\{[^}]*\}\s*\)\s*(?:export\s+)?class\s+([A-Z][a-zA-Z0-9_]*)",
    )
    .expect("Invalid regex");

    for cap in component_re.captures_iter(text) {
        if let Some(name) = cap.get(1) {
            let name_str = name.as_str();
            if seen.contains(name_str) {
                continue;
            }
            seen.insert(name_str.to_string());

            // Try to extract selector from @Component metadata
            let selector = extract_angular_selector(text, name_str);

            let mut sym =
                KgSymbolCandidate::new("ngComponent", name_str, LanguageKind::JsTs, file_path)
                    .with_framework(FrameworkHint::Angular);

            if let Some(sel) = selector {
                sym = sym.with_prop("selector".to_string(), sel);
            }
            symbols.push(sym);
        }
    }

    // Pattern: @NgModule({ ... }) export class ModuleName
    let module_re =
        Regex::new(r"@NgModule\s*\(\s*\{[^}]*\}\s*\)\s*(?:export\s+)?class\s+([A-Z][a-zA-Z0-9_]*)")
            .expect("Invalid regex");

    for cap in module_re.captures_iter(text) {
        if let Some(name) = cap.get(1) {
            let name_str = name.as_str();
            if seen.contains(name_str) {
                continue;
            }
            seen.insert(name_str.to_string());

            let sym = KgSymbolCandidate::new("ngModule", name_str, LanguageKind::JsTs, file_path)
                .with_framework(FrameworkHint::Angular);
            symbols.push(sym);
        }
    }

    // Pattern: @Injectable({ ... }) export class ServiceName
    let injectable_re = Regex::new(
        r"@Injectable\s*\(\s*(?:\{[^}]*\})?\s*\)\s*(?:export\s+)?class\s+([A-Z][a-zA-Z0-9_]*)",
    )
    .expect("Invalid regex");

    for cap in injectable_re.captures_iter(text) {
        if let Some(name) = cap.get(1) {
            let name_str = name.as_str();
            if seen.contains(name_str) {
                continue;
            }
            seen.insert(name_str.to_string());

            let sym = KgSymbolCandidate::new("ngService", name_str, LanguageKind::JsTs, file_path)
                .with_framework(FrameworkHint::Angular);
            symbols.push(sym);
        }
    }

    symbols
}

/// Extract Angular component selector from decorator.
fn extract_angular_selector(text: &str, _component_name: &str) -> Option<String> {
    // Simple extraction of selector from @Component metadata
    let selector_re = Regex::new(r#"selector\s*:\s*["']([^"']+)["']"#).ok()?;
    selector_re
        .captures(text)?
        .get(1)
        .map(|m| m.as_str().to_string())
}

/// Extract Angular module membership relations.
fn extract_angular_relations(file_path: &str, text: &str) -> Vec<KgRelationCandidate> {
    let mut relations = Vec::new();
    let file_node_id = format!("file:{}", file_path);

    // Extract declarations array from @NgModule
    let declarations_re =
        Regex::new(r"declarations\s*:\s*\[\s*([^\]]+)\s*\]").expect("Invalid regex");

    // Find the module name first
    let module_re =
        Regex::new(r"@NgModule\s*\([^)]*\)\s*(?:export\s+)?class\s+([A-Z][a-zA-Z0-9_]*)")
            .expect("Invalid regex");

    if let Some(module_cap) = module_re.captures(text) {
        if let Some(module_name) = module_cap.get(1) {
            let module_id = format!("sym:js:{}:ngModule:{}", file_path, module_name.as_str());

            // Find declarations
            if let Some(decl_cap) = declarations_re.captures(text) {
                if let Some(declarations) = decl_cap.get(1) {
                    for decl in declarations.as_str().split(',') {
                        let component_name = decl.trim();
                        if component_name.is_empty() {
                            continue;
                        }

                        // Create belongsToModule relation
                        let component_id =
                            format!("sym:js:{}:ngComponent:{}", file_path, component_name);
                        let rel =
                            KgRelationCandidate::new(&component_id, &module_id, "belongsToModule")
                                .with_props(serde_json::json!({
                                    "moduleName": module_name.as_str()
                                }));
                        relations.push(rel);
                    }
                }
            }
        }
    }

    // Also create a generic file→module relation
    let _ = file_node_id;

    relations
}

/// Extract Angular route definitions from Routes arrays.
fn extract_angular_routes(file_path: &str, text: &str) -> Vec<KgSymbolCandidate> {
    let mut symbols = Vec::new();
    let mut seen = std::collections::HashSet::new();

    // Pattern: { path: '...', component: ... } in Routes array
    let route_re = Regex::new(
        r#"\{\s*path\s*:\s*["']([^"']*)["']\s*,\s*(?:component|loadChildren|redirectTo)"#,
    )
    .expect("Invalid regex");

    for cap in route_re.captures_iter(text) {
        if let Some(path) = cap.get(1) {
            let path_str = path.as_str();
            // Skip empty paths (usually redirects)
            if path_str.is_empty() {
                continue;
            }

            // Normalize path to start with /
            let route_path = if path_str.starts_with('/') {
                path_str.to_string()
            } else {
                format!("/{}", path_str)
            };

            if seen.contains(&route_path) {
                continue;
            }
            seen.insert(route_path.clone());

            let sym = KgSymbolCandidate::new("ngRoute", &route_path, LanguageKind::JsTs, file_path)
                .with_framework(FrameworkHint::Angular);
            symbols.push(sym);
        }
    }

    symbols
}

// ============================================================================
// className/class Extraction for JSX
// ============================================================================

/// Extract className usage from JSX and create usesClass relations.
fn extract_classname_relations(file_path: &str, text: &str) -> Vec<KgRelationCandidate> {
    let mut relations = Vec::new();
    let file_node_id = format!("file:{}", file_path);
    let mut seen_classes = std::collections::HashSet::new();

    // Pattern: className="..." or className='...'
    let classname_static_re =
        Regex::new(r#"className\s*=\s*["']([^"']+)["']"#).expect("Invalid regex");

    for cap in classname_static_re.captures_iter(text) {
        if let Some(classes) = cap.get(1) {
            for class_name in classes.as_str().split_whitespace() {
                let trimmed = class_name.trim();
                if trimmed.is_empty() || seen_classes.contains(trimmed) {
                    continue;
                }
                seen_classes.insert(trimmed.to_string());

                // Create usesClass relation to virtual CSS symbol
                let style_symbol_id = format!("sym:css:*:styleClass:{}", trimmed);
                let rel = KgRelationCandidate::new(&file_node_id, &style_symbol_id, "usesClass")
                    .with_props(serde_json::json!({
                        "className": trimmed,
                        "source": "jsx-className",
                        "unresolved": true
                    }));
                relations.push(rel);
            }
        }
    }

    // Pattern: className={cn("...", ...)} or className={clsx("...", ...)}
    let classname_cn_re =
        Regex::new(r#"className\s*=\s*\{\s*(?:cn|clsx|classNames)\s*\(\s*["']([^"']+)["']"#)
            .expect("Invalid regex");

    for cap in classname_cn_re.captures_iter(text) {
        if let Some(classes) = cap.get(1) {
            for class_name in classes.as_str().split_whitespace() {
                let trimmed = class_name.trim();
                if trimmed.is_empty() || seen_classes.contains(trimmed) {
                    continue;
                }
                seen_classes.insert(trimmed.to_string());

                let style_symbol_id = format!("sym:css:*:styleClass:{}", trimmed);
                let rel = KgRelationCandidate::new(&file_node_id, &style_symbol_id, "usesClass")
                    .with_props(serde_json::json!({
                        "className": trimmed,
                        "source": "jsx-cn",
                        "unresolved": true
                    }));
                relations.push(rel);
            }
        }
    }

    // Pattern: className={`...`} template literals (extract static parts)
    let classname_template_re =
        Regex::new(r#"className\s*=\s*\{\s*`([^`]+)`"#).expect("Invalid regex");

    for cap in classname_template_re.captures_iter(text) {
        if let Some(template) = cap.get(1) {
            // Extract static class names from template literal (ignore ${...} parts)
            let static_parts: String = template
                .as_str()
                .split("${")
                .map(|part| part.split('}').next_back().unwrap_or(""))
                .collect::<Vec<_>>()
                .join(" ");

            for class_name in static_parts.split_whitespace() {
                let trimmed = class_name.trim();
                if trimmed.is_empty() || seen_classes.contains(trimmed) {
                    continue;
                }
                seen_classes.insert(trimmed.to_string());

                let style_symbol_id = format!("sym:css:*:styleClass:{}", trimmed);
                let rel = KgRelationCandidate::new(&file_node_id, &style_symbol_id, "usesClass")
                    .with_props(serde_json::json!({
                        "className": trimmed,
                        "source": "jsx-template",
                        "unresolved": true
                    }));
                relations.push(rel);
            }
        }
    }

    relations
}

// ============================================================================
// Function Call Extraction
// ============================================================================

/// Common built-in functions/methods to skip (too noisy).
const BUILTIN_FUNCTIONS: &[&str] = &[
    // Console
    "log", "error", "warn", "info", "debug", "trace", "dir", "table",
    // Array methods
    "map", "filter", "reduce", "forEach", "find", "findIndex", "some", "every",
    "includes", "indexOf", "push", "pop", "shift", "unshift", "slice", "splice",
    "concat", "join", "reverse", "sort", "flat", "flatMap", "fill", "keys", "values", "entries",
    // Object methods
    "keys", "values", "entries", "assign", "freeze", "seal", "create",
    "hasOwnProperty", "toString", "valueOf",
    // String methods
    "split", "trim", "toLowerCase", "toUpperCase", "replace", "replaceAll",
    "substring", "substr", "slice", "charAt", "charCodeAt", "includes",
    "startsWith", "endsWith", "match", "search", "padStart", "padEnd",
    // Promise/async
    "then", "catch", "finally", "resolve", "reject", "all", "race", "allSettled",
    // Math
    "floor", "ceil", "round", "abs", "min", "max", "random", "pow", "sqrt",
    // JSON
    "parse", "stringify",
    // DOM (skip if in browser context)
    "getElementById", "querySelector", "querySelectorAll", "addEventListener",
    "removeEventListener", "createElement", "appendChild", "removeChild",
    // Common utilities that are too generic
    "get", "set", "has", "delete", "clear", "add", "remove", "call", "apply", "bind",
    // React hooks (handled separately as hook usage, not calls)
    "useState", "useEffect", "useCallback", "useMemo", "useRef", "useContext",
    "useReducer", "useLayoutEffect", "useImperativeHandle", "useDebugValue",
];

/// Extract function call relations from JS/TS code.
fn extract_function_calls(file_path: &str, text: &str) -> Vec<KgRelationCandidate> {
    let mut relations = Vec::new();
    let file_node_id = format!("file:{}", file_path);
    let mut seen_calls = std::collections::HashSet::new();

    // Pattern 1: functionName(...) - direct function calls
    // Matches: myFunction(), doSomething(arg), handleClick()
    let direct_call_re = Regex::new(r"\b([a-zA-Z_$][a-zA-Z0-9_$]*)\s*\(").expect("Invalid regex");

    for cap in direct_call_re.captures_iter(text) {
        if let Some(name) = cap.get(1) {
            let name_str = name.as_str();

            // Skip built-in/common methods
            if BUILTIN_FUNCTIONS.contains(&name_str) {
                continue;
            }

            // Skip keywords and common patterns
            if matches!(
                name_str,
                "if" | "for" | "while" | "switch" | "catch" | "function"
                    | "async" | "await" | "return" | "throw" | "new" | "typeof"
                    | "instanceof" | "class" | "const" | "let" | "var" | "import"
                    | "export" | "from" | "require"
            ) {
                continue;
            }

            // Skip if already seen
            let call_key = name_str.to_string();
            if seen_calls.contains(&call_key) {
                continue;
            }
            seen_calls.insert(call_key);

            // Create unresolved call relation
            let callee_id = format!("sym:js:*:function:{}", name_str);
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
    // Matches: user.save(), api.fetchData(), this.handleSubmit()
    let method_call_re =
        Regex::new(r"\b([a-zA-Z_$][a-zA-Z0-9_$]*)\.([a-zA-Z_$][a-zA-Z0-9_$]*)\s*\(")
            .expect("Invalid regex");

    for cap in method_call_re.captures_iter(text) {
        if let (Some(obj), Some(method)) = (cap.get(1), cap.get(2)) {
            let obj_str = obj.as_str();
            let method_str = method.as_str();

            // Skip built-in methods
            if BUILTIN_FUNCTIONS.contains(&method_str) {
                continue;
            }

            // Skip console, Math, JSON, etc.
            if matches!(
                obj_str,
                "console" | "Math" | "JSON" | "Object" | "Array" | "String"
                    | "Number" | "Boolean" | "Date" | "Promise" | "window" | "document"
            ) {
                continue;
            }

            // Skip if already seen
            let call_key = format!("{}.{}", obj_str, method_str);
            if seen_calls.contains(&call_key) {
                continue;
            }
            seen_calls.insert(call_key);

            // Create unresolved method call relation
            let callee_id = format!("sym:js:*:method:{}#{}", obj_str, method_str);
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
// Standard Function/Class Extraction
// ============================================================================

/// Extract function declarations (excluding React components).
fn extract_functions(
    file_path: &str,
    text: &str,
    framework: FrameworkHint,
) -> Vec<KgSymbolCandidate> {
    let mut symbols = Vec::new();

    // Pattern: function name(...) or async function name(...)
    let fn_re =
        Regex::new(r"(?:export\s+)?(?:async\s+)?function\s+([a-zA-Z_$][a-zA-Z0-9_$]*)\s*\(")
            .expect("Invalid regex");

    for cap in fn_re.captures_iter(text) {
        if let Some(name) = cap.get(1) {
            let name_str = name.as_str();
            // Skip PascalCase names (React components handled separately)
            if is_pascal_case(name_str) {
                continue;
            }
            let sym = KgSymbolCandidate::new("function", name_str, LanguageKind::JsTs, file_path)
                .with_framework(framework);
            symbols.push(sym);
        }
    }

    // Pattern: const name = (...) => or const name = function(...)
    let arrow_re = Regex::new(
        r"(?:export\s+)?const\s+([a-zA-Z_$][a-zA-Z0-9_$]*)\s*=\s*(?:async\s+)?(?:\([^)]*\)|[a-zA-Z_$][a-zA-Z0-9_$]*)\s*=>"
    ).expect("Invalid regex");

    for cap in arrow_re.captures_iter(text) {
        if let Some(name) = cap.get(1) {
            let name_str = name.as_str();
            // Skip if it looks like a React component (PascalCase)
            if !is_pascal_case(name_str) {
                let sym =
                    KgSymbolCandidate::new("function", name_str, LanguageKind::JsTs, file_path)
                        .with_framework(framework);
                symbols.push(sym);
            }
        }
    }

    symbols
}

/// Extract class declarations.
fn extract_classes(
    file_path: &str,
    text: &str,
    framework: FrameworkHint,
) -> Vec<KgSymbolCandidate> {
    let mut symbols = Vec::new();

    // Pattern: class Name or export class Name
    let class_re = Regex::new(r"(?:export\s+)?(?:abstract\s+)?class\s+([a-zA-Z_$][a-zA-Z0-9_$]*)")
        .expect("Invalid regex");

    for cap in class_re.captures_iter(text) {
        if let Some(name) = cap.get(1) {
            let sym = KgSymbolCandidate::new("class", name.as_str(), LanguageKind::JsTs, file_path)
                .with_framework(framework);
            symbols.push(sym);
        }
    }

    symbols
}

/// Extract TypeScript interfaces.
fn extract_interfaces(
    file_path: &str,
    text: &str,
    framework: FrameworkHint,
) -> Vec<KgSymbolCandidate> {
    let mut symbols = Vec::new();

    // Pattern: interface Name or export interface Name
    let iface_re =
        Regex::new(r"(?:export\s+)?interface\s+([a-zA-Z_$][a-zA-Z0-9_$]*)").expect("Invalid regex");

    for cap in iface_re.captures_iter(text) {
        if let Some(name) = cap.get(1) {
            let sym =
                KgSymbolCandidate::new("interface", name.as_str(), LanguageKind::JsTs, file_path)
                    .with_framework(framework);
            symbols.push(sym);
        }
    }

    symbols
}

/// Extract TypeScript type aliases.
fn extract_type_aliases(
    file_path: &str,
    text: &str,
    framework: FrameworkHint,
) -> Vec<KgSymbolCandidate> {
    let mut symbols = Vec::new();

    // Pattern: type Name = ... or export type Name = ...
    let type_re =
        Regex::new(r"(?:export\s+)?type\s+([a-zA-Z_$][a-zA-Z0-9_$]*)\s*=").expect("Invalid regex");

    for cap in type_re.captures_iter(text) {
        if let Some(name) = cap.get(1) {
            let sym = KgSymbolCandidate::new("type", name.as_str(), LanguageKind::JsTs, file_path)
                .with_framework(framework);
            symbols.push(sym);
        }
    }

    symbols
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
function handleClick() {
    console.log("clicked");
}

export function fetchData() {
    return fetch("/api/data");
}

async function processItems(items) {
    return items.map(i => i.id);
}

const calculate = (a, b) => a + b;

export const transform = async (data) => {
    return data;
};
"#;

        let symbols = extract_functions("src/utils.ts", code, FrameworkHint::None);
        let names: Vec<&str> = symbols.iter().map(|s| s.name.as_str()).collect();

        assert!(names.contains(&"handleClick"));
        assert!(names.contains(&"fetchData"));
        assert!(names.contains(&"processItems"));
        assert!(names.contains(&"calculate"));
        assert!(names.contains(&"transform"));
    }

    #[test]
    fn test_extract_classes() {
        let code = r#"
class UserService {
    constructor() {}
}

export class ApiClient {
    async fetch() {}
}

abstract class BaseRepository {
    abstract find(): void;
}
"#;

        let symbols = extract_classes("src/services.ts", code, FrameworkHint::None);
        let names: Vec<&str> = symbols.iter().map(|s| s.name.as_str()).collect();

        assert!(names.contains(&"UserService"));
        assert!(names.contains(&"ApiClient"));
        assert!(names.contains(&"BaseRepository"));
    }

    #[test]
    fn test_extract_interfaces() {
        let code = r#"
interface User {
    id: string;
    name: string;
}

export interface Config {
    apiUrl: string;
}
"#;

        let symbols = extract_interfaces("src/types.ts", code, FrameworkHint::None);
        let names: Vec<&str> = symbols.iter().map(|s| s.name.as_str()).collect();

        assert!(names.contains(&"User"));
        assert!(names.contains(&"Config"));
    }

    #[test]
    fn test_extract_type_aliases() {
        let code = r#"
type UserId = string;

export type Status = 'active' | 'inactive';

type Callback = () => void;
"#;

        let symbols = extract_type_aliases("src/types.ts", code, FrameworkHint::None);
        let names: Vec<&str> = symbols.iter().map(|s| s.name.as_str()).collect();

        assert!(names.contains(&"UserId"));
        assert!(names.contains(&"Status"));
        assert!(names.contains(&"Callback"));
    }

    #[test]
    fn test_detect_nextjs() {
        let extractor = JsTsExtractor::new();

        // Note: detect_framework only looks at text content, not path
        // Path-based detection happens elsewhere
        assert_eq!(
            extractor.detect_framework("src/utils.ts", "import { useRouter } from 'next/router'"),
            FrameworkHint::NextJs
        );
        assert_eq!(
            extractor.detect_framework("src/app.ts", "import next from \"next\""),
            FrameworkHint::NextJs
        );
    }

    #[test]
    fn test_is_pascal_case() {
        assert!(is_pascal_case("Button"));
        assert!(is_pascal_case("UserCard"));
        assert!(!is_pascal_case("button"));
        assert!(!is_pascal_case("handleClick"));
        assert!(!is_pascal_case("CONSTANT"));
    }

    #[test]
    fn test_extract_react_components() {
        let code = r#"
function Button({ onClick }) {
    return <button onClick={onClick}>Click me</button>;
}

export function UserCard({ user }) {
    return (
        <div className="card">
            <h2>{user.name}</h2>
        </div>
    );
}

const HeaderNav = () => {
    return <nav>Navigation</nav>;
};

export const ProfileSection = ({ profile }) => {
    return <section>{profile.bio}</section>;
};

// Regular function (not a component)
function handleClick() {
    console.log("clicked");
}
"#;

        let symbols = extract_react_components("src/components.tsx", code, FrameworkHint::React);
        let names: Vec<&str> = symbols.iter().map(|s| s.name.as_str()).collect();
        let kinds: Vec<&str> = symbols.iter().map(|s| s.kind.as_str()).collect();

        assert!(names.contains(&"Button"));
        assert!(names.contains(&"UserCard"));
        assert!(names.contains(&"HeaderNav"));
        assert!(names.contains(&"ProfileSection"));
        // Should not contain regular functions
        assert!(!names.contains(&"handleClick"));

        // All should be reactComponent kind
        assert!(kinds.iter().all(|k| *k == "reactComponent"));
    }

    #[test]
    fn test_http_methods_not_react_components() {
        // Next.js App Router route handlers should NOT be classified as React components
        let code = r#"
export async function GET(request: Request) {
    return Response.json({ message: "Hello" });
}

export async function POST(request: Request) {
    const body = await request.json();
    return Response.json({ received: body });
}

export const PUT = async (request: Request) => {
    return new Response("Updated");
};

export function DELETE(request: Request) {
    return new Response(null, { status: 204 });
}

export async function PATCH(request: Request) {
    return Response.json({ patched: true });
}

export function HEAD(request: Request) {
    return new Response(null);
}

export function OPTIONS(request: Request) {
    return new Response(null, { status: 200 });
}
"#;

        let symbols =
            extract_react_components("app/api/users/route.ts", code, FrameworkHint::NextJs);
        let names: Vec<&str> = symbols.iter().map(|s| s.name.as_str()).collect();

        // HTTP methods should NOT be classified as React components
        assert!(!names.contains(&"GET"), "GET should not be a React component");
        assert!(
            !names.contains(&"POST"),
            "POST should not be a React component"
        );
        assert!(!names.contains(&"PUT"), "PUT should not be a React component");
        assert!(
            !names.contains(&"DELETE"),
            "DELETE should not be a React component"
        );
        assert!(
            !names.contains(&"PATCH"),
            "PATCH should not be a React component"
        );
        assert!(
            !names.contains(&"HEAD"),
            "HEAD should not be a React component"
        );
        assert!(
            !names.contains(&"OPTIONS"),
            "OPTIONS should not be a React component"
        );

        // The list should be empty (no React components in a route file with only HTTP methods)
        assert!(
            symbols.is_empty(),
            "No React components should be found in a pure route handler file"
        );
    }

    #[test]
    fn test_extract_shadcn_components() {
        let code = r#"
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader } from "@/components/ui/card";
import { Dialog, DialogContent, DialogTrigger } from '@/components/ui/dialog';
import { useToast } from "@/hooks/use-toast";
import React from 'react';
"#;

        let symbols = extract_shadcn_components("src/page.tsx", code);
        let names: Vec<&str> = symbols.iter().map(|s| s.name.as_str()).collect();
        let kinds: Vec<&str> = symbols.iter().map(|s| s.kind.as_str()).collect();

        // Check symbols
        assert!(names.contains(&"Button"));
        assert!(names.contains(&"Card"));
        assert!(names.contains(&"CardContent"));
        assert!(names.contains(&"Dialog"));
        // Should not contain non-shadcn imports
        assert!(!names.contains(&"useToast"));
        assert!(!names.contains(&"React"));

        // All should be uiComponent kind
        assert!(kinds.iter().all(|k| *k == "uiComponent"));
    }

    #[test]
    fn test_extract_shadcn_usage_relations() {
        let code = r#"
import { Button } from "@/components/ui/button";
import { Card } from "@/components/ui/card";
"#;

        let relations = extract_shadcn_usage_relations("src/page.tsx", code);

        // Check relations
        assert!(!relations.is_empty());
        assert!(relations
            .iter()
            .any(|r| r.kind == "usesUiComponent" && r.to_id.contains("Button")));
        assert!(relations
            .iter()
            .any(|r| r.kind == "usesUiComponent" && r.to_id.contains("Card")));
    }

    #[test]
    fn test_extract_angular_symbols() {
        let code = r#"
@Component({
    selector: 'app-header',
    templateUrl: './header.component.html'
})
export class HeaderComponent {
    title = 'My App';
}

@NgModule({
    declarations: [HeaderComponent],
    imports: [CommonModule]
})
export class HeaderModule { }

@Injectable({
    providedIn: 'root'
})
export class DataService {
    getData() {}
}
"#;

        let symbols = extract_angular_symbols("src/header.component.ts", code);
        let names: Vec<&str> = symbols.iter().map(|s| s.name.as_str()).collect();

        assert!(names.contains(&"HeaderComponent"));
        assert!(names.contains(&"HeaderModule"));
        assert!(names.contains(&"DataService"));

        // Check kinds
        let component = symbols
            .iter()
            .find(|s| s.name == "HeaderComponent")
            .unwrap();
        assert_eq!(component.kind, "ngComponent");

        let module = symbols.iter().find(|s| s.name == "HeaderModule").unwrap();
        assert_eq!(module.kind, "ngModule");

        let service = symbols.iter().find(|s| s.name == "DataService").unwrap();
        assert_eq!(service.kind, "ngService");
    }

    #[test]
    fn test_extract_angular_routes() {
        let code = r#"
const routes: Routes = [
    { path: 'home', component: HomeComponent },
    { path: 'users', component: UsersComponent },
    { path: 'users/:id', component: UserDetailComponent },
    { path: '', redirectTo: '/home', pathMatch: 'full' }
];

@NgModule({
    imports: [RouterModule.forRoot(routes)],
    exports: [RouterModule]
})
export class AppRoutingModule { }
"#;

        let symbols = extract_angular_routes("src/app-routing.module.ts", code);
        let names: Vec<&str> = symbols.iter().map(|s| s.name.as_str()).collect();

        assert!(names.contains(&"/home"));
        assert!(names.contains(&"/users"));
        assert!(names.contains(&"/users/:id"));
        // Empty path should be skipped (redirect)
        assert!(!names.contains(&""));

        // Check kinds
        assert!(symbols.iter().all(|s| s.kind == "ngRoute"));
    }

    #[test]
    fn test_extract_classname_relations() {
        let code = r#"
export function Card({ title }) {
    return (
        <div className="bg-white shadow-md rounded-lg p-4">
            <h2 className="text-xl font-bold text-gray-900">{title}</h2>
            <p className="text-sm text-gray-500">Description</p>
        </div>
    );
}

const Header = () => (
    <header className="flex items-center justify-between px-6 py-4">
        <Logo className="w-8 h-8" />
    </header>
);
"#;

        let relations = extract_classname_relations("src/components.tsx", code);

        // Should extract class references (to_id contains the style class)
        assert!(relations.iter().any(|r| r.to_id.contains("bg-white")));
        assert!(relations.iter().any(|r| r.to_id.contains("shadow-md")));
        assert!(relations.iter().any(|r| r.to_id.contains("text-xl")));
        assert!(relations.iter().any(|r| r.to_id.contains("font-bold")));
        assert!(relations.iter().any(|r| r.to_id.contains("flex")));
        assert!(relations.iter().any(|r| r.to_id.contains("w-8")));

        // All should be usesClass kind
        assert!(relations.iter().all(|r| r.kind == "usesClass"));
    }

    #[test]
    fn test_extract_function_calls() {
        let code = r#"
import { fetchUser, createUser } from './api';

function handleSubmit() {
    const user = fetchUser(123);
    validateInput(user.name);
    user.save();
    api.sendNotification(user.email);
    console.log("Done");
}

async function processData() {
    const result = await transformData(input);
    helper.formatOutput(result);
}
"#;

        let relations = extract_function_calls("src/handlers.ts", code);

        // Should find direct function calls
        assert!(relations.iter().any(|r| r.props["callee"] == "fetchUser"));
        assert!(relations.iter().any(|r| r.props["callee"] == "validateInput"));
        assert!(relations.iter().any(|r| r.props["callee"] == "transformData"));

        // Should find method calls
        assert!(relations
            .iter()
            .any(|r| r.props["callee"] == "save" && r.props["receiver"] == "user"));
        assert!(relations.iter().any(|r| r.props["callee"] == "sendNotification"
            && r.props["receiver"] == "api"));
        assert!(relations.iter().any(|r| r.props["callee"] == "formatOutput"
            && r.props["receiver"] == "helper"));

        // Should NOT find built-in calls
        assert!(!relations.iter().any(|r| r.props["callee"] == "log"));
        assert!(!relations.iter().any(|r| r.props["callee"] == "await"));

        // All should be calls kind
        assert!(relations.iter().all(|r| r.kind == "calls"));
    }

    #[test]
    fn test_detect_react_framework() {
        let extractor = JsTsExtractor::new();

        // React by import
        assert_eq!(
            extractor.detect_framework("src/app.tsx", "import React from 'react'"),
            FrameworkHint::React
        );
        assert_eq!(
            extractor.detect_framework("src/app.tsx", "import { useState } from 'react'"),
            FrameworkHint::React
        );

        // shadcn imports
        assert_eq!(
            extractor.detect_framework(
                "src/page.tsx",
                "import { Button } from \"@/components/ui/button\""
            ),
            FrameworkHint::Shadcn
        );

        // Angular
        assert_eq!(
            extractor.detect_framework("src/app.ts", "import { Component } from '@angular/core'"),
            FrameworkHint::Angular
        );
    }
}
