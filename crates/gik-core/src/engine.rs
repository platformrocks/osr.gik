//! GIK Engine – the core orchestrator for all GIK operations.
//!
//! The [`GikEngine`] is the main entry point for GIK functionality. It manages
//! configuration, workspace detection, knowledge bases, and query execution.

use std::fs;
use std::path::{Path, PathBuf};

use ignore::WalkBuilder;

use crate::ask::StackSummary;
use crate::config::{DevicePreference, GlobalConfig, ProjectConfig};
use crate::constants::{is_binary_extension, should_ignore_dir, GIK_IGNORE_FILENAME};
use crate::embedding::{
    check_model_compatibility, read_model_info, EmbeddingConfig, ModelCompatibility, ModelInfo,
};
use crate::errors::GikError;
use crate::memory::{
    ingest_memory_entries,
    metrics::compute_memory_metrics,
    pruning::{apply_memory_pruning_policy, load_pruning_policy, MemoryPruningPolicy},
    MemoryEntry, MemoryIngestionOptions, MemoryIngestionResult, MEMORY_BASE_NAME,
};
use crate::release::{self, ReleaseOptions, ReleaseResult};
use crate::stack::{
    read_stats_json, read_tech_jsonl, scan_stack, write_dependencies_jsonl, write_files_jsonl,
    write_stats_json, write_tech_jsonl, StackStats,
};
use crate::staging::{
    add_pending_source, detect_file_change, infer_base, is_source_already_pending,
    list_pending_sources, load_staging_summary, ChangeType, IndexedFileInfo, NewPendingSource,
    PendingSource, PendingSourceId, PendingSourceKind, StagingSummary,
};
use crate::status::{HeadInfo, StatusReport};
use crate::timeline::{
    append_revision, get_revision, last_revision, read_head, read_timeline, write_head, Revision,
    RevisionId, RevisionOperation,
};
use crate::types::{
    AddOptions, AddResult, AddSourceSkip, BaseName, CommitOptions, CommitResult, CommitResultBase,
    MemoryIngestResult, MemoryMetricsResult, MemoryPruneEngineResult, ReindexOptions,
    ReindexResult, StatsQuery, StatsReport, UnstageOptions, UnstageResult, UnstageSourceSkip,
};
use crate::workspace::{BranchName, Workspace, GUIDED_DIR, KNOWLEDGE_DIR};

// ============================================================================
// GikEngine
// ============================================================================

/// The main engine for GIK operations.
///
/// `GikEngine` is the primary interface for all GIK functionality. It manages
/// configuration, workspace detection, knowledge bases, and query execution.
///
/// # Construction
///
/// Use [`GikEngine::from_global_config`] for typical usage, or [`GikEngine::new`]
/// for testing with custom factories.
///
/// # Example
///
/// ```ignore
/// use gik_core::{GikEngine, GlobalConfig};
///
/// let config = GlobalConfig::load_default()?;
/// let engine = GikEngine::from_global_config(config)?;
/// let workspace = engine.resolve_workspace(Path::new("."))?;
/// engine.init_workspace(&workspace)?;
/// ```
#[derive(Debug)]
pub struct GikEngine {
    /// Global configuration loaded from `~/.gik/config.yaml`.
    global_config: GlobalConfig,
    // TODO(gik.phase-0.4): Add index_factory: Option<Arc<dyn VectorIndexFactory>>
    // TODO(gik.phase-0.4): Add embedding_factory: Option<Arc<dyn EmbeddingProviderFactory>>
}

impl GikEngine {
    // -------------------------------------------------------------------------
    // Construction
    // -------------------------------------------------------------------------

    /// Create a new `GikEngine` from a global configuration.
    ///
    /// This is the recommended way to create an engine instance for CLI usage.
    /// Uses default factories for embeddings and vector indices.
    ///
    /// # Arguments
    ///
    /// * `global_config` - The global configuration to use.
    ///
    /// # Errors
    ///
    /// Returns an error if required resources cannot be initialized.
    pub fn from_global_config(global_config: GlobalConfig) -> anyhow::Result<Self> {
        // TODO(gik.phase-0.4): Initialize embedding and index factories
        Ok(Self { global_config })
    }

    /// Create a new `GikEngine` with default configuration.
    ///
    /// Loads the global configuration from the default location, or uses
    /// defaults if no configuration file exists.
    ///
    /// # Errors
    ///
    /// Returns an error if configuration loading fails.
    pub fn with_defaults() -> anyhow::Result<Self> {
        let config = GlobalConfig::load_default()?;
        Self::from_global_config(config)
    }

    /// Create a new `GikEngine` with configuration from a specific path.
    ///
    /// This allows overriding the default `~/.gik/config.yaml` location,
    /// useful for testing or when using a custom configuration file.
    ///
    /// # Arguments
    ///
    /// * `path` - Path to the configuration file.
    ///
    /// # Errors
    ///
    /// Returns an error if the file cannot be read or parsed.
    pub fn with_config(path: &Path) -> anyhow::Result<Self> {
        let config = GlobalConfig::from_path(path)?;
        Self::from_global_config(config)
    }

    /// Override the device preference for this engine instance.
    ///
    /// This allows forcing CPU or GPU mode at runtime, regardless of
    /// the configuration file setting. Useful for environment variable
    /// overrides or debugging.
    ///
    /// # Arguments
    ///
    /// * `device` - The device preference to use.
    pub fn set_device(&mut self, device: DevicePreference) {
        self.global_config.device = device;
    }

    /// Get a reference to the global configuration.
    pub fn global_config(&self) -> &GlobalConfig {
        &self.global_config
    }

    // -------------------------------------------------------------------------
    // Workspace operations
    // -------------------------------------------------------------------------

    /// Resolve a workspace from the given directory.
    ///
    /// Walks up the directory tree to find the workspace root (indicated by
    /// `.git` or `.guided` markers). If no markers are found, treats the
    /// given directory as the workspace root.
    ///
    /// # Arguments
    ///
    /// * `dir` - Directory to start searching from.
    ///
    /// # Errors
    ///
    /// Returns an error if the directory does not exist.
    pub fn resolve_workspace(&self, dir: &Path) -> Result<Workspace, GikError> {
        Workspace::resolve(dir)
    }

    /// Get the current branch name for a workspace.
    ///
    /// Branch detection priority:
    /// 1. GIK-specific override in `.guided/knowledge/HEAD` (if exists)
    /// 2. Git HEAD in `.git/HEAD` (if Git repo)
    /// 3. Default branch (`main`)
    ///
    /// # Detached HEAD
    ///
    /// If the Git repo is in detached HEAD state (not on a branch), this returns
    /// `BranchName::detached_head()` which is `"HEAD"`.
    ///
    /// # Arguments
    ///
    /// * `workspace` - The workspace to get the branch for.
    ///
    /// # Returns
    ///
    /// The current branch name.
    ///
    /// # Errors
    ///
    /// Returns [`GikError::BranchDetectionFailed`] if Git HEAD exists but cannot be read.
    pub fn current_branch(&self, workspace: &Workspace) -> Result<BranchName, GikError> {
        // Priority 1: Check for GIK-specific branch override
        let gik_head = workspace.gik_head_path();
        if gik_head.exists() {
            if let Ok(content) = fs::read_to_string(&gik_head) {
                let branch_name = content.trim();
                if !branch_name.is_empty() {
                    tracing::debug!("Using GIK HEAD override: {}", branch_name);
                    return BranchName::try_new(branch_name);
                }
            }
        }

        // Priority 2: Read from Git HEAD
        if workspace.has_git() {
            let head_path = workspace.root().join(".git/HEAD");
            let content = fs::read_to_string(&head_path).map_err(|e| {
                GikError::BranchDetectionFailed(format!("Failed to read .git/HEAD: {}", e))
            })?;

            let content = content.trim();

            // Check for symbolic ref (normal branch)
            if let Some(branch) = content.strip_prefix("ref: refs/heads/") {
                tracing::debug!("Detected Git branch: {}", branch);
                return BranchName::try_new(branch);
            }

            // Detached HEAD state (content is a commit hash)
            if content.len() == 40 && content.chars().all(|c| c.is_ascii_hexdigit()) {
                tracing::debug!("Detected detached HEAD at commit: {}", &content[..8]);
                return Ok(BranchName::detached_head());
            }

            // Unknown HEAD format
            tracing::warn!("Unknown .git/HEAD format: {}", content);
        }

        // Priority 3: Default branch
        tracing::debug!("Using default branch: main");
        Ok(BranchName::default_branch())
    }

    /// List all branches that have been initialized in a workspace.
    ///
    /// Returns the names of all branch directories under `.guided/knowledge/`.
    pub fn list_branches(&self, workspace: &Workspace) -> Result<Vec<BranchName>, GikError> {
        workspace.list_branches()
    }

    /// Check if a branch exists in a workspace.
    pub fn branch_exists(&self, workspace: &Workspace, branch: &BranchName) -> bool {
        workspace.branch_exists(branch)
    }

    /// Load the project configuration for a workspace.
    ///
    /// Reads `.guided/knowledge/config.yaml` if it exists, otherwise returns defaults.
    ///
    /// # Arguments
    ///
    /// * `workspace` - The workspace to load configuration for.
    ///
    /// # Errors
    ///
    /// Returns [`GikError::InvalidProjectConfig`] if the configuration file exists but is invalid.
    pub fn load_project_config(&self, workspace: &Workspace) -> Result<ProjectConfig, GikError> {
        ProjectConfig::load_from_workspace(workspace.root())
    }

    // -------------------------------------------------------------------------
    // Commands
    // -------------------------------------------------------------------------

    /// Initialize GIK structures for a workspace.
    ///
    /// Creates the `.guided/knowledge/` directory structure and initial
    /// configuration files for the current branch. Also runs a stack scan
    /// to populate the `stack/` directory with file, dependency, and tech info.
    ///
    /// This method is **idempotent**: if the branch is already initialized
    /// (HEAD exists), it returns [`GikError::AlreadyInitialized`] with the
    /// existing HEAD revision ID. Running `gik init` multiple times is safe.
    ///
    /// # Arguments
    ///
    /// * `workspace` - The workspace to initialize.
    ///
    /// # Returns
    ///
    /// A tuple of (revision ID, stack stats) from initialization.
    ///
    /// # Errors
    ///
    /// - [`GikError::AlreadyInitialized`] if the branch is already initialized.
    /// - I/O errors if directory or file creation fails.
    /// - [`GikError::StackScanFailed`] if stack scanning fails.
    pub fn init_workspace(
        &self,
        workspace: &Workspace,
    ) -> Result<(RevisionId, StackStats), GikError> {
        let branch = self.current_branch(workspace)?;

        // Check if the branch is already initialized (HEAD exists)
        let head_path = workspace.head_path(branch.as_str());
        if let Some(existing_head) = read_head(&head_path)? {
            return Err(GikError::AlreadyInitialized {
                branch: branch.to_string(),
                head: existing_head.to_string(),
            });
        }

        // Create .guided directory
        let guided_dir = workspace.root().join(GUIDED_DIR);
        fs::create_dir_all(&guided_dir)?;

        // Create .guided/knowledge directory
        let knowledge_dir = guided_dir.join(KNOWLEDGE_DIR);
        fs::create_dir_all(&knowledge_dir)?;

        // Create branch directory
        let branch_dir = knowledge_dir.join(branch.as_str());
        fs::create_dir_all(&branch_dir)?;

        // Create bases directory for knowledge bases (code, docs, memory)
        let bases_dir = branch_dir.join("bases");
        fs::create_dir_all(&bases_dir)?;
        for base in &["code", "docs", "memory"] {
            fs::create_dir_all(bases_dir.join(base))?;
        }

        // Create special directories (not under bases/)
        for dir in &["stack", "staging"] {
            fs::create_dir_all(branch_dir.join(dir))?;
        }

        // Create initial "Init" revision and set HEAD
        let init_revision = Revision::init(branch.as_str());
        let timeline_path = workspace.timeline_path(branch.as_str());

        append_revision(&timeline_path, &init_revision)?;
        write_head(&head_path, &init_revision.id)?;

        // Run stack scan and persist results
        let stats = self.scan_and_persist_stack(workspace, &branch)?;

        tracing::info!(
            "Initialized GIK workspace at {} on branch {} (revision {}, {} files scanned)",
            workspace.root().display(),
            branch,
            init_revision.id,
            stats.total_files
        );

        Ok((init_revision.id, stats))
    }

    /// Scan the workspace and persist stack inventory.
    ///
    /// Walks the workspace tree, collects file information, parses manifests
    /// for dependencies, and writes everything to the `stack/` directory.
    ///
    /// # Arguments
    ///
    /// * `workspace` - The workspace to scan.
    /// * `branch` - The branch to store results under.
    ///
    /// # Returns
    ///
    /// Statistics about the scanned stack.
    ///
    /// # Errors
    ///
    /// Returns an error if scanning or persistence fails.
    pub fn scan_and_persist_stack(
        &self,
        workspace: &Workspace,
        branch: &BranchName,
    ) -> Result<StackStats, GikError> {
        // Run the stack scan
        let inventory = scan_stack(workspace.root())?;

        // Persist to disk
        let files_path = workspace.stack_files_path(branch.as_str());
        let deps_path = workspace.stack_dependencies_path(branch.as_str());
        let tech_path = workspace.stack_tech_path(branch.as_str());
        let stats_path = workspace.stack_stats_path(branch.as_str());

        write_files_jsonl(&files_path, &inventory.files)
            .map_err(|e| GikError::StackPersistFailed(format!("files.jsonl: {}", e)))?;

        write_dependencies_jsonl(&deps_path, &inventory.dependencies)
            .map_err(|e| GikError::StackPersistFailed(format!("dependencies.jsonl: {}", e)))?;

        write_tech_jsonl(&tech_path, &inventory.tech)
            .map_err(|e| GikError::StackPersistFailed(format!("tech.jsonl: {}", e)))?;

        write_stats_json(&stats_path, &inventory.stats)
            .map_err(|e| GikError::StackPersistFailed(format!("stats.json: {}", e)))?;

        tracing::debug!(
            "Stack scan complete: {} files, {} dependencies, {} tech tags",
            inventory.stats.total_files,
            inventory.dependencies.len(),
            inventory.tech.len()
        );

        Ok(inventory.stats)
    }

