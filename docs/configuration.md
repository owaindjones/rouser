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

When none of the default paths exist, rouser uses embedded defaults from `config/rouser.toml` and logs a warning:

```
No configuration file found at checked paths — using embedded defaults. Checked: ./config/rouser.toml, ~/.config/rouser/config.toml, /etc/rouser/config.toml
```

## Complete Configuration Example

The following is the default configuration shipped with `rouser` (from `config/rouser.toml`). Copy this file and adjust as needed. This TOML file is embedded at compile time via `include_str!()` and serves as both the shipped config file and the binary's built-in fallback.

```toml
# rouser default configuration — copy this file and adjust as needed.
# See https://github.com/owaindjones/rouser for documentation on all configuration options.

update_interval = "1s"
log_level = "info"

[metrics.cpu]
per_core_threshold = 80.0
total_threshold = 25.0
ema_alpha = 0.7

[metrics.gpu]
threshold = 15.0      # GPU usage threshold (percentage)
ema_alpha = 0.7       # EMA smoothing factor

[metrics.network]
threshold = 10.0      # Network I/O threshold (Mbps)
ema_alpha = 0.5       # EMA smoothing factor (lower for network to avoid spikes)
exclude_interfaces = ["lo"]
include_interfaces = []

[metrics.disk]
threshold = 10.0      # Disk I/O threshold (MB/s)
ema_alpha = 0.5       # EMA smoothing factor
exclude_device_prefixes = ["loop", "fd", "sr", "cdrom"]

# Timing configuration
[timing]
duration_threshold = "5s"    # Min time above threshold before inhibiting sleep
cooldown_duration = "10s"     # Time below threshold before releasing inhibition

[prediction]
update_interval = "30s"      # Seconds between averaged snapshots; must be >= root update_interval
history_length = "30d"       # Keep this much historical data; older entries pruned periodically
max_extension_time = "1h"    # Maximum additional time for predictive cooldown extension

[inhibitor]
what = "shutdown:idle"     # Lock type: idle, sleep, suspend, shutdown (colon-separated)
mode = "block"             # Mode: block, delay, block-weak
```

## Root-Level Options

| Key | Type | Default | Description |
|-----|------|---------|-------------|
| `update_interval` | duration | `"1s"` | Time between metric collection cycles. Uses humantime format: `"1s"`, `"30s"`, `"5m"` |
| `log_level` | string | `"info"` | Logging level: `debug`, `info`, `warn`, `error`. Can also be set via `-l/--log-level` CLI flag or `RUST_LOG` env var |

## Metrics Configuration

### `[metrics.cpu]` — CPU Usage Thresholds

CPU usage is measured per-core (frequency-weighted from sysfs cpufreq data) and as a total average. Both thresholds must be considered independently.

| Key | Type | Default (0–100) | Description |
|-----|------|-----------------|-------------|
| `per_core_threshold` | f64 | `80.0` | Per-core CPU usage percentage above which to inhibit sleep |
| `total_threshold` | f64 | `25.0` | Total (averaged across cores) CPU usage percentage above which to inhibit sleep |
| `ema_alpha` | f64 | `0.7` | EMA smoothing factor: higher = more responsive, lower = smoother readings |

### `[metrics.gpu]` — GPU Usage Threshold

Per-device GPU collection (NVIDIA via NVML, AMD/Intel via sysfs). Each detected GPU is compared independently against this threshold.

| Key | Type | Default (0–100) | Description |
|-----|------|-----------------|-------------|
| `threshold` | f64 | `15.0` | GPU usage percentage above which to inhibit sleep |
| `ema_alpha` | f64 | `0.7` | EMA smoothing factor for per-GPU readings |

### `[metrics.network]` — Network Throughput Threshold

Network I/O is calculated as total bytes transferred (in + out) across monitored interfaces, converted to Mbps.

| Key | Type | Default | Description |
|-----|------|---------|-------------|
| `threshold` | f64 | `10.0` | Network throughput in Mbps above which to inhibit sleep |
| `ema_alpha` | f64 | `0.5` | EMA smoothing factor for network I/O (lower for network to avoid spikes) |
| `exclude_interfaces` | array of strings | `["lo"]` | Interface names to exclude from monitoring; loopback excluded by default |
| `include_interfaces` | array of strings | `[]` | If non-empty, only monitor these interfaces; empty means all available interfaces |

### `[metrics.disk]` — Disk I/O Threshold

Disk activity is calculated as total bytes transferred across monitored devices (read + write sectors × 512 bytes), converted to MB/s.

