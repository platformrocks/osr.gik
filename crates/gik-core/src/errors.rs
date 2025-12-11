//! Error types for gik-core.

use std::path::PathBuf;

use thiserror::Error;

/// Domain-specific errors for GIK operations.
#[derive(Error, Debug)]
pub enum GikError {
    /// The workspace has not been initialized with `gik init`.
    #[error("Workspace not initialized. Run `gik init`.")]
    NotInitialized,

    /// Global configuration file not found.
    #[error("Global config not found at {0}")]
    MissingGlobalConfig(String),

    /// Global configuration file is invalid.
    #[error("Global config invalid: {0}")]
    InvalidGlobalConfig(String),

    /// Project configuration is invalid.
    #[error("Project config invalid: {0}")]
    InvalidProjectConfig(String),

    /// A configuration value is invalid.
    ///
    /// Used for validation errors detected at runtime (e.g., batch_size=0).
    #[error("Invalid configuration: {message}. {hint}")]
    InvalidConfiguration {
        /// Description of the invalid configuration.
        message: String,
        /// Actionable hint on how to fix it.
        hint: String,
    },

    /// The requested embedding profile does not exist.
    #[error("Embedding profile `{0}` not found.")]
    UnknownEmbeddingProfile(String),

    /// The embedding model used for the index does not match the active model.
    #[error("Embedding model mismatch for base `{base}`: index uses `{index_model}`, active is `{active_model}`.")]
    EmbeddingModelMismatch {
        /// The affected base name.
        base: String,
        /// The model stored in the index.
        index_model: String,
        /// The currently active model.
        active_model: String,
    },

    /// No sources or memory have been staged for commit.
    #[error("No staged sources or memory to commit.")]
    NothingToCommit,

    /// The specified base does not exist.
    #[error("Base `{0}` not found.")]
    BaseNotFound(String),

    /// The specified base exists but has no indexed content.
    ///
    /// This occurs when the base directory exists but contains no sources.jsonl
    /// or the file is empty. Run `gik add` and `gik commit` to index content.
    #[error("Base `{base}` exists but has no indexed content. Run `gik add` and `gik commit` first.")]
    BaseNotIndexed {
        /// The base that has no indexed content.
        base: String,
    },

    /// No pruning policy was configured and none was provided via CLI flags.
    ///
    /// The user must specify at least one pruning constraint (--max-entries,
    /// --max-tokens, or --max-age-days) or configure a policy in config.yaml.
    #[error("No pruning policy configured. Use --max-entries, --max-tokens, or --max-age-days, or configure a policy in config.yaml.")]
    MissingPruningPolicy,

    /// A path or file was not found.
    #[error("Path not found: {0}")]
    PathNotFound(String),

    /// An invalid path was provided (e.g., disk root, system directory).
    #[error("Invalid path: {0}")]
    InvalidPath(String),

    /// Invalid branch name (contains invalid characters or is empty).
    #[error("Invalid branch name `{0}`: branch names must be non-empty and contain only alphanumeric characters, hyphens, underscores, and forward slashes.")]
    InvalidBranchName(String),

    /// Branch detection failed.
    #[error("Failed to detect current branch: {0}")]
    BranchDetectionFailed(String),

    /// Invalid argument provided to a command.
    #[error("{0}")]
    InvalidArgument(String),

    /// Failed to write to the timeline.
    #[error("Failed to write to timeline: {0}")]
    TimelineWrite(String),

    /// Failed to read from the timeline.
    #[error("Failed to read timeline: {0}")]
    TimelineRead(String),

    /// Failed to parse a timeline entry.
    #[error("Failed to parse timeline entry: {0}")]
    TimelineParse(String),

    /// Failed to write HEAD file.
    #[error("Failed to write HEAD: {0}")]
    HeadWrite(String),

    /// Failed to read HEAD file.
    #[error("Failed to read HEAD: {0}")]
    HeadRead(String),

    /// Revision not found in timeline.
    #[error("Revision not found: {0}")]
    RevisionNotFound(String),

