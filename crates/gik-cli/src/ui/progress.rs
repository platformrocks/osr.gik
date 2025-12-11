//! Progress indicators for long-running CLI operations.
//!
//! Provides spinners, progress bars, and multi-progress support using `indicatif`.
//! Progress indicators respect color settings and are disabled when stdout is
//! not a TTY or when `--quiet` mode is enabled.
//!
//! # Design
//!
//! - `ProgressMode`: Determines how progress is displayed (interactive, quiet, silent)
//! - `Progress`: Single spinner or progress bar
//! - `MultiProgress`: Multiple concurrent progress bars for parallel operations

use indicatif::{MultiProgress as IndicatifMultiProgress, ProgressBar, ProgressStyle};
use std::time::Duration;

use super::color::ColorMode;

/// Progress feedback mode based on output context.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProgressMode {
    /// Interactive TTY: show animated spinners and progress bars
    Interactive,
    /// Non-TTY or quiet: suppress progress, show only final results
    Quiet,
    /// Machine-readable: no progress at all (for --json)
    Silent,
}

impl ProgressMode {
    /// Detect the appropriate mode from environment and flags.
    pub fn detect(quiet: bool, json: bool, color_mode: ColorMode) -> Self {
        if json {
            Self::Silent
        } else if quiet || !atty::is(atty::Stream::Stdout) {
            Self::Quiet
        } else {
            // Check if colors are enabled (implies interactive terminal)
            if color_mode.is_enabled() || atty::is(atty::Stream::Stdout) {
                Self::Interactive
            } else {
                Self::Quiet
            }
        }
    }

    /// Check if progress should be shown.
    pub fn is_interactive(&self) -> bool {
        matches!(self, Self::Interactive)
    }
}

/// Spinner tick characters (Braille-based).
const SPINNER_CHARS: &str = "⠋⠙⠹⠸⠼⠴⠦⠧⠇⠏";

/// Progress bar characters.
const BAR_CHARS: &str = "█░";

/// A progress indicator that wraps indicatif.
///
/// Supports both spinner (indeterminate) and progress bar (determinate) modes.
pub struct Progress {
    bar: ProgressBar,
    mode: ProgressMode,
}

impl Progress {
    /// Create a spinner for indeterminate operations.
    ///
    /// # Example
    ///
    /// ```ignore
    /// let progress = Progress::spinner("Indexing sources...", mode);
    /// // ... do work ...
    /// progress.finish_with_message("[ok] Indexed 42 sources");
    /// ```
    pub fn spinner(message: &str, mode: ProgressMode) -> Self {
        let bar = if mode.is_interactive() {
            let pb = ProgressBar::new_spinner();
            pb.set_style(
                ProgressStyle::default_spinner()
                    .tick_chars(SPINNER_CHARS)
                    .template("{spinner:.cyan} {msg} ({elapsed})")
                    .expect("valid template"),
            );
            pb.set_message(message.to_string());
            pb.enable_steady_tick(Duration::from_millis(80));
            pb
        } else {
            // Hidden progress bar for quiet/silent mode
            ProgressBar::hidden()
        };

        Self { bar, mode }
    }

    /// Create a progress bar for determinate operations.
    ///
    /// # Example
    ///
    /// ```ignore
    /// let progress = Progress::bar(100, "Processing files", mode);
    /// for i in 0..100 {
    ///     progress.inc(1);
    /// }
    /// progress.finish_with_message("[ok] Processed 100 files");
    /// ```
    pub fn bar(total: u64, message: &str, mode: ProgressMode) -> Self {
        let bar = if mode.is_interactive() {
            let pb = ProgressBar::new(total);
            pb.set_style(
                ProgressStyle::default_bar()
                    .template("[{bar:20.cyan/dim}] {percent:>3}% ({pos}/{len}) {msg} ({elapsed})")
                    .expect("valid template")
                    .progress_chars(BAR_CHARS),
            );
            pb.set_message(message.to_string());
            pb
        } else {
            ProgressBar::hidden()
        };

        Self { bar, mode }
    }

    /// Update the message while running.
    pub fn set_message(&self, message: &str) {
        self.bar.set_message(message.to_string());
    }

    /// Set the current position (for bars).
    pub fn set_position(&self, pos: u64) {
        self.bar.set_position(pos);
    }

    /// Increment progress by delta (for bars).
    pub fn inc(&self, delta: u64) {
        self.bar.inc(delta);
    }

    /// Finish and clear the progress line.
    pub fn finish_clear(&self) {
        self.bar.finish_and_clear();
    }

    /// Finish with a message (replaces progress line).
    pub fn finish_with_message(&self, message: &str) {
        if self.mode.is_interactive() {
            self.bar.finish_and_clear();
        }
        if !message.is_empty() {
            println!("{}", message);
        }
    }

    /// Finish indicating success (convenience for common pattern).
    pub fn finish_ok(&self, message: &str) {
        self.finish_with_message(message);
    }

