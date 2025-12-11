#!/usr/bin/env bash
# =============================================================================
# GIK CLI - Build Script (Bash)
# =============================================================================
#
# This script provides a unified interface for building, testing, and
# installing GIK CLI across different platforms and configurations.
#
# ARCHITECTURE:
#   The script uses Docker for all builds to ensure consistent environments
#   and avoid dependency issues on different host systems.
#
# DOCKERFILES:
#   Dockerfile         - Linux x86_64 release build (multi-stage, layered)
#   Dockerfile.windows - Windows x86_64 cross-compilation
#   Dockerfile.cuda    - Linux x86_64 with CUDA/GPU support
#   Dockerfile.dev     - Development environment with full toolchain
#
# LAYER STRATEGY:
#   Each Dockerfile uses a layered approach to maximize cache efficiency:
#   1. Base layer: Toolchain + system deps (rarely changes)
#   2. Deps layer: Cargo dependencies (changes on Cargo.toml)
#   3. Build layer: Source compilation (changes on code changes)
#   4. Runtime layer: Minimal image with binary (production only)
#
# USAGE:
#   ./build.sh [command] [options]
#
# COMMANDS:
#   release         Build Linux x86_64 release binary (default)
#   release-windows Build Windows x86_64 binary (cross-compilation)
#   release-cuda    Build Linux x86_64 with CUDA support
#   install         Build and install to ~/.cargo/bin
#   dev             Start development shell
#   test            Run all tests
#   test-unit       Run unit tests only (fast)
#   test-integration Run integration tests only
#   fmt             Format code with rustfmt
#   clippy          Run clippy linter
#   clean           Clean all build artifacts and caches
#   help            Show this help message
#
# OPTIONS:
#   --no-cache      Disable Docker build cache (fresh build)
#   --cuda-arch N   CUDA compute capability (default: 86 for RTX 30xx)
#
# EXAMPLES:
#   ./build.sh release                     # Linux release build
#   ./build.sh install                     # Build and install Linux binary
#   ./build.sh release-cuda --cuda-arch 89 # CUDA build for RTX 40xx
#   ./build.sh test --no-cache             # Fresh test build
#
# =============================================================================

set -e

# =============================================================================
# PATH DETECTION
# =============================================================================

# Determine repo root (script is now in scripts/ subdirectory)
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
cd "$REPO_ROOT"

# =============================================================================
# CONFIGURATION
# =============================================================================

# Docker image names
IMAGE_LINUX="gik-cli:latest"
IMAGE_WINDOWS="gik-cli-windows:latest"
IMAGE_CUDA="gik-cli-cuda:latest"

# Output paths
OUTPUT_DIR="./target"
BINARY_LINUX="$OUTPUT_DIR/gik"
BINARY_WINDOWS="$OUTPUT_DIR/gik.exe"
BINARY_CUDA="$OUTPUT_DIR/gik-cuda"

# Installation path
INSTALL_DIR="$HOME/.cargo/bin"

# Default options
NO_CACHE=""
CUDA_ARCH="86"

# =============================================================================
# ARGUMENT PARSING
# =============================================================================

COMMAND="${1:-release}"
shift || true

while [[ $# -gt 0 ]]; do
    case $1 in
        --no-cache)
            NO_CACHE="--no-cache"
            shift
            ;;
        --cuda-arch)
            CUDA_ARCH="$2"
            shift 2
            ;;
        *)
            echo "Unknown option: $1"
            exit 1
            ;;
    esac
done

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
    echo -e "\n${CYAN}==>${NC} $1"
}

success() {
    echo -e "${GREEN}$1${NC}"
}

warning() {
    echo -e "${YELLOW}$1${NC}"
}

info() {
    echo -e "${GRAY}$1${NC}"
}

error() {
    echo -e "${RED}ERROR: $1${NC}" >&2
    exit 1
}

check_docker() {
    if ! command -v docker &> /dev/null; then
        error "Docker is not installed. Please install Docker first."
    fi
    if ! docker info &> /dev/null; then
        error "Docker daemon is not running. Please start Docker."
    fi
}

ensure_output_dir() {
    mkdir -p "$OUTPUT_DIR"
}

get_git_hash() {
    git rev-parse --short HEAD 2>/dev/null || echo "unknown"
}

get_binary_size() {
    local path="$1"
    if [[ -f "$path" ]]; then
        local size=$(du -h "$path" | cut -f1)
        echo "$size"
    else
        echo "0"
    fi
}

