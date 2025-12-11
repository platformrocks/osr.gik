# GIK Architecture

This document provides a consolidated architecture view of the Guided Indexing Kernel (GIK), structured around the `crates/` workspace.

---

## Overview

GIK is a local-first knowledge engine for software projects. Similar to how Git tracks file changes, GIK tracks knowledge evolution—providing RAG (Retrieval-Augmented Generation), knowledge graphs, memory persistence, and stack inventory.

The system is built as a Rust workspace with strict layering principles:
- **Heavy dependencies** (LanceDB, Candle, Arrow) are isolated in leaf crates
- **Core logic** never imports infrastructure crates directly—uses adapter traits
- **CLI** is a thin UX layer with zero business logic

---

## Architectural Layers

```
┌─────────────────────────────────────────────────────────────────────────┐
│                         Layer 1: CLI (User Interface)                   │
│                                                                         │
│  ┌───────────────────────────────────────────────────────────────────┐  │
│  │                           gik-cli                                 │  │
│  │  • CLI parsing (clap)                                             │  │
│  │  • Output formatting (Style helpers)                              │  │
│  │  • Command dispatch to GikEngine                                  │  │
│  │  • Progress indicators                                            │  │
│  └───────────────────────────────────────────────────────────────────┘  │
│                                    │                                    │
│                                    ▼                                    │
├─────────────────────────────────────────────────────────────────────────┤
│                       Layer 2: Core (Domain Logic)                      │
│                                                                         │
│  ┌───────────────────────────────────────────────────────────────────┐  │
│  │                           gik-core                                │  │
│  │  • GikEngine orchestrator                                         │  │
│  │  • Pipelines: ask, commit, reindex, release                       │  │
│  │  • Domain types and errors                                        │  │
│  │  • Configuration management                                       │  │
│  │  • Adapter bridges to infrastructure                              │  │
│  └───────────────────────────────────────────────────────────────────┘  │
│                          │         │         │                          │
│                          ▼         ▼         ▼                          │
├─────────────────────────────────────────────────────────────────────────┤
│                    Layer 3: Infrastructure (Leaf Crates)                │
│                                                                         │
│  ┌─────────────────┐  ┌─────────────────┐  ┌─────────────────┐          │
│  │     gik-db      │  │    gik-model    │  │    gik-utils    │          │
│  │                 │  │                 │  │                 │          │
│  │ • Vector index  │  │ • Embeddings    │  │ • URL fetching  │          │
│  │ • KG storage    │  │ • Reranking     │  │ • HTML parsing  │          │
│  │ • LanceDB       │  │ • Candle        │  │ • Chrome        │          │
│  └─────────────────┘  └─────────────────┘  └─────────────────┘          │
│                                                                         │
└─────────────────────────────────────────────────────────────────────────┘
```

---

## Crate Responsibilities

| Crate | Layer | Purpose | Key Types |
|-------|-------|---------|-----------|
| `gik-cli` | CLI | User interaction, command dispatch | `Cli`, `Command`, `Style` |
| `gik-core` | Core | Domain logic, orchestration | `GikEngine`, `GikError`, `GlobalConfig` |
| `gik-db` | Infrastructure | Vector storage, KG persistence | `VectorIndexBackend`, `KgStoreBackend` |
| `gik-model` | Infrastructure | ML inference | `EmbeddingModel`, `RerankerModel` |
| `gik-utils` | Infrastructure | Utilities | `fetch_url_as_markdown` |

---

## Dependency Rules

The architecture enforces strict dependency rules to maintain clean layering:

| Crate | May Depend On | May NOT Depend On |
|-------|---------------|-------------------|
| `gik-cli` | `gik-core` | `gik-db`, `gik-model`, `gik-utils` |
| `gik-core` | `gik-db`, `gik-model`, `gik-utils` | Direct LanceDB, Candle, Arrow imports |
| `gik-db` | External storage crates | `gik-core`, `gik-model` |
| `gik-model` | External ML crates | `gik-core`, `gik-db` |
| `gik-utils` | External utility crates | `gik-core`, `gik-db`, `gik-model` |

