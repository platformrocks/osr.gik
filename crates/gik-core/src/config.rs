//! Configuration types for GIK.
//!
//! This module provides the configuration structures used by the GIK engine:
//! - [`GlobalConfig`]: User-level configuration stored in `~/.gik/config.yaml`
//! - [`ProjectConfig`]: Project-level overrides stored in `.guided/knowledge/config.yaml`
//! - [`EmbeddingConfig`]: Embedding provider configuration and profiles
//! - [`EmbeddingsSection`]: Simplified embedding config with defaults and per-base overrides
//! - [`PerformanceConfig`]: Performance tuning options (Phase 8.1)

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use crate::embedding::{
    self as emb, EmbeddingConfig as CoreEmbeddingConfig, EmbeddingModelId, EmbeddingProviderKind,
    ModelArchitecture, DEFAULT_DIMENSION, DEFAULT_MODEL_PATH,
};
use crate::errors::GikError;

// ======================================================================
// Performance Constants (Phase 8.1)
// ======================================================================

/// Default batch size for embedding operations.
/// Larger batches are more efficient but require more memory.
/// 32 is a good balance for most GPUs and CPUs.
pub const DEFAULT_EMBEDDING_BATCH_SIZE: usize = 32;

/// Default maximum file size in bytes (1 MB).
/// Files larger than this are skipped to avoid excessive memory/time.
pub const DEFAULT_MAX_FILE_SIZE_BYTES: u64 = 1_000_000;

/// Default maximum number of lines per file.
/// Files with more lines than this are skipped.
pub const DEFAULT_MAX_FILE_LINES: usize = 10_000;

/// Whether to run a warm-up embedding before processing (default: true).
/// Warm-up pays model initialization cost once before the main loop.
pub const DEFAULT_EMBEDDING_WARMUP: bool = true;

/// Whether to enable parallel file reading (default: true).
/// Uses rayon to read and validate files concurrently.
pub const DEFAULT_PARALLEL_FILE_READING: bool = true;

// ============================================================================
// DevicePreference
// ============================================================================

/// Preference for compute device used during embedding inference.
///
/// This allows users to control whether to use GPU acceleration (Metal on macOS)
/// or CPU-only inference.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DevicePreference {
    /// Automatically select the best available device.
    /// Tries GPU (Metal) first, falls back to CPU if unavailable.
    #[default]
    Auto,

    /// Force GPU acceleration (Metal on macOS).
    /// Fails with an error if GPU is not available.
    Gpu,

    /// Force CPU-only inference.
    /// Useful for debugging or when GPU causes issues.
    Cpu,
}

impl std::fmt::Display for DevicePreference {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Auto => write!(f, "auto"),
            Self::Gpu => write!(f, "gpu"),
            Self::Cpu => write!(f, "cpu"),
        }
    }
}

impl std::str::FromStr for DevicePreference {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "auto" => Ok(Self::Auto),
            "gpu" | "metal" | "cuda" => Ok(Self::Gpu),
            "cpu" => Ok(Self::Cpu),
            _ => Err(format!(
                "Unknown device preference: '{}'. Use 'auto', 'gpu', or 'cpu'.",
                s
            )),
        }
    }
}

// ============================================================================
// GlobalConfig
// ============================================================================

/// Global (user-level) configuration for GIK.
///
/// This is typically loaded from `~/.gik/config.yaml` and contains settings
/// that apply across all workspaces, such as embedding provider configuration.
///
/// # Example YAML
///
/// ```yaml
/// embedding:
///   default_profile: local
///   profiles:
///     local:
///       type: candle-sbert
///       model_id: sentence-transformers/all-MiniLM-L6-v2
///       dim: 384
///
/// # New simplified embeddings section (Phase 4.1)
/// embeddings:
///   default:
///     provider: candle
///     modelId: sentence-transformers/all-MiniLM-L6-v2
///     localPath: models/embeddings/all-MiniLM-L6-v2
///   bases:
///     code:
///       provider: candle
///       modelId: sentence-transformers/all-MiniLM-L6-v2
/// ```
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct GlobalConfig {
    /// Embedding configuration with profiles (legacy).
    #[serde(default)]
    pub embedding: EmbeddingConfig,

    /// Simplified embeddings section with default and per-base overrides.
    #[serde(default)]
    pub embeddings: EmbeddingsSection,

    /// Vector index configuration with default and per-base overrides.
    #[serde(default)]
    pub indexes: IndexesSection,

    /// Performance tuning options (Phase 8.1).
    #[serde(default)]
    pub performance: PerformanceConfig,

    /// Device preference for embedding inference (auto/gpu/cpu).
    /// Default: auto (tries GPU first, falls back to CPU).
    #[serde(default)]
    pub device: DevicePreference,

    /// Retrieval configuration (Phase 8.2) including reranker settings.
    #[serde(default)]
    pub retrieval: RetrievalConfig,
}

impl GlobalConfig {
    /// Load the global configuration from the default location (`~/.gik/config.yaml`).
    ///
    /// If the file does not exist, returns a default configuration with sensible
    /// defaults. This allows GIK to work out-of-the-box without manual configuration.
    ///
    /// # Errors
    ///
    /// Returns [`GikError::InvalidGlobalConfig`] if the file exists but cannot be parsed.
    pub fn load_default() -> Result<Self, GikError> {
        match Self::default_path() {
            Some(path) => Self::from_path(&path),
            None => {
                tracing::debug!("Could not determine home directory, using default config");
                Ok(Self::default())
            }
        }
    }

    /// Load the global configuration from a specific path.
    ///
    /// If the file does not exist, returns a default configuration.
    ///
    /// # Errors
    ///
    /// Returns [`GikError::InvalidGlobalConfig`] if the file exists but cannot be parsed.
    /// Returns [`GikError::InvalidConfiguration`] if validation fails.
    pub fn from_path(path: &Path) -> Result<Self, GikError> {
        if !path.exists() {
            tracing::debug!(
                "Global config not found at {}, using defaults",
                path.display()
            );
            return Ok(Self::default());
        }

        let content = fs::read_to_string(path).map_err(|e| {
            GikError::InvalidGlobalConfig(format!("Failed to read {}: {}", path.display(), e))
        })?;

        let config: Self = serde_yaml::from_str(&content).map_err(|e| {
            GikError::InvalidGlobalConfig(format!("Failed to parse {}: {}", path.display(), e))
        })?;

        // Validate configuration and log warnings
        let warnings = config.validate()?;
        for warning in warnings {
            tracing::warn!("Config warning: {}", warning);
        }

        Ok(config)
    }

    /// Get the default global config directory (`~/.gik`).
    pub fn default_dir() -> Option<PathBuf> {
        dirs::home_dir().map(|h| h.join(".gik"))
    }

    /// Get the default global config file path (`~/.gik/config.yaml`).
    pub fn default_path() -> Option<PathBuf> {
        Self::default_dir().map(|d| d.join("config.yaml"))
    }

    /// Create a default configuration suitable for testing.
    ///
    /// This provides a minimal valid configuration without touching the filesystem.
    pub fn default_for_testing() -> Self {
        Self::default()
    }

