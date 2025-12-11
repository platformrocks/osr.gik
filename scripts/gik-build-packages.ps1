#!/usr/bin/env pwsh
# =============================================================================
# GIK CLI - Artifact Packaging Script (PowerShell)
# =============================================================================
#
# IMPORTANT: This script uses Docker-based builds via build.ps1
#
# WHY DOCKER?
#   GIK has complex native dependencies (protobuf, cmake, MinGW-w64) that are
#   difficult to setup manually. Direct `cargo build` will fail with:
#     "error: failed to run custom build command for `lance-encoding`"
#     "protoc failed: google/protobuf/empty.proto: File not found"
#
#   The build.ps1 script handles all Docker orchestration with proper toolchains.
#
# This script creates distributable artifacts for GIK CLI including:
#   - The GIK binary for the target platform (built via Docker)
#   - Default embedding and reranker models
#   - Default configuration file
#   - LICENSE and README.md
#
# USAGE:
#   .\scripts\gik-build-packages.ps1 [OPTIONS]
#
# PARAMETERS:
#   -Target         Target platform (windows-x86_64)
#                   Default: windows-x86_64
#   -Version        Version string to include in artifact naming
#                   Default: extracted from Cargo.toml or "dev"
#   -Help           Show this help message
#
# ENVIRONMENT VARIABLES:
#   GIK_VERSION         Override version string
#   GIK_DIST_DIR        Output directory for artifacts (default: .\dist)
#   GIK_MODELS_SOURCE   Path to models directory to bundle (default: .\vendor\models)
#   GIK_CONFIG_SOURCE   Path to config template file (default: .\config.default.yaml)
#
# EXAMPLES:
#   # Build for Windows x86_64 (default)
#   .\scripts\gik-build-packages.ps1
#
#   # Build with specific version
#   .\scripts\gik-build-packages.ps1 -Version "1.0.0"
#
#   # Override model source
#   $env:GIK_MODELS_SOURCE = "C:\my-models"
#   .\scripts\gik-build-packages.ps1
#
# =============================================================================

[CmdletBinding()]
param(
    [Parameter()]
    [ValidateSet('windows-x86_64')]
    [string]$Target = 'windows-x86_64',
    
    [Parameter()]
    [string]$Version = '',
    
    [Parameter()]
    [switch]$Help
)

# =============================================================================
# CONFIGURATION
# =============================================================================

$ErrorActionPreference = "Stop"

# Script and repo paths
$ScriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$RepoRoot = Split-Path -Parent $ScriptDir

# Default values (can be overridden by environment variables)
$DistDir = if ($env:GIK_DIST_DIR) { $env:GIK_DIST_DIR } else { Join-Path $RepoRoot "dist" }
$ModelsSource = if ($env:GIK_MODELS_SOURCE) { $env:GIK_MODELS_SOURCE } else { Join-Path $RepoRoot "vendor\models" }
$ConfigSource = if ($env:GIK_CONFIG_SOURCE) { $env:GIK_CONFIG_SOURCE } else { Join-Path $RepoRoot "config.default.yaml" }

# Target mapping: logical name -> cargo target triple
$TargetMap = @{
    "windows-x86_64" = "x86_64-pc-windows-msvc"
}

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
    Write-Host "✓ " -ForegroundColor Green -NoNewline
    Write-Host $Message
}

function Write-WarningMessage {
    param([string]$Message)
    Write-Host "⚠ " -ForegroundColor Yellow -NoNewline
    Write-Host $Message
}

function Write-Info {
    param([string]$Message)
    Write-Host "  $Message" -ForegroundColor Gray
}

function Write-ErrorAndExit {
    param([string]$Message)
    Write-Host "✗ ERROR: $Message" -ForegroundColor Red
    exit 1
}

