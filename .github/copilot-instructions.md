# GIK Copilot Instructions

GIK (Guided Indexing Kernel) is a local-first knowledge engine for projects. Similar to how Git tracks files, GIK tracks knowledge evolution—providing RAG, knowledge graphs, memory, and stack inventory.

---

## Architecture Overview

```
gik-cli (thin UX layer: CLI parsing, user output)
    ↓ calls GikEngine
gik-core (domain logic, orchestration, pipelines)
    ↓ adapters (db_adapter.rs, model_adapter.rs)
gik-db (LanceDB vectors, KG persistence)    gik-model (Candle embeddings/reranking)
    ↓                                            ↓
gik-utils (URL fetching, HTML parsing)
```

**Key principle**: Heavy dependencies (LanceDB, Candle, Arrow) are isolated in leaf crates. Core never imports them directly—uses adapter traits.

### Crate Responsibilities

| Crate | Owns | Forbidden Dependencies |
|-------|------|----------------------|
| `gik-core` | Domain types, `GikEngine`, pipelines (ask, commit, reindex, release) | `lancedb`, `candle-*`, `arrow-*` |
| `gik-cli` | CLI parsing (clap), `Style` helpers, no business logic | Direct storage access |
| `gik-db` | Vector index (LanceDB), KG store, Arrow schema wrappers | ML inference |
| `gik-model` | Embedding models (Candle), reranker, model locator | Storage concerns |
| `gik-utils` | URL fetching, HTML parsing utilities | None specific |

---

## Storage Layout

```
.guided/knowledge/<branch>/
├── HEAD                    # Current revision ID (e.g., "abc123")
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
└── kg/                     # Knowledge graph (lazy init)
    ├── nodes.jsonl         # Entity nodes
    ├── edges.jsonl         # Relationship edges
    └── stats.json          # KG statistics
```

---

## CLI Commands Reference

### Global Flags
| Flag | Description |
|------|-------------|
| `-v, --verbose` | Enable debug logging |
| `-q, --quiet` | Suppress progress messages |
| `-c, --config <PATH>` | Custom config file |
| `--device <auto\|gpu\|cpu>` | Device preference |
| `--color <auto\|always\|never>` | Color output mode |

### Commands

| Command | Description | Key Flags |
|---------|-------------|-----------|
| `init` | Initialize GIK workspace | — |
| `status` | Show workspace status | `--json` |
| `bases` | List knowledge bases | — |
| `add <PATH>` | Stage sources for indexing | `-b, --base <code\|docs\|memory>`, `--url`, `-t, --type`, `--metadata` |
| `rm <PATH>` | Remove from staging | — |
| `commit` | Index staged sources | `-m, --message <MSG>` |
| `show` | Inspect revision (like git show) | `[REV]`, `-b, --branch`, `--kg-dot`, `--kg-mermaid`, `--json` |
| `ask <QUERY>` | Query knowledge (RAG) | `-b, --base`, `--top-k <N>`, `--memory`, `--url`, `--json` |
| `stats` | Show base statistics | `-b, --base`, `--json` |
| `reindex` | Rebuild embeddings | `-b, --base`, `--force`, `--dry-run`, `--json` |
| `release` | Generate CHANGELOG | `--from <REV>`, `--to <REV>`, `-o, --output <PATH>`, `--format <md\|json>`, `--tag <TAG>`, `--dry-run` |
| `config check` | Validate configuration | `--json` |
| `config show` | Show resolved config | `--json` |

---

## Docker Build & Test

### Build Scripts

Use `build.ps1` (Windows) or `build.sh` (Linux/Mac):

| Command | Description |
|---------|-------------|
| `release` | Linux x86_64 release binary |
| `release-windows` | Windows x86_64 (MinGW cross-compile) |
| `release-cuda [CAP]` | CUDA GPU build (default: 86 for RTX 30xx) |
| `install` | Build Windows and install to `~/.cargo/bin` |
| `dev` | Start development shell with tools |
| `test` | Run all tests in Docker |
| `test-unit` | Run unit tests only (fast) |
| `test-integration` | Run integration tests (needs models) |
| `fmt` | Format with rustfmt |
| `clippy` | Run clippy linter |
| `clean` | Remove all build artifacts |

