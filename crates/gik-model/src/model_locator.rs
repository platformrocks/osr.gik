//! Model locator for runtime path resolution.
//!
//! This module provides a unified way to locate model files at runtime.
//! Models are expected to be disk-based assets shipped with the GIK release.
//!
//! # Search Order
//!
//! The locator searches for models in this order:
//!
//! 1. **Environment override**: `$GIK_MODELS_DIR` (single path)
//! 2. **User directory**: `~/.gik/models`
//! 3. **Binary-relative**: `{exe_dir}/models` (for release packaging)
//!
//! # Model Layout
//!
//! Expected directory structure:
//!
//! ```text
//! {models_dir}/
//!   embeddings/
//!     all-MiniLM-L6-v2/
//!       config.json
//!       model.safetensors
//!       tokenizer.json
//!   rerankers/
//!     ms-marco-MiniLM-L6-v2/
//!       config.json
//!       model.safetensors
//!       tokenizer.json
//! ```

use std::env;
use std::path::{Path, PathBuf};

use crate::error::{ModelError, ModelResult};

/// Environment variable for overriding the models directory.
pub const GIK_MODELS_DIR_ENV: &str = "GIK_MODELS_DIR";

/// Default embedding model directory name.
pub const EMBEDDINGS_SUBDIR: &str = "embeddings";

/// Default reranker model directory name.
pub const RERANKERS_SUBDIR: &str = "rerankers";

/// Default embedding model name (short form).
pub const DEFAULT_EMBEDDING_MODEL_NAME: &str = "all-MiniLM-L6-v2";

/// Default reranker model name (short form).
pub const DEFAULT_RERANKER_MODEL_NAME: &str = "ms-marco-MiniLM-L6-v2";

/// Required files for a valid model directory.
pub const REQUIRED_MODEL_FILES: &[&str] = &["config.json", "model.safetensors", "tokenizer.json"];

// ============================================================================
// ModelLocator
// ============================================================================

/// Locates model files at runtime using a defined search order.
///
/// The locator does not download models. Models must be pre-installed at one
/// of the search locations. If models are not found, an error with guidance
/// is returned.
#[derive(Debug, Clone)]
pub struct ModelLocator {
    /// Cached base directory (resolved on first use).
    base_dir: Option<PathBuf>,
}

impl Default for ModelLocator {
    fn default() -> Self {
        Self::new()
    }
}

impl ModelLocator {
    /// Create a new model locator.
    pub fn new() -> Self {
        Self { base_dir: None }
    }

    /// Create a model locator with a fixed base directory.
    ///
    /// Useful for testing or when the models directory is known.
    pub fn with_base_dir(base_dir: impl Into<PathBuf>) -> Self {
        Self {
            base_dir: Some(base_dir.into()),
        }
    }

    /// Resolve the base models directory.
    ///
    /// Search order:
    /// 1. `$GIK_MODELS_DIR` environment variable
    /// 2. `~/.gik/models` (user home directory)
    /// 3. `{exe_dir}/models` (next to the gik binary)
    ///
    /// Returns the first directory that exists.
    pub fn resolve_base_dir(&self) -> ModelResult<PathBuf> {
        // If a fixed base directory was provided, use it.
        if let Some(ref base) = self.base_dir {
            if base.exists() {
                return Ok(base.clone());
            }
            return Err(ModelError::ModelsDirectoryNotFound {
                searched: vec![base.clone()],
            });
        }

        let mut searched = Vec::new();

        // 1. Check $GIK_MODELS_DIR
        if let Ok(env_path) = env::var(GIK_MODELS_DIR_ENV) {
            let path = PathBuf::from(&env_path);
            if path.exists() && path.is_dir() {
                return Ok(path);
            }
            searched.push(path);
        }

        // 2. Check ~/.gik/models
        if let Some(home) = dirs::home_dir() {
            let path = home.join(".gik").join("models");
            if path.exists() && path.is_dir() {
                return Ok(path);
            }
            searched.push(path);
        }

        // 3. Check {exe_dir}/models
        if let Ok(exe_path) = env::current_exe() {
            if let Some(exe_dir) = exe_path.parent() {
                let path = exe_dir.join("models");
                if path.exists() && path.is_dir() {
                    return Ok(path);
                }
                searched.push(path);
            }
        }

        Err(ModelError::ModelsDirectoryNotFound { searched })
    }

    /// Get the path to the embeddings subdirectory.
    pub fn embeddings_dir(&self) -> ModelResult<PathBuf> {
        Ok(self.resolve_base_dir()?.join(EMBEDDINGS_SUBDIR))
    }

