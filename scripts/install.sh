#!/usr/bin/env bash
# =============================================================================
# GIK Install Script (Linux/macOS)
# =============================================================================
#
# Installs the GIK CLI from GitHub Releases.
#
# USAGE:
#   curl -fsSL https://raw.githubusercontent.com/platformrocks/osr.gik/main/scripts/install.sh | bash
#
#   # Or with specific version:
#   curl -fsSL https://raw.githubusercontent.com/platformrocks/osr.gik/main/scripts/install.sh | GIK_VERSION=v0.1.0 bash
#
# OPTIONS:
#   -h, --help      Show this help message
#
# ENVIRONMENT VARIABLES:
#   GIK_REPO        GitHub repository (default: platformrocks/osr.gik)
#   GIK_VERSION     Version to install (default: latest)
#   GIK_INSTALL_BIN Custom binary install path (default: /usr/local/bin or ~/.local/bin)
#   GIK_HOME        GIK home directory (default: ~/.gik)
#
# =============================================================================

set -euo pipefail

# =============================================================================
# CONFIGURATION
# =============================================================================

GIK_REPO="${GIK_REPO:-platformrocks/osr.gik}"
GIK_VERSION="${GIK_VERSION:-latest}"
GIK_HOME="${GIK_HOME:-$HOME/.gik}"
GIK_INSTALL_BIN="${GIK_INSTALL_BIN:-}"

# Temporary directory (cleaned up on exit)
TMP_DIR=""

# =============================================================================
# COLORS
# =============================================================================

RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
CYAN='\033[0;36m'
GRAY='\033[0;90m'
NC='\033[0m'

# =============================================================================
# LOGGING
# =============================================================================

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

# =============================================================================
# CLEANUP
# =============================================================================

cleanup() {
    if [[ -n "$TMP_DIR" && -d "$TMP_DIR" ]]; then
        rm -rf "$TMP_DIR"
    fi
}

trap cleanup EXIT

# =============================================================================
# HELP
# =============================================================================

show_help() {
    cat << 'EOF'
GIK Install Script

USAGE:
  curl -fsSL https://raw.githubusercontent.com/platformrocks/osr.gik/main/scripts/install.sh | bash

  # Install specific version:
  curl -fsSL ... | GIK_VERSION=v0.1.0 bash

  # Custom install directory:
  curl -fsSL ... | GIK_INSTALL_BIN=/opt/bin bash

ENVIRONMENT VARIABLES:
  GIK_REPO        GitHub repository (default: platformrocks/osr.gik)
  GIK_VERSION     Version to install (default: latest)
  GIK_INSTALL_BIN Custom binary install path
  GIK_HOME        GIK home directory (default: ~/.gik)

WHAT GETS INSTALLED:
  - gik binary    → /usr/local/bin/gik (or ~/.local/bin/gik)
  - models        → ~/.gik/models/
  - config        → ~/.gik/config.yaml (created if not exists)

EOF
    exit 0
}

# =============================================================================
# DETECTION
# =============================================================================

detect_os() {
    local os
    os="$(uname -s)"
    case "$os" in
        Linux)  echo "linux" ;;
        Darwin) echo "macos" ;;
        *)      error "Unsupported OS: $os. GIK supports Linux and macOS." ;;
    esac
}

detect_arch() {
    local arch
    arch="$(uname -m)"
    case "$arch" in
        x86_64)         echo "x86_64" ;;
        aarch64|arm64)  echo "aarch64" ;;
        *)              error "Unsupported architecture: $arch. GIK supports x86_64 and aarch64." ;;
    esac
}

detect_target() {
    local os arch target
    os="$(detect_os)"
    arch="$(detect_arch)"
    target="${os}-${arch}"
    
    # Validate supported targets
    case "$target" in
        linux-x86_64|macos-x86_64|macos-aarch64)
            echo "$target"
            ;;
        linux-aarch64)
            error "Linux aarch64 is not yet supported. Please use x86_64."
            ;;
        *)
            error "Unsupported target: $target"
            ;;
    esac
}

# =============================================================================
# DOWNLOAD
# =============================================================================

build_download_url() {
    local target="$1"
    local version="$2"
    local repo="$3"
    local artifact="gik-${target}.tar.gz"
    
    if [[ "$version" == "latest" ]]; then
        echo "https://github.com/${repo}/releases/latest/download/${artifact}"
    else
        echo "https://github.com/${repo}/releases/download/${version}/${artifact}"
    fi
}

download_artifact() {
    local url="$1"
    local output="$2"
    
    step "Downloading GIK from $url"
    
    if ! curl -fsSL --progress-bar -o "$output" "$url"; then
        error "Failed to download from $url. Check your network connection and verify the version exists."
    fi
    
    success "Downloaded $(basename "$output")"
}

# =============================================================================
# EXTRACTION
# =============================================================================

extract_artifact() {
    local archive="$1"
    local dest="$2"
    
    step "Extracting archive"
    
    if ! tar -xzf "$archive" -C "$dest"; then
        error "Failed to extract $archive. The file may be corrupted."
    fi
    
    success "Extracted to $dest"
}

