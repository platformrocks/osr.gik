//! SQL language extractor.
//!
//! Extracts symbols from SQL files:
//! - Tables (CREATE TABLE)
//! - Views (CREATE VIEW)
//! - Functions/Procedures
//! - Indexes

use regex::Regex;

use super::{
    FrameworkHint, KgRelationCandidate, KgSymbolCandidate, LanguageExtractor, LanguageKind,
};

/// SQL extractor.
#[derive(Debug, Clone, Default)]
pub struct SqlExtractor;

impl SqlExtractor {
    /// Create a new SQL extractor.
    pub fn new() -> Self {
        Self
    }
}

impl LanguageExtractor for SqlExtractor {
    fn language(&self) -> LanguageKind {
        LanguageKind::Sql
    }

    fn detect_framework(&self, _file_path: &str, _text: &str) -> FrameworkHint {
        FrameworkHint::None
    }

    fn extract_symbols(&self, file_path: &str, text: &str) -> Vec<KgSymbolCandidate> {
        let mut symbols = Vec::new();
        let framework = self.detect_framework(file_path, text);

        symbols.extend(extract_tables(file_path, text, framework));
        symbols.extend(extract_views(file_path, text, framework));
        symbols.extend(extract_functions(file_path, text, framework));
        symbols.extend(extract_indexes(file_path, text, framework));

        symbols
    }

    fn extract_relations(&self, file_path: &str, _text: &str) -> Vec<KgRelationCandidate> {
        let _ = file_path;
        Vec::new()
    }
}

/// Extract table definitions.
fn extract_tables(file_path: &str, text: &str, framework: FrameworkHint) -> Vec<KgSymbolCandidate> {
    let mut symbols = Vec::new();

    // Pattern: CREATE TABLE name or CREATE TABLE IF NOT EXISTS name
    let table_re = Regex::new(
        r"(?i)CREATE\s+TABLE\s+(?:IF\s+NOT\s+EXISTS\s+)?(?:[`]?[a-zA-Z_][a-zA-Z0-9_]*[`]?\.)?[`]?([a-zA-Z_][a-zA-Z0-9_]*)[`]?",
    )
    .expect("Invalid regex");

    for cap in table_re.captures_iter(text) {
        if let Some(name) = cap.get(1) {
            let sym = KgSymbolCandidate::new("table", name.as_str(), LanguageKind::Sql, file_path)
                .with_framework(framework);
            symbols.push(sym);
        }
    }

    symbols
}

/// Extract view definitions.
fn extract_views(file_path: &str, text: &str, framework: FrameworkHint) -> Vec<KgSymbolCandidate> {
    let mut symbols = Vec::new();

    let view_re = Regex::new(
        r"(?i)CREATE\s+(?:OR\s+REPLACE\s+)?VIEW\s+(?:[`]?[a-zA-Z_][a-zA-Z0-9_]*[`]?\.)?[`]?([a-zA-Z_][a-zA-Z0-9_]*)[`]?",
    )
    .expect("Invalid regex");

    for cap in view_re.captures_iter(text) {
        if let Some(name) = cap.get(1) {
            let sym = KgSymbolCandidate::new("view", name.as_str(), LanguageKind::Sql, file_path)
                .with_framework(framework);
            symbols.push(sym);
        }
    }

    symbols
}

