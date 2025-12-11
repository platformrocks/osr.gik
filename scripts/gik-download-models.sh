#!/usr/bin/env bash
# =============================================================================
# GIK Model Downloader
# =============================================================================
#
# Downloads and prepares the required ML models for GIK.
# Keeps only the essential files (config.json, model.safetensors, tokenizer.json)
# and removes all other HuggingFace artifacts.
#
# Usage:
#   ./scripts/gik-download-models.sh [OPTIONS]
#
# Options:
#   -h, --help      Show this help message
#   --force         Re-download even if models already exist
#   --dry-run       Show what would be done without downloading
#
# Environment Variables:
#   MODELS_DIR      Target directory for models (default: ./vendor/models)
#
# Required Files per Model:
#   - config.json        Model configuration
#   - model.safetensors  Model weights (safe format)
#   - tokenizer.json     Tokenizer configuration
#
# Models Downloaded:
#   - Embedding:  sentence-transformers/all-MiniLM-L6-v2
#   - Reranker:   cross-encoder/ms-marco-MiniLM-L6-v2
#
# =============================================================================

set -euo pipefail

# =============================================================================
# CONFIGURATION
# =============================================================================

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"

# Target directory for models
MODELS_DIR="${MODELS_DIR:-$REPO_ROOT/vendor/models}"

# Model definitions
EMBEDDING_MODEL_REPO="sentence-transformers/all-MiniLM-L6-v2"
EMBEDDING_MODEL_NAME="all-MiniLM-L6-v2"

RERANKER_MODEL_REPO="cross-encoder/ms-marco-MiniLM-L6-v2"
RERANKER_MODEL_NAME="ms-marco-MiniLM-L6-v2"

# Required files for GIK (minimal set)
REQUIRED_FILES=("config.json" "model.safetensors" "tokenizer.json")

# Flags
FORCE_DOWNLOAD=false
DRY_RUN=false

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
# HELP
# =============================================================================

show_help() {
    head -35 "$0" | grep -E '^#' | sed 's/^# //' | sed 's/^#//'
    exit 0
}

# =============================================================================
# FUNCTIONS
# =============================================================================

check_dependencies() {
    step "Checking dependencies"
    
    if ! command -v curl &> /dev/null && ! command -v wget &> /dev/null; then
        error "curl or wget is required but neither is installed"
    fi
    
    local downloader="curl"
    if ! command -v curl &> /dev/null; then
        downloader="wget"
    fi
    
    success "Dependencies OK ($downloader available)"
}

model_exists() {
    local model_dir="$1"
    
    if [[ ! -d "$model_dir" ]]; then
        return 1
    fi
    
    for file in "${REQUIRED_FILES[@]}"; do
        if [[ ! -f "$model_dir/$file" ]]; then
            return 1
        fi
    done
    
    # Check if model.safetensors has actual content (not just LFS pointer)
    local model_file="$model_dir/model.safetensors"
    if [[ -f "$model_file" ]]; then
        local file_size
        file_size=$(wc -c < "$model_file" | tr -d ' ')
        if [[ "$file_size" -lt 1000 ]]; then
            # File is likely an LFS pointer, not the actual model
            return 1
        fi
    fi
    
    return 0
}

download_file() {
    local url="$1"
    local output="$2"
    
    if command -v curl &> /dev/null; then
        curl -L --progress-bar -o "$output" "$url"
    elif command -v wget &> /dev/null; then
        wget --show-progress -q -O "$output" "$url"
    else
        error "Neither curl nor wget is available"
    fi
}

