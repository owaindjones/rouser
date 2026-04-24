# Configuration Reference

This document describes all available configuration options for `rouser`.

## Overview

rouser uses a TOML configuration file to define thresholds, timing parameters, and daemon behavior. It supports **sequential config path resolution**: when no `-c` flag is given, rouser looks for the first existing file in this order:

1. `./config/rouser.toml` (repo-packaged default)
2. `$HOME/.config/rouser/config.toml` (XDG user config)
3. `/etc/rouser/config.toml` (system-wide config)

**TOML over YAML**: rouser uses TOML instead of YAML because it is a pure Rust implementation with no C dependencies and avoids known vulnerabilities in YAML parsers.

## Configuration File Discovery

| Method | Path / Flag |
|--------|-------------|
| CLI override | `rouser --config /path/to/config.toml` or `-c /path/to/config.toml` |
| Default search (sequential) | `./config/rouser.toml` → `~/.config/rouser/config.toml` → `/etc/rouser/config.toml` |

When none of the default paths exist, rouser uses built-in defaults and logs a warning:

```
No configuration file found at checked paths — using built-in defaults. Checked: ./config/rouser.toml, ~/.config/rouser/config.toml, /etc/rouser/config.toml
```

## Complete Configuration Example

```toml
# rouser config - see docs/configuration.md for full reference

name = "rouser"
update_interval = "5s"
log_level = "info"

[metrics.cpu]
threshold = 80.0    # CPU usage percentage (0-100) above which to inhibit sleep
ema_alpha = 0.3     # Exponential moving average smoothing factor (0.0–1.0, higher = less smoothed)

[metrics.gpu]
threshold = 90.0    # GPU usage percentage (0-100) above which to inhibit sleep
ema_alpha = 0.3     # EMA smoothing for per-GPU readings

[metrics.network]
threshold = 100.0       # Network throughput in Mbps
ema_alpha = 0.2         # EMA smoothing for network I/O
exclude_interfaces = ["lo"]    # Exclude from monitoring (default: loopback)
include_interfaces = []        # Only monitor these; empty means all

[metrics.disk]
threshold = 50.0              # Disk I/O in MB/s above which to inhibit sleep
ema_alpha = 0.2               # EMA smoothing for disk activity
exclude_device_prefixes = ["loop", "fd", "sr", "cdrom"]  # Exclude virtual devices

[timing]
duration_threshold = "30s"   # Min time metrics must exceed threshold before inhibiting
cooldown_duration = "60s"    # Time after releasing inhibition before re-inhibiting possible

[inhibitor]
what = "shutdown:idle"       # Lock types to inhibit (colon-separated)
mode = "block"               # Inhibition mode: block, delay, or block-weak
```

### Minimal Configuration

With no config file at all, rouser uses these built-in defaults:

```toml
name = "rouser"
update_interval = "5s"
log_level = "info"

[metrics.cpu]
threshold = 80.0
ema_alpha = 0.3

[metrics.gpu]
threshold = 90.0
ema_alpha = 0.3

[metrics.network]
threshold = 100.0
ema_alpha = 0.2
exclude_interfaces = ["lo"]
include_interfaces = []

[metrics.disk]
threshold = 50.0
ema_alpha = 0.2
exclude_device_prefixes = ["loop", "fd", "sr", "cdrom"]

[timing]
duration_threshold = "30s"
cooldown_duration = "60s"

[inhibitor]
what = "shutdown:idle"
mode = "block"
```

## Root-Level Options

| Key | Type | Default | Description |
|-----|------|---------|-------------|
| `name` | string | `"rouser"` | Daemon name for logging (reserved for future use) |
| `update_interval` | duration | `"5s"` | Time between metric collection cycles. Uses humantime format: `"1s"`, `"30s"`, `"5m"` |
| `log_level` | string | `"info"` | Logging level: `debug`, `info`, `warn`, `error`. Can also be set via `-l/--log-level` CLI flag or `RUST_LOG` env var |

## Metrics Configuration

### `[metrics.cpu]` — CPU Usage Threshold

| Key | Type | Default (0–100) | Description |
|-----|------|-----------------|-------------|
| `threshold` | f64 | `80.0` | CPU usage percentage above which to inhibit sleep |
| `ema_alpha` | f64 | `0.3` | EMA smoothing factor: higher = more responsive, lower = smoother readings |

### `[metrics.gpu]` — GPU Usage Threshold

Per-device GPU collection (NVIDIA via `nvidia-smi`, AMD/Intel via sysfs). Each detected GPU is compared independently against this threshold.

