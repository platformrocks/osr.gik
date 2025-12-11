# Contributing to GIK

Thank you for your interest in contributing to GIK! This document provides guidelines and instructions for contributing.

## Code of Conduct

By participating in this project, you agree to abide by our [Code of Conduct](CODE_OF_CONDUCT.md).

## How to Contribute

### Reporting Bugs

Before creating a bug report, please check existing issues to avoid duplicates. When creating a bug report, include:

- **Clear title** describing the issue
- **Steps to reproduce** the behavior
- **Expected behavior** vs. actual behavior
- **Environment details** (OS, Rust version, GIK version)
- **Relevant logs** with `--verbose` flag output

### Suggesting Features

Feature requests are welcome! Please include:

- **Use case** – What problem does this solve?
- **Proposed solution** – How should it work?
- **Alternatives considered** – Other approaches you've thought of

### Pull Requests

1. **Fork the repository** and create your branch from `main`
2. **Follow the architecture** – See `.github/copilot-instructions.md` for guidelines
3. **Write tests** – Add unit/integration tests for new functionality
4. **Update documentation** – Keep docs in sync with changes
5. **Follow conventions** – Use the coding patterns described below

## Development Setup

### Prerequisites

- Rust 1.85+
- Docker Desktop (for containerized builds)
- ~2GB disk space for models

### Building

```bash
# Clone the repository
git clone https://github.com/platformrocks/osr.gik.git
cd osr.gik

# Build the CLI
cargo build -p gik-cli

# Run tests
cargo test
```

### Using Docker

```bash
# Linux/Mac
./scripts/build.sh dev      # Start development shell
./scripts/build.sh test     # Run all tests
./scripts/build.sh fmt      # Format code
./scripts/build.sh clippy   # Lint code

# Windows (PowerShell)
.\scripts\build.ps1 dev
.\scripts\build.ps1 test
```

### Model Setup

```bash
# Download models for testing
./scripts/gik-download-models.sh
```

### Model Setup (FROM SOURCE)

GIK requires local HuggingFace models for embeddings:

```bash
# Create models directory
mkdir -p ~/.gik/models/embeddings ~/.gik/models/rerankers

# Clone embedding model (384 dimensions)
git clone https://huggingface.co/sentence-transformers/all-MiniLM-L6-v2 \
    ~/.gik/models/embeddings/all-MiniLM-L6-v2

# Clone reranker model (optional, improves results)
git clone https://huggingface.co/cross-encoder/ms-marco-MiniLM-L6-v2 \
    ~/.gik/models/rerankers/ms-marco-MiniLM-L6-v2
```

## Coding Guidelines

### Architecture Rules

| Crate | Owns | Forbidden Dependencies |
|-------|------|----------------------|
| `gik-core` | Domain types, `GikEngine`, pipelines | `lancedb`, `candle-*`, `arrow-*` |
| `gik-cli` | CLI parsing, no business logic | Direct storage access |
| `gik-db` | Vector index, KG store | ML inference |
| `gik-model` | Embedding models, reranker | Storage concerns |
| `gik-utils` | URL fetching, HTML parsing | None specific |

### Error Handling

Use `thiserror` with actionable hints:

```rust
#[derive(Error, Debug)]
pub enum GikError {
    #[error("Base `{base}` not indexed. Run `gik add` and `gik commit` first.")]
    BaseNotIndexed { base: String },
}
```

### Logging

Use `tracing` macros, never `println!` in library code:

```rust
tracing::debug!("Processing file: {}", path.display());
tracing::info!("Indexed {} chunks", count);
```

### CLI Output

Use `Style` helpers for consistent output:

```rust
println!("{}", style.message(MessageType::Ok, "Done"));
println!("{}", style.key_value("Branch", &branch));
```

### Adapter Pattern

Convert errors at public API boundaries:

```rust
db_result.into_gik_result()?;
```

## Testing

### Test Types

| Type | Location | Command |
|------|----------|---------|
| Unit | `#[cfg(test)]` in modules | `cargo test -p gik-core` |
| Integration | `gik-cli/tests/` | `cargo test -p gik-cli` |

### Running Tests

```bash
# Fast unit tests
cargo test -p gik-core

# All tests (requires models)
cargo test

# Specific test
cargo test -p gik-cli -- integration_flow
```

## Commit Messages

Follow [Conventional Commits](https://www.conventionalcommits.org/):

```
<type>(<scope>): <description>

[optional body]

[optional footer]
```

Types:
- `feat` – New feature
- `fix` – Bug fix
- `docs` – Documentation only
- `refactor` – Code change that neither fixes a bug nor adds a feature
- `test` – Adding missing tests
- `chore` – Maintenance tasks

Examples:
```
feat(ask): add knowledge graph context to queries
fix(commit): handle empty staging directory
docs: update installation instructions
```

## Building Distribution Packages

### Overview

GIK supports multiple platforms with platform-specific build strategies:

- **Linux x86_64**: Docker-based builds (consistent dependencies)
- **Windows x86_64**: Docker-based MinGW cross-compilation
- **macOS ARM64 (Apple Silicon)**: Native cargo builds
- **macOS x86_64 (Intel)**: Requires Intel machine or GitHub Actions

### Build Commands

```bash
# Linux/Mac
./scripts/gik-build-packages.sh          # Build for current platform
TARGET=linux-x86_64 ./scripts/gik-build-packages.sh
TARGET=macos-aarch64 ./scripts/gik-build-packages.sh

# Windows (PowerShell)
.\scripts\gik-build-packages.ps1         # Build Windows x86_64
```

### Package Contents

Each package includes:

```
gik-<target>/
├── bin/
│   └── gik                    # Binary executable
├── models/
│   ├── embeddings/           # Embedding models
│   │   └── all-MiniLM-L6-v2/
│   └── rerankers/            # Reranker models
│       └── ms-marco-MiniLM-L6-v2/
├── config.default.yaml       # Default configuration
├── LICENSE
└── README.md
```

Output packages are created in `dist/`:
```
dist/
├── gik-linux-x86_64.tar.gz
├── gik-windows-x86_64.zip
├── gik-macos-aarch64.tar.gz
└── gik-macos-x86_64.tar.gz
```

### macOS x86_64 (Intel) Builds

Cross-compilation from ARM64 to x86_64 on macOS fails due to linker issues. Use one of:

1. **GitHub Actions** (Recommended): Workflow runs on Intel runners automatically
2. **Build on Intel Mac**: Clone repo and run build script
3. **Remote Build Services**: CircleCI, MacStadium, etc.

### Verifying Builds

```bash
# Check binary type
file dist/gik-linux-x86_64/bin/gik
# Expected: ELF 64-bit LSB pie executable, x86-64

# Test execution
./dist/gik-linux-x86_64/bin/gik --version

# Calculate SHA256 for release notes
shasum -a 256 dist/*.tar.gz dist/*.zip
```

## Release Process

For detailed instructions on creating and publishing releases, see **[RELEASE.md](RELEASE.md)**.

Quick summary:
1. Update version in `Cargo.toml` and `crates/gik-cli/resources/windows/gik.rc`
2. Update `CHANGELOG.md`
3. Commit, tag, and push
4. Build artifacts using `scripts/gik-build-packages.ps1` (Windows) or `scripts/gik-build-packages.sh` (Linux/macOS)
5. Create GitHub Release with artifacts

## Getting Help

- Open an issue for bugs or feature requests
- Check existing issues and discussions
- Review the [architecture docs](docs/5-ARCH.md)

## License

By contributing, you agree that your contributions will be licensed under the [MIT License](LICENSE).