download_model() {
    local repo="$1"
    local name="$2"
    local target_dir="$3"
    local category="$4"  # embeddings or rerankers
    
    local full_target="$target_dir/$category/$name"
    local base_url="https://huggingface.co/$repo/resolve/main"
    
    step "Processing $category model: $name"
    info "Repository: https://huggingface.co/$repo"
    info "Target: $full_target"
    
    # Check if model already exists
    if [[ "$FORCE_DOWNLOAD" == "false" ]] && model_exists "$full_target"; then
        success "Model already exists and is complete, skipping"
        return 0
    fi
    
    if [[ "$DRY_RUN" == "true" ]]; then
        info "[DRY-RUN] Would download from $base_url"
        info "[DRY-RUN] Would download: ${REQUIRED_FILES[*]}"
        return 0
    fi
    
    # Create target directory
    mkdir -p "$full_target"
    
    # Remove .gitkeep if exists
    rm -f "$full_target/.gitkeep"
    
    # Download each required file directly
    for file in "${REQUIRED_FILES[@]}"; do
        local url="$base_url/$file"
        local output="$full_target/$file"
        
        info "Downloading $file..."
        if ! download_file "$url" "$output"; then
            error "Failed to download $file from $url"
        fi
        
        # Verify file was downloaded
        if [[ ! -f "$output" ]] || [[ ! -s "$output" ]]; then
            error "Download failed or file is empty: $file"
        fi
    done
    
    # Show file sizes
    info "Model files:"
    for file in "${REQUIRED_FILES[@]}"; do
        local size
        size=$(ls -lh "$full_target/$file" | awk '{print $5}')
        info "  $file ($size)"
    done
    
    success "Model downloaded: $name"
}

create_gitattributes() {
    local models_dir="$1"
    
    step "Creating .gitattributes for LFS tracking"
    
    if [[ "$DRY_RUN" == "true" ]]; then
        info "[DRY-RUN] Would create $models_dir/.gitattributes"
        return 0
    fi
    
    cat > "$models_dir/.gitattributes" << 'EOF'
# Track model weights with Git LFS
*.safetensors filter=lfs diff=lfs merge=lfs -text
*.bin filter=lfs diff=lfs merge=lfs -text
*.onnx filter=lfs diff=lfs merge=lfs -text
EOF
    
    success "Created .gitattributes for LFS"
}

show_summary() {
    local models_dir="$1"
    
    echo ""
    echo "=========================================="
    echo "  Model Download Summary"
    echo "=========================================="
    echo ""
    
    if [[ "$DRY_RUN" == "true" ]]; then
        warning "DRY-RUN mode - no files were downloaded"
        echo ""
        return 0
    fi
    
    # Show directory tree
    info "Directory structure:"
    if command -v tree &> /dev/null; then
        tree -L 3 "$models_dir" 2>/dev/null | while read -r line; do
            echo -e "${GRAY}  $line${NC}"
        done
    else
        find "$models_dir" -type f | while read -r file; do
            local rel_path="${file#$models_dir/}"
            local size
            size=$(ls -lh "$file" | awk '{print $5}')
            echo -e "${GRAY}  $rel_path ($size)${NC}"
        done
    fi
    
    echo ""
    
    # Show total size
    local total_size
    total_size=$(du -sh "$models_dir" 2>/dev/null | cut -f1)
    success "Total models size: $total_size"
    
    echo ""
    info "Next steps:"
    info "  1. Run: git add vendor/models/"
    info "  2. Run: git commit -m 'Add default ML models for GIK'"
    info "  3. Push to remote (git-lfs will handle large files)"
    echo ""
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
            --force)
                FORCE_DOWNLOAD=true
                shift
                ;;
            --dry-run)
                DRY_RUN=true
                shift
                ;;
            *)
                error "Unknown argument: $1. Use -h for help."
                ;;
        esac
    done
    
    echo ""
    echo "=========================================="
    echo "  GIK Model Downloader"
    echo "=========================================="
    echo ""
    
    info "Models directory: $MODELS_DIR"
    info "Force download:   $FORCE_DOWNLOAD"
    info "Dry run:          $DRY_RUN"
    echo ""
    
    # Check dependencies
    check_dependencies
    
    # Create models directory structure
    mkdir -p "$MODELS_DIR/embeddings"
    mkdir -p "$MODELS_DIR/rerankers"
    
    # Create .gitattributes for LFS
    create_gitattributes "$MODELS_DIR"
    
    # Download embedding model
    download_model "$EMBEDDING_MODEL_REPO" "$EMBEDDING_MODEL_NAME" "$MODELS_DIR" "embeddings"
    
    # Download reranker model
    download_model "$RERANKER_MODEL_REPO" "$RERANKER_MODEL_NAME" "$MODELS_DIR" "rerankers"
    
    # Show summary
    show_summary "$MODELS_DIR"
}

main "$@"
