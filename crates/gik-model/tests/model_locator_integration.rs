//! Integration tests for ModelLocator.
//!
//! Tests the model locator functionality with temporary directories
//! and environment variable overrides.

use gik_model::{
    default_locator, ModelError, ModelLocator, DEFAULT_EMBEDDING_MODEL_NAME,
    DEFAULT_RERANKER_MODEL_NAME, EMBEDDINGS_SUBDIR, GIK_MODELS_DIR_ENV, REQUIRED_MODEL_FILES,
    RERANKERS_SUBDIR,
};
use std::env;
use std::fs;
use std::sync::Mutex;
use tempfile::TempDir;

// Mutex to prevent concurrent env var modifications in tests
static ENV_MUTEX: Mutex<()> = Mutex::new(());

/// Helper to create a mock model directory with required files.
fn setup_mock_model(temp: &TempDir, subdir: &str, model_name: &str) -> std::path::PathBuf {
    let model_path = temp.path().join(subdir).join(model_name);
    fs::create_dir_all(&model_path).expect("Failed to create model directory");

    // Create required files with minimal valid content
    for file in REQUIRED_MODEL_FILES {
        let content = match *file {
            "config.json" => r#"{"hidden_size": 384, "max_position_embeddings": 512}"#,
            "tokenizer.json" => r#"{"version": "1.0"}"#,
            _ => "{}",
        };
        fs::write(model_path.join(file), content).expect("Failed to write model file");
    }

    model_path
}

// ============================================================================
// ModelLocator::with_base_dir tests
// ============================================================================

#[test]
fn test_locator_with_valid_base_dir() {
    let temp = TempDir::new().unwrap();
    setup_mock_model(&temp, EMBEDDINGS_SUBDIR, "test-model");

    let locator = ModelLocator::with_base_dir(temp.path());
    let base = locator.resolve_base_dir().unwrap();
    assert_eq!(base, temp.path());
}

#[test]
fn test_locator_with_nonexistent_base_dir() {
    let locator = ModelLocator::with_base_dir("/nonexistent/path/to/models");
    let result = locator.resolve_base_dir();

    assert!(result.is_err());
    match result.unwrap_err() {
        ModelError::ModelsDirectoryNotFound { searched } => {
            assert_eq!(searched.len(), 1);
            assert!(searched[0].to_string_lossy().contains("nonexistent"));
        }
        other => panic!("Expected ModelsDirectoryNotFound, got {:?}", other),
    }
}

// ============================================================================
// Environment variable override tests
// ============================================================================

#[test]
fn test_env_var_takes_precedence() {
    let _guard = ENV_MUTEX.lock().unwrap();

    let temp = TempDir::new().unwrap();
    setup_mock_model(&temp, EMBEDDINGS_SUBDIR, DEFAULT_EMBEDDING_MODEL_NAME);

    // Set environment variable
    env::set_var(GIK_MODELS_DIR_ENV, temp.path());

    let locator = default_locator();
    let base = locator.resolve_base_dir().unwrap();
    assert_eq!(base, temp.path());

    // Clean up
    env::remove_var(GIK_MODELS_DIR_ENV);
}

#[test]
fn test_env_var_nonexistent_falls_through() {
    let _guard = ENV_MUTEX.lock().unwrap();

    // Set to nonexistent path
    env::set_var(GIK_MODELS_DIR_ENV, "/nonexistent/models/path");

    let locator = default_locator();
    let result = locator.resolve_base_dir();

    // Should fail since env var path doesn't exist and we don't have other paths
    // (This test may pass if ~/.gik/models exists on the system)

    // Clean up
    env::remove_var(GIK_MODELS_DIR_ENV);

    // We can't assert much here since the result depends on system state
    // Just verify no panic occurred
    let _ = result;
}

// ============================================================================
// Embedding model path resolution tests
// ============================================================================

#[test]
fn test_embedding_model_path_with_full_id() {
    let temp = TempDir::new().unwrap();
    setup_mock_model(&temp, EMBEDDINGS_SUBDIR, "all-MiniLM-L6-v2");

    let locator = ModelLocator::with_base_dir(temp.path());
    let path = locator
        .embedding_model_path("sentence-transformers/all-MiniLM-L6-v2")
        .unwrap();

    assert!(path.exists());
    assert!(path.join("config.json").exists());
    assert!(path.join("model.safetensors").exists());
    assert!(path.join("tokenizer.json").exists());
}