    /// Get the active embedding profile configuration.
    ///
    /// # Arguments
    ///
    /// * `profile_name` - Optional profile name override. If `None`, uses the default profile.
    ///
    /// # Errors
    ///
    /// Returns [`GikError::UnknownEmbeddingProfile`] if the profile does not exist.
    pub fn get_embedding_profile(
        &self,
        profile_name: Option<&str>,
    ) -> Result<&EmbeddingProfileConfig, GikError> {
        let name = profile_name.unwrap_or(&self.embedding.default_profile);
        self.embedding
            .profiles
            .get(name)
            .ok_or_else(|| GikError::UnknownEmbeddingProfile(name.to_string()))
    }

    /// Resolve the embedding configuration for a specific knowledge base.
    ///
    /// Resolution precedence (highest to lowest):
    /// 1. Global per-base override (`embeddings.bases.<base>`)
    /// 2. Global default (`embeddings.default`)
    /// 3. Hard-coded default (Candle + all-MiniLM-L6-v2)
    ///
    /// Note: Project-level overrides are handled separately by passing
    /// `ProjectConfig` to `resolve_embedding_config_with_project`.
    pub fn resolve_embedding_config(&self, base: &str) -> CoreEmbeddingConfig {
        // Check per-base override
        if let Some(base_config) = self.embeddings.bases.get(base) {
            return base_config.to_core_config();
        }

        // Check global default
        if let Some(default_config) = &self.embeddings.default {
            return default_config.to_core_config();
        }

        // Fall back to hard-coded default
        emb::default_embedding_config_for_base(base)
    }

    /// Resolve the vector index configuration for a specific knowledge base.
    ///
    /// Resolution precedence (highest to lowest):
    /// 1. Global per-base override (`indexes.bases.<base>`)
    /// 2. Global default (`indexes.default`)
    /// 3. Hard-coded default (SimpleFile + Cosine)
    ///
    /// Note: Project-level overrides are handled separately by passing
    /// `ProjectConfig` to `resolve_vector_index_config_with_project`.
    ///
    /// # Arguments
    ///
    /// * `base` - The knowledge base name.
    /// * `embedding` - The resolved embedding config (for dimension).
    pub fn resolve_vector_index_config(
        &self,
        base: &str,
        embedding: &CoreEmbeddingConfig,
    ) -> crate::vector_index::VectorIndexConfig {
        use crate::vector_index::{VectorIndexBackendKind, VectorIndexConfig, VectorMetric};

        let dimension = embedding.dimension.unwrap_or(DEFAULT_DIMENSION);

        // Check per-base override
        if let Some(base_config) = self.indexes.bases.get(base) {
            let backend = base_config
                .backend
                .as_deref()
                .map(|s| s.parse().unwrap_or(VectorIndexBackendKind::SimpleFile))
                .unwrap_or(VectorIndexBackendKind::SimpleFile);
            let metric = base_config
                .metric
                .as_deref()
                .map(|s| s.parse().unwrap_or(VectorMetric::Cosine))
                .unwrap_or(VectorMetric::Cosine);

            return VectorIndexConfig::new(backend, metric, dimension, base);
        }

        // Check global default
        if let Some(default_config) = &self.indexes.default {
            let backend = default_config
                .backend
                .as_deref()
                .map(|s| s.parse().unwrap_or(VectorIndexBackendKind::SimpleFile))
                .unwrap_or(VectorIndexBackendKind::SimpleFile);
            let metric = default_config
                .metric
                .as_deref()
                .map(|s| s.parse().unwrap_or(VectorMetric::Cosine))
                .unwrap_or(VectorMetric::Cosine);

            return VectorIndexConfig::new(backend, metric, dimension, base);
        }

        // Fall back to hard-coded default
        VectorIndexConfig::default_for_base(base, dimension)
    }

    /// Resolve retrieval config with project overrides.
    ///
    /// Resolution precedence (highest to lowest):
    /// 1. Project config overrides (`.guided/knowledge/config.yaml`)
    /// 2. Global config values (`~/.gik/config.yaml`)
    /// 3. Built-in defaults
    ///
    /// # Arguments
    ///
    /// * `project` - Project configuration containing optional overrides.
    ///
    /// # Returns
    ///
    /// A fully resolved `RetrievalConfig` with all overrides applied.
    pub fn resolve_retrieval_config(&self, project: &ProjectConfig) -> RetrievalConfig {
        let mut config = self.retrieval.clone();

        if let Some(ref proj_retrieval) = project.retrieval {
            // Apply reranker overrides
            if let Some(ref proj_reranker) = proj_retrieval.reranker {
                if let Some(enabled) = proj_reranker.enabled {
                    tracing::debug!("Project override: reranker.enabled = {}", enabled);
                    config.reranker.enabled = enabled;
                }
                if let Some(top_k) = proj_reranker.top_k {
                    tracing::debug!("Project override: reranker.topK = {}", top_k);
                    config.reranker.top_k = top_k;
                }
                if let Some(final_k) = proj_reranker.final_k {
                    tracing::debug!("Project override: reranker.finalK = {}", final_k);
                    config.reranker.final_k = final_k;
                }
            }

            // Apply hybrid overrides
            if let Some(ref proj_hybrid) = proj_retrieval.hybrid {
                if let Some(enabled) = proj_hybrid.enabled {
                    tracing::debug!("Project override: hybrid.enabled = {}", enabled);
                    config.hybrid.enabled = enabled;
                }
                if let Some(dense_weight) = proj_hybrid.dense_weight {
                    tracing::debug!("Project override: hybrid.denseWeight = {}", dense_weight);
                    config.hybrid.dense_weight = dense_weight;
                }
                if let Some(sparse_weight) = proj_hybrid.sparse_weight {
                    tracing::debug!("Project override: hybrid.sparseWeight = {}", sparse_weight);
                    config.hybrid.sparse_weight = sparse_weight;
                }
                if let Some(rrf_k) = proj_hybrid.rrf_k {
                    tracing::debug!("Project override: hybrid.rrfK = {}", rrf_k);
                    config.hybrid.rrf_k = rrf_k;
                }
                if let Some(dense_top_k) = proj_hybrid.dense_top_k {
                    tracing::debug!("Project override: hybrid.denseTopK = {}", dense_top_k);
                    config.hybrid.dense_top_k = dense_top_k;
                }
                if let Some(sparse_top_k) = proj_hybrid.sparse_top_k {
                    tracing::debug!("Project override: hybrid.sparseTopK = {}", sparse_top_k);
                    config.hybrid.sparse_top_k = sparse_top_k;
                }
            }
        }

        config
    }

