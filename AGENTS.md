# rouser - Agent Guidelines

## Core Principles

- **Build before committing**: The code MUST compile (`cargo build`), pass all tests (`cargo test`), and be clean under clippy (`cargo clippy -- -D warnings`) before any git commit. Never ship broken code.
- **Conventional commits**: All git commit messages follow [Conventional Commits](https://www.conventionalcommits.org/) format: `type(scope): description`. See section below.
- **Commit frequently when stable**: Make atomic, logical commits whenever the codebase is in a working state (builds, tests pass). Do not batch unrelated changes into a single commit. Each commit should represent one coherent unit of change.
- **Follow existing patterns first**: Before proposing new patterns or structures, search for and follow established conventions in the codebase. When in doubt, match what's already there.
- **Graceful degradation over panics**: Metric collectors return `Result` types and fall back to zero values on failure. The daemon continues operating even when individual metrics are unavailable.

## Binary Artifacts

- **Never copy the compiled `rouser` binary** into the repo root or any tracked directory. The `.gitignore` already covers `/target/`; do not create standalone copies like `cp target/debug/rouser rouser`.
- Run binaries directly from their build output: `./target/debug/rouser` for debug, `./target/release/rouser` for release builds.
- Never commit binary artifacts (compiled executables, `.rlib`, etc.) to git.

## Git Commit Conventions

### Format

```
type(scope): description

[optional body explaining why]
```

**Rules:**
- Lowercase subject line (after the scope colon)
- No period at end of subject line
- Subject line max 72 characters
- Body wraps at 72 characters
- Use imperative mood: "add feature", not "added" or "adds"

### Types

| Type | When to use |
|------|-------------|
| `feat` | New functionality (e.g., `feat(gpu): add per-device GPU reporting`) |
| `fix` | Bug fixes that restore expected behavior (e.g., `fix(service): correct log level parsing`) |
| `refactor` | Code changes with no external behavior change (e.g., `refactor(gpu): restructure collection methods`) |
| `test` | Adding or modifying tests only (e.g., `test(service): add EMA convergence tests`) |
| `docs` | Documentation-only changes (e.g., `docs(readme): update GPU monitoring section`) |
| `chore` | Build/config/tooling changes that don't affect source code (e.g., `chore(deps): bump toml to 0.8`) |

### Scope Guidelines

Use the affected module as scope: `service`, `config`, `gpu`, `cpu`, `network`, `disk`, `inhibit`, or omit for cross-cutting changes.

## Logging Conventions

- Use the `tracing` crate (`debug!`, `info!`, `warn!`, `error!` macros)
- Log levels: DEBUG for collection details, INFO for state transitions (not per-tick noise), WARN for recoverable issues, ERROR for unrecoverable failures
- Include contextual identifiers in log messages (GPU device IDs, interface names, threshold values)
- **State-change-only logging**: When tracking persistent states (inhibition, connection status), only emit INFO logs on actual state transitions. Do not log every polling cycle when state is unchanged. Track previous state and compare at the end of each tick/loop iteration.

## Error Handling Conventions

- Use `thiserror` for library-facing error types with descriptive variants
- Use `anyhow::Result<T>` for binary-level entry points (`main.rs`)
- Metric collectors return `Result<value, CollectorError>` — callers handle errors gracefully (fallback to zero)
- Never silently swallow errors: log them at minimum via `warn!` or `error!`

## Async / Tokio Conventions

- All I/O-bound operations are async; CPU-bound work should be spawned blocking (`tokio::task::spawn_blocking`)
- Use `tokio::time::sleep` for polling intervals, never blocking `std::thread::sleep` in async contexts
- The main loop uses `tokio::select!` for shutdown signal handling

## Configuration Conventions

- TOML format via the `toml` crate with serde derive macros
- All config values have sensible defaults defined as `fn default_*() -> T` helper functions
- Optional fields use `#[serde(default)]`; required overrides use `#[serde(default = "default_fn")]`
- Duration parsing uses `humantime_serde` for human-readable format (e.g., `"5s"`, `"30m"`)

## Metrics Collection Conventions

### Per-device reporting over aggregation

Each physical device should be reported individually. Aggregation across devices happens at the threshold-checking layer, not during collection. This enables:
- Accurate per-GPU logging (`GPU0(nvidia): 45.2%, card1(amdgpu): 78.1%`)
- Per-device EMA smoothing for stable readings
- Better diagnostics when one device is anomalous

### GPU metrics specifically

| Vendor | Data source | Device identification | Driver detection |
|--------|------------|----------------------|------------------|
| NVIDIA | `nvidia-smi` subprocess query per-GPU | Index from `--query-gpu=index,...` output → `GPU{n}` | Always `"nvidia"` (binary implies driver) |
| AMD    | `/sys/class/drm/cardN/device/gpu_busy_percent` | Sysfs card path (`card0`, `card1`) | Symlink at `device/driver` target |
| Intel  | Same sysfs path as AMD | Sysfs card path | Symlink: `i915` or `xe` driver |

**GpuData struct fields:**
- `device_id: String` — human-readable device identifier (e.g., `"GPU0"`, `"card1"`)
- `driver_name: String` — kernel driver name (e.g., `"nvidia"`, `"amdgpu"`, `"i915"`, `"xe"`, `"unknown"`)
- `usage: f64` — utilization percentage (0.0–100.0)

**NVIDIA subprocess constraint:** The nvidia-smi binary is required for NVIDIA GPU monitoring on proprietary drivers. This is an unavoidable external dependency since the driver package ships it. No well-maintained Rust NVML binding crate exists in crates.io that would reduce this to a library call. Per-device parsing of `nvidia-smi` output (rather than averaging) is the correct approach.

## Code Structure Conventions

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
    ├── gpu.rs       # nvidia-smi (NVIDIA) + sysfs (AMD/Intel) GPU collection
    ├── network.rs   # /proc/net/dev network I/O
    └── disk.rs      # /proc/diskstats disk activity
```

## Testing Conventions

- Unit tests in the same file under `#[cfg(test)] mod tests { ... }` modules
- Test function names follow: `test_<module>_<scenario>_expected_behavior`
- Run full test suite before any commit: `cargo test --all-targets`
- Mock external I/O where possible (file paths, command outputs)

## Build & CI Checklist

Before merging or releasing, verify all of the following pass:

```bash
cargo fmt --check          # Code formatting
cargo clippy -- -D warnings  # Lint check
cargo test                 # Unit tests
cargo build --release      # Release build succeeds
cargo doc --no-deps        # Documentation compiles (if public API changed)
```

## Lessons Learned

### GPU Collection — Verify External Binary Output Format

When parsing output from subprocesses like `nvidia-smi`, always verify the actual output format before implementing parsers. The query `--query-gpu=index,utilization.gpu` returns **CSV** (`"0, 7"` per line), not a single value. Parsing the entire line as one float silently produces zero results — no GPU data appears in logs with no error indication.

### Driver Detection Must Cover All Vendors

Driver detection functions must recognize ALL driver types present on target systems, including proprietary drivers like `"nvidia"` and `"nouveau"`. If a card's driver isn't recognized, it falls through to `"unknown"` which breaks the skip logic in collectors — NVIDIA cards may appear as duplicate entries or be silently dropped.

### Manual QA with Debug Logging on Real Hardware

Unit tests cannot verify actual subprocess output parsing or real hardware detection. Always run `RUST_LOG=debug ./target/debug/rouser --config <path> --dry-run` to see what collectors actually produce before considering a change complete. This catches format mismatches, missing drivers, and sysfs path issues that unit tests miss.

### Fail-Fast with Diagnostics for External Tools

When an external binary (nvidia-smi) is available but returns zero results while hardware exists in sysfs, log a `warn!` message rather than silently returning empty data. This helps operators distinguish between "no GPUs" and "GPU monitoring tool failed." Add sysfs fallback collection as emergency recovery when primary methods fail entirely.

## Dependency Policy

- Prefer stdlib and crate ecosystem over external binary dependencies where possible
- For hardware-specific access (NVIDIA GPUs), subprocess to shipped binaries is acceptable (`nvidia-smi` from driver package, sysfs for AMD/Intel kernel interfaces)
- When adding a new dependency: justify in the commit message, prefer widely-packaged crates, avoid unmaintained crates
- Current external binary dependencies: `nvidia-smi` (NVIDIA proprietary drivers only), no other binaries
