//! Workspace detection and management.
//!
//! This module provides the [`Workspace`] type which represents a resolved
//! GIK workspace on disk, including its root path, knowledge directory,
//! and Git integration status.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::errors::GikError;

// Re-export constants from the constants module for backward compatibility.
// New code should prefer importing from `crate::constants` directly.
pub use crate::constants::{GUIDED_DIR, KNOWLEDGE_DIR};

// ============================================================================
// Helper Functions
// ============================================================================

/// Check if a path is a disk root (e.g., C:\ on Windows, / on Unix).
///
/// This prevents GIK from creating workspaces in dangerous locations like
/// the root of a filesystem, which could affect the entire system.
///
/// # Examples
///
/// ```ignore
/// assert!(is_disk_root(Path::new("/")));           // Unix root
/// assert!(is_disk_root(Path::new("C:\\"))); // Windows drive root
/// assert!(!is_disk_root(Path::new("/home/user")));
/// ```
fn is_disk_root(path: &Path) -> bool {
    // Check if the path has no parent (is a root)
    if path.parent().is_some() {
        return false;
    }

    // On Windows, check if it's a drive root like "C:\"
    #[cfg(windows)]
    {
        if let Some(s) = path.to_str() {
            // Windows drive roots are like "C:\" or "C:\\\\"
            if s.len() >= 2 && s.chars().nth(1) == Some(':') {
                return true;
            }
        }
    }

    // On Unix, "/" has no parent and is the root
    #[cfg(not(windows))]
    {
        if path == Path::new("/") {
            return true;
        }
    }

    // Fallback: if canonicalized path equals itself and has no parent
    path.canonicalize().ok().map_or(false, |p| p.parent().is_none())
}

// ============================================================================
// Workspace
// ============================================================================

/// A resolved GIK workspace.
///
/// Represents a project directory that has been identified as a GIK workspace
/// (or a candidate for initialization). Contains paths to key directories and
/// metadata about the workspace state.
///
/// # Example
///
/// ```ignore
/// use gik_core::Workspace;
/// use std::path::Path;
///
/// let workspace = Workspace::from_root(Path::new("/path/to/project"))?;
/// println!("Knowledge root: {:?}", workspace.knowledge_root());
/// ```
#[derive(Debug, Clone)]
pub struct Workspace {
    /// Absolute path to the workspace root directory.
    root: PathBuf,

    /// Path to the knowledge directory (`.guided/knowledge`).
    knowledge_root: PathBuf,

    /// Whether a `.git` directory exists at the workspace root.
    has_git: bool,

    /// Whether the workspace has been initialized (`.guided/knowledge` exists).
    initialized: bool,
}

impl Workspace {
    /// Create a `Workspace` from a root directory path.
    ///
    /// This constructor validates that the path exists and is a directory,
    /// then probes for `.git` and `.guided/knowledge` directories.
    ///
    /// # Arguments
    ///
    /// * `root` - Path to the workspace root directory.
    ///
    /// # Errors
    ///
    /// Returns [`GikError::PathNotFound`] if the path does not exist or is not
    /// a directory.
    ///
    /// # Example
    ///
    /// ```ignore
    /// let workspace = Workspace::from_root(Path::new("."))?;
    /// ```
    pub fn from_root(root: &Path) -> Result<Self, GikError> {
        let root = root
            .canonicalize()
            .map_err(|_| GikError::PathNotFound(root.display().to_string()))?;

        if !root.is_dir() {
            return Err(GikError::PathNotFound(root.display().to_string()));
        }

        // Prevent creating workspaces at disk roots (C:\, /, etc.)
        if is_disk_root(&root) {
            return Err(GikError::InvalidPath(
                format!(
                    "Cannot create GIK workspace at disk root: {}. \
                     Please create a workspace in a project directory instead.",
                    root.display()
                )
            ));
        }

        let guided_dir = root.join(GUIDED_DIR);
        let knowledge_root = guided_dir.join(KNOWLEDGE_DIR);
        let has_git = root.join(".git").is_dir();
        let initialized = knowledge_root.is_dir();

        Ok(Self {
            root,
            knowledge_root,
            has_git,
            initialized,
        })
    }