/// Extract function/procedure definitions.
fn extract_functions(
    file_path: &str,
    text: &str,
    framework: FrameworkHint,
) -> Vec<KgSymbolCandidate> {
    let mut symbols = Vec::new();

    // Functions
    let fn_re = Regex::new(
        r"(?i)CREATE\s+(?:OR\s+REPLACE\s+)?FUNCTION\s+(?:[`]?[a-zA-Z_][a-zA-Z0-9_]*[`]?\.)?[`]?([a-zA-Z_][a-zA-Z0-9_]*)[`]?",
    )
    .expect("Invalid regex");

    for cap in fn_re.captures_iter(text) {
        if let Some(name) = cap.get(1) {
            let sym =
                KgSymbolCandidate::new("function", name.as_str(), LanguageKind::Sql, file_path)
                    .with_framework(framework);
            symbols.push(sym);
        }
    }

    // Procedures
    let proc_re = Regex::new(
        r"(?i)CREATE\s+(?:OR\s+REPLACE\s+)?PROCEDURE\s+(?:[`]?[a-zA-Z_][a-zA-Z0-9_]*[`]?\.)?[`]?([a-zA-Z_][a-zA-Z0-9_]*)[`]?",
    )
    .expect("Invalid regex");

    for cap in proc_re.captures_iter(text) {
        if let Some(name) = cap.get(1) {
            let sym =
                KgSymbolCandidate::new("procedure", name.as_str(), LanguageKind::Sql, file_path)
                    .with_framework(framework);
            symbols.push(sym);
        }
    }

    symbols
}

/// Extract index definitions.
fn extract_indexes(
    file_path: &str,
    text: &str,
    framework: FrameworkHint,
) -> Vec<KgSymbolCandidate> {
    let mut symbols = Vec::new();

    let idx_re = Regex::new(
        r"(?i)CREATE\s+(?:UNIQUE\s+)?INDEX\s+(?:IF\s+NOT\s+EXISTS\s+)?[`]?([a-zA-Z_][a-zA-Z0-9_]*)[`]?",
    )
    .expect("Invalid regex");

    for cap in idx_re.captures_iter(text) {
        if let Some(name) = cap.get(1) {
            let sym = KgSymbolCandidate::new("index", name.as_str(), LanguageKind::Sql, file_path)
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
    fn test_extract_tables() {
        let code = r#"
CREATE TABLE users (
    id INTEGER PRIMARY KEY,
    name TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS posts (
    id INTEGER PRIMARY KEY,
    user_id INTEGER REFERENCES users(id)
);
"#;
        let symbols = extract_tables("migrations/001_init.sql", code, FrameworkHint::None);
        let names: Vec<&str> = symbols.iter().map(|s| s.name.as_str()).collect();

        assert!(names.contains(&"users"));
        assert!(names.contains(&"posts"));
    }

    #[test]
    fn test_extract_views() {
        let code = r#"
CREATE VIEW active_users AS
SELECT * FROM users WHERE active = true;

CREATE OR REPLACE VIEW user_stats AS
SELECT user_id, COUNT(*) as post_count FROM posts GROUP BY user_id;
"#;
        let symbols = extract_views("migrations/002_views.sql", code, FrameworkHint::None);
        let names: Vec<&str> = symbols.iter().map(|s| s.name.as_str()).collect();

        assert!(names.contains(&"active_users"));
        assert!(names.contains(&"user_stats"));
    }

    #[test]
    fn test_extract_functions() {
        let code = r#"
CREATE FUNCTION get_user_count() RETURNS INTEGER AS $$
    SELECT COUNT(*) FROM users;
$$ LANGUAGE SQL;

CREATE OR REPLACE PROCEDURE update_stats() AS $$
BEGIN
    -- update logic
END;
$$ LANGUAGE plpgsql;
"#;
        let symbols = extract_functions("migrations/003_functions.sql", code, FrameworkHint::None);
        let names: Vec<&str> = symbols.iter().map(|s| s.name.as_str()).collect();

        assert!(names.contains(&"get_user_count"));
        assert!(names.contains(&"update_stats"));
    }

    #[test]
    fn test_extract_indexes() {
        let code = r#"
CREATE INDEX idx_users_name ON users(name);

CREATE UNIQUE INDEX idx_users_email ON users(email);

CREATE INDEX IF NOT EXISTS idx_posts_user ON posts(user_id);
"#;
        let symbols = extract_indexes("migrations/004_indexes.sql", code, FrameworkHint::None);
        let names: Vec<&str> = symbols.iter().map(|s| s.name.as_str()).collect();

        assert!(names.contains(&"idx_users_name"));
        assert!(names.contains(&"idx_users_email"));
        assert!(names.contains(&"idx_posts_user"));
    }
}
