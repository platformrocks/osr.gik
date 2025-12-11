#!/usr/bin/env pwsh
# =============================================================================
# GIK CLI - Build Script (PowerShell)
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
#   .\build.ps1 [command] [options]
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
#   -NoBuildCache   Disable Docker build cache (fresh build)
#   -CudaArch       CUDA compute capability (default: 86 for RTX 30xx)
#
# EXAMPLES:
#   .\build.ps1 release                    # Linux release build
#   .\build.ps1 install                    # Build and install Windows binary
#   .\build.ps1 release-cuda -CudaArch 89  # CUDA build for RTX 40xx
#   .\build.ps1 test -NoBuildCache         # Fresh test build
#
# =============================================================================

param(
    [Parameter(Position = 0)]
    [ValidateSet(
        'release', 
        'release-windows', 
        'release-cuda', 
        'install', 
        'dev', 
        'test', 
        'test-unit',
        'test-integration',
        'fmt', 
        'clippy', 
        'clean',
        'help'
    )]
    [string]$Command = 'release',
    
    [Parameter()]
    [switch]$NoBuildCache,
    
    [Parameter()]
    [ValidateSet('70', '75', '80', '86', '89', '90')]
    [string]$CudaArch = '86'
)

# =============================================================================
# CONFIGURATION
# =============================================================================

$ErrorActionPreference = "Stop"

# Docker image names
$IMAGE_LINUX = "gik-cli:latest"
$IMAGE_WINDOWS = "gik-cli-windows:latest"
$IMAGE_CUDA = "gik-cli-cuda:latest"

# Output paths
$OUTPUT_DIR = "./target"
$BINARY_LINUX = "$OUTPUT_DIR/gik"
$BINARY_WINDOWS = "$OUTPUT_DIR/gik.exe"
$BINARY_CUDA = "$OUTPUT_DIR/gik-cuda"

# Installation path
$INSTALL_DIR = "$env:USERPROFILE\.cargo\bin"

# =============================================================================
# UTILITY FUNCTIONS
# =============================================================================

function Write-Step {
    param([string]$Message)
    Write-Host "`n" -NoNewline
    Write-Host "==> " -ForegroundColor Cyan -NoNewline
    Write-Host $Message
}

function Write-Success {
    param([string]$Message)
    Write-Host $Message -ForegroundColor Green
}

function Write-Warning {
    param([string]$Message)
    Write-Host $Message -ForegroundColor Yellow
}

function Write-Info {
    param([string]$Message)
    Write-Host $Message -ForegroundColor Gray
}

function Get-GitHash {
    try {
        $hash = git rev-parse --short HEAD 2>$null
        if ($LASTEXITCODE -eq 0 -and $hash) {
            return $hash.Trim()
        }
    } catch {}
    return "unknown"
}

function Invoke-DockerBuild {
    param(
        [string]$ImageName,
        [string]$DockerFile,
        [string]$ExtraBuildArg = $null
    )
    
    $gitHash = Get-GitHash
    $cmd = "docker build -t $ImageName -f $DockerFile --build-arg GIT_HASH=$gitHash"
    if ($NoBuildCache) {
        $cmd += " --no-cache"
    }
    if ($ExtraBuildArg) {
        $cmd += " --build-arg $ExtraBuildArg"
    }
    $cmd += " ."
    
    Write-Info "Running: $cmd"
    Invoke-Expression $cmd
    
    if ($LASTEXITCODE -ne 0) {
        throw "Docker build failed with exit code $LASTEXITCODE"
    }
}

function Test-DockerAvailable {
    try {
        $null = docker version 2>&1
        # Docker is available, continue
    } catch {
        Write-Host "ERROR: Docker is not available or not running." -ForegroundColor Red
        Write-Host "Please install Docker Desktop and ensure it's running." -ForegroundColor Yellow
        exit 1
    }
}

function Ensure-OutputDir {
    if (-not (Test-Path $OUTPUT_DIR)) {
        New-Item -ItemType Directory -Path $OUTPUT_DIR -Force | Out-Null
    }
}

function Get-BinarySize {
    param([string]$Path)
    if (Test-Path $Path) {
        $size = (Get-Item $Path).Length
        return [math]::Round($size / 1MB, 1)
    }
    return 0
}

function Extract-Binary {
    param(
        [string]$ImageName,
        [string]$ContainerPath,
        [string]$HostPath
    )
    
    Write-Step "Extracting binary..."
    $containerId = docker create $ImageName
    try {
        docker cp "${containerId}:${ContainerPath}" $HostPath
        Write-Success "Binary extracted to: $HostPath"
    } finally {
        docker rm $containerId | Out-Null
    }
}

# =============================================================================
# BUILD FUNCTIONS
# =============================================================================

function Build-DevImage {
    Write-Step "Building development image..."
    docker-compose build gik-dev
}

