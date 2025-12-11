#!/usr/bin/env pwsh
# =============================================================================
# Test Installation Script - Uses Local Artifact
# =============================================================================
#
# This script tests the installation process using the locally built artifact
# instead of downloading from GitHub. Useful for development and testing.
#
# USAGE:
#   .\scripts\test-install.ps1
#
# =============================================================================

[CmdletBinding()]
param(
    [Parameter()]
    [string]$ArtifactPath = "",
    
    [Parameter()]
    [string]$TestInstallDir = "",
    
    [Parameter()]
    [string]$TestGikHome = ""
)

$ErrorActionPreference = "Stop"

# =============================================================================
# CONFIGURATION
# =============================================================================

$RepoRoot = Split-Path -Parent (Split-Path -Parent $PSCommandPath)
$DefaultArtifact = Join-Path $RepoRoot "dist\gik-windows-x86_64.zip"
$ArtifactToTest = if ($ArtifactPath) { $ArtifactPath } else { $DefaultArtifact }

# Use temp directories for testing
$TestDir = Join-Path $env:TEMP "gik-install-test-$(Get-Random)"
$InstallDir = if ($TestInstallDir) { $TestInstallDir } else { Join-Path $TestDir "install" }
$GikHome = if ($TestGikHome) { $TestGikHome } else { Join-Path $TestDir "gik-home" }

$Target = "windows-x86_64"

# =============================================================================
# HELPER FUNCTIONS
# =============================================================================

function Write-Step {
    param([string]$Message)
    Write-Host "`n==> " -ForegroundColor Cyan -NoNewline
    Write-Host $Message
}

function Write-Success {
    param([string]$Message)
    Write-Host "✓ " -ForegroundColor Green -NoNewline
    Write-Host $Message
}

