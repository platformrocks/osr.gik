# gik-model

> ML inference layer for GIK—embeddings, reranking, and model discovery.

## Overview

`gik-model` is the infrastructure library crate that handles all machine learning inference for GIK. It provides embedding generation and cross-encoder reranking using Candle (Rust ML framework). This crate is the single source of truth for embedding/reranking configuration, model discovery, and device selection. It isolates heavy ML dependencies (Candle, tokenizers, safetensors) from `gik-core`.

## Goals

- **Dependency isolation**: Keep Candle, tokenizers, and safetensors out of `gik-core`
- **Trait-based abstraction**: Expose ML operations through `EmbeddingModel` and `RerankerModel` traits
- **Model discovery**: Automatically find models in standard locations
- **Device flexibility**: Support CPU, GPU (CUDA), and Metal acceleration via feature flags

## Features

- Provides `EmbeddingModel` trait for text embedding generation
- Provides `RerankerModel` trait for cross-encoder reranking
- Implements Candle-based embedding model with batch processing
- Implements Candle-based cross-encoder reranker
- Discovers models in `$GIK_MODELS_DIR`, `~/.gik/models`, or `{exe_dir}/models`
- Supports device selection: auto, gpu, cpu
- Provides canonical configuration types (`EmbeddingConfig`, `RerankerConfig`)

## Architecture

### Module Overview

```
src/
├── lib.rs              # Traits, factory functions, re-exports (~246 lines)
├── config.rs           # EmbeddingConfig, RerankerConfig (~482 lines)
├── error.rs            # ModelError with actionable messages (~167 lines)
├── model_locator.rs    # Runtime model path resolution
├── embedding.rs        # CandleEmbeddingModel (feature-gated)
└── reranker.rs         # CandleRerankerModel (feature-gated)
```

### Key Types

| Type | Role |
|------|------|
| `EmbeddingModel` | Trait for embedding providers |
| `RerankerModel` | Trait for reranking providers |
| `EmbeddingConfig` | Canonical embedding configuration |
| `RerankerConfig` | Canonical reranker configuration |
| `DevicePreference` | auto / gpu / cpu selection |
| `ModelError` | Error type with actionable messages |
| `ModelLocator` | Runtime model path discovery |

### Trait Contracts

**EmbeddingModel**:
```rust
pub trait EmbeddingModel: Send + Sync {
    /// Generate embeddings for a batch of texts
    fn embed_batch(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>, ModelError>;
    
    /// Get embedding dimensions
    fn dimensions(&self) -> usize;
    
    /// Get model identifier
    fn model_id(&self) -> &str;
}
```

**RerankerModel**:
```rust
pub trait RerankerModel: Send + Sync {
    /// Score query-document pairs, returning relevance scores
    fn rerank(&self, query: &str, documents: &[&str]) -> Result<Vec<f32>, ModelError>;
    
    /// Get model identifier
    fn model_id(&self) -> &str;
}
```

### Model Search Paths

Models are discovered in the following order:

1. `$GIK_MODELS_DIR` environment variable
2. `~/.gik/models` user directory
3. `{exe_dir}/models` next to the binary

Directory structure expected:
```
models/
├── embeddings/
│   └── all-MiniLM-L6-v2/
│       ├── config.json
│       ├── tokenizer.json
│       └── model.safetensors
└── rerankers/
    └── ms-marco-MiniLM-L6-v2/
        ├── config.json
        ├── tokenizer.json
        └── model.safetensors
```

### Device Selection

The `DevicePreference` enum controls hardware selection:

```rust
pub enum DevicePreference {
    Auto,  // Try GPU first, fall back to CPU
    Gpu,   // Require GPU (fails if unavailable)
    Cpu,   // Force CPU even if GPU available
}
```

With feature flags:
- `metal`: Enables Metal GPU on macOS
- `cuda`: Enables CUDA GPU on Linux/Windows

## Dependencies

### External