    /// Stage sources for indexing.
    ///
    /// Adds paths, URLs, or archive references to the staging area for the
    /// next commit. Also triggers a full stack rescan to keep inventory updated.
    ///
    /// # Arguments
    ///
    /// * `workspace` - The workspace to stage sources in.
    /// * `branch` - The branch to stage sources for.
    /// * `opts` - Options including targets to stage and optional base override.
    ///
    /// # Returns
    ///
    /// An `AddResult` containing the IDs of created pending sources, any skipped
    /// sources with reasons, and updated stack statistics.
    ///
    /// # Errors
    ///
    /// Returns an error if the workspace is not initialized or if staging fails.
    pub fn add(
        &self,
        workspace: &Workspace,
        branch: &BranchName,
        opts: AddOptions,
    ) -> Result<AddResult, GikError> {
        if !workspace.is_initialized() {
            return Err(GikError::NotInitialized);
        }

        let pending_path = workspace.staging_pending_path(branch.as_str());
        let summary_path = workspace.staging_summary_path(branch.as_str());

        // Build index of already-indexed files for incremental add.
        // Maps file_path (workspace-relative) -> IndexedFileInfo
        let indexed_files = self.build_indexed_files_map(workspace, &branch)?;

        let mut created: Vec<String> = Vec::new();
        let mut skipped: Vec<AddSourceSkip> = Vec::new();
        let mut unchanged_count: usize = 0;

        for target in &opts.targets {
            // Infer source kind from the target string
            let kind = PendingSourceKind::infer(target, Some(workspace.root()));

            // Resolve full path for local files/directories
            // Use current directory for relative paths, not workspace root
            let full_path = if Path::new(target).is_absolute() {
                Path::new(target).to_path_buf()
            } else {
                std::env::current_dir()
                    .unwrap_or_else(|_| workspace.root().to_path_buf())
                    .join(target)
            };

            // Check for invalid sources
            match &kind {
                PendingSourceKind::FilePath | PendingSourceKind::Directory => {
                    if !full_path.exists() {
                        skipped.push(AddSourceSkip::new(
                            target.clone(),
                            format!("path not found: {}", full_path.display()),
                        ));
                        continue;
                    }
                }
                PendingSourceKind::Other(s) if s == "unknown" => {
                    skipped.push(AddSourceSkip::new(
                        target.clone(),
                        "could not determine source type",
                    ));
                    continue;
                }
                _ => {}
            }

            // Handle directory expansion
            if kind == PendingSourceKind::Directory {
                // Walk the directory and add each file
                let files = self.expand_directory(workspace, &full_path)?;

                if files.is_empty() {
                    skipped.push(AddSourceSkip::new(
                        target.clone(),
                        "directory contains no indexable files",
                    ));
                    continue;
                }

                for file_path in files {
                    let file_uri = self.normalize_uri(
                        workspace,
                        &file_path.to_string_lossy(),
                        &PendingSourceKind::FilePath,
                    );

                    // Determine base from file extension
                    let file_base = opts
                        .base
                        .clone()
                        .unwrap_or_else(|| infer_base(&file_uri, &PendingSourceKind::FilePath));

                    // Check for duplicates in staging
                    if is_source_already_pending(
                        &pending_path,
                        branch.as_str(),
                        &file_base,
                        &file_uri,
                    )? {
                        // Silently skip duplicates for directory expansion
                        continue;
                    }

                    // Check if file has changed since last index (incremental add)
                    let indexed_info = indexed_files.get(&file_uri);
                    let change_type = match detect_file_change(&file_path, indexed_info) {
                        Ok(ct) => ct,
                        Err(_) => ChangeType::New, // If we can't read metadata, treat as new
                    };

                    // Skip unchanged files
                    if change_type == ChangeType::Unchanged {
                        unchanged_count += 1;
                        continue;
                    }

                    // Phase 8.6: Skip empty files (0 bytes) - they cause embedding failures
                    if let Ok(metadata) = std::fs::metadata(&file_path) {
                        if metadata.len() == 0 {
                            tracing::debug!("Skipping empty file: {}", file_path.display());
                            continue;
                        }
                    }

                    let new_source = NewPendingSource {
                        base: Some(file_base),
                        uri: file_uri,
                        kind: Some(PendingSourceKind::FilePath),
                        change_type: Some(change_type),
                        metadata: None,
                    };

                    let id = add_pending_source(
                        &pending_path,
                        &summary_path,
                        branch.as_str(),
                        new_source,
                        Some(workspace.root()),
                    )?;

                    created.push(id.to_string());
                }
            } else {
                // Single file or URL
                let uri = self.normalize_uri(workspace, target, &kind);

                // Determine base: use explicit option, or infer from kind/extension
                let base = opts.base.clone().unwrap_or_else(|| infer_base(&uri, &kind));

                // Check for duplicates
                if is_source_already_pending(&pending_path, branch.as_str(), &base, &uri)? {
                    skipped.push(AddSourceSkip::new(
                        target.clone(),
                        format!("already pending for base '{}'", base),
                    ));
                    continue;
                }

                // Phase 8.6: Skip empty files (0 bytes) - they cause embedding failures
                if kind == PendingSourceKind::FilePath {
                    if let Ok(metadata) = std::fs::metadata(&full_path) {
                        if metadata.len() == 0 {
                            skipped.push(AddSourceSkip::new(
                                target.clone(),
                                "empty file (0 bytes)",
                            ));
                            continue;
                        }
                    }
                }

                // Detect change type for files
                let change_type = if kind == PendingSourceKind::FilePath {
                    let indexed_info = indexed_files.get(&uri);
                    match detect_file_change(&full_path, indexed_info) {
                        Ok(ct) => {
                            if ct == ChangeType::Unchanged {
                                unchanged_count += 1;
                                skipped.push(AddSourceSkip::new(
                                    target.clone(),
                                    "file unchanged since last index",
                                ));
                                continue;
                            }
                            Some(ct)
                        }
                        Err(_) => Some(ChangeType::New),
                    }
                } else {
                    None // URLs/archives don't have change detection
                };

                // Create the pending source
                let new_source = NewPendingSource {
                    base: Some(base),
                    uri,
                    kind: Some(kind),
                    change_type,
                    metadata: None,
                };

                let id = add_pending_source(
                    &pending_path,
                    &summary_path,
                    branch.as_str(),
                    new_source,
                    Some(workspace.root()),
                )?;

                created.push(id.to_string());
            }
        }

        // Stack scanning is now done in commit, not add.
        // This makes `gik add` faster and only does work when committing.
        let stack_stats = None;

        tracing::info!(
            "add: {} source(s) staged, {} skipped, {} unchanged",
            created.len(),
            skipped.len(),
            unchanged_count
        );

        Ok(AddResult {
            created,
            skipped,
            stack_stats,
        })
    }

    /// Unstage sources from the staging area.
    ///
    /// Removes pending sources that match the given targets. Targets can be
    /// file paths (relative to workspace root) that were previously staged.
    ///
    /// Only sources with status `Pending` or `Failed` can be unstaged.
    /// Sources that are `Processing` or `Indexed` are not affected.
    ///
    /// # Arguments
    ///
    /// * `workspace` - The resolved workspace.
    /// * `branch` - The branch to unstage sources from.
    /// * `opts` - Options including targets to unstage.
    ///
    /// # Returns
    ///
    /// An `UnstageResult` containing the files that were unstaged and any
    /// targets that were not found in staging.
    ///
    /// # Errors
    ///
    /// Returns an error if the workspace is not initialized or if unstaging fails.
    pub fn unstage(
        &self,
        workspace: &Workspace,
        branch: &BranchName,
        opts: UnstageOptions,
    ) -> Result<UnstageResult, GikError> {
        use crate::staging::unstage_sources;

        if !workspace.is_initialized() {
            return Err(GikError::NotInitialized);
        }

        let pending_path = workspace.staging_pending_path(branch.as_str());
        let summary_path = workspace.staging_summary_path(branch.as_str());

        // Normalize targets to workspace-relative paths
        let normalized_uris: Vec<String> = opts
            .targets
            .iter()
            .map(|target| self.normalize_uri(workspace, target, &PendingSourceKind::FilePath))
            .collect();

        // Call the staging module function
        let (removed, not_found) = unstage_sources(
            &pending_path,
            &summary_path,
            branch.as_str(),
            &normalized_uris,
        )?;

        // Map not_found back to original targets for better error messages
        let not_found_skips: Vec<UnstageSourceSkip> = not_found
            .iter()
            .map(|uri| {
                // Find the original target that normalized to this URI
                let original = opts
                    .targets
                    .iter()
                    .find(|t| {
                        self.normalize_uri(workspace, t, &PendingSourceKind::FilePath) == *uri
                    })
                    .cloned()
                    .unwrap_or_else(|| uri.clone());

                UnstageSourceSkip::new(original, "not found in staging")
            })
            .collect();

        tracing::info!(
            "unstage: {} source(s) removed, {} not found",
            removed.len(),
            not_found_skips.len()
        );

        Ok(UnstageResult {
            unstaged: removed,
            not_found: not_found_skips,
        })
    }

    /// Build a map of indexed files for change detection.
    ///
    /// Loads sources.jsonl from all bases and extracts file metadata.
    fn build_indexed_files_map(
        &self,
        workspace: &Workspace,
        branch: &BranchName,
    ) -> Result<std::collections::HashMap<String, IndexedFileInfo>, GikError> {
        use crate::base::{list_indexed_bases, load_base_sources, sources_path, base_root};

        let mut indexed_files = std::collections::HashMap::new();

        // Get all indexed bases for this branch
        let bases = list_indexed_bases(workspace.knowledge_root(), branch.as_str());

        for base_name in bases {
            let base_dir = base_root(workspace.knowledge_root(), branch.as_str(), &base_name);
            let sources_file = sources_path(&base_dir);

            // Load all source entries from this base
            let entries = load_base_sources(&sources_file)?;

            for entry in entries {
                // Use the file_path as the key (workspace-relative)
                indexed_files.insert(
                    entry.file_path.clone(),
                    IndexedFileInfo {
                        file_path: entry.file_path,
                        indexed_mtime: entry.indexed_mtime,
                        indexed_size: entry.indexed_size,
                    },
                );
            }
        }

        Ok(indexed_files)
    }

    /// Normalize a target URI for storage in pending sources.
    ///
    /// For local paths, converts to workspace-relative paths.
    /// For URLs, keeps them as-is.
    fn normalize_uri(
        &self,
        workspace: &Workspace,
        target: &str,
        kind: &PendingSourceKind,
    ) -> String {
        match kind {
            PendingSourceKind::Url => target.to_string(),
            _ => {
                // Local path: make workspace-relative
                let path = if Path::new(target).is_absolute() {
                    Path::new(target).to_path_buf()
                } else {
                    workspace.root().join(target)
                };

                // Try to make it relative to workspace root
                if let Ok(rel) = path.strip_prefix(workspace.root()) {
                    rel.to_string_lossy().to_string()
                } else {
                    // Fallback: use as-is (might be outside workspace)
                    target.to_string()
                }
            }
        }
    }

    /// Expand a directory into a list of indexable files.
    ///
    /// Walks the directory tree, respecting .gitignore and other ignore patterns,
    /// and returns a list of absolute paths to files that can be indexed.
    ///
    /// # Arguments
    ///
    /// * `workspace` - The workspace context.
    /// * `dir` - The directory to expand.
    ///
    /// # Returns
    ///
    /// A list of absolute paths to indexable files.
    fn expand_directory(
        &self,
        _workspace: &Workspace,
        dir: &Path,
    ) -> Result<Vec<PathBuf>, GikError> {
        let mut files = Vec::new();

        // Use WalkBuilder to respect .gitignore and other ignore patterns
        let walker = WalkBuilder::new(dir)
            .hidden(true) // Skip hidden files
            .git_ignore(true) // Respect .gitignore
            .git_global(true) // Respect global gitignore
            .git_exclude(true) // Respect .git/info/exclude
            .add_custom_ignore_filename(GIK_IGNORE_FILENAME) // Custom GIK ignore
            .follow_links(false) // Don't follow symlinks/junction points (prevents accessing protected system dirs on Windows)
            .filter_entry(|entry| {
                // Skip these directories
                let name = entry.file_name().to_string_lossy();
                !should_ignore_dir(&name)
            })
            .build();

        for result in walker {
            let entry = match result {
                Ok(e) => e,
                Err(e) => {
                    // Log permission errors but continue scanning
                    // Common on Windows with system directories
                    if let Some(io_err) = e.io_error() {
                        if io_err.kind() == std::io::ErrorKind::PermissionDenied {
                            tracing::debug!("Skipping directory due to permission denied: {}", e);
                            continue;
                        }
                    }
                    // For other errors, fail
                    return Err(GikError::StackScanFailed(format!("Failed to walk directory: {}", e)));
                }
            };

            let path = entry.path();

            // Skip the root directory itself
            if path == dir {
                continue;
            }

            // Skip directories
            if path.is_dir() {
                continue;
            }

            // Skip binary files by extension
            let ext = path
                .extension()
                .map(|e| e.to_string_lossy().to_lowercase())
                .unwrap_or_default();

            if is_binary_extension(&ext) {
                continue;
            }

            files.push(path.to_path_buf());
        }

        Ok(files)
    }

    // -------------------------------------------------------------------------
    // Staging APIs
    // -------------------------------------------------------------------------

    /// Add a pending source to the staging area.
    ///
    /// This is the low-level API for adding sources. The `add` method provides
    /// a higher-level interface that handles path walking and file discovery.
    ///
    /// # Arguments
    ///
    /// * `workspace` - The workspace to add the source to.
    /// * `branch` - The branch to stage the source for.
    /// * `new` - The new pending source to add.
    ///
    /// # Returns
    ///
    /// The ID of the newly created pending source.
    ///
    /// # Errors
    ///
    /// Returns an error if the workspace is not initialized or if staging
    /// file operations fail.
    pub fn add_pending_source(
        &self,
        workspace: &Workspace,
        branch: &BranchName,
        new: NewPendingSource,
    ) -> Result<PendingSourceId, GikError> {
        if !workspace.is_initialized() {
            return Err(GikError::NotInitialized);
        }

        let pending_path = workspace.staging_pending_path(branch.as_str());
        let summary_path = workspace.staging_summary_path(branch.as_str());

        add_pending_source(
            &pending_path,
            &summary_path,
            branch.as_str(),
            new,
            Some(workspace.root()),
        )
    }

    /// List all pending sources in the staging area.
    ///
    /// # Arguments
    ///
    /// * `workspace` - The workspace to list sources from.
    /// * `branch` - The branch to list sources for.
    ///
    /// # Returns
    ///
    /// A vector of all pending sources for the branch.
    pub fn list_pending_sources(
        &self,
        workspace: &Workspace,
        branch: &BranchName,
    ) -> Result<Vec<PendingSource>, GikError> {
        if !workspace.is_initialized() {
            return Ok(Vec::new());
        }

        let pending_path = workspace.staging_pending_path(branch.as_str());
        list_pending_sources(&pending_path)
    }

    /// Get the staging summary for a branch.
    ///
    /// # Arguments
    ///
    /// * `workspace` - The workspace to get the summary from.
    /// * `branch` - The branch to get the summary for.
    ///
    /// # Returns
    ///
    /// A summary of pending, indexed, and failed sources.
    pub fn staging_summary(
        &self,
        workspace: &Workspace,
        branch: &BranchName,
    ) -> Result<StagingSummary, GikError> {
        if !workspace.is_initialized() {
            return Ok(StagingSummary::default());
        }

        let pending_path = workspace.staging_pending_path(branch.as_str());
        let summary_path = workspace.staging_summary_path(branch.as_str());

        load_staging_summary(&summary_path, &pending_path)
    }

