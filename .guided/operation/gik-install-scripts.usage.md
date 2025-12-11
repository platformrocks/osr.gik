# GIK Install Scripts Usage Guide

This document describes how to install GIK using the official install scripts.

## Quick Install

### Linux / macOS

```bash
curl -fsSL https://raw.githubusercontent.com/platformrocks/osr.gik/main/scripts/install.sh | bash
```

### Windows (PowerShell)

```powershell
powershell -ExecutionPolicy Bypass -c "irm https://raw.githubusercontent.com/platformrocks/osr.gik/main/scripts/windows-install.ps1 | iex"
```

---

## What Gets Installed

| Component | Linux/macOS | Windows |
|-----------|-------------|---------|
| Binary | `/usr/local/bin/gik` or `~/.local/bin/gik` | `C:\Program Files\GIK\gik.exe` |
| Models | `~/.gik/models/` | `%USERPROFILE%\.gik\models\` |
| Config | `~/.gik/config.yaml` | `%USERPROFILE%\.gik\config.yaml` |

**Note**: The config file is only created if it doesn't already exist. Existing configurations are preserved.

---

## Installing a Specific Version

### Linux / macOS

```bash
# Using environment variable
curl -fsSL https://raw.githubusercontent.com/platformrocks/osr.gik/main/scripts/install.sh | GIK_VERSION=v0.1.0 bash
```

### Windows

```powershell
# Using environment variable
$env:GIK_VERSION = "v0.1.0"
powershell -ExecutionPolicy Bypass -c "irm https://raw.githubusercontent.com/platformrocks/osr.gik/main/scripts/windows-install.ps1 | iex"
```

---

## Environment Variables

### Linux / macOS

| Variable | Default | Description |
|----------|---------|-------------|
| `GIK_REPO` | `platformrocks/osr.gik` | GitHub repository |
| `GIK_VERSION` | `latest` | Version to install (e.g., `v0.1.0`) |
| `GIK_INSTALL_BIN` | Auto-detect | Custom binary install path |
| `GIK_HOME` | `~/.gik` | GIK home directory |

### Windows

| Variable | Default | Description |
|----------|---------|-------------|
| `GIK_REPO` | `platformrocks/osr.gik` | GitHub repository |
| `GIK_VERSION` | `latest` | Version to install (e.g., `v0.1.0`) |
| `GIK_INSTALL_DIR` | `C:\Program Files\GIK` | Binary install directory |
| `GIK_HOME` | `%USERPROFILE%\.gik` | GIK home directory |

---

## Custom Install Locations

### Linux / macOS

```bash
# Install binary to custom location
curl -fsSL ... | GIK_INSTALL_BIN=/opt/gik/bin/gik bash

# Custom GIK home directory
curl -fsSL ... | GIK_HOME=/opt/gik bash

# Both
curl -fsSL ... | GIK_INSTALL_BIN=/opt/bin/gik GIK_HOME=/opt/gik bash
```

### Windows

```powershell
# Custom install directory
$env:GIK_INSTALL_DIR = "D:\Tools\GIK"
powershell -ExecutionPolicy Bypass -c "irm ... | iex"

# Custom GIK home
$env:GIK_HOME = "D:\GIK"
powershell -ExecutionPolicy Bypass -c "irm ... | iex"
```

---

## Binary Install Location Logic

### Linux / macOS

The install script tries these locations in order:

1. **Custom path**: If `GIK_INSTALL_BIN` is set, use that path
2. **System path**: `/usr/local/bin/gik` (if writable or sudo available)
3. **User path**: `~/.local/bin/gik` (fallback)

### Windows

1. **Custom path**: If `GIK_INSTALL_DIR` is set, use that directory
2. **Program Files**: `C:\Program Files\GIK\gik.exe` (default)
3. **User path**: `%USERPROFILE%\.local\bin\gik.exe` (fallback if no admin)

---

## Verifying Installation

After installation, open a new terminal and run:

```bash
gik --version
```

Expected output:
```
gik 0.1.0
```

---

## Troubleshooting

### Permission Issues (Linux/macOS)

**Symptom**: "Permission denied" when installing to `/usr/local/bin`

**Solution**: The script will automatically fall back to `~/.local/bin`. Alternatively:

```bash
# Use sudo explicitly
curl -fsSL ... | sudo bash

