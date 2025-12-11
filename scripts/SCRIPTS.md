# GIK Packaging Scripts Usage Guide

This document describes how to use the GIK packaging scripts to create distributable artifacts.

## 🏗️ Build Architecture

**IMPORTANT**: GIK uses **Docker-based builds** to ensure consistent environments across all platforms.

### Why Docker?

GIK has complex native dependencies that are difficult to setup manually:

- **Protocol Buffers** (`protoc`) - Required by `lance-encoding` crate
- **CMake** - Required by cryptographic and compression libraries
- **Build toolchains** - GCC/MinGW-w64 with proper configurations
- **System libraries** - OpenSSL, protobuf-dev, etc.

**Direct `cargo build` will fail** on most systems due to missing dependencies.

### Build Script Hierarchy

```
scripts/build.ps1 / scripts/build.sh          ← Core build scripts (use Docker)
    ↓ used by
scripts/gik-build-packages.ps1 / .sh  ← Packaging scripts (orchestrate + bundle)
```

**Never call `cargo build` directly** - always use `scripts/build.ps1` or `scripts/build.sh`.

---

## Overview

GIK provides packaging and installation scripts:

### Packaging Scripts

| Script | Platform | Output | Build Method |
|--------|----------|--------|--------------|
| `scripts/gik-build-packages.sh` | Linux, macOS | `.tar.gz` | Via `scripts/build.sh` (Docker) |
| `scripts/gik-build-packages.ps1` | Windows | `.zip` | Via `scripts/build.ps1` (Docker) |

Both scripts produce self-contained artifacts with:
- GIK binary (`bin/gik` or `bin/gik.exe`)
- Default ML models (`models/`)
- Configuration template (`config.default.yaml`)
- Documentation (`LICENSE`, `README.md`)

### Installation Scripts

| Script | Purpose | Usage |
|--------|---------|-------|
| `scripts/windows-install.ps1` | Install from GitHub releases | Download & run |
| `scripts/install.sh` | Install from GitHub releases (Linux/macOS) | Download & run |
| `scripts/test-install.ps1` | Test installation locally (Windows) | Development/testing |

---

## Prerequisites

### Docker

**Required**: Docker must be installed and running.

```powershell
# Windows - Check Docker
docker --version

# Linux/macOS - Check Docker
docker --version
```

If Docker is not installed, follow: https://docs.docker.com/get-docker/

### Models

Before packaging, ensure models are downloaded:

```bash
./scripts/gik-download-models.sh
```

This downloads the required models to `vendor/models/`:
- `embeddings/all-MiniLM-L6-v2/`
- `rerankers/ms-marco-MiniLM-L6-v2/`

---

## Bash Script (Linux/macOS)

### Basic Usage

```bash
# Build for current platform (auto-detected)
./scripts/gik-build-packages.sh

# Show help
./scripts/gik-build-packages.sh --help
```

### Parameters

All parameters are provided via environment variables:

| Variable | Default | Description |
|----------|---------|-------------|
| `TARGET` | Auto-detected | Logical target: `linux-x86_64`, `macos-x86_64`, `macos-aarch64` |
| `DIST_DIR` | `./dist` | Output directory for artifacts |
| `MODELS_SOURCE` | `./vendor/models` | Source directory for models |
| `CONFIG_SOURCE` | `./config.default.yaml` | Path to config template |
| `GIK_VERSION` | From `Cargo.toml` | Version string for artifact naming |

### Examples

```bash
# Build for specific target
TARGET=macos-x86_64 ./scripts/gik-build-packages.sh

# Custom output directory
DIST_DIR=/tmp/gik-release ./scripts/gik-build-packages.sh

# Custom models location
MODELS_SOURCE=~/.gik/models ./scripts/gik-build-packages.sh

# Set explicit version
GIK_VERSION=1.0.0 ./scripts/gik-build-packages.sh
```

### Target Mapping

| Logical Target | Cargo Triple | Output |
|----------------|--------------|--------|
| `linux-x86_64` | `x86_64-unknown-linux-gnu` | `gik-linux-x86_64.tar.gz` |
| `macos-x86_64` | `x86_64-apple-darwin` | `gik-macos-x86_64.tar.gz` |
| `macos-aarch64` | `aarch64-apple-darwin` | `gik-macos-aarch64.tar.gz` |

---

## PowerShell Script (Windows)

### Basic Usage

```powershell
# Build for Windows x86_64 (default)
.\scripts\gik-build-packages.ps1

# Show help
.\scripts\gik-build-packages.ps1 -Help
```

### Parameters

| Parameter | Default | Description |
|-----------|---------|-------------|
| `-Target` | `windows-x86_64` | Logical target name |
| `-Version` | From `Cargo.toml` | Version string |
| `-Help` | - | Show help message |

Environment variables:

| Variable | Default | Description |
|----------|---------|-------------|
| `GIK_DIST_DIR` | `.\dist` | Output directory |
| `GIK_MODELS_SOURCE` | `.\vendor\models` | Models source directory |
| `GIK_CONFIG_SOURCE` | `.\config.default.yaml` | Config template path |

### Examples

```powershell
# Build with explicit version
.\scripts\gik-build-packages.ps1 -Version "1.0.0"

# Custom output directory
$env:GIK_DIST_DIR = "C:\releases\gik"
.\scripts\gik-build-packages.ps1

# Custom models location
$env:GIK_MODELS_SOURCE = "$env:USERPROFILE\.gik\models"
.\scripts\gik-build-packages.ps1
```

### Target Mapping

| Logical Target | Cargo Triple | Output |
|----------------|--------------|--------|
| `windows-x86_64` | `x86_64-pc-windows-msvc` | `gik-windows-x86_64.zip` |

---

## Artifact Structure

Both scripts produce the same internal structure:

```
gik-<os>-<arch>/
├── bin/
│   └── gik (or gik.exe on Windows)
├── models/
│   ├── embeddings/
│   │   └── all-MiniLM-L6-v2/
│   │       ├── config.json
│   │       ├── model.safetensors
│   │       └── tokenizer.json
│   └── rerankers/
│       └── ms-marco-MiniLM-L6-v2/
│           ├── config.json
│           ├── model.safetensors
│           └── tokenizer.json
├── config.default.yaml
├── LICENSE
└── README.md
```

---

## Testing Installation

### Test Local Artifact (Windows)

After building the package, test the installation process locally:

```powershell
# Build package first
.\scripts\gik-build-packages.ps1

# Test installation
.\scripts\test-install.ps1
```

The test script will:
1. ✅ Extract the artifact to a temp directory
2. ✅ Validate package structure
3. ✅ Install binary to a test location
4. ✅ Install models to test GIK home
5. ✅ Install configuration
6. ✅ Verify all files exist
7. ✅ Execute `gik --version` to test binary
8. ✅ Clean up test files

**Example output:**

```
==========================================
  GIK Installation Test
==========================================

==> Extracting archive
✓ Extracted to C:\Users\...\Temp\gik-install-test-84295208\extracted

==> Validating package contents
✓ Binary found: bin\gik.exe
✓ Models directory found
✓ Config file found

==> Installing GIK binary to ...
✓ Installed gik.exe

==> Installing models ...
✓ Models installed (2 .safetensors files)

==> Installing configuration
✓ Created config.yaml

==> Verifying installation
✓ Binary exists
✓ Embedding model found
✓ Reranker model found
✓ Config file found

==> Testing binary execution
✓ Binary executes successfully
  Version output: gik 0.1.0 (8e3a73b)

==========================================
  Installation Test PASSED!
==========================================
```

### Custom Test Parameters

```powershell
# Use specific artifact
.\scripts\test-install.ps1 -ArtifactPath "C:\custom\path\gik.zip"

# Keep test files for inspection (comment out cleanup in script)
# Edit test-install.ps1: comment out the Remove-Item in finally block
```

---

## CI/CD Integration

### GitHub Actions - Linux

```yaml
jobs:
  build-linux:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
        with:
          lfs: true
      
      - name: Setup Rust
        uses: dtolnay/rust-action@stable
      
      - name: Build Package
        run: |
          chmod +x ./scripts/gik-build-packages.sh
          ./scripts/gik-build-packages.sh
      
      - name: Upload Artifact
        uses: actions/upload-artifact@v4
        with:
          name: gik-linux-x86_64
          path: dist/gik-linux-x86_64.tar.gz
```

### GitHub Actions - macOS

```yaml
jobs:
  build-macos:
    runs-on: macos-latest
    steps:
      - uses: actions/checkout@v4
        with:
          lfs: true
      
      - name: Setup Rust
        uses: dtolnay/rust-action@stable
      
      - name: Build Package (ARM64)
        run: |
          chmod +x ./scripts/gik-build-packages.sh
          ./scripts/gik-build-packages.sh
      
      - name: Build Package (x86_64)
        run: |
          rustup target add x86_64-apple-darwin
          TARGET=macos-x86_64 ./scripts/gik-build-packages.sh
      
      - name: Upload Artifacts
        uses: actions/upload-artifact@v4
        with:
          name: gik-macos
          path: dist/gik-macos-*.tar.gz
```

### GitHub Actions - Windows

```yaml
jobs:
  build-windows:
    runs-on: windows-latest
    steps:
      - uses: actions/checkout@v4
        with:
          lfs: true
      
      - name: Setup Rust
        uses: dtolnay/rust-action@stable
      
      - name: Build Package
        run: .\scripts\gik-build-packages.ps1
      
      - name: Upload Artifact
        uses: actions/upload-artifact@v4
        with:
          name: gik-windows-x86_64
          path: dist/gik-windows-x86_64.zip
```