    /// Commit staged sources and create a new revision.
    ///
    /// Indexes all staged sources, generates embeddings, updates the vector
    /// index, and creates a new revision in the timeline.
    ///
    /// # Arguments
    ///
    /// * `workspace` - The workspace to commit in.
    /// * `opts` - Options including commit message.
    ///
    /// # Errors
    ///
    /// Returns [`GikError::NotInitialized`] if the workspace is not initialized,
    /// or [`GikError::CommitNoPendingSources`] if no sources are staged.
    pub fn commit(
        &self,
        workspace: &Workspace,
        opts: CommitOptions,
    ) -> Result<CommitResult, GikError> {
        if !workspace.is_initialized() {
            return Err(GikError::NotInitialized);
        }

        // Get current branch
        let branch = self.current_branch(workspace)?;

        // Run the commit pipeline with global config
        let summary = crate::commit::run_commit(workspace, &branch, &opts, &self.global_config)?;

        // Convert CommitSummary to CommitResult
        Ok(CommitResult {
            revision_id: summary.revision_id,
            total_indexed: summary.total_indexed,
            total_failed: summary.total_failed,
            touched_bases: summary.touched_bases,
            bases: summary
                .bases
                .into_iter()
                .map(|b| CommitResultBase {
                    base: b.base,
                    indexed_count: b.indexed_count,
                    failed_count: b.failed_count,
                    chunk_count: b.chunk_count,
                    file_count: b.file_count,
                })
                .collect(),
        })
    }

    /// Rebuild embeddings and index for a specific base.
    ///
    /// Re-processes all sources in the specified base and rebuilds the
    /// vector index from scratch with the current embedding model.
    ///
    /// # Arguments
    ///
    /// * `workspace` - The workspace containing the base.
    /// * `opts` - Options including the base name, force flag, and dry_run flag.
    ///
    /// # Behavior
    ///
    /// - If `dry_run` is true, reports what would change without writing.
    /// - If `force` is true, reindexes even if the model hasn't changed.
    /// - Creates a timeline revision only if `dry_run` is false AND actual
    ///   reindexing occurred.
    ///
    /// # Errors
    ///
    /// Returns [`GikError::NotInitialized`] if the workspace is not initialized,
    /// [`GikError::BaseNotFound`] if the base does not exist, or embedding/index
    /// errors if processing fails.
    pub fn reindex(
        &self,
        workspace: &Workspace,
        opts: ReindexOptions,
    ) -> Result<ReindexResult, GikError> {
        use crate::reindex::run_reindex;
        use crate::timeline::{append_revision, last_revision, RevisionId};

        if !workspace.is_initialized() {
            return Err(GikError::NotInitialized);
        }

        // Resolve branch
        let branch = opts
            .branch
            .clone()
            .unwrap_or_else(|| self.current_branch(workspace).unwrap().to_string());

        // Validate base exists and has indexed content
        if !crate::base::base_exists(workspace.knowledge_root(), &branch, &opts.base) {
            return Err(GikError::BaseNotFound(opts.base.clone()));
        }
        if !crate::base::is_base_indexed(workspace.knowledge_root(), &branch, &opts.base) {
            return Err(GikError::BaseNotIndexed {
                base: opts.base.clone(),
            });
        }

        // Get embedding config for the base from global/project config
        let embedding_config = self.embedding_config_for_base(workspace, &opts.base);

        // Get current git commit if available
        let git_commit = if workspace.has_git() {
            // Try to read git HEAD
            let head_path = workspace.root().join(".git/HEAD");
            fs::read_to_string(&head_path).ok().and_then(|content| {
                let content = content.trim();
                // Handle symbolic ref
                if let Some(refpath) = content.strip_prefix("ref: ") {
                    let ref_file = workspace.root().join(".git").join(refpath);
                    fs::read_to_string(ref_file)
                        .ok()
                        .map(|s| s.trim().to_string())
                } else if content.len() == 40 && content.chars().all(|c| c.is_ascii_hexdigit()) {
                    // Detached HEAD - content is the commit hash
                    Some(content.to_string())
                } else {
                    None
                }
            })
        } else {
            None
        };

        // Run reindex with resolved branch
        let resolved_opts = ReindexOptions {
            branch: Some(branch.clone()),
            ..opts
        };
        let mut result = run_reindex(
            workspace,
            &resolved_opts,
            &embedding_config,
            Some(&RevisionId::generate()),
            git_commit.as_deref(),
            self.global_config.device,
        )?;

        // If we have a revision and not dry_run, append to timeline
        if let Some(ref mut revision) = result.revision {
            let timeline_path = workspace.timeline_path(&branch);
            let head_path = workspace.head_path(&branch);

            // Get parent revision
            let parent = last_revision(&timeline_path)?;
            revision.parent_id = parent.map(|r| r.id);

            // Append to timeline
            append_revision(&timeline_path, revision)?;

            // Update HEAD
            crate::timeline::write_head(&head_path, &revision.id)?;

            tracing::info!(
                revision_id = %revision.id,
                base = %resolved_opts.base,
                "Reindex complete"
            );

            // Sync KG after successful reindex (Phase 9.2)
            //
            // KG sync is best-effort: failures are logged but don't fail the reindex.
            if let Err(e) = crate::kg::sync_branch_kg_default(workspace, &branch) {
                tracing::warn!(
                    branch = %branch,
                    error = %e,
                    "KG sync failed after reindex. Reindex succeeded but KG may be stale."
                );
            }
        } else if resolved_opts.dry_run {
            tracing::info!(base = %resolved_opts.base, "Reindex dry run complete");
        } else {
            tracing::info!(base = %resolved_opts.base, "No reindex needed (model unchanged)");
        }

        Ok(result)
    }

    // -------------------------------------------------------------------------
    // Memory Ingestion
    // -------------------------------------------------------------------------

    /// Ingest memory entries into the memory knowledge base.
    ///
    /// This method adds memory entries to the `memory` base, generating embeddings
    /// and creating vector index entries so they become immediately searchable.
    ///
    /// # Arguments
    ///
    /// * `workspace` - The workspace to ingest into.
    /// * `entries` - The memory entries to ingest.
    /// * `message` - Optional commit message for the timeline revision.
    ///
    /// # Returns
    ///
    /// A [`MemoryIngestionResult`] with counts and IDs of ingested/failed entries,
    /// along with the revision ID for the timeline entry.
    ///
    /// # Errors
    ///
    /// Returns [`GikError::NotInitialized`] if the workspace is not initialized,
    /// or embedding/index errors if processing fails.
    ///
    /// # Example
    ///
    /// ```ignore
    /// use gik_core::{GikEngine, MemoryEntry, MemoryScope, MemorySource};
    ///
    /// let engine = GikEngine::with_defaults()?;
    /// let workspace = engine.resolve_workspace(Path::new("."))?;
    ///
    /// let entries = vec![
    ///     MemoryEntry::new(
    ///         MemoryScope::Project,
    ///         MemorySource::Decision,
    ///         "We chose Rust for its safety and performance guarantees.",
    ///     )
    ///     .with_title("Language Choice")
    ///     .with_tags(vec!["architecture".into(), "language".into()]),
    /// ];
    ///
    /// let result = engine.ingest_memory(&workspace, entries, Some("Add architecture decision"))?;
    /// println!("Ingested {} entries", result.result.ingested_count);
    /// ```
    pub fn ingest_memory(
        &self,
        workspace: &Workspace,
        entries: Vec<MemoryEntry>,
        message: Option<&str>,
    ) -> Result<MemoryIngestResult, GikError> {
        if !workspace.is_initialized() {
            return Err(GikError::NotInitialized);
        }

        if entries.is_empty() {
            return Ok(MemoryIngestResult {
                revision_id: None,
                result: MemoryIngestionResult::default(),
            });
        }

        // Get current branch
        let branch = self.current_branch(workspace)?;
        let branch_str = branch.as_str();

        // Generate revision ID upfront so we can reference it in the entries
        let revision_id = RevisionId::generate();

        // Run ingestion
        let opts = MemoryIngestionOptions::default();
        let result = ingest_memory_entries(
            workspace.knowledge_root(),
            branch_str,
            entries,
            revision_id.as_str(),
            &opts,
        )?;

        // Only create a revision if we ingested something
        if result.ingested_count == 0 {
            return Ok(MemoryIngestResult {
                revision_id: None,
                result,
            });
        }

        // Build timeline revision
        let timeline_path = workspace.timeline_path(branch_str);
        let head_path = workspace.head_path(branch_str);

        let parent_id = last_revision(&timeline_path)?.map(|r| r.id);

        let message = message.unwrap_or("Ingest memory entries").to_string();
        let operation = RevisionOperation::MemoryIngest {
            count: result.ingested_count,
        };

        let revision = Revision::new(branch_str, parent_id, message, vec![operation]);

        // Append to timeline and update HEAD
        append_revision(&timeline_path, &revision)?;
        write_head(&head_path, &revision.id)?;

        tracing::info!(
            revision_id = %revision.id,
            count = result.ingested_count,
            "Memory ingestion complete"
        );

        Ok(MemoryIngestResult {
            revision_id: Some(revision.id.as_str().to_string()),
            result,
        })
    }

    // ========================================================================
    // Memory Metrics & Pruning
    // ========================================================================

    /// Get memory metrics for the specified branch.
    ///
    /// Returns metrics specific to the memory knowledge base, including:
    /// - Entry count
    /// - Estimated token count (using ~chars/4 heuristic)
    /// - Total character count
    /// - Configured pruning policy (if any)
    ///
    /// For read-only queries like metrics, returns zero values for uninitialized
    /// workspaces (graceful degradation, exit 0).
    ///
    /// # Arguments
    ///
    /// * `workspace` - The workspace to query.
    /// * `branch` - Optional branch (defaults to current branch).
    ///
    /// # Returns
    ///
    /// A [`MemoryMetricsResult`] with metrics and policy information.
    ///
    /// # Example
    ///
    /// ```ignore
    /// use gik_core::GikEngine;
    ///
    /// let engine = GikEngine::with_defaults()?;
    /// let workspace = engine.resolve_workspace(Path::new("."))?;
    ///
    /// let result = engine.memory_metrics(&workspace, None)?;
    /// println!("Memory entries: {}", result.metrics.entry_count);
    /// println!("Estimated tokens: {}", result.metrics.estimated_token_count);
    /// ```
    pub fn memory_metrics(
        &self,
        workspace: &Workspace,
        branch: Option<&str>,
    ) -> Result<MemoryMetricsResult, GikError> {
        // For read-only queries, return empty metrics for uninitialized workspaces
        if !workspace.is_initialized() {
            let branch_str = branch.unwrap_or("main");
            return Ok(MemoryMetricsResult {
                branch: branch_str.to_string(),
                metrics: crate::memory::metrics::MemoryMetrics::default(),
                pruning_policy: None,
            });
        }

        // Get branch
        let branch = match branch {
            Some(b) => BranchName::try_new(b)?,
            None => self.current_branch(workspace)?,
        };
        let branch_str = branch.as_str();

        // Get memory base directory
        let base_dir =
            crate::base::base_root(workspace.knowledge_root(), branch_str, MEMORY_BASE_NAME);

        // Compute metrics
        let metrics = compute_memory_metrics(&base_dir)?;

        // Load pruning policy (if configured)
        let pruning_policy = load_pruning_policy(&base_dir)?;

        Ok(MemoryMetricsResult {
            branch: branch_str.to_string(),
            metrics,
            pruning_policy,
        })
    }

    /// Prune memory entries based on the configured policy.
    ///
    /// This function applies the pruning policy configured in the memory base's
    /// `config.json` file. Entries can be either deleted or archived based on
    /// the policy mode.
    ///
    /// **Key behaviors:**
    /// - Pruning is EXPLICIT only (not auto-triggered by other operations)
    /// - Archived entries are NOT searchable but preserved for audit
    /// - A timeline revision is created to record the pruning operation
    ///
    /// # Arguments
    ///
    /// * `workspace` - The workspace to prune memory in.
    /// * `policy_override` - Optional policy to use instead of the configured one.
    /// * `message` - Optional commit message for the timeline revision.
    ///
    /// # Returns
    ///
    /// A [`MemoryPruneEngineResult`] with pruning statistics and revision ID.
    ///
    /// # Errors
    ///
    /// Returns [`GikError::NotInitialized`] if the workspace is not initialized.
    /// Returns [`GikError::BaseNotFound`] if no pruning policy is configured and
    /// no override is provided.
    ///
    /// # Example
    ///
    /// ```ignore
    /// use gik_core::GikEngine;
    ///
    /// let engine = GikEngine::with_defaults()?;
    /// let workspace = engine.resolve_workspace(Path::new("."))?;
    ///
    /// // Use configured policy
    /// let result = engine.prune_memory(&workspace, None, None)?;
    /// println!("Pruned {} entries", result.result.pruned_count);
    ///
    /// // Or with an explicit policy override
    /// use gik_core::memory::pruning::MemoryPruningPolicy;
    /// let policy = MemoryPruningPolicy::with_max_entries(100);
    /// let result = engine.prune_memory(&workspace, Some(policy), Some("Prune old entries"))?;
    /// ```
    pub fn prune_memory(
        &self,
        workspace: &Workspace,
        policy_override: Option<MemoryPruningPolicy>,
        message: Option<&str>,
    ) -> Result<MemoryPruneEngineResult, GikError> {
        use crate::memory::pruning::MemoryPruneResult;

        if !workspace.is_initialized() {
            return Err(GikError::NotInitialized);
        }

        // Get current branch
        let branch = self.current_branch(workspace)?;
        let branch_str = branch.as_str();

        // Get memory base directory
        let base_dir =
            crate::base::base_root(workspace.knowledge_root(), branch_str, MEMORY_BASE_NAME);

        // Get policy (override or configured)
        let policy = match policy_override {
            Some(p) => p,
            None => load_pruning_policy(&base_dir)?.ok_or(GikError::MissingPruningPolicy)?,
        };

        // Check if policy is disabled
        if policy.is_disabled() {
            return Ok(MemoryPruneEngineResult {
                revision_id: None,
                result: MemoryPruneResult::default(),
            });
        }

        // Apply pruning (without vector index for now - we can add index support later)
        // TODO: Open vector index and pass it for removing pruned vectors
        let result = apply_memory_pruning_policy(&base_dir, &policy, None)?;

        // Only create a revision if we pruned something
        if result.is_empty() {
            return Ok(MemoryPruneEngineResult {
                revision_id: None,
                result,
            });
        }

        // Build timeline revision
        let timeline_path = workspace.timeline_path(branch_str);
        let head_path = workspace.head_path(branch_str);

        let parent_id = last_revision(&timeline_path)?.map(|r| r.id);

        let message = message.unwrap_or("Prune memory entries").to_string();
        let operation = RevisionOperation::MemoryPrune {
            count: result.pruned_count as usize,
            archived_count: result.archived_count as usize,
            deleted_count: result.deleted_count as usize,
        };

        let revision = Revision::new(branch_str, parent_id, message, vec![operation]);

        // Append to timeline and update HEAD
        append_revision(&timeline_path, &revision)?;
        write_head(&head_path, &revision.id)?;

        tracing::info!(
            revision_id = %revision.id,
            count = result.pruned_count,
            archived = result.archived_count,
            deleted = result.deleted_count,
            "Memory pruning complete"
        );

        Ok(MemoryPruneEngineResult {
            revision_id: Some(revision.id.as_str().to_string()),
            result,
        })
    }

