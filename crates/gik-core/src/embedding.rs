//! Embedding provider abstraction and model management for GIK.
//!
//! This module provides:
//! - [`EmbeddingProviderKind`] - enum of supported embedding backends
//! - [`EmbeddingModelId`] - newtype for embedding model identifiers
//! - [`EmbeddingConfig`] - configuration for an embedding provider/model
//! - [`ModelInfo`] - on-disk metadata for a base's embedding model
//! - [`ModelCompatibility`] - result of comparing config vs stored model-info
//! - [`EmbeddingBackend`] - trait for embedding providers
//! - [`CandleEmbeddingBackend`] - Candle-based embeddings (via gik-model)
//!
//! ## Runtime vs Test Backends
//!
//! - **Runtime**: Only real embedding backends are used (Candle, future Ollama).
//!   If the model is not available, operations fail with a clear error.
//! - **Tests**: A `MockEmbeddingBackend` is available under `#[cfg(test)]` for
//!   exercising commit/index/ask flows without requiring real models.
//!
//! There is **no silent fallback** to mock backends at runtime. If the embedding
//! model is missing, `gik commit` and `gik ask` will fail with an actionable error
//! message instructing the user to download the model.
//!
//! ## Default Model
//!
//! GIK uses `sentence-transformers/all-MiniLM-L6-v2` as the default embedding model.
//! This model should be cloned from Hugging Face:
//!
//! ```bash
//! git clone https://huggingface.co/sentence-transformers/all-MiniLM-L6-v2 \
//!     models/embeddings/all-MiniLM-L6-v2
//! ```
//!
//! The model produces 384-dimensional embeddings and supports up to 256 tokens.
//!
//! ## Architecture Note
//!
//! As of gik-model migration, the actual Candle implementation lives in the
//! `gik-model` crate. This module provides:
//! - Domain types and traits (stable API)
//! - `CandleEmbeddingBackend` as a wrapper around `gik-model::CandleEmbeddingModel`
//! - Factory function `create_backend()` that delegates to gik-model

use std::fmt;
use std::path::PathBuf;
use std::str::FromStr;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::config::DevicePreference;
use crate::errors::GikError;

// ============================================================================
// Constants
// ============================================================================

/// Default embedding model ID (Hugging Face identifier).
pub const DEFAULT_MODEL_ID: &str = "sentence-transformers/all-MiniLM-L6-v2";

/// Default local path for the embedding model (relative to workspace).
pub const DEFAULT_MODEL_PATH: &str = "models/embeddings/all-MiniLM-L6-v2";

/// Default embedding dimension for all-MiniLM-L6-v2.
pub const DEFAULT_DIMENSION: u32 = 384;

/// Default max tokens for all-MiniLM-L6-v2.
pub const DEFAULT_MAX_TOKENS: u32 = 256;

// ============================================================================
// ModelArchitecture
// ============================================================================

/// Transformer model architecture family for embedding backends.
///
/// Different model architectures require different loading and tokenization
/// strategies. GIK auto-detects the architecture from the model's `config.json`
/// but this can be overridden in the embedding configuration.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ModelArchitecture {
    /// BERT-family models (BERT, DistilBERT, SentenceTransformers).
    #[default]
    Bert,

    /// RoBERTa-family models (RoBERTa, XLM-RoBERTa, CodeBERT).
    Roberta,
}

impl fmt::Display for ModelArchitecture {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Bert => write!(f, "bert"),
            Self::Roberta => write!(f, "roberta"),
        }
    }
}

impl FromStr for ModelArchitecture {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "bert" | "distilbert" => Ok(Self::Bert),
            "roberta" | "xlm-roberta" | "camembert" => Ok(Self::Roberta),
            other => Err(format!("Unsupported model architecture: {}", other)),
        }
    }
}

impl ModelArchitecture {
    /// Detect architecture from HuggingFace model config.json.
    pub fn detect_from_config(config: &HuggingFaceModelConfig) -> Result<Self, String> {
        config.model_type.parse()
    }