### Dockerfiles

| File | Purpose | Base Image |
|------|---------|------------|
| `Dockerfile` | Linux x86_64 production | `rust:slim-bookworm` → `debian:bookworm-slim` |
| `Dockerfile.windows` | Windows cross-compile | `rust:slim-bookworm` + MinGW-w64 |
| `Dockerfile.cuda` | CUDA GPU support | `nvidia/cuda:12.4.0-devel-ubuntu22.04` |
| `Dockerfile.dev` | Development environment | `rust:bookworm` |

### CUDA Compute Capabilities
- `70`: V100
- `75`: RTX 20xx (Turing)
- `80`: A100
- `86`: RTX 30xx (Ampere) — default
- `89`: RTX 40xx (Ada)
- `90`: H100 (Hopper)

### Examples

```powershell
# Windows (PowerShell)
.\build.ps1 install                    # Build + install gik.exe
.\build.ps1 release-cuda 89            # RTX 40xx CUDA build
.\build.ps1 test-unit                  # Fast unit tests
```

```bash
# Linux/Mac (Bash)
./build.sh install                     # Build + install gik
./build.sh release-cuda 89             # RTX 40xx CUDA build
./build.sh test                        # All tests
```

---

## Testing Patterns

### Test Types

| Type | Location | Command | Speed |
|------|----------|---------|-------|
| Unit | `#[cfg(test)]` in modules | `cargo test -p gik-core` | Fast |
| Integration | `gik-cli/tests/` | `cargo test -p gik-cli` | Needs models |
| Ignored | Require real ML models | `cargo test -p gik-cli -- --ignored` | Slow |

### Integration Test Files
- `integration_flow.rs` — Full init→add→commit→ask flow
- `memory_ask_and_status.rs` — Memory ingestion/retrieval
- `kg_extraction_from_bases.rs` — Knowledge graph extraction
- `release_and_changelog.rs` — Release generation
- `show_cli.rs` — Show command tests

### Test Helper
```rust
// gik-cli/tests/common/mod.rs
pub fn gik_cmd() -> Command {
    Command::cargo_bin("gik").unwrap()
}
```

---

## Rust Best Practices

### Error Handling (thiserror)

```rust
#[derive(Error, Debug)]
pub enum GikError {
    #[error("Base `{base}` exists but has no indexed content. Run `gik add` and `gik commit` first.")]
    BaseNotIndexed { base: String },
    
    #[error("Embedding model mismatch for base `{base}`: index uses `{index_model}`, active is `{active_model}`.")]
    EmbeddingModelMismatch { base: String, index_model: String, active_model: String },
}
```

**Rule**: Every error variant must include an actionable hint.

### Adapter Pattern (IntoGikResult)

```rust
// gik-core/src/db_adapter.rs
pub trait IntoGikResult<T> {
    fn into_gik_result(self) -> Result<T, GikError>;
}

impl<T> IntoGikResult<T> for gik_db::DbResult<T> {
    fn into_gik_result(self) -> Result<T, GikError> {
        self.map_err(from_db_error)
    }
}
```

**Rule**: At public API boundaries, convert errors using `.into_gik_result()`.

### Async Wrapping (block_on)

```rust
// gik-db wraps async LanceDB with sync interface
pub fn query(&self, embedding: &[f32], top_k: usize) -> DbResult<Vec<SearchResult>> {
    self.runtime.block_on(async { /* async LanceDB calls */ })
}
```

**Rule**: Keep `gik-core` sync. Async is isolated in `gik-db`.

### Logging (tracing)

```rust
tracing::debug!("Failed to read Cargo.toml at {}: {}", path.display(), e);
tracing::info!("Initialized GIK workspace at {} on branch {}", root, branch);
```

