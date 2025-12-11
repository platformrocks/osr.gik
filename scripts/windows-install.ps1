#Requires -Version 5.0
<#
.SYNOPSIS
    GIK Install Script for Windows

.DESCRIPTION
    Installs the GIK CLI from GitHub Releases.
    Downloads the Windows artifact, extracts it, and installs:
    - gik.exe to Program Files (or custom location)
    - models to ~/.gik/models/
    - config.yaml to ~/.gik/ (if not exists)

.PARAMETER Version
    Version to install (e.g., v0.1.0). Default: latest

.PARAMETER InstallDir
    Directory to install gik.exe. Default: C:\Program Files\GIK

.PARAMETER GikHome
    GIK home directory for models and config. Default: $env:USERPROFILE\.gik

.PARAMETER Help
    Show this help message

.EXAMPLE
    # Install latest version:
    powershell -ExecutionPolicy Bypass -c "irm https://raw.githubusercontent.com/platformrocks/osr.gik/main/scripts/windows-install.ps1 | iex"

.EXAMPLE
    # Install specific version:
    $env:GIK_VERSION = "v0.1.0"
    powershell -ExecutionPolicy Bypass -c "irm https://raw.githubusercontent.com/platformrocks/osr.gik/main/scripts/windows-install.ps1 | iex"

.NOTES
    Repository: https://github.com/platformrocks/osr.gik
#>

[CmdletBinding()]
param(
    [Parameter()]
    [string]$Version = '',
    
    [Parameter()]
    [string]$InstallDir = '',
    
    [Parameter()]
    [string]$GikHome = '',
    
    [Parameter()]
    [switch]$Help
)

# =============================================================================
# CONFIGURATION
# =============================================================================

$ErrorActionPreference = "Stop"

# Resolve from environment variables if parameters not provided
$GikRepo = if ($env:GIK_REPO) { $env:GIK_REPO } else { "platformrocks/osr.gik" }
$GikVersion = if ($Version) { $Version } elseif ($env:GIK_VERSION) { $env:GIK_VERSION } else { "latest" }
$GikInstallDir = if ($InstallDir) { $InstallDir } elseif ($env:GIK_INSTALL_DIR) { $env:GIK_INSTALL_DIR } else { "$env:ProgramFiles\GIK" }
$GikHomeDir = if ($GikHome) { $GikHome } elseif ($env:GIK_HOME) { $env:GIK_HOME } else { "$env:USERPROFILE\.gik" }

# Target
$Target = "windows-x86_64"
$ArtifactName = "gik-$Target.zip"

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

