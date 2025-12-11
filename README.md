# GIK – Guided Indexing Kernel

[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE) [![Version](https://img.shields.io/badge/version-0.1.2-green.svg)](CHANGELOG.md) [![Rust](https://img.shields.io/badge/rust-1.85%2B-orange.svg)](https://www.rust-lang.org/)

**GIK is a local-first knowledge engine for software projects.** Think of it as Git for knowledge-while Git tracks the evolution of files and code, GIK tracks the evolution of *knowledge and understanding* of a codebase.

GIK provides:
- **RAG (Retrieval-Augmented Generation)** – Semantic search over code, docs, and structured data
- **Knowledge Graph** – Entities and relationships between files, modules, services, dependencies
- **Memory** – Events, decisions, and rationales logged over time
- **Stack Inventory** – Structural view of the project (files, dependencies, technologies)

---

## Features

- **Hybrid Search**: BM25 lexical + vector similarity with RRF fusion
- **Cross-Encoder Reranking**: Improved relevance scoring
- **Knowledge Graph**: Multi-language symbol extraction (13 languages)
- **Memory System**: Structured events with scope, source, and tags
- **Git Branch Alignment**: Automatically follows Git branches
- **GPU Acceleration**: Metal (macOS) and CUDA (NVIDIA) support
- **Local-First**: All operations work fully offline, no LLM calls

![GIK Terminal Demo](site/gik-terminal-demo.gif)

---

## Quick Install

**Homebrew (macOS/Linux):**
```bash
brew tap platformrocks/gik
brew install gik
```

**Linux / macOS (curl):**
```bash
curl -fsSL https://raw.githubusercontent.com/platformrocks/osr.gik/main/scripts/install.sh | bash
```

**Windows (PowerShell):**
```powershell
irm https://raw.githubusercontent.com/platformrocks/osr.gik/main/scripts/windows-install.ps1 | iex
```

All methods install the GIK binary, models, and default config.

---

## Quick Start

```bash
# Initialize GIK in your project
gik init

# Stage files for indexing
gik add src/

# Commit to create a knowledge revision
gik commit -m "Initial knowledge index"

# Query your codebase
gik ask "How does authentication work?"
```

---

## CLI Commands

| Command | Description |
|---------|-------------|
| `gik init` | Initialize GIK workspace |
| `gik status` | Show workspace status |
| `gik add <PATH>` | Stage sources for indexing |
| `gik rm <PATH>` | Remove from staging |
| `gik commit -m "msg"` | Index staged sources |
| `gik ask <QUERY>` | Query knowledge (RAG) |
| `gik stats` | Show base statistics |
| `gik show [REV]` | Inspect revision |
| `gik reindex` | Rebuild embeddings |
| `gik release --tag <TAG>` | Generate CHANGELOG |
| `gik config check` | Validate configuration |

### Global Flags

| Flag | Description |
|------|-------------|
| `-v, --verbose` | Enable debug logging |
| `-q, --quiet` | Suppress progress messages |
| `-c, --config <PATH>` | Custom config file |
| `--device <auto\|gpu\|cpu>` | Device preference |

---

## Configuration

GIK looks for configuration in this order:
1. CLI flags
2. Environment variables (`GIK_CONFIG`, `GIK_DEVICE`)
3. Project config (`.guided/knowledge/config.yaml`)
4. Global config (`~/.gik/config.yaml`)

Example configuration:

```yaml
device: auto

embeddings:
  default:
    provider: candle
    modelId: sentence-transformers/all-MiniLM-L6-v2
    dimension: 384

retrieval:
  reranker:
    enabled: true
    topK: 30
    finalK: 5
  hybrid:
    enabled: true
    denseWeight: 0.5
    sparseWeight: 0.5

performance:
  embeddingBatchSize: 32
  enableWarmup: true
```

See [`config.default.yaml`](config.default.yaml) for full options.


## Storage Layout

```
.guided/knowledge/<branch>/
├── HEAD                    # Current revision ID
├── timeline.jsonl          # Revision history
├── staging/                # Staged sources
├── bases/
│   ├── code/               # Code chunks + embeddings
│   ├── docs/               # Documentation chunks
│   └── memory/             # Memory entries
├── stack/                  # Project inventory
└── kg/                     # Knowledge graph
```

---

## License

[MIT](LICENSE) © 2025 PLATFORM ROCKS LTDA.

---

## Links

- [Documentation](https://docs.rs/gik-core)
- [Repository](https://github.com/platformrocks/osr.gik)
- [Changelog](CHANGELOG.md)