    /// Resolve a workspace by walking up from the given directory.
    ///
    /// Searches for a directory containing `.guided` or `.git` markers,
    /// starting from `start_dir` and walking up to parent directories.
    ///
    /// # Arguments
    ///
    /// * `start_dir` - Directory to start the search from.
    ///
    /// # Returns
    ///
    /// Returns `Ok(Workspace)` if a workspace root is found, or
    /// `Err(GikError::NotInitialized)` if no workspace markers are found.
    pub fn resolve(start_dir: &Path) -> Result<Self, GikError> {
        let start = start_dir
            .canonicalize()
            .map_err(|_| GikError::PathNotFound(start_dir.display().to_string()))?;

        let mut current = start.as_path();

        loop {
            // Stop searching at disk roots to prevent finding/creating workspaces there
            if is_disk_root(current) {
                break;
            }

            // Check for .guided directory (GIK workspace marker)
            if current.join(GUIDED_DIR).is_dir() {
                return Self::from_root(current);
            }

            // Check for .git directory (Git repository root)
            if current.join(".git").is_dir() {
                return Self::from_root(current);
            }

            // Move up to parent directory
            match current.parent() {
                Some(parent) => current = parent,
                None => break,
            }
        }

        // No workspace found - return the original directory as a candidate
        // This allows `gik init` to work in any directory
        Self::from_root(&start)
    }

    /// Get the absolute path to the workspace root.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Get the path to the knowledge directory (`.guided/knowledge`).
    pub fn knowledge_root(&self) -> &Path {
        &self.knowledge_root
    }

    /// Get the path to the `.guided` directory.
    pub fn guided_dir(&self) -> PathBuf {
        self.root.join(GUIDED_DIR)
    }

    /// Check if this workspace has a `.git` directory.
    pub fn has_git(&self) -> bool {
        self.has_git
    }

    /// Check if this workspace has been initialized with GIK.
    pub fn is_initialized(&self) -> bool {
        self.initialized
    }

    /// Get the path to a specific branch's knowledge directory.
    ///
    /// Returns `.guided/knowledge/{branch_name}`.
    pub fn branch_dir(&self, branch: &str) -> PathBuf {
        self.knowledge_root.join(branch)
    }

    /// Get the path to a specific base within a branch.
    ///
    /// Returns `.guided/knowledge/{branch_name}/{base_name}`.
    pub fn base_dir(&self, branch: &str, base: &str) -> PathBuf {
        self.branch_dir(branch).join(base)
    }

    /// Get the path to the staging file for a branch.
    ///
    /// Returns `.guided/knowledge/{branch_name}/staging.json`.
    pub fn staging_path(&self, branch: &str) -> PathBuf {
        self.branch_dir(branch).join("staging.json")
    }

    /// Get the path to the timeline file for a branch.
    ///
    /// Returns `.guided/knowledge/{branch_name}/timeline.jsonl`.
    pub fn timeline_path(&self, branch: &str) -> PathBuf {
        self.branch_dir(branch).join("timeline.jsonl")
    }

    /// Get the path to the HEAD file for a branch.
    ///
    /// Returns `.guided/knowledge/{branch_name}/HEAD`.
    pub fn head_path(&self, branch: &str) -> PathBuf {
        self.branch_dir(branch).join("HEAD")
    }

    /// Get the path to the GIK HEAD file (branch override).
    ///
    /// Returns `.guided/knowledge/HEAD`.
    pub fn gik_head_path(&self) -> PathBuf {
        self.knowledge_root.join("HEAD")
    }

