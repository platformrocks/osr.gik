# gik-core

> The heart of GIK—domain logic, `GikEngine` orchestrator, and all pipelines.

## Overview

`gik-core` is the central library crate that owns all domain logic for the Guided Indexing Kernel. It provides the `GikEngine` orchestrator which implements pipelines for `init`, `add`, `commit`, `ask`, `reindex`, `release`, and other operations. This crate defines domain types, configuration handling, and error types with actionable hints. It never imports heavy dependencies directly—instead using adapter traits to bridge to `gik-db`, `gik-model`, and `gik-utils`.

## Goals

- **Domain ownership**: Single source of truth for all GIK business logic
- **Dependency isolation**: Keep heavy dependencies (LanceDB, Candle, Arrow) in leaf crates
- **Actionable errors**: Every error variant includes context and hints for resolution
- **Sync API**: Maintain synchronous interface; async is hidden in infrastructure crates

## Features

- Provides `GikEngine` as the main orchestrator for all operations
- Implements RAG pipeline with hybrid search (dense + sparse) and reranking
- Manages workspace initialization, branch handling, and revision timeline
- Handles staging, commit, and indexing of code, docs, and memory bases
- Extracts and maintains knowledge graphs from indexed content
- Generates release changelogs from revision history
- Bridges to infrastructure crates via `db_adapter` and `model_adapter`

## Architecture

### Module Overview

```
src/
├── lib.rs                    # Public re-exports
├── engine.rs                 # GikEngine (~3800 lines) - main orchestrator
├── errors.rs                 # GikError (~400 lines) - domain errors
├── config.rs                 # GlobalConfig, ProjectConfig (~1800 lines)
├── types.rs                  # AddOptions, CommitOptions, AskOptions, etc.
├── workspace.rs              # Workspace, BranchName detection
├── constants.rs              # Ignore patterns, default paths
│
├── ask.rs                    # RAG query pipeline
├── commit.rs                 # Commit/indexing pipeline
├── reindex.rs                # Reindex pipeline
├── release.rs                # Changelog generation
├── show.rs                   # Revision inspection
├── status.rs                 # Status reporting
├── log.rs                    # Log queries
│
├── timeline.rs               # Revision history management
├── staging.rs                # File staging operations
├── base.rs                   # Knowledge base operations
├── stack.rs                  # Stack inventory (tech detection)
│
├── embedding.rs              # EmbeddingBackend trait
├── embedding_config_bridge.rs # Config resolution
├── reranker.rs               # RerankerModel abstraction
├── query_expansion.rs        # Query expansion for search
│
├── db_adapter.rs             # Bridge to gik-db (~510 lines)
├── model_adapter.rs          # Bridge to gik-model (~390 lines)
│
├── vector_index/             # Vector index abstractions
│   ├── mod.rs
│   └── metadata.rs
├── kg/                       # Knowledge graph
│   ├── mod.rs, entities.rs, export.rs, extractor.rs
│   ├── query.rs, store.rs, sync.rs
│   └── lang/                 # Language-specific extractors
├── memory/                   # Memory base
│   ├── mod.rs (~1000 lines)
│   ├── metrics.rs
│   └── pruning.rs
└── bm25/                     # BM25 sparse retrieval
    ├── mod.rs, index.rs, scorer.rs
    ├── storage.rs, tokenizer.rs
```

### Key Types

| Type | Role |
|------|------|
| `GikEngine` | Main orchestrator; owns config, workspace, and model backends |
| `GikError` | Domain errors with actionable hints |
| `GlobalConfig` | User-level configuration from `gik.yaml` |
| `ProjectConfig` | Project-level configuration |
| `Workspace` | Workspace root and branch detection |
| `BranchName` | Validated branch identifier |
| `EmbeddingBackend` | Trait for embedding providers |
| `RerankerModel` | Trait for reranking providers |

### Pipeline Data Flow

```
┌─────────────────────────────────────────────────────────────────────┐
│                           ask Pipeline                               │
├─────────────────────────────────────────────────────────────────────┤
│                                                                      │
│  Query → Query Expansion → ┬→ Dense Search (embeddings)             │
│                            └→ Sparse Search (BM25)                  │
│                                      ↓                               │
│                              RRF Fusion                              │
│                                      ↓                               │
│                         Cross-Encoder Reranking                      │
│                                      ↓                               │
│                           Top-K Results                              │
│                                                                      │
└─────────────────────────────────────────────────────────────────────┘

┌─────────────────────────────────────────────────────────────────────┐
│                         commit Pipeline                              │
├─────────────────────────────────────────────────────────────────────┤
│                                                                      │
│  Staged Files → Chunking → Embedding (batched) → Vector Upsert      │
│       ↓                                                              │
│  BM25 Index Update                                                   │
│       ↓                                                              │
│  KG Entity Extraction (if enabled)                                   │
│       ↓                                                              │
│  Timeline Revision                                                   │
│                                                                      │
└─────────────────────────────────────────────────────────────────────┘
```