    /// Get the default pad token ID for this architecture.
    pub fn default_pad_token_id(&self) -> u32 {
        match self {
            Self::Bert => 0,
            Self::Roberta => 1,
        }
    }

    /// Get the default pad token string for this architecture.
    pub fn default_pad_token(&self) -> &'static str {
        match self {
            Self::Bert => "[PAD]",
            Self::Roberta => "<pad>",
        }
    }
}

// ============================================================================
// HuggingFaceModelConfig
// ============================================================================

/// Parsed configuration from a HuggingFace model's `config.json`.
#[derive(Debug, Clone, Deserialize)]
pub struct HuggingFaceModelConfig {
    /// Model architecture type (e.g., "bert", "roberta", "distilbert").
    #[serde(default)]
    pub model_type: String,

    /// Hidden layer size / embedding dimension.
    pub hidden_size: usize,

    /// Maximum position embeddings (context length).
    #[serde(default = "default_max_position_embeddings")]
    pub max_position_embeddings: usize,

    /// Padding token ID.
    #[serde(default)]
    pub pad_token_id: Option<u32>,

    /// Vocabulary size.
    #[serde(default)]
    pub vocab_size: Option<usize>,

    /// Layer normalization epsilon.
    #[serde(default = "default_layer_norm_eps")]
    pub layer_norm_eps: f64,

    /// Number of attention heads.
    #[serde(default = "default_num_attention_heads")]
    pub num_attention_heads: usize,

    /// Intermediate (feed-forward) layer size.
    #[serde(default)]
    pub intermediate_size: Option<usize>,

    /// Activation function name.
    #[serde(default = "default_hidden_act")]
    pub hidden_act: String,

    /// Number of hidden layers.
    #[serde(default = "default_num_hidden_layers")]
    pub num_hidden_layers: usize,

    /// Token type vocabulary size.
    #[serde(default = "default_type_vocab_size")]
    pub type_vocab_size: usize,

    /// Attention dropout probability.
    #[serde(default)]
    pub attention_probs_dropout_prob: Option<f32>,

    /// Hidden layer dropout probability.
    #[serde(default)]
    pub hidden_dropout_prob: Option<f32>,
}

fn default_max_position_embeddings() -> usize {
    512
}
fn default_layer_norm_eps() -> f64 {
    1e-12
}
fn default_num_attention_heads() -> usize {
    12
}
fn default_hidden_act() -> String {
    "gelu".to_string()
}
fn default_num_hidden_layers() -> usize {
    6
}
fn default_type_vocab_size() -> usize {
    2
}

impl HuggingFaceModelConfig {
    /// Load model config from a directory containing `config.json`.
    pub fn load_from_path(model_path: &std::path::Path) -> Result<Self, GikError> {
        let config_path = model_path.join("config.json");
        if !config_path.exists() {
            return Err(GikError::EmbeddingProviderUnavailable {
                provider: "candle".to_string(),
                reason: format!("config.json not found at {}", config_path.display()),
            });
        }

        let config_content = std::fs::read_to_string(&config_path).map_err(|e| {
            GikError::EmbeddingProviderUnavailable {
                provider: "candle".to_string(),
                reason: format!("Failed to read config.json: {}", e),
            }
        })?;

        serde_json::from_str(&config_content).map_err(|e| GikError::EmbeddingProviderUnavailable {
            provider: "candle".to_string(),
            reason: format!("Failed to parse config.json: {}", e),
        })
    }

    /// Detect the model architecture from this config.
    pub fn detect_architecture(&self) -> Result<ModelArchitecture, GikError> {
        if self.model_type.is_empty() {
            return Err(GikError::EmbeddingProviderUnavailable {
                provider: "candle".to_string(),
                reason: "config.json missing 'model_type' field".to_string(),
            });
        }

        self.model_type
            .parse()
            .map_err(|e: String| GikError::UnsupportedModelArchitecture {
                architecture: self.model_type.clone(),
                details: e,
            })
    }