    // -------------------------------------------------------------------------
    // Knowledge Graph (KG) Operations
    // -------------------------------------------------------------------------

    /// Read KG statistics for a workspace and branch.
    ///
    /// Returns aggregate statistics about the knowledge graph including node count,
    /// edge count, last update timestamp, and schema version.
    ///
    /// If no KG has been created yet for this branch, returns default stats
    /// with zero counts.
    ///
    /// # Arguments
    ///
    /// * `workspace` - The workspace to read KG stats from.
    /// * `branch` - Optional branch override. If None, uses current branch.
    ///
    /// # Returns
    ///
    /// A [`KgStats`] struct with node/edge counts and metadata.
    ///
    /// # Errors
    ///
    /// Returns [`GikError::NotInitialized`] if the workspace is not initialized.
    /// Returns an error if the stats file exists but cannot be read.
    ///
    /// # Example
    ///
    /// ```ignore
    /// use gik_core::GikEngine;
    ///
    /// let engine = GikEngine::with_defaults()?;
    /// let workspace = engine.resolve_workspace(Path::new("."))?;
    ///
    /// let stats = engine.kg_read_stats(&workspace, None)?;
    /// println!("Nodes: {}, Edges: {}", stats.node_count, stats.edge_count);
    /// ```
    pub fn kg_read_stats(
        &self,
        workspace: &Workspace,
        branch: Option<&str>,
    ) -> Result<crate::kg::KgStats, GikError> {
        if !workspace.is_initialized() {
            return Err(GikError::NotInitialized);
        }

        let branch = match branch {
            Some(b) => BranchName::try_new(b)?,
            None => self.current_branch(workspace)?,
        };

        crate::kg::read_stats(workspace, branch.as_str())
    }

    /// List all KG nodes for a workspace and branch.
    ///
    /// Returns all nodes in the knowledge graph for the specified branch.
    /// If no KG has been created yet, returns an empty vector.
    ///
    /// # Arguments
    ///
    /// * `workspace` - The workspace to read nodes from.
    /// * `branch` - Optional branch override. If None, uses current branch.
    ///
    /// # Returns
    ///
    /// A vector of [`KgNode`] structs.
    ///
    /// # Errors
    ///
    /// Returns [`GikError::NotInitialized`] if the workspace is not initialized.
    /// Returns an error if the nodes file exists but cannot be read.
    ///
    /// # Example
    ///
    /// ```ignore
    /// use gik_core::GikEngine;
    ///
    /// let engine = GikEngine::with_defaults()?;
    /// let workspace = engine.resolve_workspace(Path::new("."))?;
    ///
    /// let nodes = engine.kg_list_nodes(&workspace, None)?;
    /// for node in &nodes {
    ///     println!("{}: {} ({})", node.id, node.label, node.kind);
    /// }
    /// ```
    pub fn kg_list_nodes(
        &self,
        workspace: &Workspace,
        branch: Option<&str>,
    ) -> Result<Vec<crate::kg::KgNode>, GikError> {
        if !workspace.is_initialized() {
            return Err(GikError::NotInitialized);
        }

        let branch = match branch {
            Some(b) => BranchName::try_new(b)?,
            None => self.current_branch(workspace)?,
        };

        crate::kg::read_all_nodes(workspace, branch.as_str())
    }

    /// List all KG edges for a workspace and branch.
    ///
    /// Returns all edges in the knowledge graph for the specified branch.
    /// If no KG has been created yet, returns an empty vector.
    ///
    /// # Arguments
    ///
    /// * `workspace` - The workspace to read edges from.
    /// * `branch` - Optional branch override. If None, uses current branch.
    ///
    /// # Returns
    ///
    /// A vector of [`KgEdge`] structs.
    ///
    /// # Errors
    ///
    /// Returns [`GikError::NotInitialized`] if the workspace is not initialized.
    /// Returns an error if the edges file exists but cannot be read.
    ///
    /// # Example
    ///
    /// ```ignore
    /// use gik_core::GikEngine;
    ///
    /// let engine = GikEngine::with_defaults()?;
    /// let workspace = engine.resolve_workspace(Path::new("."))?;
    ///
    /// let edges = engine.kg_list_edges(&workspace, None)?;
    /// for edge in &edges {
    ///     println!("{} --{}-> {}", edge.from, edge.kind, edge.to);
    /// }
    /// ```
    pub fn kg_list_edges(
        &self,
        workspace: &Workspace,
        branch: Option<&str>,
    ) -> Result<Vec<crate::kg::KgEdge>, GikError> {
        if !workspace.is_initialized() {
            return Err(GikError::NotInitialized);
        }

        let branch = match branch {
            Some(b) => BranchName::try_new(b)?,
            None => self.current_branch(workspace)?,
        };

        crate::kg::read_all_edges(workspace, branch.as_str())
    }

    /// Synchronize the Knowledge Graph for a branch.
    ///
    /// Performs a **full rebuild** of the KG by extracting nodes and edges
    /// from all bases (code, docs, etc.) for the specified branch.
    ///
    /// ## Phase 9.2 Behavior
    ///
    /// - Extracts file-level nodes (kind = "file") from code base
    /// - Extracts doc-level nodes (kind = "doc") from docs base
    /// - Creates import edges (kind = "imports") between files
    /// - Overwrites existing KG files (full rebuild, not incremental)
    ///
    /// ## Integration Points
    ///
    /// This method is called automatically by:
    /// - `commit()` - after base sources are updated
    /// - `reindex()` - after vector index is rebuilt
    ///
    /// It can also be called manually to refresh the KG.
    ///
    /// # Arguments
    ///
    /// * `workspace` - The workspace to sync KG for.
    /// * `branch` - Optional branch override. If None, uses current branch.
    ///
    /// # Returns
    ///
    /// A [`KgSyncResult`] with details of the operation.
    ///
    /// # Errors
    ///
    /// Returns [`GikError::NotInitialized`] if the workspace is not initialized.
    /// Returns an error if extraction or file I/O fails.
    ///
    /// # Example
    ///
    /// ```ignore
    /// use gik_core::GikEngine;
    ///
    /// let engine = GikEngine::with_defaults()?;
    /// let workspace = engine.resolve_workspace(Path::new("."))?;
    ///
    /// let result = engine.sync_kg_for_branch(&workspace, None)?;
    /// println!("Nodes: {}, Edges: {}", result.nodes_written, result.edges_written);
    /// ```
    pub fn sync_kg_for_branch(
        &self,
        workspace: &Workspace,
        branch: Option<&str>,
    ) -> Result<crate::kg::KgSyncResult, GikError> {
        if !workspace.is_initialized() {
            return Err(GikError::NotInitialized);
        }

        let branch = match branch {
            Some(b) => BranchName::try_new(b)?,
            None => self.current_branch(workspace)?,
        };

        crate::kg::sync_branch_kg_default(workspace, branch.as_str())
    }

    /// Export a KG subgraph in DOT or Mermaid format.
    ///
    /// Loads nodes and edges for the branch, applies size limits, and
    /// returns the formatted output string.
    ///
    /// # Arguments
    ///
    /// * `workspace` - The workspace to export from.
    /// * `branch` - Optional branch override. If None, uses current branch.
    /// * `format` - Output format (DOT or Mermaid).
    /// * `max_nodes` - Maximum number of nodes to include.
    /// * `max_edges` - Maximum number of edges to include.
    /// * `title` - Optional title for the graph.
    ///
    /// # Returns
    ///
    /// The formatted graph string, or None if no KG exists.
    ///
    /// # Errors
    ///
    /// Returns [`GikError::NotInitialized`] if the workspace is not initialized.
    pub fn export_kg_subgraph(
        &self,
        workspace: &Workspace,
        branch: Option<&str>,
        format: crate::kg::KgExportFormat,
        max_nodes: usize,
        max_edges: usize,
        title: Option<String>,
    ) -> Result<Option<String>, GikError> {
        if !workspace.is_initialized() {
            return Err(GikError::NotInitialized);
        }

        let branch = match branch {
            Some(b) => BranchName::try_new(b)?,
            None => self.current_branch(workspace)?,
        };

        let branch_str = branch.as_str();

        // Check if KG exists
        if !crate::kg_exists(workspace, branch_str) {
            return Ok(None);
        }

        // Load nodes and edges
        let nodes = crate::kg::read_all_nodes(workspace, branch_str).unwrap_or_default();
        let edges = crate::kg::read_all_edges(workspace, branch_str).unwrap_or_default();

        // Strategy: Greedily select edges, tracking node budget.
        // This ensures we get a connected graph within both limits.
        use std::collections::{HashMap, HashSet};

        let node_map: HashMap<&str, &crate::kg::KgNode> =
            nodes.iter().map(|n| (n.id.as_str(), n)).collect();

        let mut selected_node_ids: HashSet<String> = HashSet::new();
        let mut selected_edges: Vec<crate::kg::KgEdge> = Vec::new();

        for edge in &edges {
            if selected_edges.len() >= max_edges {
                break;
            }

            // Check if adding this edge would exceed node budget
            let from_new = !selected_node_ids.contains(&edge.from);
            let to_new = !selected_node_ids.contains(&edge.to);
            let nodes_needed = (from_new as usize) + (to_new as usize);

            if selected_node_ids.len() + nodes_needed <= max_nodes {
                // Both endpoints must exist in node_map
                if node_map.contains_key(edge.from.as_str())
                    && node_map.contains_key(edge.to.as_str())
                {
                    selected_node_ids.insert(edge.from.clone());
                    selected_node_ids.insert(edge.to.clone());
                    selected_edges.push(edge.clone());
                }
            }
        }

        // Build filtered nodes from selected IDs
        let filtered_nodes: Vec<_> = selected_node_ids
            .iter()
            .filter_map(|id| node_map.get(id.as_str()).cloned())
            .cloned()
            .collect();

        // Build export options
        let opts = crate::kg::KgExportOptions::new()
            .with_max_nodes(max_nodes)
            .with_max_edges(max_edges);
        let opts = if let Some(t) = title {
            opts.with_title(t)
        } else {
            opts
        };

        let output = crate::kg::export_kg(&filtered_nodes, &selected_edges, format, opts);
        Ok(Some(output))
    }

    /// Get current status of the workspace.
    ///
    /// Returns comprehensive information about the workspace state including:
    /// - Initialization status
    /// - HEAD revision information (id, operation, timestamp, message)
    /// - Staging summary (pending, indexed, failed counts)
    /// - Stack statistics (file count, languages, etc.)
    ///
    /// # Arguments
    ///
    /// * `workspace` - The workspace to get status for.
    /// * `branch` - The branch to get status for.
    ///
    /// # Returns
    ///
    /// A [`StatusReport`] with complete workspace state information.
    pub fn status(
        &self,
        workspace: &Workspace,
        branch: &BranchName,
    ) -> Result<StatusReport, GikError> {
        let initialized = workspace.is_initialized();

        // Early return for uninitialized workspace
        if !initialized {
            return Ok(StatusReport::uninitialized(
                workspace.root().to_path_buf(),
                branch.clone(),
            ));
        }

        // Read HEAD and get revision info
        let head = self.read_head_info(workspace, branch)?;

        // Get staging summary
        let staging = self.staging_summary(workspace, branch).ok();

        // Get stack stats and build summary
        let stack_stats_path = workspace.stack_stats_path(branch.as_str());
        let stack = read_stats_json(&stack_stats_path)?;

        // Build stack summary from stats and tech entries
        let stack_summary = if let Some(ref stats) = stack {
            let tech_path = workspace.stack_tech_path(branch.as_str());
            match read_tech_jsonl(&tech_path) {
                Ok(tech) if !tech.is_empty() => Some(StackSummary::from_stats_with_tech(stats, &tech)),
                _ => Some(StackSummary::from_stats(stats)),
            }
        } else {
            None
        };

        // Compute per-base stats (Phase 6.2)
        let bases = self.compute_bases_stats(workspace, branch);

        // Compute git-like working tree status
        let (staged_files, modified_files, working_tree_clean) =
            self.compute_working_tree_status(workspace, branch)?;

        Ok(StatusReport {
            workspace_root: workspace.root().to_path_buf(),
            branch: branch.clone(),
            is_initialized: true,
            head,
            staging,
            stack,
            stack_summary,
            bases,
            staged_files,
            modified_files,
            working_tree_clean,
        })
    }

    /// Compute git-like working tree status.
    ///
    /// Returns:
    /// - staged_files: Files in pending.jsonl with their change type
    /// - modified_files: Indexed files that have changed on disk since last commit
    /// - working_tree_clean: Whether there are no staged or modified files
    fn compute_working_tree_status(
        &self,
        workspace: &Workspace,
        branch: &BranchName,
    ) -> Result<(Option<Vec<crate::status::StagedFile>>, Option<Vec<String>>, Option<bool>), GikError>
    {
        use crate::staging::PendingSourceStatus;
        use crate::status::StagedFile;

        let pending_path = workspace.staging_pending_path(branch.as_str());

        // Get staged files from pending.jsonl
        let pending_sources = list_pending_sources(&pending_path)?;
        let staged_files: Vec<StagedFile> = pending_sources
            .iter()
            .filter(|s| s.status == PendingSourceStatus::Pending)
            .map(|s| StagedFile {
                path: s.uri.clone(),
                change_type: s.change_type.unwrap_or(ChangeType::New),
            })
            .collect();

        // Build indexed files map for modified detection
        let indexed_files = self.build_indexed_files_map(workspace, branch)?;

        // Check indexed files for modifications
        let mut modified_files: Vec<String> = Vec::new();
        for (file_path, info) in &indexed_files {
            let full_path = workspace.root().join(file_path);
            if full_path.exists() {
                match detect_file_change(&full_path, Some(info)) {
                    Ok(ChangeType::Modified) => {
                        // Only report as modified if not already staged
                        if !staged_files.iter().any(|s| &s.path == file_path) {
                            modified_files.push(file_path.clone());
                        }
                    }
                    _ => {}
                }
            }
        }

        // Compute working_tree_clean
        let working_tree_clean = staged_files.is_empty() && modified_files.is_empty();

        Ok((
            if staged_files.is_empty() {
                None
            } else {
                Some(staged_files)
            },
            if modified_files.is_empty() {
                None
            } else {
                Some(modified_files)
            },
            Some(working_tree_clean),
        ))
    }