### Adapter Pattern

`gik-core` uses adapter modules to bridge to infrastructure crates without importing their heavy dependencies:

**`db_adapter.rs`**:
- Converts `gik_db::DbError` → `GikError`
- Wraps `VectorIndexBackend` and `KgStoreBackend`
- Provides type conversions for KG entities

**`model_adapter.rs`**:
- Converts `gik_model::ModelError` → `GikError`
- Wraps model factory functions
- Converts config types between crates

Both implement the `IntoGikResult<T>` extension trait:

```rust
pub trait IntoGikResult<T> {
    fn into_gik_result(self) -> Result<T, GikError>;
}
```

## Modules

| Module | Purpose |
|--------|---------|
| `engine` | `GikEngine` orchestrator with all command implementations |
| `errors` | `GikError` enum with 30+ variants, all with actionable messages |
| `config` | Configuration loading, merging, and validation |
| `ask` | RAG pipeline: hybrid search, fusion, reranking |
| `commit` | Indexing pipeline: chunking, embedding, storage |
| `timeline` | Revision history: Init, Commit, Reindex, Release events |
| `staging` | File staging for add/rm commands |
| `kg/` | Knowledge graph extraction and querying |
| `memory/` | Memory base for decisions, notes, context |
| `bm25/` | Sparse retrieval with BM25 scoring |
| `vector_index/` | Vector index trait and metadata |

## Dependencies

### Internal (Workspace)

| Crate | Purpose |
|-------|---------|
| `gik-db` | Vector storage and KG persistence (via adapter) |
| `gik-model` | Embedding and reranking models (via adapter) |
| `gik-utils` | URL fetching and HTML parsing |

### External

| Crate | Purpose |
|-------|---------|
| `thiserror` | Derive macro for error types |
| `anyhow` | Error context propagation |
| `serde` / `serde_json` | Serialization for config and data |
| `chrono` | Timestamps for revisions |
| `uuid` | Unique identifiers |
| `tracing` | Structured logging |
| `rayon` | Parallel file processing |
| `walkdir` | Directory traversal |
| `ignore` | Gitignore-style filtering |
| `regex` | Pattern matching |
| `rust-stemmers` | Word stemming for BM25 |
| `bincode` | Binary serialization for BM25 index |

## Usage

### Creating an Engine

```rust
use gik_core::{GikEngine, GlobalConfig};

// Load configuration
let config = GlobalConfig::load(None)?;

// Create engine for current directory
let engine = GikEngine::new(".", Some(config))?;
```

### Basic Operations

```rust
// Initialize workspace
engine.init()?;

// Stage and commit
engine.add(&["src/"], &AddOptions::default())?;
engine.commit("Index source code")?;

// Query
let results = engine.ask("How does config work?", &AskOptions::default())?;
```

### Configuration Resolution

```rust
use gik_core::GlobalConfig;

// Priority: CLI flags > env vars > config file > defaults
let config = GlobalConfig::load(Some("custom.yaml"))?;

// Check effective values
println!("Device: {:?}", config.device);
println!("Embedding model: {}", config.embedding.model);
```

## Configuration

Configuration is loaded from `gik.yaml` with the following structure:

```yaml
# Device preference
device: auto  # auto | gpu | cpu

# Embedding settings
embedding:
  model: all-MiniLM-L6-v2
  dimensions: 384
  batch_size: 32

# Reranker settings
reranker:
  model: ms-marco-MiniLM-L6-v2
  enabled: true

# Performance tuning
performance:
  embeddingBatchSize: 32
```

## Feature Flags

| Flag | Effect | Propagates To |
|------|--------|---------------|
| `metal` | Enable macOS GPU acceleration | `gik-model/metal` |
| `cuda` | Enable NVIDIA GPU acceleration | `gik-model/cuda` |

## Testing

```bash
# Run unit tests
cargo test -p gik-core

# Run with logging
RUST_LOG=debug cargo test -p gik-core
```

Unit tests are located in `#[cfg(test)]` modules within each source file.

## Versioning

This crate follows the workspace version defined in the root `Cargo.toml`.
See [CHANGELOG.md](./CHANGELOG.md) for version history.

## Related Documentation

- [Crates Overview](../../.guided/architecture/crates-overview.md) — All crates in the workspace
- [Architecture Document](../../docs/5-ARCH.md) — Global architecture view
- [Entity Definitions](../../docs/2-ENTITIES.md) — Domain entity specifications
- [API Contracts](../../docs/3-CONTRACTS.md) — Interface contracts