| Key | Type | Default | Description |
|-----|------|---------|-------------|
| `threshold` | f64 | `10.0` | Disk I/O in MB/s above which to inhibit sleep |
| `ema_alpha` | f64 | `0.5` | EMA smoothing factor for disk activity |
| `exclude_device_prefixes` | array of strings | `["loop", "fd", "sr", "cdrom"]` | Device name prefixes to exclude (e.g., `loop*`, `fd*`) |

## Timing Configuration

### `[timing]` Section

| Key | Type | Default | Description |
|-----|------|---------|-------------|
| `duration_threshold` | duration | `"5s"` | Minimum continuous time metrics must exceed threshold before inhibiting sleep. Prevents brief spikes from triggering inhibition. |
| `cooldown_duration` | duration | `"10s"` | Time after releasing inhibition during which the daemon won't re-inhibit even if thresholds are exceeded again. Helps with bursty workloads. |

**Note**: There is no `idle_duration` field — the cooldown mechanism replaces it. A metric exceeding threshold for at least `duration_threshold` triggers inhibition; all metrics below their respective thresholds for at least `cooldown_duration` releases inhibition. See [d-bus-inhibition.md](d-bus-inhibition.md) for details on how inhibition works.

## Prediction Configuration

### `[prediction]` Section — Adaptive Cooldown Extension

The prediction module learns from historical system metric patterns over days and weeks, then dynamically extends the post-idle cooldown duration when patterns indicate likely continued active use at the current time of day. This reduces false-positive sleep inhibition during typical work hours while still allowing sleep during known idle periods (e.g., late nights). See [prediction-model.md](prediction-model.md) for a detailed explanation of how the model works.

| Key | Type | Default | Description |
|-----|------|---------|-------------|
| `update_interval` | duration | `"30s"` | Seconds between averaged snapshots written to history log. Must be greater than or equal to the root `update_interval`. Metrics from each tick are accumulated and averaged, then a single snapshot is flushed every N ticks where N = update_interval / root_update_interval. Set to `"0s"` to disable prediction entirely. |
| `history_length` | duration | `"30d"` | Amount of historical data to retain. Older entries and files are pruned automatically. Uses humantime format: `"7d"`, `"30d"`, `"90d"` |
| `max_extension_time` | duration | `"1h"` | Maximum additional time added to the cooldown duration by prediction. The model will never extend beyond this cap, even if historical patterns suggest it. Uses humantime format: `"5m"`, `"30m"`, `"1h"` |

**Data storage**: Historical data is stored as binary files (`history.log.YYYYMMDD`) using bincode v2 serialization under `$XDG_DATA_HOME/rouser/` (or `/var/lib/rouser/` when running as root). Files are date-partitioned for efficient pruning.

## Inhibition Configuration

### `[inhibitor]` Section

| Key | Type | Default | Description |
|-----|------|---------|-------------|
| `what` | string | `"shutdown:idle"` | Lock types to inhibit (colon-separated). Values: `idle`, `sleep`, `suspend`, `shutdown`. Multiple values combined with colons, e.g., `"sleep:suspend"`. See [d-bus-inhibition.md](d-bus-inhibition.md) for lock type details. |
| `mode` | string | `"block"` | Inhibition mode. Values: `block` (completely blocks sleep), `delay` (delays sleep for duration of inhibition), `block-weak` (blocks but can be overridden by privileged processes). See [d-bus-inhibition.md](d-bus-inhibition.md) for more on inhibition modes. |

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

Test configuration with live metric collection without inhibiting sleep. See [command-line.md](command-line.md) for full CLI reference.

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

1. **Start with conservative thresholds**: Begin with higher per-core CPU (80%) and GPU (15%) thresholds, then lower them based on observed baselines from dry-run logs
2. **Use EMA smoothing**: Default alpha values provide a good balance between responsiveness and noise filtering for your workload
3. **Test before production**: Always use `--dry-run` mode to verify thresholds before deploying in daemon mode
4. **Review logs regularly**: Use debug logging (`RUST_LOG=debug`) to understand your system's baseline activity before finalizing thresholds

## See Also

- [Command Line Reference](command-line.md) — All CLI arguments and usage examples
- [Metrics Overview](metrics-overview.md) — How CPU, GPU, network, and disk metrics are collected
- [Prediction Model](prediction-model.md) — How adaptive cooldown extension works from historical patterns
- [D-Bus Inhibition](d-bus-inhibition.md) — How sleep inhibition works under the hood
