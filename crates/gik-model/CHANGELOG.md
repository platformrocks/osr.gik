# Changelog

All notable changes to `gik-model` will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.1.0] - 2024-12-09

### Added

- Initial release of the `gik-model` library
- `EmbeddingModel` trait for embedding providers
- `RerankerModel` trait for reranking providers
- Candle-based embedding model implementation
- Candle-based cross-encoder reranker implementation
- `ModelLocator` for runtime model path discovery
- Model search paths: `$GIK_MODELS_DIR`, `~/.gik/models`, `{exe_dir}/models`
- `EmbeddingConfig` and `RerankerConfig` canonical configuration types
- `DevicePreference` enum (auto/gpu/cpu)
- `ModelError` with actionable error messages
- Batch embedding generation for performance
- Feature flags for GPU support (`metal`, `cuda`)
- Placeholder for Ollama remote API (`ollama` feature)

### Architecture

- Trait-based abstraction for model providers
- Single source of truth for embedding/reranking configuration
- Device selection with automatic fallback
- Feature-gated implementations to minimize compile-time dependencies

### Default Models

- Embeddings: `all-MiniLM-L6-v2` (384 dimensions)
- Reranker: `ms-marco-MiniLM-L6-v2`
