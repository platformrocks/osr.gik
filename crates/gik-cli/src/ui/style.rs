//! Message styling for CLI output.
//!
//! Provides consistent prefixes, colors, and formatting for all CLI messages.
//!
//! ## Message Types
//!
//! | Prefix | Meaning | Color |
//! |--------|---------|-------|
//! | `[ok]` | Success | Green |
//! | `[err]` | Error | Red |
//! | `[warn]` | Warning | Yellow |
//! | `[info]` | Information | Blue |
//! | `[hint]` | Suggestion | Cyan |
//! | `[skip]` | Skipped | Dim |

use owo_colors::OwoColorize;

use super::color::ColorMode;

/// Message severity/type for CLI output.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MessageType {
    /// Success - operation completed successfully
    Ok,
    /// Error - operation failed, cannot continue
    Err,
    /// Warning - operation succeeded with caveats
    Warn,
    /// Information - neutral status or progress update
    Info,
    /// Hint - actionable next step or tip
    Hint,
    /// Skipped - item was intentionally not processed
    Skip,
}

impl MessageType {
    /// Returns the prefix text for this message type.
    pub fn prefix(&self) -> &'static str {
        match self {
            Self::Ok => "[ok]",
            Self::Err => "[err]",
            Self::Warn => "[warn]",
            Self::Info => "[info]",
            Self::Hint => "[hint]",
            Self::Skip => "[skip]",
        }
    }
}

/// Main styling interface for CLI output.
///
/// The Style struct provides methods to format messages, sections, and other
/// CLI output elements with consistent styling.
///
/// # Example
///
/// ```
/// use gik_cli::ui::{Style, MessageType, ColorMode};
///
/// let style = Style::new(ColorMode::Never);
/// println!("{}", style.message(MessageType::Ok, "Operation completed"));
/// ```
#[derive(Debug, Clone)]
pub struct Style {
    color_mode: ColorMode,
}

impl Default for Style {
    fn default() -> Self {
        Self::from_env()
    }
}

impl Style {
    /// Create a Style instance by detecting environment settings.
    ///
    /// This checks for `NO_COLOR` env var and TTY status.
    pub fn from_env() -> Self {
        Self {
            color_mode: ColorMode::detect(),
        }
    }

    /// Create a Style instance with an explicit color mode.
    ///
    /// Useful for testing or when the CLI provides a `--color` flag.
    pub fn new(color_mode: ColorMode) -> Self {
        Self { color_mode }
    }

    /// Check if colors are enabled.
    pub fn colors_enabled(&self) -> bool {
        self.color_mode.is_enabled()
    }

    /// Get the current color mode.
    pub fn color_mode(&self) -> ColorMode {
        self.color_mode
    }

    /// Format a simple message with a type prefix.
    ///
    /// # Example
    ///
    /// ```
    /// use gik_cli::ui::{Style, MessageType, ColorMode};
    ///
    /// let style = Style::new(ColorMode::Never);
    /// assert_eq!(
    ///     style.message(MessageType::Ok, "Done"),
    ///     "[ok] Done"
    /// );
    /// ```
    pub fn message(&self, msg_type: MessageType, text: &str) -> String {
        let prefix = msg_type.prefix();
        if self.colors_enabled() {
            let colored_prefix = match msg_type {
                MessageType::Ok => prefix.green().to_string(),
                MessageType::Err => prefix.red().to_string(),
                MessageType::Warn => prefix.yellow().to_string(),
                MessageType::Info => prefix.blue().to_string(),
                MessageType::Hint => prefix.cyan().to_string(),
                MessageType::Skip => prefix.dimmed().to_string(),
            };
            format!("{} {}", colored_prefix, text)
        } else {
            format!("{} {}", prefix, text)
        }
    }