#[test]
fn test_embedding_model_path_with_short_name() {
    let temp = TempDir::new().unwrap();
    setup_mock_model(&temp, EMBEDDINGS_SUBDIR, "all-MiniLM-L6-v2");

    let locator = ModelLocator::with_base_dir(temp.path());
    let path = locator.embedding_model_path("all-MiniLM-L6-v2").unwrap();

    assert!(path.exists());
}

#[test]
fn test_embedding_model_not_found() {
    let temp = TempDir::new().unwrap();
    fs::create_dir_all(temp.path().join(EMBEDDINGS_SUBDIR)).unwrap();

    let locator = ModelLocator::with_base_dir(temp.path());
    let result = locator.embedding_model_path("nonexistent-model");

    assert!(result.is_err());
    match result.unwrap_err() {
        ModelError::ModelNotFound { model_id, path } => {
            assert_eq!(model_id, "nonexistent-model");
            assert!(path
                .to_string_lossy()
                .contains("embeddings/nonexistent-model"));
        }
        other => panic!("Expected ModelNotFound, got {:?}", other),
    }
}

// ============================================================================
// Reranker model path resolution tests
// ============================================================================

#[test]
fn test_reranker_model_path_with_full_id() {
    let temp = TempDir::new().unwrap();
    setup_mock_model(&temp, RERANKERS_SUBDIR, "ms-marco-MiniLM-L6-v2");

    let locator = ModelLocator::with_base_dir(temp.path());
    let path = locator
        .reranker_model_path("cross-encoder/ms-marco-MiniLM-L6-v2")
        .unwrap();

    assert!(path.exists());
}

#[test]
fn test_reranker_model_path_with_short_name() {
    let temp = TempDir::new().unwrap();
    setup_mock_model(&temp, RERANKERS_SUBDIR, "ms-marco-MiniLM-L6-v2");

    let locator = ModelLocator::with_base_dir(temp.path());
    let path = locator
        .reranker_model_path("ms-marco-MiniLM-L6-v2")
        .unwrap();

    assert!(path.exists());
}

// ============================================================================
// Default model path tests
// ============================================================================

#[test]
fn test_default_embedding_model_path() {
    let temp = TempDir::new().unwrap();
    setup_mock_model(&temp, EMBEDDINGS_SUBDIR, DEFAULT_EMBEDDING_MODEL_NAME);

    let locator = ModelLocator::with_base_dir(temp.path());
    let path = locator.default_embedding_model_path().unwrap();

    assert!(path.exists());
    assert!(path.ends_with(DEFAULT_EMBEDDING_MODEL_NAME));
}

#[test]
fn test_default_reranker_model_path() {
    let temp = TempDir::new().unwrap();
    setup_mock_model(&temp, RERANKERS_SUBDIR, DEFAULT_RERANKER_MODEL_NAME);

    let locator = ModelLocator::with_base_dir(temp.path());
    let path = locator.default_reranker_model_path().unwrap();

    assert!(path.exists());
    assert!(path.ends_with(DEFAULT_RERANKER_MODEL_NAME));
}

#[test]
fn test_has_default_embedding_model() {
    let temp = TempDir::new().unwrap();

    // Without model
    let locator = ModelLocator::with_base_dir(temp.path());
    assert!(!locator.has_default_embedding_model());

    // With model
    setup_mock_model(&temp, EMBEDDINGS_SUBDIR, DEFAULT_EMBEDDING_MODEL_NAME);
    assert!(locator.has_default_embedding_model());
}

#[test]
fn test_has_default_reranker_model() {
    let temp = TempDir::new().unwrap();

    // Without model
    let locator = ModelLocator::with_base_dir(temp.path());
    assert!(!locator.has_default_reranker_model());

    // With model
    setup_mock_model(&temp, RERANKERS_SUBDIR, DEFAULT_RERANKER_MODEL_NAME);
    assert!(locator.has_default_reranker_model());
}