    /// Get the effective pad token ID.
    pub fn effective_pad_token_id(&self, arch: ModelArchitecture) -> u32 {
        self.pad_token_id
            .unwrap_or_else(|| arch.default_pad_token_id())
    }
}

// ============================================================================
// EmbeddingProviderKind
// ============================================================================

/// Supported embedding provider backends.
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum EmbeddingProviderKind {
    /// Candle-based local embedding using a downloaded model.
    #[default]
    Candle,

    /// Ollama-based local embedding via HTTP API.
    Ollama,

    /// Other/custom provider (for extensibility).
    #[serde(untagged)]
    Other(String),
}

impl fmt::Display for EmbeddingProviderKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Candle => write!(f, "candle"),
            Self::Ollama => write!(f, "ollama"),
            Self::Other(s) => write!(f, "{}", s),
        }
    }
}

impl FromStr for EmbeddingProviderKind {
    type Err = std::convert::Infallible;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(match s.to_lowercase().as_str() {
            "candle" => Self::Candle,
            "ollama" => Self::Ollama,
            other => Self::Other(other.to_string()),
        })
    }
}

// ============================================================================
// EmbeddingModelId
// ============================================================================

/// Identifier for an embedding model.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct EmbeddingModelId(pub String);

impl EmbeddingModelId {
    /// Create a new model ID.
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    /// Get the model ID as a string slice.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Get the default model ID (all-MiniLM-L6-v2).
    pub fn default_model() -> Self {
        Self(DEFAULT_MODEL_ID.to_string())
    }
}

impl Default for EmbeddingModelId {
    fn default() -> Self {
        Self::default_model()
    }
}

impl fmt::Display for EmbeddingModelId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl FromStr for EmbeddingModelId {
    type Err = std::convert::Infallible;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(Self(s.to_string()))
    }
}

impl AsRef<str> for EmbeddingModelId {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

// ============================================================================
// EmbeddingConfig
// ============================================================================

/// Configuration for an embedding provider and model.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EmbeddingConfig {
    /// The embedding provider backend.
    pub provider: EmbeddingProviderKind,

    /// The model identifier (e.g., Hugging Face model ID).
    pub model_id: EmbeddingModelId,

    /// Model architecture (BERT, RoBERTa, etc.).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub architecture: Option<ModelArchitecture>,

    /// Vector dimension produced by this model (if known).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dimension: Option<u32>,

    /// Maximum tokens the model accepts (if known).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_tokens: Option<u32>,

    /// Local path to the model files (for Candle backend).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub local_path: Option<PathBuf>,
}

impl Default for EmbeddingConfig {
    fn default() -> Self {
        let local_path = dirs::home_dir()
            .map(|home| home.join(".gik").join(DEFAULT_MODEL_PATH))
            .or_else(|| Some(PathBuf::from(DEFAULT_MODEL_PATH)));

        Self {
            provider: EmbeddingProviderKind::Candle,
            model_id: EmbeddingModelId::default_model(),
            architecture: Some(ModelArchitecture::Bert),
            dimension: Some(DEFAULT_DIMENSION),
            max_tokens: Some(DEFAULT_MAX_TOKENS),
            local_path,
        }
    }
}

impl EmbeddingConfig {
    /// Create a new embedding config with the specified provider and model.
    pub fn new(provider: EmbeddingProviderKind, model_id: impl Into<String>) -> Self {
        Self {
            provider,
            model_id: EmbeddingModelId::new(model_id),
            architecture: None,
            dimension: None,
            max_tokens: None,
            local_path: None,
        }
    }

    /// Set the architecture.
    pub fn with_architecture(mut self, arch: ModelArchitecture) -> Self {
        self.architecture = Some(arch);
        self
    }

    /// Set the dimension.
    pub fn with_dimension(mut self, dim: u32) -> Self {
        self.dimension = Some(dim);
        self
    }