extract_binary() {
    local image="$1"
    local container_path="$2"
    local host_path="$3"
    
    step "Extracting binary..."
    local container_id=$(docker create "$image")
    docker cp "${container_id}:${container_path}" "$host_path"
    docker rm "$container_id" > /dev/null
    chmod +x "$host_path"
    success "Binary extracted to: $host_path"
}

# =============================================================================
# BUILD FUNCTIONS
# =============================================================================

build_dev_image() {
    step "Building development image..."
    docker-compose build gik-dev
}

build_release() {
    # Build Linux x86_64 release binary using multi-stage Docker build.
    # Uses Dockerfile with layered caching:
    # - Base layer cached until Rust version changes
    # - Deps layer cached until Cargo.toml changes
    # - Build layer recompiled on source changes
    
    step "Building Linux release binary..."
    info "Using Dockerfile with layered caching strategy"
    
    ensure_output_dir
    
    local git_hash=$(get_git_hash)
    docker build $NO_CACHE --platform linux/amd64 --build-arg "GIT_HASH=$git_hash" -t "$IMAGE_LINUX" -f Dockerfile .
    
    extract_binary "$IMAGE_LINUX" "/usr/local/bin/gik" "$BINARY_LINUX"
    
    local size=$(get_binary_size "$BINARY_LINUX")
    success "Linux binary: $BINARY_LINUX ($size)"
}

build_release_windows() {
    # Build Windows x86_64 binary using MinGW cross-compilation.
    # Uses Dockerfile.windows with MinGW-w64 toolchain.
    
    step "Building Windows release binary (cross-compilation)..."
    info "Using Dockerfile.windows with MinGW-w64 toolchain"
    
    ensure_output_dir
    
    local git_hash=$(get_git_hash)
    docker build $NO_CACHE --platform linux/amd64 --build-arg "GIT_HASH=$git_hash" -t "$IMAGE_WINDOWS" -f Dockerfile.windows .
    
    extract_binary "$IMAGE_WINDOWS" \
        "/build/target/x86_64-pc-windows-gnu/release/gik.exe" \
        "$BINARY_WINDOWS"
    
    local size=$(get_binary_size "$BINARY_WINDOWS")
    success "Windows binary: $BINARY_WINDOWS ($size)"
}

build_release_cuda() {
    # Build Linux x86_64 binary with CUDA GPU support.
    # Uses Dockerfile.cuda with NVIDIA CUDA 12.4 toolkit.
    #
    # CUDA compute capabilities:
    # - 70: Volta (V100)
    # - 75: Turing (RTX 20xx, T4)
    # - 80: Ampere (A100)
    # - 86: Ampere (RTX 30xx, A10) [default]
    # - 89: Ada Lovelace (RTX 40xx, L4)
    # - 90: Hopper (H100)
    
    step "Building CUDA-enabled release binary..."
    info "CUDA compute capability: $CUDA_ARCH"
    warning "Requires NVIDIA Container Toolkit for runtime"
    
    ensure_output_dir
    
    local git_hash=$(get_git_hash)
    docker build $NO_CACHE \
        --platform linux/amd64 \
        --build-arg "GIT_HASH=$git_hash" \
        --build-arg "CUDA_COMPUTE_CAP=$CUDA_ARCH" \
        -t "$IMAGE_CUDA" \
        -f Dockerfile.cuda .
    
    extract_binary "$IMAGE_CUDA" "/usr/local/bin/gik" "$BINARY_CUDA"
    
    local size=$(get_binary_size "$BINARY_CUDA")
    success "CUDA binary: $BINARY_CUDA ($size)"
    info "Run with: docker run --gpus all -v \$PWD:/workspace $IMAGE_CUDA [command]"
}

install_gik() {
    # Build and install GIK to user's cargo bin directory.
    # Builds Linux binary and copies to ~/.cargo/bin/gik
    
    step "Installing GIK to $INSTALL_DIR..."
    
    # Ensure install directory exists
    if [[ ! -d "$INSTALL_DIR" ]]; then
        mkdir -p "$INSTALL_DIR"
        warning "Created directory: $INSTALL_DIR"
        warning "Make sure $INSTALL_DIR is in your PATH"
    fi
    
    # Build Linux binary
    build_release
    
    # Copy to install directory
    if [[ -f "$BINARY_LINUX" ]]; then
        cp "$BINARY_LINUX" "$INSTALL_DIR/gik"
        chmod +x "$INSTALL_DIR/gik"
        local size=$(get_binary_size "$INSTALL_DIR/gik")
        success "Installed: $INSTALL_DIR/gik ($size)"
        
        # Verify installation
        if "$INSTALL_DIR/gik" --version &> /dev/null; then
            local version=$("$INSTALL_DIR/gik" --version)
            success "Verified: $version"
        else
            warning "Installation complete but verification failed"
        fi
    else
        error "Build failed: Linux binary not found at $BINARY_LINUX"
    fi
}

