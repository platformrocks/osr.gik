//! Configuration types for gik-model.
//!
//! This module provides the canonical configuration types for embedding and
//! reranker models. These types are the single source of truth - other crates
//! should use or re-export these types rather than defining duplicates.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

use crate::model_locator::ModelLocator;
use crate::{DEFAULT_EMBEDDING_MODEL_ID, DEFAULT_RERANKER_MODEL_ID};

// ============================================================================
// Helper functions
// ============================================================================

/// Extract the model name from a full model ID.
///
/// E.g., "sentence-transformers/all-MiniLM-L6-v2" → "all-MiniLM-L6-v2"
fn extract_model_name(model_id: &str) -> &str {
    model_id.rsplit('/').next().unwrap_or(model_id)
}

// ============================================================================
// DevicePreference
// ============================================================================

/// Preference for compute device.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DevicePreference {
    /// Auto-select best device (GPU if available, else CPU).
    #[default]
    Auto,
    /// Force GPU (Metal on macOS, CUDA on Linux).
    Gpu,
    /// Force CPU only.
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
                "Unknown device: '{}'. Use 'auto', 'gpu', or 'cpu'.",
                s
            )),
        }
    }
}

// ============================================================================
// EmbeddingProviderKind
// ============================================================================

/// Embedding provider type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum EmbeddingProviderKind {
    /// Local Candle inference.
    #[default]
    Candle,
    /// Remote Ollama API.
    Ollama,
}

impl std::fmt::Display for EmbeddingProviderKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Candle => write!(f, "candle"),
            Self::Ollama => write!(f, "ollama"),
        }
    }
}

impl std::str::FromStr for EmbeddingProviderKind {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "candle" | "local" | "embedded" => Ok(Self::Candle),
            "ollama" => Ok(Self::Ollama),
            _ => Err(format!(
                "Unknown provider: '{}'. Use 'candle' or 'ollama'.",
                s
            )),
        }
    }
}

// ============================================================================
// ModelArchitecture
// ============================================================================

/// Model architecture type.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ModelArchitecture {
    #[default]
    Bert,
    Roberta,
    Mpnet,
    Unknown,
}

impl std::fmt::Display for ModelArchitecture {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Bert => write!(f, "bert"),
            Self::Roberta => write!(f, "roberta"),
            Self::Mpnet => write!(f, "mpnet"),
            Self::Unknown => write!(f, "unknown"),
        }
    }
}

impl std::str::FromStr for ModelArchitecture {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "bert" => Ok(Self::Bert),
            "roberta" => Ok(Self::Roberta),
            "mpnet" => Ok(Self::Mpnet),
            _ => Ok(Self::Unknown),
        }
    }
}

// ============================================================================
// ModelInfo
// ============================================================================

/// Information about a loaded model.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelInfo {
    /// Model identifier.
    pub model_id: String,
    /// Embedding dimension.
    pub dimension: usize,
    /// Maximum sequence length.
    pub max_seq_len: usize,
    /// Model architecture.
    #[serde(default)]
    pub architecture: ModelArchitecture,
}

impl ModelInfo {
    /// Create new model info.
    pub fn new(model_id: impl Into<String>, dimension: usize, max_seq_len: usize) -> Self {
        Self {
            model_id: model_id.into(),
            dimension,
            max_seq_len,
            architecture: ModelArchitecture::default(),
        }
    }

    /// Set architecture.
    pub fn with_architecture(mut self, arch: ModelArchitecture) -> Self {
        self.architecture = arch;
        self
    }
}

// ============================================================================
// EmbeddingConfig
// ============================================================================

/// Configuration for embedding model.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmbeddingConfig {
    /// Provider type.
    #[serde(default)]
    pub provider: EmbeddingProviderKind,

    /// Model ID (e.g., "sentence-transformers/all-MiniLM-L6-v2").
    #[serde(default = "default_embedding_model_id")]
    pub model_id: String,

    /// Device preference.
    #[serde(default)]
    pub device: DevicePreference,

    /// Local path to model files.
    /// If None, uses bundled model or ~/.gik/models path.
    #[serde(default)]
    pub local_path: Option<PathBuf>,

    /// Maximum sequence length.
    #[serde(default = "default_max_seq_len")]
    pub max_sequence_length: usize,

    /// Batch size for embedding.
    #[serde(default = "default_batch_size")]
    pub batch_size: usize,
}

fn default_embedding_model_id() -> String {
    DEFAULT_EMBEDDING_MODEL_ID.to_string()
}

fn default_max_seq_len() -> usize {
    512
}

fn default_batch_size() -> usize {
    32
}

impl Default for EmbeddingConfig {
    fn default() -> Self {
        Self {
            provider: EmbeddingProviderKind::default(),
            model_id: default_embedding_model_id(),
            device: DevicePreference::default(),
            local_path: None,
            max_sequence_length: default_max_seq_len(),
            batch_size: default_batch_size(),
        }
    }
}

impl EmbeddingConfig {
    /// Resolve the effective model path using ModelLocator.
    ///
    /// Priority:
    /// 1. Explicit `local_path` if set
    /// 2. ModelLocator search order ($GIK_MODELS_DIR → ~/.gik/models → {exe}/models)
    ///
    /// Returns the path even if it doesn't exist (caller should validate).
    pub fn effective_model_path(&self) -> PathBuf {
        if let Some(ref path) = self.local_path {
            return path.clone();
        }

        // Use ModelLocator to find the model
        let locator = ModelLocator::new();
        match locator.embedding_model_path(&self.model_id) {
            Ok(path) => path,
            Err(_) => {
                // Fall back to default location if locator fails
                dirs::home_dir()
                    .unwrap_or_else(|| PathBuf::from("."))
                    .join(".gik")
                    .join("models")
                    .join("embeddings")
                    .join(extract_model_name(&self.model_id))
            }
        }
    }