    /// Read HEAD revision and build HeadInfo.
    ///
    /// Returns `None` if HEAD is missing or timeline is empty.
    fn read_head_info(
        &self,
        workspace: &Workspace,
        branch: &BranchName,
    ) -> Result<Option<HeadInfo>, GikError> {
        let head_path = workspace.head_path(branch.as_str());
        let timeline_path = workspace.timeline_path(branch.as_str());

        // Read HEAD pointer
        let head_id = match read_head(&head_path)? {
            Some(id) => id,
            None => return Ok(None),
        };

        // Get the revision from timeline
        let revision = match get_revision(&timeline_path, &head_id)? {
            Some(rev) => rev,
            None => {
                // HEAD points to a revision not in timeline, try last_revision
                match last_revision(&timeline_path)? {
                    Some(rev) => rev,
                    None => return Ok(None),
                }
            }
        };

        // Extract the primary operation (first in operations list)
        let operation = revision
            .operations
            .first()
            .cloned()
            .unwrap_or(crate::timeline::RevisionOperation::Init);

        Ok(Some(HeadInfo {
            revision_id: revision.id.to_string(),
            operation,
            timestamp: revision.timestamp,
            message: if revision.message.is_empty() {
                None
            } else {
                Some(revision.message)
            },
        }))
    }

    /// Compute per-base stats for a branch (Phase 6.2).
    ///
    /// Returns `Some(Vec<BaseStatsReport>)` with stats for each base,
    /// or `None` if no bases exist.
    fn compute_bases_stats(
        &self,
        workspace: &Workspace,
        branch: &BranchName,
    ) -> Option<Vec<crate::base::BaseStatsReport>> {
        use crate::status::compute_branch_stats;

        let branch_dir = workspace.branch_dir(branch.as_str());
        if !branch_dir.exists() {
            return None;
        }

        // Create closures for compatibility checks
        let model_compat_fn = |base: &str| -> Option<crate::embedding::ModelCompatibility> {
            self.model_compatibility(workspace, branch.as_str(), base)
                .ok()
        };

        let index_compat_fn =
            |base: &str| -> Option<crate::vector_index::VectorIndexCompatibility> {
                self.vector_index_compatibility(workspace, branch.as_str(), base)
                    .ok()
            };

        let bases = compute_branch_stats(&branch_dir, model_compat_fn, index_compat_fn);

        if bases.is_empty() {
            None
        } else {
            Some(bases)
        }
    }

    // -------------------------------------------------------------------------
    // Ask (Phase 5.1)
    // -------------------------------------------------------------------------

    /// Query the knowledge base and return relevant context.
    ///
    /// This is the main entry point for RAG-style queries. It:
    /// 1. Validates the workspace is initialized
    /// 2. Determines which bases to query
    /// 3. Embeds the question using the active embedding backend
    /// 4. Searches the vector indices for relevant chunks
    /// 5. Returns an `AskContextBundle` with results and metadata
    ///
    /// # Arguments
    ///
    /// * `workspace` - The workspace to query.
    /// * `branch` - The branch to query.
    /// * `opts` - Ask options (question, bases, top_k).
    ///
    /// # Returns
    ///
    /// An [`AskContextBundle`](crate::ask::AskContextBundle) containing:
    /// - Retrieved RAG chunks sorted by relevance
    /// - Stack summary (project fingerprint)
    /// - Debug info (model used, timing, per-base counts)
    ///
    /// # Errors
    ///
    /// - [`GikError::NotInitialized`] if the workspace is not initialized.
    /// - [`GikError::AskNoIndexedBases`] if no indexed bases are available.
    /// - [`GikError::AskEmbeddingError`] if query embedding fails.
    ///
    /// # Example
    ///
    /// ```ignore
    /// use gik_core::{GikEngine, ask::AskOptions};
    ///
    /// let engine = GikEngine::with_defaults()?;
    /// let workspace = engine.resolve_workspace(Path::new("."))?;
    /// let branch = engine.current_branch(&workspace)?;
    ///
    /// let opts = AskOptions::new("How does the API client work?")
    ///     .with_bases(vec!["code".to_string()])
    ///     .with_top_k(5);
    ///
    /// let result = engine.ask(&workspace, &branch, opts)?;
    /// for chunk in &result.rag_chunks {
    ///     println!("{}: {} (score: {})", chunk.path, chunk.snippet, chunk.score);
    /// }
    /// ```
    pub fn ask(
        &self,
        workspace: &Workspace,
        branch: &BranchName,
        opts: crate::ask::AskOptions,
    ) -> Result<crate::ask::AskContextBundle, GikError> {
        // Capture question and bases before consuming opts
        let question = opts.question.clone();
        let bases_filter = opts.bases.clone();

        // Load project config and resolve retrieval settings (project overrides global)
        let project_config = self.load_project_config(workspace)?;
        let retrieval_config = self.global_config.resolve_retrieval_config(&project_config);

        // Run the ask pipeline with resolved configs
        let bundle = crate::ask::run_ask(workspace, branch, opts, &self.global_config, &retrieval_config)?;

        // Persist ask log entry
        let bases = if let Some(ref filter) = bases_filter {
            filter.clone()
        } else {
            // If no filter, use the bases from the bundle
            bundle.bases.clone()
        };

        let entry = crate::log::AskLogEntry::new(
            branch.as_str(),
            question,
            bases,
            bundle.rag_chunks.len() as u32,
        );

        // Append to ask log (ignore errors for now, don't fail the ask)
        if let Err(e) = crate::log::append_ask_log(workspace, &entry) {
            tracing::warn!("Failed to persist ask log entry: {}", e);
        }

        Ok(bundle)
    }

    /// List available knowledge bases for a branch.
    ///
    /// Returns the names of all bases in the workspace's knowledge directory
    /// for the specified branch.
    ///
    /// # Arguments
    ///
    /// * `workspace` - The workspace to list bases for.
    /// * `branch` - The branch to list bases for.
    ///
    /// # Returns
    ///
    /// A vector of base names.
    pub fn list_bases(
        &self,
        workspace: &Workspace,
        branch: &BranchName,
    ) -> Result<Vec<BaseName>, GikError> {
        if !workspace.is_initialized() {
            return Ok(vec![]);
        }

        let branch_dir = workspace.branch_dir(branch.as_str());
        let bases_dir = branch_dir.join("bases");

        // Knowledge bases are under bases/ subdirectory
        let knowledge_bases = ["code", "docs", "memory"];
        // Special directories are at branch level
        let special_dirs = ["stack"];

        let mut bases = Vec::new();

        // Check bases/ subdirectory for knowledge bases
        if bases_dir.is_dir() {
            if let Ok(entries) = fs::read_dir(&bases_dir) {
                for entry in entries.flatten() {
                    if entry.path().is_dir() {
                        if let Some(name) = entry.file_name().to_str() {
                            if knowledge_bases.contains(&name) {
                                bases.push(BaseName::new(name));
                            }
                        }
                    }
                }
            }
        }

        // Check branch directory for special dirs (stack)
        if branch_dir.is_dir() {
            if let Ok(entries) = fs::read_dir(&branch_dir) {
                for entry in entries.flatten() {
                    if entry.path().is_dir() {
                        if let Some(name) = entry.file_name().to_str() {
                            if special_dirs.contains(&name) {
                                bases.push(BaseName::new(name));
                            }
                        }
                    }
                }
            }
        }

        Ok(bases)
    }

    // -------------------------------------------------------------------------
    // Embedding configuration (Phase 4.1)
    // -------------------------------------------------------------------------

    /// Get the root directory for a knowledge base.
    ///
    /// Returns the path `.guided/knowledge/<branch>/bases/<base>/`.
    ///
    /// # Arguments
    ///
    /// * `workspace` - The workspace containing the base.
    /// * `branch` - The branch name.
    /// * `base` - The knowledge base name (e.g., "code", "docs").
    pub fn base_root(&self, workspace: &Workspace, branch: &str, base: &str) -> PathBuf {
        workspace.branch_dir(branch).join("bases").join(base)
    }

    /// Get the path to the model-info file for a knowledge base.
    ///
    /// Returns `.guided/knowledge/<branch>/bases/<base>/model-info.json`.
    ///
    /// # Arguments
    ///
    /// * `workspace` - The workspace containing the base.
    /// * `branch` - The branch name.
    /// * `base` - The knowledge base name.
    pub fn model_info_path(&self, workspace: &Workspace, branch: &str, base: &str) -> PathBuf {
        self.base_root(workspace, branch, base)
            .join("model-info.json")
    }

    /// Get the resolved embedding configuration for a knowledge base.
    ///
    /// Resolution precedence:
    /// 1. Project per-base override (`.guided/knowledge/config.yaml`)
    /// 2. Global per-base override (`~/.gik/config.yaml`)
    /// 3. Global default embedding config
    /// 4. Hard-coded default (Candle + all-MiniLM-L6-v2)
    ///
    /// # Arguments
    ///
    /// * `workspace` - The workspace (for loading project config).
    /// * `base` - The knowledge base name.
    ///
    /// # Returns
    ///
    /// The resolved embedding configuration.
    pub fn embedding_config_for_base(&self, workspace: &Workspace, base: &str) -> EmbeddingConfig {
        // Try to load project config
        let project_config =
            ProjectConfig::load_from_workspace(workspace.root()).unwrap_or_default();

        // Use project config resolution which considers global config
        project_config.resolve_embedding_config(base, &self.global_config)
    }

    /// Load the model-info for a knowledge base.
    ///
    /// Returns `Ok(None)` if the model-info file does not exist (base has not
    /// been indexed yet).
    ///
    /// # Arguments
    ///
    /// * `workspace` - The workspace containing the base.
    /// * `branch` - The branch name.
    /// * `base` - The knowledge base name.
    ///
    /// # Errors
    ///
    /// Returns an error if the file exists but cannot be read or parsed.
    pub fn load_model_info(
        &self,
        workspace: &Workspace,
        branch: &str,
        base: &str,
    ) -> Result<Option<ModelInfo>, GikError> {
        let path = self.model_info_path(workspace, branch, base);
        read_model_info(&path)
    }

    /// Check model compatibility for a knowledge base.
    ///
    /// Compares the current embedding configuration with the stored model-info
    /// to determine if the index can be used or needs reindexing.
    ///
    /// # Arguments
    ///
    /// * `workspace` - The workspace containing the base.
    /// * `branch` - The branch name.
    /// * `base` - The knowledge base name.
    ///
    /// # Returns
    ///
    /// - `Compatible` - The configured model matches the stored model-info.
    /// - `MissingModelInfo` - No model-info exists (base not indexed yet).
    /// - `Mismatch { .. }` - The configured model differs from stored info.
    ///
    /// # Errors
    ///
    /// Returns an error if model-info loading fails.
    pub fn model_compatibility(
        &self,
        workspace: &Workspace,
        branch: &str,
        base: &str,
    ) -> Result<ModelCompatibility, GikError> {
        // Resolve current embedding config
        let config = self.embedding_config_for_base(workspace, base);

        // Load stored model-info (if any)
        let model_info = self.load_model_info(workspace, branch, base)?;

        // Check compatibility
        Ok(check_model_compatibility(&config, model_info.as_ref()))
    }

    // -------------------------------------------------------------------------
    // Vector Index Operations (Phase 4.2)
    // -------------------------------------------------------------------------

    /// Get the path to the index directory for a knowledge base.
    ///
    /// Returns `.guided/knowledge/<branch>/bases/<base>/index/`.
    ///
    /// # Arguments
    ///
    /// * `workspace` - The workspace containing the base.
    /// * `branch` - The branch name.
    /// * `base` - The knowledge base name.
    pub fn index_root(&self, workspace: &Workspace, branch: &str, base: &str) -> PathBuf {
        self.base_root(workspace, branch, base).join("index")
    }

    /// Get the path to the index metadata file for a knowledge base.
    ///
    /// Returns `.guided/knowledge/<branch>/bases/<base>/index/meta.json`.
    ///
    /// # Arguments
    ///
    /// * `workspace` - The workspace containing the base.
    /// * `branch` - The branch name.
    /// * `base` - The knowledge base name.
    pub fn index_meta_path(&self, workspace: &Workspace, branch: &str, base: &str) -> PathBuf {
        use crate::vector_index::INDEX_META_FILENAME;
        self.index_root(workspace, branch, base)
            .join(INDEX_META_FILENAME)
    }

    /// Get the resolved vector index configuration for a knowledge base.
    ///
    /// Resolution precedence:
    /// 1. Project per-base override (`.guided/knowledge/config.yaml`)
    /// 2. Global per-base override (`~/.gik/config.yaml`)
    /// 3. Global default index config
    /// 4. Hard-coded default (SimpleFile + Cosine)
    ///
    /// # Arguments
    ///
    /// * `workspace` - The workspace (for loading project config).
    /// * `base` - The knowledge base name.
    ///
    /// # Returns
    ///
    /// The resolved vector index configuration.
    pub fn vector_index_config_for_base(
        &self,
        workspace: &Workspace,
        base: &str,
    ) -> crate::vector_index::VectorIndexConfig {
        // First, resolve embedding config to get dimension
        let embedding_config = self.embedding_config_for_base(workspace, base);

        // Try to load project config
        let project_config =
            ProjectConfig::load_from_workspace(workspace.root()).unwrap_or_default();

        // Use project config resolution which considers global config
        project_config.resolve_vector_index_config(base, &embedding_config, &self.global_config)
    }

    /// Load the vector index metadata for a knowledge base.
    ///
    /// Returns `Ok(None)` if the metadata file does not exist (index has not
    /// been created yet).
    ///
    /// # Arguments
    ///
    /// * `workspace` - The workspace containing the base.
    /// * `branch` - The branch name.
    /// * `base` - The knowledge base name.
    ///
    /// # Errors
    ///
    /// Returns an error if the file exists but cannot be read or parsed.
    pub fn load_vector_index_meta(
        &self,
        workspace: &Workspace,
        branch: &str,
        base: &str,
    ) -> Result<Option<crate::vector_index::VectorIndexMeta>, GikError> {
        use crate::vector_index::load_index_meta;
        let path = self.index_meta_path(workspace, branch, base);
        load_index_meta(&path)
    }

    /// Check vector index compatibility for a knowledge base.
    ///
    /// Compares the current configuration with the stored index metadata
    /// to determine if the index can be used or needs reindexing.
    ///
    /// Checks are performed in this order (per user request):
    /// 1. Embedding model mismatch
    /// 2. Dimension mismatch
    /// 3. Backend mismatch
    ///
    /// # Arguments
    ///
    /// * `workspace` - The workspace containing the base.
    /// * `branch` - The branch name.
    /// * `base` - The knowledge base name.
    ///
    /// # Returns
    ///
    /// - `Compatible` - The index matches current configuration.
    /// - `MissingMeta` - No index metadata exists (index not created yet).
    /// - `EmbeddingMismatch { .. }` - Embedding model differs.
    /// - `DimensionMismatch { .. }` - Vector dimension differs.
    /// - `BackendMismatch { .. }` - Backend type differs.
    ///
    /// # Errors
    ///
    /// Returns an error if metadata loading fails.
    pub fn vector_index_compatibility(
        &self,
        workspace: &Workspace,
        branch: &str,
        base: &str,
    ) -> Result<crate::vector_index::VectorIndexCompatibility, GikError> {
        use crate::vector_index::check_index_compatibility;

        // Resolve current configs
        let embedding_config = self.embedding_config_for_base(workspace, base);
        let index_config = self.vector_index_config_for_base(workspace, base);

        // Load stored index metadata (if any)
        let index_meta = self.load_vector_index_meta(workspace, branch, base)?;

        // Check compatibility
        Ok(check_index_compatibility(
            &index_config,
            &embedding_config,
            index_meta.as_ref(),
        ))
    }

