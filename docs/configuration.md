# Configuration Reference

This document describes all available configuration options for `rouser`.

## Overview

rouser uses a TOML configuration file to define thresholds, timing parameters, and daemon behavior.

**TOML vs YAML**: rouser uses TOML instead of YAML because:

- Pure Rust implementation with no C dependencies
- Native support via the `toml` crate
- Simpler, more readable format for configuration
- Better security (avoids RUSTSEC-2025-0068 vulnerability in YAML parsers)

## Configuration File

**Default Location**: `/etc/rouser/config.toml`

**Command Line Override**: `rouser --config /path/to/config.toml`

**Security Note**: The configuration file should have restricted permissions (mode `0600`, owned by root) to prevent unauthorized modification.

## Complete Configuration Example

```toml
# /etc/rouser/config.toml

# Daemon configuration
[daemon]
name = "rouser"
update_interval = "5s"
log_level = "info"

# Metric thresholds and EMA smoothing
[thresholds]
cpu_usage = 80.0
cpu_ema_alpha = 0.1
gpu_usage = 90.0
gpu_ema_alpha = 0.1
network_io = 100.0
network_io_ema_alpha = 0.1
disk_activity = 50.0
disk_activity_ema_alpha = 0.1

# Per-GPU thresholds (optional)
[[thresholds.gpu]]
name = "nvidia-0"
usage = 90.0
ema_alpha = 0.1

# Timing parameters
[timing]
duration_threshold = "30s"
idle_duration = "60s"
cooldown_duration = "60s"

# Inhibition settings
[inhibition]
what = "shutdown:idle"
mode = "block"

# Network interface configuration
[network]
exclude_interfaces = ["lo"]
include_interfaces = []

# Disk device configuration
[disk]
exclude_device_prefixes = ["loop", "fd", "sr", "cdrom"]

# Logging configuration
[logging]
file = "/var/log/rouser/rouser.log"
rotation.max_size_mb = 10
rotation.max_files = 5
rotation.compress = true
format = "text"

# Performance tuning
[performance]
max_metric_samples = 1000
```

## Daemon Configuration

### `[daemon]` Section

| Key | Type | Default | Description |
|-----|------|---------|-------------|
| `name` | string | `"rouser"` | Daemon name for logging |
| `update_interval` | duration | `"5s"` | Time between metric collection cycles |
| `log_level` | string | `"info"` | Logging level: debug, info, warn, error |

**Example**:

```toml
[daemon]
name = "rouser"
update_interval = "10s"
log_level = "debug"
```

## Threshold Configuration

### `[thresholds]` Section

| Key | Type | Default | Valid Range | Description |
|-----|------|---------|-------------|-------------|
| `cpu_usage` | float | `80.0` | 0.0 - 100.0 | CPU usage percentage |
| `cpu_ema_alpha` | float | `0.1` | 0.0 - 1.0 | EMA smoothing factor for CPU |
| `gpu_usage` | float | `90.0` | 0.0 - 100.0 | GPU usage percentage (average) |
| `gpu_ema_alpha` | float | `0.1` | 0.0 - 1.0 | EMA smoothing factor for GPU |
| `network_io` | float | `100.0` | 0.0+ | Network throughput in Mbps |
| `network_io_ema_alpha` | float | `0.1` | 0.0 - 1.0 | EMA smoothing factor for network |
| `disk_activity` | float | `50.0` | 0.0+ | Disk I/O in MB/s |
| `disk_activity_ema_alpha` | float | `0.1` | 0.0 - 1.0 | EMA smoothing factor for disk |

**Example**:

```toml
[thresholds]
cpu_usage = 85.0
cpu_ema_alpha = 0.15
gpu_usage = 95.0
gpu_ema_alpha = 0.2
network_io = 150.0
network_io_ema_alpha = 0.05
disk_activity = 75.0
disk_activity_ema_alpha = 0.1
```

### Per-GPU Configuration

For systems with multiple GPUs, you can configure each GPU individually:

```toml
[[thresholds.gpu]]
name = "nvidia-0"
usage = 85.0
ema_alpha = 0.15

[[thresholds.gpu]]
name = "nvidia-1"
usage = 90.0
ema_alpha = 0.1
```

This is useful when GPUs have different workloads or capabilities.

## Timing Configuration

### `[timing]` Section

| Key | Type | Default | Description |
|-----|------|---------|-------------|
| `duration_threshold` | duration | `"30s"` | Min time above threshold before inhibiting |
| `idle_duration` | duration | `"60s"` | Time below threshold before releasing |
| `cooldown_duration` | duration | `"60s"` | Time to wait after releasing before re-inhibiting |

**Example**:

```toml
[timing]
duration_threshold = "45s"
idle_duration = "90s"
cooldown_duration = "30s"
```

### Timing Parameters Explained

- **`duration_threshold`**: Prevents brief spikes from triggering inhibition. The metric must exceed threshold for this duration before sleep is inhibited.

- **`idle_duration`**: Provides hysteresis to prevent rapid inhibit/release cycling. All metrics must stay below threshold for this duration before releasing inhibition.

- **`cooldown_duration`**: Additional time after releasing inhibition during which the daemon won't re-inhibit even if thresholds are exceeded again. Helps with bursty workloads.

## Inhibition Configuration

### `[inhibition]` Section

| Key | Type | Default | Description |
|-----|------|---------|-------------|
| `what` | string | `"shutdown:idle"` | Lock types to inhibit (colon-separated) |
| `mode` | string | `"block"` | Inhibition mode: block, delay, block-weak |

**Example**:

```toml
[inhibition]
what = "sleep:suspend"
mode = "delay"
```

### Lock Types (`what`)

| Value | Description |
|-------|-------------|
| `idle` | Prevents idle suspend |
| `sleep` | Prevents sleep/hibernate (suspend-to-RAM) |
| `suspend` | Prevents suspend-to-RAM |
| `shutdown` | Prevents shutdown or reboot |

Combine with colons: `"shutdown:idle"` inhibits both shutdown and idle.

### Inhibition Modes (`mode`)

| Value | Description |
|-------|-------------|
| `block` | Completely blocks the operation |
| `delay` | Delays the operation for the duration of inhibition |
| `block-weak` | Blocks but can be overridden by privileged processes |

## Network Configuration

### `[network]` Section

| Key | Type | Default | Description |
|-----|------|---------|-------------|
| `exclude_interfaces` | array | `["lo"]` | Interfaces to exclude from monitoring |
| `include_interfaces` | array | `[]` | Only monitor these interfaces (empty = all) |

**Example**:

```toml
[network]
exclude_interfaces = ["lo", "docker0"]
include_interfaces = ["eth0", "ens192"]
```

**Loopback Interface**: By default, the loopback interface (`lo`) is excluded because:
- Loopback traffic is internal to the system
- External network activity is more relevant for sleep decisions
- Database replication or internal services may use loopback

## Disk Configuration

### `[disk]` Section

| Key | Type | Default | Description |
|-----|------|---------|-------------|
| `exclude_device_prefixes` | array | `["loop", "fd", "sr", "cdrom"]` | Device prefixes to exclude |

**Example**:

```toml
[disk]
exclude_device_prefixes = ["loop", "fd", "sr", "cdrom", "nbd"]
```

**Device Detection**:
- **Excluded** (virtual/simulated): `loop`, `fd` (file descriptor backends), `sr`, `cdrom`
- **Included** (real storage): `dm-` (LVM), `sdX`, `nvmeX`, `vdX` (virtio)

## Logging Configuration

### `[logging]` Section

| Key | Type | Default | Description |
|-----|------|---------|-------------|
| `file` | string | `null` | Log file path, `stdout`, or `stderr` |
| `format` | string | `"text"` | Log format: text or json |

**Example**:

```toml
[logging]
file = "/var/log/rouser/rouser.log"
format = "json"
```

### Log Rotation

```toml
[logging]
file = "/var/log/rouser/rouser.log"

[logging.rotation]
max_size_mb = 10
max_files = 5
compress = true
```