    /// Validates the entire configuration, returning collected warnings.
    ///
    /// Runs validation on all sub-configurations and aggregates warnings.
    ///
    /// # Errors
    ///
    /// Returns the first critical error encountered. All critical errors are
    /// returned as `GikError::InvalidConfiguration`.
    ///
    /// # Warnings
    ///
    /// Non-fatal issues are collected and returned as a list of warning strings.
    /// Callers should log these warnings but can proceed with the configuration.
    pub fn validate(&self) -> Result<Vec<String>, GikError> {
        let mut all_warnings = Vec::new();

        // Validate performance config
        let perf_warnings = self.performance.validate()?;
        all_warnings.extend(perf_warnings);

        // Validate retrieval config
        let reranker_warnings = self.retrieval.reranker.validate()?;
        all_warnings.extend(reranker_warnings);

        let hybrid_warnings = self.retrieval.hybrid.validate()?;
        all_warnings.extend(hybrid_warnings);

        Ok(all_warnings)
    }
}

// ============================================================================
// EmbeddingConfig
// ============================================================================

/// Configuration for embedding providers.
///
/// Defines a default profile and a map of named profiles, each specifying
/// how to generate embeddings (local Candle, Ollama, etc.).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmbeddingConfig {
    /// Name of the default embedding profile to use.
    #[serde(default = "default_profile_name")]
    pub default_profile: String,

    /// Map of profile name to profile configuration.
    #[serde(default)]
    pub profiles: HashMap<String, EmbeddingProfileConfig>,
}

fn default_profile_name() -> String {
    "local".to_string()
}

impl Default for EmbeddingConfig {
    fn default() -> Self {
        let mut profiles = HashMap::new();
        profiles.insert(
            "local".to_string(),
            EmbeddingProfileConfig {
                r#type: "candle-sbert".to_string(),
                model_id: "sentence-transformers/all-MiniLM-L6-v2".to_string(),
                dim: 384,
                path: None,
                host: None,
                model: None,
            },
        );
        Self {
            default_profile: "local".to_string(),
            profiles,
        }
    }
}

// ============================================================================
// EmbeddingProfileConfig
// ============================================================================

/// Configuration for a specific embedding profile.
///
/// Different profile types require different fields:
/// - `candle-sbert`: Uses `model_id`, `dim`, and optionally `path` for local models.
/// - `ollama`: Uses `host` and `model` for remote Ollama server.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmbeddingProfileConfig {
    /// Provider type: "candle-sbert", "ollama", etc.
    #[serde(rename = "type")]
    pub r#type: String,

    /// HuggingFace model ID (e.g., "sentence-transformers/all-MiniLM-L6-v2").
    #[serde(default)]
    pub model_id: String,

    /// Embedding vector dimension.
    #[serde(default)]
    pub dim: usize,

    /// Local path to model files (optional, for offline/cached models).
    #[serde(default)]
    pub path: Option<PathBuf>,

    /// Host URL for remote providers like Ollama (e.g., "http://localhost:11434").
    #[serde(default)]
    pub host: Option<String>,

    /// Model name for remote providers (e.g., "nomic-embed-text").
    #[serde(default)]
    pub model: Option<String>,
}

// ============================================================================
// EmbeddingsSection (Phase 4.1)
// ============================================================================

/// Simplified embeddings configuration section.
///
/// This provides a cleaner configuration format than the profile-based approach:
///
/// ```yaml
/// embeddings:
///   default:
///     provider: candle
///     modelId: sentence-transformers/all-MiniLM-L6-v2
///     localPath: models/embeddings/all-MiniLM-L6-v2
///   bases:
///     code:
///       provider: candle
///       modelId: sentence-transformers/all-MiniLM-L6-v2
///     docs:
///       provider: ollama
///       modelId: nomic-embed-text
/// ```
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct EmbeddingsSection {
    /// Default embedding configuration used when no per-base override exists.
    #[serde(default)]
    pub default: Option<EmbeddingOverride>,

    /// Per-base embedding configuration overrides.
    #[serde(default)]
    pub bases: HashMap<String, EmbeddingOverride>,
}

/// Embedding configuration override.
///
/// All fields are optional to allow partial overrides.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EmbeddingOverride {
    /// Provider type: "candle", "ollama", etc.
    #[serde(default)]
    pub provider: Option<String>,

    /// Model ID (e.g., Hugging Face model ID).
    #[serde(default)]
    pub model_id: Option<String>,

    /// Model architecture: "bert", "roberta", etc.
    ///
    /// If not specified, the architecture will be auto-detected from
    /// the model's `config.json` file. Set this explicitly to override
    /// auto-detection for models with non-standard config files.
    #[serde(default)]
    pub architecture: Option<String>,

    /// Local path to model files.
    #[serde(default)]
    pub local_path: Option<PathBuf>,

    /// Embedding vector dimension.
    #[serde(default)]
    pub dimension: Option<u32>,

    /// Maximum tokens the model accepts.
    #[serde(default)]
    pub max_tokens: Option<u32>,
}

impl EmbeddingOverride {
    /// Convert to a core `EmbeddingConfig`, filling in defaults for missing fields.
    ///
    /// When no `local_path` is specified, resolves the default model path
    /// relative to the global GIK directory (`~/.gik/models/embeddings/...`).
    pub fn to_core_config(&self) -> CoreEmbeddingConfig {
        let provider = self
            .provider
            .as_deref()
            .map(|s| s.parse().unwrap_or(EmbeddingProviderKind::Candle))
            .unwrap_or(EmbeddingProviderKind::Candle);

        let model_id = self
            .model_id
            .as_deref()
            .map(EmbeddingModelId::new)
            .unwrap_or_else(EmbeddingModelId::default_model);

        // Parse architecture if specified
        let architecture = self
            .architecture
            .as_deref()
            .and_then(|s| s.parse::<ModelArchitecture>().ok());

        // Resolve local_path to global ~/.gik/ directory when not specified
        let local_path = self.local_path.clone().or_else(|| {
            GlobalConfig::default_dir().map(|gik_dir| gik_dir.join(DEFAULT_MODEL_PATH))
        });

        CoreEmbeddingConfig {
            provider,
            model_id,
            architecture,
            dimension: self.dimension,
            max_tokens: self.max_tokens,
            local_path,
        }
    }
}

// ============================================================================
// IndexesSection (Phase 4.2)
// ============================================================================

/// Vector index configuration section.
///
/// This provides configuration for vector index backends:
///
/// ```yaml
/// indexes:
///   default:
///     backend: simple_file
///     metric: cosine
///   bases:
///     code:
///       backend: lancedb
///       metric: dot
/// ```
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct IndexesSection {
    /// Default index configuration used when no per-base override exists.
    #[serde(default)]
    pub default: Option<IndexOverride>,

    /// Per-base index configuration overrides.
    #[serde(default)]
    pub bases: HashMap<String, IndexOverride>,
}

/// Index configuration override.
///
/// All fields are optional to allow partial overrides.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IndexOverride {
    /// Backend type: "simple_file", "lancedb", etc.
    #[serde(default)]
    pub backend: Option<String>,

    /// Similarity metric: "cosine", "dot", "l2".
    #[serde(default)]
    pub metric: Option<String>,
}

// ======================================================================
// PerformanceConfig (Phase 8.1)
// ======================================================================

