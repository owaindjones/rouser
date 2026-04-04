# Metrics Overview

This document describes how `rouser` collects system metrics from various sources.

## Overview

rouser monitors four main categories of system metrics:

| Metric | Source | Polling | Description |
|--------|--------|---------|-------------|
| **CPU Usage** | `/proc/stat` | Configurable (default: 5s) | Aggregate CPU usage across all cores |
| **GPU Usage** | `nvidia-smi` or `/sys/class/drm/` | Configurable (default: 5s) | GPU utilization percentage |
| **Network I/O** | `/proc/net/dev` | Configurable (default: 5s) | Network throughput in Mbps |
| **Disk Activity** | `/proc/diskstats` | Configurable (default: 5s) | Disk read/write in MB/s |

All metrics are collected asynchronously using Tokio's async runtime.

## CPU Usage

### Data Source: `/proc/stat`

rouser reads CPU statistics from the virtual `/proc/stat` file, which provides system-wide CPU usage information.

#### File Format

```
cpu  234567 12345 2345678 789012345 123456 1234 567 8901 0 0
cpu0 23456 1234 234567 789012 12345 123 56 789 0 0
cpu1 23456 1234 234567 789012 12345 123 56 789 0 0
```

The first line (starting with `cpu `) contains aggregate statistics for all cores. Subsequent lines (`cpu0`, `cpu1`, etc.) contain per-core statistics.

#### Fields (Verified Against Kernel Documentation)

| Field | Description |
|-------|-------------|
| `user` | Normal processes executing in user mode (jiffies) |
| `nice` | Niced processes executing in user mode (jiffies) |
| `system` | Processes executing in kernel mode (jiffies) |
| `idle` | Time spent in idle task (jiffies) |
| `iowait` | Time waiting for I/O to complete (jiffies) |
| `irq` | Time servicing hardware interrupts (jiffies) |
| `softirq` | Time servicing software interrupts (jiffies) |
| `steal` | Stolen time in virtual machines (jiffies) |
| `guest` | Time spent running guest OS (jiffies) |
| `guest_nice` | Time spent running nice guest OS (jiffies, Linux 2.6.24+) |

### Calculation Method

rouser uses a **two-sample delta** approach to calculate CPU usage:

1. Read current values from `/proc/stat`
2. Wait for polling interval (default: 5 seconds)
3. Read new values
4. Calculate the difference between samples
5. Compute usage percentage based on idle vs non-idle time

#### Formula

```
total_time = user + nice + system + idle + iowait + irq + softirq + steal + guest
non_idle_time = user + nice + system + iowait + irq + softirq + steal + guest
cpu_usage = (non_idle_time / total_time) * 100.0
```

**Note**: `guest_nice` is not included as it represents time already counted in `guest`.

### Edge Cases

#### First Sample

On the first read, there's no baseline for comparison. rouser:
- Initializes with the first sample
- Returns 0% usage for the first poll cycle
- Begins actual calculation from the second sample

#### Jiffies Overflow

On long-running systems, jiffies can overflow (wrap from `u64::MAX` back to 0). rouser handles this by:
- Using `u64` for jiffies (can run ~580 years at 100 Hz before overflow)
- Detecting wraparound via negative deltas
- Applying correction: `correction = u64::MAX - b + a + 1`

### Implementation

```rust
pub struct CpuCollector {
    last_stats: Option<CpuStats>,
    last_time: Option<SystemTime>,
}

impl CpuCollector {
    pub async fn collect_cpu_usage(&self) -> f64 {
        let stats = self.read_stats().await;
        let now = SystemTime::now();
        
        match (&self.last_stats, &self.last_time) {
            (Some(prev), Some(prev_time)) => {
                let delta_total = total_delta(&stats, prev);
                let delta_non_idle = non_idle_delta(&stats, prev);
                
                if delta_total > 0 {
                    (delta_non_idle as f64 / delta_total as f64) * 100.0
                } else {
                    0.0
                }
            }
            _ => {
                // First sample
                0.0
            }
        }
    }
}
```