function Show-Help {
    $helpText = @"
GIK CLI - Artifact Packaging Script (PowerShell)

USAGE:
    .\scripts\gik-build-packages.ps1 [OPTIONS]

PARAMETERS:
    -Target         Target platform (windows-x86_64)
                    Default: windows-x86_64
    -Version        Version string to include in artifact naming
                    Default: extracted from Cargo.toml or "dev"
    -Help           Show this help message

ENVIRONMENT VARIABLES:
    GIK_VERSION         Override version string
    GIK_DIST_DIR        Output directory for artifacts (default: .\dist)
    GIK_MODELS_SOURCE   Path to models directory to bundle (default: .\vendor\models)
    GIK_CONFIG_SOURCE   Path to config template file (default: .\config.default.yaml)

EXAMPLES:
    # Build for Windows x86_64 (default)
    .\scripts\gik-build-packages.ps1

    # Build with specific version
    .\scripts\gik-build-packages.ps1 -Version "1.0.0"

    # Override model source
    `$env:GIK_MODELS_SOURCE = "C:\my-models"
    .\scripts\gik-build-packages.ps1
"@
    Write-Host $helpText
    exit 0
}

# =============================================================================
# HELPER FUNCTIONS
# =============================================================================

function Get-CargoTriple {
    param([string]$TargetName)
    
    if (-not $TargetMap.ContainsKey($TargetName)) {
        $supportedTargets = $TargetMap.Keys -join ', '
        Write-ErrorAndExit "Unknown target: $TargetName. Supported: $supportedTargets"
    }
    return $TargetMap[$TargetName]
}

function Get-GikVersion {
    # Check environment variable first
    if ($env:GIK_VERSION) {
        return $env:GIK_VERSION
    }
    
    # Check parameter
    if ($Version) {
        return $Version
    }
    
    # Try to extract from Cargo.toml
    $cargoToml = Join-Path $RepoRoot "Cargo.toml"
    if (Test-Path $cargoToml) {
        $content = Get-Content $cargoToml -Raw
        if ($content -match 'version\s*=\s*"([^"]+)"') {
            return $Matches[1]
        }
    }
    
    return "dev"
}

# =============================================================================
# BUILD FUNCTIONS
# =============================================================================

function Build-Binary {
    param(
        [string]$TargetName,
        [string]$CargoTriple
    )
    
    Write-Step "Building GIK binary for $TargetName ($CargoTriple)"
    Write-Info "Using Docker build via build.ps1 for consistent environment"
    
    # Use build.ps1 which handles Docker-based builds with all dependencies
    # This ensures protobuf, cmake, and other system dependencies are available
    $buildScript = Join-Path $RepoRoot "build.ps1"
    
    if (-not (Test-Path $buildScript)) {
        Write-ErrorAndExit "Build script not found: $buildScript"
    }
    
    # Call build.ps1 to create the binary using Docker
    # Must be called from repo root for relative paths to work
    Write-Info "Running: .\build.ps1 release-windows from $RepoRoot"
    
    $currentDir = Get-Location
    Set-Location $RepoRoot
    try {
        & .\build.ps1 release-windows | Out-Host
        
        if ($LASTEXITCODE -ne 0) {
            Write-ErrorAndExit "Docker build failed. Ensure Docker is running and build.ps1 is accessible."
        }
    }
    finally {
        Set-Location $currentDir
    }
    
    # The build.ps1 script outputs to target/gik.exe (relative to repo root)
    $binaryPath = Join-Path $RepoRoot "target" | Join-Path -ChildPath "gik.exe"
    
    Write-Info "Checking for binary at: $binaryPath"
    if (-not (Test-Path $binaryPath)) {
        Write-ErrorAndExit "Build failed: binary not found at $binaryPath"
    }
    
    $binarySize = [math]::Round((Get-Item $binaryPath).Length / 1MB, 1)
    Write-Success "Binary built via Docker: $binaryPath (${binarySize}MB)"
    
    # Return the absolute path using GetFullPath for reliability
    $absolutePath = [System.IO.Path]::GetFullPath($binaryPath)
    Write-Info "Returning absolute path: $absolutePath"
    
    # Use Write-Output to explicitly return value
    Write-Output $absolutePath
}

# =============================================================================
# STAGING FUNCTIONS
# =============================================================================

function New-StagingDir {
    param([string]$TargetName)
    
    $stagingDir = Join-Path $DistDir "gik-$TargetName"
    
    Write-Step "Creating staging directory: $stagingDir"
    
    # Clean existing staging directory for this target
    if (Test-Path $stagingDir) {
        Write-Info "Cleaning existing staging directory"
        Remove-Item -Recurse -Force $stagingDir
    }
    
    New-Item -ItemType Directory -Path $stagingDir -Force | Out-Null
    New-Item -ItemType Directory -Path (Join-Path $stagingDir "bin") -Force | Out-Null
    New-Item -ItemType Directory -Path (Join-Path $stagingDir "models") -Force | Out-Null
    
    return $stagingDir
}

function Copy-Binary {
    param(
        [string]$BinaryPath,
        [string]$StagingDir
    )
    
    Write-Step "Copying binary"
    
    # Validate binary exists
    if (-not (Test-Path $BinaryPath)) {
        Write-ErrorAndExit "Binary not found at: $BinaryPath"
    }
    
    $binDir = Join-Path $StagingDir "bin"
    if (-not (Test-Path $binDir)) {
        New-Item -ItemType Directory -Path $binDir -Force | Out-Null
    }
    
    $destPath = Join-Path $binDir "gik.exe"
    
    try {
        Copy-Item -Path $BinaryPath -Destination $destPath -Force -ErrorAction Stop
        Write-Success "Binary copied to bin\gik.exe"
    }
    catch {
        Write-ErrorAndExit "Failed to copy binary: $_"
    }
}

function Copy-Models {
    param([string]$StagingDir)
    
    Write-Step "Copying models from $ModelsSource"
    
    if (-not (Test-Path $ModelsSource)) {
        Write-ErrorAndExit "Models directory not found: $ModelsSource"
    }
    
    # Check for actual model files (not just .gitkeep)
    $hasModels = $false
    $embeddingsDir = Join-Path $ModelsSource "embeddings\all-MiniLM-L6-v2"
    if (Test-Path $embeddingsDir) {
        $modelFiles = Get-ChildItem -Path $embeddingsDir -Include "*.safetensors","*.json" -Recurse -ErrorAction SilentlyContinue
        if ($modelFiles) {
            $hasModels = $true
        }
    }
    
    if (-not $hasModels) {
        Write-WarningMessage "Models directory exists but appears empty (only .gitkeep files)"
        Write-WarningMessage "Ensure models are downloaded before creating distribution artifacts"
        Write-Info "Expected: embeddings\all-MiniLM-L6-v2\ and rerankers\ms-marco-MiniLM-L6-v2\"
    }
    
    # Copy models directory structure
    $destModels = Join-Path $StagingDir "models"
    Copy-Item -Path "$ModelsSource\*" -Destination $destModels -Recurse -Force -ErrorAction SilentlyContinue
    
    Write-Success "Models copied to models\"
}

function Copy-Config {
    param([string]$StagingDir)
    
    Write-Step "Copying config template"
    
    if (-not (Test-Path $ConfigSource)) {
        Write-ErrorAndExit "Config file not found: $ConfigSource"
    }
    
    $destPath = Join-Path $StagingDir "config.default.yaml"
    Copy-Item -Path $ConfigSource -Destination $destPath -Force
    
    Write-Success "Config copied as config.default.yaml"
}

function Copy-Docs {
    param([string]$StagingDir)
    
    Write-Step "Copying documentation"
    
    $licensePath = Join-Path $RepoRoot "LICENSE"
    if (Test-Path $licensePath) {
        Copy-Item -Path $licensePath -Destination $StagingDir -Force
        Write-Success "LICENSE copied"
    } else {
        Write-WarningMessage "LICENSE not found at $licensePath"
    }
    
    $readmePath = Join-Path $RepoRoot "README.md"
    if (Test-Path $readmePath) {
        Copy-Item -Path $readmePath -Destination $StagingDir -Force
        Write-Success "README.md copied"
    } else {
        Write-WarningMessage "README.md not found at $readmePath"
    }
}

# =============================================================================
# VALIDATION FUNCTIONS
# =============================================================================

function Test-StagingDir {
    param([string]$StagingDir)
    
    Write-Step "Validating staging directory"
    
    $errors = 0
    
    # Check binary exists
    $binaryPath = Join-Path $StagingDir "bin\gik.exe"
    if (-not (Test-Path $binaryPath)) {
        Write-Host "✗ Binary missing: $binaryPath" -ForegroundColor Red
        $errors++
    } else {
        Write-Success "Binary exists"
    }
    
    # Check models directory exists
    $modelsPath = Join-Path $StagingDir "models"
    if (-not (Test-Path $modelsPath)) {
        Write-Host "✗ Models directory missing: $modelsPath" -ForegroundColor Red
        $errors++
    } else {
        Write-Success "Models directory exists"
    }
    
    # Check config exists
    $configPath = Join-Path $StagingDir "config.default.yaml"
    if (-not (Test-Path $configPath)) {
        Write-Host "✗ Config file missing: $configPath" -ForegroundColor Red
        $errors++
    } else {
        Write-Success "Config file exists"
    }
    
    if ($errors -gt 0) {
        Write-ErrorAndExit "Validation failed with $errors error(s)"
    }
    
    Write-Success "Staging directory validated"
}

# =============================================================================
# ARCHIVE FUNCTIONS
# =============================================================================

function New-Archive {
    param(
        [string]$TargetName,
        [string]$StagingDir
    )
    
    $archiveName = "gik-$TargetName.zip"
    $archivePath = Join-Path $DistDir $archiveName
    
    Write-Step "Creating archive: $archiveName"
    
    # Remove existing archive
    if (Test-Path $archivePath) {
        Remove-Item -Force $archivePath
    }
    
    # Create zip archive
    Compress-Archive -Path $StagingDir -DestinationPath $archivePath -Force
    
    $size = (Get-Item $archivePath).Length
    $sizeFormatted = "{0:N2} MB" -f ($size / 1MB)
    Write-Success "Archive created: $archivePath ($sizeFormatted)"
    
    # List archive contents (first 20 items)
    Write-Info "Archive contents:"
    $zipFile = [System.IO.Compression.ZipFile]::OpenRead($archivePath)
    try {
        $entries = $zipFile.Entries | Select-Object -First 20
        foreach ($entry in $entries) {
            Write-Info "  $($entry.FullName)"
        }
    }
    finally {
        $zipFile.Dispose()
    }
    
    return $archivePath
}

# =============================================================================
# PREREQUISITE CHECKS
# =============================================================================

function Test-Prerequisites {
    Write-Step "Checking prerequisites..."
    
    # Check if Docker is installed
    $dockerInstalled = Get-Command docker -ErrorAction SilentlyContinue
    if (-not $dockerInstalled) {
        Write-Host ""
        Write-Host "=========================================="
        Write-Host "  DOCKER NOT FOUND"
        Write-Host "=========================================="
        Write-Host ""
        Write-ErrorAndExit @"
Docker is required but not installed or not in PATH.

GIK uses Docker-based builds to ensure consistent environments with all
required dependencies (protobuf, cmake, MinGW-w64, etc.).

Direct 'cargo build' will fail with protobuf errors.

INSTALLATION:
  Windows: https://docs.docker.com/desktop/install/windows-install/
  Linux:   https://docs.docker.com/engine/install/
  macOS:   https://docs.docker.com/desktop/install/mac-install/

After installing Docker:
  1. Start Docker Desktop (Windows/macOS) or docker service (Linux)
  2. Verify: docker --version
  3. Run this script again

For more information, see: docs/BUILD-ARCHITECTURE.md
"@
    }
    
    # Check if Docker is running
    try {
        $null = docker version 2>&1
        if ($LASTEXITCODE -ne 0) {
            Write-Host ""
            Write-Host "=========================================="
            Write-Host "  DOCKER NOT RUNNING"
            Write-Host "=========================================="
            Write-Host ""
            Write-ErrorAndExit @"
Docker is installed but not running.

SOLUTION:
  Windows/macOS: Start Docker Desktop
  Linux:         sudo systemctl start docker

Verify with: docker ps
"@
        }
    } catch {
        Write-Host ""
        Write-Host "=========================================="
        Write-Host "  DOCKER NOT RUNNING"
        Write-Host "=========================================="
        Write-Host ""
        Write-ErrorAndExit "Docker is installed but not running. Please start Docker and try again."
    }
    
    Write-Success "Docker is installed and running"
    
    # Check if build.ps1 exists
    $buildScript = Join-Path $RepoRoot "build.ps1"
    if (-not (Test-Path $buildScript)) {
        Write-ErrorAndExit "Build script not found: $buildScript"
    }
    
    Write-Success "Build script found: build.ps1"
}

# =============================================================================
# MAIN
# =============================================================================

function Main {
    # Show help if requested
    if ($Help) {
        Show-Help
    }
    
    # Check prerequisites first
    Test-Prerequisites
    
    # Get configuration
    $cargoTriple = Get-CargoTriple -TargetName $Target
    $gikVersion = Get-GikVersion
    
    Write-Host ""
    Write-Host "=========================================="
    Write-Host "  GIK Artifact Packaging"
    Write-Host "=========================================="
    Write-Host ""
    Write-Info "Target:     $Target"
    Write-Info "Triple:     $cargoTriple"
    Write-Info "Version:    $gikVersion"
    Write-Info "Dist dir:   $DistDir"
    Write-Info "Models:     $ModelsSource"
    Write-Info "Config:     $ConfigSource"
    
    # Ensure dist directory exists
    if (-not (Test-Path $DistDir)) {
        New-Item -ItemType Directory -Path $DistDir -Force | Out-Null
    }
    
    # Build binary
    $binaryPath = Build-Binary -TargetName $Target -CargoTriple $cargoTriple
    
    # Create staging directory
    $stagingDir = New-StagingDir -TargetName $Target
    
    # Copy files to staging
    Copy-Binary -BinaryPath $binaryPath -StagingDir $stagingDir
    Copy-Models -StagingDir $stagingDir
    Copy-Config -StagingDir $stagingDir
    Copy-Docs -StagingDir $stagingDir
    
    # Validate staging directory
    Test-StagingDir -StagingDir $stagingDir
    
    # Create archive
    $archivePath = New-Archive -TargetName $Target -StagingDir $stagingDir
    
    Write-Host ""
    Write-Host "=========================================="
    Write-Host "  Packaging Complete"
    Write-Host "=========================================="
    Write-Host ""
    Write-Success "Artifact: $archivePath"
    Write-Success "Staging:  $stagingDir"
    Write-Host ""
}

# Add required assembly for zip operations
Add-Type -AssemblyName System.IO.Compression.FileSystem

# Run main
Main