function Show-Help {
    Write-Host @"

GIK Install Script for Windows

USAGE:
  powershell -ExecutionPolicy Bypass -c "irm https://raw.githubusercontent.com/platformrocks/osr.gik/main/scripts/windows-install.ps1 | iex"

  # Install specific version:
  `$env:GIK_VERSION = "v0.1.0"
  powershell -ExecutionPolicy Bypass -c "irm ... | iex"

ENVIRONMENT VARIABLES:
  GIK_REPO        GitHub repository (default: platformrocks/osr.gik)
  GIK_VERSION     Version to install (default: latest)
  GIK_INSTALL_DIR Custom install directory (default: C:\Program Files\GIK)
  GIK_HOME        GIK home directory (default: %USERPROFILE%\.gik)

WHAT GETS INSTALLED:
  - gik.exe       → C:\Program Files\GIK\gik.exe
  - models        → %USERPROFILE%\.gik\models\
  - config        → %USERPROFILE%\.gik\config.yaml (created if not exists)

"@
    exit 0
}

# =============================================================================
# DOWNLOAD FUNCTIONS
# =============================================================================

function Get-DownloadUrl {
    param(
        [string]$Repo,
        [string]$Version,
        [string]$Artifact
    )
    
    if ($Version -eq "latest") {
        return "https://github.com/$Repo/releases/latest/download/$Artifact"
    } else {
        return "https://github.com/$Repo/releases/download/$Version/$Artifact"
    }
}

function Get-Artifact {
    param(
        [string]$Url,
        [string]$OutputPath
    )
    
    Write-Step "Downloading GIK from $Url"
    
    try {
        # Use TLS 1.2
        [Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12
        
        $ProgressPreference = 'SilentlyContinue'
        Invoke-WebRequest -Uri $Url -OutFile $OutputPath -UseBasicParsing
        $ProgressPreference = 'Continue'
        
        Write-Success "Downloaded $(Split-Path $OutputPath -Leaf)"
    }
    catch {
        Write-ErrorAndExit "Failed to download from $Url. Check your network connection and verify the version exists. Error: $_"
    }
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
    
    # Find package directory if exact match not found
    if (-not (Test-Path $PkgDir)) {
        $PkgDir = Get-ChildItem -Path $ExtractDir -Directory -Filter "gik-*" | Select-Object -First 1 -ExpandProperty FullName
        if (-not $PkgDir) {
            Write-ErrorAndExit "Package structure invalid: no gik-* directory found"
        }
    }
    
    # Validate required files
    $BinPath = Join-Path $PkgDir "bin\gik.exe"
    if (-not (Test-Path $BinPath)) {
        Write-ErrorAndExit "Package invalid: bin\gik.exe not found"
    }
    
    $ModelsPath = Join-Path $PkgDir "models"
    if (-not (Test-Path $ModelsPath)) {
        Write-ErrorAndExit "Package invalid: models\ directory not found"
    }
    
    $ConfigPath = Join-Path $PkgDir "config.default.yaml"
    if (-not (Test-Path $ConfigPath)) {
        Write-ErrorAndExit "Package invalid: config.default.yaml not found"
    }
    
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
        
        # Update PATH if needed
        $UserPath = [Environment]::GetEnvironmentVariable("Path", "User")
        if ($UserPath -notlike "*$InstallDir*") {
            Write-Step "Adding $InstallDir to user PATH"
            $NewPath = "$UserPath;$InstallDir"
            [Environment]::SetEnvironmentVariable("Path", $NewPath, "User")
            Write-Success "Updated user PATH"
            Write-Warning "You need to open a new terminal for PATH changes to take effect"
        } else {
            Write-Info "$InstallDir is already in PATH"
        }
        
        return $DestBin
    }
    catch {
        # Try alternative location if Program Files fails
        if ($InstallDir -like "*Program Files*") {
            Write-Warning "Cannot write to $InstallDir (may need admin). Trying alternative..."
            $AltDir = "$env:USERPROFILE\.local\bin"
            return Install-Binary -SourceBin $SourceBin -InstallDir $AltDir
        }
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
        
        Write-Success "Models installed"
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
        
        # Copy config only if it doesn't exist
        if (Test-Path $DestConfig) {
            Write-Info "Existing config.yaml found, keeping it unchanged"
        } else {
            Copy-Item -Path $SourceConfig -Destination $DestConfig
            Write-Success "Created $DestConfig"
        }
    }
    catch {
        Write-ErrorAndExit "Failed to install config. Error: $_"
    }
}

# =============================================================================
# MAIN
# =============================================================================

function Main {
    # Show help if requested
    if ($Help) {
        Show-Help
    }
    
    Write-Host ""
    Write-Host "=========================================="
    Write-Host "  GIK Installer for Windows"
    Write-Host "=========================================="
    Write-Host ""
    
    Write-Info "Repository:  $GikRepo"
    Write-Info "Version:     $GikVersion"
    Write-Info "Install Dir: $GikInstallDir"
    Write-Info "GIK Home:    $GikHomeDir"
    
    # Create temporary directory
    $TmpDir = Join-Path $env:TEMP "gik-install-$(Get-Random)"
    New-Item -ItemType Directory -Path $TmpDir -Force | Out-Null
    
    try {
        # Build download URL
        $Url = Get-DownloadUrl -Repo $GikRepo -Version $GikVersion -Artifact $ArtifactName
        
        # Download
        $ArchivePath = Join-Path $TmpDir $ArtifactName
        Get-Artifact -Url $Url -OutputPath $ArchivePath
        
        # Extract
        $ExtractDir = Join-Path $TmpDir "extracted"
        Expand-Artifact -Archive $ArchivePath -Destination $ExtractDir
        
        # Validate
        $PkgDir = Test-Package -ExtractDir $ExtractDir -Target $Target
        
        # Install binary
        $BinSource = Join-Path $PkgDir "bin\gik.exe"
        $BinPath = Install-Binary -SourceBin $BinSource -InstallDir $GikInstallDir
        
        # Install models
        $ModelsSource = Join-Path $PkgDir "models"
        Install-Models -SourceModels $ModelsSource -GikHome $GikHomeDir
        
        # Install config
        $ConfigSource = Join-Path $PkgDir "config.default.yaml"
        Install-Config -SourceConfig $ConfigSource -GikHome $GikHomeDir
        
        # Summary
        Write-Host ""
        Write-Host "=========================================="
        Write-Host "  Installation Complete!"
        Write-Host "=========================================="
        Write-Host ""
        Write-Success "Binary:   $BinPath"
        Write-Success "GIK Home: $GikHomeDir"
        Write-Host ""
        Write-Info "Open a NEW terminal and verify installation:"
        Write-Info "  gik --version"
        Write-Host ""
    }
    finally {
        # Cleanup
        if (Test-Path $TmpDir) {
            Remove-Item -Path $TmpDir -Recurse -Force -ErrorAction SilentlyContinue
        }
    }
}

# Run main
Main