**Key Principle**: Infrastructure crates are leaf nodes—they never depend on each other or on core.

---

## Adapter Pattern

`gik-core` bridges to infrastructure crates using adapter modules that:
1. Convert error types to `GikError`
2. Wrap trait implementations
3. Convert config and data types

### db_adapter.rs

```rust
// Error conversion
pub fn from_db_error(err: gik_db::DbError) -> GikError

// Extension trait for Result conversion
pub trait IntoGikResult<T> {
    fn into_gik_result(self) -> Result<T, GikError>;
}

// Wrappers
pub struct DbVectorIndex   // wraps gik_db::VectorIndexBackend
pub struct DbKgStore       // wraps gik_db::KgStoreBackend
```

### model_adapter.rs

```rust
// Error conversion
pub fn from_model_error(err: gik_model::ModelError) -> GikError

// Config conversion
pub fn to_model_embedding_config(...) -> gik_model::EmbeddingConfig

// Factory wrappers
pub fn create_embedding_backend(...) -> Box<dyn EmbeddingBackend>
pub fn create_reranker_backend(...) -> Box<dyn RerankerModel>
```

---

## Data Flows

### `gik init`

```
CLI: gik init
        │
        ▼
GikEngine::init()
        │
        ├─→ Create .guided/knowledge/<branch>/ structure
        ├─→ Initialize HEAD, timeline.jsonl
        ├─→ Create staging/, bases/, stack/, kg/ directories
        └─→ Write Init revision to timeline
```

### `gik add` + `gik commit`

```
CLI: gik add src/
        │
        ▼
GikEngine::add()
        │
        └─→ Write entries to staging/sources.jsonl

CLI: gik commit -m "Index"
        │
        ▼
GikEngine::commit()
        │
        ├─→ Read staged sources
        ├─→ Chunk files into segments
        ├─→ Generate embeddings (batched) ─────→ gik-model
        ├─→ Upsert vectors ────────────────────→ gik-db
        ├─→ Update BM25 index
        ├─→ Extract KG entities (optional) ───→ gik-db
        ├─→ Write Commit revision to timeline
        └─→ Clear staging
```

### `gik ask`

```
CLI: gik ask "How does config work?"
        │
        ▼
GikEngine::ask()
        │
        ├─→ Query expansion (optional)
        │
        ├─→ Dense search ─────────────────────→ gik-model (embed)
        │       │                                    │
        │       └──────→ gik-db (vector query) ←─────┘
        │
        ├─→ Sparse search (BM25) ─────────────→ internal index
        │
        ├─→ RRF fusion (combine dense + sparse)
        │
        ├─→ Cross-encoder reranking ──────────→ gik-model (rerank)
        │
        └─→ Return top-k results
```

### `gik release`

```
CLI: gik release --from v0.1.0 --to HEAD
        │
        ▼
GikEngine::release()
        │
        ├─→ Read timeline revisions in range
        ├─→ Aggregate changes by type
        ├─→ Format as Markdown/JSON
        └─→ Write to CHANGELOG or stdout
```

---

## Storage Layout

```
.guided/knowledge/<branch>/
├── HEAD                    # Current revision ID
├── timeline.jsonl          # Revision history (Init, Commit, Reindex, Release)
├── staging/
│   └── sources.jsonl       # Staged file entries
├── bases/
│   ├── code/               # Code chunks + embeddings
│   │   ├── sources.jsonl   # Indexed source metadata
│   │   ├── stats.json      # Base statistics
│   │   ├── model-info.json # Embedding model metadata
│   │   ├── meta.json       # Vector index metadata
│   │   └── vectors/        # LanceDB vector storage
│   ├── docs/               # Documentation chunks
│   └── memory/             # Memory entries (decisions, notes)
├── stack/
│   ├── files.jsonl         # All files in workspace
│   ├── dependencies.jsonl  # Parsed dependencies
│   ├── tech.jsonl          # Detected technologies
│   └── stats.json          # Stack statistics
└── kg/                     # Knowledge graph
    ├── nodes.jsonl         # Entity nodes
    ├── edges.jsonl         # Relationship edges
    └── stats.json          # KG statistics
```