    /// Create a config with a specific local path.
    pub fn with_local_path(mut self, path: impl Into<PathBuf>) -> Self {
        self.local_path = Some(path.into());
        self
    }

    /// Create a config with a specific model ID.
    pub fn with_model_id(mut self, model_id: impl Into<String>) -> Self {
        self.model_id = model_id.into();
        self
    }
}

// ============================================================================
// RerankerConfig
// ============================================================================

/// Configuration for reranker model.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RerankerConfig {
    /// Whether reranking is enabled.
    #[serde(default = "default_true")]
    pub enabled: bool,

    /// Model ID (e.g., "cross-encoder/ms-marco-MiniLM-L6-v2").
    #[serde(default = "default_reranker_model_id")]
    pub model_id: String,

    /// Device preference.
    #[serde(default)]
    pub device: DevicePreference,

    /// Local path to model files.
    #[serde(default)]
    pub local_path: Option<PathBuf>,

    /// Number of candidates to rerank.
    #[serde(default = "default_top_k")]
    pub top_k: usize,

    /// Number of final results.
    #[serde(default = "default_final_k")]
    pub final_k: usize,
}

fn default_true() -> bool {
    true
}

fn default_reranker_model_id() -> String {
    DEFAULT_RERANKER_MODEL_ID.to_string()
}

fn default_top_k() -> usize {
    30
}

fn default_final_k() -> usize {
    5
}

impl Default for RerankerConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            model_id: default_reranker_model_id(),
            device: DevicePreference::default(),
            local_path: None,
            top_k: default_top_k(),
            final_k: default_final_k(),
        }
    }
}

impl RerankerConfig {
    /// Resolve the effective model path using ModelLocator.
    ///
    /// Priority:
    /// 1. Explicit `local_path` if set
    /// 2. ModelLocator search order ($GIK_MODELS_DIR → ~/.gik/models → {exe}/models)
    ///
    /// Returns the path even if it doesn't exist (caller should validate).
    pub fn effective_model_path(&self) -> PathBuf {
        if let Some(ref path) = self.local_path {
            return path.clone();
        }

        // Use ModelLocator to find the model
        let locator = ModelLocator::new();
        match locator.reranker_model_path(&self.model_id) {
            Ok(path) => path,
            Err(_) => {
                // Fall back to default location if locator fails
                dirs::home_dir()
                    .unwrap_or_else(|| PathBuf::from("."))
                    .join(".gik")
                    .join("models")
                    .join("rerankers")
                    .join(extract_model_name(&self.model_id))
            }
        }
    }

    /// Create a config with a specific local path.
    pub fn with_local_path(mut self, path: impl Into<PathBuf>) -> Self {
        self.local_path = Some(path.into());
        self
    }

    /// Create a config with a specific model ID.
    pub fn with_model_id(mut self, model_id: impl Into<String>) -> Self {
        self.model_id = model_id.into();
        self
    }
}

// ============================================================================
// HuggingFaceModelConfig
// ============================================================================

/// Config.json structure from HuggingFace models.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HuggingFaceModelConfig {
    #[serde(default)]
    pub architectures: Vec<String>,
    #[serde(default)]
    pub hidden_size: usize,
    #[serde(default = "default_max_position")]
    pub max_position_embeddings: usize,
    #[serde(default)]
    pub model_type: String,
}

fn default_max_position() -> usize {
    512
}

impl HuggingFaceModelConfig {
    /// Infer architecture from config.
    pub fn infer_architecture(&self) -> ModelArchitecture {
        for arch in &self.architectures {
            let lower = arch.to_lowercase();
            if lower.contains("roberta") {
                return ModelArchitecture::Roberta;
            }
            if lower.contains("mpnet") {
                return ModelArchitecture::Mpnet;
            }
            if lower.contains("bert") {
                return ModelArchitecture::Bert;
            }
        }

        match self.model_type.to_lowercase().as_str() {
            "bert" => ModelArchitecture::Bert,
            "roberta" | "xlm-roberta" => ModelArchitecture::Roberta,
            "mpnet" => ModelArchitecture::Mpnet,
            _ => ModelArchitecture::Unknown,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_device_preference_parsing() {
        assert_eq!(
            "auto".parse::<DevicePreference>().unwrap(),
            DevicePreference::Auto
        );
        assert_eq!(
            "gpu".parse::<DevicePreference>().unwrap(),
            DevicePreference::Gpu
        );
        assert_eq!(
            "cpu".parse::<DevicePreference>().unwrap(),
            DevicePreference::Cpu
        );
    }

    #[test]
    fn test_embedding_config_default() {
        let config = EmbeddingConfig::default();
        assert_eq!(config.provider, EmbeddingProviderKind::Candle);
        assert_eq!(config.model_id, DEFAULT_EMBEDDING_MODEL_ID);
    }

    #[test]
    fn test_reranker_config_default() {
        let config = RerankerConfig::default();
        assert!(config.enabled);
        assert_eq!(config.model_id, DEFAULT_RERANKER_MODEL_ID);
    }

    #[test]
    fn test_hf_config_infer_architecture() {
        let config = HuggingFaceModelConfig {
            architectures: vec!["BertModel".to_string()],
            hidden_size: 384,
            max_position_embeddings: 512,
            model_type: "bert".to_string(),
        };
        assert_eq!(config.infer_architecture(), ModelArchitecture::Bert);

        let roberta = HuggingFaceModelConfig {
            architectures: vec!["RobertaModel".to_string()],
            model_type: "roberta".to_string(),
            ..config.clone()
        };
        assert_eq!(roberta.infer_architecture(), ModelArchitecture::Roberta);
    }
}
