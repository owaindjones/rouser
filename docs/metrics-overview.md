# Metrics Overview

This document describes how `rouser` collects system metrics from various sources.

## Overview

rouser monitors four main categories of system metrics:

| Metric | Source | Polling | Description |
|--------|--------|---------|-------------|
| **CPU Usage** | `/proc/stat` | Top-level `update_interval` config | Per-core maximum and total average CPU usage (frequency-weighted) |
| **GPU Usage** | NVML (`libnvidia-ml.so`) or `/sys/class/drm/` | Configurable via top-level `update_interval` | GPU utilization percentage per device |
| **Network I/O** | `/proc/net/dev` | Configurable via top-level `update_interval` | Network throughput in Mbps |
| **Disk Activity** | `/proc/diskstats` | Configurable via top-level `update_interval` | Disk read/write in MB/s |

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
2. Wait for polling interval (set via top-level `update_interval`)
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

**Primary Source**: NVML library (`libnvidia-ml.so`) — the same API used by `nvidia-smi` and [nvtop](https://github.com/Syllo/nvtop)

```bash
# NVML is loaded dynamically from the NVIDIA driver package
# No separate binary required — accessed via libnvidia-ml.so.1
ldconfig -p | grep libnvidia-ml
```

**Implementation**:
- Dynamically loads `libnvidia-ml.so` at runtime (graceful fallback if unavailable)
- Enumerates GPUs by index, matches to sysfs cards via PCI bus ID
- Uses `nvmlDeviceGetUtilizationRates()` for per-GPU compute utilization
- Falls back to 0% if NVML is unavailable or GPU not supported

#### AMD/Intel GPUs

**Primary Source**: `/sys/class/drm/cardX/device/gpu_busy_percent`

```bash
# Check GPU usage (if available)
cat /sys/class/drm/card0/device/gpu_busy_percent

# AMD: rocm-smi provides similar data via ROCm tools
rocm-smi --showgpuutilization
```

**Requirements**:
- AMD amdgpu drivers or Intel i915/xe driver installed
- Access to `/sys/class/drm/cardX/device/gpu_busy_percent` (readable by the rouser process user)

**Implementation**:
- Scans `/sys/class/drm/` directory for GPU devices
- Reads `gpu_busy_percent` from each device
- Reports per-device utilization independently (no averaging)
- Falls back to 0% if no GPUs detected

### Driver Measurement Differences

NVML, amdgpu, and i915 all report a 0–100% value but measure different things under the hood. NVIDIA's SM kernel utilization, AMD's aggregate IP core activity via SMU firmware, and Intel's GT engine ticks are not directly comparable as percentages. See [GPU Usage Measurement](gpu-usage-measurement.md) for a detailed breakdown of what each driver reports and why this doesn't affect rouser's sleep inhibition behavior in practice.

### Aggregation Strategy — Per-Device Reporting Over Averaging

rouser reports each physical GPU **individually** rather than aggregating across devices. Each detected GPU is compared independently against the configured threshold:

```
card0(nvidia): 95%   ← above 90% threshold → inhibits sleep
card1(amdgpu): 78%  ← below 90% threshold → does not inhibit alone
```

A single GPU exceeding its threshold triggers inhibition regardless of other GPUs' states. This provides accurate per-GPU logging and prevents one low-usage card from masking a high-usage card's activity.

**EMA Smoothing**: Each device has independent EMA smoothing applied to its readings before comparison against the threshold. The `ema_alpha` value in `[metrics.gpu]` controls smoothing strength uniformly across all GPUs.

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

No interfaces are excluded by default — all available interfaces are monitored. To exclude specific interfaces (e.g., loopback):

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

Fields (simplified): `major minor name reads_merged sectors_read writes_merged sectors_written ...`

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

The polling interval is a top-level config field:

```toml
update_interval = "1s"   # Time between metric collection cycles (default)
log_level = "info"
...
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
| GPU | ~1 KB | Very low (NVML library call, no subprocess) | None |
| Network | ~100 bytes per interface | Negligible | None (/proc) |
| Disk | ~50 bytes per device | Negligible | None (/proc) |

### Optimization Tips

To reduce resource usage:

```toml
# Increase polling interval (configurable via top-level update_interval)
update_interval = "10s"   # Less responsive but lower overhead

[metrics.network]
exclude_interfaces = ["lo", "docker0", "virbr0"]   # Exclude virtual interfaces

[metrics.disk]
exclude_device_prefixes = ["loop", "fd", "sr", "cdrom", "nbd"]  # Exclude more virtual devices
```

## See Also

- [GPU Usage Measurement](gpu-usage-measurement.md) — Driver measurement differences (NVML vs amdgpu vs i915/xe)
- [Averaging Explained](averaging.md) - Understanding threshold calculations with EMA
- [Configuration Reference](configuration.md) - Configuration options for metrics