    /// List all branches that have been initialized in this workspace.
    ///
    /// Returns the names of all directories under `.guided/knowledge/`.
    pub fn list_branches(&self) -> Result<Vec<BranchName>, GikError> {
        if !self.initialized {
            return Ok(Vec::new());
        }

        let mut branches = Vec::new();
        let entries = std::fs::read_dir(&self.knowledge_root)?;

        for entry in entries {
            let entry = entry?;
            let path = entry.path();

            // Only include directories (not files like config.yaml or HEAD)
            if path.is_dir() {
                if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                    // Skip hidden directories and validate the name
                    if !name.starts_with('.') && is_valid_branch_name(name) {
                        branches.push(BranchName::new_unchecked(name));
                    }
                }
            }
        }

        branches.sort_by(|a, b| a.as_str().cmp(b.as_str()));
        Ok(branches)
    }

    /// Check if a branch has been initialized in this workspace.
    pub fn branch_exists(&self, branch: &BranchName) -> bool {
        self.branch_dir(branch.as_str()).is_dir()
    }

    /// Get the path to the stack directory for a branch.
    ///
    /// Returns `.guided/knowledge/{branch_name}/stack/`.
    pub fn stack_dir(&self, branch: &str) -> PathBuf {
        self.branch_dir(branch).join("stack")
    }

    /// Get the path to the stack files JSONL for a branch.
    ///
    /// Returns `.guided/knowledge/{branch_name}/stack/files.jsonl`.
    pub fn stack_files_path(&self, branch: &str) -> PathBuf {
        self.stack_dir(branch).join("files.jsonl")
    }

    /// Get the path to the stack dependencies JSONL for a branch.
    ///
    /// Returns `.guided/knowledge/{branch_name}/stack/dependencies.jsonl`.
    pub fn stack_dependencies_path(&self, branch: &str) -> PathBuf {
        self.stack_dir(branch).join("dependencies.jsonl")
    }

    /// Get the path to the stack tech JSONL for a branch.
    ///
    /// Returns `.guided/knowledge/{branch_name}/stack/tech.jsonl`.
    pub fn stack_tech_path(&self, branch: &str) -> PathBuf {
        self.stack_dir(branch).join("tech.jsonl")
    }

    /// Get the path to the stack stats JSON for a branch.
    ///
    /// Returns `.guided/knowledge/{branch_name}/stack/stats.json`.
    pub fn stack_stats_path(&self, branch: &str) -> PathBuf {
        self.stack_dir(branch).join("stats.json")
    }

    /// Get the path to the staging directory for a branch.
    ///
    /// Returns `.guided/knowledge/{branch_name}/staging/`.
    pub fn staging_dir(&self, branch: &str) -> PathBuf {
        self.branch_dir(branch).join("staging")
    }

    /// Get the path to the pending sources JSONL for a branch.
    ///
    /// Returns `.guided/knowledge/{branch_name}/staging/pending.jsonl`.
    pub fn staging_pending_path(&self, branch: &str) -> PathBuf {
        self.staging_dir(branch).join("pending.jsonl")
    }

    /// Get the path to the staging summary JSON for a branch.
    ///
    /// Returns `.guided/knowledge/{branch_name}/staging/summary.json`.
    pub fn staging_summary_path(&self, branch: &str) -> PathBuf {
        self.staging_dir(branch).join("summary.json")
    }
}

// ============================================================================
// BranchName
// ============================================================================

/// A branch name within a GIK workspace.
///
/// Branch names correspond to Git branches or can be manually specified.
/// The default branch is typically `main`.
///
/// # Validation Rules
///
/// Valid branch names:
/// - Must be non-empty
/// - Can only contain alphanumeric characters, hyphens (`-`), underscores (`_`), and forward slashes (`/`)
/// - Forward slashes allow hierarchical branches (e.g., `feature/foo`)
///
/// # Example
///
/// ```
/// use gik_core::BranchName;
///
/// // Valid branch names
/// assert!(BranchName::try_new("main").is_ok());
/// assert!(BranchName::try_new("feature/my-feature").is_ok());
/// assert!(BranchName::try_new("release_v1.0").is_ok());
///
/// // Invalid branch names
/// assert!(BranchName::try_new("").is_err());
/// assert!(BranchName::try_new("branch with spaces").is_err());
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct BranchName(String);

