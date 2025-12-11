#!/usr/bin/env bash
# =============================================================================
# GIK CLI - Artifact Packaging Script (Bash)
# =============================================================================
#
# IMPORTANT: This script uses Docker-based builds via build.sh
#
# WHY DOCKER?
#   GIK has complex native dependencies (protobuf, cmake, build toolchains) that
#   are difficult to setup manually. Direct `cargo build` will fail with:
#     "error: failed to run custom build command for `lance-encoding`"
#     "protoc failed: google/protobuf/empty.proto: File not found"
#
#   The build.sh script handles all Docker orchestration with proper toolchains.
#
# This script creates distributable artifacts for GIK CLI including:
#   - The GIK binary for the target platform (built via Docker)
#   - Default embedding and reranker models
#   - Default configuration file
#   - LICENSE and README.md
#
# USAGE:
#   ./scripts/gik-build-packages.sh [OPTIONS]
#
# OPTIONS:
#   -h, --help      Show this help message
#
# ENVIRONMENT VARIABLES:
#   TARGET          Target platform (linux-x86_64, macos-x86_64, macos-aarch64)
#                   Default: auto-detected from current system
#   GIK_VERSION     Version string to include in artifact naming
#                   Default: extracted from Cargo.toml or "dev"
#   DIST_DIR        Output directory for artifacts
#                   Default: ./dist
#   MODELS_SOURCE   Path to models directory to bundle
#                   Default: ./vendor/models
#   CONFIG_SOURCE   Path to config template file
#                   Default: ./config.default.yaml
#
# EXAMPLES:
#   # Build for current platform
#   ./scripts/gik-build-packages.sh
#
#   # Build for specific target
#   TARGET=macos-aarch64 ./scripts/gik-build-packages.sh
#
#   # Override model source
#   MODELS_SOURCE=~/my-models ./scripts/gik-build-packages.sh
#
# =============================================================================

set -euo pipefail

# =============================================================================
# CONFIGURATION
# =============================================================================

# Script directory (for relative paths)
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"

# Default values (can be overridden by environment variables)
DIST_DIR="${DIST_DIR:-$REPO_ROOT/dist}"
MODELS_SOURCE="${MODELS_SOURCE:-$REPO_ROOT/vendor/models}"
CONFIG_SOURCE="${CONFIG_SOURCE:-$REPO_ROOT/config.default.yaml}"

# =============================================================================
# UTILITY FUNCTIONS
# =============================================================================

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[0;33m'
CYAN='\033[0;36m'
GRAY='\033[0;90m'
NC='\033[0m' # No Color

step() {
    echo -e "${CYAN}==>${NC} $1" >&2
}

success() {
    echo -e "${GREEN}✓${NC} $1" >&2
}

warning() {
    echo -e "${YELLOW}⚠${NC} $1" >&2
}

info() {
    echo -e "${GRAY}  $1${NC}" >&2
}

error() {
    echo -e "${RED}✗ ERROR: $1${NC}" >&2
    exit 1
}

show_help() {
    head -50 "$0" | grep -E '^#' | sed 's/^# //' | sed 's/^#//'
    exit 0
}

# =============================================================================
# DETECTION FUNCTIONS
# =============================================================================

detect_os() {
    local os
    os="$(uname -s)"
    case "$os" in
        Linux)  echo "linux" ;;
        Darwin) echo "macos" ;;
        *)      error "Unsupported OS: $os" ;;
    esac
}

detect_arch() {
    local arch
    arch="$(uname -m)"
    case "$arch" in
        x86_64)         echo "x86_64" ;;
        aarch64|arm64)  echo "aarch64" ;;
        *)              error "Unsupported architecture: $arch" ;;
    esac
}

detect_target() {
    local os arch
    os="$(detect_os)"
    arch="$(detect_arch)"
    echo "${os}-${arch}"
}