    /// Set the max tokens.
    pub fn with_max_tokens(mut self, tokens: u32) -> Self {
        self.max_tokens = Some(tokens);
        self
    }

    /// Set the local path.
    pub fn with_local_path(mut self, path: impl Into<PathBuf>) -> Self {
        self.local_path = Some(path.into());
        self
    }
}

// ============================================================================
// BaseEmbeddingConfig
// ============================================================================

/// Resolved embedding configuration for a specific knowledge base.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BaseEmbeddingConfig {
    /// The knowledge base name (e.g., "code", "docs").
    pub base: String,

    /// The resolved embedding configuration.
    pub config: EmbeddingConfig,
}

// ============================================================================
// ModelInfo
// ============================================================================

/// On-disk metadata about the embedding model used for a knowledge base's index.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelInfo {
    /// The provider that created this index.
    pub provider: String,

    /// The model ID used for embeddings.
    pub model_id: String,

    /// Vector dimension of the embeddings.
    pub dimension: u32,

    /// When this base was first indexed with this model.
    pub created_at: DateTime<Utc>,

    /// When this base was last reindexed.
    pub last_reindexed_at: DateTime<Utc>,
}

impl ModelInfo {
    /// Create a new ModelInfo with the current timestamp.
    pub fn new(provider: impl Into<String>, model_id: impl Into<String>, dimension: u32) -> Self {
        let now = Utc::now();
        Self {
            provider: provider.into(),
            model_id: model_id.into(),
            dimension,
            created_at: now,
            last_reindexed_at: now,
        }
    }

    /// Create ModelInfo from an EmbeddingConfig.
    pub fn from_config(config: &EmbeddingConfig) -> Self {
        Self::new(
            config.provider.to_string(),
            config.model_id.as_str(),
            config.dimension.unwrap_or(DEFAULT_DIMENSION),
        )
    }

    /// Update the last_reindexed_at timestamp.
    pub fn touch_reindex(&mut self) {
        self.last_reindexed_at = Utc::now();
    }
}

// ============================================================================
// ModelCompatibility
// ============================================================================

/// Result of comparing the configured embedding model with the stored model-info.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ModelCompatibility {
    /// The configured model matches the stored model-info.
    Compatible,

    /// No model-info exists (base has not been indexed yet).
    MissingModelInfo,

    /// The configured model differs from the stored model-info.
    Mismatch {
        configured: EmbeddingConfig,
        stored: ModelInfo,
    },
}

impl ModelCompatibility {
    /// Returns true if the model is compatible.
    pub fn is_compatible(&self) -> bool {
        matches!(self, Self::Compatible | Self::MissingModelInfo)
    }

    /// Returns true if there is a mismatch requiring reindex.
    pub fn is_mismatch(&self) -> bool {
        matches!(self, Self::Mismatch { .. })
    }
}

/// Check compatibility between an embedding config and stored model-info.
pub fn check_model_compatibility(
    config: &EmbeddingConfig,
    model_info: Option<&ModelInfo>,
) -> ModelCompatibility {
    match model_info {
        None => ModelCompatibility::MissingModelInfo,
        Some(info) => {
            let provider_matches = config.provider.to_string() == info.provider;
            let model_matches = config.model_id.as_str() == info.model_id;

            if provider_matches && model_matches {
                ModelCompatibility::Compatible
            } else {
                ModelCompatibility::Mismatch {
                    configured: config.clone(),
                    stored: info.clone(),
                }
            }
        }
    }
}

/// Get the default embedding configuration for a knowledge base.
pub fn default_embedding_config_for_base(_base: &str) -> EmbeddingConfig {
    EmbeddingConfig::default()
}

// ============================================================================
// EmbeddingBackend Trait
// ============================================================================

/// Trait for embedding providers.
pub trait EmbeddingBackend: Send + Sync {
    /// Get the provider kind for this backend.
    fn provider_kind(&self) -> EmbeddingProviderKind;