/// Performance tuning configuration.
///
/// Controls batching, parallelism, and resource limits for commit/embedding
/// operations. All fields have sensible defaults for typical workloads.
///
/// # Example YAML
///
/// ```yaml
/// performance:
///   embeddingBatchSize: 32
///   maxFileSizeBytes: 1000000
///   maxFileLines: 10000
///   enableWarmup: true
///   parallelFileReading: true
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PerformanceConfig {
    /// Number of texts to embed in a single batch.
    /// Larger batches are more efficient but require more memory.
    #[serde(default = "default_embedding_batch_size")]
    pub embedding_batch_size: usize,

    /// Maximum file size in bytes. Files larger than this are skipped.
    #[serde(default = "default_max_file_size_bytes")]
    pub max_file_size_bytes: u64,

    /// Maximum number of lines per file. Files with more lines are skipped.
    #[serde(default = "default_max_file_lines")]
    pub max_file_lines: usize,

    /// Whether to run a warm-up embedding call before the main loop.
    /// This pays model initialization cost once upfront.
    #[serde(default = "default_enable_warmup")]
    pub enable_warmup: bool,

    /// Whether to read and validate files in parallel using rayon.
    #[serde(default = "default_parallel_file_reading")]
    pub parallel_file_reading: bool,
}

fn default_embedding_batch_size() -> usize {
    DEFAULT_EMBEDDING_BATCH_SIZE
}
fn default_max_file_size_bytes() -> u64 {
    DEFAULT_MAX_FILE_SIZE_BYTES
}
fn default_max_file_lines() -> usize {
    DEFAULT_MAX_FILE_LINES
}
fn default_enable_warmup() -> bool {
    DEFAULT_EMBEDDING_WARMUP
}
fn default_parallel_file_reading() -> bool {
    DEFAULT_PARALLEL_FILE_READING
}

impl Default for PerformanceConfig {
    fn default() -> Self {
        Self {
            embedding_batch_size: DEFAULT_EMBEDDING_BATCH_SIZE,
            max_file_size_bytes: DEFAULT_MAX_FILE_SIZE_BYTES,
            max_file_lines: DEFAULT_MAX_FILE_LINES,
            enable_warmup: DEFAULT_EMBEDDING_WARMUP,
            parallel_file_reading: DEFAULT_PARALLEL_FILE_READING,
        }
    }
}

impl PerformanceConfig {
    /// Validates the performance configuration, returning warnings for questionable values.
    ///
    /// # Errors
    /// Returns an error if `embedding_batch_size` is 0 (would cause division by zero).
    ///
    /// # Warnings
    /// - `embedding_batch_size > 512`: May cause OOM on constrained devices
    /// - `max_file_size_bytes < 1024`: Extremely restrictive, most files will be skipped
    /// - `max_file_lines < 10`: Extremely restrictive, most files will be skipped
    /// - `max_file_size_bytes > 100MB`: Very large files may slow indexing significantly
    pub fn validate(&self) -> Result<Vec<String>, GikError> {
        let mut warnings = Vec::new();

        // Critical: batch size 0 would cause division by zero
        if self.embedding_batch_size == 0 {
            return Err(GikError::InvalidConfiguration {
                message: "performance.embeddingBatchSize cannot be 0".to_string(),
                hint: "Set embeddingBatchSize to at least 1 (recommended: 32-128)".to_string(),
            });
        }

        // Warning: very large batch sizes may cause OOM
        if self.embedding_batch_size > 512 {
            warnings.push(format!(
                "performance.embeddingBatchSize={} is very large; may cause OOM on constrained devices (recommended: 32-128)",
                self.embedding_batch_size
            ));
        }

        // Warning: extremely restrictive file size limit
        if self.max_file_size_bytes < 1024 {
            warnings.push(format!(
                "performance.maxFileSizeBytes={} bytes is very restrictive; most source files will be skipped",
                self.max_file_size_bytes
            ));
        }

        // Warning: very large file size limit
        if self.max_file_size_bytes > 100 * 1024 * 1024 {
            warnings.push(format!(
                "performance.maxFileSizeBytes={}MB is very large; indexing may be slow for large files",
                self.max_file_size_bytes / (1024 * 1024)
            ));
        }

        // Warning: extremely restrictive line limit
        if self.max_file_lines < 10 {
            warnings.push(format!(
                "performance.maxFileLines={} is very restrictive; most source files will be skipped",
                self.max_file_lines
            ));
        }

        Ok(warnings)
    }
}

// ======================================================================
// RetrievalConfig (Phase 8.2)
// ======================================================================

/// Default reranker model ID.
const DEFAULT_RERANKER_MODEL_ID: &str = "cross-encoder/ms-marco-MiniLM-L6-v2";

/// Default number of candidates to pass to the reranker.
const DEFAULT_RERANKER_TOP_K: usize = 30;

/// Default number of final results after reranking.
const DEFAULT_RERANKER_FINAL_K: usize = 5;

/// Retrieval pipeline configuration (Phase 8.2).
///
/// Controls the two-stage retrieval pipeline:
/// 1. Dense retrieval per base (using embeddings)
/// 2. Global reranking of merged candidates (using cross-encoder)
///
/// # Example YAML
///
/// ```yaml
/// retrieval:
///   reranker:
///     enabled: true
///     modelId: cross-encoder/ms-marco-MiniLM-L6-v2
///     topK: 30
///     finalK: 5
///   hybrid:
///     enabled: true
///     rrfK: 60
///     denseWeight: 0.5
///     sparseWeight: 0.5
///     denseTopK: 50
///     sparseTopK: 50
///     bm25:
///       k1: 1.2
///       b: 0.75
/// ```
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct RetrievalConfig {
    /// Reranker configuration.
    #[serde(default)]
    pub reranker: RerankerConfig,

    /// Hybrid search configuration (BM25 + Dense).
    #[serde(default)]
    pub hybrid: crate::bm25::HybridSearchConfig,
}

/// Cross-encoder reranker configuration.
///
/// The reranker uses a cross-encoder model to re-score candidates
/// from the dense retrieval stage, improving relevance ranking.
///
/// # Fields
///
/// - `enabled`: Whether reranking is active (default: true).
/// - `model_id`: HuggingFace model identifier (default: `cross-encoder/ms-marco-MiniLM-L6-v2`).
/// - `local_path`: Optional local path to model weights. If not set, uses `~/.gik/models/rerankers/<model_name>`.
/// - `top_k`: Number of candidates to pass to the reranker (default: 30).
/// - `final_k`: Default number of final results after reranking (default: 5).
///   Note: This can be overridden by the CLI `--top-k` flag in `gik ask`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RerankerConfig {
    /// Whether reranking is enabled.
    #[serde(default = "default_reranker_enabled")]
    pub enabled: bool,

    /// HuggingFace model ID for the cross-encoder.
    #[serde(default = "default_reranker_model_id")]
    pub model_id: String,

    /// Optional local path to model weights.
    /// If not set, defaults to `~/.gik/models/rerankers/<model_name>`.
    #[serde(default)]
    pub local_path: Option<PathBuf>,

    /// Number of candidates to pass to the reranker.
    #[serde(default = "default_reranker_top_k")]
    pub top_k: usize,

    /// Default number of final results after reranking.
    /// Can be overridden by the CLI `--top-k` flag.
    #[serde(default = "default_reranker_final_k")]
    pub final_k: usize,
}

