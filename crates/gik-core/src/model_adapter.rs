//! Adapter layer for gik-model infrastructure.
//!
//! This module bridges gik-model implementations with gik-core's domain types.
//! It provides:
//!
//! - Error conversion from `ModelError` to `GikError`
//! - Type conversion utilities between gik-model and gik-core types
//! - Wrapper types that implement gik-core traits using gik-model backends
//!
//! ## Architecture
//!
//! ```text
//! gik-core domain code (engine, reindex, ask, commit)
//!        ↓
//!   model_adapter (this module) - wrappers + conversions
//!        ↓
//!     gik-model implementations (Candle embeddings/reranking)
//! ```
//!
//! ## Migration Status (Phase 4)
//!
//! This module replaces the embedding portions of db_adapter. The gik-model
//! crate now owns all ML inference (embeddings + reranking), while gik-db
//! owns storage (vectors + KG).

use crate::config::DevicePreference as CoreDevicePreference;
use crate::errors::GikError;

// ============================================================================
// Error Conversion
// ============================================================================

/// Convert a gik-model error to a gik-core error.
pub fn from_model_error(err: gik_model::ModelError) -> GikError {
    use gik_model::ModelError;

    match err {
        ModelError::ModelsDirectoryNotFound { searched } => {
            let paths = searched
                .iter()
                .map(|p| p.display().to_string())
                .collect::<Vec<_>>()
                .join(", ");
            GikError::EmbeddingProviderUnavailable {
                provider: "model-locator".to_string(),
                reason: format!("Models directory not found. Searched: {}", paths),
            }
        }

        ModelError::ModelNotFound { model_id, path } => GikError::EmbeddingProviderUnavailable {
            provider: model_id,
            reason: format!("Model not found at {:?}", path),
        },

        ModelError::IncompleteModelFiles { path, missing } => {
            GikError::EmbeddingProviderUnavailable {
                provider: path.display().to_string(),
                reason: format!("Missing model files: {}", missing.join(", ")),
            }
        }

        ModelError::ModelLoad { model_id, message } => GikError::EmbeddingProviderUnavailable {
            provider: model_id,
            reason: message,
        },

        ModelError::InvalidConfig { message } => GikError::EmbeddingConfigError { message },

        ModelError::Tokenization { message } => GikError::EmbeddingProviderUnavailable {
            provider: "tokenizer".to_string(),
            reason: message,
        },

        ModelError::EmbeddingFailed { model_id, message } => {
            GikError::EmbeddingProviderUnavailable {
                provider: model_id,
                reason: message,
            }
        }

        ModelError::RerankingFailed { model_id, message } => GikError::RerankerInferenceFailed {
            model_id,
            reason: message,
        },

        ModelError::ProviderNotAvailable { provider, reason } => {
            GikError::EmbeddingProviderUnavailable { provider, reason }
        }

        ModelError::DeviceNotAvailable { reason } => GikError::EmbeddingProviderUnavailable {
            provider: "device".to_string(),
            reason,
        },

        ModelError::Io(io_err) => GikError::Io(io_err),

        ModelError::Json(json_err) => GikError::EmbeddingConfigError {
            message: json_err.to_string(),
        },
    }
}

/// Extension trait to convert gik-model Result to Result<T, GikError>.
pub trait IntoGikResult<T> {
    /// Convert a gik-model result to a GikError result.
    fn into_gik_result(self) -> Result<T, GikError>;
}

impl<T> IntoGikResult<T> for Result<T, gik_model::ModelError> {
    fn into_gik_result(self) -> Result<T, GikError> {
        self.map_err(from_model_error)
    }
}

// ============================================================================
// Config Conversion
// ============================================================================

/// Convert gik-core DevicePreference to gik-model DevicePreference.
pub fn to_model_device_preference(pref: CoreDevicePreference) -> gik_model::DevicePreference {
    match pref {
        CoreDevicePreference::Auto => gik_model::DevicePreference::Auto,
        CoreDevicePreference::Gpu => gik_model::DevicePreference::Gpu,
        CoreDevicePreference::Cpu => gik_model::DevicePreference::Cpu,
    }
}