    /// Get the path to the rerankers subdirectory.
    pub fn rerankers_dir(&self) -> ModelResult<PathBuf> {
        Ok(self.resolve_base_dir()?.join(RERANKERS_SUBDIR))
    }

    /// Resolve the path to a specific embedding model.
    ///
    /// # Arguments
    ///
    /// * `model_id` - Full model ID (e.g., "sentence-transformers/all-MiniLM-L6-v2")
    ///   or short name (e.g., "all-MiniLM-L6-v2")
    ///
    /// # Returns
    ///
    /// The path to the model directory, or an error if not found.
    pub fn embedding_model_path(&self, model_id: &str) -> ModelResult<PathBuf> {
        let base = self.resolve_base_dir()?;
        let model_name = extract_model_name(model_id);

        // Try these locations in order:
        // 1. {base}/embeddings/{model_name}
        // 2. {base}/{full_model_id} (for HF-style paths)
        // 3. {base}/{model_name} (flat layout)
        let candidates = [
            base.join(EMBEDDINGS_SUBDIR).join(model_name),
            base.join(model_id),
            base.join(model_name),
        ];

        for path in &candidates {
            if is_valid_model_dir(path) {
                return Ok(path.clone());
            }
        }

        Err(ModelError::ModelNotFound {
            model_id: model_id.to_string(),
            path: candidates[0].clone(),
        })
    }

    /// Resolve the path to a specific reranker model.
    ///
    /// # Arguments
    ///
    /// * `model_id` - Full model ID (e.g., "cross-encoder/ms-marco-MiniLM-L6-v2")
    ///   or short name (e.g., "ms-marco-MiniLM-L6-v2")
    ///
    /// # Returns
    ///
    /// The path to the model directory, or an error if not found.
    pub fn reranker_model_path(&self, model_id: &str) -> ModelResult<PathBuf> {
        let base = self.resolve_base_dir()?;
        let model_name = extract_model_name(model_id);

        // Try these locations in order:
        // 1. {base}/rerankers/{model_name}
        // 2. {base}/{full_model_id} (for HF-style paths)
        // 3. {base}/{model_name} (flat layout)
        let candidates = [
            base.join(RERANKERS_SUBDIR).join(model_name),
            base.join(model_id),
            base.join(model_name),
        ];

        for path in &candidates {
            if is_valid_model_dir(path) {
                return Ok(path.clone());
            }
        }

        Err(ModelError::ModelNotFound {
            model_id: model_id.to_string(),
            path: candidates[0].clone(),
        })
    }

    /// Get the path to the default embedding model.
    pub fn default_embedding_model_path(&self) -> ModelResult<PathBuf> {
        self.embedding_model_path(DEFAULT_EMBEDDING_MODEL_NAME)
    }

    /// Get the path to the default reranker model.
    pub fn default_reranker_model_path(&self) -> ModelResult<PathBuf> {
        self.reranker_model_path(DEFAULT_RERANKER_MODEL_NAME)
    }

    /// Check if the default embedding model is available.
    pub fn has_default_embedding_model(&self) -> bool {
        self.default_embedding_model_path().is_ok()
    }

    /// Check if the default reranker model is available.
    pub fn has_default_reranker_model(&self) -> bool {
        self.default_reranker_model_path().is_ok()
    }

    /// Validate that a model directory contains all required files.
    pub fn validate_model_dir(&self, path: &Path) -> ModelResult<()> {
        if !path.exists() {
            return Err(ModelError::ModelNotFound {
                model_id: path.display().to_string(),
                path: path.to_path_buf(),
            });
        }

        let mut missing = Vec::new();
        for file in REQUIRED_MODEL_FILES {
            if !path.join(file).exists() {
                missing.push(*file);
            }
        }

        if !missing.is_empty() {
            return Err(ModelError::IncompleteModelFiles {
                path: path.to_path_buf(),
                missing,
            });
        }

        Ok(())
    }
}

// ============================================================================
// Helper Functions
// ============================================================================

/// Extract the model name from a full model ID.
///
/// E.g., "sentence-transformers/all-MiniLM-L6-v2" → "all-MiniLM-L6-v2"
fn extract_model_name(model_id: &str) -> &str {
    model_id.rsplit('/').next().unwrap_or(model_id)
}

/// Check if a directory is a valid model directory.
fn is_valid_model_dir(path: &Path) -> bool {
    if !path.exists() || !path.is_dir() {
        return false;
    }

    // Must have at least config.json
    path.join("config.json").exists()
}

