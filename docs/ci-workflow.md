# GitHub Actions / CI Workflow

This document describes the complete continuous integration pipeline for `rouser` as defined in `.github/workflows/ci.yml`.

## Triggers

The workflow is triggered on three events:

| Event | Jobs Run | Purpose |
|-------|----------|---------|
| Push to `main` | CI lint/test, debug builds | Validate every commit before merge |
| Pull request to `main` | CI lint/test, debug builds | Gate merges behind passing checks |
| Release published (`types: [published]`) | Full pipeline + release artifacts | Build and publish distributable packages |

## Job Groups

### 1. `ci-lint-test` — Quality Gate (non-release only)

Runs on every push/PR to ensure code quality before merging. All steps must pass; failure blocks merges.

| Step | Command | What It Checks |
|------|---------|----------------|
| Format check | `cargo fmt --check` | Code is consistently formatted |
| Clippy lint | `cargo clippy --all-targets -- -D warnings` | Zero lint warnings allowed (strict mode) |
| Unit tests | `cargo test --all-targets` | All unit and integration tests pass |

**Runner**: `ubuntu-latest`, Rust stable toolchain, target `x86_64-unknown-linux-gnu`. Dependencies: `libsystemd-dev`, `libdbus-1-dev`.

### 2. `build-artifacts-ci` — Debug Builds (non-release only)

Builds debug binaries for both architectures and uploads them as GHA artifacts (7-day retention). These are **not** release assets — they're CI run backups useful for ad-hoc testing PR builds.

| Matrix | Target Triple | Arch | Artifact Name |
|--------|--------------|------|---------------|
| x86_64 | `x86_64-unknown-linux-gnu` | `x86_64` | `rouser-x86_64-linux-ci` |
| aarch64 | `aarch64-unknown-linux-gnu` | `aarch64` | `rouser-aarch64-linux-ci` |

