//! Table rendering for CLI output using comfy-table.
//!
//! Provides consistent table formatting for commands that display tabular data.
//!
//! ## Tables Overview
//!
//! | Command | Table Function |
//! |---------|----------------|
//! | `gik bases` | `render_bases_table()` |
//! | `gik status` | `render_bases_table()` (compact) |
//! | `gik stats` | `render_stats_breakdown()` |
//! | `gik log` | `render_timeline_table()` |

use comfy_table::presets::NOTHING;
use comfy_table::{Cell, CellAlignment, ColumnConstraint, Table, Width};

use super::format::{format_bytes, format_relative_time, format_thousands, truncate_str};

/// Memory age bucket for metrics display.
#[derive(Debug, Clone)]
pub struct AgeBucket {
    /// Label for the age range (e.g., "< 1 hour")
    pub label: String,
    /// Number of entries in this bucket
    pub entries: u64,
    /// Total tokens in this bucket
    pub tokens: u64,
}

/// Category count for release preview.
#[derive(Debug, Clone)]
pub struct CategoryCount {
    /// Category name (e.g., "feat", "fix")
    pub category: String,
    /// Number of commits in this category
    pub count: u64,
}

/// Base information for table rendering.
///
/// This is a simplified view of base data for display purposes.
#[derive(Debug, Clone)]
pub struct BaseRow {
    /// Base name
    pub name: String,
    /// Number of documents
    pub documents: u64,
    /// Number of vectors
    pub vectors: u64,
    /// Number of files
    pub files: u64,
    /// Size in bytes
    pub size_bytes: u64,
    /// Health status string
    pub health: String,
    /// Last indexed timestamp (optional)
    pub last_indexed: Option<chrono::DateTime<chrono::Utc>>,
}

/// Timeline entry for log table.
#[derive(Debug, Clone)]
pub struct TimelineRow {
    /// Short revision ID (8 chars)
    pub revision: String,
    /// Timestamp
    pub timestamp: chrono::DateTime<chrono::Utc>,
    /// Commit message
    pub message: Option<String>,
    /// Number of sources affected
    pub sources: u64,
}

/// Render a bases table for `gik bases` or `gik status`.
///
/// When `include_last_indexed` is true, includes the LAST INDEXED column.
///
/// # Example Output
///
/// ```text
/// BASE         DOCS   VECS   FILES   SIZE       HEALTH   LAST INDEXED
/// code           42    840      38   2.3 MB     ok       2 hours ago
/// docs           12    156      10   512 KB     ok       2 hours ago
/// ```
pub fn render_bases_table(bases: &[BaseRow], include_last_indexed: bool) -> String {
    if bases.is_empty() {
        return String::new();
    }

    let mut table = Table::new();
    table.load_preset(NOTHING);

    // Build header
    let mut headers: Vec<Cell> = vec![
        Cell::new("BASE"),
        Cell::new("DOCS").set_alignment(CellAlignment::Right),
        Cell::new("VECS").set_alignment(CellAlignment::Right),
        Cell::new("FILES").set_alignment(CellAlignment::Right),
        Cell::new("SIZE").set_alignment(CellAlignment::Right),
        Cell::new("HEALTH"),
    ];

    if include_last_indexed {
        headers.push(Cell::new("LAST INDEXED"));
    }

    table.set_header(headers);

    // Set column constraints
    let mut constraints = vec![
        ColumnConstraint::LowerBoundary(Width::Fixed(8)),  // BASE
        ColumnConstraint::LowerBoundary(Width::Fixed(6)),  // DOCS
        ColumnConstraint::LowerBoundary(Width::Fixed(6)),  // VECS
        ColumnConstraint::LowerBoundary(Width::Fixed(6)),  // FILES
        ColumnConstraint::LowerBoundary(Width::Fixed(10)), // SIZE
        ColumnConstraint::LowerBoundary(Width::Fixed(8)),  // HEALTH
    ];

    if include_last_indexed {
        constraints.push(ColumnConstraint::LowerBoundary(Width::Fixed(12))); // LAST INDEXED
    }

    table.set_constraints(constraints);

    // Add rows
    for base in bases {
        let name = truncate_str(&base.name, 15);
        let size = format_bytes(base.size_bytes);
        let health = base.health.clone();

        let mut row = vec![
            Cell::new(name),
            Cell::new(base.documents).set_alignment(CellAlignment::Right),
            Cell::new(base.vectors).set_alignment(CellAlignment::Right),
            Cell::new(base.files).set_alignment(CellAlignment::Right),
            Cell::new(size).set_alignment(CellAlignment::Right),
            Cell::new(health),
        ];

        if include_last_indexed {
            let indexed = base
                .last_indexed
                .map(format_relative_time)
                .unwrap_or_else(|| "-".to_string());
            row.push(Cell::new(indexed));
        }

        table.add_row(row);
    }

    table.trim_fmt().to_string()
}