    /// Open a vector index for a knowledge base.
    ///
    /// Creates a new index if it doesn't exist, or opens an existing one.
    /// Returns an error if the index is incompatible with current configuration.
    ///
    /// # Arguments
    ///
    /// * `workspace` - The workspace containing the base.
    /// * `branch` - The branch name.
    /// * `base` - The knowledge base name.
    ///
    /// # Returns
    ///
    /// A boxed vector index backend instance.
    ///
    /// # Errors
    ///
    /// - `VectorIndexIncompatible` - Index exists but is incompatible.
    /// - `VectorIndexBackendUnavailable` - Requested backend is not available.
    /// - I/O errors from index creation/loading.
    pub fn open_vector_index(
        &self,
        workspace: &Workspace,
        branch: &str,
        base: &str,
    ) -> Result<Box<dyn crate::vector_index::VectorIndexBackend>, GikError> {
        use crate::vector_index::{open_vector_index, VectorIndexCompatibility};

        // Check compatibility first
        let compatibility = self.vector_index_compatibility(workspace, branch, base)?;

        match compatibility {
            VectorIndexCompatibility::Compatible | VectorIndexCompatibility::MissingMeta => {
                // OK to proceed
            }
            VectorIndexCompatibility::LegacyFormat { message } => {
                return Err(GikError::VectorIndexIncompatible {
                    base: base.to_string(),
                    reason: format!(
                        "Legacy index format detected: {}. Run `gik reindex` to upgrade to LanceDB.",
                        message
                    ),
                });
            }
            VectorIndexCompatibility::EmbeddingMismatch {
                config_model,
                meta_model,
            } => {
                return Err(GikError::VectorIndexIncompatible {
                    base: base.to_string(),
                    reason: format!(
                        "Embedding model mismatch: config uses '{}', index was built with '{}'. Run `gik reindex` to rebuild.",
                        config_model, meta_model
                    ),
                });
            }
            VectorIndexCompatibility::DimensionMismatch { config, meta } => {
                return Err(GikError::VectorIndexIncompatible {
                    base: base.to_string(),
                    reason: format!(
                        "Dimension mismatch: config has {}, index has {}. Run `gik reindex` to rebuild.",
                        config, meta
                    ),
                });
            }
            VectorIndexCompatibility::BackendMismatch {
                config_backend,
                meta_backend,
            } => {
                return Err(GikError::VectorIndexIncompatible {
                    base: base.to_string(),
                    reason: format!(
                        "Backend mismatch: config uses '{}', index uses '{}'. Run `gik reindex` to rebuild.",
                        config_backend, meta_backend
                    ),
                });
            }
        }

        // Get configs
        let embedding_config = self.embedding_config_for_base(workspace, base);
        let index_config = self.vector_index_config_for_base(workspace, base);
        let index_root = self.index_root(workspace, branch, base);

        // Open the vector index using the unified factory
        open_vector_index(index_root, index_config, &embedding_config)
    }

    /// Get statistics for knowledge bases.
    ///
    /// Returns aggregated statistics for all bases or a specific base.
    /// For read-only queries like stats, returns empty data for uninitialized
    /// workspaces (graceful degradation, exit 0).
    ///
    /// # Arguments
    ///
    /// * `workspace` - The workspace to get stats for.
    /// * `branch` - The branch to get stats for.
    /// * `query` - Options including optional base name filter.
    ///
    /// # Returns
    ///
    /// A stats report with per-base statistics.
    ///
    /// # Errors
    ///
    /// Returns [`GikError::BaseNotFound`] if a specific base was requested but
    /// does not exist.
    pub fn stats(
        &self,
        workspace: &Workspace,
        branch: &BranchName,
        query: StatsQuery,
    ) -> Result<StatsReport, GikError> {
        // For read-only queries, return empty stats for uninitialized workspaces
        if !workspace.is_initialized() {
            return Ok(StatsReport {
                branch: branch.to_string(),
                bases: vec![],
                stack: None,
                total_documents: 0,
                total_vectors: 0,
                total_on_disk_bytes: 0,
            });
        }

        // Compute base stats using the same logic as status command
        let all_bases = self.compute_bases_stats(workspace, branch).unwrap_or_default();

        // Filter by base name if specified
        let bases: Vec<crate::base::BaseStatsReport> = match &query.base {
            Some(base_name) => {
                let filtered: Vec<_> = all_bases
                    .into_iter()
                    .filter(|b| b.base == *base_name)
                    .collect();
                // If a specific base was requested but not found, return error
                if filtered.is_empty() {
                    return Err(GikError::BaseNotFound(base_name.clone()));
                }
                filtered
            }
            None => all_bases,
        };

        // Compute aggregated totals
        let total_documents: u64 = bases.iter().map(|b| b.documents).sum();
        let total_vectors: u64 = bases.iter().map(|b| b.vectors).sum();
        let total_on_disk_bytes: u64 = bases.iter().map(|b| b.on_disk_bytes).sum();

        // Load stack stats from stats.json
        let stack_stats_path = workspace.stack_stats_path(branch.as_str());
        let stack = crate::stack::read_stats_json(&stack_stats_path).ok().flatten();

        Ok(StatsReport {
            branch: branch.to_string(),
            bases,
            stack,
            total_documents,
            total_vectors,
            total_on_disk_bytes,
        })
    }

    /// Get revision history (timeline) for a branch.
    ///
    /// Returns the list of revisions from the timeline, ordered by timestamp.
    ///
    /// # Arguments
    ///
    /// * `workspace` - The workspace to get history for.
    /// * `branch` - The branch to get history for.
    ///
    /// # Returns
    ///
    /// A vector of revisions from the timeline.
    ///
    /// # Deprecated
    ///
    /// Consider using [`Self::log_query`] for more flexible filtering.
    pub fn log(
        &self,
        workspace: &Workspace,
        branch: &BranchName,
    ) -> Result<Vec<Revision>, GikError> {
        if !workspace.is_initialized() {
            return Ok(vec![]);
        }

        let timeline_path = workspace.timeline_path(branch.as_str());
        read_timeline(&timeline_path)
    }

    /// Query the knowledge log with filtering support.
    ///
    /// This is the preferred way to query GIK's history, supporting:
    /// - Timeline entries (commits, reindex operations, releases)
    /// - Ask log entries (query history)
    /// - Filtering by operation type, base, time range, and more
    ///
    /// # Arguments
    ///
    /// * `workspace` - The workspace to query.
    /// * `scope` - Query scope and filters (see [`crate::log::LogQueryScope`]).
    ///
    /// # Returns
    ///
    /// A [`crate::log::LogQueryResult`] containing matching entries.
    ///
    /// # Example
    ///
    /// ```ignore
    /// use gik_core::log::{LogQueryScope, LogKind, TimelineOperationKind};
    ///
    /// let scope = LogQueryScope::new()
    ///     .with_kind(LogKind::Timeline)
    ///     .with_ops(vec![TimelineOperationKind::Commit])
    ///     .with_limit(10);
    ///
    /// let result = engine.log_query(&workspace, scope)?;
    /// for entry in result.entries {
    ///     println!("{:?}", entry);
    /// }
    /// ```
    pub fn log_query(
        &self,
        workspace: &Workspace,
        scope: crate::log::LogQueryScope,
    ) -> Result<crate::log::LogQueryResult, GikError> {
        crate::log::run_log_query(self, workspace, scope)
    }

    /// Inspect a single knowledge revision (similar to `git show`).
    ///
    /// Shows revision metadata, base impacts, KG summaries, and sources
    /// for a specific revision in the timeline.
    ///
    /// # Arguments
    ///
    /// * `workspace` - The workspace to inspect.
    /// * `opts` - Options including revision reference and output limits.
    ///
    /// # Returns
    ///
    /// A [`ShowReport`] containing detailed information about the revision.
    ///
    /// # Errors
    ///
    /// Returns [`GikError::NotInitialized`] if the workspace is not initialized.
    /// Returns [`GikError::RevisionNotFound`] if the revision reference cannot be resolved.
    ///
    /// # Example
    ///
    /// ```ignore
    /// use gik_core::{GikEngine, ShowOptions};
    ///
    /// let engine = GikEngine::with_defaults()?;
    /// let workspace = engine.resolve_workspace(Path::new("."))?;
    ///
    /// // Show HEAD
    /// let report = engine.show(&workspace, ShowOptions::default())?;
    /// println!("{}", report.render_text());
    ///
    /// // Show a specific revision
    /// let opts = ShowOptions::new().with_revision_ref("HEAD~1");
    /// let report = engine.show(&workspace, opts)?;
    /// ```
    pub fn show(
        &self,
        workspace: &Workspace,
        opts: crate::show::ShowOptions,
    ) -> Result<crate::show::ShowReport, GikError> {
        if !workspace.is_initialized() {
            return Err(GikError::NotInitialized);
        }

        // Determine branch (use current branch if not specified)
        let branch = match &opts.branch {
            Some(b) => BranchName::try_new(b)?,
            None => self.current_branch(workspace)?,
        };

        crate::show::run_show(workspace, branch.as_str(), opts)
    }

    /// Generate a release (CHANGELOG.md generation).
    ///
    /// Creates or overwrites CHANGELOG.md from the timeline by:
    /// 1. Gathering Commit revisions in the specified range
    /// 2. Parsing conventional commit messages
    /// 3. Grouping entries by type (feat, fix, etc.)
    /// 4. Rendering and writing CHANGELOG.md
    ///
    /// **Note:** This does NOT mutate the timeline. No `RevisionOperation::Release`
    /// is appended.
    ///
    /// # Arguments
    ///
    /// * `workspace` - The workspace to create a release for.
    /// * `opts` - Options including release tag, branch, and range.
    ///
    /// # Errors
    ///
    /// Returns [`GikError::NotInitialized`] if the workspace is not initialized.
    pub fn release(
        &self,
        workspace: &Workspace,
        opts: ReleaseOptions,
    ) -> Result<ReleaseResult, GikError> {
        if !workspace.is_initialized() {
            return Err(GikError::NotInitialized);
        }

        // Determine branch (use current branch if not specified)
        let branch = match &opts.branch {
            Some(b) => BranchName::try_new(b)?,
            None => self.current_branch(workspace)?,
        };

        // Run the release pipeline
        release::run_release(workspace, &branch, &opts)
    }

    // -------------------------------------------------------------------------
    // Timeline helpers
    // -------------------------------------------------------------------------

    /// Get the current HEAD revision ID for a branch.
    ///
    /// Returns the revision ID stored in the branch's HEAD file, or `None` if
    /// no HEAD file exists.
    ///
    /// # Arguments
    ///
    /// * `workspace` - The workspace to read HEAD from.
    /// * `branch` - The branch to read HEAD for.
    ///
    /// # Errors
    ///
    /// Returns [`GikError::HeadRead`] if the HEAD file exists but cannot be read.
    pub fn get_head(
        &self,
        workspace: &Workspace,
        branch: &BranchName,
    ) -> Result<Option<RevisionId>, GikError> {
        let head_path = workspace.head_path(branch.as_str());
        read_head(&head_path)
    }

    /// Set the HEAD revision ID for a branch.
    ///
    /// Atomically writes the revision ID to the branch's HEAD file.
    ///
    /// # Arguments
    ///
    /// * `workspace` - The workspace to write HEAD to.
    /// * `branch` - The branch to write HEAD for.
    /// * `revision_id` - The revision ID to set as HEAD.
    ///
    /// # Errors
    ///
    /// Returns [`GikError::HeadWrite`] if the HEAD file cannot be written.
    pub fn set_head(
        &self,
        workspace: &Workspace,
        branch: &BranchName,
        revision_id: &RevisionId,
    ) -> Result<(), GikError> {
        let head_path = workspace.head_path(branch.as_str());
        write_head(&head_path, revision_id)
    }

    /// Get the last (most recent) revision for a branch.
    ///
    /// Returns the most recently appended revision from the timeline, or `None`
    /// if the timeline is empty.
    ///
    /// # Arguments
    ///
    /// * `workspace` - The workspace to read from.
    /// * `branch` - The branch to get the last revision for.
    ///
    /// # Errors
    ///
    /// Returns an error if the timeline cannot be read.
    pub fn get_last_revision(
        &self,
        workspace: &Workspace,
        branch: &BranchName,
    ) -> Result<Option<Revision>, GikError> {
        let timeline_path = workspace.timeline_path(branch.as_str());
        last_revision(&timeline_path)
    }

    /// Append a new revision to the timeline and update HEAD.
    ///
    /// This is the primary method for creating new revisions. It appends the
    /// revision to the timeline and atomically updates HEAD to point to it.
    ///
    /// # Arguments
    ///
    /// * `workspace` - The workspace to write to.
    /// * `branch` - The branch to append the revision to.
    /// * `revision` - The revision to append.
    ///
    /// # Errors
    ///
    /// Returns an error if the timeline or HEAD cannot be written.
    pub fn append_revision_and_update_head(
        &self,
        workspace: &Workspace,
        branch: &BranchName,
        revision: &Revision,
    ) -> Result<(), GikError> {
        let timeline_path = workspace.timeline_path(branch.as_str());
        let head_path = workspace.head_path(branch.as_str());

        append_revision(&timeline_path, revision)?;
        write_head(&head_path, &revision.id)?;

        Ok(())
    }

    // -------------------------------------------------------------------------
    // Configuration management
    // -------------------------------------------------------------------------

    /// Validate configuration files and return detailed results.
    ///
    /// Checks all configuration sources (global, project) for:
    /// - Parse errors (YAML syntax)
    /// - Semantic errors (invalid values)
    /// - Warnings (suboptimal settings)
    ///
    /// # Arguments
    ///
    /// * `workspace` - The workspace to check for project config.
    ///
    /// # Returns
    ///
    /// A [`ConfigValidationResult`] with source info, warnings, and errors.
    pub fn validate_config(
        &self,
        workspace: &Workspace,
    ) -> Result<crate::types::ConfigValidationResult, GikError> {
        use crate::types::{ConfigSourceInfo, ConfigValidationResult};

        let mut result = ConfigValidationResult::new();

        // Check global config
        let global_path = GlobalConfig::default_path().unwrap_or_default();
        let global_source = ConfigSourceInfo {
            name: "global".to_string(),
            path: global_path.clone(),
            exists: global_path.exists(),
            valid: true, // Already loaded successfully
            error: None,
        };
        result.sources.push(global_source);

        // Validate global config values
        result.warnings.extend(self.global_config.validate()?);

        // Check project config
        let project_path = ProjectConfig::config_path_for_workspace(workspace.root());
        let project_exists = project_path.exists();

        let mut project_valid = true;
        let mut project_error = None;

        if project_exists {
            match ProjectConfig::load_from_workspace(workspace.root()) {
                Ok(project_config) => {
                    // Validate project config values
                    result.warnings.extend(project_config.validate()?);
                }
                Err(e) => {
                    project_valid = false;
                    project_error = Some(e.to_string());
                    result.errors.push(format!("Project config: {}", e));
                }
            }
        }

        let project_source = ConfigSourceInfo {
            name: "project".to_string(),
            path: project_path,
            exists: project_exists,
            valid: project_valid,
            error: project_error,
        };
        result.sources.push(project_source);

        Ok(result)
    }