function Write-Warning {
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

# =============================================================================
# EXTRACTION FUNCTIONS
# =============================================================================

function Expand-Artifact {
    param(
        [string]$Archive,
        [string]$Destination
    )
    
    Write-Step "Extracting archive"
    
    try {
        Expand-Archive -Path $Archive -DestinationPath $Destination -Force
        Write-Success "Extracted to $Destination"
    }
    catch {
        Write-ErrorAndExit "Failed to extract $Archive. Error: $_"
    }
}

function Test-Package {
    param(
        [string]$ExtractDir,
        [string]$Target
    )
    
    Write-Step "Validating package contents"
    
    $PkgDir = Join-Path $ExtractDir "gik-$Target"
    
    if (-not (Test-Path $PkgDir)) {
        Write-ErrorAndExit "Package structure invalid: $PkgDir not found"
    }
    
    # Validate required files
    $BinPath = Join-Path $PkgDir "bin\gik.exe"
    if (-not (Test-Path $BinPath)) {
        Write-ErrorAndExit "Package invalid: bin\gik.exe not found"
    }
    Write-Success "Binary found: bin\gik.exe"
    
    $ModelsPath = Join-Path $PkgDir "models"
    if (-not (Test-Path $ModelsPath)) {
        Write-ErrorAndExit "Package invalid: models\ directory not found"
    }
    Write-Success "Models directory found"
    
    $ConfigPath = Join-Path $PkgDir "config.default.yaml"
    if (-not (Test-Path $ConfigPath)) {
        Write-ErrorAndExit "Package invalid: config.default.yaml not found"
    }
    Write-Success "Config file found"
    
    Write-Success "Package validated"
    return $PkgDir
}

# =============================================================================
# INSTALLATION FUNCTIONS
# =============================================================================

function Install-Binary {
    param(
        [string]$SourceBin,
        [string]$InstallDir
    )
    
    Write-Step "Installing GIK binary to $InstallDir"
    
    try {
        # Create install directory
        if (-not (Test-Path $InstallDir)) {
            New-Item -ItemType Directory -Path $InstallDir -Force | Out-Null
        }
        
        # Copy binary
        $DestBin = Join-Path $InstallDir "gik.exe"
        Copy-Item -Path $SourceBin -Destination $DestBin -Force
        
        Write-Success "Installed gik.exe to $InstallDir"
        return $DestBin
    }
    catch {
        Write-ErrorAndExit "Failed to install binary. Error: $_"
    }
}

function Install-Models {
    param(
        [string]$SourceModels,
        [string]$GikHome
    )
    
    Write-Step "Installing models to $GikHome\models"
    
    $DestModels = Join-Path $GikHome "models"
    
    try {
        # Create models directory
        if (-not (Test-Path $DestModels)) {
            New-Item -ItemType Directory -Path $DestModels -Force | Out-Null
        }
        
        # Copy models (overwrite existing)
        Copy-Item -Path "$SourceModels\*" -Destination $DestModels -Recurse -Force
        
        # Count model files
        $modelFiles = Get-ChildItem $DestModels -Recurse -Filter "*.safetensors"
        Write-Success "Models installed ($($modelFiles.Count) .safetensors files)"
    }
    catch {
        Write-ErrorAndExit "Failed to install models. Error: $_"
    }
}

function Install-Config {
    param(
        [string]$SourceConfig,
        [string]$GikHome
    )
    
    Write-Step "Installing configuration"
    
    $DestConfig = Join-Path $GikHome "config.yaml"
    
    try {
        # Create GIK home directory
        if (-not (Test-Path $GikHome)) {
            New-Item -ItemType Directory -Path $GikHome -Force | Out-Null
        }
        
        # Copy config
        Copy-Item -Path $SourceConfig -Destination $DestConfig
        Write-Success "Created $DestConfig"
    }
    catch {
        Write-ErrorAndExit "Failed to install config. Error: $_"
    }
}

# =============================================================================
# VERIFICATION FUNCTIONS
# =============================================================================

function Test-Installation {
    param(
        [string]$BinaryPath,
        [string]$GikHome
    )
    
    Write-Step "Verifying installation"
    
    # Test binary exists
    if (-not (Test-Path $BinaryPath)) {
        Write-ErrorAndExit "Binary not found: $BinaryPath"
    }
    Write-Success "Binary exists: $BinaryPath"
    
    # Test binary size
    $size = (Get-Item $BinaryPath).Length / 1MB
    Write-Info "Binary size: $([math]::Round($size, 1)) MB"
    
    # Test models exist
    $modelsDir = Join-Path $GikHome "models"
    if (-not (Test-Path $modelsDir)) {
        Write-ErrorAndExit "Models directory not found: $modelsDir"
    }
    
    $embeddingModel = Join-Path $modelsDir "embeddings\all-MiniLM-L6-v2\model.safetensors"
    if (-not (Test-Path $embeddingModel)) {
        Write-ErrorAndExit "Embedding model not found"
    }
    Write-Success "Embedding model found"
    
    $rerankerModel = Join-Path $modelsDir "rerankers\ms-marco-MiniLM-L6-v2\model.safetensors"
    if (-not (Test-Path $rerankerModel)) {
        Write-ErrorAndExit "Reranker model not found"
    }
    Write-Success "Reranker model found"
    
    # Test config exists
    $configFile = Join-Path $GikHome "config.yaml"
    if (-not (Test-Path $configFile)) {
        Write-ErrorAndExit "Config file not found: $configFile"
    }
    Write-Success "Config file found"
    
    # Test binary executes
    Write-Step "Testing binary execution"
    try {
        $output = & $BinaryPath --version 2>&1
        if ($LASTEXITCODE -eq 0) {
            Write-Success "Binary executes successfully"
            Write-Info "Version output: $output"
        } else {
            Write-Warning "Binary execution returned non-zero exit code: $LASTEXITCODE"
            Write-Info "Output: $output"
        }
    }
    catch {
        Write-Warning "Could not execute binary: $_"
    }
}

# =============================================================================
# MAIN
# =============================================================================

function Main {
    Write-Host ""
    Write-Host "=========================================="
    Write-Host "  GIK Installation Test"
    Write-Host "=========================================="
    Write-Host ""
    
    Write-Info "Artifact:    $ArtifactToTest"
    Write-Info "Test Dir:    $TestDir"
    Write-Info "Install Dir: $InstallDir"
    Write-Info "GIK Home:    $GikHome"
    
    # Verify artifact exists
    if (-not (Test-Path $ArtifactToTest)) {
        Write-ErrorAndExit "Artifact not found: $ArtifactToTest. Run .\scripts\gik-build-packages.ps1 first."
    }
    
    # Create test directories
    New-Item -ItemType Directory -Path $TestDir -Force | Out-Null
    
    try {
        # Extract
        $ExtractDir = Join-Path $TestDir "extracted"
        Expand-Artifact -Archive $ArtifactToTest -Destination $ExtractDir
        
        # Validate
        $PkgDir = Test-Package -ExtractDir $ExtractDir -Target $Target
        
        # Install binary
        $BinSource = Join-Path $PkgDir "bin\gik.exe"
        $BinPath = Install-Binary -SourceBin $BinSource -InstallDir $InstallDir
        
        # Install models
        $ModelsSource = Join-Path $PkgDir "models"
        Install-Models -SourceModels $ModelsSource -GikHome $GikHome
        
        # Install config
        $ConfigSource = Join-Path $PkgDir "config.default.yaml"
        Install-Config -SourceConfig $ConfigSource -GikHome $GikHome
        
        # Verify installation
        Test-Installation -BinaryPath $BinPath -GikHome $GikHome
        
        # Summary
        Write-Host ""
        Write-Host "=========================================="
        Write-Host "  Installation Test PASSED!"
        Write-Host "=========================================="
        Write-Host ""
        Write-Success "Binary:   $BinPath"
        Write-Success "GIK Home: $GikHome"
        Write-Host ""
        Write-Info "Test files will be cleaned up on exit"
        Write-Info "To manually inspect: cd $TestDir"
        Write-Host ""
    }
    finally {
        # Cleanup (optional - comment out to keep test files)
        Write-Step "Cleaning up test files"
        if (Test-Path $TestDir) {
            Remove-Item -Path $TestDir -Recurse -Force -ErrorAction SilentlyContinue
            Write-Success "Cleaned up $TestDir"
        }
    }
}

# Run main
Main
