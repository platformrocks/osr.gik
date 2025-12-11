//! Color mode detection for CLI output.
//!
//! Respects the `NO_COLOR` environment variable and TTY detection.
//! See https://no-color.org/ for the NO_COLOR standard.

use std::io::IsTerminal;

/// Color output mode for the CLI.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ColorMode {
    /// Always use colors, even when output is not a TTY.
    Always,
    /// Never use colors.
    Never,
    /// Automatically detect based on TTY and NO_COLOR env var.
    #[default]
    Auto,
}

impl ColorMode {
    /// Detect color mode from environment.
    ///
    /// Checks for explicit mode override, then falls back to Auto.
    pub fn detect() -> Self {
        Self::Auto
    }

    /// Create ColorMode from a CLI flag value.
    ///
    /// Accepts: "always", "never", "auto"
    pub fn from_str(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "always" => Some(Self::Always),
            "never" => Some(Self::Never),
            "auto" => Some(Self::Auto),
            _ => None,
        }
    }

    /// Check if colors should be used based on current mode.
    pub fn is_enabled(&self) -> bool {
        match self {
            Self::Always => true,
            Self::Never => false,
            Self::Auto => Self::should_auto_colorize(),
        }
    }

    /// Determine if colors should be used in auto mode.
    ///
    /// Rules:
    /// 1. If `NO_COLOR` env var is set (any value), disable colors
    /// 2. If stdout is not a TTY, disable colors
    /// 3. Otherwise, enable colors
    fn should_auto_colorize() -> bool {
        // Rule 1: NO_COLOR env var disables colors (per https://no-color.org/)
        if std::env::var("NO_COLOR").is_ok() {
            return false;
        }

        // Rule 2: Check if stdout is a TTY
        std::io::stdout().is_terminal()
    }
}

/// Get the current terminal width, or a sensible default.
///
/// Returns 80 if the terminal width cannot be determined.
pub fn terminal_width() -> usize {
    terminal_size::terminal_size()
        .map(|(w, _)| w.0 as usize)
        .unwrap_or(80)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_color_mode_from_str() {
        assert_eq!(ColorMode::from_str("always"), Some(ColorMode::Always));
        assert_eq!(ColorMode::from_str("ALWAYS"), Some(ColorMode::Always));
        assert_eq!(ColorMode::from_str("never"), Some(ColorMode::Never));
        assert_eq!(ColorMode::from_str("auto"), Some(ColorMode::Auto));
        assert_eq!(ColorMode::from_str("invalid"), None);
    }

    #[test]
    fn test_color_mode_always() {
        assert!(ColorMode::Always.is_enabled());
    }

    #[test]
    fn test_color_mode_never() {
        assert!(!ColorMode::Never.is_enabled());
    }
}