impl BranchName {
    /// Create a new branch name with validation.
    ///
    /// # Errors
    ///
    /// Returns [`GikError::InvalidBranchName`] if the name is empty or contains
    /// invalid characters.
    pub fn try_new(name: impl Into<String>) -> Result<Self, GikError> {
        let name = name.into();
        if is_valid_branch_name(&name) {
            Ok(Self(name))
        } else {
            Err(GikError::InvalidBranchName(name))
        }
    }

    /// Create a new branch name without validation.
    ///
    /// # Safety
    ///
    /// This function does not validate the branch name. Use [`try_new`] for
    /// user-provided input. This method is intended for internal use where
    /// the branch name is known to be valid (e.g., from Git or constants).
    pub fn new_unchecked(name: impl Into<String>) -> Self {
        Self(name.into())
    }

    /// Get the default branch name (`main`).
    pub fn default_branch() -> Self {
        Self("main".to_string())
    }

    /// Get the branch name for detached HEAD state.
    pub fn detached_head() -> Self {
        Self("HEAD".to_string())
    }

    /// Get the branch name as a string slice.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Check if this is the detached HEAD branch.
    pub fn is_detached(&self) -> bool {
        self.0 == "HEAD"
    }
}

/// Check if a string is a valid branch name.
///
/// Valid branch names:
/// - Must be non-empty
/// - Can only contain alphanumeric characters, hyphens, underscores, periods, and forward slashes
/// - Cannot start or end with a slash
/// - Cannot contain consecutive slashes
pub fn is_valid_branch_name(name: &str) -> bool {
    if name.is_empty() {
        return false;
    }

    // Cannot start or end with slash
    if name.starts_with('/') || name.ends_with('/') {
        return false;
    }

    // Cannot contain consecutive slashes
    if name.contains("//") {
        return false;
    }

    // All characters must be valid
    name.chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '/' || c == '.')
}

impl std::fmt::Display for BranchName {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl Default for BranchName {
    fn default() -> Self {
        Self::default_branch()
    }
}

impl AsRef<str> for BranchName {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    // ------------------------------------------------------------------------
    // BranchName validation tests
    // ------------------------------------------------------------------------

    #[test]
    fn test_branch_name_valid_simple() {
        assert!(BranchName::try_new("main").is_ok());
        assert!(BranchName::try_new("develop").is_ok());
        assert!(BranchName::try_new("feature").is_ok());
    }

    #[test]
    fn test_branch_name_valid_with_hyphens() {
        assert!(BranchName::try_new("my-feature").is_ok());
        assert!(BranchName::try_new("bug-fix-123").is_ok());
    }

    #[test]
    fn test_branch_name_valid_with_underscores() {
        assert!(BranchName::try_new("my_feature").is_ok());
        assert!(BranchName::try_new("bug_fix_123").is_ok());
    }

    #[test]
    fn test_branch_name_valid_with_slashes() {
        assert!(BranchName::try_new("feature/my-feature").is_ok());
        assert!(BranchName::try_new("release/v1.0").is_ok());
        assert!(BranchName::try_new("user/john/experiment").is_ok());
    }

    #[test]
    fn test_branch_name_valid_with_periods() {
        assert!(BranchName::try_new("v1.0.0").is_ok());
        assert!(BranchName::try_new("release.1").is_ok());
    }

    #[test]
    fn test_branch_name_valid_with_numbers() {
        assert!(BranchName::try_new("release123").is_ok());
        assert!(BranchName::try_new("123").is_ok());
    }

