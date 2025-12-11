//! Embedding configuration bridge for gik-core.
//!
//! This module bridges gik-core's legacy configuration structures to the
//! canonical `gik_model` configuration types. It provides:
//!
//! - Conversion from `GlobalConfig`/`ProjectConfig` to `gik_model::EmbeddingConfig`
//! - Default configuration factories for shipped models
//! - Configuration resolution following the documented precedence order
//!
//! ## Resolution Precedence
//!
//! 1. Project per-base override (`.guided/knowledge/config.yaml`)
//! 2. Global per-base override (`~/.gik/config.yaml`)
//! 3. Global default (`~/.gik/config.yaml`)
//! 4. Hard-coded default (Candle + all-MiniLM-L6-v2)
//!
//! ## Usage
//!
//! ```ignore
//! use gik_core::embedding_config_bridge::{resolve_embedding_config, resolve_reranker_config};
//! use gik_core::config::{GlobalConfig, ProjectConfig};
//!
//! let global = GlobalConfig::load_default()?;
//! let project = ProjectConfig::load_from_workspace(workspace_root)?;
//!
//! let embedding_config = resolve_embedding_config("code", &global, &project);
//! let reranker_config = resolve_reranker_config(&global);
//! ```

use std::path::PathBuf;

use crate::config::{DevicePreference, GlobalConfig, ProjectConfig, RerankerConfig};
use crate::embedding::EmbeddingConfig as CoreEmbeddingConfig;

// Re-export gik_model types for convenience
pub use gik_model::{
    EmbeddingConfig as ModelEmbeddingConfig, RerankerConfig as ModelRerankerConfig,
};

// ============================================================================
// DevicePreference conversion
// ============================================================================

/// Convert gik-core DevicePreference to gik-model DevicePreference.
pub fn to_model_device_preference(pref: DevicePreference) -> gik_model::DevicePreference {
    match pref {
        DevicePreference::Auto => gik_model::DevicePreference::Auto,
        DevicePreference::Gpu => gik_model::DevicePreference::Gpu,
        DevicePreference::Cpu => gik_model::DevicePreference::Cpu,
    }
}

/// Convert gik-model DevicePreference to gik-core DevicePreference.
pub fn from_model_device_preference(pref: gik_model::DevicePreference) -> DevicePreference {
    match pref {
        gik_model::DevicePreference::Auto => DevicePreference::Auto,
        gik_model::DevicePreference::Gpu => DevicePreference::Gpu,
        gik_model::DevicePreference::Cpu => DevicePreference::Cpu,
    }
}

// ============================================================================
// EmbeddingConfig conversion
// ============================================================================

/// Convert a gik-core EmbeddingConfig to a gik-model EmbeddingConfig.
///
/// This allows using gik-model's inference infrastructure with configuration
/// resolved through gik-core's config system.
pub fn to_model_embedding_config(
    config: &CoreEmbeddingConfig,
    device_pref: DevicePreference,
) -> ModelEmbeddingConfig {
    ModelEmbeddingConfig {
        provider: gik_model::EmbeddingProviderKind::Candle,
        model_id: config.model_id.as_ref().to_string(),
        device: to_model_device_preference(device_pref),
        local_path: config.local_path.clone(),
        max_sequence_length: config.max_tokens.map(|t| t as usize).unwrap_or(512),
        batch_size: 32,
    }
}

/// Create a default gik-model EmbeddingConfig for the bundled embedding model.
///
/// Uses the shipped all-MiniLM-L6-v2 model with auto device selection.
pub fn default_embedding_config() -> ModelEmbeddingConfig {
    ModelEmbeddingConfig::default()
}

/// Create a gik-model EmbeddingConfig with a specific device preference.
pub fn default_embedding_config_with_device(device: DevicePreference) -> ModelEmbeddingConfig {
    ModelEmbeddingConfig {
        device: to_model_device_preference(device),
        ..ModelEmbeddingConfig::default()
    }
}

// ============================================================================
// RerankerConfig conversion
// ============================================================================

/// Convert a gik-core RerankerConfig to a gik-model RerankerConfig.
pub fn to_model_reranker_config(
    config: &RerankerConfig,
    device_pref: DevicePreference,
) -> ModelRerankerConfig {
    ModelRerankerConfig {
        enabled: config.enabled,
        model_id: config.model_id.clone(),
        device: to_model_device_preference(device_pref),
        local_path: config.local_path.clone(),
        top_k: config.top_k,
        final_k: config.final_k,
    }
}

/// Create a default gik-model RerankerConfig for the bundled reranker model.
///
/// Uses the shipped ms-marco-MiniLM-L6-v2 model with auto device selection.
pub fn default_reranker_config() -> ModelRerankerConfig {
    ModelRerankerConfig::default()
}

/// Create a gik-model RerankerConfig with a specific device preference.
pub fn default_reranker_config_with_device(device: DevicePreference) -> ModelRerankerConfig {
    ModelRerankerConfig {
        device: to_model_device_preference(device),
        ..ModelRerankerConfig::default()
    }
}

// ============================================================================
// Resolution functions
// ============================================================================

/// Resolve the embedding configuration for a specific knowledge base.
///
/// This function implements the full resolution chain:
/// 1. Project per-base override
/// 2. Global per-base override
/// 3. Global default
/// 4. Hard-coded default
///
/// # Arguments
///
/// * `base` - The knowledge base name (e.g., "code", "docs")
/// * `global_config` - Global configuration from `~/.gik/config.yaml`
/// * `project_config` - Project configuration from `.guided/knowledge/config.yaml`
/// * `device_pref` - Device preference for inference
///
/// # Returns
///
/// A `gik_model::EmbeddingConfig` ready for use with `gik_model::create_embedding_model`.
pub fn resolve_embedding_config(
    base: &str,
    global_config: &GlobalConfig,
    project_config: &ProjectConfig,
    device_pref: DevicePreference,
) -> ModelEmbeddingConfig {
    // Use gik-core's resolution to get the CoreEmbeddingConfig
    let core_config = project_config.resolve_embedding_config(base, global_config);

    // Convert to gik-model format
    to_model_embedding_config(&core_config, device_pref)
}

