# Release Process

This document describes how to create a new release of IvoryValley.

## Prerequisites

### GitHub Repository Secrets

The following secret must be configured in the GitHub repository settings:

- `CRATES_IO_TOKEN`: API token for publishing to crates.io
  - Generate at: https://crates.io/settings/tokens
  - Required permissions: publish-new, publish-update

## Creating a Release

### 1. Update Version

Update the version in `Cargo.toml`:

```toml
[package]
version = "X.Y.Z"
```

Commit and push the version bump:

```bash
git add Cargo.toml
git commit -m "chore: Bump version to X.Y.Z"
git push
```

### 2. Create GitHub Release

1. Go to [Releases](https://github.com/jensens/ivoryvalley/releases)
2. Click "Create a new release"
3. Create a new tag (e.g., `v0.1.0`) or select an existing one
4. Set the release title (e.g., "v0.1.0")
5. Write release notes describing changes
6. For pre-releases (alpha, beta, rc), check "Set as a pre-release"
7. Click "Publish release"

### 3. Automated Build Process

Once the release is published, GitHub Actions automatically:

1. **Builds binaries** for all supported platforms:
   - Linux x86_64 (`x86_64-unknown-linux-gnu`)
   - Linux ARM64 (`aarch64-unknown-linux-gnu`)
   - macOS x86_64 (`x86_64-apple-darwin`)
   - macOS ARM64 Apple Silicon (`aarch64-apple-darwin`)
   - Windows x86_64 (`x86_64-pc-windows-msvc`)

2. **Uploads binaries** to the GitHub Release as downloadable assets

3. **Publishes to crates.io** after all builds complete

## Release Artifacts

After the workflow completes, the release will contain:

| Platform | Archive |
|----------|---------|
| Linux x86_64 | `ivoryvalley-X.Y.Z-x86_64-unknown-linux-gnu.tar.gz` |
| Linux ARM64 | `ivoryvalley-X.Y.Z-aarch64-unknown-linux-gnu.tar.gz` |
| macOS x86_64 | `ivoryvalley-X.Y.Z-x86_64-apple-darwin.tar.gz` |
| macOS ARM64 | `ivoryvalley-X.Y.Z-aarch64-apple-darwin.tar.gz` |
| Windows x86_64 | `ivoryvalley-X.Y.Z-x86_64-pc-windows-msvc.zip` |

## Version Numbering

This project follows [Semantic Versioning](https://semver.org/):

- **MAJOR** (X.0.0): Incompatible API/config changes
- **MINOR** (0.X.0): New features, backwards compatible
- **PATCH** (0.0.X): Bug fixes, backwards compatible

Pre-release versions use suffixes:
- `X.Y.Z-alpha.N`: Early development
- `X.Y.Z-beta.N`: Feature complete, testing
- `X.Y.Z-rc.N`: Release candidate

## Troubleshooting

### Build Failures

Check the [Actions tab](https://github.com/jensens/ivoryvalley/actions) for workflow logs.

Common issues:
- **Cross-compilation failure**: ARM64 Linux builds use `cross`. Check Docker availability.
- **Missing target**: Ensure the Rust target is installed in the workflow.

### crates.io Publish Failure

- Verify `CRATES_IO_TOKEN` secret is set and not expired
- Ensure version in `Cargo.toml` is higher than the published version
- Check that all required metadata is present in `Cargo.toml`

### Manual Publishing

If automated publishing fails, you can publish manually:

```bash
cargo publish --token $CRATES_IO_TOKEN
```

## Installation from Release

Users can install from various sources:

### From crates.io

```bash
cargo install ivoryvalley
```

### From GitHub Releases

Download the appropriate archive for your platform and extract:

```bash
# Linux/macOS
tar xzf ivoryvalley-X.Y.Z-<target>.tar.gz
./ivoryvalley --help

# Windows
unzip ivoryvalley-X.Y.Z-x86_64-pc-windows-msvc.zip
ivoryvalley.exe --help
```

### From Source

```bash
git clone https://github.com/jensens/ivoryvalley.git
cd ivoryvalley
cargo build --release
```