function Build-Release {
    <#
    .SYNOPSIS
    Build Linux x86_64 release binary using multi-stage Docker build.
    
    .DESCRIPTION
    Uses Dockerfile with layered caching:
    - Base layer cached until Rust version changes
    - Deps layer cached until Cargo.toml changes
    - Build layer recompiled on source changes
    #>
    
    Write-Step "Building Linux release binary..."
    Write-Info "Using Dockerfile with layered caching strategy"
    
    Ensure-OutputDir
    
    Invoke-DockerBuild -ImageName $IMAGE_LINUX -DockerFile "Dockerfile"
    
    Extract-Binary -ImageName $IMAGE_LINUX `
                   -ContainerPath "/usr/local/bin/gik" `
                   -HostPath $BINARY_LINUX
    
    $size = Get-BinarySize $BINARY_LINUX
    Write-Success "Linux binary: $BINARY_LINUX (${size}MB)"
}

function Build-ReleaseWindows {
    <#
    .SYNOPSIS
    Build Windows x86_64 binary using MinGW cross-compilation.
    
    .DESCRIPTION
    Uses Dockerfile.windows with MinGW-w64 toolchain.
    Cross-compiles from Linux to avoid Windows build environment issues.
    #>
    
    Write-Step "Building Windows release binary (cross-compilation)..."
    Write-Info "Using Dockerfile.windows with MinGW-w64 toolchain"
    
    Ensure-OutputDir
    
    Invoke-DockerBuild -ImageName $IMAGE_WINDOWS -DockerFile "Dockerfile.windows"
    
    Extract-Binary -ImageName $IMAGE_WINDOWS `
                   -ContainerPath "/build/target/x86_64-pc-windows-gnu/release/gik.exe" `
                   -HostPath $BINARY_WINDOWS
    
    $size = Get-BinarySize $BINARY_WINDOWS
    Write-Success "Windows binary: $BINARY_WINDOWS (${size}MB)"
}

function Build-ReleaseCuda {
    <#
    .SYNOPSIS
    Build Linux x86_64 binary with CUDA GPU support.
    
    .DESCRIPTION
    Uses Dockerfile.cuda with NVIDIA CUDA 12.4 toolkit.
    Requires NVIDIA Container Toolkit for runtime.
    
    .PARAMETER CudaArch
    CUDA compute capability:
    - 70: Volta (V100)
    - 75: Turing (RTX 20xx, T4)
    - 80: Ampere (A100)
    - 86: Ampere (RTX 30xx, A10) [default]
    - 89: Ada Lovelace (RTX 40xx, L4)
    - 90: Hopper (H100)
    #>
    
    Write-Step "Building CUDA-enabled release binary..."
    Write-Info "CUDA compute capability: $CudaArch"
    Write-Warning "Requires NVIDIA Container Toolkit for runtime"
    
    Ensure-OutputDir
    
    Invoke-DockerBuild -ImageName $IMAGE_CUDA -DockerFile "Dockerfile.cuda" -ExtraBuildArg "CUDA_COMPUTE_CAP=$CudaArch"
    
    Extract-Binary -ImageName $IMAGE_CUDA `
                   -ContainerPath "/usr/local/bin/gik" `
                   -HostPath $BINARY_CUDA
    
    $size = Get-BinarySize $BINARY_CUDA
    Write-Success "CUDA binary: $BINARY_CUDA (${size}MB)"
    Write-Info "Run with: docker run --gpus all -v `$PWD:/workspace $IMAGE_CUDA [command]"
}

function Install-Gik {
    <#
    .SYNOPSIS
    Build and install GIK to user's cargo bin directory.
    
    .DESCRIPTION
    Builds Windows binary and copies to ~/.cargo/bin/gik.exe
    Assumes cargo bin is in PATH (standard Rust installation).
    #>
    
    Write-Step "Installing GIK to $INSTALL_DIR..."
    
    # Ensure install directory exists
    if (-not (Test-Path $INSTALL_DIR)) {
        New-Item -ItemType Directory -Path $INSTALL_DIR -Force | Out-Null
        Write-Warning "Created directory: $INSTALL_DIR"
        Write-Warning "Make sure $INSTALL_DIR is in your PATH"
    }
    
    # Build Windows binary
    Build-ReleaseWindows
    
    # Copy to install directory
    if (Test-Path $BINARY_WINDOWS) {
        Copy-Item $BINARY_WINDOWS "$INSTALL_DIR\gik.exe" -Force
        $size = Get-BinarySize "$INSTALL_DIR\gik.exe"
        Write-Success "Installed: $INSTALL_DIR\gik.exe (${size}MB)"
        
        # Verify installation
        try {
            $version = & "$INSTALL_DIR\gik.exe" --version 2>&1
            Write-Success "Verified: $version"
        } catch {
            Write-Warning "Installation complete but verification failed"
        }
    } else {
        Write-Error "Build failed: Windows binary not found at $BINARY_WINDOWS"
        exit 1
    }
}

function Start-DevShell {
    <#
    .SYNOPSIS
    Start interactive development shell with full toolchain.
    
    .DESCRIPTION
    Uses docker-compose with Dockerfile.dev for development environment.
    Includes: rustfmt, clippy, rust-analyzer, cargo-watch, etc.
    Source code is mounted for live editing.
    #>
    
    Write-Step "Starting development shell..."
    Write-Info "Type 'exit' to leave the shell"
    
    Build-DevImage
    docker-compose run --rm gik-dev bash
}