# Or specify a custom location
curl -fsSL ... | GIK_INSTALL_BIN="$HOME/.local/bin/gik" bash
```

### Permission Issues (Windows)

**Symptom**: "Cannot write to Program Files"

**Solution**: Run PowerShell as Administrator, or the script will fall back to `%USERPROFILE%\.local\bin`.

### PATH Not Updated

**Symptom**: `gik: command not found` after installation

**Solutions**:

1. **Open a new terminal** - PATH changes only take effect in new terminals

2. **Add to PATH manually** (Linux/macOS):
   ```bash
   # Add to ~/.bashrc or ~/.zshrc
   export PATH="$HOME/.local/bin:$PATH"
   
   # Then reload
   source ~/.bashrc
   ```

3. **Add to PATH manually** (Windows):
   - Open System Properties → Environment Variables
   - Edit user PATH and add the install directory

### Download Failures

**Symptom**: "Failed to download" error

**Possible causes**:

1. **Network issues**: Check your internet connection
2. **Proxy**: Configure proxy settings
3. **Version doesn't exist**: Verify the version tag exists on GitHub

**Debug**:
```bash
# Test download URL manually
curl -I https://github.com/platformrocks/osr.gik/releases/latest/download/gik-linux-x86_64.tar.gz
```

### Missing Tools

**Linux/macOS requirements**:
- `curl` - For downloading
- `tar` - For extracting
- `bash` - Shell

Install missing tools:
```bash
# Debian/Ubuntu
sudo apt-get install curl tar

# macOS (usually pre-installed)
# If needed: brew install curl
```

**Windows requirements**:
- PowerShell 5.0+ (included in Windows 10+)
- `Expand-Archive` cmdlet (included in PowerShell 5.0+)

### Corrupt Download

**Symptom**: "Failed to extract" error

**Solution**: The download may have been interrupted. Try again:

```bash
# Clear any cached files and retry
rm -rf /tmp/gik-*
curl -fsSL ... | bash
```

### Unsupported Platform

**Symptom**: "Unsupported OS/architecture" error

**Supported platforms**:
- Linux x86_64
- macOS x86_64 (Intel)
- macOS aarch64 (Apple Silicon)
- Windows x86_64

**Not yet supported**:
- Linux aarch64 (ARM)
- Windows ARM

---

## Uninstalling GIK

### Linux / macOS

```bash
# Remove binary
rm -f /usr/local/bin/gik
# or
rm -f ~/.local/bin/gik

# Remove GIK home (including models and config)
rm -rf ~/.gik
```

### Windows

```powershell
# Remove binary directory
Remove-Item -Recurse -Force "C:\Program Files\GIK"

# Remove GIK home
Remove-Item -Recurse -Force "$env:USERPROFILE\.gik"

# Remove from PATH (manual step in System Properties)
```

---

## Upgrading GIK

To upgrade to the latest version, simply run the install script again:

```bash
# Linux/macOS
curl -fsSL https://raw.githubusercontent.com/platformrocks/osr.gik/main/scripts/install.sh | bash

# Windows
powershell -ExecutionPolicy Bypass -c "irm https://raw.githubusercontent.com/platformrocks/osr.gik/main/scripts/windows-install.ps1 | iex"
```

The script will:
- ✅ Overwrite the existing binary
- ✅ Update models
- ✅ Preserve your existing `config.yaml`

---

## Script Locations

| Script | URL |
|--------|-----|
| Linux/macOS | `https://raw.githubusercontent.com/platformrocks/osr.gik/main/scripts/install.sh` |
| Windows | `https://raw.githubusercontent.com/platformrocks/osr.gik/main/scripts/windows-install.ps1` |

## Related Documentation

- [Packaging Scripts Usage](.guided/operation/gik-packages-script.usage.md)
- [Release CI Workflow](.guided/operation/gik-release-ci.usage.md)