fn default_reranker_enabled() -> bool {
    true
}

fn default_reranker_model_id() -> String {
    DEFAULT_RERANKER_MODEL_ID.to_string()
}

fn default_reranker_top_k() -> usize {
    DEFAULT_RERANKER_TOP_K
}

fn default_reranker_final_k() -> usize {
    DEFAULT_RERANKER_FINAL_K
}

impl Default for RerankerConfig {
    fn default() -> Self {
        Self {
            enabled: default_reranker_enabled(),
            model_id: default_reranker_model_id(),
            local_path: None,
            top_k: default_reranker_top_k(),
            final_k: default_reranker_final_k(),
        }
    }
}

impl RerankerConfig {
    /// Returns the effective local path for the reranker model.
    ///
    /// If `local_path` is set, returns that. Otherwise, computes a default path
    /// based on `~/.gik/models/rerankers/<model_name>`.
    pub fn effective_local_path(&self) -> Option<PathBuf> {
        if let Some(ref path) = self.local_path {
            return Some(path.clone());
        }

        // Compute default path from model_id
        let model_name = self
            .model_id
            .split('/')
            .next_back()
            .unwrap_or(&self.model_id);
        GlobalConfig::default_dir()
            .map(|gik_dir| gik_dir.join("models").join("rerankers").join(model_name))
    }

    /// Validate the reranker configuration.
    ///
    /// Returns a list of warnings for non-fatal issues.
    ///
    /// # Errors
    ///
    /// Returns an error if the configuration is invalid.
    pub fn validate(&self) -> Result<Vec<String>, GikError> {
        let mut warnings = Vec::new();

        // Validate topK
        if self.top_k == 0 {
            return Err(GikError::InvalidConfiguration {
                message: "retrieval.reranker.topK cannot be 0".to_string(),
                hint: "Set topK to at least 1 (recommended: 20-50)".to_string(),
            });
        }

        // Validate finalK
        if self.final_k == 0 {
            return Err(GikError::InvalidConfiguration {
                message: "retrieval.reranker.finalK cannot be 0".to_string(),
                hint: "Set finalK to at least 1 (recommended: 5-20)".to_string(),
            });
        }

        // Warn if finalK > topK (will be clamped at runtime)
        if self.final_k > self.top_k {
            warnings.push(format!(
                "retrieval.reranker.finalK ({}) > topK ({}); finalK will be clamped to topK",
                self.final_k, self.top_k
            ));
        }

        // Warn about very large topK values
        if self.top_k > 1000 {
            warnings.push(format!(
                "retrieval.reranker.topK ({}) is very large; this may impact performance",
                self.top_k
            ));
        }

        Ok(warnings)
    }
}

// ======================================================================
// RetrievalConfigOverride (Project-level overrides)
// ======================================================================

/// Project-level retrieval configuration overrides.
///
/// All fields are optional; unset fields inherit from global config.
/// This enables per-project customization of reranker and hybrid search settings.
///
/// # Example YAML
///
/// ```yaml
/// retrieval:
///   reranker:
///     finalK: 15
///   hybrid:
///     denseWeight: 0.7
/// ```
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RetrievalConfigOverride {
    /// Reranker configuration overrides.
    #[serde(default)]
    pub reranker: Option<RerankerConfigOverride>,

    /// Hybrid search configuration overrides.
    #[serde(default)]
    pub hybrid: Option<HybridConfigOverride>,
}

/// Project-level reranker configuration overrides.
///
/// All fields optional; unset fields inherit from global config.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RerankerConfigOverride {
    /// Override for enabled state.
    pub enabled: Option<bool>,

    /// Override for topK (candidates to rerank).
    pub top_k: Option<usize>,

    /// Override for finalK (results after reranking).
    pub final_k: Option<usize>,
}

/// Project-level hybrid search configuration overrides.
///
/// All fields optional; unset fields inherit from global config.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HybridConfigOverride {
    /// Override for enabled state.
    pub enabled: Option<bool>,

    /// Override for denseWeight.
    pub dense_weight: Option<f32>,

    /// Override for sparseWeight.
    pub sparse_weight: Option<f32>,

    /// Override for rrfK.
    pub rrf_k: Option<f32>,

    /// Override for denseTopK.
    pub dense_top_k: Option<usize>,

    /// Override for sparseTopK.
    pub sparse_top_k: Option<usize>,
}

// ======================================================================
// ProjectConfig
// ======================================================================

/// Project-level configuration for GIK.
///
/// This is stored in `.guided/knowledge/config.yaml` within a workspace and allows
/// per-project overrides of global settings.
///
/// # Example YAML
///
/// ```yaml
/// embedding_profile: local
///
/// # Phase 4.1: Per-base embedding overrides
/// embeddings:
///   bases:
///     code:
///       provider: candle
///       modelId: sentence-transformers/all-MiniLM-L6-v2
///
/// # Phase 4.2: Per-base index overrides
/// indexes:
///   bases:
///     code:
///       backend: simple_file
///       metric: cosine
///
/// # Phase 8.2: Per-project retrieval overrides
/// retrieval:
///   reranker:
///     finalK: 15
///   hybrid:
///     denseWeight: 0.7
/// ```
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ProjectConfig {
    /// Override the embedding profile for this workspace (legacy).
    #[serde(default)]
    pub embedding_profile: Option<String>,

    /// Per-base embedding configuration overrides.
    #[serde(default)]
    pub embeddings: EmbeddingsSection,

    /// Per-base vector index configuration overrides.
    #[serde(default)]
    pub indexes: IndexesSection,

    /// Per-project retrieval configuration overrides.
    ///
    /// Allows overriding global reranker and hybrid search settings.
    /// All fields are optional; unset fields inherit from global config.
    #[serde(default)]
    pub retrieval: Option<RetrievalConfigOverride>,
}

impl ProjectConfig {
    /// Load the project configuration from a workspace.
    ///
    /// Looks for `.guided/knowledge/config.yaml` in the workspace root.
    /// If the file does not exist, returns a default (empty) configuration.
    ///
    /// # Errors
    ///
    /// Returns [`GikError::InvalidProjectConfig`] if the file exists but cannot be parsed.
    pub fn load_from_workspace(workspace_root: &Path) -> Result<Self, GikError> {
        let path = Self::config_path_for_workspace(workspace_root);
        Self::from_path(&path)
    }

    /// Load the project configuration from a specific path.
    ///
    /// If the file does not exist, returns a default configuration.
    ///
    /// # Errors
    ///
    /// Returns [`GikError::InvalidProjectConfig`] if the file exists but cannot be parsed.
    pub fn from_path(path: &Path) -> Result<Self, GikError> {
        if !path.exists() {
            tracing::debug!(
                "Project config not found at {}, using defaults",
                path.display()
            );
            return Ok(Self::default());
        }

        let content = fs::read_to_string(path).map_err(|e| {
            GikError::InvalidProjectConfig(format!("Failed to read {}: {}", path.display(), e))
        })?;

        serde_yaml::from_str(&content).map_err(|e| {
            GikError::InvalidProjectConfig(format!("Failed to parse {}: {}", path.display(), e))
        })
    }