    /// Get the resolved configuration (merged from all sources).
    ///
    /// Returns a snapshot of the effective configuration after merging
    /// global and project-level settings.
    ///
    /// # Arguments
    ///
    /// * `workspace` - The workspace to load project config from.
    ///
    /// # Returns
    ///
    /// A [`ResolvedConfig`] with the effective settings.
    pub fn resolved_config(
        &self,
        workspace: &Workspace,
    ) -> Result<crate::types::ResolvedConfig, GikError> {
        use crate::types::ResolvedConfig;

        let project_config = self.load_project_config(workspace)?;

        // Resolve retrieval config with project overrides
        let resolved_retrieval = self.global_config.resolve_retrieval_config(&project_config);

        // Build resolved config snapshot
        let resolved = ResolvedConfig {
            device: format!("{:?}", self.global_config.device),
            embedding: serde_json::to_value(&self.global_config.embedding)
                .unwrap_or(serde_json::Value::Null),
            retrieval: serde_json::to_value(&resolved_retrieval)
                .unwrap_or(serde_json::Value::Null),
            model_paths: serde_json::to_value(&self.global_config.embeddings)
                .unwrap_or(serde_json::Value::Null),
            project_overrides: serde_json::to_value(&project_config.retrieval)
                .unwrap_or(serde_json::Value::Null),
        };

        Ok(resolved)
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

    fn create_engine() -> GikEngine {
        GikEngine::from_global_config(GlobalConfig::default_for_testing()).unwrap()
    }

    // ------------------------------------------------------------------------
    // current_branch tests
    // ------------------------------------------------------------------------

    #[test]
    fn test_current_branch_no_git() {
        let temp = TempDir::new().unwrap();
        fs::create_dir_all(temp.path().join(".guided/knowledge")).unwrap();

        let engine = create_engine();
        let workspace = Workspace::from_root(temp.path()).unwrap();
        let branch = engine.current_branch(&workspace).unwrap();

        assert_eq!(branch.as_str(), "main");
    }

    #[test]
    fn test_current_branch_git_symbolic_ref() {
        let temp = TempDir::new().unwrap();
        fs::create_dir(temp.path().join(".git")).unwrap();
        fs::write(
            temp.path().join(".git/HEAD"),
            "ref: refs/heads/feature/my-branch\n",
        )
        .unwrap();

        let engine = create_engine();
        let workspace = Workspace::from_root(temp.path()).unwrap();
        let branch = engine.current_branch(&workspace).unwrap();

        assert_eq!(branch.as_str(), "feature/my-branch");
    }

    #[test]
    fn test_current_branch_git_detached_head() {
        let temp = TempDir::new().unwrap();
        fs::create_dir(temp.path().join(".git")).unwrap();
        // 40-character hex string = commit hash
        fs::write(
            temp.path().join(".git/HEAD"),
            "abc123def456789012345678901234567890abcd\n",
        )
        .unwrap();

        let engine = create_engine();
        let workspace = Workspace::from_root(temp.path()).unwrap();
        let branch = engine.current_branch(&workspace).unwrap();

        assert_eq!(branch.as_str(), "HEAD");
        assert!(branch.is_detached());
    }

    #[test]
    fn test_current_branch_gik_head_override() {
        let temp = TempDir::new().unwrap();
        fs::create_dir(temp.path().join(".git")).unwrap();
        fs::write(temp.path().join(".git/HEAD"), "ref: refs/heads/main\n").unwrap();

        // Create GIK HEAD override
        fs::create_dir_all(temp.path().join(".guided/knowledge")).unwrap();
        fs::write(
            temp.path().join(".guided/knowledge/HEAD"),
            "custom-branch\n",
        )
        .unwrap();

        let engine = create_engine();
        let workspace = Workspace::from_root(temp.path()).unwrap();
        let branch = engine.current_branch(&workspace).unwrap();

        // GIK HEAD takes precedence
        assert_eq!(branch.as_str(), "custom-branch");
    }

    #[test]
    fn test_current_branch_gik_head_empty_falls_through() {
        let temp = TempDir::new().unwrap();
        fs::create_dir(temp.path().join(".git")).unwrap();
        fs::write(temp.path().join(".git/HEAD"), "ref: refs/heads/develop\n").unwrap();

        // Create empty GIK HEAD (should be ignored)
        fs::create_dir_all(temp.path().join(".guided/knowledge")).unwrap();
        fs::write(temp.path().join(".guided/knowledge/HEAD"), "  \n").unwrap();

        let engine = create_engine();
        let workspace = Workspace::from_root(temp.path()).unwrap();
        let branch = engine.current_branch(&workspace).unwrap();

        // Falls through to Git HEAD
        assert_eq!(branch.as_str(), "develop");
    }

    // ------------------------------------------------------------------------
    // list_branches tests
    // ------------------------------------------------------------------------

    #[test]
    fn test_list_branches() {
        let temp = TempDir::new().unwrap();
        fs::create_dir_all(temp.path().join(".guided/knowledge/main")).unwrap();
        fs::create_dir_all(temp.path().join(".guided/knowledge/develop")).unwrap();

        let engine = create_engine();
        let workspace = Workspace::from_root(temp.path()).unwrap();
        let branches = engine.list_branches(&workspace).unwrap();

        assert_eq!(branches.len(), 2);
        assert!(branches.iter().any(|b| b.as_str() == "main"));
        assert!(branches.iter().any(|b| b.as_str() == "develop"));
    }

    #[test]
    fn test_branch_exists() {
        let temp = TempDir::new().unwrap();
        fs::create_dir_all(temp.path().join(".guided/knowledge/main")).unwrap();

        let engine = create_engine();
        let workspace = Workspace::from_root(temp.path()).unwrap();

        let main = BranchName::try_new("main").unwrap();
        let other = BranchName::try_new("other").unwrap();

        assert!(engine.branch_exists(&workspace, &main));
        assert!(!engine.branch_exists(&workspace, &other));
    }

    // ------------------------------------------------------------------------
    // init_workspace tests
    // ------------------------------------------------------------------------

    #[test]
    fn test_init_workspace_creates_structure() {
        let temp = TempDir::new().unwrap();

        let engine = create_engine();
        let workspace = Workspace::from_root(temp.path()).unwrap();

        // First init should succeed
        let (revision_id, _stack_stats) = engine.init_workspace(&workspace).unwrap();

        // Stack stats should be populated (verify it was returned)

        // Check that directories were created
        assert!(temp.path().join(".guided").exists());
        assert!(temp.path().join(".guided/knowledge").exists());
        assert!(temp.path().join(".guided/knowledge/main").exists());
        // Knowledge bases are under bases/ subdirectory
        assert!(temp
            .path()
            .join(".guided/knowledge/main/bases/code")
            .exists());
        assert!(temp
            .path()
            .join(".guided/knowledge/main/bases/docs")
            .exists());
        assert!(temp
            .path()
            .join(".guided/knowledge/main/bases/memory")
            .exists());
        // Special directories are at branch level
        assert!(temp.path().join(".guided/knowledge/main/stack").exists());
        assert!(temp.path().join(".guided/knowledge/main/staging").exists());

        // Check that timeline and HEAD were created
        assert!(temp
            .path()
            .join(".guided/knowledge/main/timeline.jsonl")
            .exists());
        assert!(temp.path().join(".guided/knowledge/main/HEAD").exists());

        // Check HEAD contains the revision ID
        let head_content =
            fs::read_to_string(temp.path().join(".guided/knowledge/main/HEAD")).unwrap();
        assert_eq!(head_content.trim(), revision_id.to_string());
    }

    #[test]
    fn test_init_workspace_idempotent() {
        let temp = TempDir::new().unwrap();

        let engine = create_engine();
        let workspace = Workspace::from_root(temp.path()).unwrap();

        // First init should succeed
        let (first_revision_id, _) = engine.init_workspace(&workspace).unwrap();

        // Second init should return AlreadyInitialized
        let result = engine.init_workspace(&workspace);
        match result {
            Err(GikError::AlreadyInitialized { branch, head }) => {
                assert_eq!(branch, "main");
                assert_eq!(head, first_revision_id.to_string());
            }
            Ok(_) => panic!("Expected AlreadyInitialized error"),
            Err(e) => panic!("Expected AlreadyInitialized error, got: {}", e),
        }

        // Timeline should still have only one revision
        let timeline_path = temp.path().join(".guided/knowledge/main/timeline.jsonl");
        let content = fs::read_to_string(&timeline_path).unwrap();
        let lines: Vec<&str> = content.lines().collect();
        assert_eq!(lines.len(), 1, "Timeline should have exactly one revision");
    }

    #[test]
    fn test_init_workspace_different_branch() {
        let temp = TempDir::new().unwrap();

        // Create a Git repo with a specific branch
        fs::create_dir(temp.path().join(".git")).unwrap();
        fs::write(temp.path().join(".git/HEAD"), "ref: refs/heads/feature-x\n").unwrap();

        let engine = create_engine();
        let workspace = Workspace::from_root(temp.path()).unwrap();

        // Init for feature-x branch
        let (revision_id, _) = engine.init_workspace(&workspace).unwrap();

        // Check that the correct branch directory was created
        assert!(temp.path().join(".guided/knowledge/feature-x").exists());
        assert!(temp
            .path()
            .join(".guided/knowledge/feature-x/HEAD")
            .exists());

        // HEAD should contain the revision ID
        let head_content =
            fs::read_to_string(temp.path().join(".guided/knowledge/feature-x/HEAD")).unwrap();
        assert_eq!(head_content.trim(), revision_id.to_string());
    }

    #[test]
    fn test_init_workspace_new_branch_existing_workspace() {
        let temp = TempDir::new().unwrap();

        // Create a Git repo on main branch
        fs::create_dir(temp.path().join(".git")).unwrap();
        fs::write(temp.path().join(".git/HEAD"), "ref: refs/heads/main\n").unwrap();

        let engine = create_engine();
        let workspace = Workspace::from_root(temp.path()).unwrap();

        // Init on main
        let (main_revision, _) = engine.init_workspace(&workspace).unwrap();

        // Switch to a different branch
        fs::write(temp.path().join(".git/HEAD"), "ref: refs/heads/develop\n").unwrap();

        // Init on develop should succeed (different branch)
        let (develop_revision, _) = engine.init_workspace(&workspace).unwrap();

        // Both branches should exist
        assert!(temp.path().join(".guided/knowledge/main/HEAD").exists());
        assert!(temp.path().join(".guided/knowledge/develop/HEAD").exists());

        // Revisions should be different
        assert_ne!(main_revision.to_string(), develop_revision.to_string());
    }

    // ------------------------------------------------------------------------
    // Staging API tests
    // ------------------------------------------------------------------------

    #[test]
    fn test_add_pending_source() {
        let temp = TempDir::new().unwrap();

        // Create a source file
        fs::write(temp.path().join("main.rs"), "fn main() {}").unwrap();

        let engine = create_engine();
        let workspace = Workspace::from_root(temp.path()).unwrap();

        // Initialize first
        engine.init_workspace(&workspace).unwrap();

        // Re-load workspace to pick up initialized state
        let workspace = Workspace::from_root(temp.path()).unwrap();
        let branch = engine.current_branch(&workspace).unwrap();

        // Add a pending source
        let new_source = crate::staging::NewPendingSource::from_uri("main.rs");
        let id = engine
            .add_pending_source(&workspace, &branch, new_source)
            .unwrap();

        // Verify source was added
        let sources = engine.list_pending_sources(&workspace, &branch).unwrap();
        assert_eq!(sources.len(), 1);
        assert_eq!(sources[0].id.as_str(), id.as_str());
        assert_eq!(sources[0].uri, "main.rs");
        assert_eq!(sources[0].base, "code");
    }

    #[test]
    fn test_list_pending_sources_empty() {
        let temp = TempDir::new().unwrap();

        let engine = create_engine();
        let workspace = Workspace::from_root(temp.path()).unwrap();

        engine.init_workspace(&workspace).unwrap();

        // Re-load workspace to pick up initialized state
        let workspace = Workspace::from_root(temp.path()).unwrap();
        let branch = engine.current_branch(&workspace).unwrap();

        // Should be empty initially
        let sources = engine.list_pending_sources(&workspace, &branch).unwrap();
        assert!(sources.is_empty());
    }

    #[test]
    fn test_staging_summary() {
        let temp = TempDir::new().unwrap();

        fs::write(temp.path().join("lib.rs"), "pub fn lib() {}").unwrap();
        fs::write(temp.path().join("README.md"), "# README").unwrap();

        let engine = create_engine();
        let workspace = Workspace::from_root(temp.path()).unwrap();

        engine.init_workspace(&workspace).unwrap();

        // Re-load workspace to pick up initialized state
        let workspace = Workspace::from_root(temp.path()).unwrap();
        let branch = engine.current_branch(&workspace).unwrap();

        // Add sources to different bases
        engine
            .add_pending_source(
                &workspace,
                &branch,
                crate::staging::NewPendingSource::from_uri("lib.rs"),
            )
            .unwrap();

        engine
            .add_pending_source(
                &workspace,
                &branch,
                crate::staging::NewPendingSource::from_uri("README.md"),
            )
            .unwrap();

        // Check summary
        let summary = engine.staging_summary(&workspace, &branch).unwrap();
        assert_eq!(summary.pending_count, 2);
        assert_eq!(summary.by_base.get("code"), Some(&1));
        assert_eq!(summary.by_base.get("docs"), Some(&1));
    }

    #[test]
    fn test_status_shows_staged_count() {
        let temp = TempDir::new().unwrap();

        fs::write(temp.path().join("test.rs"), "fn test() {}").unwrap();

        let engine = create_engine();
        let workspace = Workspace::from_root(temp.path()).unwrap();

        engine.init_workspace(&workspace).unwrap();

        // Re-load workspace to pick up initialized state
        let workspace = Workspace::from_root(temp.path()).unwrap();

        let branch = engine.current_branch(&workspace).unwrap();

        // Initially no staged sources
        let status = engine.status(&workspace, &branch).unwrap();
        assert_eq!(status.staging.map(|s| s.pending_count).unwrap_or(0), 0);

        // Add a source
        engine
            .add_pending_source(
                &workspace,
                &branch,
                crate::staging::NewPendingSource::from_uri("test.rs"),
            )
            .unwrap();

        // Now status should show staged count
        let status = engine.status(&workspace, &branch).unwrap();
        assert_eq!(status.staging.map(|s| s.pending_count).unwrap_or(0), 1);
    }

    // ------------------------------------------------------------------------
    // add tests
    // ------------------------------------------------------------------------

    #[test]
    fn test_add_local_file() {
        let temp = TempDir::new().unwrap();
        fs::write(temp.path().join("src.rs"), "fn main() {}").unwrap();

        let engine = create_engine();
        let workspace = Workspace::from_root(temp.path()).unwrap();
        engine.init_workspace(&workspace).unwrap();

        let workspace = Workspace::from_root(temp.path()).unwrap();
        let branch = engine.current_branch(&workspace).unwrap();

        let opts = AddOptions {
            targets: vec!["src.rs".to_string()],
            base: None,
        };
        let result = engine.add(&workspace, &branch, opts).unwrap();

        assert_eq!(result.created.len(), 1);
        assert!(result.skipped.is_empty());
        // Stack scanning now happens during commit, not add
        assert!(result.stack_stats.is_none());
    }

    #[test]
    fn test_add_directory() {
        let temp = TempDir::new().unwrap();
        fs::create_dir(temp.path().join("src")).unwrap();
        fs::write(temp.path().join("src/lib.rs"), "pub fn foo() {}").unwrap();

        let engine = create_engine();
        let workspace = Workspace::from_root(temp.path()).unwrap();
        engine.init_workspace(&workspace).unwrap();

        let workspace = Workspace::from_root(temp.path()).unwrap();
        let branch = engine.current_branch(&workspace).unwrap();

        let opts = AddOptions {
            targets: vec!["src".to_string()],
            base: None,
        };
        let result = engine.add(&workspace, &branch, opts).unwrap();

        assert_eq!(result.created.len(), 1);
        assert!(result.skipped.is_empty());
    }

    #[test]
    fn test_add_url() {
        let temp = TempDir::new().unwrap();

        let engine = create_engine();
        let workspace = Workspace::from_root(temp.path()).unwrap();
        engine.init_workspace(&workspace).unwrap();

        let workspace = Workspace::from_root(temp.path()).unwrap();
        let branch = engine.current_branch(&workspace).unwrap();

        let opts = AddOptions {
            targets: vec!["https://example.com/docs".to_string()],
            base: None,
        };
        let result = engine.add(&workspace, &branch, opts).unwrap();

        assert_eq!(result.created.len(), 1);
        assert!(result.skipped.is_empty());

        // Verify URL is stored as-is
        let pending = engine.list_pending_sources(&workspace, &branch).unwrap();
        assert_eq!(pending[0].uri, "https://example.com/docs");
        assert_eq!(pending[0].base, "docs"); // URLs default to docs base
    }

    #[test]
    fn test_add_with_explicit_base() {
        let temp = TempDir::new().unwrap();
        fs::write(temp.path().join("README.md"), "# Hello").unwrap();

        let engine = create_engine();
        let workspace = Workspace::from_root(temp.path()).unwrap();
        engine.init_workspace(&workspace).unwrap();

        let workspace = Workspace::from_root(temp.path()).unwrap();
        let branch = engine.current_branch(&workspace).unwrap();

        let opts = AddOptions {
            targets: vec!["README.md".to_string()],
            base: Some("custom".to_string()), // Override inferred base
        };
        let result = engine.add(&workspace, &branch, opts).unwrap();

        assert_eq!(result.created.len(), 1);

        let pending = engine.list_pending_sources(&workspace, &branch).unwrap();
        assert_eq!(pending[0].base, "custom");
    }

    #[test]
    fn test_add_missing_path_skipped() {
        let temp = TempDir::new().unwrap();

        let engine = create_engine();
        let workspace = Workspace::from_root(temp.path()).unwrap();
        engine.init_workspace(&workspace).unwrap();

        let workspace = Workspace::from_root(temp.path()).unwrap();
        let branch = engine.current_branch(&workspace).unwrap();

        let opts = AddOptions {
            targets: vec!["nonexistent.rs".to_string()],
            base: None,
        };
        let result = engine.add(&workspace, &branch, opts).unwrap();

        assert!(result.created.is_empty());
        assert_eq!(result.skipped.len(), 1);
        assert!(
            result.skipped[0].reason.contains("not found"),
            "Expected 'not found' in reason, got: {}",
            result.skipped[0].reason
        );
    }

    #[test]
    fn test_add_duplicate_skipped() {
        let temp = TempDir::new().unwrap();
        fs::write(temp.path().join("main.rs"), "fn main() {}").unwrap();

        let engine = create_engine();
        let workspace = Workspace::from_root(temp.path()).unwrap();
        engine.init_workspace(&workspace).unwrap();

        let workspace = Workspace::from_root(temp.path()).unwrap();
        let branch = engine.current_branch(&workspace).unwrap();

        // First add
        let opts = AddOptions {
            targets: vec!["main.rs".to_string()],
            base: None,
        };
        let result1 = engine.add(&workspace, &branch, opts).unwrap();
        assert_eq!(result1.created.len(), 1);

        // Second add of same file
        let opts = AddOptions {
            targets: vec!["main.rs".to_string()],
            base: None,
        };
        let result2 = engine.add(&workspace, &branch, opts).unwrap();

        assert!(result2.created.is_empty());
        assert_eq!(result2.skipped.len(), 1);
        assert!(result2.skipped[0].reason.contains("already pending"));
    }

    #[test]
    fn test_add_multiple_targets() {
        let temp = TempDir::new().unwrap();
        fs::write(temp.path().join("main.rs"), "fn main() {}").unwrap();
        fs::write(temp.path().join("lib.rs"), "pub mod foo;").unwrap();
        fs::write(temp.path().join("README.md"), "# Docs").unwrap();

        let engine = create_engine();
        let workspace = Workspace::from_root(temp.path()).unwrap();
        engine.init_workspace(&workspace).unwrap();

        let workspace = Workspace::from_root(temp.path()).unwrap();
        let branch = engine.current_branch(&workspace).unwrap();

        let opts = AddOptions {
            targets: vec![
                "main.rs".to_string(),
                "lib.rs".to_string(),
                "README.md".to_string(),
                "missing.txt".to_string(), // Should be skipped
            ],
            base: None,
        };
        let result = engine.add(&workspace, &branch, opts).unwrap();

        assert_eq!(result.created.len(), 3);
        assert_eq!(result.skipped.len(), 1);
        // Stack scanning now happens during commit, not add
        assert!(result.stack_stats.is_none());
    }

    #[test]
    fn test_add_archive_by_extension() {
        let temp = TempDir::new().unwrap();
        // Create a dummy file with archive extension
        fs::write(temp.path().join("data.zip"), "fake zip").unwrap();

        let engine = create_engine();
        let workspace = Workspace::from_root(temp.path()).unwrap();
        engine.init_workspace(&workspace).unwrap();

        let workspace = Workspace::from_root(temp.path()).unwrap();
        let branch = engine.current_branch(&workspace).unwrap();

        let opts = AddOptions {
            targets: vec!["data.zip".to_string()],
            base: None,
        };
        let result = engine.add(&workspace, &branch, opts).unwrap();

        assert_eq!(result.created.len(), 1);

        let pending = engine.list_pending_sources(&workspace, &branch).unwrap();
        assert_eq!(pending[0].kind, crate::staging::PendingSourceKind::Archive);
    }

    // ------------------------------------------------------------------------
    // Enhanced status tests
    // ------------------------------------------------------------------------

    #[test]
    fn test_status_uninitialized_workspace() {
        let temp = TempDir::new().unwrap();

        let engine = create_engine();
        let workspace = Workspace::from_root(temp.path()).unwrap();
        let branch = BranchName::default_branch();

        // Status of uninitialized workspace
        let status = engine.status(&workspace, &branch).unwrap();

        assert!(!status.is_initialized);
        assert!(status.head.is_none());
        assert!(status.staging.is_none());
        assert!(status.stack.is_none());
        assert_eq!(status.workspace_root, workspace.root());
        assert_eq!(status.branch.as_str(), "main");
    }

    #[test]
    fn test_status_after_init_shows_head() {
        let temp = TempDir::new().unwrap();

        let engine = create_engine();
        let workspace = Workspace::from_root(temp.path()).unwrap();

        engine.init_workspace(&workspace).unwrap();

        // Re-load workspace to pick up initialized state
        let workspace = Workspace::from_root(temp.path()).unwrap();
        let branch = engine.current_branch(&workspace).unwrap();

        let status = engine.status(&workspace, &branch).unwrap();

        assert!(status.is_initialized);
        assert!(status.head.is_some());

        let head = status.head.unwrap();
        assert!(!head.revision_id.is_empty());
        assert!(matches!(
            head.operation,
            crate::timeline::RevisionOperation::Init
        ));
        assert!(head.message.is_some());
    }

    #[test]
    fn test_status_shows_stack_stats() {
        let temp = TempDir::new().unwrap();
        fs::write(temp.path().join("main.rs"), "fn main() {}").unwrap();
        fs::write(temp.path().join("lib.rs"), "pub mod test;").unwrap();

        let engine = create_engine();
        let workspace = Workspace::from_root(temp.path()).unwrap();
        engine.init_workspace(&workspace).unwrap();

        let workspace = Workspace::from_root(temp.path()).unwrap();
        let branch = engine.current_branch(&workspace).unwrap();

        let status = engine.status(&workspace, &branch).unwrap();

        assert!(status.stack.is_some());
        let stack = status.stack.unwrap();
        assert!(stack.total_files >= 2);
        assert!(stack.languages.contains_key("rust"));
    }

    #[test]
    fn test_status_serialization_json() {
        let temp = TempDir::new().unwrap();
        fs::write(temp.path().join("test.rs"), "fn test() {}").unwrap();

        let engine = create_engine();
        let workspace = Workspace::from_root(temp.path()).unwrap();
        engine.init_workspace(&workspace).unwrap();

        let workspace = Workspace::from_root(temp.path()).unwrap();
        let branch = engine.current_branch(&workspace).unwrap();

        let status = engine.status(&workspace, &branch).unwrap();
        let json = serde_json::to_string(&status).unwrap();

        // Verify camelCase keys
        assert!(json.contains("\"workspaceRoot\""));
        assert!(json.contains("\"isInitialized\""));
        assert!(json.contains("\"head\""));
        // Stack and staging should be present (not skipped since they have data)
        assert!(json.contains("\"stack\""));
    }

    // ------------------------------------------------------------------------
    // Embedding configuration tests (Phase 4.1)
    // ------------------------------------------------------------------------

    #[test]
    fn test_base_root_path() {
        let temp = TempDir::new().unwrap();
        let engine = create_engine();
        let workspace = Workspace::from_root(temp.path()).unwrap();

        let path = engine.base_root(&workspace, "main", "code");
        assert!(path.ends_with(".guided/knowledge/main/bases/code"));
    }

    #[test]
    fn test_model_info_path() {
        let temp = TempDir::new().unwrap();
        let engine = create_engine();
        let workspace = Workspace::from_root(temp.path()).unwrap();

        let path = engine.model_info_path(&workspace, "main", "code");
        assert!(path.ends_with(".guided/knowledge/main/bases/code/model-info.json"));
    }

    #[test]
    fn test_embedding_config_for_base_default() {
        let temp = TempDir::new().unwrap();
        let engine = create_engine();
        let workspace = Workspace::from_root(temp.path()).unwrap();

        let config = engine.embedding_config_for_base(&workspace, "code");

        // Should use default Candle + MiniLM config
        assert_eq!(
            config.provider,
            crate::embedding::EmbeddingProviderKind::Candle
        );
        assert_eq!(config.model_id.as_str(), crate::embedding::DEFAULT_MODEL_ID);
    }

    #[test]
    fn test_load_model_info_missing() {
        let temp = TempDir::new().unwrap();
        let engine = create_engine();
        let workspace = Workspace::from_root(temp.path()).unwrap();

        // Model-info doesn't exist
        let result = engine.load_model_info(&workspace, "main", "code").unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn test_load_model_info_exists() {
        let temp = TempDir::new().unwrap();
        let engine = create_engine();
        let workspace = Workspace::from_root(temp.path()).unwrap();

        // Create model-info file
        let base_dir = temp.path().join(".guided/knowledge/main/bases/code");
        fs::create_dir_all(&base_dir).unwrap();

        let model_info = crate::embedding::ModelInfo::new("candle", "test-model", 384);
        crate::embedding::write_model_info(&base_dir.join("model-info.json"), &model_info).unwrap();

        let result = engine.load_model_info(&workspace, "main", "code").unwrap();
        assert!(result.is_some());
        let info = result.unwrap();
        assert_eq!(info.provider, "candle");
        assert_eq!(info.model_id, "test-model");
    }

    #[test]
    fn test_model_compatibility_missing() {
        let temp = TempDir::new().unwrap();
        let engine = create_engine();
        let workspace = Workspace::from_root(temp.path()).unwrap();

        let compatibility = engine
            .model_compatibility(&workspace, "main", "code")
            .unwrap();

        assert!(matches!(
            compatibility,
            crate::embedding::ModelCompatibility::MissingModelInfo
        ));
    }

    #[test]
    fn test_model_compatibility_compatible() {
        let temp = TempDir::new().unwrap();
        let engine = create_engine();
        let workspace = Workspace::from_root(temp.path()).unwrap();

        // Create model-info matching the default config
        let base_dir = temp.path().join(".guided/knowledge/main/bases/code");
        fs::create_dir_all(&base_dir).unwrap();

        let model_info =
            crate::embedding::ModelInfo::new("candle", crate::embedding::DEFAULT_MODEL_ID, 384);
        crate::embedding::write_model_info(&base_dir.join("model-info.json"), &model_info).unwrap();

        let compatibility = engine
            .model_compatibility(&workspace, "main", "code")
            .unwrap();

        assert!(matches!(
            compatibility,
            crate::embedding::ModelCompatibility::Compatible
        ));
    }

    #[test]
    fn test_model_compatibility_mismatch() {
        let temp = TempDir::new().unwrap();
        let engine = create_engine();
        let workspace = Workspace::from_root(temp.path()).unwrap();

        // Create model-info with different model
        let base_dir = temp.path().join(".guided/knowledge/main/bases/code");
        fs::create_dir_all(&base_dir).unwrap();

        let model_info = crate::embedding::ModelInfo::new(
            "candle",
            "different-model", // Not the default
            384,
        );
        crate::embedding::write_model_info(&base_dir.join("model-info.json"), &model_info).unwrap();

        let compatibility = engine
            .model_compatibility(&workspace, "main", "code")
            .unwrap();

        assert!(matches!(
            compatibility,
            crate::embedding::ModelCompatibility::Mismatch { .. }
        ));
    }
}