| Crate | Purpose |
|-------|---------|
| `candle-core` | Tensor operations and device management |
| `candle-nn` | Neural network layers |
| `candle-transformers` | Transformer model implementations |
| `tokenizers` | HuggingFace tokenizer loading |
| `safetensors` | Model weight loading |
| `thiserror` | Error derive macro |
| `serde` | Configuration serialization |
| `tracing` | Structured logging |

## Usage

### Creating an Embedding Model

```rust
use gik_model::{create_embedding_model, EmbeddingConfig, DevicePreference};

let config = EmbeddingConfig {
    model: "all-MiniLM-L6-v2".to_string(),
    dimensions: 384,
    ..Default::default()
};

let model = create_embedding_model(&config, DevicePreference::Auto)?;

// Generate embeddings
let texts = vec!["Hello world", "How are you?"];
let embeddings = model.embed_batch(&texts.iter().map(|s| *s).collect::<Vec<_>>())?;

assert_eq!(embeddings.len(), 2);
assert_eq!(embeddings[0].len(), 384);
```

### Creating a Reranker

```rust
use gik_model::{create_reranker_model, RerankerConfig, DevicePreference};

let config = RerankerConfig {
    model: "ms-marco-MiniLM-L6-v2".to_string(),
    ..Default::default()
};

let reranker = create_reranker_model(&config, DevicePreference::Auto)?;

// Rerank documents
let query = "What is Rust?";
let docs = vec!["Rust is a systems programming language", "Python is interpreted"];
let scores = reranker.rerank(query, &docs)?;

// Higher score = more relevant
assert!(scores[0] > scores[1]);
```

### Configuration Types

```rust
use gik_model::{EmbeddingConfig, RerankerConfig};

// Embedding config with defaults
let embed_config = EmbeddingConfig::default();
// model: "all-MiniLM-L6-v2"
// dimensions: 384
// batch_size: 32

// Reranker config with defaults
let rerank_config = RerankerConfig::default();
// model: "ms-marco-MiniLM-L6-v2"
// enabled: true
```

## Configuration

Configuration is typically provided by `gik-core` after loading from `gik.yaml`:

```yaml
embedding:
  model: all-MiniLM-L6-v2
  dimensions: 384
  batch_size: 32

reranker:
  model: ms-marco-MiniLM-L6-v2
  enabled: true
```

## Feature Flags

| Flag | Default | Effect |
|------|---------|--------|
| `embedded` | ✓ | Enable Candle local inference |
| `metal` | — | Enable macOS GPU acceleration |
| `cuda` | — | Enable NVIDIA GPU acceleration |
| `ollama` | — | Enable Ollama remote API (placeholder) |

Build with GPU support:

```bash
# macOS Metal
cargo build -p gik-model --features metal

# NVIDIA CUDA
cargo build -p gik-model --features cuda
```

**Note**: The `ollama` feature is defined but not yet implemented—returns an error if selected.

## Error Handling

`ModelError` variants include actionable hints:

```rust
#[derive(Error, Debug)]
pub enum ModelError {
    #[error("Models directory not found. Searched: {searched:?}\n\nTo fix:\n1. Set GIK_MODELS_DIR environment variable\n2. Or create ~/.gik/models/\n3. Or place models next to the binary")]
    ModelsDirectoryNotFound { searched: Vec<PathBuf> },
    
    #[error("Model '{model}' not found in {path}\n\nExpected structure:\n  {path}/config.json\n  {path}/tokenizer.json\n  {path}/model.safetensors")]
    ModelNotFound { model: String, path: PathBuf },
}
```

## Testing

```bash
# Run unit tests (no models needed)
cargo test -p gik-model

# Run integration tests (requires models)
cargo test -p gik-model -- --ignored
```

Integration tests require models to be present in one of the search paths.

## Versioning

This crate follows the workspace version defined in the root `Cargo.toml`.
See [CHANGELOG.md](./CHANGELOG.md) for version history.

## Related Documentation

- [Crates Overview](../../.guided/architecture/crates-overview.md) — All crates in the workspace
- [Architecture Document](../../docs/5-ARCH.md) — Global architecture view
- [gik-core README](../gik-core/README.md) — How core uses this crate via adapters