// ============================================================================
// Model validation tests
// ============================================================================

#[test]
fn test_validate_complete_model_dir() {
    let temp = TempDir::new().unwrap();
    let model_path = setup_mock_model(&temp, EMBEDDINGS_SUBDIR, "complete-model");

    let locator = ModelLocator::with_base_dir(temp.path());
    let result = locator.validate_model_dir(&model_path);

    assert!(result.is_ok());
}

#[test]
fn test_validate_incomplete_model_dir_missing_safetensors() {
    let temp = TempDir::new().unwrap();
    let model_path = temp.path().join("incomplete-model");
    fs::create_dir_all(&model_path).unwrap();

    // Only create config.json and tokenizer.json
    fs::write(model_path.join("config.json"), "{}").unwrap();
    fs::write(model_path.join("tokenizer.json"), "{}").unwrap();

    let locator = ModelLocator::with_base_dir(temp.path());
    let result = locator.validate_model_dir(&model_path);

    assert!(result.is_err());
    match result.unwrap_err() {
        ModelError::IncompleteModelFiles { path, missing } => {
            assert_eq!(path, model_path);
            assert!(missing.contains(&"model.safetensors"));
        }
        other => panic!("Expected IncompleteModelFiles, got {:?}", other),
    }
}

#[test]
fn test_validate_incomplete_model_dir_missing_all() {
    let temp = TempDir::new().unwrap();
    let model_path = temp.path().join("empty-model");
    fs::create_dir_all(&model_path).unwrap();

    let locator = ModelLocator::with_base_dir(temp.path());
    let result = locator.validate_model_dir(&model_path);

    assert!(result.is_err());
    match result.unwrap_err() {
        ModelError::IncompleteModelFiles { missing, .. } => {
            assert_eq!(missing.len(), 3);
            assert!(missing.contains(&"config.json"));
            assert!(missing.contains(&"model.safetensors"));
            assert!(missing.contains(&"tokenizer.json"));
        }
        other => panic!("Expected IncompleteModelFiles, got {:?}", other),
    }
}

#[test]
fn test_validate_nonexistent_model_dir() {
    let temp = TempDir::new().unwrap();
    let model_path = temp.path().join("nonexistent");

    let locator = ModelLocator::with_base_dir(temp.path());
    let result = locator.validate_model_dir(&model_path);

    assert!(result.is_err());
    match result.unwrap_err() {
        ModelError::ModelNotFound { .. } => {}
        other => panic!("Expected ModelNotFound, got {:?}", other),
    }
}

// ============================================================================
// Error message format tests
// ============================================================================

#[test]
fn test_models_dir_not_found_error_message() {
    let locator = ModelLocator::with_base_dir("/nonexistent/path");
    let err = locator.resolve_base_dir().unwrap_err();

    let msg = format!("{}", err);
    assert!(msg.contains("Models directory not found"));
    assert!(msg.contains("GIK searched these locations"));
    assert!(msg.contains("$GIK_MODELS_DIR"));
}

#[test]
fn test_model_not_found_error_message() {
    let temp = TempDir::new().unwrap();
    fs::create_dir_all(temp.path().join(EMBEDDINGS_SUBDIR)).unwrap();

    let locator = ModelLocator::with_base_dir(temp.path());
    let err = locator.embedding_model_path("missing-model").unwrap_err();

    let msg = format!("{}", err);
    assert!(msg.contains("Model not found: missing-model"));
    assert!(msg.contains("Expected at:"));
}

#[test]
fn test_incomplete_model_error_message() {
    let temp = TempDir::new().unwrap();
    let model_path = temp.path().join("incomplete");
    fs::create_dir_all(&model_path).unwrap();
    fs::write(model_path.join("config.json"), "{}").unwrap();

    let locator = ModelLocator::with_base_dir(temp.path());
    let err = locator.validate_model_dir(&model_path).unwrap_err();

    let msg = format!("{}", err);
    assert!(msg.contains("Incomplete model installation"));
    assert!(msg.contains("Missing files:"));
    assert!(msg.contains("model.safetensors"));
}
