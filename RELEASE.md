# Release Process

This document describes the step-by-step process for creating and publishing a new GIK release.

## Prerequisites

- [ ] Write access to the repository
- [ ] Docker Desktop installed and running (for building artifacts)
- [ ] All changes merged to `main` branch
- [ ] All tests passing

## Release Steps

### 1. Update Version Numbers

Update the version in the workspace `Cargo.toml`:

```bash
# Edit Cargo.toml
# Change version from current (e.g., 0.1.2) to new version (e.g., 0.1.3)
```

**File:** `Cargo.toml`
```toml
[workspace.package]
version = "0.1.3"  # Update this line
```

All crates (`gik-cli`, `gik-core`, `gik-db`, `gik-model`, `gik-utils`) inherit this version via `version.workspace = true`.

### 2. Update Windows Resource File

Update the version information in the Windows executable metadata:

**File:** `crates/gik-cli/resources/windows/gik.rc`
```c
#define VER_FILEVERSION             0,1,3,0    // Update
#define VER_FILEVERSION_STR         "0.1.3.0"  // Update

#define VER_PRODUCTVERSION          0,1,3,0    // Update
#define VER_PRODUCTVERSION_STR      "0.1.3"    // Update
```

### 3. Update CHANGELOG.md

Add a new section at the top of `CHANGELOG.md` following [Keep a Changelog](https://keepachangelog.com/) format:

```markdown
## [0.1.3] - YYYY-MM-DD

### Added
- New feature descriptions

### Changed
- Changes to existing functionality

### Fixed
- Bug fixes

### Deprecated
- Soon-to-be removed features

### Removed
- Removed features

### Security
- Security fixes
```

**Categories:**
- `Added` for new features
- `Changed` for changes in existing functionality
- `Deprecated` for soon-to-be removed features
- `Removed` for now removed features
- `Fixed` for any bug fixes
- `Security` for security fixes

### 4. Commit Version Changes

```bash
git add Cargo.toml crates/gik-cli/resources/windows/gik.rc CHANGELOG.md
git commit -m "chore: bump version to 0.1.3"
```

### 5. Create Annotated Git Tag

```bash
git tag -a v0.1.3 -m "Release v0.1.3

Brief summary of changes in this release.
Highlight major features, fixes, or improvements."
```

### 6. Push Changes and Tag

```bash
git push origin main
git push origin v0.1.3
```

### 7. Build Release Artifacts

#### Windows (PowerShell)

```powershell
# Build Windows x86_64 package
.\scripts\gik-build-packages.ps1
```

Output: `dist/gik-windows-x86_64.zip` (~194 MB)

#### Linux/macOS (Bash)

```bash
# Build Linux x86_64 (requires Docker)
TARGET=linux-x86_64 ./scripts/gik-build-packages.sh

# Build macOS ARM64 (Apple Silicon, native build)
TARGET=macos-aarch64 ./scripts/gik-build-packages.sh

# macOS x86_64 (Intel) - use GitHub Actions or Intel Mac
# Cross-compilation from ARM64 fails due to linker issues
```

Outputs:
- `dist/gik-linux-x86_64.tar.gz`
- `dist/gik-macos-aarch64.tar.gz`
- `dist/gik-macos-x86_64.tar.gz` (via GitHub Actions)

### 8. Calculate SHA256 Checksums

```bash
# Linux/macOS
shasum -a 256 dist/*.tar.gz dist/*.zip > dist/checksums.txt

# Windows (PowerShell)
Get-ChildItem dist/*.tar.gz, dist/*.zip | Get-FileHash -Algorithm SHA256 | Format-Table Hash, @{Label="File";Expression={$_.Path | Split-Path -Leaf}} | Out-File dist/checksums.txt
```

### 9. Test Artifacts

Before publishing, verify each artifact:

```bash
# Extract and test
tar -xzf dist/gik-linux-x86_64.tar.gz
./gik-linux-x86_64/bin/gik --version
# Expected: gik 0.1.3 (git-hash)

# Test basic functionality
./gik-linux-x86_64/bin/gik init
./gik-linux-x86_64/bin/gik status
```

Verify Windows executable properties:
- Right-click `gik.exe` → Properties → Details
- Check: Version, Product Name, Copyright, Company

### 10. Create GitHub Release

1. Go to https://github.com/platformrocks/osr.gik/releases/new
2. Select the tag: `v0.1.3`
3. Fill in release details:

**Release Title:**
```
v0.1.3 - Brief Title Describing Main Change
```

**Release Notes Template:**
```markdown
## What's Changed

### Features
- **Feature Name**: Description of what it does and why it matters

### Bug Fixes
- **Issue Description**: What was fixed and the impact

### Documentation
- Changes to documentation

### Performance
- Performance improvements

### Maintenance
- Internal improvements and refactoring

---

## Installation

Download the appropriate binary for your platform from the assets below, or use the installation script:

\`\`\`bash
# Linux/macOS
curl -fsSL https://raw.githubusercontent.com/platformrocks/osr.gik/main/scripts/install.sh | bash

# Windows (PowerShell)
irm https://raw.githubusercontent.com/platformrocks/osr.gik/main/scripts/windows-install.ps1 | iex
\`\`\`

## Checksums

See `checksums.txt` for SHA256 hashes of all artifacts.

**Full Changelog**: https://github.com/platformrocks/osr.gik/compare/v0.1.2...v0.1.3
```

4. **Attach artifacts:**
   - Drag and drop all files from `dist/`:
     - `gik-windows-x86_64.zip`
     - `gik-linux-x86_64.tar.gz`
     - `gik-macos-aarch64.tar.gz`
     - `gik-macos-x86_64.tar.gz` (if available)
     - `checksums.txt`

5. **Set as latest release:** Check "Set as the latest release"

6. **Publish release**

### 11. Verify Release

1. Check release page: https://github.com/platformrocks/osr.gik/releases
2. Download an artifact and verify SHA256 matches
3. Test installation scripts with the new release

### 12. Update Installation Instructions (if needed)

If there are breaking changes or new installation requirements, update:
- `README.md`
- `CONTRIBUTING.md`
- Documentation in `docs/`

## Hotfix Release Process

For urgent bug fixes:

1. Create a branch from the release tag: `git checkout -b hotfix/0.1.3 v0.1.2`
2. Make the minimal fix
3. Update version to a patch increment: `0.1.2` → `0.1.2-patch.1` or `0.1.3`
4. Update CHANGELOG.md with the fix
5. Follow steps 4-11 above
6. Merge hotfix back to main: `git checkout main && git merge hotfix/0.1.3`

## Troubleshooting

### Docker Build Fails

```bash
# Clean Docker cache
docker system prune -a

# Restart Docker Desktop
# Retry build
```

### Cross-compilation Errors

For macOS Intel builds, use GitHub Actions:
- Push the tag to trigger the workflow
- Download artifacts from the Actions page

### Missing Models

Ensure models are present in `vendor/models/`:
```bash
./scripts/gik-download-models.sh
```

### Version Mismatch

If `gik --version` shows wrong version:
- Verify `Cargo.toml` was updated
- Rebuild: `cargo clean && cargo build --release`
- Check binary was replaced: `which gik` (Unix) or `where gik` (Windows)

## Rollback

If a release has critical issues:

1. Mark the release as "Pre-release" on GitHub
2. Create a new hotfix release
3. Update documentation to reference the working version
4. Consider yanking the problematic version if published to crates.io

## Automation (Future)

Consider automating with GitHub Actions:
- Trigger on tag push
- Build all platforms
- Generate checksums
- Create draft release
- Upload artifacts

See `.github/workflows/` for existing automation.