### Complete Release Workflow

```yaml
name: Release

on:
  push:
    tags:
      - 'v*'

jobs:
  build:
    strategy:
      matrix:
        include:
          - os: ubuntu-latest
            target: linux-x86_64
            script: ./scripts/gik-build-packages.sh
            artifact: gik-linux-x86_64.tar.gz
          - os: macos-latest
            target: macos-aarch64
            script: ./scripts/gik-build-packages.sh
            artifact: gik-macos-aarch64.tar.gz
          - os: windows-latest
            target: windows-x86_64
            script: .\scripts\gik-build-packages.ps1
            artifact: gik-windows-x86_64.zip
    
    runs-on: ${{ matrix.os }}
    
    steps:
      - uses: actions/checkout@v4
        with:
          lfs: true
      
      - uses: dtolnay/rust-action@stable
      
      - name: Build
        run: ${{ matrix.script }}
        env:
          GIK_VERSION: ${{ github.ref_name }}
      
      - name: Upload
        uses: actions/upload-artifact@v4
        with:
          name: ${{ matrix.target }}
          path: dist/${{ matrix.artifact }}

  release:
    needs: build
    runs-on: ubuntu-latest
    steps:
      - uses: actions/download-artifact@v4
      
      - name: Create Release
        uses: softprops/action-gh-release@v1
        with:
          files: |
            linux-x86_64/gik-linux-x86_64.tar.gz
            macos-aarch64/gik-macos-aarch64.tar.gz
            windows-x86_64/gik-windows-x86_64.zip
```

---

## Installation from Artifact

### Linux/macOS

```bash
# Extract
tar -xzf gik-macos-aarch64.tar.gz

# Install to ~/.gik
mkdir -p ~/.gik
cp -r gik-macos-aarch64/models ~/.gik/
cp gik-macos-aarch64/config.default.yaml ~/.gik/config.yaml

# Add binary to PATH
sudo cp gik-macos-aarch64/bin/gik /usr/local/bin/
# OR
cp gik-macos-aarch64/bin/gik ~/.local/bin/
```

### Windows

```powershell
# Extract
Expand-Archive -Path gik-windows-x86_64.zip -DestinationPath .

# Install to %USERPROFILE%\.gik
New-Item -ItemType Directory -Force -Path "$env:USERPROFILE\.gik"
Copy-Item -Recurse gik-windows-x86_64\models "$env:USERPROFILE\.gik\"
Copy-Item gik-windows-x86_64\config.default.yaml "$env:USERPROFILE\.gik\config.yaml"

# Add binary to PATH (add to user PATH or copy to a directory in PATH)
Copy-Item gik-windows-x86_64\bin\gik.exe "$env:USERPROFILE\.cargo\bin\"
```

---

## Troubleshooting

### ❌ Protobuf Errors (lance-encoding)

```
error: failed to run custom build command for `lance-encoding v0.39.0`
protoc failed: google/protobuf/empty.proto: File not found.
```

**Cause**: Direct `cargo build` was attempted without Docker. GIK requires:
- Protocol Buffers compiler (`protoc`)
- System protobuf headers (`libprotobuf-dev`)
- CMake and other native dependencies

**Solution**: 

The packaging scripts have been updated to **always use Docker via `scripts/build.ps1`/`scripts/build.sh`**.

If you modified the scripts to use direct `cargo build`, revert those changes:

```powershell
# Windows - Correct way
.\scripts\gik-build-packages.ps1

# Linux/macOS - Correct way  
./scripts/gik-build-packages.sh
```

These scripts internally call `scripts/build.ps1`/`scripts/build.sh` which handle all Docker orchestration.

**Manual Build Alternative**:

If you need to build manually:

```powershell
# Windows
.\scripts\build.ps1 release-windows    # Creates target/gik.exe

# Linux
./scripts/build.sh release             # Creates target/gik
```

### Models Not Found

```
✗ ERROR: Models directory not found: ./vendor/models
```

**Solution**: Run `./scripts/gik-download-models.sh` first.

### Build Fails with Target Not Found

```
error: target 'x86_64-apple-darwin' not found
```

**Solution**: Install the target with `rustup target add x86_64-apple-darwin`.

### Docker Not Running

```
Cannot connect to the Docker daemon at unix:///var/run/docker.sock
```

**Solution**: 
- Windows: Start Docker Desktop
- Linux: `sudo systemctl start docker`
- macOS: Start Docker Desktop

### LFS Files Not Downloaded

If `model.safetensors` files are tiny (~130 bytes), Git LFS pointers weren't resolved.

**Solution**: Run `git lfs pull` or clone with `git clone --lfs`.