    /// Finish indicating error (convenience for common pattern).
    pub fn finish_err(&self, message: &str) {
        self.finish_with_message(message);
    }

    /// Get the elapsed time.
    pub fn elapsed(&self) -> Duration {
        self.bar.elapsed()
    }
}

/// Multi-progress for parallel operations.
///
/// Allows multiple progress bars to be displayed and updated concurrently.
pub struct MultiProgress {
    mp: IndicatifMultiProgress,
    mode: ProgressMode,
}

impl MultiProgress {
    /// Create a new multi-progress container.
    pub fn new(mode: ProgressMode) -> Self {
        let mp = if mode.is_interactive() {
            IndicatifMultiProgress::new()
        } else {
            // Create a hidden multi-progress for quiet mode
            let mp = IndicatifMultiProgress::new();
            mp.set_draw_target(indicatif::ProgressDrawTarget::hidden());
            mp
        };
        Self { mp, mode }
    }

    /// Add a spinner to the multi-progress.
    pub fn add_spinner(&self, message: &str) -> Progress {
        let bar = if self.mode.is_interactive() {
            let pb = self.mp.add(ProgressBar::new_spinner());
            pb.set_style(
                ProgressStyle::default_spinner()
                    .tick_chars(SPINNER_CHARS)
                    .template("{spinner:.cyan} {msg} ({elapsed})")
                    .expect("valid template"),
            );
            pb.set_message(message.to_string());
            pb.enable_steady_tick(Duration::from_millis(80));
            pb
        } else {
            self.mp.add(ProgressBar::hidden())
        };

        Progress {
            bar,
            mode: self.mode,
        }
    }

    /// Add a progress bar to the multi-progress.
    pub fn add_bar(&self, total: u64, message: &str) -> Progress {
        let bar = if self.mode.is_interactive() {
            let pb = self.mp.add(ProgressBar::new(total));
            pb.set_style(
                ProgressStyle::default_bar()
                    .template("[{bar:20.cyan/dim}] {percent:>3}% ({pos}/{len}) {msg} ({elapsed})")
                    .expect("valid template")
                    .progress_chars(BAR_CHARS),
            );
            pb.set_message(message.to_string());
            pb
        } else {
            self.mp.add(ProgressBar::hidden())
        };

        Progress {
            bar,
            mode: self.mode,
        }
    }

    /// Clear all progress bars and print a final message.
    pub fn finish_with_message(&self, message: &str) {
        self.mp.clear().ok();
        if !message.is_empty() {
            println!("{}", message);
        }
    }

    /// Clear all progress bars.
    pub fn finish_clear(&self) {
        self.mp.clear().ok();
    }
}

// Keep the old Spinner for backwards compatibility during migration
// TODO: Remove after full migration to Progress

/// Legacy spinner - wraps the new Progress API.
///
/// Kept for backwards compatibility. New code should use `Progress::spinner()`.
pub struct Spinner {
    progress: Progress,
}

impl Spinner {
    /// Create a new spinner with the given message.
    pub fn new(message: &str, color_mode: ColorMode) -> Self {
        let mode = ProgressMode::detect(false, false, color_mode);
        Self {
            progress: Progress::spinner(message, mode),
        }
    }

    /// Start the spinner (no-op with indicatif, starts automatically).
    pub fn start(&mut self) {
        // indicatif spinners start automatically with enable_steady_tick
    }

    /// Stop the spinner and print a final message.
    pub fn stop_with_message(self, final_message: &str) {
        self.progress.finish_with_message(final_message);
    }

    /// Stop the spinner without printing a message.
    pub fn stop(self) {
        self.progress.finish_clear();
    }
}

/// Legacy status line - wraps the new Progress API.
///
/// Kept for backwards compatibility. New code should use `Progress`.
pub struct StatusLine {
    is_tty: bool,
    last_len: usize,
}

impl StatusLine {
    /// Create a new status line.
    pub fn new() -> Self {
        Self {
            is_tty: atty::is(atty::Stream::Stdout),
            last_len: 0,
        }
    }

    /// Update the status line with a new message.
    pub fn update(&mut self, message: &str) {
        use std::io::{self, Write};
        let stdout = io::stdout();
        let mut handle = stdout.lock();

        if self.is_tty {
            let clear = " ".repeat(self.last_len);
            let _ = write!(handle, "\r{}\r{}", clear, message);
            self.last_len = message.len();
        } else {
            let _ = writeln!(handle, "{}", message);
        }
        let _ = handle.flush();
    }

    /// Finish the status line.
    pub fn finish(self) {
        if self.is_tty {
            println!();
        }
    }

    /// Finish with a final message.
    pub fn finish_with_message(self, message: &str) {
        use std::io::{self, Write};
        let stdout = io::stdout();
        let mut handle = stdout.lock();

        if self.is_tty {
            let clear = " ".repeat(self.last_len);
            let _ = write!(handle, "\r{}\r", clear);
        }
        let _ = writeln!(handle, "{}", message);
        let _ = handle.flush();
    }
}