    #[test]
    fn test_branch_name_invalid_empty() {
        let result = BranchName::try_new("");
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            GikError::InvalidBranchName(_)
        ));
    }

    #[test]
    fn test_branch_name_invalid_spaces() {
        assert!(BranchName::try_new("my feature").is_err());
        assert!(BranchName::try_new(" leading").is_err());
        assert!(BranchName::try_new("trailing ").is_err());
    }

    #[test]
    fn test_branch_name_invalid_special_chars() {
        assert!(BranchName::try_new("feature@1").is_err());
        assert!(BranchName::try_new("feature#1").is_err());
        assert!(BranchName::try_new("feature$1").is_err());
        assert!(BranchName::try_new("feature:1").is_err());
    }

    #[test]
    fn test_branch_name_invalid_slash_positions() {
        assert!(BranchName::try_new("/leading").is_err());
        assert!(BranchName::try_new("trailing/").is_err());
        assert!(BranchName::try_new("double//slash").is_err());
    }

    #[test]
    fn test_branch_name_default() {
        let branch = BranchName::default_branch();
        assert_eq!(branch.as_str(), "main");
    }

    #[test]
    fn test_branch_name_detached_head() {
        let branch = BranchName::detached_head();
        assert_eq!(branch.as_str(), "HEAD");
        assert!(branch.is_detached());
    }

    #[test]
    fn test_branch_name_display() {
        let branch = BranchName::try_new("feature/test").unwrap();
        assert_eq!(format!("{}", branch), "feature/test");
    }

    // ------------------------------------------------------------------------
    // Workspace tests
    // ------------------------------------------------------------------------

    #[test]
    fn test_workspace_from_root_with_git() {
        let temp = TempDir::new().unwrap();
        fs::create_dir(temp.path().join(".git")).unwrap();

        let workspace = Workspace::from_root(temp.path()).unwrap();
        assert!(workspace.has_git());
        assert!(!workspace.is_initialized());
    }

    #[test]
    fn test_workspace_from_root_with_guided() {
        let temp = TempDir::new().unwrap();
        fs::create_dir_all(temp.path().join(".guided/knowledge")).unwrap();

        let workspace = Workspace::from_root(temp.path()).unwrap();
        assert!(!workspace.has_git());
        assert!(workspace.is_initialized());
    }

    #[test]
    fn test_workspace_from_root_with_both() {
        let temp = TempDir::new().unwrap();
        fs::create_dir(temp.path().join(".git")).unwrap();
        fs::create_dir_all(temp.path().join(".guided/knowledge")).unwrap();

        let workspace = Workspace::from_root(temp.path()).unwrap();
        assert!(workspace.has_git());
        assert!(workspace.is_initialized());
    }

    #[test]
    fn test_workspace_resolve_from_subdir_with_git() {
        let temp = TempDir::new().unwrap();
        fs::create_dir(temp.path().join(".git")).unwrap();
        let subdir = temp.path().join("src/deep/nested");
        fs::create_dir_all(&subdir).unwrap();

        let workspace = Workspace::resolve(&subdir).unwrap();
        assert_eq!(
            workspace.root().canonicalize().unwrap(),
            temp.path().canonicalize().unwrap()
        );
    }

    #[test]
    fn test_workspace_resolve_from_subdir_with_guided() {
        let temp = TempDir::new().unwrap();
        fs::create_dir_all(temp.path().join(".guided/knowledge")).unwrap();
        let subdir = temp.path().join("src/deep/nested");
        fs::create_dir_all(&subdir).unwrap();

        let workspace = Workspace::resolve(&subdir).unwrap();
        assert_eq!(
            workspace.root().canonicalize().unwrap(),
            temp.path().canonicalize().unwrap()
        );
    }

    #[test]
    fn test_workspace_guided_takes_precedence() {
        // If both .git and .guided exist in different directories,
        // the one found first (walking up) should be used
        let temp = TempDir::new().unwrap();

        // Create .git at root
        fs::create_dir(temp.path().join(".git")).unwrap();

        // Create .guided in a subdirectory
        let subdir = temp.path().join("project");
        fs::create_dir_all(subdir.join(".guided/knowledge")).unwrap();

        // Resolve from within project/
        let deep = subdir.join("src");
        fs::create_dir_all(&deep).unwrap();

        let workspace = Workspace::resolve(&deep).unwrap();
        // Should find .guided first (in project/)
        assert_eq!(
            workspace.root().canonicalize().unwrap(),
            subdir.canonicalize().unwrap()
        );
    }

    #[test]
    fn test_workspace_paths() {
        let temp = TempDir::new().unwrap();
        fs::create_dir_all(temp.path().join(".guided/knowledge")).unwrap();

        let workspace = Workspace::from_root(temp.path()).unwrap();

        // Use ends_with to avoid macOS /private/var vs /var symlink issues
        assert!(workspace.knowledge_root().ends_with(".guided/knowledge"));
        assert!(workspace.guided_dir().ends_with(".guided"));
        assert!(workspace
            .branch_dir("main")
            .ends_with(".guided/knowledge/main"));
        assert!(workspace
            .base_dir("main", "code")
            .ends_with(".guided/knowledge/main/code"));
        assert!(workspace
            .staging_path("main")
            .ends_with(".guided/knowledge/main/staging.json"));
        assert!(workspace
            .timeline_path("main")
            .ends_with(".guided/knowledge/main/timeline.jsonl"));
        assert!(workspace
            .head_path("main")
            .ends_with(".guided/knowledge/main/HEAD"));
        assert!(workspace
            .gik_head_path()
            .ends_with(".guided/knowledge/HEAD"));
    }

    #[test]
    fn test_workspace_list_branches_empty() {
        let temp = TempDir::new().unwrap();
        fs::create_dir_all(temp.path().join(".guided/knowledge")).unwrap();

        let workspace = Workspace::from_root(temp.path()).unwrap();
        let branches = workspace.list_branches().unwrap();
        assert!(branches.is_empty());
    }

    #[test]
    fn test_workspace_list_branches() {
        let temp = TempDir::new().unwrap();
        fs::create_dir_all(temp.path().join(".guided/knowledge/main")).unwrap();
        fs::create_dir_all(temp.path().join(".guided/knowledge/develop")).unwrap();
        fs::create_dir_all(temp.path().join(".guided/knowledge/feature-x")).unwrap();
        // Also create a file (should be ignored)
        fs::write(
            temp.path().join(".guided/knowledge/config.yaml"),
            "test: true",
        )
        .unwrap();

        let workspace = Workspace::from_root(temp.path()).unwrap();
        let branches = workspace.list_branches().unwrap();

        assert_eq!(branches.len(), 3);
        assert_eq!(branches[0].as_str(), "develop");
        assert_eq!(branches[1].as_str(), "feature-x");
        assert_eq!(branches[2].as_str(), "main");
    }

    #[test]
    fn test_workspace_branch_exists() {
        let temp = TempDir::new().unwrap();
        fs::create_dir_all(temp.path().join(".guided/knowledge/main")).unwrap();

        let workspace = Workspace::from_root(temp.path()).unwrap();

        let main = BranchName::try_new("main").unwrap();
        let other = BranchName::try_new("other").unwrap();

        assert!(workspace.branch_exists(&main));
        assert!(!workspace.branch_exists(&other));
    }

    #[test]
    fn test_workspace_not_initialized_list_branches() {
        let temp = TempDir::new().unwrap();
        // No .guided directory

        let workspace = Workspace::from_root(temp.path()).unwrap();
        let branches = workspace.list_branches().unwrap();
        assert!(branches.is_empty());
    }

    #[test]
    #[cfg(windows)]
    fn test_workspace_from_root_rejects_windows_disk_root() {
        // Test that we reject Windows drive roots like C:\
        let result = Workspace::from_root(Path::new("C:\\"));
        assert!(result.is_err());
        
        if let Err(GikError::InvalidPath(msg)) = result {
            assert!(msg.contains("disk root"));
        } else {
            panic!("Expected InvalidPath error for disk root");
        }
    }

    #[test]
    #[cfg(not(windows))]
    fn test_workspace_from_root_rejects_unix_root() {
        // Test that we reject Unix root /
        let result = Workspace::from_root(Path::new("/"));
        assert!(result.is_err());
        
        if let Err(GikError::InvalidPath(msg)) = result {
            assert!(msg.contains("disk root"));
        } else {
            panic!("Expected InvalidPath error for disk root");
        }
    }
}