**Rule**: Use `tracing` macros (`debug!`, `info!`, `warn!`, `error!`), never `println!` in library code.

### CLI Output (Style helpers)

```rust
// gik-cli/src/ui/style.rs
println!("{}", style.message(MessageType::Ok, "Staged sources"));
println!("{}", style.key_value("Branch", &branch.to_string()));
println!("{}", style.error_with_context("Failed", Some(&reason), Some("Try X")));
```

**Rule**: All CLI output through `Style` helpers. Never raw `println!` in CLI handlers.

### Feature Flags

```toml
# Cargo.toml
[features]
default = []
metal = ["gik-model/metal"]  # macOS GPU acceleration
cuda = ["gik-model/cuda"]    # NVIDIA GPU acceleration
```

---

## Project Patterns & Conventions

### Config Priority (highest → lowest)
1. CLI flags (`--device`, `--config`)
2. Environment variables (`GIK_CONFIG`, `GIK_DEVICE`, `GIK_VERBOSE`)
3. Config file (`gik.yaml` or `--config`)
4. Built-in defaults

### Model Search Paths
1. `$GIK_MODELS_DIR` environment variable
2. `~/.gik/models` user directory
3. `{exe_dir}/models` next to binary

### Performance Optimizations
- **Batched embeddings**: Default batch size 32 (`performance.embeddingBatchSize`)
- **Parallel file reading**: Using rayon
- **Warm-up embedding**: Pays initialization cost upfront
- **Hybrid search**: BM25 + vector search with RRF fusion
- **Dev profile optimization**: Heavy deps with `opt-level=2`

### Adding New Features

1. **New command**: Add to `gik-cli/src/cli.rs` (`Command` enum), implement handler, delegate to `GikEngine`
2. **New engine method**: Add to `gik-core/src/engine.rs`, use adapters for storage/ML
3. **New error**: Add variant to `gik-core/src/errors.rs`, include actionable hints

---

## Models Setup

GIK requires local HuggingFace models:

```bash
# Embeddings (384 dimensions)
git clone https://huggingface.co/sentence-transformers/all-MiniLM-L6-v2 models/embeddings/all-MiniLM-L6-v2

# Reranker
git clone https://huggingface.co/cross-encoder/ms-marco-MiniLM-L6-v2 models/rerankers/ms-marco-MiniLM-L6-v2
```

---

## Development Workflows

```bash
# Local development
cargo build -p gik-cli
cargo run -p gik-cli -- init
cargo run -p gik-cli -- add . && cargo run -p gik-cli -- commit -m "Index"

# Quick iteration
cargo watch -x "test -p gik-core"

# Format and lint
cargo fmt && cargo clippy -p gik-core -p gik-cli -- -D warnings

# Docker development
.\build.ps1 dev          # Start dev shell (Windows)
./build.sh dev           # Start dev shell (Linux/Mac)
```

---

## Key Files Reference

| Purpose | Location |
|---------|----------|
| Engine entry point | `gik-core/src/engine.rs` |
| CLI command dispatch | `gik-cli/src/cli.rs` |
| Error types | `gik-core/src/errors.rs` |
| Embedding abstraction | `gik-core/src/embedding.rs` |
| Vector index trait | `gik-core/src/vector_index.rs` |
| DB adapter | `gik-core/src/db_adapter.rs` |
| Model adapter | `gik-core/src/model_adapter.rs` |
| Config resolution | `gik-core/src/config.rs` |
| Ask pipeline | `gik-core/src/ask.rs` |
| Commit pipeline | `gik-core/src/commit.rs` |
| Memory module | `gik-core/src/memory/` |
| KG module | `gik-core/src/kg/` |
| CLI UI helpers | `gik-cli/src/ui/` |
| Integration tests | `gik-cli/tests/` |
| Build scripts | `build.ps1`, `build.sh` |
| Docker docs | `README-DOCKER.md` |