    /// Get the config file path for a given workspace root.
    pub fn config_path_for_workspace(workspace_root: &Path) -> PathBuf {
        workspace_root
            .join(".guided")
            .join("knowledge")
            .join("config.yaml")
    }

    /// Create a default configuration suitable for testing.
    pub fn default_for_testing() -> Self {
        Self::default()
    }

    /// Resolve the embedding configuration for a specific knowledge base.
    ///
    /// Resolution precedence (highest to lowest):
    /// 1. Project per-base override (`embeddings.bases.<base>`)
    /// 2. Global per-base override (`global_config.embeddings.bases.<base>`)
    /// 3. Global default (`global_config.embeddings.default`)
    /// 4. Hard-coded default (Candle + all-MiniLM-L6-v2)
    pub fn resolve_embedding_config(
        &self,
        base: &str,
        global_config: &GlobalConfig,
    ) -> CoreEmbeddingConfig {
        // Check project per-base override
        if let Some(base_config) = self.embeddings.bases.get(base) {
            return base_config.to_core_config();
        }

        // Delegate to global config resolution
        global_config.resolve_embedding_config(base)
    }

    /// Resolve the vector index configuration for a specific knowledge base.
    ///
    /// Resolution precedence (highest to lowest):
    /// 1. Project per-base override (`indexes.bases.<base>`)
    /// 2. Global per-base override (`global_config.indexes.bases.<base>`)
    /// 3. Global default (`global_config.indexes.default`)
    /// 4. Hard-coded default (SimpleFile + Cosine)
    ///
    /// # Arguments
    ///
    /// * `base` - The knowledge base name.
    /// * `embedding` - The resolved embedding config (for dimension).
    /// * `global_config` - The global configuration.
    pub fn resolve_vector_index_config(
        &self,
        base: &str,
        embedding: &CoreEmbeddingConfig,
        global_config: &GlobalConfig,
    ) -> crate::vector_index::VectorIndexConfig {
        use crate::vector_index::{VectorIndexBackendKind, VectorIndexConfig, VectorMetric};

        let dimension = embedding.dimension.unwrap_or(DEFAULT_DIMENSION);

        // Check project per-base override
        if let Some(base_config) = self.indexes.bases.get(base) {
            let backend = base_config
                .backend
                .as_deref()
                .map(|s| s.parse().unwrap_or(VectorIndexBackendKind::SimpleFile))
                .unwrap_or(VectorIndexBackendKind::SimpleFile);
            let metric = base_config
                .metric
                .as_deref()
                .map(|s| s.parse().unwrap_or(VectorMetric::Cosine))
                .unwrap_or(VectorMetric::Cosine);

            return VectorIndexConfig::new(backend, metric, dimension, base);
        }

        // Delegate to global config resolution
        global_config.resolve_vector_index_config(base, embedding)
    }

    /// Validate the project configuration.
    ///
    /// Returns a list of warnings for non-fatal issues.
    ///
    /// # Errors
    ///
    /// Returns an error if the configuration is invalid.
    pub fn validate(&self) -> Result<Vec<String>, GikError> {
        let mut warnings = Vec::new();

        // Validate retrieval overrides if present
        if let Some(ref retrieval) = self.retrieval {
            // Validate reranker overrides
            if let Some(ref reranker) = retrieval.reranker {
                if let Some(top_k) = reranker.top_k {
                    if top_k == 0 {
                        return Err(GikError::InvalidConfiguration {
                            message: "retrieval.reranker.topK cannot be 0".to_string(),
                            hint: "Set topK to at least 1 (recommended: 20-50)".to_string(),
                        });
                    }
                }
                if let Some(final_k) = reranker.final_k {
                    if final_k == 0 {
                        return Err(GikError::InvalidConfiguration {
                            message: "retrieval.reranker.finalK cannot be 0".to_string(),
                            hint: "Set finalK to at least 1 (recommended: 5-20)".to_string(),
                        });
                    }
                }
                // Warn if both are set and finalK > topK
                if let (Some(top_k), Some(final_k)) = (reranker.top_k, reranker.final_k) {
                    if final_k > top_k {
                        warnings.push(format!(
                            "project retrieval.reranker.finalK ({}) > topK ({}); finalK will be clamped",
                            final_k, top_k
                        ));
                    }
                }
            }

            // Validate hybrid overrides
            if let Some(ref hybrid) = retrieval.hybrid {
                if let Some(dense_weight) = hybrid.dense_weight {
                    if dense_weight < 0.0 {
                        return Err(GikError::InvalidConfiguration {
                            message: format!("retrieval.hybrid.denseWeight ({}) cannot be negative", dense_weight),
                            hint: "Use a value between 0.0 and 1.0".to_string(),
                        });
                    }
                    if dense_weight > 1.0 {
                        warnings.push(format!(
                            "project retrieval.hybrid.denseWeight ({}) > 1.0; consider normalizing",
                            dense_weight
                        ));
                    }
                }
                if let Some(sparse_weight) = hybrid.sparse_weight {
                    if sparse_weight < 0.0 {
                        return Err(GikError::InvalidConfiguration {
                            message: format!("retrieval.hybrid.sparseWeight ({}) cannot be negative", sparse_weight),
                            hint: "Use a value between 0.0 and 1.0".to_string(),
                        });
                    }
                }
                if let Some(rrfk) = hybrid.rrf_k {
                    if rrfk <= 0.0 {
                        return Err(GikError::InvalidConfiguration {
                            message: "retrieval.hybrid.rrfK must be positive".to_string(),
                            hint: "Set rrfK to at least 1 (recommended: 60)".to_string(),
                        });
                    }
                }
            }
        }

        Ok(warnings)
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::TempDir;

    #[test]
    fn test_global_config_default() {
        let config = GlobalConfig::default();
        assert_eq!(config.embedding.default_profile, "local");
        assert!(config.embedding.profiles.contains_key("local"));
    }

    #[test]
    fn test_global_config_from_yaml() {
        let yaml = r#"
embedding:
  default_profile: custom
  profiles:
    custom:
      type: candle-sbert
      model_id: sentence-transformers/all-MiniLM-L6-v2
      dim: 384
"#;
        let config: GlobalConfig = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(config.embedding.default_profile, "custom");
        assert!(config.embedding.profiles.contains_key("custom"));
        let profile = &config.embedding.profiles["custom"];
        assert_eq!(profile.r#type, "candle-sbert");
        assert_eq!(profile.dim, 384);
    }

    #[test]
    fn test_global_config_missing_file() {
        let temp = TempDir::new().unwrap();
        let path = temp.path().join("nonexistent.yaml");
        let config = GlobalConfig::from_path(&path).unwrap();
        // Should return default config
        assert_eq!(config.embedding.default_profile, "local");
    }

    #[test]
    fn test_global_config_invalid_yaml() {
        let temp = TempDir::new().unwrap();
        let path = temp.path().join("invalid.yaml");
        std::fs::write(&path, "not: [valid: yaml").unwrap();
        let result = GlobalConfig::from_path(&path);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(matches!(err, GikError::InvalidGlobalConfig(_)));
    }

    #[test]
    fn test_global_config_get_embedding_profile() {
        let config = GlobalConfig::default();
        // Default profile should exist
        let profile = config.get_embedding_profile(None).unwrap();
        assert_eq!(profile.r#type, "candle-sbert");

        // Explicit profile name
        let profile = config.get_embedding_profile(Some("local")).unwrap();
        assert_eq!(profile.r#type, "candle-sbert");

        // Unknown profile should error
        let result = config.get_embedding_profile(Some("unknown"));
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            GikError::UnknownEmbeddingProfile(_)
        ));
    }

    #[test]
    fn test_project_config_default() {
        let config = ProjectConfig::default();
        assert!(config.embedding_profile.is_none());
    }

    #[test]
    fn test_project_config_from_yaml() {
        let yaml = "embedding_profile: custom\n";
        let config: ProjectConfig = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(config.embedding_profile, Some("custom".to_string()));
    }

    #[test]
    fn test_project_config_missing_file() {
        let temp = TempDir::new().unwrap();
        let config = ProjectConfig::load_from_workspace(temp.path()).unwrap();
        // Should return default config
        assert!(config.embedding_profile.is_none());
    }

    #[test]
    fn test_project_config_from_workspace() {
        let temp = TempDir::new().unwrap();
        let config_dir = temp.path().join(".guided").join("knowledge");
        fs::create_dir_all(&config_dir).unwrap();

        let config_path = config_dir.join("config.yaml");
        let mut file = fs::File::create(&config_path).unwrap();
        writeln!(file, "embedding_profile: my-profile").unwrap();

        let config = ProjectConfig::load_from_workspace(temp.path()).unwrap();
        assert_eq!(config.embedding_profile, Some("my-profile".to_string()));
    }

    #[test]
    fn test_project_config_invalid_yaml() {
        let temp = TempDir::new().unwrap();
        let config_dir = temp.path().join(".guided").join("knowledge");
        fs::create_dir_all(&config_dir).unwrap();

        let config_path = config_dir.join("config.yaml");
        fs::write(&config_path, "invalid: [yaml").unwrap();

        let result = ProjectConfig::load_from_workspace(temp.path());
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            GikError::InvalidProjectConfig(_)
        ));
    }