---

## Cross-Cutting Concerns

### Error Handling

All crates use `thiserror` with **actionable error messages**:

```rust
#[error("Base `{base}` exists but has no indexed content. Run `gik add` and `gik commit` first.")]
BaseNotIndexed { base: String },
```

Every error includes:
- What went wrong
- Relevant context (base name, path, etc.)
- How to fix it

### Configuration

Configuration follows precedence:
1. CLI flags (`--device`, `--config`)
2. Environment variables (`GIK_CONFIG`, `GIK_DEVICE`)
3. Config file (`gik.yaml`)
4. Built-in defaults

`gik-core` owns `GlobalConfig` and `ProjectConfig` types.

### Logging

All crates use `tracing` for structured logging:

```rust
tracing::debug!("Processing file: {}", path.display());
tracing::info!("Indexed {} chunks", count);
tracing::warn!("Model mismatch, reindex recommended");
```

CLI controls verbosity via `--verbose` / `--quiet` flags.

### Sync vs Async

- `gik-core` and `gik-cli` are **synchronous**
- `gik-db` wraps async LanceDB with `tokio::runtime::block_on()`
- This keeps the API simple while supporting async infrastructure

---

## Feature Flags

Feature flags propagate through the workspace:

```
gik-cli --features cuda
    └─→ gik-core --features cuda
            └─→ gik-model --features cuda
                    └─→ candle-core --features cuda
```

| Flag | Effect |
|------|--------|
| `metal` | Enable macOS GPU acceleration |
| `cuda` | Enable NVIDIA GPU acceleration |

`gik-db` has separate flags:
| Flag | Effect |
|------|--------|
| `lancedb` | Production vector store (default) |
| `simple` | File-based backend for testing |

---

## Extension Points

### Adding a New Knowledge Base

1. Define base type in `gik-core/src/base.rs`
2. Add chunking logic in `gik-core/src/commit.rs`
3. Update storage paths in `gik-core/src/constants.rs`

### Adding a New Embedding Provider

1. Implement `EmbeddingModel` trait in `gik-model`
2. Add factory function with feature flag
3. Update `model_adapter.rs` to expose new backend

### Adding a New CLI Command

1. Add variant to `Command` enum in `gik-cli/src/cli.rs`
2. Implement handler that delegates to `GikEngine`
3. Add integration test in `gik-cli/tests/`

---

## Per-Crate Documentation

For detailed documentation on each crate:

- [gik-cli README](../crates/gik-cli/README.md) — CLI binary
- [gik-core README](../crates/gik-core/README.md) — Core library
- [gik-db README](../crates/gik-db/README.md) — Database layer
- [gik-model README](../crates/gik-model/README.md) — ML inference
- [gik-utils README](../crates/gik-utils/README.md) — Utilities

See also:
- [Crates Overview](../.guided/architecture/crates-overview.md) — Inventory and dependency graph
- [Crates README Template](../.guided/architecture/crates-readmes-template.md) — Documentation standards

---

## Related Documentation

- [Product Requirements (0-PRD.md)](./0-PRD.md) — Product vision and requirements
- [Specification (1-SPEC.md)](./1-SPEC.md) — Detailed specification
- [Entity Definitions (2-ENTITIES.md)](./2-ENTITIES.md) — Domain entities
- [API Contracts (3-CONTRACTS.md)](./3-CONTRACTS.md) — Interface contracts
- [Command Reference (4-COMMANDS.md)](./4-COMMANDS.md) — CLI commands
