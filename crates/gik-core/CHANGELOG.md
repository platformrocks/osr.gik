# Changelog

All notable changes to `gik-core` will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.1.0] - 2024-12-09

### Added

- Initial release of the `gik-core` library
- `GikEngine` orchestrator as main entry point for all operations
- Workspace initialization with `.guided/knowledge/` structure
- Branch-based knowledge isolation
- Staging system for files with `add` and `rm` operations
- Commit pipeline with chunking, embedding, and vector storage
- RAG query pipeline with hybrid search (dense + sparse) and reranking
- BM25 sparse retrieval with Porter stemming
- RRF (Reciprocal Rank Fusion) for combining dense and sparse results
- Cross-encoder reranking for final result ordering
- Knowledge graph extraction from indexed content
- Memory base for storing decisions, notes, and context
- Timeline/revision system (Init, Commit, Reindex, Release events)
- Release pipeline for changelog generation
- Stack inventory for tech detection
- Configuration system with file, environment, and CLI precedence
- `GikError` with 30+ variants, all with actionable hints
- Adapter modules (`db_adapter`, `model_adapter`) for infrastructure isolation
- Query expansion for improved search recall
- Feature flag propagation for `metal` and `cuda` GPU support

### Architecture

- Domain-first design with clean separation from infrastructure
- Adapter pattern to bridge to `gik-db` and `gik-model`
- `IntoGikResult` trait for ergonomic error conversion
- Synchronous API with async hidden in infrastructure crates
- Parallel file processing with rayon
- Batched embedding generation for performance

### Modules

- `ask` - RAG query pipeline
- `commit` - Indexing pipeline
- `reindex` - Rebuild embeddings pipeline
- `release` - Changelog generation
- `timeline` - Revision history management
- `staging` - File staging operations
- `base` - Knowledge base operations
- `kg/` - Knowledge graph extraction and querying
- `memory/` - Memory base with metrics and pruning
- `bm25/` - Sparse retrieval implementation
- `vector_index/` - Vector index abstractions