/// Resolve the reranker configuration from global config.
///
/// Reranker configuration is global (not per-base) since reranking happens
/// after merging results from all bases.
///
/// # Arguments
///
/// * `global_config` - Global configuration from `~/.gik/config.yaml`
/// * `device_pref` - Device preference for inference
///
/// # Returns
///
/// A `gik_model::RerankerConfig` ready for use with `gik_model::create_reranker_model`.
pub fn resolve_reranker_config(
    global_config: &GlobalConfig,
    device_pref: DevicePreference,
) -> ModelRerankerConfig {
    to_model_reranker_config(&global_config.retrieval.reranker, device_pref)
}

/// Resolve the reranker configuration with defaults.
///
/// Uses auto device selection if not specified in config.
pub fn resolve_reranker_config_with_defaults(global_config: &GlobalConfig) -> ModelRerankerConfig {
    resolve_reranker_config(global_config, global_config.device)
}

// ============================================================================
// Convenience functions for model_adapter
// ============================================================================

/// Create a gik-model EmbeddingConfig from a model ID and device preference.
///
/// This is a convenience function for cases where full config resolution
/// is not needed.
pub fn embedding_config_from_model_id(
    model_id: impl Into<String>,
    device_pref: DevicePreference,
) -> ModelEmbeddingConfig {
    ModelEmbeddingConfig {
        model_id: model_id.into(),
        device: to_model_device_preference(device_pref),
        ..ModelEmbeddingConfig::default()
    }
}

/// Create a gik-model EmbeddingConfig from a local path and device preference.
///
/// This is a convenience function for cases where the model path is known.
pub fn embedding_config_from_path(
    path: impl Into<PathBuf>,
    model_id: impl Into<String>,
    device_pref: DevicePreference,
) -> ModelEmbeddingConfig {
    ModelEmbeddingConfig {
        model_id: model_id.into(),
        local_path: Some(path.into()),
        device: to_model_device_preference(device_pref),
        ..ModelEmbeddingConfig::default()
    }
}

/// Create a gik-model RerankerConfig from a model ID and device preference.
pub fn reranker_config_from_model_id(
    model_id: impl Into<String>,
    device_pref: DevicePreference,
) -> ModelRerankerConfig {
    ModelRerankerConfig {
        model_id: model_id.into(),
        device: to_model_device_preference(device_pref),
        ..ModelRerankerConfig::default()
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::embedding::EmbeddingProviderKind;

    #[test]
    fn test_device_preference_conversion() {
        assert!(matches!(
            to_model_device_preference(DevicePreference::Auto),
            gik_model::DevicePreference::Auto
        ));
        assert!(matches!(
            to_model_device_preference(DevicePreference::Gpu),
            gik_model::DevicePreference::Gpu
        ));
        assert!(matches!(
            to_model_device_preference(DevicePreference::Cpu),
            gik_model::DevicePreference::Cpu
        ));
    }

    #[test]
    fn test_device_preference_roundtrip() {
        for pref in [
            DevicePreference::Auto,
            DevicePreference::Gpu,
            DevicePreference::Cpu,
        ] {
            let model_pref = to_model_device_preference(pref);
            let back = from_model_device_preference(model_pref);
            assert_eq!(pref, back);
        }
    }

    #[test]
    fn test_default_embedding_config() {
        let config = default_embedding_config();
        assert_eq!(config.model_id, gik_model::DEFAULT_EMBEDDING_MODEL_ID);
        assert!(matches!(config.device, gik_model::DevicePreference::Auto));
    }

    #[test]
    fn test_default_reranker_config() {
        let config = default_reranker_config();
        assert_eq!(config.model_id, gik_model::DEFAULT_RERANKER_MODEL_ID);
        assert!(config.enabled);
    }

    #[test]
    fn test_to_model_embedding_config() {
        let core_config = CoreEmbeddingConfig {
            provider: EmbeddingProviderKind::Candle,
            model_id: crate::embedding::EmbeddingModelId::new("custom-model"),
            architecture: None,
            dimension: Some(768),
            max_tokens: Some(256),
            local_path: Some(PathBuf::from("/path/to/model")),
        };

        let model_config = to_model_embedding_config(&core_config, DevicePreference::Cpu);

        assert_eq!(model_config.model_id, "custom-model");
        assert_eq!(
            model_config.local_path,
            Some(PathBuf::from("/path/to/model"))
        );
        assert_eq!(model_config.max_sequence_length, 256);
        assert!(matches!(
            model_config.device,
            gik_model::DevicePreference::Cpu
        ));
    }

    #[test]
    fn test_embedding_config_from_model_id() {
        let config = embedding_config_from_model_id("my-model", DevicePreference::Gpu);
        assert_eq!(config.model_id, "my-model");
        assert!(matches!(config.device, gik_model::DevicePreference::Gpu));
    }

    #[test]
    fn test_resolve_with_defaults() {
        let global = GlobalConfig::default();
        let project = ProjectConfig::default();

        let config = resolve_embedding_config("code", &global, &project, DevicePreference::Auto);

        // Should fall back to default model
        assert!(!config.model_id.is_empty());
    }
}