| Key | Type | Default (0–100) | Description |
|-----|------|-----------------|-------------|
| `threshold` | f64 | `90.0` | GPU usage percentage above which to inhibit sleep |
| `ema_alpha` | f64 | `0.3` | EMA smoothing factor for per-GPU readings |

### `[metrics.network]` — Network Throughput Threshold

Network I/O is calculated as total bytes transferred (in + out) across monitored interfaces, converted to Mbps.

| Key | Type | Default | Description |
|-----|------|---------|-------------|
| `threshold` | f64 | `100.0` | Network throughput in Mbps above which to inhibit sleep |
| `ema_alpha` | f64 | `0.2` | EMA smoothing factor for network I/O |
| `exclude_interfaces` | array of strings | `["lo"]` | Interface names to exclude from monitoring |
| `include_interfaces` | array of strings | `[]` | If non-empty, only monitor these interfaces; empty means all available interfaces |

### `[metrics.disk]` — Disk I/O Threshold

Disk activity is calculated as total bytes transferred across monitored devices (read + write sectors × 512 bytes), converted to MB/s.

| Key | Type | Default | Description |
|-----|------|---------|-------------|
| `threshold` | f64 | `50.0` | Disk I/O in MB/s above which to inhibit sleep |
| `ema_alpha` | f64 | `0.2` | EMA smoothing factor for disk activity |
| `exclude_device_prefixes` | array of strings | `["loop", "fd", "sr", "cdrom"]` | Device name prefixes to exclude (e.g., `loop*`, `fd*`) |

## Timing Configuration

### `[timing]` Section

| Key | Type | Default | Description |
|-----|------|---------|-------------|
| `duration_threshold` | duration | `"30s"` | Minimum continuous time metrics must exceed threshold before inhibiting sleep. Prevents brief spikes from triggering inhibition. |
| `cooldown_duration` | duration | `"60s"` | Time after releasing inhibition during which the daemon won't re-inhibit even if thresholds are exceeded again. Helps with bursty workloads. |

**Note**: There is no `idle_duration` field — the cooldown mechanism replaces it. A metric exceeding threshold for at least `duration_threshold` triggers inhibition; all metrics below their respective thresholds for at least `cooldown_duration` releases inhibition.

## Inhibition Configuration

### `[inhibitor]` Section

| Key | Type | Default | Description |
|-----|------|---------|-------------|
| `what` | string | `"shutdown:idle"` | Lock types to inhibit (colon-separated). Values: `idle`, `sleep`, `suspend`, `shutdown`. Multiple values combined with colons, e.g., `"sleep:suspend"`. |
| `mode` | string | `"block"` | Inhibition mode. Values: `block` (completely blocks sleep), `delay` (delays sleep for duration of inhibition), `block-weak` (blocks but can be overridden by privileged processes). |

## Configuration File Security

The configuration file may contain sensitive information depending on your deployment. When installed via the system-wide path `/etc/rouser/config.toml`, restrict permissions:

```bash
sudo chmod 0600 /etc/rouser/config.toml
```

User-level config files (`~/.config/rouser/config.toml`) inherit standard home directory permissions.

## Validation

### Validate Configuration via CLI

```bash
# With default paths (sequential search)
rouser --validate-config

# With explicit path
rouser -c /etc/rouser/config.toml --validate-config
```

Output on success: `Configuration validation passed`
Output on failure: `Configuration validation failed: <error details>`

### Dry Run Testing

Test configuration with live metric collection without inhibiting sleep:

```bash
# Collect metrics indefinitely in dry-run mode
rouser -c /etc/rouser/config.toml --dry-run

# With debug logging to see per-device readings
RUST_LOG=debug rouser -c /etc/rouser/config.toml --dry-run
```

## Environment Variables

| Variable | Description | Precedence Over |
|----------|-------------|-----------------|
| `RUST_LOG` | Logging level filter (e.g., `"debug"`, `"info"`) | config.log_level and `-l/--log-level` is higher priority than both |
| `-l, --log-level <LEVEL>` | CLI log level override | config.log_level only |

There are no `ROUSER_*` environment variable overrides for configuration values — all settings must come from the TOML file or be overridden at runtime via CLI flags.

## Best Practices

1. **Start with conservative thresholds**: Begin with higher CPU/GPU thresholds (90%) and lower network/disk thresholds
2. **Use EMA smoothing**: Default alpha values (0.3 for CPU/GPU, 0.2 for network/disk) provide a good balance between responsiveness and noise filtering
3. **Test before production**: Always use `--dry-run` mode to verify thresholds before deploying in daemon mode
4. **Review logs regularly**: Use debug logging (`RUST_LOG=debug`) to understand your system's baseline activity before finalizing thresholds