    /// The workspace/branch is already initialized.
    #[error("Branch `{branch}` already initialized (HEAD: {head}). Nothing to do.")]
    AlreadyInitialized {
        /// The branch that is already initialized.
        branch: String,
        /// The current HEAD revision ID.
        head: String,
    },

    /// Stack scanning failed.
    #[error("Stack scan failed: {0}")]
    StackScanFailed(String),

    /// Stack persistence failed.
    #[error("Failed to persist stack data: {0}")]
    StackPersistFailed(String),

    /// Staging I/O error.
    #[error("Staging IO error: {0}")]
    StagingIo(String),

    /// Staging parse error.
    #[error("Staging parse error: {0}")]
    StagingParse(String),

    /// Embedding configuration error.
    #[error("Embedding config error: {message}")]
    EmbeddingConfigError {
        /// Description of the configuration error.
        message: String,
    },

    /// Embedding provider is unavailable or not implemented.
    #[error("Embedding provider `{provider}` is unavailable: {reason}")]
    EmbeddingProviderUnavailable {
        /// The provider that is unavailable.
        provider: String,
        /// Reason why the provider is unavailable.
        reason: String,
    },

    /// The model architecture is not supported by the embedding backend.
    #[error("Unsupported model architecture `{architecture}`: {details}")]
    UnsupportedModelArchitecture {
        /// The architecture that is not supported.
        architecture: String,
        /// Details about why it's unsupported.
        details: String,
    },

    /// Failed to read/write model-info file.
    #[error("Model-info I/O error at `{path}`: {message}")]
    EmbeddingModelInfoIo {
        /// Path to the model-info file.
        path: PathBuf,
        /// Description of the I/O error.
        message: String,
    },

    /// Failed to parse model-info file.
    #[error("Model-info parse error at `{path}`: {message}")]
    EmbeddingModelInfoParse {
        /// Path to the model-info file.
        path: PathBuf,
        /// Description of the parse error.
        message: String,
    },

    /// Vector index I/O error.
    #[error("Vector index I/O error at `{path}`: {message}")]
    VectorIndexIo {
        /// Path to the index file or directory.
        path: PathBuf,
        /// Description of the I/O error.
        message: String,
    },

    /// Vector index parse error.
    #[error("Vector index parse error at `{path}`: {message}")]
    VectorIndexParse {
        /// Path to the index file.
        path: PathBuf,
        /// Description of the parse error.
        message: String,
    },

    /// Vector index is incompatible with current configuration.
    #[error("Vector index incompatible for base `{base}`: {reason}")]
    VectorIndexIncompatible {
        /// The affected base name.
        base: String,
        /// Reason for incompatibility.
        reason: String,
    },

    /// Vector index backend is unavailable.
    #[error("Vector index backend `{backend}` is unavailable: {reason}")]
    VectorIndexBackendUnavailable {
        /// The requested backend.
        backend: String,
        /// Reason why the backend is unavailable.
        reason: String,
    },

    // =========================================================================
    // Reranker Errors (Phase 8.2)
    // =========================================================================
    /// Reranker model not found at the specified path.
    #[error("Reranker model not found: {model_id} at `{path}`")]
    RerankerModelNotFound {
        /// The model identifier.
        model_id: String,
        /// Path where the model was expected.
        path: std::path::PathBuf,
    },

    /// Reranker inference failed.
    #[error("Reranker inference failed for model `{model_id}`: {reason}")]
    RerankerInferenceFailed {
        /// The model identifier.
        model_id: String,
        /// Reason for the inference failure.
        reason: String,
    },

    /// Reranker backend is unavailable.
    #[error("Reranker backend is unavailable: {reason}")]
    RerankerBackendUnavailable {
        /// Reason why the backend is unavailable.
        reason: String,
    },

    // =========================================================================
    // Base Store Errors
    // =========================================================================
    /// Base store I/O error.
    #[error("Base store I/O error at `{path}`: {message}")]
    BaseStoreIo {
        /// Path to the base store file.
        path: std::path::PathBuf,
        /// Description of the I/O error.
        message: String,
    },

