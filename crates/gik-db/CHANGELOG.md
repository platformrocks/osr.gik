# Changelog

All notable changes to `gik-db` will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.1.0] - 2024-12-09

### Added

- Initial release of the `gik-db` library
- `VectorIndexBackend` trait for vector operations
- `KgStoreBackend` trait for knowledge graph persistence
- LanceDB backend implementation with ANN search
- Simple file-based backend for testing (feature-gated)
- Arrow schema definitions for vector storage
- Vector metadata support with filtering
- Knowledge graph entities: `KgNode`, `KgEdge`, `KgStats`
- Node and edge query operations
- `DbError` with IO, LanceDB, and serialization variants
- Async-to-sync bridging using `tokio::runtime::block_on()`

### Architecture

- Trait-based abstraction for backend flexibility
- Feature flags for backend selection (`lancedb`, `simple`)
- Internal tokio runtime for async LanceDB operations
- Clean separation between vector and KG modules