/// Convert gik-core EmbeddingConfig to gik-model EmbeddingConfig.
pub fn to_model_embedding_config(
    config: &crate::embedding::EmbeddingConfig,
    device_pref: CoreDevicePreference,
) -> gik_model::EmbeddingConfig {
    gik_model::EmbeddingConfig {
        provider: gik_model::EmbeddingProviderKind::Candle,
        model_id: config.model_id.as_ref().to_string(),
        device: to_model_device_preference(device_pref),
        local_path: config.local_path.clone(),
        max_sequence_length: config.max_tokens.map(|t| t as usize).unwrap_or(512),
        batch_size: 32,
    }
}

/// Convert gik-core RerankerConfig to gik-model RerankerConfig.
pub fn to_model_reranker_config(
    config: &crate::config::RerankerConfig,
    device_pref: CoreDevicePreference,
) -> gik_model::RerankerConfig {
    gik_model::RerankerConfig {
        enabled: config.enabled,
        model_id: config.model_id.clone(),
        device: to_model_device_preference(device_pref),
        local_path: config.local_path.clone(),
        top_k: config.top_k,
        final_k: config.final_k,
    }
}

// ============================================================================
// Embedding Backend Wrapper
// ============================================================================

use crate::embedding::{
    EmbeddingBackend as CoreEmbeddingBackend, EmbeddingConfig as CoreEmbeddingConfig,
    EmbeddingModelId, EmbeddingProviderKind,
};

/// Wrapper around gik-model embedding backend for gik-core.
///
/// This adapts the gik-model `EmbeddingModel` trait to match the gik-core
/// `EmbeddingBackend` trait signature.
pub struct ModelEmbeddingBackend {
    inner: Box<dyn gik_model::EmbeddingModel>,
    model_id: EmbeddingModelId,
}

impl std::fmt::Debug for ModelEmbeddingBackend {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ModelEmbeddingBackend")
            .field("model_id", &self.model_id)
            .field("dimension", &self.inner.dimension())
            .finish()
    }
}

impl ModelEmbeddingBackend {
    /// Create a new wrapper from a gik-model embedding model.
    pub fn new(model: Box<dyn gik_model::EmbeddingModel>) -> Self {
        let model_id = EmbeddingModelId::new(model.model_id().to_string());
        Self {
            inner: model,
            model_id,
        }
    }

    /// Create a wrapper from gik-core configuration.
    ///
    /// This converts the gik-core EmbeddingConfig to gik-model format and creates
    /// the backend using gik-model's infrastructure.
    pub fn from_core_config(
        config: &CoreEmbeddingConfig,
        device_pref: CoreDevicePreference,
    ) -> Result<Self, GikError> {
        let model_config = to_model_embedding_config(config, device_pref);
        let model = gik_model::create_embedding_model(&model_config).into_gik_result()?;
        Ok(Self::new(model))
    }
}

impl CoreEmbeddingBackend for ModelEmbeddingBackend {
    fn provider_kind(&self) -> EmbeddingProviderKind {
        EmbeddingProviderKind::Candle
    }

    fn model_id(&self) -> &EmbeddingModelId {
        &self.model_id
    }

    fn dimension(&self) -> u32 {
        self.inner.dimension() as u32
    }

    fn embed_batch(&self, inputs: &[String]) -> Result<Vec<Vec<f32>>, GikError> {
        let refs: Vec<&str> = inputs.iter().map(|s| s.as_str()).collect();
        self.inner.embed(&refs).into_gik_result()
    }

    fn warm_up(&self) -> Result<(), GikError> {
        self.inner.warm_up().into_gik_result()
    }
}

// ============================================================================
// Reranker Backend Wrapper
// ============================================================================

use crate::reranker::{
    RerankerBackend as CoreRerankerBackend, RerankerModelId, RerankerProviderKind,
};

/// Wrapper around gik-model reranker for gik-core.
///
/// This adapts the gik-model `RerankerModel` trait to match the gik-core
/// `RerankerBackend` trait signature.
pub struct ModelRerankerBackend {
    inner: Box<dyn gik_model::RerankerModel>,
    model_id: RerankerModelId,
}

impl std::fmt::Debug for ModelRerankerBackend {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ModelRerankerBackend")
            .field("model_id", &self.model_id)
            .finish()
    }
}

