# GIK Release CI Workflow

This document describes the GitHub Actions release workflow for GIK.

## Overview

The release workflow automatically builds and publishes GIK artifacts when a version tag is pushed.

| Trigger | Tag Pattern | Example |
|---------|-------------|---------|
| Tag push | `v*` | `v0.1.0`, `v1.0.0-rc.1`, `v2.0.0-beta.3` |

## Workflow Jobs

### 1. `build-unix`

Builds Linux and macOS artifacts using a matrix strategy.

| Runner | Target | Artifact |
|--------|--------|----------|
| `ubuntu-latest` | `linux-x86_64` | `gik-linux-x86_64.tar.gz` |
| `macos-13` | `macos-x86_64` | `gik-macos-x86_64.tar.gz` |
| `macos-latest` | `macos-aarch64` | `gik-macos-aarch64.tar.gz` |

### 2. `build-windows`

Builds Windows artifact.

| Runner | Target | Artifact |
|--------|--------|----------|
| `windows-latest` | `windows-x86_64` | `gik-windows-x86_64.zip` |

### 3. `publish-release`

Downloads all artifacts and creates a GitHub Release.

- Depends on: `build-unix`, `build-windows`
- Generates SHA256 checksums
- Detects prerelease from tag (e.g., `-rc.1`, `-beta.2`)
- Auto-generates release notes from commits

## Artifacts Produced

| File | Platform | Contents |
|------|----------|----------|
| `gik-linux-x86_64.tar.gz` | Linux x86_64 | Binary + models + config |
| `gik-macos-x86_64.tar.gz` | macOS Intel | Binary + models + config |
| `gik-macos-aarch64.tar.gz` | macOS ARM64 | Binary + models + config |
| `gik-windows-x86_64.zip` | Windows x86_64 | Binary + models + config |
| `SHA256SUMS.txt` | All | Checksums for verification |

## Creating a Release

### Prerequisites

1. Ensure the `main` branch is green (all tests passing)
2. Ensure models are committed via Git LFS
3. Update version in `Cargo.toml` if needed

### Steps

```bash
# 1. Ensure you're on main and up to date
git checkout main
git pull origin main

# 2. Create a version tag
git tag v0.1.0

# 3. Push the tag to trigger the workflow
git push origin v0.1.0
```

### Monitor the Workflow

1. Go to **Actions** tab in GitHub
2. Find the **Release** workflow run
3. Monitor the three jobs: `build-unix`, `build-windows`, `publish-release`
4. Once complete, check **Releases** for the published release

## Testing with Pre-release Tags

Before creating a stable release, test the workflow with a pre-release tag:

```bash
# Create a release candidate tag
git tag v0.1.0-rc.1
git push origin v0.1.0-rc.1
```

Pre-release tags (containing `-`) are automatically marked as "Pre-release" in GitHub.

### Pre-release Examples

| Tag | Type | Marked as Prerelease |
|-----|------|---------------------|
| `v0.1.0` | Stable | No |
| `v0.1.0-rc.1` | Release Candidate | Yes |
| `v0.1.0-beta.1` | Beta | Yes |
| `v0.1.0-alpha.1` | Alpha | Yes |

## Downloading Artifacts

### From GitHub Releases

1. Go to the repository's **Releases** page
2. Find the desired version
3. Download the appropriate artifact for your platform
4. Verify checksum (optional):

```bash
# Download artifact and checksum
curl -LO https://github.com/platformrocks/osr.gik/releases/download/v0.1.0/gik-macos-aarch64.tar.gz
curl -LO https://github.com/platformrocks/osr.gik/releases/download/v0.1.0/SHA256SUMS.txt

# Verify
sha256sum -c SHA256SUMS.txt --ignore-missing
```

## Troubleshooting

### Build Failure: "Cargo.lock not found"

**Cause**: `Cargo.lock` is gitignored.

**Solution**: The packaging scripts handle this automatically, but ensure `cargo build` works locally first.

### Build Failure: "model.safetensors is empty"

**Cause**: Git LFS files weren't pulled.

**Solution**: The workflow includes `git lfs pull`, but ensure LFS is set up:
```bash
git lfs install
git lfs pull
```

### Build Failure: "Target not found"

**Cause**: Rust target triple not installed.

**Solution**: The packaging scripts automatically add the target, but you can verify:
```bash
rustup target list --installed
```

### Publish Failure: "Resource not accessible by integration"

**Cause**: Missing `contents: write` permission.

**Solution**: The workflow includes the required permissions. If using a fork, ensure Actions have write permissions in repository settings.

### Artifact Size is Too Small

**Cause**: Models weren't included (LFS pointers instead of actual files).

**Solution**: Ensure Git LFS is properly configured and files are pulled:
```bash
git lfs status
git lfs pull
```

Expected artifact sizes:
- Each platform: ~190-200 MB (includes ~175 MB of models)

### Release Already Exists

**Cause**: Tag was pushed multiple times or release was created manually.

**Solution**: Delete the existing release and re-push the tag:
```bash
# Delete local and remote tag
git tag -d v0.1.0
git push origin :refs/tags/v0.1.0

# Recreate and push
git tag v0.1.0
git push origin v0.1.0
```

## Workflow File Location

`.github/workflows/gik-release.yml`

## Related Documentation

- [Packaging Scripts Usage](.guided/operation/gik-packages-script.usage.md)
- [Docker Build Guide](README-DOCKER.md)