get_cargo_triple() {
    local target="$1"
    case "$target" in
        linux-x86_64)   echo "x86_64-unknown-linux-gnu" ;;
        macos-x86_64)   echo "x86_64-apple-darwin" ;;
        macos-aarch64)  echo "aarch64-apple-darwin" ;;
        *)              error "Unknown target: $target. Supported: linux-x86_64, macos-x86_64, macos-aarch64" ;;
    esac
}

get_version() {
    if [[ -n "${GIK_VERSION:-}" ]]; then
        echo "$GIK_VERSION"
        return
    fi
    # Try to extract from Cargo.toml
    if [[ -f "$REPO_ROOT/Cargo.toml" ]]; then
        local version
        version=$(grep -m1 '^version' "$REPO_ROOT/Cargo.toml" | sed 's/.*= *"\([^"]*\)".*/\1/' || true)
        if [[ -n "$version" ]]; then
            echo "$version"
            return
        fi
    fi
    echo "dev"
}

# =============================================================================
# BUILD FUNCTIONS
# =============================================================================

build_binary() {
    local target="$1"
    local cargo_triple="$2"
    
    step "Building GIK binary for $target ($cargo_triple)"
    
    cd "$REPO_ROOT"
    
    # Determine build method based on target
    case "$target" in
        linux-*)
            # Linux builds use Docker for consistent environment with all dependencies
            info "Using Docker build via build.sh for consistent environment"
            
            local build_script="$REPO_ROOT/scripts/build.sh"
            if [[ ! -f "$build_script" ]]; then
                error "Build script not found: $build_script"
            fi
            
            # Call build.sh to create the binary using Docker
            info "Running: ./build.sh release"
            "$build_script" release >&2
            
            if [[ $? -ne 0 ]]; then
                error "Docker build failed. Ensure Docker is running and build.sh is accessible."
            fi
            
            # The build.sh script outputs to target/gik
            local binary_path="$REPO_ROOT/target/gik"
            if [[ ! -f "$binary_path" ]]; then
                error "Build failed: binary not found at $binary_path"
            fi
            
            success "Binary built via Docker: $binary_path"
            echo "$binary_path"
            ;;
            
        macos-*)
            # macOS builds use native cargo (cross-compilation not well supported in Docker)
            info "Using native cargo build for macOS"
            
            # Check if cargo is available
            if ! command -v cargo &> /dev/null; then
                error "cargo not found. Please install Rust: https://rustup.rs/"
            fi
            
            # Check if target is installed
            if ! rustup target list --installed | grep -q "$cargo_triple"; then
                info "Installing Rust target: $cargo_triple"
                rustup target add "$cargo_triple" >&2
            fi
            
            # Build for the specific target
            info "Running: cargo build --release --target $cargo_triple -p gik-cli"
            if ! cargo build --release --target "$cargo_triple" -p gik-cli >&2; then
                error "Cargo build failed for target $cargo_triple"
            fi
            
            # Output path for macOS builds
            local binary_path="$REPO_ROOT/target/$cargo_triple/release/gik"
            if [[ ! -f "$binary_path" ]]; then
                error "Build failed: binary not found at $binary_path"
            fi
            
            success "Binary built natively: $binary_path"
            echo "$binary_path"
            ;;
            
        *)
            error "Unsupported target: $target"
            ;;
    esac
}

# =============================================================================
# STAGING FUNCTIONS
# =============================================================================

create_staging_dir() {
    local target="$1"
    local staging_dir="$DIST_DIR/gik-$target"
    
    step "Creating staging directory: $staging_dir"
    
    # Clean existing staging directory for this target
    if [[ -d "$staging_dir" ]]; then
        info "Cleaning existing staging directory"
        rm -rf "$staging_dir"
    fi
    
    mkdir -p "$staging_dir/bin"
    mkdir -p "$staging_dir/models"
    
    echo "$staging_dir"
}

copy_binary() {
    local binary_path="$1"
    local staging_dir="$2"
    
    step "Copying binary"
    cp "$binary_path" "$staging_dir/bin/gik"
    chmod +x "$staging_dir/bin/gik"
    success "Binary copied to bin/gik"
}

