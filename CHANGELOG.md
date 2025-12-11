# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.1.0] - 2025-12-09

### Added

- **Core Engine**: Git-inspired knowledge workflow with `init`, `add`, `commit`, `status`, and `show` commands
- **RAG Pipeline**: Semantic search with hybrid BM25 + vector similarity using RRF fusion
- **Cross-Encoder Reranking**: Improved relevance scoring for search results
- **Knowledge Graph**: Multi-language symbol extraction supporting 13 programming languages
- **Memory System**: Structured events with scope, source, tags, and pruning support
- **Stack Inventory**: Automatic detection of files, dependencies, and technologies
- **Multi-Base Storage**: Separate bases for `code`, `docs`, and `memory` content
- **Query Expansion**: Enhanced recall through semantic query expansion
- **Git Branch Alignment**: Automatic alignment with Git branches (fallback to `main`)
- **CHANGELOG Generation**: `gik release` command for automated changelog creation
- **Revision Inspection**: `gik show` with DOT/Mermaid KG export formats
- **Docker Build System**: Cross-platform builds for Linux, Windows, and CUDA
- **GPU Acceleration**: Metal (macOS) and CUDA (NVIDIA) support via feature flags
- **Configuration System**: YAML config with CLI, environment, and file-based overrides
- **Local Embeddings**: Candle-based inference with `all-MiniLM-L6-v2` model
- **Reranker Support**: Optional `ms-marco-MiniLM-L6-v2` cross-encoder

[0.1.0]: https://github.com/platformrocks/osr.gik/releases/tag/v0.1.0
