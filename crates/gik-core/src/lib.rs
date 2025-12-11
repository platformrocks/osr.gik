//! # gik-core
//!
//! **Guided Indexing Kernel** – core engine library.
//!
//! This crate provides the domain logic, storage abstractions, and query engine
//! for GIK. It is designed to be consumed by the `gik` CLI and other Rust tools.
//!
//! ## Main Types
//!
//! - [`GikEngine`] – the main entry point for all GIK operations
//! - [`Workspace`] – represents a resolved GIK workspace on disk
//! - [`GikError`] – domain-specific error type
//!
//! ## Modules
//!
//! - [`config`] – configuration types (GlobalConfig, ProjectConfig)
//! - [`engine`] – the GikEngine implementation
//! - [`errors`] – error types
//! - [`workspace`] – workspace detection and management
//! - [`types`] – common types (BaseName, options, results, traits)
//!
//! ## Example
//!
//! ```ignore
//! use gik_core::{GikEngine, Workspace};
//! use std::path::Path;
//!
//! // Create engine with default configuration
//! let engine = GikEngine::with_defaults()?;
//!
//! // Resolve workspace from current directory
//! let workspace = engine.resolve_workspace(Path::new("."))?;
//!
//! // Initialize if needed
//! if !workspace.is_initialized() {
//!     engine.init_workspace(&workspace)?;
//! }
//!
//! // Get status
//! let branch = engine.current_branch(&workspace)?;
//! let status = engine.status(&workspace, &branch)?;
//! println!("Initialized: {}", status.initialized);
//! ```

// Modules
pub mod ask;
pub mod base;
pub mod bm25;
pub mod commit;
pub mod config;
pub mod constants;
pub mod db_adapter;
pub mod embedding;
pub mod embedding_config_bridge;
pub mod engine;
pub mod errors;
pub mod kg;
pub mod log;
pub mod memory;
pub mod model_adapter;
pub mod query_expansion;
pub mod reindex;
pub mod release;
pub(crate) mod reranker;
pub mod show;
pub mod stack;
pub mod staging;
pub mod status;
pub mod timeline;
pub mod types;
pub mod vector_index;
pub mod workspace;

// Re-exports for convenience
//
// Note on naming:
// - `AskPipelineOptions` (from ask.rs) is the full ask pipeline options with builder pattern
// - `AskOptions` (from types.rs) is a simpler version for backward compatibility
// - `CoreEmbeddingConfig` avoids collision with config::EmbeddingConfig
// - KG filename constants are prefixed with `KG_` to distinguish from base constants

