//! Formatting utilities for CLI output.
//!
//! Provides consistent formatting for bytes, time, and text truncation.

use chrono::{DateTime, Utc};

/// Format bytes as a human-readable string (KB, MB, GB).
///
/// # Examples
///
/// ```
/// use gik_cli::ui::format::format_bytes;
///
/// assert_eq!(format_bytes(512), "512 B");
/// assert_eq!(format_bytes(1024), "1.0 KB");
/// assert_eq!(format_bytes(1_500_000), "1.4 MB");
/// ```
pub fn format_bytes(bytes: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = KB * 1024;
    const GB: u64 = MB * 1024;

    if bytes >= GB {
        format!("{:.1} GB", bytes as f64 / GB as f64)
    } else if bytes >= MB {
        format!("{:.1} MB", bytes as f64 / MB as f64)
    } else if bytes >= KB {
        format!("{:.1} KB", bytes as f64 / KB as f64)
    } else {
        format!("{} B", bytes)
    }
}

/// Truncate a string to a maximum length with ellipsis.
///
/// If the string is longer than `max_len`, it is truncated and `...` is appended.
/// The total output length will be exactly `max_len` characters.
///
/// # Examples
///
/// ```
/// use gik_cli::ui::format::truncate_str;
///
/// assert_eq!(truncate_str("hello", 10), "hello");
/// assert_eq!(truncate_str("hello world", 8), "hello...");
/// ```
pub fn truncate_str(s: &str, max_len: usize) -> String {
    if s.len() <= max_len {
        s.to_string()
    } else if max_len <= 3 {
        ".".repeat(max_len)
    } else {
        // Handle Unicode properly by using char indices
        let mut end = 0;
        for (i, (idx, _)) in s.char_indices().enumerate() {
            if i >= max_len - 3 {
                break;
            }
            end = idx;
        }
        // Get the next char boundary
        let truncate_at = s[end..]
            .char_indices()
            .nth(1)
            .map(|(i, _)| end + i)
            .unwrap_or(end);
        format!("{}...", &s[..truncate_at])
    }
}

/// Format a timestamp as relative time (e.g., "2 hours ago", "3d ago").
///
/// # Examples
///
/// - Less than 1 hour: "5 mins ago"
/// - Less than 24 hours: "3h ago"
/// - Less than 7 days: "2d ago"
/// - Older: "2025-01-15"
pub fn format_relative_time(timestamp: DateTime<Utc>) -> String {
    let now = Utc::now();
    let duration = now.signed_duration_since(timestamp);

    if duration.num_seconds() < 0 {
        // Future timestamp
        return timestamp.format("%Y-%m-%d").to_string();
    }

    if duration.num_minutes() < 1 {
        "just now".to_string()
    } else if duration.num_hours() < 1 {
        format!("{} mins ago", duration.num_minutes())
    } else if duration.num_hours() < 24 {
        format!("{}h ago", duration.num_hours())
    } else if duration.num_days() < 7 {
        format!("{}d ago", duration.num_days())
    } else {
        timestamp.format("%Y-%m-%d").to_string()
    }
}

/// Format a number with thousands separators.
///
/// # Examples
///
/// ```
/// use gik_cli::ui::format::format_thousands;
///
/// assert_eq!(format_thousands(1000), "1,000");
/// assert_eq!(format_thousands(1234567), "1,234,567");
/// ```
pub fn format_thousands(n: u64) -> String {
    let s = n.to_string();
    let mut result = String::with_capacity(s.len() + s.len() / 3);
    let chars: Vec<char> = s.chars().collect();
    let len = chars.len();

    for (i, c) in chars.into_iter().enumerate() {
        if i > 0 && (len - i) % 3 == 0 {
            result.push(',');
        }
        result.push(c);
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_bytes() {
        assert_eq!(format_bytes(0), "0 B");
        assert_eq!(format_bytes(512), "512 B");
        assert_eq!(format_bytes(1023), "1023 B");
        assert_eq!(format_bytes(1024), "1.0 KB");
        assert_eq!(format_bytes(1536), "1.5 KB");
        assert_eq!(format_bytes(1_048_576), "1.0 MB");
        assert_eq!(format_bytes(1_572_864), "1.5 MB");
        assert_eq!(format_bytes(1_073_741_824), "1.0 GB");
    }

    #[test]
    fn test_truncate_str() {
        assert_eq!(truncate_str("hello", 10), "hello");
        assert_eq!(truncate_str("hello", 5), "hello");
        assert_eq!(truncate_str("hello world", 8), "hello...");
        assert_eq!(truncate_str("hello world", 5), "he...");
        assert_eq!(truncate_str("hi", 2), "hi");
        assert_eq!(truncate_str("hello", 3), "...");
    }

    #[test]
    fn test_format_thousands() {
        assert_eq!(format_thousands(0), "0");
        assert_eq!(format_thousands(999), "999");
        assert_eq!(format_thousands(1000), "1,000");
        assert_eq!(format_thousands(12345), "12,345");
        assert_eq!(format_thousands(1234567), "1,234,567");
    }
}
