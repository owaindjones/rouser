# Contributing to rouser

Thank you for your interest in contributing to **rouser**! This guide covers everything a developer needs — from building the project and coding conventions to git workflow and release procedures. Whether you are a human developer or an LLM agent, these rules apply equally.

## Table of Contents

1. [Getting Started](#getting-started)
2. [Building & Testing](#building--testing)
3. [Code Quality Checklist](#code-quality-checklist)
4. [Coding Standards](#coding-standards)
   - [Rust Style & Formatting](#rust-style--formatting)
   - [Error Handling](#error-handling)
   - [Logging](#logging)
   - [Async / Tokio Conventions](#async--tokio-conventions)
   - [Configuration Conventions](#configuration-conventions)
5. [Metrics Collection Conventions](#metrics-collection-conventions)
6. [Code Structure](#code-structure)
7. [Testing Guidelines](#testing-guidelines)
8. [Git Workflow & Commits](#git-workflow--commits)
   - [Conventional Commits](#conventional-commits)
   - [Branching Strategy (pre-v1.0)](#branching-strategy-pre-v10)
   - [Git Flow (v1.0+)](#git-flow-v10)
9. [Versioning Policy](#versioning-policy)
10. [Binary Artifacts](#binary-artifacts)
11. [Architecture Reference](#architecture-reference)

---

## Getting Started

### Prerequisites

- **Rust 1.70+** — install via [`rustup`](https://rustup.rs/)
- **Systemd with D-Bus** (login1 API, available on any modern Linux distro)
- **Optional**: NVIDIA drivers with NVML library (`libnvidia-ml.so`) for GPU monitoring

### Clone & Build

```bash
git clone https://github.com/owaindjones/rouser.git
cd rouser

# Debug build
cargo build

# Release build (optimized, LTO, stripped)
cargo build --release
```

### Run as a Daemon

```bash
./target/release/rouser              # normal mode
./target/release/rouser --dry-run    # test without inhibition
RUST_LOG=debug ./target/release/rouser -l debug  # verbose diagnostics
```

---

## Building & Testing

All build commands use Cargo's built-in tooling — no external build system required.

| Command | Description |
|---------|-------------|
| `cargo build` | Debug build (fast, with debugging symbols) |
| `cargo build --release` | Release build (optimized, LTO enabled, stripped) |
| `cargo test` | Run all unit tests |
| `cargo test --all-targets` | Run all targets including doctests and examples |

---

## Code Quality Checklist

**All of the following MUST pass before any commit or pull request.** Never ship broken code.

```bash
cargo fmt --check          # Consistent formatting
cargo clippy --all-targets -- -D warnings  # Zero lint warnings
cargo test --all-targets   # All tests passing
cargo build --release      # Release binary compiles
cargo doc --no-deps        # Documentation compiles (if public API changed)
```

---

## Coding Standards

### Rust Style & Formatting

- Use `rustfmt` for all code formatting. Configure via `[package]` or workspace settings.
- Run `cargo fmt` before committing to auto-format your changes.
- Follow [Rust API Guidelines](https://rust-lang.github.io/api-guidelines/) where applicable.

### Error Handling

- Use **`thiserror`** for library-facing error types with descriptive variants:

```rust
use thiserror::Error;

#[derive(Error, Debug)]
pub enum GpuCollectorError {
    #[error("NVIDIA drivers not installed")]
    NvidiaNotAvailable,

    #[error("AMD/Intel sysfs not available at {0}")]
    SysfsNotFound(String),

    #[error("Failed to parse GPU usage: {0}")]
    ParseError(String),
}
```

- Use **`anyhow::Result<T>`** for binary-level entry points (`main.rs`).
- Metric collectors return `Result<value, CollectorError>` — callers handle errors gracefully by falling back to zero.
- **Never silently swallow errors**: log them at minimum via `warn!` or `error!`.

### Logging

Use the `tracing` crate throughout:

```rust
use tracing::{debug, info, warn, error};
```

| Level | When to Use | Example |
|-------|-------------|---------|
| `debug!` | Collection details, per-tick diagnostics | Per-device GPU readings |
| `info!` | **State transitions only** (not per-tick noise) | Inhibition acquired/released |
| `warn!` | Recoverable issues that deserve attention | Metric collection failed for one device |
| `error!` | Unrecoverable failures | D-Bus connection lost |

**State-change-only logging**: When tracking persistent states (inhibition, connection status), only emit `info!` logs on actual state transitions. Track previous state and compare at the end of each tick/loop iteration — do not log every polling cycle when state is unchanged.

Include contextual identifiers in log messages: GPU device IDs (`card0(nvidia)`), interface names, threshold values.

### Async / Tokio Conventions

- All I/O-bound operations are `async`; CPU-bound work should be spawned blocking via `tokio::task::spawn_blocking`.
- Use `tokio::time::sleep` for polling intervals — never use blocking `std::thread::sleep` in async contexts.
- The main loop uses `tokio::select!` for shutdown signal handling.

### Configuration Conventions

- TOML format via the `toml` crate with serde derive macros.
- All config values have sensible defaults defined as `fn default_*() -> T` helper functions.
- Optional fields use `#[serde(default)]`; required overrides use `#[serde(default = "default_fn")]`.
- Duration parsing uses `humantime_serde` for human-readable format (e.g., `"5s"`, `"30m"`).

---

## Metrics Collection Conventions

### Per-device reporting over aggregation

Each physical device should be reported individually. Aggregation across devices happens at the **threshold-checking layer**, not during collection. This enables:

- Accurate per-GPU logging (`card0(nvidia): 45.2%, card1(amdgpu): 78.1%`)
- Per-device EMA smoothing for stable readings
- Better diagnostics when one device is anomalous

### GPU metrics — data sources by vendor

| Vendor | Data source | Device identification | Driver detection |
|--------|------------|----------------------|------------------|
| NVIDIA | NVML library (`libnvidia-ml.so`) via `nvml-wrapper` crate | Matched to sysfs card via PCI bus ID from uevent file → `"cardN"` | Always `"nvidia"` (NVML implies proprietary driver) |
| AMD    | `/sys/class/drm/cardN/device/gpu_busy_percent` | Sysfs card path (`card0`, `card1`) | Symlink at `device/driver` target |
| Intel  | Same sysfs path as AMD | Sysfs card path | Symlink: `i915` or `xe` driver |

**GpuData struct fields:**
- `device_id: String` — human-readable device identifier (e.g., `"card0"`, `"card1"`)
- `driver_name: String` — kernel driver name (e.g., `"nvidia"`, `"amdgpu"`, `"i915"`, `"xe"`, `"unknown"`)
- `usage: f64` — utilization percentage (0.0–100.0)

**NVIDIA NVML approach:** NVIDIA GPU monitoring uses the NVML library (`libnvidia-ml.so`) loaded dynamically at runtime via the `nvml-wrapper` Rust crate. The same API is used by `nvidia-smi`, nvtop, and other NVIDIA tools. Device enumeration uses `device_by_index()`, matching to sysfs cards via PCI bus ID comparison between NVML's `pci_info.bus_id` and `/sys/class/drm/cardN/device/uevent` (`PCI_SLOT_NAME`). This eliminates subprocess spawning overhead entirely.

### Lessons Learned (for developers working on metrics collection)

- **NVML library loading is dynamic** — handle initialization failures gracefully. On systems without NVIDIA drivers, `Nvml::init()` returns an error rather than panicking.
- **PCI bus ID format mismatch between NVML and sysfs** — NVML reports 8-digit domain (`"00000000:09:00.0"`), sysfs uses 4-digit (`"0000:09:00.0"`). Match by substring check, not exact equality.
- **Driver detection must cover all vendors**, including proprietary drivers like `"nvidia"` and `"nouveau"`. Unrecognized drivers fall through to `"unknown"` which breaks skip logic.
- **Run with `RUST_LOG=debug` on real hardware** for manual QA before considering changes complete — unit tests cannot verify GPU detection or NVML interaction.
- **Fail-fast with diagnostics**: When an external binary returns zero results while hardware exists in sysfs, log a `warn!` message rather than silently returning empty data.

---

## Code Structure

```
src/
├── main.rs          # CLI entry point, clap args, dry-run/daemon modes
├── lib.rs           # Public module re-exports
├── config.rs        # Config structs + ConfigLoader (TOML parsing)
├── service.rs       # DataService/DataManager core loop, threshold checking, EMA smoothing
├── inhibit.rs       # D-Bus sleep inhibition via zbus v4
└── metrics/
    ├── mod.rs       # Metrics struct + module re-exports
    ├── cpu.rs       # /proc/stat CPU collection
    ├── gpu.rs       # NVML (NVIDIA via nvml-wrapper) + sysfs (AMD/Intel) GPU collection
    ├── network.rs   # /proc/net/dev network I/O
    └── disk.rs      # /proc/diskstats disk activity
```

---

## Testing Guidelines

- Unit tests live in the same file under `#[cfg(test)] mod tests { ... }` modules.
- Test function names follow: `test_<module>_<scenario>_expected_behavior`.
- Mock external I/O where possible (file paths, command outputs).
- Run the full test suite before any commit:

```bash
cargo test --all-targets
```

---

## Git Workflow & Commits

### Conventional Commits

All git commit messages **must** follow [Conventional Commits](https://www.conventionalcommits.org/) format:

```
type(scope): description

[optional body explaining why]
```

**Rules:**
- Lowercase subject line (after the scope colon)
- No period at end of subject line
- Subject line max 72 characters
- Body wraps at 72 characters
- Use imperative mood: `"add feature"`, not `"added"` or `"adds"`

### Commit Types

| Type | When to use | Example |
|------|-------------|---------|
| `feat` | New functionality | `feat(gpu): add per-device GPU reporting` |
| `fix` | Bug fixes restoring expected behavior | `fix(service): correct log level parsing` |
| `refactor` | Code changes with no external behavior change | `refactor(gpu): restructure collection methods` |
| `test` | Adding or modifying tests only | `test(service): add EMA convergence tests` |
| `docs` | Documentation-only changes | `docs(readme): update GPU monitoring section` |
| `chore` | Build/config/tooling changes | `chore(deps): bump toml to 0.8` |

### Scope Guidelines

Use the affected module as scope: `service`, `config`, `gpu`, `cpu`, `network`, `disk`, `inhibit`, or omit for cross-cutting changes.

### Commit Discipline

- **Build before committing**: The code MUST compile, pass all tests (`cargo test`), and be clean under clippy (`cargo clippy -- -D warnings`) before any commit.
- **Commit frequently when stable**: Make atomic, logical commits whenever the codebase is in a working state. Do not batch unrelated changes into a single commit. Each commit should represent one coherent unit of change.
- **Before every commit**, ensure all changed files are in a good state — no half-written code, incomplete docs, or skipped tests. Code must at minimum compile before committing; full functionality is ideal but a working build is the absolute floor.
- **Upon finishing each task**, immediately commit all changes with a descriptive message. Do not defer commits across tasks.
- For larger units of work (major refactoring, big new feature), split into small, manageable commits rather than one massive commit to preserve history granularity and make rollbacks easier.

### Git Flow

Thr project uses **git-flow** branching:

```
release/* ──────┐
     │          ├──▶ main (tagged releases)
develop ◀────────┘
     ▲
feature/* ───────┘
fix/*   ───────┘
hotfix/* ──┘
```

| Branch Type | Purpose | Target | Example |
|-------------|---------|--------|---------|
| `main` | Production-ready code | — | Tagged releases only |
| `develop` | Integration branch for features | — | Default development branch |
| `feature/<name>` | New features | `develop` | `feature/intel-gpu-support` |
| `release/*` | Prepare new release | `main` + `develop` | `release/1.1.0` |
| `hotfix/*` | Quick fixes for production | `main` + `develop` | `hotfix/crash-on-no-gpu` |

**Release workflow:**
1. Create `release/X.Y.Z` from `develop`
2. Bump version, update changelog, run full CI checks
3. Merge to `main` and tag with `vX.Y.Z`
4. Merge back into `develop`

**Hotfix workflow:**
1. Create `hotfix/<name>` from `main`
2. Fix the issue, update version patch
3. Merge to `main`, tag release, merge back to `develop`

---

## Versioning Policy

rouser follows [Semantic Versioning 2.0](https://semver.org/). All version bumps use `MAJOR.MINOR.PATCH`.

| Change Type | Version Bump | Example |
|-------------|-------------|---------|
| Breaking API change | MAJOR | `v1.2.3 → v2.0.0` |
| New feature (backward compatible) | MINOR | `v1.2.3 → v1.3.0` |
| Bug fix | PATCH | `v1.2.3 → v1.2.4` |

### Version Management in Cargo.toml

- Update `[package] version = "..."` before any release tag.
- The `--version` flag is derived from Cargo.toml automatically by clap — no manual sync needed.

---

## Binary Artifacts

- **Never copy the compiled `rouser` binary** into the repo root or any tracked directory. The `.gitignore` covers `/target/`.
- Run binaries directly from their build output: `./target/debug/rouser` for debug, `./target/release/rouser` for release builds.
- Never commit binary artifacts (compiled executables, `.rlib`, etc.) to git.

---

## Architecture Reference

For detailed architecture documentation — including component diagrams, how to add new metrics modules, inhibitor implementations, and the full service loop design — see:

- **[docs/developer-guide.md](docs/developer-guide.md)** — Architecture overview, extending rouser, coding standards
- **[docs/metrics-overview.md](docs/metrics-overview.md)** — How each metric type is collected
- **[docs/configuration.md](docs/configuration.md)** — Full configuration reference
- **[AGENTS.md](AGENTS.md)** — Agent-specific development guidelines