/// Render a per-base breakdown table for `gik stats`.
///
/// Includes a computed "% OF TOTAL" column.
///
/// # Example Output
///
/// ```text
/// BASE      DOCS   VECS   SIZE       % OF TOTAL
/// code        42    840   2.3 MB     79%
/// docs        12    156   512 KB     17%
/// ```
pub fn render_stats_breakdown(bases: &[BaseRow], total_bytes: u64) -> String {
    if bases.is_empty() {
        return String::new();
    }

    let mut table = Table::new();
    table.load_preset(NOTHING);

    table.set_header(vec![
        Cell::new("BASE"),
        Cell::new("DOCS").set_alignment(CellAlignment::Right),
        Cell::new("VECS").set_alignment(CellAlignment::Right),
        Cell::new("SIZE").set_alignment(CellAlignment::Right),
        Cell::new("% OF TOTAL").set_alignment(CellAlignment::Right),
    ]);

    table.set_constraints(vec![
        ColumnConstraint::LowerBoundary(Width::Fixed(8)),  // BASE
        ColumnConstraint::LowerBoundary(Width::Fixed(6)),  // DOCS
        ColumnConstraint::LowerBoundary(Width::Fixed(6)),  // VECS
        ColumnConstraint::LowerBoundary(Width::Fixed(10)), // SIZE
        ColumnConstraint::LowerBoundary(Width::Fixed(10)), // % OF TOTAL
    ]);

    for base in bases {
        let name = truncate_str(&base.name, 12);
        let size = format_bytes(base.size_bytes);
        let percent = if total_bytes > 0 {
            (base.size_bytes as f64 / total_bytes as f64 * 100.0).round() as u64
        } else {
            0
        };

        table.add_row(vec![
            Cell::new(name),
            Cell::new(base.documents).set_alignment(CellAlignment::Right),
            Cell::new(base.vectors).set_alignment(CellAlignment::Right),
            Cell::new(size).set_alignment(CellAlignment::Right),
            Cell::new(format!("{}%", percent)).set_alignment(CellAlignment::Right),
        ]);
    }

    table.trim_fmt().to_string()
}

/// Render a timeline table for `gik log`.
///
/// # Example Output
///
/// ```text
/// REV        DATE       MESSAGE                              SOURCES
/// abc12345   2h ago     feat: add authentication                  14
/// def67890   1d ago     docs: update API reference                 3
/// ```
pub fn render_timeline_table(entries: &[TimelineRow]) -> String {
    if entries.is_empty() {
        return String::new();
    }

    let mut table = Table::new();
    table.load_preset(NOTHING);

    table.set_header(vec![
        Cell::new("REV"),
        Cell::new("DATE"),
        Cell::new("MESSAGE"),
        Cell::new("SOURCES").set_alignment(CellAlignment::Right),
    ]);

    table.set_constraints(vec![
        ColumnConstraint::LowerBoundary(Width::Fixed(8)),  // REV
        ColumnConstraint::LowerBoundary(Width::Fixed(10)), // DATE
        ColumnConstraint::LowerBoundary(Width::Fixed(30)), // MESSAGE
        ColumnConstraint::LowerBoundary(Width::Fixed(7)),  // SOURCES
    ]);

    for entry in entries {
        let rev = if entry.revision.len() > 8 {
            &entry.revision[..8]
        } else {
            &entry.revision
        };
        let date = format_relative_time(entry.timestamp);
        let message = entry
            .message
            .as_ref()
            .map(|m| truncate_str(m, 40))
            .unwrap_or_else(|| "-".to_string());

        table.add_row(vec![
            Cell::new(rev),
            Cell::new(date),
            Cell::new(message),
            Cell::new(entry.sources).set_alignment(CellAlignment::Right),
        ]);
    }

    table.trim_fmt().to_string()
}