    /// Get the model ID this backend uses.
    fn model_id(&self) -> &EmbeddingModelId;

    /// Get the embedding dimension.
    fn dimension(&self) -> u32;

    /// Embed a batch of text inputs.
    fn embed_batch(&self, inputs: &[String]) -> Result<Vec<Vec<f32>>, GikError>;

    /// Embed a single text input.
    fn embed(&self, input: &str) -> Result<Vec<f32>, GikError> {
        let results = self.embed_batch(&[input.to_string()])?;
        results
            .into_iter()
            .next()
            .ok_or_else(|| GikError::EmbeddingConfigError {
                message: "embed_batch returned empty results".to_string(),
            })
    }

    /// Warm up the embedding backend.
    fn warm_up(&self) -> Result<(), GikError> {
        let _ = self.embed("warm-up")?;
        Ok(())
    }
}

// ============================================================================
// CandleEmbeddingBackend (wrapper around gik-model)
// ============================================================================

/// Candle-based embedding backend using local model files.
///
/// This is a wrapper around `gik_model::CandleEmbeddingModel` that implements
/// the gik-core `EmbeddingBackend` trait. The actual ML implementation is in
/// the `gik-model` crate.
///
/// ## Supported Models
///
/// - **BERT family**: sentence-transformers/all-MiniLM-L6-v2, bert-base-uncased, etc.
/// - **RoBERTa family**: microsoft/codebert-base, roberta-base, xlm-roberta-base, etc.
///
/// ## Model Setup
///
/// Clone the model from Hugging Face:
///
/// ```bash
/// git clone https://huggingface.co/sentence-transformers/all-MiniLM-L6-v2 \
///     ~/.gik/models/embeddings/all-MiniLM-L6-v2
/// ```
pub struct CandleEmbeddingBackend {
    inner: crate::model_adapter::ModelEmbeddingBackend,
    config: EmbeddingConfig,
}

impl CandleEmbeddingBackend {
    /// Create a new Candle embedding backend.
    ///
    /// Loads the model and tokenizer from the configured local path.
    pub fn new(config: EmbeddingConfig, device_pref: DevicePreference) -> Result<Self, GikError> {
        let inner =
            crate::model_adapter::ModelEmbeddingBackend::from_core_config(&config, device_pref)?;
        Ok(Self { inner, config })
    }

    /// Get the embedding configuration.
    pub fn config(&self) -> &EmbeddingConfig {
        &self.config
    }
}

impl EmbeddingBackend for CandleEmbeddingBackend {
    fn provider_kind(&self) -> EmbeddingProviderKind {
        self.inner.provider_kind()
    }

    fn model_id(&self) -> &EmbeddingModelId {
        self.inner.model_id()
    }

    fn dimension(&self) -> u32 {
        self.inner.dimension()
    }

    fn embed_batch(&self, inputs: &[String]) -> Result<Vec<Vec<f32>>, GikError> {
        self.inner.embed_batch(inputs)
    }

    fn warm_up(&self) -> Result<(), GikError> {
        self.inner.warm_up()
    }
}

// ============================================================================
// Backend Factory
// ============================================================================

/// Create an embedding backend from configuration.
///
/// This function delegates to gik-model for the actual backend implementation.
/// The gik-model crate handles the heavy ML dependencies (Candle, tokenizers, etc.).
pub fn create_backend(
    config: &EmbeddingConfig,
    device_pref: DevicePreference,
) -> Result<Box<dyn EmbeddingBackend>, GikError> {
    match &config.provider {
        EmbeddingProviderKind::Candle => {
            let backend = CandleEmbeddingBackend::new(config.clone(), device_pref)?;
            Ok(Box::new(backend))
        }
        EmbeddingProviderKind::Ollama => Err(GikError::EmbeddingProviderUnavailable {
            provider: "ollama".to_string(),
            reason: "Ollama backend is not yet implemented.".to_string(),
        }),
        EmbeddingProviderKind::Other(name) => Err(GikError::EmbeddingProviderUnavailable {
            provider: name.clone(),
            reason: format!("Unknown embedding provider: {}", name),
        }),
    }
}

