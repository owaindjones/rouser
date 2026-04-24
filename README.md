# rouser

<p align="center">
  <a href="#readme"><img src="docs/rouser-logo.svg" alt="rouser logo — eye/radar with animated metric bars" width="600"></a>
</p>

A Linux daemon that monitors system metrics (CPU, GPU, network, disk) and inhibits sleep when activity thresholds are exceeded.

[![CI](https://github.com/yourusername/rouser/actions/workflows/ci.yml/badge.svg)](https://github.com/yourusername/rouser/actions/workflows/ci.yml)
[![Rust](https://img.shields.io/badge/rust-stable-orange?style=flat&logo=rust)](https://www.rust-lang.org)
[![License](https://img.shields.io/badge/license-MIT-blue?style=flat)](LICENSE)

## Build & Packaging Coverage

| Target | CI Status | Package Format |
|--------|-----------|----------------|
| x86_64 Linux | ![CI build](https://github.com/yourusername/rouser/actions/workflows/ci.yml/badge.svg?event=release) | Tarball + DEB + RPM |
| aarch64 Linux | ![CI cross-build](https://github.com/yourusername/rouser/actions/workflows/ci.yml/badge.svg?event=release) | Tarball + DEB (arm64) + RPM (aarch64) |
| Arch / Bazzite | — | PKGBUILD archive on release |

> **Pre-release**: rouser is at `v0.0.0` (unreleased). No official releases yet. See [AGENTS.md](AGENTS.md) for versioning policy.

## Overview

`rouser` is designed for headless servers and desktop systems that need to automatically sleep when idle and stay awake during active use. It monitors multiple system metrics and uses systemd's login1 D-Bus API to prevent unwanted suspend or hibernation.

### Key Features

- **Multi-metric monitoring**: CPU, GPU (NVIDIA/AMD/Intel), network I/O, and disk activity
- **Configurable thresholds**: Set individual percentage thresholds for each metric
- **Hysteresis timing**: Prevents rapid sleep/inhibit cycling with configurable delays
- **Systemd integration**: Uses `org.freedesktop.login1.Manager.Inhibit` for reliable sleep inhibition
- **TOML configuration**: Pure Rust, secure configuration format
- **Dry-run mode**: Test configuration without inhibiting sleep
- **Graceful error handling**: Continues operation even if some metrics are unavailable

### Use Case

Allow a system to:
- Sleep after idle time when no activity is detected
- Stay awake during high CPU/GPU usage (compiling, rendering, gaming)
- Remain active during network transfers or disk I/O
- Be woken by WOL (Wake-on-LAN) packets

## Installation

### From Source

```bash
git clone https://github.com/yourusername/rouser.git
cd rouser
cargo build --release
cp target/release/rouser ~/.local/bin/
```

### Via Installer Script

The provided installer script fetches the latest release from GitHub and sets up systemd:

```bash
curl -fsSL https://raw.githubusercontent.com/yourusername/rouser/main/scripts/install.sh | bash -s -- --help
# Then run without flags to install:
curl -fsSL https://raw.githubusercontent.com/yourusername/rouser/main/scripts/install.sh | bash
```

The installer will:
1. Download the latest release archive matching your architecture (x86_64 or aarch64)
2. Copy the binary to `~/.local/bin/rouser`
3. Install default config to `~/.config/rouser/config.toml`
4. Enable systemd user service at `~/.config/systemd/user/rouser.service`

> **Note**: The installer requires `logind lingering` enabled for your user. Run the script's built-in guidance if systemctl complains about a non-active session.

### Dependencies

- Rust 1.70+ (for source builds)
- Systemd with D-Bus (login1 API, typically available on any modern distro)
- Optional: NVIDIA drivers with `nvidia-smi` for GPU monitoring
- Linux kernel with `/sys/class/drm` for AMD/Intel GPU monitoring

## Quick Start

### 1. Create Configuration

rouser searches for config files in this order (first existing wins):

| Priority | Path | Description |
|----------|------|-------------|
| 1 | `./config/rouser.toml` | Repo-packaged default |
| 2 | `~/.config/rouser/config.toml` | XDG user config |
| 3 | `/etc/rouser/config.toml` | System-wide config (requires root) |

Copy the repo default or create your own:

```bash
# Use repo-packaged default in current directory
cp config/rouser.toml ./config/rouser.toml

# Or install to XDG path
mkdir -p ~/.config/rouser
cp config/rouser.toml ~/.config/rouser/config.toml
```

### 2. Validate Configuration

```bash
rouser --validate-config
```

### 3. Test in Dry-Run Mode

```bash
# Collect metrics with default log level (info)
rouser --dry-run

# With debug logging to see per-device readings
RUST_LOG=debug rouser --dry-run -l debug
```

Sample output:
```
CPU threshold: 80%, EMA alpha: 0.30
GPU threshold: 90%, EMA alpha: 0.30
Network threshold: 100 Mbps, EMA alpha: 0.20
Disk threshold: 50 MB/s, EMA alpha: 0.20
Duration threshold: 30s
Cooldown duration: 60s
```

### 4. Run as a Daemon

```bash
rouser --dry-run   # first test without inhibition
rouser              # normal mode — inhibits sleep when thresholds exceeded (Ctrl+C to stop)
```

### 5. Install as Systemd User Service

After verifying with dry-run:

```bash
# Enable and start the user service
systemctl --user daemon-reload
systemctl --user enable --now rouser.service

# Check status
journalctl --user -u rouser -f
```

## Configuration

Create a config file at one of the default search paths (see Quick Start above) or pass `-c /path/to/config.toml`. The TOML format uses **nested metric sections** over flat threshold keys for clarity and per-metric EMA smoothing:

```toml
name = "rouser"
update_interval = "5s"
log_level = "info"

[metrics.cpu]
threshold = 80.0       # CPU usage % (0–100) above which to inhibit sleep
ema_alpha = 0.3        # EMA smoothing: higher = more responsive, lower = smoother

[metrics.gpu]
threshold = 90.0       # GPU usage % per device
ema_alpha = 0.3

[metrics.network]
threshold = 100.0      # Network throughput in Mbps
ema_alpha = 0.2        # EMA smoothing for network I/O
exclude_interfaces = ["lo"]    # Exclude from monitoring (default: loopback)
include_interfaces = []        # Only monitor these; empty means all

[metrics.disk]
threshold = 50.0       # Disk I/O in MB/s above which to inhibit sleep
ema_alpha = 0.2        # EMA smoothing for disk activity
exclude_device_prefixes = ["loop", "fd", "sr", "cdrom"]  # Exclude virtual devices

[timing]
duration_threshold = "30s"   # Min time metrics must exceed threshold before inhibiting
cooldown_duration = "60s"    # Time after releasing inhibition before re-inhibiting possible

[inhibitor]
what = "shutdown:idle"       # Lock types: idle, sleep, suspend, shutdown (colon-separated)
mode = "block"               # Mode: block, delay, or block-weak
```

### Configuration Sections Reference

#### `[metrics.cpu]` / `[metrics.gpu]` / `[metrics.network]` / `[metrics.disk]`
Each metric section has `threshold`, `ema_alpha`, and optional interface/device filters (network, disk only). See [Configuration Reference](docs/configuration.md) for full details.

#### `[timing]` — Hysteresis Timing
- **`duration_threshold`** (`"30s"`): Minimum continuous time metrics must exceed threshold before inhibiting sleep. Prevents brief spikes from triggering inhibition.
- **`cooldown_duration`** (`"60s"`): Time after releasing inhibition during which the daemon won't re-inhibit even if thresholds are exceeded again.

#### `[inhibitor]` — D-Bus Inhibition Settings
| Key | Default | Description |
|-----|---------|-------------|
| `what` | `"shutdown:idle"` | Lock types to inhibit (colon-separated): `idle`, `sleep`, `suspend`, `shutdown`. Multiple combined with colons, e.g., `"sleep:suspend"`. Reference: [systemd inhibitor locks](https://systemd.io/INHIBITOR_LOCKS/) |
| `mode` | `"block"` | Inhibition mode: `block` (completely blocks), `delay` (delays for duration of inhibition), `block-weak` (blocks but overridable by privileged processes) |

#### `[network]` and `[disk]` sub-fields
These are nested under their respective metric sections (`[metrics.network]`, `[metrics.disk]`) in the TOML file, not top-level. See [Configuration Reference](docs/configuration.md).
- `cpu_usage`: CPU usage percentage (0-100)
- `gpu_usage`: GPU usage percentage (0-100), averaged across all GPUs
- `network_io`: Network throughput in Mbps
- `disk_activity`: Disk I/O in MB/s

#### `timing`
- `duration_threshold`: Minimum time metrics must exceed threshold before inhibiting
- `idle_duration`: Time all metrics must stay below threshold before releasing inhibition

## Command Line Arguments

```
Usage: rouser [OPTIONS]

Options:
  -c, --config <CONFIG>          Path to configuration file (sequential search if omitted)
      --validate-config          Validate configuration and exit without running
      --dry-run                  Dry run mode (collect metrics but don't inhibit sleep)
  -l, --log-level <LOG_LEVEL>    Set log level: debug, info, warn, error [overrides config + RUST_LOG]
  -h, --help                     Print help
  -V, --version                  Print version
```

For full CLI reference see [docs/command-line.md](docs/command-line.md).

## Running as a Service

### Systemd User Service (recommended)

rouser is installed as a **user service** — it runs under your user account without root:

```bash
# After running the installer script, or manually:
systemctl --user daemon-reload
systemctl --user enable --now rouser.service

# Check status
journalctl --user -u rouser -f
```

The service file is at `~/.config/systemd/user/rouser.service` and targets `$HOME/.local/bin/rouser`.

### Manual execution (not as a service)

```bash
# Run with default config search path
rouser

# Custom config, dry run for testing
rouser --dry-run -l debug

# Validate configuration before running
rouser --validate-config

# Stop the daemon: Ctrl+C or systemctl --user stop rouser.service
```

### logind lingering (required for user services when not logged in)

For systemd user services to persist after logout, enable lingering:

```bash
loginctl enable-linger $USER
```

Without lingering, `systemctl --user` only works while you have an active login session. The installer script provides guidance if systemctl reports issues with a non-active user session.

## Environment Variables

| Variable | Description | Affects |
|----------|-------------|---------|
| `RUST_LOG` | Logging level filter (e.g., `"debug"`, `"info"`, `"rouser=debug,zbus=info"`) | Console logging output only |

There are no `ROUSER_*` environment variable overrides for configuration values. All settings must come from the TOML file or be overridden at runtime via CLI flags (`-l/--log-level`).

## Troubleshooting

### Check active inhibitors
```bash
# List active sleep inhibitors
loginctl show-sessions | grep -i inhibited
systemd-inhibit --list
```

### Inhibition not working on KDE Plasma

KDE Powerdevil may ignore inhibitors from unprivileged users. Solutions:

1. **Add polkit rule** (recommended):
    Create `/etc/polkit-1/rules.d/50-rouser.rules`:
    ```javascript
    polkit.addRule(function(action, subject) {
        if (action.id == "org.freedesktop.login1.inhibit" &&
            subject.user == "your_username") {
            return polkit.Result.YES;
        }
    });
    ```

### Service logs not showing

When running as a systemd user service, use `--user` flag:

```bash
# Wrong (system journal — won't have user-level service)
journalctl -u rouser -f

# Correct (user journal)
journalctl --user -u rouser -f
```

## Architecture

```
┌──────────────┐    ┌───────────┐    ┌───────────┐
│ Config       │───▶│ Core      │◀───│ Metrics   │
│ Loader       │    │ Logic     │    │ Collectors│
└──────────────┘    └─────┬─────┘    └───────────┘
                          │
                     ┌────▼────┐
                     │Threshold│
                     │Manager  │
                     └────┬────┘
                          │
                     ┌────▼────┐
                     │Inhibitor│
                     └────┬────┘
                          │
                     org.freedesktop.login1
```

### Metrics Collection

- **CPU**: `/proc/stat` (system-wide)
- **GPU**: `nvidia-smi` (NVIDIA) or `/sys/class/drm/device/gpu_busy_percent` (AMD/Intel)
- **Network**: `/proc/net/dev`
- **Disk**: `/proc/diskstats`

### Inhibition Flow

1. Collect metrics every `update_interval` seconds
2. If any metric exceeds its threshold continuously for at least `duration_threshold`, acquire sleep inhibition lock via D-Bus
3. While inhibited, continue collecting metrics; brief drops below threshold do not release immediately — all metrics must stay below thresholds for the full `cooldown_duration` before releasing
4. Release inhibition lock (file descriptor closes automatically)

## Development

### Building

```bash
# Debug build
cargo build

# Release build (optimized)
cargo build --release
```

### Running Tests & Code Quality

All checks must pass before any commit. See [AGENTS.md](AGENTS.md) for the full checklist:

```bash
cargo fmt --check          # Ensure consistent formatting
cargo clippy --all-targets -- -D warnings  # Zero lint warnings allowed
cargo test --all-targets   # All unit tests passing
cargo build --release      # Release binary compiles successfully
```

### CI/CD Pipeline

The GitHub Actions workflow (`.github/workflows/ci.yml`) runs:
- **On push/PR**: format check, clippy lint, tests, debug builds for x86_64 + aarch64 with tarball artifacts
- **On release tag**: cross-compile releases, build DEB/RPM packages and Arch PKGBUILD, publish as GitHub Release assets

## Documentation

| Document | Description |
|----------|-------------|
| [docs/configuration.md](docs/configuration.md) | Full configuration reference with all options |
| [docs/command-line.md](docs/command-line.md) | CLI argument reference and examples |
| [docs/systemd-user-service.md](docs/systemd-user-service.md) | Running rouser as a systemd user service |
| [docs/metrics-overview.md](docs/metrics-overview.md) | How each metric type is collected |
| [docs/quickstart.md](docs/quickstart.md) | Step-by-step getting started guide |
| [AGENTS.md](AGENTS.md) | Developer guidelines, versioning policy, coding conventions |