/// Render a memory age distribution table.
///
/// # Example Output
///
/// ```text
/// AGE RANGE        ENTRIES    TOKENS
/// < 1 hour              42    14,820
/// 1-24 hours            56    19,740
/// ```
pub fn render_memory_age_table(buckets: &[AgeBucket]) -> String {
    if buckets.is_empty() {
        return String::new();
    }

    let mut table = Table::new();
    table.load_preset(NOTHING);

    table.set_header(vec![
        Cell::new("AGE RANGE"),
        Cell::new("ENTRIES").set_alignment(CellAlignment::Right),
        Cell::new("TOKENS").set_alignment(CellAlignment::Right),
    ]);

    table.set_constraints(vec![
        ColumnConstraint::LowerBoundary(Width::Fixed(14)), // AGE RANGE
        ColumnConstraint::LowerBoundary(Width::Fixed(8)),  // ENTRIES
        ColumnConstraint::LowerBoundary(Width::Fixed(10)), // TOKENS
    ]);

    for bucket in buckets {
        table.add_row(vec![
            Cell::new(&bucket.label),
            Cell::new(bucket.entries).set_alignment(CellAlignment::Right),
            Cell::new(format_thousands(bucket.tokens)).set_alignment(CellAlignment::Right),
        ]);
    }

    table.trim_fmt().to_string()
}

/// Render a release category preview table.
///
/// # Example Output
///
/// ```text
/// CATEGORY   ENTRIES
/// feat             8
/// fix              5
/// ```
pub fn render_release_preview(categories: &[CategoryCount]) -> String {
    if categories.is_empty() {
        return String::new();
    }

    let mut table = Table::new();
    table.load_preset(NOTHING);

    table.set_header(vec![
        Cell::new("CATEGORY"),
        Cell::new("ENTRIES").set_alignment(CellAlignment::Right),
    ]);

    table.set_constraints(vec![
        ColumnConstraint::LowerBoundary(Width::Fixed(10)), // CATEGORY
        ColumnConstraint::LowerBoundary(Width::Fixed(8)),  // ENTRIES
    ]);

    for cat in categories {
        table.add_row(vec![
            Cell::new(&cat.category),
            Cell::new(cat.count).set_alignment(CellAlignment::Right),
        ]);
    }

    table.trim_fmt().to_string()
}