## GPU Usage

### Data Sources by GPU Type

rouser supports multiple GPU vendors with different data sources.

#### NVIDIA GPUs

**Primary Source**: `nvidia-smi` command-line tool

```bash
# Query all GPUs
nvidia-smi --query-gpu=utilization.gpu --format=csv,noheader,nounits

# Sample output:
# 95%
# 78%
```

**Implementation**:
- Runs `nvidia-smi` command and parses output
- Extracts percentage values from each GPU
- Calculates average across all detected GPUs
- Falls back to 0% if `nvidia-smi` unavailable

#### AMD/Intel GPUs

**Primary Source**: `/sys/class/drm/cardX/device/gpu_busy_percent`

```bash
# Check GPU usage (if available)
cat /sys/class/drm/card0/device/gpu_busy_percent

# Or use rocm-smi tool (similar to nvidia-smi)
rocm-smi --showgpuutilization
```

**Requirements**:
- AMD ROCm drivers or Intel i915 driver installed
- `uvm` (Unified Virtual Memory) subsystem loaded
- Access to `/sys/class/drm/cardX/device/`

**Implementation**:
- Scans `/sys/class/drm/` directory for GPU devices
- Reads `gpu_busy_percent` from each device
- Calculates average across all detected GPUs
- Falls back to 0% if no GPUs detected

### Aggregation Strategy

#### Multiple GPUs

For systems with multiple GPUs, rouser calculates the **average** usage:

```rust
fn calculate_average(usage_values: &[f64]) -> f64 {
    if usage_values.is_empty() {
        0.0
    } else {
        usage_values.iter().sum::<f64>() / usage_values.len() as f64
    }
}
```

**Per-GPU Tracking**: rouser can also track each GPU individually via per-GPU configuration:

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

## Network I/O

### Data Source: `/proc/net/dev`

rouser reads network statistics from the virtual `/proc/net/dev` file.

#### File Format

```
Inter-|   Receive                                                |  Transmit
 face |bytes    packets errs drop fifo frame compressed multicast|bytes    packets errs drop fifo colls carrier compressed
    lo: 1234567   12345    0    0    0     0          0         0  1234567   12345    0    0    0     0       0          0
  eth0: 9876543  987654    0    0    0     0          0         0  8765432  876543    0    0    0     0       0          0
```

### Calculation Method

1. Read current byte/packet counts for each interface
2. Wait for polling interval
3. Read new values
4. Calculate delta (bytes transferred during interval)
5. Convert to Mbps: `(delta_bytes * 8) / interval_seconds / 1,000,000`

#### Formula

```
bytes_transferred = current_bytes - previous_bytes
interval_seconds = current_time - previous_time
network_io_mbps = (bytes_transferred * 8.0) / interval_seconds / 1_000_000.0
```

### Interface Filtering

rouser supports two filtering strategies:

#### Exclude Interfaces

By default, loopback (`lo`) is excluded:

```toml
[network]
exclude_interfaces = ["lo"]
```

#### Include Only Specific Interfaces

Monitor only specified interfaces:

```toml
[network]
include_interfaces = ["eth0", "ens192"]
```

**Note**: If both `include_interfaces` and `exclude_interfaces` are specified, `include_interfaces` takes precedence.

### Edge Cases

#### Interface Changes

If an interface disappears between polls:
- rouser continues with available interfaces
- Logs a warning at debug level
- Does not fail the metric collection

#### No Interfaces Available

If no interfaces are available (highly unlikely):
- Returns 0 Mbps network usage
- Logs an error
- Continues operation

## Disk Activity

### Data Source: `/proc/diskstats`

rouser reads disk statistics from the virtual `/proc/diskstats` file.

#### File Format

```
259:0 md0 rw 1000 5000 100000 500000 0 0 0 0 100 2000 3000
259:1 md1 rw 500 2500 50000 250000 0 0 0 0 50 1000 1500
```