# =============================================================================
# TEST FUNCTIONS
# =============================================================================

function Run-Tests {
    <#
    .SYNOPSIS
    Run all tests (unit + integration).
    #>
    
    Write-Step "Running all tests..."
    Build-DevImage
    docker-compose run --rm gik-dev cargo test --workspace
}

function Run-UnitTests {
    <#
    .SYNOPSIS
    Run unit tests only (faster, no integration tests).
    #>
    
    Write-Step "Running unit tests..."
    Build-DevImage
    docker-compose run --rm gik-dev cargo test --workspace --lib
}

function Run-IntegrationTests {
    <#
    .SYNOPSIS
    Run integration tests only.
    #>
    
    Write-Step "Running integration tests..."
    Build-DevImage
    docker-compose run --rm gik-dev cargo test --workspace --test '*'
}

# =============================================================================
# LINT/FORMAT FUNCTIONS
# =============================================================================

function Run-Format {
    <#
    .SYNOPSIS
    Format all code with rustfmt.
    #>
    
    Write-Step "Formatting code..."
    Build-DevImage
    docker-compose run --rm gik-dev cargo fmt --all
    Write-Success "Code formatted"
}

function Run-Clippy {
    <#
    .SYNOPSIS
    Run clippy linter with strict settings.
    #>
    
    Write-Step "Running clippy..."
    Build-DevImage
    docker-compose run --rm gik-dev cargo clippy --workspace --all-targets --all-features -- -D warnings
}

# =============================================================================
# CLEANUP FUNCTIONS
# =============================================================================

function Clean-Build {
    <#
    .SYNOPSIS
    Clean all build artifacts, Docker images, and caches.
    #>
    
    Write-Step "Cleaning build artifacts..."
    
    # Clean cargo target inside container
    try {
        docker-compose run --rm gik-dev cargo clean 2>$null
    } catch {
        Write-Info "Skipping cargo clean (container not available)"
    }
    
    # Remove local binaries
    Remove-Item -Path $BINARY_LINUX -ErrorAction SilentlyContinue
    Remove-Item -Path $BINARY_WINDOWS -ErrorAction SilentlyContinue
    Remove-Item -Path $BINARY_CUDA -ErrorAction SilentlyContinue
    
    # Stop and remove containers
    docker-compose down -v 2>$null
    
    # Remove Docker images
    docker rmi $IMAGE_LINUX -f 2>$null
    docker rmi $IMAGE_WINDOWS -f 2>$null
    docker rmi $IMAGE_CUDA -f 2>$null
    
    Write-Success "Clean complete!"
}

# =============================================================================
# HELP
# =============================================================================

function Show-Help {
    $helpText = @"
GIK CLI Build Script

USAGE:
    .\build.ps1 [command] [options]

COMMANDS:
    release           Build Linux x86_64 release binary (default)
    release-windows   Build Windows x86_64 binary (cross-compilation)
    release-cuda      Build Linux x86_64 with CUDA/GPU support
    install           Build Windows binary and install to ~/.cargo/bin
    dev               Start interactive development shell
    test              Run all tests (unit + integration)
    test-unit         Run unit tests only (faster)
    test-integration  Run integration tests only
    fmt               Format code with rustfmt
    clippy            Run clippy linter
    clean             Clean all build artifacts and caches
    help              Show this help message

OPTIONS:
    -NoBuildCache     Disable Docker cache (fresh build)
    -CudaArch <cap>   CUDA compute capability (70|75|80|86|89|90)

EXAMPLES:
    .\build.ps1 release                    # Linux release build
    .\build.ps1 install                    # Build and install Windows binary
    .\build.ps1 release-cuda -CudaArch 89  # CUDA build for RTX 40xx
    .\build.ps1 test -NoBuildCache         # Fresh test build

DOCKERFILES:
    Dockerfile         Linux x86_64 (multi-stage, layered)
    Dockerfile.windows Windows x86_64 (MinGW cross-compilation)
    Dockerfile.cuda    Linux x86_64 + CUDA (GPU support)
    Dockerfile.dev     Development environment

For more information, see README-DOCKER.md
"@
    Write-Host $helpText
}

# =============================================================================
# MAIN EXECUTION
# =============================================================================

# Ensure Docker is available
Test-DockerAvailable

# Execute command
switch ($Command) {
    'release'          { Build-Release }
    'release-windows'  { Build-ReleaseWindows }
    'release-cuda'     { Build-ReleaseCuda }
    'install'          { Install-Gik }
    'dev'              { Start-DevShell }
    'test'             { Run-Tests }
    'test-unit'        { Run-UnitTests }
    'test-integration' { Run-IntegrationTests }
    'fmt'              { Run-Format }
    'clippy'           { Run-Clippy }
    'clean'            { Clean-Build }
    'help'             { Show-Help }
}