// ============================================================================
// Model Info I/O
// ============================================================================

/// Read model-info from a JSON file.
pub fn read_model_info(path: &std::path::Path) -> Result<Option<ModelInfo>, GikError> {
    if !path.exists() {
        return Ok(None);
    }

    let content = std::fs::read_to_string(path).map_err(|e| GikError::EmbeddingModelInfoIo {
        path: path.to_path_buf(),
        message: e.to_string(),
    })?;

    let info: ModelInfo =
        serde_json::from_str(&content).map_err(|e| GikError::EmbeddingModelInfoParse {
            path: path.to_path_buf(),
            message: e.to_string(),
        })?;

    Ok(Some(info))
}

/// Write model-info to a JSON file.
pub fn write_model_info(path: &std::path::Path, info: &ModelInfo) -> Result<(), GikError> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| GikError::EmbeddingModelInfoIo {
            path: path.to_path_buf(),
            message: format!("Failed to create parent directory: {}", e),
        })?;
    }

    let content =
        serde_json::to_string_pretty(info).map_err(|e| GikError::EmbeddingModelInfoParse {
            path: path.to_path_buf(),
            message: e.to_string(),
        })?;

    std::fs::write(path, content).map_err(|e| GikError::EmbeddingModelInfoIo {
        path: path.to_path_buf(),
        message: e.to_string(),
    })?;

    Ok(())
}

// ============================================================================
// Test-only Mock Backend
// ============================================================================

/// A mock embedding backend for testing.
#[cfg(test)]
pub struct MockEmbeddingBackend {
    config: EmbeddingConfig,
    model_id: EmbeddingModelId,
    dimension: u32,
}

#[cfg(test)]
impl MockEmbeddingBackend {
    /// Create a new mock embedding backend.
    pub fn new(config: EmbeddingConfig) -> Self {
        let model_id = config.model_id.clone();
        let dimension = config.dimension.unwrap_or(DEFAULT_DIMENSION);
        Self {
            config,
            model_id,
            dimension,
        }
    }

    /// Get the embedding configuration.
    pub fn config(&self) -> &EmbeddingConfig {
        &self.config
    }

    fn hash_to_embedding(&self, content: &str) -> Vec<f32> {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};

        let mut hasher = DefaultHasher::new();
        content.hash(&mut hasher);
        let seed = hasher.finish();

        let mut embedding = Vec::with_capacity(self.dimension as usize);
        let mut state = seed;

        for _ in 0..self.dimension {
            state = state.wrapping_mul(6364136223846793005).wrapping_add(1);
            let value = ((state >> 33) as f32 / (u32::MAX as f32 / 2.0)) - 1.0;
            embedding.push(value);
        }

        let norm: f32 = embedding.iter().map(|x| x * x).sum::<f32>().sqrt();
        if norm > 0.0 {
            for x in &mut embedding {
                *x /= norm;
            }
        }

        embedding
    }
}

#[cfg(test)]
impl EmbeddingBackend for MockEmbeddingBackend {
    fn provider_kind(&self) -> EmbeddingProviderKind {
        EmbeddingProviderKind::Other("mock".to_string())
    }

    fn model_id(&self) -> &EmbeddingModelId {
        &self.model_id
    }

    fn dimension(&self) -> u32 {
        self.dimension
    }

    fn embed_batch(&self, inputs: &[String]) -> Result<Vec<Vec<f32>>, GikError> {
        Ok(inputs.iter().map(|s| self.hash_to_embedding(s)).collect())
    }
}