Cross-compilation uses [`cross-rs`](https://github.com/cross-rs/cross) via the `taiki-e/install-action@v2` tool installer for aarch64. x86_64 builds with native `cargo build --release`.

### 3. `release-tarballs` — Release Binary Tarballs (release only)

Runs exclusively on release publish (`github.event_name == 'release'`). Produces versioned tarballs uploaded as GitHub Release assets and GHA artifact backups (30-day retention).

| Matrix | Target Triple | Arch | Archive Name |
|--------|--------------|------|-------------|
| x86_64 | `x86_64-unknown-linux-gnu` | `x86_64` | `rouser-v{VERSION}-linux-x86_64.tar.gz` |
| aarch64 | `aarch64-unknown-linux-gnu` | `aarch64` | `rouser-v{VERSION}-linux-aarch64.tar.gz` |

The version is extracted from the git tag via `${GITHUB_REF#refs/tags/v}` (e.g., `v0.1.0` → `0.1.0`). Tarballs are created by `.github/scripts/packaging.sh package` which bundles binary + default config + systemd service file. Assets are uploaded with `--clobber` to replace any previous upload of the same name.

### 4. `package-deb` — DEB Package (release only)

Depends on `release-tarballs`. For each architecture:
1. Downloads release tarball artifacts from step 3
2. Extracts binary + config into a staging directory via `.github/scripts/packaging.sh extract`
3. Installs `dpkg-dev`, `fakeroot` for native DEB building
4. Runs `.github/scripts/deb.sh build` which constructs a minimal Debian control tree and runs `dpkg-deb --build`

| Arch | deb_arch | Output File |
|------|----------|-------------|
| x86_64 | `amd64` | `rouser-v{VERSION}-x86_64.deb` |
| aarch64 | `arm64` | `rouser-v{VERSION}-aarch64.deb` |

The DEB control file declares dependencies on `systemd, libdbus-1-0`. The package installs the binary to `/usr/local/bin/`, config to `/etc/rouser/config.toml`, and service files to `/lib/systemd/system/`.

### 5. `package-rpm` — RPM Package (release only)

Depends on `release-tarballs`. Runs inside a containerized build environment (`fedora:latest`) for clean dependency resolution. For each architecture:
1. Installs RPM build dependencies via `dnf install`: `rpm-build`, `redhat-rpm-config`, `systemd-devel`, `dbus-devel`, `gcc`, `make`, `tar`, `gzip`
2. Downloads and extracts release tarballs
3. Runs `.github/scripts/rpm.sh build` which constructs an RPM spec file inline, builds the package with `rpmbuild`, and outputs to `/root/rpmbuild/RPMS/`

| Arch | rpm_arch | Output File Pattern |
|------|----------|-------------------|
| x86_64 | `x86_64` | `rouser-v{VERSION}-x86_64.rpm` (found via glob) |
| aarch64 | `aarch64` | `rouser-v{VERSION}-aarch64.rpm` |

RPM assets are uploaded with a fallback search (`find . -maxdepth 2`) since rpmbuild nests files in architecture subdirectories under `RPMS/`. A GHA artifact backup is also created for safety.

### 6. `package-arch-pkgbuild` — Arch PKGBUILD (release only)

Depends on `release-tarballs`. Generates a `PKGBUILD` that references all release tarball source URLs via GitHub API, then archives it:
1. Downloads metadata for all release assets using the GitHub REST API (`gh api`)
2. Runs `.github/scripts/pkgbuild.sh generate PKGBUILD {VERSION}` to produce a valid Arch Linux package build file
3. Creates `rouser-pkgbuild-v{VERSION}.tar.gz` containing just the `PKGBUILD`

The resulting PKGBUILD is uploaded as a GitHub Release asset for manual `makepkg` builds, and backed up as a GHA artifact.

## Artifacts Summary

| Artifact Type | Retention | Uploaded Where? |
|--------------|-----------|-----------------|
| CI tarballs (debug) | 7 days | GHA artifacts only — not release assets |
| Release tarballs (.tar.gz) | Permanent via GitHub Releases + 30-day GHA backup | Both release page and GHA artifact storage |
| DEB packages (.deb) | Permanent via GitHub Releases + 30-day GHA backup | Both release page and GHA artifact storage |
| RPM packages (.rpm) | Permanent via GitHub Releases + 30-day GHA backup | Both release page and GHA artifact storage |
| Arch PKGBUILD archive | Permanent via GitHub Releases + 30-day GHA backup | Both release page and GHA artifact storage |

## Build Matrix Summary

| Job | Triggers On | Runs On | Cross-Compile? | Outputs |
|-----|-----------|---------|----------------|---------|
| `ci-lint-test` | push/PR | ubuntu-latest (x86_64) | No | Pass/fail gate |
| `build-artifacts-ci` | push/PR | ubuntu-latest (x86_64 + aarch64 cross) | Yes (aarch64 via cross-rs) | GHA debug artifacts |
| `release-tarballs` | release tag only | ubuntu-latest (x86_64 + aarch64 cross) | Yes (aarch64 via cross-rs) | Release tarball assets |
| `package-deb` | release tag only | ubuntu-latest | No (native per arch from extracted binary) | DEB assets |
| `package-rpm` | release tag only | fedora:latest container | No (native per arch from extracted binary) | RPM assets |
| `package-arch-pkgbuild` | release tag only | ubuntu-latest | N/A (source archive, not compiled) | PKGBUILD tarball |

## Script Reference

Three helper scripts in `.github/scripts/`:

| Script | Purpose | Functions |
|--------|---------|-----------|
| `packaging.sh` | Tarball packaging and extraction | `package <binary> <output.tar.gz>` — bundles binary + config + service; `extract <tarball> [dest]` — unpacks for DEB/RPM building |
| `deb.sh` | Native DEB package construction | `build <source-dir> <output.deb> <version> <arch>` — builds a minimal Debian control tree and runs dpkg-deb |
| `rpm.sh` | RPM spec generation and build | `build <source-dir> <version> <arch>` — creates an inline .spec file, populates the rpmbuild directory structure, runs rpmbuild |

## Failure Modes

- **Lint/test failure on push/PR**: Merge is blocked. Developer must fix formatting (`cargo fmt`), resolve clippy warnings, and make tests pass before re-triggering CI via new commit or PR update.
- **Release build failure** (e.g., cross-compilation error): No release assets are published. The developer must tag a new version after fixing the issue — existing releases on GitHub remain unchanged.
- **RPM glob not found**: The `find . -maxdepth 2` fallback in the RPM upload step will echo available `.rpm` files for manual inspection if the expected filename pattern doesn't match.
