# rouser - Agent Guidelines

These guidelines are specific to **AI/LLM agents** working on this codebase. Human developers should follow [CONTRIBUTING.md](./CONTRIBUTING.md).

## Core Principles

- **Read CONTRIBUTING.md first**: Before making changes, read [CONTRIBUTING.md](./CONTRIBUTING.md) for coding standards, testing conventions, and documentation sync rules that apply to all contributors (agents included). AGENTS.md covers agent-specific behavior; CONTRIBUTING.md covers everything else.
- **Build before committing**: The code MUST compile (`cargo build`), pass all tests (`cargo test --all-targets`), and be clean under clippy (`cargo clippy --all-targets -- -D warnings`) before any git commit. Never ship broken code. Always match CI commands exactly — `--all-targets` includes test targets which may have lint warnings not visible otherwise.
- **Conventional commits**: All git commit messages follow [Conventional Commits](https://www.conventionalcommits.org/) format: `type(scope): description`. See section below.
- **Commit frequently when stable**: Make atomic, logical commits whenever the codebase is in a working state (builds, tests pass). Do not batch unrelated changes into a single commit. Each commit should represent one coherent unit of change.
  - **Before every commit**, ensure all changed files are in a good state — no half-written code, incomplete docs, or skipped tests. Code must at minimum compile before committing; full functionality is ideal but a working build is the absolute floor.
  - **Upon finishing each task**, immediately commit all changes with a descriptive message. Do not defer commits across tasks.
  - For larger units of work (major refactoring, big new feature), split into small, manageable commits rather than one massive commit to preserve history granularity and make rollbacks easier.
- **Follow existing patterns first**: Before proposing new patterns or structures, search for and follow established conventions in the codebase. When in doubt, match what's already there.
- **Graceful degradation over panics**: Metric collectors return `Result` types and fall back to zero values on failure. The daemon continues operating even when individual metrics are unavailable.
- **Descriptive comments are encouraged**: Comments that explain non-obvious intent, arithmetic expectations, or why a particular approach was chosen should be kept — especially in tests where the "what" is clear but the "why" and expected values may not be. Docstrings on public APIs and complex algorithms (e.g., accumulation logic, security-critical code) are welcome. Avoid comments that merely restate what the code already says ("increment counter by one"), but keep those that add context a reader wouldn't get from reading alone.

### Agent-Specific Rules (do NOT apply to human developers)

- **No background tasks**: All work must be performed by subagents or in the foreground. Background tasks are not allowed because they often time out or exhaust the context window before completing.
- **Sequential workers only (foreground)**: Delegating to subagents is allowed, but ONLY one at a time with `run_in_background=false` (synchronous mode). Never run multiple agents concurrently — always wait for each worker to finish before spawning the next. This ensures agent output is available in-session and prevents context loss from timed-out background tasks.
- **Never introduce `unsafe` code without explicit instruction**: Do not add `unsafe {}` blocks, FFI bindings, or pointer operations unless the user explicitly requests it with a clear justification. Rust's safety guarantees are a primary design goal of this project.
- **Preserve CI/CD least-privilege permissions**: When editing GitHub Actions workflows, never widen existing permission scopes. Always use job-level `permissions:` blocks — never workflow-level broad grants like `permissions: contents: write`. Each job should have only the minimum permissions it requires (`contents: read` for linting/testing jobs).
- **Never weaken dependency pinning in CI**: When modifying workflow actions references, prefer immutable commit SHAs over mutable tags. If a tag is used (e.g., `@v4`), document the SHA version in code comments or security docs so reviewers can verify it matches an expected release.
- **Validate all external input**: Any file path, environment variable, CLI argument, or config value that originates outside the program must be validated before use. Never assume user-provided values are safe — apply bounds checks for numeric ranges and whitelist validation for string enums.
- **Follow docs/security.md patterns**: Read `docs/security.md` before making changes to CI workflows, packaging scripts, install scripts, or security-related code. Reference it when explaining why a particular pattern is used (e.g., TOCTOU avoidance in temp file handling).

## Versioning Policy

- **Semantic Versioning (SemVer)** is strictly enforced. All version bumps follow `MAJOR.MINOR.PATCH` format per [semver.org](https://semver.org/).
- **Pre-release rule**: Until first stable release, only patch-level changes are expected between minor releases.
  - `v0.0.1`, `v0.0.2`... — bug fixes and minor improvements while pre-1.0
  - `v0.1.0` — first feature release (when ready)
  - `vX.Y.Z` post-1.0: MAJOR for breaking changes, MINOR for new features, PATCH for bugfixes

### Version Bump Rules

| Change Type | Version Bump | Examples |
|-------------|-------------|----------|
| Breaking API change (config format, CLI args) | MAJOR or MINOR (pre-1.0: MINOR) | `v0.1.0 → v0.2.0` |
| New feature / capability | MINOR (pre-1.0: minor patch) | `v0.0.3 → v0.0.4` |
| Bug fix, no behavior change | PATCH | `v0.0.4 → v0.0.5` |
| CI/CD, packaging, docs-only | No version bump needed (unless it affects user-visible behavior) | — |

### Version Management in Cargo.toml

- Update `[package] version = "..."` before any release tag.
- Never commit a version bump without an associated release PR or explicit user request.
- The `--version` flag is derived from Cargo.toml by clap's automatic version handling — no manual sync needed.

### Git Tagging Convention

- Pre-release: `v0.0.X` (e.g., `git tag -a v0.0.1 -m "Patch: fix config path resolution"`)
- Release candidates: `v0.1.0-rc.1`, etc.
- Stable releases: `vX.Y.Z` with annotated tags and release notes

**Never release without explicit user instruction.** If the user asks to release, bump version first, then tag, then create GitHub release.

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

| Type | When to use | Example |
|------|-------------|---------|
| `feat` | New functionality | `feat(gpu): add per-device GPU reporting` |
| `fix` | Bug fixes restoring expected behavior | `fix(service): correct log level parsing` |
| `refactor` | Code changes with no external behavior change | `refactor(gpu): restructure collection methods` |
| `test` | Adding or modifying tests only | `test(service): add EMA convergence tests` |
| `docs` | Documentation-only changes | `docs(readme): update GPU monitoring section` |
| `chore` | Build/config/tooling changes that don't affect source code | `chore(deps): bump toml to 0.8` |

### Scope Guidelines

Use the affected module as scope: `service`, `config`, `gpu`, `cpu`, `network`, `disk`, `inhibit`, or omit for cross-cutting changes.

## Logging Conventions

- Use the `tracing` crate (`debug!`, `info!`, `warn!`, `error!` macros).
- **State-change-only logging**: When tracking persistent states (inhibition, connection status), only emit INFO logs on actual state transitions. Do not log every polling cycle when state is unchanged. Track previous state and compare at the end of each tick/loop iteration.

## Error Handling Conventions

- Use `thiserror` for library-facing error types with descriptive variants.
- Use `anyhow::Result<T>` for binary-level entry points (`main.rs`).
- Metric collectors return `Result<value, CollectorError>` — callers handle errors gracefully (fallback to zero).
- Never silently swallow errors: log them at minimum via `warn!` or `error!`.

## Async / Tokio Conventions

- All I/O-bound operations are async; CPU-bound work should be spawned blocking (`tokio::task::spawn_blocking`).
- Use `tokio::time::sleep` for polling intervals, never blocking `std::thread::sleep` in async contexts.
- The main loop uses `tokio::select!` for shutdown signal handling.

## Configuration Conventions

- TOML format via the `toml` crate with serde derive macros.
- All config values have sensible defaults defined as `fn default_*() -> T` helper functions.
- Optional fields use `#[serde(default)]`; required overrides use `#[serde(default = "default_fn")]`.
- Duration parsing uses `humantime_serde` for human-readable format (e.g., `"5s"`, `"30m"`).

## XDG Base Directory Compliance

All file paths on disk must conform to the [XDG Base Directory Specification](https://specifications.freedesktop.org/basedir-spec/):

- **User binaries**: `${XDG_BIN_HOME:-$HOME/.local/bin}` — use shell expansion pattern `"${XDG_BIN_HOME:-$HOME/.local/bin}/binary_name"` in install scripts and systemd service files.
- **User configs**: `${XDG_CONFIG_HOME:-$HOME/.config}` — config directories are created under this path with fallback to `$HOME/.config` when the variable is unset.
- **System-level overrides** (root): `/etc/rouser/config.toml` for system-wide defaults, written on first run if rouser runs as root and no user config exists.

Never hardcode `~/.local/bin/` or `~/.config/`. Always use the shell expansion pattern with a fallback: `"${XDG_BIN_HOME:-$HOME/.local/bin}"` (not `${XDG_DATA_HOME}../bin}` — that is missing the `/` between `$XDG_DATA_HOME` and `..`, producing invalid paths like `share../bin`).

## Metrics Collection Conventions

### Per-device reporting over aggregation

Each physical device should be reported individually. Aggregation across devices happens at the threshold-checking layer, not during collection. This enables:
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
    ├── gpu.rs       # NVML (NVIDIA via nvml-wrapper) + sysfs (AMD/Intel) GPU collection
    ├── network.rs   # /proc/net/dev network I/O
    └── disk.rs      # /proc/diskstats disk activity
```

## Testing Conventions

- Unit tests in the same file under `#[cfg(test)] mod tests { ... }` modules.
- Test function names follow: `test_<module>_<scenario>_expected_behavior`.
- Run full test suite before any commit: `cargo test --all-targets`.
- Mock external I/O where possible (file paths, command outputs).

## Build & CI Checklist

Before merging or releasing, verify all of the following pass:

```bash
cargo fmt --check          # Code formatting
cargo clippy --all-targets -- -D warnings  # Lint check (must match CI exactly — includes test code)
cargo test --all-targets   # Unit tests (must match CI exactly — includes test targets)
cargo build --release      # Release build succeeds
cargo doc --no-deps        # Documentation compiles (if public API changed)
```

## Lessons Learned

### NVML Library Loading Is Dynamic — Handle Initialization Failures Gracefully

The `nvml-wrapper` crate dynamically loads `libnvidia-ml.so` at runtime. On systems without NVIDIA drivers, `Nvml::init()` returns an error rather than panicking. Always check the result and fall back gracefully (e.g., return 0% utilization) instead of failing the entire metrics collection. NVML initialization is thread-safe — calling it multiple times reuses the same loaded library instance internally.

### PCI Bus ID Format Mismatch Between NVML and Sysfs

NVML reports bus IDs in 8-digit domain format (`"00000000:09:00.0"`) while sysfs uevent files use 4-digit domain format (`"0000:09:00.0"`). When matching NVML devices to sysfs cards, always check if the shorter sysfs PCI slot is a substring of the NVML bus ID rather than requiring exact string equality.

### Driver Detection Must Cover All Vendors

Driver detection functions must recognize ALL driver types present on target systems, including proprietary drivers like `"nvidia"` and `"nouveau"`. If a card's driver isn't recognized, it falls through to `"unknown"` which breaks the skip logic in collectors — NVIDIA cards may appear as duplicate entries or be silently dropped.

### Manual QA with Debug Logging on Real Hardware

Unit tests cannot verify actual GPU detection or real hardware interaction. Always run `RUST_LOG=debug ./target/debug/rouser --config <path> --dry-run` to see what collectors actually produce before considering a change complete. This catches NVML initialization failures, PCI matching issues, and sysfs path problems that unit tests miss.

### Fail-Fast with Diagnostics for GPU Collection Failures

When NVML is available but returns zero utilization results while hardware exists in sysfs, log a `warn!` message rather than silently returning empty data. This helps operators distinguish between "no GPUs" and "GPU monitoring tool failed." Keep sysfs as the canonical device enumeration source — NVML devices are matched to sysfs cards by PCI bus ID, not vice versa.

### Deprecated FreeDesktop PowerManagement API Must Not Be Used

The old `/org/freedesktop/PowerManagement.Inhibit` API is obsolete (deprecated ~2014) and must not be referenced as a viable approach. Always use `org.freedesktop.login1.Manager.Inhibit`. Document why deprecated approaches were abandoned in AGENTS.MD so future agents don't revisit dead ends.

### config/rouser.toml Is the Source of Truth for All Defaults

`config/rouser.toml` is the single source of truth for all configuration defaults — not `src/config.rs`, not documentation, not code comments. When updating default values:

1. **Always update `config/rouser.toml` first** with the new default value
2. Then update `src/config.rs` to match (default helper functions like `default_ema_alpha_cpu()`)
3. Then update all documentation (`docs/configuration.md`, `docs/metrics-overview.md`, etc.)

The code defaults in `config/rouser.toml` are embedded at compile time via `include_str!()` and served as both the shipped config file AND the binary's built-in fallback. Never change a default value without updating all three locations simultaneously.

### D-Bus Inhibition: "sleep" vs "shutdown:idle" in `[inhibitor].what`

The `what` parameter controls which operations rouser inhibits. Two reasonable defaults exist for different deployment profiles:

- **`"sleep"`** (simple) — Good for headless servers and traditional daemon deployments. Blocks sleep/hibernate but does not interfere with desktop environment idle timers or shutdown delays.
- **`"shutdown:idle"`** (conservative, current default) — Better for workstations/home-labs running DEs like KDE/GNOME. Prevents the system from entering any powered-off or suspended state while metrics are active.

Observed behavior on KDE: `"sleep"` alone may cause KDE to never automatically sleep after inhibition is released, as it interprets the lock as a persistent "don't touch my power management" signal. Conversely, `"shutdown:idle"` lets KDE respect its configured idle delay — if set to 15 minutes, KDE will put the system to sleep 15 minutes after rouser releases its locks.

When writing docs or examples about inhibition behavior, document this difference explicitly so users understand why their DE reacts differently to each option. The default in `config/rouser.toml` is `"shutdown:idle"`.

## Dependency Policy

- Prefer stdlib and crate ecosystem over external binary dependencies where possible.
- For hardware-specific access (NVIDIA GPUs), use the NVML library (`libnvidia-ml.so`) loaded dynamically at runtime via `nvml-wrapper` — no separate binary dependency required since it ships with NVIDIA drivers alongside nvidia-smi itself.
- When adding a new dependency: justify in the commit message, prefer widely-packaged crates, avoid unmaintained crates.
- Current external dependencies: `libnvidia-ml.so` (NVIDIA proprietary drivers only, loaded dynamically at runtime), no separate binary dependencies.

## CI/CD Pipeline Debugging with GitHub API

When debugging failing CI pipelines in-session, the GitHub REST API provides faster diagnostics than waiting for the GHA web UI or trying to read step-level logs programmatically. This is especially valuable when working with agents in the loop where context window and time are limited.

### Workflow Runs — Check Status of Recent Runs

```bash
# Get latest 5 workflow runs across all workflows (use ?workflow_id=X for specific)
gh api repos/{owner}/{repo}/actions/runs?per_page=5 \
  | jq -r '.workflow_runs[] | "Run \(.id) | event:\(.event) | status:\(.status)/\(.conclusion) | sha:\(.head_sha[0:7])"'
```

### Jobs Within a Run — See Which Specific Job/Step Failed

```bash
# Get all jobs for a specific run (replace RUN_ID from above)
gh api repos/{owner}/{repo}/actions/runs/RUN_ID/jobs?per_page=100 \
  | jq -r '.jobs[] | "\(.conclusion): \(.name)"' | sort
```

### Release Assets — Verify What's Actually on a Release After CI Completes

```bash
gh api repos/{owner}/{repo}/releases?per_page=10 \
  | jq -r '.[] | select(.tag_name == "vX.Y.Z") | .assets[].name'
```

### Why This Works Better Than Alternatives

- **GHA web UI**: Requires manual browser interaction; no API access for step-level log contents (step `logs_url` returns falsy in API responses even when logs exist).
- **Step-level logs**: The GitHub Actions API does not expose per-step log URLs via `/jobs` or `/runs/{id}/attempts/1/jobs` endpoints without additional authentication headers that are often unavailable to agent sessions. Use run-level job summaries (`conclusion` field) instead — if a specific step fails but the overall job shows `success`, check for conditional steps that may have errored silently (e.g., upload scripts with non-zero exit codes caught by shell error handling).
- **Wait time**: API calls return immediately; waiting for GHA UI to update can take 5–10 minutes after a tag push.

### Practical Debugging Flow

```bash
# 1. Push tag → wait ~30 seconds → check latest runs
gh api repos/{owner}/{repo}/actions/runs?per_page=3 | jq -r '.workflow_runs[:2][] | "\(.id): \(.status)/\(.conclusion)"'

# 2. Pick the newest run ID, list all jobs
gh api repos/{owner}/{repo}/actions/runs/RUN_ID/jobs?per_page=100 \
  | jq -r '.jobs[] | select(.conclusion != "success") | "\(.name): \(.conclusion)"'

# 3. Check release assets (if run was triggered by tag/release)
gh api repos/{owner}/{repo}/releases?per_page=5 \
  | jq -r 'map(select(.tag_name == "vX.Y.Z"))[0].assets[]?.name // empty' | sort

# 4. If a job failed, visit the GHA web UI directly for step logs:
echo "https://github.com/{owner}/{repo}/actions/runs/RUN_ID"
```

### Common Pitfalls When Debugging CI with Agents

- **YAML indentation errors**: A single-space indent instead of two spaces under `jobs:` causes silent parse failures that mask all real runtime errors. Validate YAML syntax before blaming script logic: `python3 -c "import yaml; yaml.safe_load(open('.github/workflows/ci.yml'))"`.
- **Missing `needs` dependencies**: If a job references another via `needs: [foo]`, and `foo` is conditional (`if:`), the dependent job inherits that condition — it will skip if the dependency was skipped. Always verify both jobs have matching trigger conditions.
- **Container vs runner environment mismatch**: Steps running in containers (e.g., `container: fedora:latest`) cannot access tools on the host runner (like `gh` CLI). Split containerized build steps from upload/CLI steps that run on `ubuntu-latest` without a container.
- **Artifact download path defaults to `.`**: When using `actions/download-artifact@v4`, always specify `path: some-dir/` explicitly, then move files with `mv some-dir/* .` before consuming them — default behavior may merge artifacts unpredictably.