    /// Base store parse error.
    #[error("Base store parse error at `{path}`: {message}")]
    BaseStoreParse {
        /// Path to the base store file.
        path: std::path::PathBuf,
        /// Description of the parse error.
        message: String,
    },

    // =========================================================================
    // Commit Errors
    // =========================================================================
    /// Commit has no pending sources to process.
    #[error("No pending sources for branch `{branch}`. Nothing to commit.")]
    CommitNoPendingSources {
        /// The branch with no pending sources.
        branch: String,
    },

    /// Commit failed due to embedding incompatibility.
    #[error("Cannot commit base `{base}`: {reason}. Run `gik reindex --base {base}` first.")]
    CommitEmbeddingIncompatible {
        /// The affected base.
        base: String,
        /// Description of the incompatibility.
        reason: String,
    },

    /// Commit failed due to vector index incompatibility.
    #[error("Cannot commit base `{base}`: {reason}. Run `gik reindex --base {base}` first.")]
    CommitIndexIncompatible {
        /// The affected base.
        base: String,
        /// Description of the incompatibility.
        reason: String,
    },

    /// Commit failed during source ingestion.
    #[error("Failed to ingest `{uri}` for base `{base}`: {reason}")]
    CommitIngestionError {
        /// The affected base.
        base: String,
        /// The source URI that failed.
        uri: String,
        /// Description of the failure.
        reason: String,
    },

    // =========================================================================
    // Ask Errors
    // =========================================================================
    /// No indexed bases available for ask query.
    #[error("No indexed knowledge bases found for branch `{branch}`. Run `gik add` and `gik commit` first.")]
    AskNoIndexedBases {
        /// The branch with no indexed bases.
        branch: String,
    },

    /// Failed to embed the ask query.
    #[error("Failed to embed query `{question}`: {reason}")]
    AskEmbeddingError {
        /// The query that failed to embed.
        question: String,
        /// Description of the failure.
        reason: String,
    },

    /// Failed to search vector index during ask.
    #[error("Failed to search base `{base}` for query: {reason}")]
    AskSearchError {
        /// The base that failed to search.
        base: String,
        /// Description of the failure.
        reason: String,
    },

    // =========================================================================
    // Reindex Errors
    // =========================================================================
    /// Reindex failed because base has no sources.
    #[error("Base `{base}` has no sources to reindex.")]
    ReindexNoSources {
        /// The base with no sources.
        base: String,
    },

    /// Reindex failed during embedding.
    #[error("Failed to embed during reindex of base `{base}`: {reason}")]
    ReindexEmbeddingError {
        /// The affected base.
        base: String,
        /// Description of the failure.
        reason: String,
    },

    /// Reindex failed during vector index update.
    #[error("Failed to update vector index during reindex of base `{base}`: {reason}")]
    ReindexIndexError {
        /// The affected base.
        base: String,
        /// Description of the failure.
        reason: String,
    },

    // -------------------------------------------------------------------------
    // Log Errors
    // -------------------------------------------------------------------------
    /// An error occurred while reading or writing a log file.
    #[error("Log I/O error at {path}: {reason}")]
    LogIoError {
        /// The path to the log file.
        path: std::path::PathBuf,
        /// Description of the failure.
        reason: String,
    },

    // -------------------------------------------------------------------------
    // Stats Errors
    // -------------------------------------------------------------------------
    /// The requested base was not found during stats computation.
    #[error("Stats: base `{base}` not found.")]
    StatsBaseNotFound {
        /// The base that was not found.
        base: String,
    },

    /// I/O error during stats computation.
    #[error("Stats I/O error at `{path}`: {reason}")]
    StatsIoError {
        /// The path that caused the error.
        path: std::path::PathBuf,
        /// Description of the failure.
        reason: String,
    },

    /// An I/O error occurred.
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    /// JSON serialization/deserialization error.
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),

    /// YAML serialization/deserialization error.
    #[error("YAML error: {0}")]
    Yaml(#[from] serde_yaml::Error),

    /// A wrapped generic error.
    #[error(transparent)]
    Other(#[from] anyhow::Error),
}