validate_package() {
    local dir="$1"
    local target="$2"
    local pkg_dir="$dir/gik-$target"
    
    step "Validating package contents"
    
    # Find the package directory
    if [[ ! -d "$pkg_dir" ]]; then
        # Try to find any gik-* directory
        pkg_dir=$(find "$dir" -maxdepth 1 -type d -name "gik-*" | head -1)
        if [[ -z "$pkg_dir" ]]; then
            error "Package structure invalid: no gik-* directory found"
        fi
    fi
    
    # Validate required files
    if [[ ! -f "$pkg_dir/bin/gik" ]]; then
        error "Package invalid: bin/gik not found"
    fi
    
    if [[ ! -d "$pkg_dir/models" ]]; then
        error "Package invalid: models/ directory not found"
    fi
    
    if [[ ! -f "$pkg_dir/config.default.yaml" ]]; then
        error "Package invalid: config.default.yaml not found"
    fi
    
    success "Package validated"
    echo "$pkg_dir"
}

# =============================================================================
# INSTALLATION
# =============================================================================

install_binary() {
    local src="$1"
    local install_bin="$2"
    local bin_dir
    local bin_path
    
    step "Installing GIK binary"
    
    if [[ -n "$install_bin" ]]; then
        # Custom install path specified
        bin_dir="$(dirname "$install_bin")"
        bin_path="$install_bin"
        
        mkdir -p "$bin_dir"
        cp "$src" "$bin_path"
        chmod +x "$bin_path"
        success "Installed to $bin_path"
        echo "$bin_path"
        return
    fi
    
    # Try /usr/local/bin first (requires sudo)
    if [[ -w "/usr/local/bin" ]]; then
        bin_path="/usr/local/bin/gik"
        cp "$src" "$bin_path"
        chmod +x "$bin_path"
        success "Installed to $bin_path"
        echo "$bin_path"
        return
    fi
    
    # Try with sudo
    if command -v sudo &> /dev/null; then
        if sudo -n true 2>/dev/null || sudo true; then
            bin_path="/usr/local/bin/gik"
            sudo install -m 755 "$src" "$bin_path"
            success "Installed to $bin_path (with sudo)"
            echo "$bin_path"
            return
        fi
    fi
    
    # Fallback to ~/.local/bin
    bin_dir="$HOME/.local/bin"
    bin_path="$bin_dir/gik"
    
    mkdir -p "$bin_dir"
    cp "$src" "$bin_path"
    chmod +x "$bin_path"
    
    success "Installed to $bin_path"
    
    # Check if ~/.local/bin is in PATH
    if [[ ":$PATH:" != *":$bin_dir:"* ]]; then
        warning "$bin_dir is not in your PATH"
        info "Add this to your shell profile (~/.bashrc, ~/.zshrc, etc.):"
        info "  export PATH=\"\$HOME/.local/bin:\$PATH\""
    fi
    
    echo "$bin_path"
}

install_models() {
    local src="$1"
    local gik_home="$2"
    
    step "Installing models to $gik_home/models"
    
    mkdir -p "$gik_home/models"
    
    # Copy models, overwriting existing
    cp -R "$src"/* "$gik_home/models/"
    
    success "Models installed"
}

install_config() {
    local src="$1"
    local gik_home="$2"
    local config_path="$gik_home/config.yaml"
    
    step "Installing configuration"
    
    mkdir -p "$gik_home"
    
    if [[ -f "$config_path" ]]; then
        info "Existing config.yaml found, keeping it unchanged"
    else
        cp "$src" "$config_path"
        success "Created $config_path"
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
    
    echo ""
    echo "=========================================="
    echo "  GIK Installer"
    echo "=========================================="
    echo ""
    
    # Detect target platform
    step "Detecting platform"
    local target
    target="$(detect_target)"
    success "Detected: $target"
    
    # Show configuration
    info "Repository: $GIK_REPO"
    info "Version:    $GIK_VERSION"
    info "GIK Home:   $GIK_HOME"
    echo ""
    
    # Create temporary directory
    TMP_DIR="$(mktemp -d)"
    
    # Build download URL
    local url
    url="$(build_download_url "$target" "$GIK_VERSION" "$GIK_REPO")"
    
    # Download
    local archive="$TMP_DIR/gik.tar.gz"
    download_artifact "$url" "$archive"
    
    # Extract
    extract_artifact "$archive" "$TMP_DIR"
    
    # Validate
    local pkg_dir
    pkg_dir="$(validate_package "$TMP_DIR" "$target")"
    
    # Install binary
    local bin_path
    bin_path="$(install_binary "$pkg_dir/bin/gik" "$GIK_INSTALL_BIN")"
    
    # Install models
    install_models "$pkg_dir/models" "$GIK_HOME"
    
    # Install config
    install_config "$pkg_dir/config.default.yaml" "$GIK_HOME"
    
    # Summary
    echo ""
    echo "=========================================="
    echo "  Installation Complete!"
    echo "=========================================="
    echo ""
    success "Binary:   $bin_path"
    success "GIK Home: $GIK_HOME"
    echo ""
    info "Verify installation:"
    info "  gik --version"
    echo ""
    
    # Check if binary is accessible
    if command -v gik &> /dev/null; then
        info "GIK is ready to use!"
    else
        warning "You may need to restart your terminal or add the install directory to PATH"
    fi
}

main "$@"