/// Render a simple key-value metrics table.
///
/// # Example Output
///
/// ```text
/// METRIC              VALUE
/// Total documents        62
/// Total vectors       1,060
/// ```
pub fn render_metrics_table(metrics: &[(&str, String)]) -> String {
    if metrics.is_empty() {
        return String::new();
    }

    let mut table = Table::new();
    table.load_preset(NOTHING);

    table.set_header(vec![
        Cell::new("METRIC"),
        Cell::new("VALUE").set_alignment(CellAlignment::Right),
    ]);

    table.set_constraints(vec![
        ColumnConstraint::LowerBoundary(Width::Fixed(18)), // METRIC
        ColumnConstraint::LowerBoundary(Width::Fixed(12)), // VALUE
    ]);

    for (key, value) in metrics {
        table.add_row(vec![
            Cell::new(*key),
            Cell::new(value).set_alignment(CellAlignment::Right),
        ]);
    }

    table.trim_fmt().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    fn sample_bases() -> Vec<BaseRow> {
        vec![
            BaseRow {
                name: "code".to_string(),
                documents: 42,
                vectors: 840,
                files: 38,
                size_bytes: 2_411_724,
                health: "ok".to_string(),
                last_indexed: Some(Utc::now()),
            },
            BaseRow {
                name: "docs".to_string(),
                documents: 12,
                vectors: 156,
                files: 10,
                size_bytes: 524_288,
                health: "ok".to_string(),
                last_indexed: Some(Utc::now()),
            },
        ]
    }

    #[test]
    fn test_bases_table_structure() {
        let output = render_bases_table(&sample_bases(), false);

        // Verify headers
        assert!(output.contains("BASE"));
        assert!(output.contains("DOCS"));
        assert!(output.contains("VECS"));
        assert!(output.contains("FILES"));
        assert!(output.contains("SIZE"));
        assert!(output.contains("HEALTH"));

        // Verify LAST INDEXED is excluded
        assert!(!output.contains("LAST INDEXED"));

        // Verify data
        assert!(output.contains("code"));
        assert!(output.contains("42"));
        assert!(output.contains("ok"));
    }

    #[test]
    fn test_bases_table_with_last_indexed() {
        let output = render_bases_table(&sample_bases(), true);
        assert!(output.contains("LAST INDEXED"));
        assert!(output.contains("just now"));
    }

    #[test]
    fn test_stats_breakdown() {
        let bases = sample_bases();
        let total = bases.iter().map(|b| b.size_bytes).sum();
        let output = render_stats_breakdown(&bases, total);

        assert!(output.contains("% OF TOTAL"));
        assert!(output.contains("code"));
        assert!(output.contains("docs"));
    }

    #[test]
    fn test_timeline_table() {
        let entries = vec![
            TimelineRow {
                revision: "abc12345def67890".to_string(),
                timestamp: Utc::now(),
                message: Some("feat: add authentication".to_string()),
                sources: 14,
            },
            TimelineRow {
                revision: "def67890abc12345".to_string(),
                timestamp: Utc::now(),
                message: Some("docs: update readme".to_string()),
                sources: 3,
            },
        ];

        let output = render_timeline_table(&entries);

        assert!(output.contains("REV"));
        assert!(output.contains("DATE"));
        assert!(output.contains("MESSAGE"));
        assert!(output.contains("SOURCES"));
        assert!(output.contains("abc12345")); // Truncated to 8 chars
        assert!(output.contains("feat: add authentication"));
    }

    #[test]
    fn test_memory_age_table() {
        let buckets = vec![
            AgeBucket {
                label: "< 1 hour".to_string(),
                entries: 42,
                tokens: 14820,
            },
            AgeBucket {
                label: "1-24 hours".to_string(),
                entries: 56,
                tokens: 19740,
            },
        ];

        let output = render_memory_age_table(&buckets);

        assert!(output.contains("AGE RANGE"));
        assert!(output.contains("ENTRIES"));
        assert!(output.contains("TOKENS"));
        assert!(output.contains("< 1 hour"));
        assert!(output.contains("14,820")); // Thousands separator
    }

    #[test]
    fn test_release_preview() {
        let categories = vec![
            CategoryCount {
                category: "feat".to_string(),
                count: 8,
            },
            CategoryCount {
                category: "fix".to_string(),
                count: 5,
            },
        ];

        let output = render_release_preview(&categories);

        assert!(output.contains("CATEGORY"));
        assert!(output.contains("ENTRIES"));
        assert!(output.contains("feat"));
        assert!(output.contains("8"));
    }

    #[test]
    fn test_empty_tables() {
        assert_eq!(render_bases_table(&[], false), "");
        assert_eq!(render_stats_breakdown(&[], 0), "");
        assert_eq!(render_timeline_table(&[]), "");
        assert_eq!(render_memory_age_table(&[]), "");
        assert_eq!(render_release_preview(&[]), "");
    }
}