| Key | Type | Default | Description |
|-----|------|---------|-------------|
| `max_size_mb` | integer | `10` | Max log file size before rotation |
| `max_files` | integer | `5` | Number of rotated files to keep |
| `compress` | boolean | `true` | Compress old log files (gzip) |

## Performance Configuration

### `[performance]` Section

| Key | Type | Default | Description |
|-----|------|---------|-------------|
| `max_metric_samples` | integer | `1000` | Max historical samples to keep in memory |

**Example**:

```toml
[performance]
max_metric_samples = 500
```

## Environment Variable Overrides

Configuration values can be overridden via environment variables using the pattern `ROUSER_<SECTION>_<KEY>`:

```bash
export ROUSER_THRESHOLDS_CPU_USAGE=75
export ROUSER_DAEMON_LOG_LEVEL=debug
rouser
```

| Environment Variable | Config Path |
|---------------------|-------------|
| `ROUSER_DAEMON_NAME` | `daemon.name` |
| `ROUSER_DAEMON_UPDATE_INTERVAL` | `daemon.update_interval` |
| `ROUSER_DAEMON_LOG_LEVEL` | `daemon.log_level` |
| `ROUSER_THRESHOLDS_CPU_USAGE` | `thresholds.cpu_usage` |
| `ROUSER_THRESHOLDS_GPU_USAGE` | `thresholds.gpu_usage` |
| `ROUSER_THRESHOLDS_NETWORK_IO` | `thresholds.network_io` |
| `ROUSER_THRESHOLDS_DISK_ACTIVITY` | `thresholds.disk_activity` |
| `ROUSER_TIMING_DURATION_THRESHOLD` | `timing.duration_threshold` |
| `ROUSER_TIMING_IDLE_DURATION` | `timing.idle_duration` |
| `ROUSER_TIMING_COOLDOWN_DURATION` | `timing.cooldown_duration` |

## Validation

### Validate Configuration

Before deploying, validate your configuration:

```bash
rouser --validate-config /etc/rouser/config.toml
```

This command will:
- Check for missing required fields
- Validate value ranges and types
- Verify file paths exist and are accessible
- Report any syntax errors

### Testing Configuration

Test with a dry-run:

```bash
rouser --config /etc/rouser/config.toml --dry-run --duration 60s
```

This will:
- Parse and validate the configuration
- Collect metrics for the specified duration
- Log what would trigger inhibition
- Exit without inhibiting sleep

## Best Practices

1. **Start with conservative thresholds**: Begin with higher CPU/GPU thresholds (90%) and lower network/disk thresholds
2. **Monitor and adjust**: Use logging to understand your system's baseline activity before finalizing thresholds
3. **Use timing values**: Start with `duration_threshold` of 60s and `idle_duration` of 120s to avoid false positives
4. **Test before production**: Always use `--dry-run` mode to verify thresholds before deploying
5. **Review logs regularly**: Check for threshold triggers and adjust based on actual usage patterns
6. **Secure the configuration file**: Use mode `0600` and root ownership

## Error Handling

### Missing Metrics

If a metric source is unavailable:

- **CPU**: Always available via `/proc/stat`
- **GPU**: Falls back to 0% usage if `nvidia-smi` or `/sys/class/drm/` unavailable
- **Network**: Monitors only available interfaces (always at least `lo`)
- **Disk**: Monitors available devices; excludes virtual devices per configuration

### `/proc` Filesystem Unavailable

If the `/proc` filesystem is unavailable:

- Daemon will log an error and exit
- Systemd will restart the service according to `Restart=` policy

## References

- [TOML Specification](https://toml.io/en/v1.0.0)
- [Linux Kernel Documentation](https://www.kernel.org/doc/html/latest/)
- [systemd D-Bus API](https://www.freedesktop.org/software/systemd/man/org.freedesktop.login1.html)

## See Also

- [Quick Start Guide](quickstart.md) - Getting started with rouser
- [Command Line Arguments](command-line.md) - CLI usage details
- [Metrics Overview](metrics-overview.md) - How metrics are collected
- [Averaging Explained](averaging.md) - Understanding threshold calculations
