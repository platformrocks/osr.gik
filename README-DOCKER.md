# GIK CLI - Docker Build System

This document describes the Docker-based build system for GIK CLI, which provides
consistent, reproducible builds across all platforms.

## Table of Contents

- [Overview](#overview)
- [Prerequisites](#prerequisites)
- [Quick Start](#quick-start)
- [Build Commands](#build-commands)
- [Dockerfiles](#dockerfiles)
- [Build Architecture](#build-architecture)
- [Performance Optimization](#performance-optimization)
- [Troubleshooting](#troubleshooting)

## Overview

The GIK build system uses Docker to:
- Ensure consistent build environments across platforms
- Avoid complex native dependency management (protobuf, OpenSSL, etc.)
- Enable cross-compilation (e.g., build Windows binaries from Linux)
- Maximize cache efficiency with layered builds

### Build Targets

| Target | Dockerfile | Output | Use Case |
|--------|------------|--------|----------|
| Linux x86_64 | `Dockerfile` | `target/gik` | Linux servers, containers |
| Windows x86_64 | `Dockerfile.windows` | `target/gik.exe` | Windows workstations |
| Linux + CUDA | `Dockerfile.cuda` | `target/gik-cuda` | GPU-accelerated inference |

## Prerequisites

- **Docker Desktop** (Windows/Mac) or **Docker Engine** (Linux)
- **Docker Compose** v2+ (included with Docker Desktop)
- ~10GB disk space for Docker images and caches

## Quick Start

### Windows (PowerShell)

```powershell
# Build and install to ~/.cargo/bin
.\build.ps1 install

# Verify installation
gik --version

# Build Linux binary (for containers/servers)
.\build.ps1 release

# Start development shell
.\build.ps1 dev
```

### Linux/Mac (Bash)

```bash
# Make script executable
chmod +x build.sh

# Build and install to ~/.cargo/bin
./build.sh install

# Verify installation
gik --version

# Build Linux binary
./build.sh release

# Start development shell
./build.sh dev
```

## Build Commands

Both `build.ps1` (PowerShell) and `build.sh` (Bash) support identical commands:

### Production Builds

| Command | Description |
|---------|-------------|
| `release` | Build Linux x86_64 release binary (default) |
| `release-windows` | Build Windows x86_64 binary (cross-compilation) |
| `release-cuda` | Build Linux x86_64 with CUDA GPU support |
| `install` | Build and install to `~/.cargo/bin` |

### Development

| Command | Description |
|---------|-------------|
| `dev` | Start interactive development shell |
| `test` | Run all tests (unit + integration) |
| `test-unit` | Run unit tests only (faster) |
| `test-integration` | Run integration tests only |
| `fmt` | Format code with rustfmt |
| `clippy` | Run clippy linter |

### Maintenance

| Command | Description |
|---------|-------------|
| `clean` | Remove all artifacts, images, and caches |
| `help` | Show help message |

### Options

| Option | Description |
|--------|-------------|
| `-NoBuildCache` / `--no-cache` | Disable Docker cache (fresh build) |
| `-CudaArch N` / `--cuda-arch N` | CUDA compute capability |

### Examples

```powershell
# Fresh build without cache
.\build.ps1 release -NoBuildCache

# CUDA build for RTX 40xx (Ada Lovelace)
.\build.ps1 release-cuda -CudaArch 89

# Run unit tests only (faster CI)
.\build.ps1 test-unit
```

## Dockerfiles

### `Dockerfile` - Linux Production Build

Multi-stage build optimized for:
- **Small runtime image**: Uses `debian:bullseye-slim` (~50MB base)
- **Layer caching**: Dependencies cached separately from source
- **Security**: Non-root runtime, minimal attack surface

```
Stage 1: base      → Rust toolchain + system deps
Stage 2: deps      → Build dependencies only (cached)
Stage 3: builder   → Compile full source
Stage 4: runtime   → Minimal image + binary
```

### `Dockerfile.windows` - Windows Cross-Compilation

Cross-compiles from Linux to Windows using:
- **MinGW-w64**: GCC toolchain for Windows targets
- **Target**: `x86_64-pc-windows-gnu`
- **Avoids**: MSVC, Windows SDK, native build issues

### `Dockerfile.cuda` - GPU-Accelerated Build

CUDA-enabled build for GPU inference:
- **Base**: `nvidia/cuda:12.4.0-devel-ubuntu22.04`
- **Runtime**: CUDA 12.4 runtime libraries
- **Feature**: `--features cuda` enabled

**CUDA Compute Capabilities:**

| Cap | Architecture | GPUs |
|-----|--------------|------|
| 70 | Volta | V100 |
| 75 | Turing | RTX 20xx, T4 |
| 80 | Ampere | A100 |
| 86 | Ampere | RTX 30xx, A10 |
| 89 | Ada Lovelace | RTX 40xx, L4 |
| 90 | Hopper | H100 |

### `Dockerfile.dev` - Development Environment

Full development toolchain:
- rustfmt, clippy, rust-analyzer
- cargo-watch, cargo-expand, cargo-audit
- gdb, vim, git

## Build Architecture

### Multi-Stage Build

Each Dockerfile uses a multi-stage approach:

```
┌─────────────────────────────────────────────────────────────┐
│ Stage 1: Base                                               │
│ - Rust nightly toolchain                                    │
│ - System dependencies (cmake, protobuf, openssl)            │
│ Cached: Until Rust version or system deps change            │
├─────────────────────────────────────────────────────────────┤
│ Stage 2: Builder                                            │
│ - Full workspace copied                                     │
│ - cargo build --release                                     │
│ Rebuild: On any source file change                          │
├─────────────────────────────────────────────────────────────┤
│ Stage 3: Runtime (Linux only)                               │
│ - Minimal debian:bookworm-slim image                        │
│ - Only runtime libs (ca-certs, openssl)                     │
│ - Binary copied from builder                                │
│ Size: ~100MB total                                          │
└─────────────────────────────────────────────────────────────┘
```

Note: For Cargo workspaces with internal crate dependencies (like GIK), 
we build all crates together rather than caching external dependencies 
separately, as workspace crates reference each other and must be 
compiled together.

### Why This Matters

GIK has heavy dependencies that take a long time to compile:

| Crate | Compile Time | Notes |
|-------|--------------|-------|
| `lancedb` | ~3-5 min | Vector database, Arrow/Parquet |
| `candle-*` | ~2-3 min | ML framework |
| `arrow-*` | ~1-2 min | Columnar data format |
| `tokenizers` | ~1-2 min | NLP tokenization |

With layer caching:
- **First build**: ~10-15 minutes (all layers)
- **Code change**: ~1-2 minutes (only Layer 3)
- **Cargo.toml change**: ~5-7 minutes (Layers 2-3)

## Performance Optimization

### Cargo Profiles

The workspace `Cargo.toml` includes optimized profiles:

```toml
[profile.dev]
opt-level = 0
debug = 1  # Line tables only (faster)
incremental = true

[profile.release]
opt-level = 3
lto = "thin"  # Fast LTO
codegen-units = 1
strip = true
panic = "abort"

# Optimize heavy deps even in dev
[profile.dev.package.lancedb]
opt-level = 2
```

### Docker BuildKit

Docker BuildKit is enabled by default for:
- Parallel layer building
- Better cache management
- Improved build output

### Volume Caching

`docker-compose.yml` defines persistent volumes:

```yaml
volumes:
  cargo-cache:   # ~/.cargo/registry (~2GB)
  target-cache:  # target/ directory (~5GB)
```

This speeds up `docker-compose` based builds significantly.

## Troubleshooting

### Build Fails with Protobuf Error

```
google/protobuf/empty.proto: File not found
```

**Solution**: The Dockerfiles include `protobuf-compiler`. If building locally,
install protobuf:

```bash
# Ubuntu/Debian
sudo apt install protobuf-compiler

# macOS
brew install protobuf

# Windows (use Docker instead)
.\build.ps1 release-windows
```

### Windows Binary Won't Run

```
The specified executable is not a valid application
```

**Cause**: You extracted a Linux binary to Windows.

**Solution**: Use `release-windows` to build Windows binary:

```powershell
.\build.ps1 release-windows
# Creates target/gik.exe
```

### CUDA Build Fails

```
nvcc not found / CUDA_HOME not set
```

**Solution**: Use the CUDA Dockerfile, not local build:

```bash
./build.sh release-cuda
```

### Slow Rebuilds

**Symptom**: Full rebuild on every code change.

**Check**:
1. Docker BuildKit enabled: `DOCKER_BUILDKIT=1`
2. Not using `--no-cache` unnecessarily
3. Cargo.lock is committed (consistent deps)

### Out of Disk Space

```
no space left on device
```

**Solution**: Clean Docker artifacts:

```powershell
.\build.ps1 clean
docker system prune -a
```

## Advanced Usage

### Building Specific Crates

```bash
# In development shell
docker-compose run --rm gik-dev cargo build -p gik-core
docker-compose run --rm gik-dev cargo test -p gik-cli
```

### Extracting Binaries Manually

```bash
# Build image
docker build -t gik-cli:latest -f Dockerfile .

# Create temporary container
container_id=$(docker create gik-cli:latest)

# Copy binary
docker cp "${container_id}:/usr/local/bin/gik" ./gik

# Cleanup
docker rm "${container_id}"
```

### Running CUDA Container

```bash
# Build CUDA image
./build.sh release-cuda

# Run with GPU access
docker run --gpus all \
  -v "$PWD:/workspace" \
  -w /workspace \
  gik-cli-cuda:latest \
  ask "What is this project about?"
```

### CI/CD Integration

```yaml
# GitHub Actions example
jobs:
  build:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - name: Build Linux binary
        run: ./build.sh release
      - name: Build Windows binary
        run: ./build.sh release-windows
      - name: Upload artifacts
        uses: actions/upload-artifact@v4
        with:
          name: binaries
          path: |
            target/gik
            target/gik.exe
```
