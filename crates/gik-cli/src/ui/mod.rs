//! # CLI UI Module
//!
//! This module provides a consistent styling and formatting layer for GIK CLI output.
//!
//! ## Design Principles
//!
//! 1. **Scannable**: Users should identify success/failure in < 1 second
//! 2. **Consistent**: Same patterns across all commands
//! 3. **Progressive**: Show more detail only when needed
//! 4. **Accessible**: Work without colors (respect `NO_COLOR`)
//! 5. **Scriptable**: Machine-parseable with `--json` flag
//!
//! ## Module Structure
//!
//! - `color`: Color mode detection and terminal capability checks
//! - `style`: Message types, prefixes, and styling functions
//! - `format`: Utility formatters (bytes, time, truncation)
//! - `table`: Table rendering with comfy-table
//! - `progress`: Spinners and status lines for long operations

pub mod color;
pub mod format;
pub mod progress;
pub mod style;
pub mod table;

// Re-export main types for convenient access
pub use color::ColorMode;
pub use progress::{Progress, ProgressMode, StepTree};
pub use style::{MessageType, Style};