Fields: `major minor name reads merged sectors writes merged sec_wait ected sectors`

### Calculation Method

1. Read current sector counts for each device
2. Wait for polling interval
3. Read new values
4. Calculate delta (sectors transferred)
5. Convert to MB/s: `(delta_sectors * 512) / interval_seconds / 1,000,000`

#### Formula

```
sectors_transferred = current_sectors - previous_sectors
bytes_transferred = sectors_transferred * 512  # 512 bytes per sector
interval_seconds = current_time - previous_time
disk_activity_mbps = (bytes_transferred as f64) / interval_seconds / 1_000_000.0
```

### Device Filtering

rouser excludes virtual/simulated devices by default:

```toml
[disk]
exclude_device_prefixes = ["loop", "fd", "sr", "cdrom"]
```

#### Excluded Devices

| Prefix | Type | Reason |
|--------|------|--------|
| `loop` | Loop device | Virtual block device for disk images |
| `fd` | File descriptor backend | Virtual block device |
| `sr` | SCSI CD-ROM | Optical drive (rarely active) |
| `cdrom` | CD-ROM | Optical drive (rarely active) |

#### Included Devices

| Prefix | Type | Notes |
|--------|------|-------|
| `sdX` | SATA/SAS disk | Real storage devices |
| `nvmeX` | NVMe SSD | Real storage devices |
| `dm-X` | Device mapper/LVM | LVM volumes (real storage) |
| `vdX` | VirtIO | KVM/virtio block devices |

### Edge Cases

#### Device Changes

If a device disappears between polls:
- rouser continues with available devices
- Logs a warning at debug level
- Does not fail the metric collection

#### No Devices Available

If no devices are available (highly unlikely):
- Returns 0 MB/s disk activity
- Logs an error
- Continues operation

## Collection Timing

### Polling Interval

The polling interval is configurable via `daemon.update_interval`:

```toml
[daemon]
update_interval = "5s"  # Default: 5 seconds
```

**Trade-offs**:
- Shorter interval: More responsive to activity, higher CPU usage
- Longer interval: Lower CPU usage, less responsive

### Asynchronous Collection

rouser uses Tokio's async runtime to collect metrics concurrently:

```rust
// Collect all metrics concurrently
let (cpu, gpu, network, disk) = tokio::join!(
    cpu_collector.collect(),
    gpu_collector.collect(),
    network_collector.collect(),
    disk_collector.collect()
);
```

This approach minimizes the time spent collecting metrics, reducing the impact on system performance.

## Error Handling

### Graceful Degradation

If a metric source is unavailable:

| Metric | Fallback Behavior |
|--------|-------------------|
| CPU | Always available via `/proc/stat` |
| GPU | Returns 0% usage, logs warning |
| Network | Monitors available interfaces, continues |
| Disk | Monitors available devices, continues |

### Logging

Errors are logged at appropriate levels:

```rust
// Warning for non-critical issues
warn!("GPU metrics unavailable, using 0%");

// Error for critical issues
error!("No disk devices available");
```

## Performance Considerations

### Resource Usage

| Metric | Memory Impact | CPU Impact | Disk I/O |
|--------|--------------|------------|----------|
| CPU | ~100 bytes | Negligible | None (/proc) |
| GPU | ~1 KB | Low (nvidia-smi spawn) | None |
| Network | ~100 bytes per interface | Negligible | None (/proc) |
| Disk | ~50 bytes per device | Negligible | None (/proc) |

### Optimization Tips

To reduce resource usage:

```toml
[daemon]
update_interval = "10s"  # Increase interval

[network]
exclude_interfaces = ["lo", "docker0", "virbr0"]  # Exclude virtual interfaces

[disk]
exclude_device_prefixes = ["loop", "fd", "sr", "cdrom", "nbd"]  # Exclude more virtual devices
```

## See Also

- [Averaging Explained](averaging.md) - Understanding threshold calculations with EMA
- [Configuration Reference](configuration.md) - Configuration options for metrics
