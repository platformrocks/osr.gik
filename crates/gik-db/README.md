# gik-db

> Vector storage and knowledge graph persistence for GIK—isolates heavy dependencies.

## Overview

`gik-db` is the infrastructure library crate that handles all persistent storage for GIK. It provides vector index operations via LanceDB and knowledge graph persistence. This crate isolates heavy dependencies (LanceDB, Arrow, async runtimes) from `gik-core`, exposing them through trait-based abstractions. Internally, it wraps async LanceDB operations with a sync interface using `tokio::runtime::block_on()`.

## Goals

- **Dependency isolation**: Keep LanceDB, Arrow, and tokio out of `gik-core`
- **Trait-based abstraction**: Expose storage operations through `VectorIndexBackend` and `KgStoreBackend` traits
- **Sync interface**: Hide async complexity from consumers
- **Backend flexibility**: Support different backends via feature flags (LanceDB for production, simple file-based for testing)

## Features

- Provides `VectorIndexBackend` trait for vector operations (create, upsert, query, delete)
- Provides `KgStoreBackend` trait for knowledge graph operations (nodes, edges, queries)
- Implements LanceDB backend with ANN (Approximate Nearest Neighbor) search
- Implements simple file-based backend for testing scenarios
- Manages Arrow schema definitions for vector storage
- Handles async-to-sync bridging internally

## Architecture

### Module Overview

```
src/
├── lib.rs              # Re-exports, factory functions
├── error.rs            # DbError, DbResult (~146 lines)
│
├── vector/
│   ├── mod.rs          # Re-exports
│   ├── config.rs       # VectorIndexConfig, metadata structs
│   ├── traits.rs       # VectorIndexBackend trait
│   ├── metadata.rs     # VectorMetadata, filter types
│   └── backend/
│       ├── mod.rs
│       ├── lancedb.rs  # LanceDB implementation
│       └── simple.rs   # File-based implementation (feature-gated)
│
└── kg/
    ├── mod.rs          # Re-exports (~100 lines)
    ├── entities.rs     # KgNode, KgEdge, KgStats
    ├── traits.rs       # KgStoreBackend trait
    └── backend/
        ├── mod.rs
        └── lancedb.rs  # LanceDB KG implementation
```

### Key Types

| Type | Role |
|------|------|
| `DbError` | Error type with IO, LanceDB, and serialization variants |
| `DbResult<T>` | Alias for `Result<T, DbError>` |
| `VectorIndexBackend` | Trait for vector index operations |
| `KgStoreBackend` | Trait for knowledge graph operations |
| `VectorIndexConfig` | Configuration for vector index creation |
| `VectorMetadata` | Metadata attached to vectors |
| `KgNode` | Knowledge graph node entity |
| `KgEdge` | Knowledge graph edge/relationship |
| `KgStats` | Knowledge graph statistics |

### Trait Contracts

**VectorIndexBackend**:
```rust
pub trait VectorIndexBackend: Send + Sync {
    fn create(&self, config: &VectorIndexConfig) -> DbResult<()>;
    fn upsert(&self, vectors: &[VectorRecord]) -> DbResult<usize>;
    fn query(&self, embedding: &[f32], top_k: usize, filter: Option<&Filter>) -> DbResult<Vec<SearchResult>>;
    fn delete(&self, ids: &[String]) -> DbResult<usize>;
    fn count(&self) -> DbResult<usize>;
    fn exists(&self) -> DbResult<bool>;
}
```

**KgStoreBackend**:
```rust
pub trait KgStoreBackend: Send + Sync {
    fn upsert_nodes(&self, nodes: &[KgNode]) -> DbResult<usize>;
    fn upsert_edges(&self, edges: &[KgEdge]) -> DbResult<usize>;
    fn query_nodes(&self, query: &KgNodeQuery) -> DbResult<Vec<KgNode>>;
    fn query_edges(&self, query: &KgEdgeQuery) -> DbResult<Vec<KgEdge>>;
    fn stats(&self) -> DbResult<KgStats>;
}
```

### Async Bridging

LanceDB is async, but `gik-core` is sync. This crate bridges the gap:

```rust
impl VectorIndexBackend for LanceDbBackend {
    fn query(&self, embedding: &[f32], top_k: usize, filter: Option<&Filter>) -> DbResult<Vec<SearchResult>> {
        self.runtime.block_on(async {
            // Async LanceDB operations here
            self.table.search(embedding).limit(top_k).execute().await
        })
    }
}
```

The `tokio::runtime::Runtime` is created once per backend instance and reused.

## Dependencies

### External

| Crate | Purpose |
|-------|---------|
| `lancedb` | Vector database with ANN search (feature-gated) |
| `arrow` / `arrow-array` / `arrow-schema` | Arrow data structures for LanceDB |
| `tokio` | Async runtime for LanceDB operations |
| `futures` | Async utilities |
| `thiserror` | Error derive macro |
| `serde` / `serde_json` | Serialization |
| `tracing` | Structured logging |

## Usage

### Creating a Vector Index

```rust
use gik_db::{create_vector_backend, VectorIndexConfig};

let backend = create_vector_backend("/path/to/vectors")?;

let config = VectorIndexConfig {
    dimensions: 384,
    metric: "cosine".to_string(),
    ..Default::default()
};

backend.create(&config)?;
```

### Upserting and Querying Vectors

```rust
use gik_db::{VectorRecord, VectorMetadata};

// Upsert vectors
let records = vec![
    VectorRecord {
        id: "chunk-1".to_string(),
        embedding: vec![0.1, 0.2, ...],  // 384 dimensions
        metadata: VectorMetadata { ... },
    },
];
backend.upsert(&records)?;

// Query similar vectors
let query_embedding = vec![0.1, 0.2, ...];
let results = backend.query(&query_embedding, 10, None)?;
```

### Knowledge Graph Operations

```rust
use gik_db::{create_kg_backend, KgNode, KgEdge};

let kg = create_kg_backend("/path/to/kg")?;

// Upsert nodes
let nodes = vec![
    KgNode {
        id: "fn:main".to_string(),
        label: "main".to_string(),
        kind: "function".to_string(),
        ..Default::default()
    },
];
kg.upsert_nodes(&nodes)?;

// Query nodes
let query = KgNodeQuery { kind: Some("function".to_string()), ..Default::default() };
let functions = kg.query_nodes(&query)?;
```

## Feature Flags

| Flag | Default | Effect |
|------|---------|--------|
| `lancedb` | ✓ | Enable LanceDB backend with ANN search |
| `simple` | — | Enable file-based backend for testing |

Build with specific backend:

```bash
# Production (default)
cargo build -p gik-db

# Testing backend only
cargo build -p gik-db --no-default-features --features simple
```

## Testing

```bash
# Run all tests
cargo test -p gik-db

# Run with specific backend
cargo test -p gik-db --features simple
```

## Versioning

This crate follows the workspace version defined in the root `Cargo.toml`.
See [CHANGELOG.md](./CHANGELOG.md) for version history.

## Related Documentation

- [Crates Overview](../../.guided/architecture/crates-overview.md) — All crates in the workspace
- [Architecture Document](../../docs/5-ARCH.md) — Global architecture view
- [gik-core README](../gik-core/README.md) — How core uses this crate via adapters