copy_models() {
    local staging_dir="$1"
    
    step "Copying models from $MODELS_SOURCE"
    
    if [[ ! -d "$MODELS_SOURCE" ]]; then
        error "Models directory not found: $MODELS_SOURCE"
    fi
    
    # Check for actual model files (not just .gitkeep)
    local has_models=false
    if [[ -d "$MODELS_SOURCE/embeddings/all-MiniLM-L6-v2" ]]; then
        if find "$MODELS_SOURCE/embeddings/all-MiniLM-L6-v2" -name "*.safetensors" -o -name "*.json" 2>/dev/null | grep -q .; then
            has_models=true
        fi
    fi
    
    if [[ "$has_models" == "false" ]]; then
        warning "Models directory exists but appears empty (only .gitkeep files)"
        warning "Ensure models are downloaded before creating distribution artifacts"
        info "Expected: embeddings/all-MiniLM-L6-v2/ and rerankers/ms-marco-MiniLM-L6-v2/"
    fi
    
    # Copy models directory structure
    cp -R "$MODELS_SOURCE"/* "$staging_dir/models/" 2>/dev/null || true
    
    success "Models copied to models/"
}

copy_config() {
    local staging_dir="$1"
    
    step "Copying config template"
    
    if [[ ! -f "$CONFIG_SOURCE" ]]; then
        error "Config file not found: $CONFIG_SOURCE"
    fi
    
    cp "$CONFIG_SOURCE" "$staging_dir/config.default.yaml"
    success "Config copied as config.default.yaml"
}

copy_docs() {
    local staging_dir="$1"
    
    step "Copying documentation"
    
    if [[ -f "$REPO_ROOT/LICENSE" ]]; then
        cp "$REPO_ROOT/LICENSE" "$staging_dir/"
        success "LICENSE copied"
    else
        warning "LICENSE not found at $REPO_ROOT/LICENSE"
    fi
    
    if [[ -f "$REPO_ROOT/README.md" ]]; then
        cp "$REPO_ROOT/README.md" "$staging_dir/"
        success "README.md copied"
    else
        warning "README.md not found at $REPO_ROOT/README.md"
    fi
}

# =============================================================================
# VALIDATION FUNCTIONS
# =============================================================================

validate_staging() {
    local staging_dir="$1"
    
    step "Validating staging directory"
    
    local errors=0
    
    # Check binary exists and is executable
    if [[ ! -x "$staging_dir/bin/gik" ]]; then
        error "Binary missing or not executable: $staging_dir/bin/gik"
        ((errors++))
    else
        success "Binary is executable"
    fi
    
    # Check models directory exists
    if [[ ! -d "$staging_dir/models" ]]; then
        error "Models directory missing: $staging_dir/models"
        ((errors++))
    else
        success "Models directory exists"
    fi
    
    # Check config exists
    if [[ ! -f "$staging_dir/config.default.yaml" ]]; then
        error "Config file missing: $staging_dir/config.default.yaml"
        ((errors++))
    else
        success "Config file exists"
    fi
    
    if [[ $errors -gt 0 ]]; then
        error "Validation failed with $errors error(s)"
    fi
    
    success "Staging directory validated"
}

# =============================================================================
# ARCHIVE FUNCTIONS
# =============================================================================

create_archive() {
    local target="$1"
    local staging_dir="$2"
    
    local archive_name="gik-$target.tar.gz"
    local archive_path="$DIST_DIR/$archive_name"
    
    step "Creating archive: $archive_name"
    
    # Remove existing archive
    rm -f "$archive_path"
    
    # Create tar.gz archive
    tar -czf "$archive_path" -C "$DIST_DIR" "gik-$target"
    
    local size
    size=$(du -h "$archive_path" | cut -f1)
    success "Archive created: $archive_path ($size)"
    
    # List archive contents
    info "Archive contents:"
    tar -tzf "$archive_path" | head -20 | while read -r line; do
        info "  $line"
    done
    
    echo "$archive_path"
}

# =============================================================================
# PREREQUISITE CHECKS
# =============================================================================

check_prerequisites() {
    local target="$1"
    
    step "Checking prerequisites..."
    
    # Docker is only required for Linux builds
    if [[ "$target" == linux-* ]]; then
        # Check if Docker is installed
        if ! command -v docker &> /dev/null; then
            echo ""
            echo "=========================================="
            echo "  DOCKER NOT FOUND"
            echo "=========================================="
            echo ""
            error "Docker is required for Linux builds but not installed or not in PATH.

GIK uses Docker-based builds for Linux to ensure consistent environments with all
required dependencies (protobuf, cmake, build toolchains, etc.).

INSTALLATION:
  Windows: https://docs.docker.com/desktop/install/windows-install/
  Linux:   https://docs.docker.com/engine/install/
  macOS:   https://docs.docker.com/desktop/install/mac-install/

After installing Docker:
  1. Start Docker Desktop (Windows/macOS) or docker service (Linux)
  2. Verify: docker --version
  3. Run this script again

For more information, see: docs/BUILD-ARCHITECTURE.md"
        fi
        
        # Check if Docker is running
        if ! docker version &> /dev/null; then
            echo ""
            echo "=========================================="
            echo "  DOCKER NOT RUNNING"
            echo "=========================================="
            echo ""
            error "Docker is installed but not running.

SOLUTION:
  Windows/macOS: Start Docker Desktop
  Linux:         sudo systemctl start docker

Verify with: docker ps"
        fi
        
        success "Docker is installed and running"
        
        # Check if build.sh exists
        if [[ ! -f "$REPO_ROOT/build.sh" ]]; then
            error "Build script not found: $REPO_ROOT/build.sh"
        fi
        
        success "Build script found: build.sh"
    fi
    
    # Cargo is required for macOS builds
    if [[ "$target" == macos-* ]]; then
        if ! command -v cargo &> /dev/null; then
            error "cargo not found. Please install Rust from https://rustup.rs/"
        fi
        success "Rust toolchain found"
    fi
}

# =============================================================================
# MAIN
# =============================================================================

main() {
    # Parse arguments
    while [[ $# -gt 0 ]]; do
        case $1 in
            -h|--help)
                show_help
                ;;
            *)
                error "Unknown argument: $1. Use -h for help."
                ;;
        esac
    done
    
    # Detect or use provided target first
    local target="${TARGET:-$(detect_target)}"
    
    # Check prerequisites based on target
    check_prerequisites "$target"
    local cargo_triple
    cargo_triple="$(get_cargo_triple "$target")"
    local version
    version="$(get_version)"
    
    echo ""
    echo "=========================================="
    echo "  GIK Artifact Packaging"
    echo "=========================================="
    echo ""
    info "Target:     $target"
    info "Triple:     $cargo_triple"
    info "Version:    $version"
    info "Dist dir:   $DIST_DIR"
    info "Models:     $MODELS_SOURCE"
    info "Config:     $CONFIG_SOURCE"
    
    # Ensure dist directory exists
    mkdir -p "$DIST_DIR"
    
    # Build binary
    local binary_path
    binary_path="$(build_binary "$target" "$cargo_triple")"
    
    # Create staging directory
    local staging_dir
    staging_dir="$(create_staging_dir "$target")"
    
    # Copy files to staging
    copy_binary "$binary_path" "$staging_dir"
    copy_models "$staging_dir"
    copy_config "$staging_dir"
    copy_docs "$staging_dir"
    
    # Validate staging directory
    validate_staging "$staging_dir"
    
    # Create archive
    local archive_path
    archive_path="$(create_archive "$target" "$staging_dir")"
    
    echo ""
    echo "=========================================="
    echo "  Packaging Complete"
    echo "=========================================="
    echo ""
    success "Artifact: $archive_path"
    success "Staging:  $staging_dir"
    echo ""
}

main "$@"