/// Create a mock embedding backend for testing.
#[cfg(test)]
pub fn create_mock_backend(config: &EmbeddingConfig) -> Box<dyn EmbeddingBackend> {
    Box::new(MockEmbeddingBackend::new(config.clone()))
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_provider_kind_display() {
        assert_eq!(EmbeddingProviderKind::Candle.to_string(), "candle");
        assert_eq!(EmbeddingProviderKind::Ollama.to_string(), "ollama");
        assert_eq!(
            EmbeddingProviderKind::Other("custom".to_string()).to_string(),
            "custom"
        );
    }

    #[test]
    fn test_provider_kind_from_str() {
        assert_eq!(
            "candle".parse::<EmbeddingProviderKind>().unwrap(),
            EmbeddingProviderKind::Candle
        );
        assert_eq!(
            "CANDLE".parse::<EmbeddingProviderKind>().unwrap(),
            EmbeddingProviderKind::Candle
        );
        assert_eq!(
            "ollama".parse::<EmbeddingProviderKind>().unwrap(),
            EmbeddingProviderKind::Ollama
        );
    }

    #[test]
    fn test_model_id_default() {
        let id = EmbeddingModelId::default();
        assert_eq!(id.as_str(), DEFAULT_MODEL_ID);
    }

    #[test]
    fn test_embedding_config_default() {
        let config = EmbeddingConfig::default();
        assert_eq!(config.provider, EmbeddingProviderKind::Candle);
        assert_eq!(config.model_id.as_str(), DEFAULT_MODEL_ID);
        assert_eq!(config.dimension, Some(DEFAULT_DIMENSION));
    }

    #[test]
    fn test_model_info_new() {
        let info = ModelInfo::new("candle", "test-model", 384);
        assert_eq!(info.provider, "candle");
        assert_eq!(info.model_id, "test-model");
        assert_eq!(info.dimension, 384);
    }

    #[test]
    fn test_compatibility_missing() {
        let config = EmbeddingConfig::default();
        let result = check_model_compatibility(&config, None);
        assert_eq!(result, ModelCompatibility::MissingModelInfo);
        assert!(result.is_compatible());
    }

    #[test]
    fn test_compatibility_compatible() {
        let config = EmbeddingConfig::default();
        let info = ModelInfo::from_config(&config);
        let result = check_model_compatibility(&config, Some(&info));
        assert_eq!(result, ModelCompatibility::Compatible);
    }

    #[test]
    fn test_read_model_info_missing() {
        let temp = TempDir::new().unwrap();
        let path = temp.path().join("model-info.json");
        let result = read_model_info(&path).unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn test_write_and_read_model_info() {
        let temp = TempDir::new().unwrap();
        let path = temp.path().join("base/model-info.json");

        let info = ModelInfo::new("candle", "test-model", 384);
        write_model_info(&path, &info).unwrap();

        let loaded = read_model_info(&path).unwrap().unwrap();
        assert_eq!(loaded.provider, info.provider);
        assert_eq!(loaded.model_id, info.model_id);
    }

    #[test]
    fn test_mock_backend_embedding_dimension() {
        let config = EmbeddingConfig::default();
        let backend = MockEmbeddingBackend::new(config);

        assert_eq!(backend.dimension(), DEFAULT_DIMENSION);

        let embeddings = backend.embed_batch(&["test".to_string()]).unwrap();
        assert_eq!(embeddings[0].len(), DEFAULT_DIMENSION as usize);
    }

    #[test]
    fn test_mock_backend_deterministic() {
        let config = EmbeddingConfig::default();
        let backend = MockEmbeddingBackend::new(config);

        let emb1 = backend.embed("hello world").unwrap();
        let emb2 = backend.embed("hello world").unwrap();
        let emb3 = backend.embed("different text").unwrap();

        assert_eq!(emb1, emb2);
        assert_ne!(emb1, emb3);
    }

    #[test]
    fn test_create_mock_backend() {
        let config = EmbeddingConfig::default();
        let backend = create_mock_backend(&config);

        assert_eq!(backend.dimension(), DEFAULT_DIMENSION);
        let result = backend.embed("test");
        assert!(result.is_ok());
    }
}