    #[test]
    fn test_embedding_profile_config_serialization() {
        let profile = EmbeddingProfileConfig {
            r#type: "ollama".to_string(),
            model_id: String::new(),
            dim: 768,
            path: None,
            host: Some("http://localhost:11434".to_string()),
            model: Some("nomic-embed-text".to_string()),
        };

        let yaml = serde_yaml::to_string(&profile).unwrap();
        assert!(yaml.contains("type: ollama"));
        assert!(yaml.contains("host: http://localhost:11434"));
        assert!(yaml.contains("model: nomic-embed-text"));
    }

    // ------------------------------------------------------------------------
    // Embedding resolution tests (Phase 4.1)
    // ------------------------------------------------------------------------

    #[test]
    fn test_resolve_embedding_config_hardcoded_default() {
        use crate::embedding::DEFAULT_MODEL_ID;

        // No config files - should fall back to hard-coded default
        let global = GlobalConfig::default();
        let config = global.resolve_embedding_config("code");

        assert_eq!(config.provider, EmbeddingProviderKind::Candle);
        assert_eq!(config.model_id.as_str(), DEFAULT_MODEL_ID);
        assert_eq!(config.dimension, Some(DEFAULT_DIMENSION));
        // local_path should resolve to ~/.gik/models/embeddings/...
        assert!(config.local_path.is_some());
        let path = config.local_path.unwrap();
        assert!(
            path.ends_with(DEFAULT_MODEL_PATH),
            "Expected path to end with {}, got {:?}",
            DEFAULT_MODEL_PATH,
            path
        );
    }

    #[test]
    fn test_resolve_embedding_config_global_default() {
        let yaml = r#"
embeddings:
  default:
    provider: candle
    modelId: custom-model
    dimension: 512
"#;
        let global: GlobalConfig = serde_yaml::from_str(yaml).unwrap();
        let config = global.resolve_embedding_config("code");

        assert_eq!(config.provider, EmbeddingProviderKind::Candle);
        assert_eq!(config.model_id.as_str(), "custom-model");
        assert_eq!(config.dimension, Some(512));
    }

    #[test]
    fn test_resolve_embedding_config_global_per_base() {
        let yaml = r#"
embeddings:
  default:
    provider: candle
    modelId: default-model
  bases:
    code:
      provider: candle
      modelId: code-model
      dimension: 768
"#;
        let global: GlobalConfig = serde_yaml::from_str(yaml).unwrap();

        // "code" base should use per-base override
        let code_config = global.resolve_embedding_config("code");
        assert_eq!(code_config.model_id.as_str(), "code-model");
        assert_eq!(code_config.dimension, Some(768));

        // "docs" base should fall back to default
        let docs_config = global.resolve_embedding_config("docs");
        assert_eq!(docs_config.model_id.as_str(), "default-model");
    }

    #[test]
    fn test_resolve_embedding_config_project_override() {
        let global_yaml = r#"
embeddings:
  default:
    provider: candle
    modelId: global-model
  bases:
    code:
      modelId: global-code-model
"#;
        let project_yaml = r#"
embeddings:
  bases:
    code:
      modelId: project-code-model
      dimension: 1024
"#;
        let global: GlobalConfig = serde_yaml::from_str(global_yaml).unwrap();
        let project: ProjectConfig = serde_yaml::from_str(project_yaml).unwrap();

        // Project override should take precedence
        let config = project.resolve_embedding_config("code", &global);
        assert_eq!(config.model_id.as_str(), "project-code-model");
        assert_eq!(config.dimension, Some(1024));

        // "docs" should fall back to global default
        let docs_config = project.resolve_embedding_config("docs", &global);
        assert_eq!(docs_config.model_id.as_str(), "global-model");
    }

    #[test]
    fn test_embedding_override_to_core_config() {
        let override_config = EmbeddingOverride {
            provider: Some("ollama".to_string()),
            model_id: Some("nomic-embed-text".to_string()),
            architecture: None,
            local_path: None,
            dimension: Some(768),
            max_tokens: Some(512),
        };

        let config = override_config.to_core_config();
        assert_eq!(config.provider, EmbeddingProviderKind::Ollama);
        assert_eq!(config.model_id.as_str(), "nomic-embed-text");
        assert_eq!(config.dimension, Some(768));
        assert_eq!(config.max_tokens, Some(512));
        // local_path should fall back to default (~/.gik/models/embeddings/...)
        assert!(config.local_path.is_some());
        let path = config.local_path.unwrap();
        assert!(
            path.ends_with(DEFAULT_MODEL_PATH),
            "Expected path to end with {}, got {:?}",
            DEFAULT_MODEL_PATH,
            path
        );
    }