pub use ask::{
    run_ask, AskContextBundle, AskDebugInfo, AskKgResult, AskOptions as AskPipelineOptions,
    MemoryEvent, RagChunk, StackSummary, DEFAULT_TOP_K, RAG_BASES,
};
pub use base::{
    append_base_sources, load_base_sources, load_base_stats, save_base_stats, BaseHealthState,
    BaseSourceEntry, BaseStats, BaseStatsReport, ChunkId, MAX_FILE_LINES, MAX_FILE_SIZE_BYTES,
    SOURCES_FILENAME, STATS_FILENAME,
};
pub use bm25::{
    load_bm25_index, rrf_fusion, save_bm25_index, Bm25Config, Bm25Index, Bm25SearchResult,
    FusedResult, HybridSearchConfig, Tokenizer as Bm25Tokenizer, BM25_DIR_NAME,
};
pub use commit::{run_commit, CommitSummary, CommitSummaryBase};
pub use config::{
    DevicePreference,
    EmbeddingConfig,
    EmbeddingOverride,
    EmbeddingProfileConfig,
    EmbeddingsSection,
    GlobalConfig,
    IndexOverride,
    IndexesSection,
    PerformanceConfig,
    ProjectConfig,
    // Performance constants (Phase 8.1)
    DEFAULT_EMBEDDING_BATCH_SIZE,
    DEFAULT_EMBEDDING_WARMUP,
    DEFAULT_MAX_FILE_LINES,
    DEFAULT_MAX_FILE_SIZE_BYTES,
    DEFAULT_PARALLEL_FILE_READING,
};
pub use constants::{
    is_binary_extension, should_ignore_dir, ALWAYS_IGNORED_DIRS, BINARY_EXTENSIONS, GIK_HOME_DIR,
    GIK_IGNORE_FILENAME, GLOBAL_CONFIG_FILENAME, GUIDED_DIR, KNOWLEDGE_DIR,
    PROJECT_CONFIG_FILENAME,
};
pub use embedding::{
    check_model_compatibility, create_backend, default_embedding_config_for_base, read_model_info,
    write_model_info, BaseEmbeddingConfig, CandleEmbeddingBackend, EmbeddingBackend,
    EmbeddingConfig as CoreEmbeddingConfig, EmbeddingModelId, EmbeddingProviderKind,
    ModelCompatibility, ModelInfo, DEFAULT_DIMENSION, DEFAULT_MAX_TOKENS, DEFAULT_MODEL_ID,
    DEFAULT_MODEL_PATH,
};
pub use engine::GikEngine;
pub use errors::GikError;
pub use kg::{
    build_ask_kg_context, clear_branch_kg, export_kg, export_to_dot, export_to_mermaid,
    init_kg_for_branch, kg_exists, sync_branch_kg, sync_branch_kg_default, DefaultKgExtractor,
    KgEdge, KgExportFormat, KgExportOptions, KgExtractionConfig, KgExtractionResult, KgExtractor,
    KgNode, KgQueryConfig, KgStats, KgSyncResult, RagChunkRef, EDGES_FILENAME as KG_EDGES_FILENAME,
    KG_DIR_NAME, KG_VERSION, NODES_FILENAME as KG_NODES_FILENAME,
    STATS_FILENAME as KG_STATS_FILENAME,
};
pub use log::{
    append_ask_log, run_log_query, AskLogEntry, AskLogView, LogEntry, LogKind, LogQueryResult,
    LogQueryScope, TimelineLogEntry, TimelineOperationKind, ASKS_DIR, ASK_LOG_FILENAME,
};
pub use memory::{
    ingest_memory_entries, MemoryEntry, MemoryIngestionOptions, MemoryIngestionResult, MemoryScope,
    MemorySource, MEMORY_BASE_NAME,
};
pub use query_expansion::{average_embeddings, ExpansionConfig, QueryExpander};
pub use reindex::{reindex_base, run_reindex};
pub use release::{
    gather_release_entries, group_entries_by_kind, render_changelog_markdown, run_release,
    ReleaseEntry, ReleaseEntryKind, ReleaseGroup, ReleaseMode, ReleaseOptions, ReleaseRange,
    ReleaseResult, ReleaseSummary,
};
pub use show::{run_show, BaseImpact, KgImpactSummary, ShowOptions, ShowReport};
pub use stack::{
    StackDependencyEntry, StackFileEntry, StackFileKind, StackInventory, StackStats, StackTechEntry,
};
pub use staging::{
    detect_file_change, get_file_metadata, is_source_already_pending, unstage_sources, ChangeType,
    IndexedFileInfo, NewPendingSource, PendingSource, PendingSourceId, PendingSourceKind,
    PendingSourceStatus, StagingSummary,
};
pub use status::{HeadInfo, StagedFile, StatusReport};
pub use timeline::{resolve_revision_ref, Revision, RevisionId, RevisionOperation};
pub use types::{
    AddOptions, AddResult, AddSourceSkip, AskOptions, BaseName, CommitOptions, CommitResult,
    CommitResultBase, ConfigSourceInfo, ConfigValidationResult, EmbeddingProvider, MemoryIngestResult,
    ReindexBaseResult, ReindexEntry, ReindexOptions, ReindexResult, ResolvedConfig, SearchResult,
    StatsQuery, StatsReport, UnstageOptions, UnstageResult, UnstageSourceSkip, VectorIndex,
};
pub use vector_index::{
    check_index_compatibility, default_vector_index_config_for_base, load_index_meta,
    write_index_meta, VectorId, VectorIndexBackend, VectorIndexBackendKind,
    VectorIndexCompatibility, VectorIndexConfig, VectorIndexMeta, VectorIndexStats, VectorInsert,
    VectorMetric, VectorSearchResult, DEFAULT_BACKEND, DEFAULT_METRIC, INDEX_META_FILENAME,
    INDEX_RECORDS_FILENAME,
};
pub use workspace::{is_valid_branch_name, BranchName, Workspace};

// gik-db adapter - for bridging storage layer (vectors, KG)
pub use db_adapter::{from_db_error, DbKgStore, DbVectorIndex, IntoGikResult};

// gik-model adapter - for bridging ML inference layer (embeddings, reranking)
pub use model_adapter::{
    create_embedding_backend, create_reranker_backend, from_model_error, ModelEmbeddingBackend,
    ModelRerankerBackend,
};

// Embedding config bridge - for unified config resolution
pub use embedding_config_bridge::{
    default_embedding_config, default_embedding_config_with_device, default_reranker_config,
    default_reranker_config_with_device, embedding_config_from_model_id,
    embedding_config_from_path, from_model_device_preference, reranker_config_from_model_id,
    resolve_embedding_config, resolve_reranker_config, resolve_reranker_config_with_defaults,
    to_model_device_preference, to_model_embedding_config, to_model_reranker_config,
    ModelEmbeddingConfig, ModelRerankerConfig,
};