    /// Format a detail line with 5-space indentation.
    ///
    /// Use this for multi-line messages where details follow the main message.
    ///
    /// # Example
    ///
    /// ```
    /// use gik_cli::ui::{Style, ColorMode};
    ///
    /// let style = Style::new(ColorMode::Never);
    /// assert_eq!(
    ///     style.message_detail("Indexed", "14 sources"),
    ///     "     Indexed: 14 sources"
    /// );
    /// ```
    pub fn message_detail(&self, label: &str, value: &str) -> String {
        format!("     {}: {}", label, value)
    }

    /// Format a section header.
    ///
    /// Creates a simple header like: `STATUS`
    ///
    /// # Example
    ///
    /// ```
    /// use gik_cli::ui::{Style, ColorMode};
    ///
    /// let style = Style::new(ColorMode::Never);
    /// let header = style.section("STATUS");
    /// assert_eq!(header, "STATUS");
    /// ```
    pub fn section(&self, title: &str) -> String {
        if self.colors_enabled() {
            title.bold().to_string()
        } else {
            title.to_string()
        }
    }

    /// Format a structured error with optional cause and hint.
    ///
    /// # Example
    ///
    /// ```
    /// use gik_cli::ui::{Style, ColorMode};
    ///
    /// let style = Style::new(ColorMode::Never);
    /// let output = style.error_with_context(
    ///     "Failed to connect",
    ///     Some("Connection refused"),
    ///     Some("Check if the server is running"),
    /// );
    /// assert!(output.contains("[err] Failed to connect"));
    /// assert!(output.contains("Cause: Connection refused"));
    /// assert!(output.contains("Hint: Check if the server is running"));
    /// ```
    pub fn error_with_context(
        &self,
        msg: &str,
        cause: Option<&str>,
        hint: Option<&str>,
    ) -> String {
        let mut output = self.message(MessageType::Err, msg);

        if let Some(cause_text) = cause {
            output.push('\n');
            output.push_str(&format!("      Cause: {}", cause_text));
        }

        if let Some(hint_text) = hint {
            output.push('\n');
            output.push_str(&format!("      Hint: {}", hint_text));
        }

        output
    }

    /// Format a list item with a prefix marker.
    ///
    /// The prefix `+` is colored green, `-` is colored red.
    ///
    /// # Example
    ///
    /// ```
    /// use gik_cli::ui::{Style, ColorMode};
    ///
    /// let style = Style::new(ColorMode::Never);
    /// assert_eq!(style.list_item("+", "src/lib.rs"), "  + src/lib.rs");
    /// assert_eq!(style.list_item("-", "temp.txt"), "  - temp.txt");
    /// ```
    pub fn list_item(&self, prefix: &str, text: &str) -> String {
        let styled_prefix = if self.colors_enabled() {
            match prefix {
                "+" => prefix.green().to_string(),
                "-" => prefix.red().to_string(),
                _ => prefix.to_string(),
            }
        } else {
            prefix.to_string()
        };
        format!("  {} {}", styled_prefix, text)
    }

    /// Format a key-value pair with optional coloring.
    ///
    /// # Example
    ///
    /// ```
    /// use gik_cli::ui::{Style, ColorMode};
    ///
    /// let style = Style::new(ColorMode::Never);
    /// assert_eq!(style.key_value("Branch", "main"), "Branch: main");
    /// ```
    pub fn key_value(&self, key: &str, value: &str) -> String {
        if self.colors_enabled() {
            format!("{}: {}", key.dimmed(), value)
        } else {
            format!("{}: {}", key, value)
        }
    }

    /// Format a revision/hash (first 8 chars, colored yellow).
    pub fn revision(&self, rev: &str) -> String {
        let short = if rev.len() > 8 { &rev[..8] } else { rev };
        if self.colors_enabled() {
            short.yellow().to_string()
        } else {
            short.to_string()
        }
    }

    /// Format a file path (colored cyan).
    pub fn file_path(&self, path: &str) -> String {
        if self.colors_enabled() {
            path.cyan().to_string()
        } else {
            path.to_string()
        }
    }