impl Default for StatusLine {
    fn default() -> Self {
        Self::new()
    }
}

/// Step-tree progress for multi-phase operations.
///
/// Displays steps in a tree format with animated spinner:
/// ```text
/// ├─ ⠋ Parsing sources... (0.2s)      <- animated while running
/// ├─ Parsing sources done (0.2s)      <- after completion
/// └─ Building index done (0.5s)
/// ```
pub struct StepTree {
    mode: ProgressMode,
    completed_steps: Vec<String>,
    current_bar: Option<ProgressBar>,
    current_name: Option<String>,
    start_time: Option<std::time::Instant>,
}

impl StepTree {
    /// Create a new step-tree progress.
    pub fn new(mode: ProgressMode) -> Self {
        Self {
            mode,
            completed_steps: Vec::new(),
            current_bar: None,
            current_name: None,
            start_time: None,
        }
    }

    /// Start a new step. If a step is in progress, it will be marked as done.
    pub fn step(&mut self, name: &str) {
        // Finish previous step if any
        self.finish_current(false);

        // Start new step with animated spinner
        self.current_name = Some(name.to_string());
        self.start_time = Some(std::time::Instant::now());

        if self.mode.is_interactive() {
            let bar = ProgressBar::new_spinner();
            bar.set_style(
                ProgressStyle::default_spinner()
                    .tick_chars(SPINNER_CHARS)
                    .template("├─ {spinner:.cyan} {msg}")
                    .expect("valid template"),
            );
            bar.set_message(format!("{}...", name));
            bar.enable_steady_tick(Duration::from_millis(80));
            self.current_bar = Some(bar);
        }
    }

    /// Finish the current step with success.
    pub fn finish_step(&mut self) {
        self.finish_current(false);
    }

    /// Finish the current step, marking it as the last one.
    pub fn finish_last_step(&mut self) {
        self.finish_current(true);
    }

    fn finish_current(&mut self, is_last: bool) {
        if let Some(name) = self.current_name.take() {
            let elapsed = self.start_time.take()
                .map(|t| t.elapsed())
                .unwrap_or_default();
            let elapsed_str = format_duration(elapsed);

            // Stop and clear the spinner
            if let Some(bar) = self.current_bar.take() {
                bar.finish_and_clear();
            }

            // Print completed step
            if self.mode.is_interactive() {
                let prefix = if is_last { "└─" } else { "├─" };
                println!("{} {} done ({})", prefix, name, elapsed_str);
            }

            self.completed_steps.push(name);
        }
    }

    /// Get the number of completed steps.
    pub fn completed_count(&self) -> usize {
        self.completed_steps.len()
    }
}

/// Format a duration for display (e.g., "0.2s", "2.8s").
fn format_duration(d: std::time::Duration) -> String {
    let secs = d.as_secs_f64();
    if secs < 0.1 {
        format!("{:.0}ms", secs * 1000.0)
    } else {
        format!("{:.1}s", secs)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_progress_mode_detection() {
        // JSON mode always silent
        assert_eq!(
            ProgressMode::detect(false, true, ColorMode::Auto),
            ProgressMode::Silent
        );

        // Quiet mode
        assert_eq!(
            ProgressMode::detect(true, false, ColorMode::Auto),
            ProgressMode::Quiet
        );
    }

    #[test]
    fn test_progress_mode_is_interactive() {
        assert!(ProgressMode::Interactive.is_interactive());
        assert!(!ProgressMode::Quiet.is_interactive());
        assert!(!ProgressMode::Silent.is_interactive());
    }

    #[test]
    fn test_spinner_creation() {
        let spinner = Spinner::new("Testing...", ColorMode::Never);
        // Just verify it doesn't panic
        spinner.stop();
    }

    #[test]
    fn test_progress_spinner() {
        let progress = Progress::spinner("Testing...", ProgressMode::Quiet);
        progress.set_message("Updated");
        progress.finish_clear();
    }

    #[test]
    fn test_progress_bar() {
        let progress = Progress::bar(100, "Processing", ProgressMode::Quiet);
        progress.inc(50);
        progress.set_position(75);
        progress.finish_with_message("Done");
    }

    #[test]
    fn test_multi_progress() {
        let mp = MultiProgress::new(ProgressMode::Quiet);
        let p1 = mp.add_spinner("Task 1");
        let p2 = mp.add_bar(10, "Task 2");
        p1.finish_clear();
        p2.inc(5);
        p2.finish_clear();
        mp.finish_clear();
    }

    #[test]
    fn test_step_tree() {
        let mut tree = StepTree::new(ProgressMode::Quiet);
        tree.step("Step 1");
        tree.step("Step 2");
        tree.finish_last_step();
        // Verify no panic
    }
}

