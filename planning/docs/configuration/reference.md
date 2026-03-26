# Configuration Reference

## Overview

`rouser` uses a TOML configuration file to define thresholds, timing parameters, and daemon behavior. This document describes all available configuration options.

**TOML vs YAML**: `rouser` uses TOML (Tom's Obvious, Minimal Language) instead of YAML because:

- Pure Rust implementation with no C dependencies
- Native support via the `toml` crate
- Simpler, more readable format for configuration
- Better maintained in the Rust ecosystem (avoids security issues in YAML parsers like RUSTSEC-2025-0068)

## Configuration File

**Default Location**: `/etc/rouser/config.toml`

**Command Line Override**: `rouser --config /path/to/config.toml`

**Security Note**: The configuration file should have restricted permissions (mode `0600`, owned by root) to prevent unauthorized modification. See [SECURITY.md](security.md) for details.

## Complete Configuration Example

```toml
# /etc/rouser/config.toml

# Daemon configuration
[daemon]
name = "rouser"
update_interval = "5s"
log_level = "info"

# Metric thresholds (percentages and bytes/second)
[thresholds]
cpu_usage = 80.0
gpu_usage = 90.0
network_io = 100.0  # Mbps
disk_activity = 50.0  # MB/s

# Timing parameters
[timing]
duration_threshold = "30s"
idle_duration = "60s"

# D-Bus inhibition settings
[inhibition]
what = ["sleep", "hibernate", "shutdown"]
mode = "block"

# Network interface configuration
[network]
# Interfaces to monitor (empty = all interfaces)
include_interfaces = []
# Interfaces to explicitly exclude
exclude_interfaces = ["lo"]  # Exclude loopback by default

# Disk device configuration
[disk]
# Device prefixes to exclude (e.g., loop devices, virtual devices)
exclude_device_prefixes = ["loop", "fd", "sr", "cdrom"]
# Note: dm- (device mapper/LVM) devices are included by default

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

# Environment-specific overrides (optional)
# Values can be overridden via environment variables
[environment]
# Example: ROUSER_CPU_THRESHOLD=75
# environment variables can override any config value
```

## Threshold Configuration

### CPU Usage Threshold

```toml
[thresholds]
cpu_usage = 80.0
```

- **Valid Range**: 0.0 - 100.0
- **Default**: 80.0
- **Description**: Aggregate CPU usage percentage across all cores
- **Calculation**: Based on non-idle time from `/proc/stat`

**Example**: If CPU is at 75% usage for 35 seconds (exceeding `duration_threshold` of 30s), sleep will be inhibited.

### GPU Usage Threshold

```toml
[thresholds]
gpu_usage = 90.0
```

- **Valid Range**: 0.0 - 100.0
- **Default**: 90.0
- **Requirements**: NVIDIA GPU with `nvidia-smi` or AMD/Intel GPU with `/sys/class/drm/` support
- **Note**: On systems with multiple GPUs, the average usage across all detected GPUs is used

**Hardware Support**:
- **NVIDIA**: `nvidia-smi` query (enterprise and consumer GPUs)
- **AMD**: `/sys/class/drm/cardX/device/gpu_busy_percent`
- **Intel**: `/sys/class/drm/cardX/device/gpu_busy_percent` (requires `uvm` subsystem)

### Network I/O Threshold

```toml
[thresholds]
network_io = 100.0
```

- **Valid Range**: 0.0 - infinity
- **Default**: 100.0 (Mbps)
- **Description**: Network throughput threshold
- **Unit**: Megabits per second (Mbps)

**Example**: A sustained network throughput of 101 Mbps for longer than `duration_threshold` will inhibit sleep.

### Disk Activity Threshold

```toml
[thresholds]
disk_activity = 50.0
```

- **Valid Range**: 0.0 - infinity
- **Default**: 50.0 (MB/s)
- **Description**: Disk read/write throughput
- **Unit**: Megabytes per second (MB/s)
- **Calculation**: Aggregated I/O from all monitored devices

## Timing Configuration

### Duration Threshold

```toml
[timing]
duration_threshold = "30s"
```

- **Valid Range**: 0s - infinity (e.g., "10s", "1m", "5m")
- **Default**: 30s
- **Purpose**: Prevents brief CPU/network/disk spikes from triggering sleep inhibition
- **Behavior**: The metric must exceed the threshold for at least this duration before inhibition is triggered

**Use Case**: Prevents short video rendering spikes or network bursts from keeping the system awake unnecessarily.

### Idle Duration

```toml
[timing]
idle_duration = "60s"
```

- **Valid Range**: 0s - infinity
- **Default**: 60s
- **Purpose**: Hysteresis to prevent rapid inhibit/release cycling
- **Behavior**: After metrics drop below threshold, the daemon waits this duration before releasing the inhibition lock

**Example**: CPU drops from 85% to 40% (below threshold). The daemon waits 60 seconds (idle_duration). If metrics stay below threshold, sleep inhibition is released.

## D-Bus Inhibition Configuration

### What to Inhibit

```toml
[inhibition]
what = ["sleep", "hibernate", "shutdown"]
```

- **Type**: Array of strings
- **Valid Values**:
  - `sleep`: Suspend-to-RAM (ACPI S3 state)
  - `hibernate`: Suspend-to-disk (ACPI S4 state)
  - `shutdown`: Shutdown or reboot operations
  - `idle`: Idle operations (varies by system)
- **Note**: Multiple values can be specified (array in TOML)

### Inhibition Mode

```toml
[inhibition]
mode = "block"
```

- **Valid Values**:
  - `block`: Completely block the operation (default) - equivalent to `InhibitLock()`
  - `delay`: Delay the operation for a short period - equivalent to `InhibitDelayMaxUSec()`
  - `interact`: Require user interaction before proceeding - equivalent to `InhibitInteractively()`

## Network Configuration

### Interface Filtering

```toml
[network]
# Interfaces to explicitly exclude
exclude_interfaces = ["lo"]
```

**Loopback Interface**: By default, the loopback interface (`lo`) is excluded from monitoring because:

- Loopback traffic is internal to the system
- External network activity is more relevant for sleep inhibition decisions
- Database replication or internal services may use loopback

**Customization**: To include loopback traffic in monitoring:

```toml
[network]
exclude_interfaces = []  # Empty array = monitor all interfaces
```

**Virtual Interfaces**: Docker bridges (`docker0`), tunnels (`tun0`), and virtual network interfaces are included by default.

## Disk Configuration

### Device Filtering

```toml
[disk]
exclude_device_prefixes = ["loop", "fd", "sr", "cdrom"]
```

**Virtual Device Detection**:

- **Excluded** (truly virtual/simulated): `loop`, `fd` (file descriptor backends), `sr`, `cdrom`
- **Included** (real storage): `dm-` (device mapper/LVM), `sdX`, `nvmeX`, `vdX` (KVM/virtio)
- **Rationale**: LVM volumes (`dm-`) represent real storage devices and should contribute to disk activity metrics

**Device Name Stability**: `/proc/diskstats` reports device names which may change across reboots for some devices. `rouser` handles this by monitoring by major:minor numbers rather than names.

## Logging Configuration

### Log File

```toml
[logging]
file = "/var/log/rouser/rouser.log"
```

- **Options**: File path (e.g., `/var/log/rouser/rouser.log`), `stdout`, `stderr`
- **Permissions**: Log file should be readable by the logging user/group
- **Default**: If not specified, logs to `stdout`

### Log Rotation

```toml
[logging]
rotation.max_size_mb = 10
rotation.max_files = 5
rotation.compress = true
```

- **max_size_mb**: Maximum log file size before rotation (in MB)
- **max_files**: Number of rotated files to keep
- **compress**: Compress old log files (gzip)

### Log Format

```toml
[logging]
format = "text"  # or "json"
```

- **Valid Values**: `text`, `json`
- **text**: Human-readable format (default)
- **json**: Structured JSON lines for log aggregation

## Performance Tuning

### Metric Samples

```toml
[performance]
max_metric_samples = 1000
```

- **Valid Range**: 100 - 10000
- **Default**: 1000
- **Purpose**: Maximum number of historical samples to keep in memory
- **Memory Impact**: ~8KB per 1000 samples (8 bytes per sample)

### Update Interval

```toml
[daemon]
update_interval = "5s"
```

- **Valid Range**: 1s - 60s
- **Default**: 5s
- **Trade-offs**:
  - Shorter interval: More responsive to activity, higher CPU usage
  - Longer interval: Lower CPU usage, less responsive
- **Recommended**: 5s balances responsiveness and resource usage

## Environment Variable Overrides

Configuration values can be overridden via environment variables using the pattern `ROUSER_<SECTION>_<KEY>`:

```bash
export ROUSER_THRESHOLDS_CPU_USAGE=75
export ROUSER_DAEMON_LOG_LEVEL=debug
rouser
```

**Supported Overrides**:

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

## Validation

### Configuration Validation

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

Test configuration with a dry-run:

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
3. **Use longer timing values**: Start with `duration_threshold` of 60s and `idle_duration` of 120s to avoid false positives
4. **Test before production**: Always use `--dry-run` mode to verify thresholds before deploying
5. **Review logs regularly**: Check for threshold triggers and adjust based on actual usage patterns
6. **Secure the configuration file**: Use mode `0600` and root ownership (see [SECURITY.md](security.md))

## Error Handling

### Missing Metrics

If a metric source is unavailable:

- **CPU**: Always available via `/proc/stat`
- **GPU**: Falls back to 0% usage if `nvidia-smi` or `/sys/class/drm/` unavailable
- **Network**: Monitors only available interfaces (always at least `lo`)
- **Disk**: Monitors available devices; excludes virtual devices per configuration

### `/proc` Filesystem Unavailable

If the `/proc` filesystem is unavailable (highly unlikely):

- Daemon will log an error and exit
- Systemd will restart the service according to `Restart=` policy

## References

- [TOML Specification](https://toml.io/en/v1.0.0)
- [Linux Kernel Documentation](https://www.kernel.org/doc/html/latest/)
- [systemd D-Bus API](https://www.freedesktop.org/software/systemd/man/org.freedesktop.login1.html)
- [zbus Rust Documentation](https://docs.rs/zbus/)
- [Rust Security Advisories](https://rustsec.org/)

## See Also

- [Architecture Overview](../architecture/overview.md)
- [D-Bus Inhibition API](../d-bus/inhibition.md)
- [Security Best Practices](../security.md)
- [Quick Start Guide](../quickstart.md)