    /// Format a score value with color based on magnitude.
    ///
    /// - >= 0.8: green
    /// - >= 0.5: yellow
    /// - < 0.5: red
    pub fn score(&self, value: f32) -> String {
        let formatted = format!("{:.2}", value);
        if self.colors_enabled() {
            if value >= 0.8 {
                formatted.green().to_string()
            } else if value >= 0.5 {
                formatted.yellow().to_string()
            } else {
                formatted.red().to_string()
            }
        } else {
            formatted
        }
    }

    /// Format a staged file indicator (git-like status).
    ///
    /// Used for "new file:" (green) or "modified:" (green) in staging area.
    ///
    /// # Example
    ///
    /// ```
    /// use gik_cli::ui::{Style, ColorMode};
    ///
    /// let style = Style::new(ColorMode::Never);
    /// assert_eq!(style.staged_new("src/lib.rs"), "        new file:   src/lib.rs");
    /// assert_eq!(style.staged_modified("src/main.rs"), "        modified:   src/main.rs");
    /// ```
    pub fn staged_new(&self, path: &str) -> String {
        if self.colors_enabled() {
            format!("        {}   {}", "new file:".green(), path.green())
        } else {
            format!("        new file:   {}", path)
        }
    }

    /// Format a modified staged file (git-like status).
    pub fn staged_modified(&self, path: &str) -> String {
        if self.colors_enabled() {
            format!("        {}   {}", "modified:".green(), path.green())
        } else {
            format!("        modified:   {}", path)
        }
    }

    /// Format an unstaged modified file indicator (git-like status).
    ///
    /// Used for files modified in working tree but not staged.
    pub fn unstaged_modified(&self, path: &str) -> String {
        if self.colors_enabled() {
            format!("        {}   {}", "modified:".red(), path.red())
        } else {
            format!("        modified:   {}", path)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_message_type_prefix() {
        assert_eq!(MessageType::Ok.prefix(), "[ok]");
        assert_eq!(MessageType::Err.prefix(), "[err]");
        assert_eq!(MessageType::Warn.prefix(), "[warn]");
        assert_eq!(MessageType::Info.prefix(), "[info]");
        assert_eq!(MessageType::Hint.prefix(), "[hint]");
        assert_eq!(MessageType::Skip.prefix(), "[skip]");
    }

    #[test]
    fn test_message_no_color() {
        let style = Style::new(ColorMode::Never);
        assert_eq!(style.message(MessageType::Ok, "Success"), "[ok] Success");
        assert_eq!(style.message(MessageType::Err, "Failed"), "[err] Failed");
    }

    #[test]
    fn test_message_detail() {
        let style = Style::new(ColorMode::Never);
        assert_eq!(
            style.message_detail("Count", "42"),
            "     Count: 42"
        );
    }

    #[test]
    fn test_section_header() {
        let style = Style::new(ColorMode::Never);
        let header = style.section("STATUS");
        assert_eq!(header, "STATUS");
    }

    #[test]
    fn test_error_with_context() {
        let style = Style::new(ColorMode::Never);
        let output = style.error_with_context(
            "Connection failed",
            Some("Timeout"),
            Some("Check network"),
        );
        assert!(output.contains("[err] Connection failed"));
        assert!(output.contains("Cause: Timeout"));
        assert!(output.contains("Hint: Check network"));
    }

    #[test]
    fn test_list_item() {
        let style = Style::new(ColorMode::Never);
        assert_eq!(style.list_item("+", "file.rs"), "  + file.rs");
        assert_eq!(style.list_item("-", "old.txt"), "  - old.txt");
    }

    #[test]
    fn test_revision() {
        let style = Style::new(ColorMode::Never);
        assert_eq!(style.revision("abc12345def67890"), "abc12345");
        assert_eq!(style.revision("short"), "short");
    }
}
