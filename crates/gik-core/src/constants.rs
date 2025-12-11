//! Common constants used throughout gik-core.
//!
//! This module centralizes paths, directory names, and configuration constants
//! to avoid duplication and ensure consistency across the codebase.

// ============================================================================
// Directory Names
// ============================================================================

/// The name of the GIK metadata directory within a workspace.
///
/// All GIK-managed data lives under `.guided/` at the workspace root.
pub const GUIDED_DIR: &str = ".guided";

/// The subdirectory within `.guided` that stores knowledge data.
///
/// Layout: `.guided/knowledge/{branch}/bases/...`
pub const KNOWLEDGE_DIR: &str = "knowledge";

/// The name of the global GIK configuration directory.
///
/// Located at `~/.gik/` on Unix-like systems.
pub const GIK_HOME_DIR: &str = ".gik";

/// The name of the custom ignore file for GIK.
///
/// Similar to `.gitignore`, but specific to GIK indexing.
pub const GIK_IGNORE_FILENAME: &str = ".gikignore";

// ============================================================================
// Ignored Directories
// ============================================================================

/// Directories that should always be skipped during file traversal.
///
/// These directories typically contain generated/cached content that
/// shouldn't be indexed:
/// - `.git` - Git metadata
/// - `.guided` - GIK metadata
/// - `target` - Rust build output
/// - `node_modules` - Node.js dependencies
/// - `.next` - Next.js build output
/// - `dist` - Generic build output
/// - `build` - Generic build output
/// - `__pycache__` - Python bytecode cache
/// - `.venv`, `venv` - Python virtual environments
/// - `.mypy_cache` - MyPy type checker cache
/// - `.pytest_cache` - Pytest cache
pub const ALWAYS_IGNORED_DIRS: &[&str] = &[
    ".git",
    ".guided",
    "target",
    "node_modules",
    ".next",
    "dist",
    "build",
    "__pycache__",
    ".venv",
    "venv",
    ".mypy_cache",
    ".pytest_cache",
];

/// Check if a directory name should always be ignored.
///
/// # Arguments
///
/// * `name` - The directory name to check (not a full path).
///
/// # Returns
///
/// `true` if the directory should be skipped, `false` otherwise.
#[inline]
pub fn should_ignore_dir(name: &str) -> bool {
    ALWAYS_IGNORED_DIRS.contains(&name)
}

// ============================================================================
// Binary File Extensions
// ============================================================================

/// File extensions that indicate binary (non-text) content.
///
/// Files with these extensions are skipped during indexing since they
/// cannot be meaningfully embedded as text.
pub const BINARY_EXTENSIONS: &[&str] = &[
    // Images
    "png", "jpg", "jpeg", "gif", "bmp", "ico", "webp", "svg", "tiff", "tif",
    // Audio/Video
    "mp3", "mp4", "wav", "avi", "mov", "mkv", "flac", "ogg", "webm", // Archives
    "zip", "tar", "gz", "rar", "7z", "bz2", "xz", // Binaries/Executables
    "exe", "dll", "so", "dylib", "bin", "o", "a", "lib", "obj", // Documents
    "pdf", "doc", "docx", "xls", "xlsx", "ppt", "pptx", // Fonts
    "ttf", "otf", "woff", "woff2", "eot", // Database
    "db", "sqlite", "sqlite3", // Other binary/generated formats
    "pyc", "pyo", "class", "jar", "war",
    // Lock files and source maps (generated, not useful to index)
    "lock", "map",
];

/// Check if a file extension indicates binary content.
///
/// # Arguments
///
/// * `ext` - The file extension to check (without the leading dot).
///
/// # Returns
///
/// `true` if the file is likely binary, `false` otherwise.
#[inline]
pub fn is_binary_extension(ext: &str) -> bool {
    BINARY_EXTENSIONS.contains(&ext.to_lowercase().as_str())
}

// ============================================================================
// Configuration Filenames
// ============================================================================

/// The name of the global configuration file.
pub const GLOBAL_CONFIG_FILENAME: &str = "config.yaml";

/// The name of the project-level configuration file.
pub const PROJECT_CONFIG_FILENAME: &str = "gik.yaml";

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_should_ignore_dir() {
        assert!(should_ignore_dir(".git"));
        assert!(should_ignore_dir("node_modules"));
        assert!(should_ignore_dir("target"));
        assert!(should_ignore_dir(".guided"));
        assert!(!should_ignore_dir("src"));
        assert!(!should_ignore_dir("lib"));
    }

    #[test]
    fn test_is_binary_extension() {
        assert!(is_binary_extension("png"));
        assert!(is_binary_extension("PNG"));
        assert!(is_binary_extension("exe"));
        assert!(is_binary_extension("pdf"));
        assert!(!is_binary_extension("rs"));
        assert!(!is_binary_extension("ts"));
        assert!(!is_binary_extension("md"));
    }
}