// ============================================================================
// Global Accessor
// ============================================================================

/// Get a default model locator.
///
/// This creates a new locator each time. For repeated use, consider caching
/// the result or using `ModelLocator::new()` directly.
pub fn default_locator() -> ModelLocator {
    ModelLocator::new()
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn setup_mock_model_dir(temp: &TempDir, subdir: &str, model_name: &str) -> PathBuf {
        let model_path = temp.path().join(subdir).join(model_name);
        fs::create_dir_all(&model_path).unwrap();

        // Create required files
        for file in REQUIRED_MODEL_FILES {
            fs::write(model_path.join(file), "{}").unwrap();
        }

        model_path
    }

    #[test]
    fn test_extract_model_name() {
        assert_eq!(
            extract_model_name("sentence-transformers/all-MiniLM-L6-v2"),
            "all-MiniLM-L6-v2"
        );
        assert_eq!(
            extract_model_name("cross-encoder/ms-marco-MiniLM-L6-v2"),
            "ms-marco-MiniLM-L6-v2"
        );
        assert_eq!(extract_model_name("simple-model"), "simple-model");
    }

    #[test]
    fn test_locator_with_base_dir() {
        let temp = TempDir::new().unwrap();
        setup_mock_model_dir(&temp, "embeddings", "test-model");

        let locator = ModelLocator::with_base_dir(temp.path());
        let base = locator.resolve_base_dir().unwrap();
        assert_eq!(base, temp.path());
    }

    #[test]
    fn test_embedding_model_path() {
        let temp = TempDir::new().unwrap();
        setup_mock_model_dir(&temp, "embeddings", "all-MiniLM-L6-v2");

        let locator = ModelLocator::with_base_dir(temp.path());

        // Full model ID
        let path = locator
            .embedding_model_path("sentence-transformers/all-MiniLM-L6-v2")
            .unwrap();
        assert!(path.exists());

        // Short name
        let path = locator.embedding_model_path("all-MiniLM-L6-v2").unwrap();
        assert!(path.exists());
    }

    #[test]
    fn test_reranker_model_path() {
        let temp = TempDir::new().unwrap();
        setup_mock_model_dir(&temp, "rerankers", "ms-marco-MiniLM-L6-v2");

        let locator = ModelLocator::with_base_dir(temp.path());

        // Full model ID
        let path = locator
            .reranker_model_path("cross-encoder/ms-marco-MiniLM-L6-v2")
            .unwrap();
        assert!(path.exists());

        // Short name
        let path = locator
            .reranker_model_path("ms-marco-MiniLM-L6-v2")
            .unwrap();
        assert!(path.exists());
    }

    #[test]
    fn test_model_not_found() {
        let temp = TempDir::new().unwrap();
        fs::create_dir_all(temp.path().join("embeddings")).unwrap();

        let locator = ModelLocator::with_base_dir(temp.path());
        let result = locator.embedding_model_path("nonexistent-model");
        assert!(result.is_err());

        match result.unwrap_err() {
            ModelError::ModelNotFound { model_id, .. } => {
                assert_eq!(model_id, "nonexistent-model");
            }
            other => panic!("Expected ModelNotFound, got {:?}", other),
        }
    }

    #[test]
    fn test_validate_model_dir_incomplete() {
        let temp = TempDir::new().unwrap();
        let model_path = temp.path().join("incomplete-model");
        fs::create_dir_all(&model_path).unwrap();

        // Only create config.json
        fs::write(model_path.join("config.json"), "{}").unwrap();

        let locator = ModelLocator::with_base_dir(temp.path());
        let result = locator.validate_model_dir(&model_path);
        assert!(result.is_err());

        match result.unwrap_err() {
            ModelError::IncompleteModelFiles { missing, .. } => {
                assert!(missing.contains(&"model.safetensors"));
                assert!(missing.contains(&"tokenizer.json"));
            }
            other => panic!("Expected IncompleteModelFiles, got {:?}", other),
        }
    }

    #[test]
    fn test_env_var_override() {
        let temp = TempDir::new().unwrap();
        setup_mock_model_dir(&temp, "embeddings", "test-model");

        // Set environment variable
        env::set_var(GIK_MODELS_DIR_ENV, temp.path());

        let locator = ModelLocator::new();
        let base = locator.resolve_base_dir().unwrap();
        assert_eq!(base, temp.path());

        // Clean up
        env::remove_var(GIK_MODELS_DIR_ENV);
    }
}