impl ModelRerankerBackend {
    /// Create a new wrapper from a gik-model reranker.
    pub fn new(model: Box<dyn gik_model::RerankerModel>) -> Self {
        let model_id = RerankerModelId::from(model.model_id().to_string());
        Self {
            inner: model,
            model_id,
        }
    }

    /// Create a wrapper from gik-core configuration.
    pub fn from_core_config(
        config: &crate::config::RerankerConfig,
        device_pref: CoreDevicePreference,
    ) -> Result<Self, GikError> {
        let model_config = to_model_reranker_config(config, device_pref);
        let model = gik_model::create_reranker_model(&model_config).into_gik_result()?;
        Ok(Self::new(model))
    }
}

impl CoreRerankerBackend for ModelRerankerBackend {
    fn provider_kind(&self) -> RerankerProviderKind {
        RerankerProviderKind::Candle
    }

    fn model_id(&self) -> &RerankerModelId {
        &self.model_id
    }

    fn score_batch(&self, query: &str, documents: &[String]) -> Result<Vec<f32>, GikError> {
        self.inner.score_batch(query, documents).into_gik_result()
    }
}

// ============================================================================
// Factory Functions (replacing embedding.rs and reranker.rs factories)
// ============================================================================

/// Create an embedding backend from gik-core configuration.
///
/// This is the primary factory function for creating embedding backends.
/// It delegates to gik-model for the actual ML implementation.
pub fn create_embedding_backend(
    config: &CoreEmbeddingConfig,
    device_pref: CoreDevicePreference,
) -> Result<Box<dyn CoreEmbeddingBackend>, GikError> {
    let backend = ModelEmbeddingBackend::from_core_config(config, device_pref)?;
    Ok(Box::new(backend))
}

/// Create a reranker backend from gik-core configuration.
///
/// This is the primary factory function for creating reranker backends.
/// It delegates to gik-model for the actual ML implementation.
pub fn create_reranker_backend(
    config: &crate::config::RerankerConfig,
    device_pref: CoreDevicePreference,
) -> Result<Box<dyn CoreRerankerBackend>, GikError> {
    let backend = ModelRerankerBackend::from_core_config(config, device_pref)?;
    Ok(Box::new(backend))
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_error_conversion_model_not_found() {
        let model_err = gik_model::ModelError::ModelNotFound {
            model_id: "test-model".to_string(),
            path: std::path::PathBuf::from("/path/to/model"),
        };
        let gik_err = from_model_error(model_err);

        match gik_err {
            GikError::EmbeddingProviderUnavailable { provider, reason } => {
                assert_eq!(provider, "test-model");
                assert!(reason.contains("/path/to/model"));
            }
            other => panic!("Expected EmbeddingProviderUnavailable, got {:?}", other),
        }
    }

    #[test]
    fn test_error_conversion_model_load() {
        let model_err = gik_model::ModelError::ModelLoad {
            model_id: "test-model".to_string(),
            message: "failed to load".to_string(),
        };
        let gik_err = from_model_error(model_err);

        match gik_err {
            GikError::EmbeddingProviderUnavailable { provider, reason } => {
                assert_eq!(provider, "test-model");
                assert_eq!(reason, "failed to load");
            }
            other => panic!("Expected EmbeddingProviderUnavailable, got {:?}", other),
        }
    }

    #[test]
    fn test_device_preference_conversion() {
        assert!(matches!(
            to_model_device_preference(CoreDevicePreference::Auto),
            gik_model::DevicePreference::Auto
        ));
        assert!(matches!(
            to_model_device_preference(CoreDevicePreference::Gpu),
            gik_model::DevicePreference::Gpu
        ));
        assert!(matches!(
            to_model_device_preference(CoreDevicePreference::Cpu),
            gik_model::DevicePreference::Cpu
        ));
    }

    #[test]
    fn test_embedding_config_conversion() {
        let core_config = crate::embedding::EmbeddingConfig::default();
        let model_config = to_model_embedding_config(&core_config, CoreDevicePreference::Auto);

        assert_eq!(model_config.model_id, core_config.model_id.as_ref());
        // max_sequence_length corresponds to max_tokens
        assert_eq!(
            model_config.max_sequence_length,
            core_config.max_tokens.map(|t| t as usize).unwrap_or(512)
        );
    }
}