    // ========================================================================
    // Validation Tests
    // ========================================================================

    #[test]
    fn test_performance_config_validate_batch_size_zero() {
        let config = PerformanceConfig {
            embedding_batch_size: 0,
            ..Default::default()
        };
        let result = config.validate();
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(matches!(err, GikError::InvalidConfiguration { .. }));
    }

    #[test]
    fn test_performance_config_validate_large_batch_size_warning() {
        let config = PerformanceConfig {
            embedding_batch_size: 1000,
            ..Default::default()
        };
        let warnings = config.validate().unwrap();
        assert!(!warnings.is_empty());
        assert!(warnings[0].contains("very large"));
    }

    #[test]
    fn test_performance_config_validate_small_file_limits_warning() {
        let config = PerformanceConfig {
            max_file_size_bytes: 100,
            max_file_lines: 5,
            ..Default::default()
        };
        let warnings = config.validate().unwrap();
        assert!(warnings.len() >= 2);
    }

    #[test]
    fn test_reranker_config_validate_top_k_zero() {
        let config = RerankerConfig {
            top_k: 0,
            ..Default::default()
        };
        let result = config.validate();
        assert!(result.is_err());
    }

    #[test]
    fn test_reranker_config_validate_final_k_greater_than_top_k() {
        let config = RerankerConfig {
            top_k: 10,
            final_k: 20,
            ..Default::default()
        };
        let warnings = config.validate().unwrap();
        assert!(!warnings.is_empty());
        assert!(warnings[0].contains("clamped"));
    }

    #[test]
    fn test_global_config_validate_default_is_valid() {
        let config = GlobalConfig::default();
        let result = config.validate();
        assert!(result.is_ok());
        // Default config should have no warnings
        assert!(result.unwrap().is_empty());
    }

    #[test]
    fn test_global_config_from_path_with_invalid_batch_size() {
        let temp = TempDir::new().unwrap();
        let path = temp.path().join("config.yaml");
        let mut file = std::fs::File::create(&path).unwrap();
        writeln!(
            file,
            r#"
performance:
  embeddingBatchSize: 0
"#
        )
        .unwrap();

        let result = GlobalConfig::from_path(&path);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(matches!(err, GikError::InvalidConfiguration { .. }));
    }

    // -------------------------------------------------------------------------
    // Project config validation tests
    // -------------------------------------------------------------------------

    #[test]
    fn test_project_config_validate_empty() {
        let config = ProjectConfig::default();
        let result = config.validate();
        assert!(result.is_ok());
        assert!(result.unwrap().is_empty());
    }

    #[test]
    fn test_project_config_validate_negative_dense_weight() {
        let config = ProjectConfig {
            retrieval: Some(RetrievalConfigOverride {
                hybrid: Some(HybridConfigOverride {
                    dense_weight: Some(-0.5),
                    ..Default::default()
                }),
                ..Default::default()
            }),
            ..Default::default()
        };
        let result = config.validate();
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(matches!(err, GikError::InvalidConfiguration { .. }));
    }

    #[test]
    fn test_project_config_validate_negative_sparse_weight() {
        let config = ProjectConfig {
            retrieval: Some(RetrievalConfigOverride {
                hybrid: Some(HybridConfigOverride {
                    sparse_weight: Some(-1.0),
                    ..Default::default()
                }),
                ..Default::default()
            }),
            ..Default::default()
        };
        let result = config.validate();
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(matches!(err, GikError::InvalidConfiguration { .. }));
    }

    #[test]
    fn test_project_config_validate_zero_rrf_k() {
        let config = ProjectConfig {
            retrieval: Some(RetrievalConfigOverride {
                hybrid: Some(HybridConfigOverride {
                    rrf_k: Some(0.0),
                    ..Default::default()
                }),
                ..Default::default()
            }),
            ..Default::default()
        };
        let result = config.validate();
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(matches!(err, GikError::InvalidConfiguration { .. }));
    }

    #[test]
    fn test_project_config_validate_zero_top_k() {
        let config = ProjectConfig {
            retrieval: Some(RetrievalConfigOverride {
                reranker: Some(RerankerConfigOverride {
                    top_k: Some(0),
                    ..Default::default()
                }),
                ..Default::default()
            }),
            ..Default::default()
        };
        let result = config.validate();
        assert!(result.is_err());
    }

    #[test]
    fn test_project_config_validate_warns_final_k_greater_than_top_k() {
        let config = ProjectConfig {
            retrieval: Some(RetrievalConfigOverride {
                reranker: Some(RerankerConfigOverride {
                    top_k: Some(10),
                    final_k: Some(20),
                    ..Default::default()
                }),
                ..Default::default()
            }),
            ..Default::default()
        };
        let result = config.validate();
        assert!(result.is_ok());
        let warnings = result.unwrap();
        assert!(!warnings.is_empty());
        assert!(warnings[0].contains("clamped"));
    }

    // -------------------------------------------------------------------------
    // Retrieval config resolution tests
    // -------------------------------------------------------------------------

    #[test]
    fn test_resolve_retrieval_config_no_overrides() {
        let global = GlobalConfig::default();
        let project = ProjectConfig::default();
        let resolved = global.resolve_retrieval_config(&project);
        // Should match global defaults
        assert_eq!(resolved.reranker.enabled, global.retrieval.reranker.enabled);
        assert_eq!(resolved.hybrid.dense_weight, global.retrieval.hybrid.dense_weight);
    }

    #[test]
    fn test_resolve_retrieval_config_with_reranker_overrides() {
        let global = GlobalConfig::default();
        let project = ProjectConfig {
            retrieval: Some(RetrievalConfigOverride {
                reranker: Some(RerankerConfigOverride {
                    enabled: Some(false),
                    top_k: Some(100),
                    final_k: None, // Should inherit from global
                    ..Default::default()
                }),
                ..Default::default()
            }),
            ..Default::default()
        };
        let resolved = global.resolve_retrieval_config(&project);
        assert!(!resolved.reranker.enabled);
        assert_eq!(resolved.reranker.top_k, 100);
        assert_eq!(resolved.reranker.final_k, global.retrieval.reranker.final_k);
    }

    #[test]
    fn test_resolve_retrieval_config_with_hybrid_overrides() {
        let global = GlobalConfig::default();
        let project = ProjectConfig {
            retrieval: Some(RetrievalConfigOverride {
                hybrid: Some(HybridConfigOverride {
                    dense_weight: Some(0.8),
                    sparse_weight: Some(0.2),
                    rrf_k: None, // Should inherit
                    ..Default::default()
                }),
                ..Default::default()
            }),
            ..Default::default()
        };
        let resolved = global.resolve_retrieval_config(&project);
        assert!((resolved.hybrid.dense_weight - 0.8).abs() < 0.001);
        assert!((resolved.hybrid.sparse_weight - 0.2).abs() < 0.001);
        // rrfK should inherit from global
        assert!((resolved.hybrid.rrf_k - global.retrieval.hybrid.rrf_k).abs() < 0.001);
    }
}