start_dev_shell() {
    # Start interactive development shell with full toolchain.
    # Uses docker-compose with Dockerfile.dev for development environment.
    
    step "Starting development shell..."
    info "Type 'exit' to leave the shell"
    
    build_dev_image
    docker-compose run --rm gik-dev bash
}

# =============================================================================
# TEST FUNCTIONS
# =============================================================================

run_tests() {
    step "Running all tests..."
    build_dev_image
    docker-compose run --rm gik-dev cargo test --workspace
}

run_unit_tests() {
    step "Running unit tests..."
    build_dev_image
    docker-compose run --rm gik-dev cargo test --workspace --lib
}

run_integration_tests() {
    step "Running integration tests..."
    build_dev_image
    docker-compose run --rm gik-dev cargo test --workspace --test '*'
}

# =============================================================================
# LINT/FORMAT FUNCTIONS
# =============================================================================

run_format() {
    step "Formatting code..."
    build_dev_image
    docker-compose run --rm gik-dev cargo fmt --all
    success "Code formatted"
}

run_clippy() {
    step "Running clippy..."
    build_dev_image
    docker-compose run --rm gik-dev cargo clippy --workspace --all-targets --all-features -- -D warnings
}

# =============================================================================
# CLEANUP FUNCTIONS
# =============================================================================

clean_build() {
    step "Cleaning build artifacts..."
    
    # Clean cargo target inside container
    docker-compose run --rm gik-dev cargo clean 2>/dev/null || true
    
    # Remove local binaries
    rm -f "$BINARY_LINUX" "$BINARY_WINDOWS" "$BINARY_CUDA"
    
    # Stop and remove containers
    docker-compose down -v 2>/dev/null || true
    
    # Remove Docker images
    docker rmi "$IMAGE_LINUX" -f 2>/dev/null || true
    docker rmi "$IMAGE_WINDOWS" -f 2>/dev/null || true
    docker rmi "$IMAGE_CUDA" -f 2>/dev/null || true
    
    success "Clean complete!"
}

# =============================================================================
# HELP
# =============================================================================

show_help() {
    cat << 'EOF'
GIK CLI Build Script

USAGE:
    ./build.sh [command] [options]

COMMANDS:
    release           Build Linux x86_64 release binary (default)
    release-windows   Build Windows x86_64 binary (cross-compilation)
    release-cuda      Build Linux x86_64 with CUDA/GPU support
    install           Build Linux binary and install to ~/.cargo/bin
    dev               Start interactive development shell
    test              Run all tests (unit + integration)
    test-unit         Run unit tests only (faster)
    test-integration  Run integration tests only
    fmt               Format code with rustfmt
    clippy            Run clippy linter
    clean             Clean all build artifacts and caches
    help              Show this help message

OPTIONS:
    --no-cache        Disable Docker cache (fresh build)
    --cuda-arch N     CUDA compute capability (70|75|80|86|89|90)

EXAMPLES:
    ./build.sh release                     # Linux release build
    ./build.sh install                     # Build and install Linux binary
    ./build.sh release-cuda --cuda-arch 89 # CUDA build for RTX 40xx
    ./build.sh test --no-cache             # Fresh test build

DOCKERFILES:
    Dockerfile         Linux x86_64 (multi-stage, layered)
    Dockerfile.windows Windows x86_64 (MinGW cross-compilation)
    Dockerfile.cuda    Linux x86_64 + CUDA (GPU support)
    Dockerfile.dev     Development environment

For more information, see README-DOCKER.md
EOF
}

# =============================================================================
# MAIN EXECUTION
# =============================================================================

# Ensure Docker is available
check_docker

# Execute command
case "$COMMAND" in
    release)
        build_release
        ;;
    release-windows)
        build_release_windows
        ;;
    release-cuda)
        build_release_cuda
        ;;
    install)
        install_gik
        ;;
    dev)
        start_dev_shell
        ;;
    test)
        run_tests
        ;;
    test-unit)
        run_unit_tests
        ;;
    test-integration)
        run_integration_tests
        ;;
    fmt)
        run_format
        ;;
    clippy)
        run_clippy
        ;;
    clean)
        clean_build
        ;;
    help)
        show_help
        ;;
    *)
        echo "Unknown command: $COMMAND"
        echo "Run './build.sh help' for usage information."
        exit 1
        ;;
esac
