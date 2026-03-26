# CPU Metrics

## Overview

The `rouser` daemon collects CPU usage metrics by reading data from the `/proc/stat` file. This provides aggregate CPU usage across all processor cores on the system.

## Metric Details

### Primary Metric

| Metric | Type | Unit | Description |
|--------|------|------|-------------|
| `cpu_usage` | Percentage | 0.0 - 100.0 | Aggregate CPU usage across all cores |

## Data Source: `/proc/stat`

### File Format

The `/proc/stat` file contains system-wide CPU statistics in a specific text format. Here's a sample excerpt:

```
cpu  234567 12345 2345678 789012345 123456 1234 567 8901 0 0
cpu0 23456 1234 234567 789012 12345 123 56 789 0 0
cpu1 23456 1234 234567 789012 12345 123 56 789 0 0
...
```

### Field Descriptions (Verified Against Kernel Documentation)

#### System-wide CPU line (first line starting with `cpu `)

The field order matches the official kernel documentation at [Linux Kernel CPU Load](https://www.kernel.org/doc/html/latest/admin-guide/cpu-load.html):

| Field Index | Name | Description |
|-------------|------|-------------|
| 1 | `user` | Normal processes executing in user mode (jiffies) |
| 2 | `nice` | Niced processes executing in user mode (jiffies) |
| 3 | `system` | Processes executing in kernel mode (jiffies) |
| 4 | `idle` | Time spent in idle task (jiffies) |
| 5 | `iowait` | Time waiting for I/O to complete (jiffies) |
| 6 | `irq` | Time servicing hardware interrupts (jiffies) |
| 7 | `softirq` | Time servicing software interrupts (jiffies) |
| 8 | `steal` | Stolen time in virtual machines (jiffies) |
| 9 | `guest` | Time spent running guest OS (jiffies) |
| 10 | `guest_nice` | Time spent running nice guest OS (jiffies, Linux 2.6.24+) |

**Important**: Fields 9 and 10 (guest, guest_nice) are Linux 2.6.24+ additions and may be absent on older systems.

#### Per-core lines (lines starting with `cpu0`, `cpu1`, etc.)

Same field structure as system-wide CPU line, but for individual CPU cores.

### Calculation of CPU Usage

To calculate CPU usage percentage, we use a two-sample polling approach:

1. **Read initial values** at time `t1`
2. **Wait for interval** (configurable, default 5 seconds)
3. **Read final values** at time `t2`
4. **Calculate differences** between t1 and t2
5. **Compute percentage** based on idle vs non-idle time

#### Formula

```rust
// Total time is sum of all CPU states (excluding guest_nice)
total_time = user + nice + system + idle + iowait + irq + softirq + steal + guest

// Non-idle time (time spent doing work)
non_idle_time = user + nice + system + iowait + irq + softirq + steal + guest

// CPU usage percentage
cpu_usage = ((non_idle_time / total_time) * 100.0)
```

**Note**: The `guest_nice` field is not included in the calculation as it represents time already counted in `guest`.

### Implementation Approach

#### Two-Poll Method (Recommended)

```rust
pub struct CpuCollector {
    last_stats: Option<CpuStats>,
    last_time: Option<SystemTime>,
}

impl CpuCollector {
    pub async fn collect_cpu_usage(&self) -> Result<f64> {
        let stats = self.read_stats().await?;
        let now = SystemTime::now();
        
        match (&self.last_stats, &self.last_time) {
            (Some(prev_stats), Some(prev_time)) => {
                // Calculate delta
                let user_delta = stats.user - prev_stats.user;
                let system_delta = stats.system - prev_stats.system;
                let idle_delta = stats.idle - prev_stats.idle;
                let other_delta = (stats.iowait + stats.irq + stats.softirq + 
                                  stats.steal + stats.guest) -
                                 (prev_stats.iowait + prev_stats.irq + 
                                  prev_stats.softirq + prev_stats.steal + 
                                  prev_stats.guest);
                
                let total_delta = user_delta + system_delta + idle_delta + other_delta;
                let non_idle_delta = user_delta + system_delta + other_delta;
                
                // Calculate usage percentage
                let usage = if total_delta > 0 {
                    ((non_idle_delta as f64) / (total_delta as f64)) * 100.0
                } else {
                    0.0
                };
                
                self.last_stats = Some(stats);
                self.last_time = Some(now);
                
                Ok(usage)
            }
            _ => {
                // First sample - initialize and return 0
                self.last_stats = Some(stats);
                self.last_time = Some(now);
                Ok(0.0)
            }
        }
    }
}
```

#### Per-Core Aggregation

For aggregate usage, we use the system-wide `cpu` line (recommended approach):

```rust
fn read_cpu_stat() -> Result<CpuStats> {
    let content = fs::read_to_string("/proc/stat")?;
    let first_line = content.lines()
        .find(|l| l.starts_with("cpu "))
        .ok_or(CpuError::InvalidFormat)?;
    
    let fields: Vec<u64> = first_line.split_whitespace()
        .skip(1)
        .map(|s| s.parse().unwrap_or(0))
        .collect();
    
    Ok(CpuStats {
        user: fields[0],
        nice: fields[1],
        system: fields[2],
        idle: fields[3],
        iowait: fields[4],
        irq: fields[5],
        softirq: fields[6],
        steal: fields[7],
        guest: fields[8],
        guest_nice: fields.get(9).copied().unwrap_or(0),
    })
}
```

**Rationale for using system-wide line**:
- Simpler implementation
- Already aggregated across all cores
- Avoids issues with per-core frequency scaling

### Edge Cases and Considerations

#### 1. First Sample

On first read, we cannot calculate usage (no baseline). Solution:

```rust
// First read: initialize values, return 0.0 usage
// Second read onwards: calculate usage from deltas
```

#### 2. Jiffies Overflow (u64 Wraparound)

**Problem**: On long-running systems, jiffies can overflow (wrap from `u64::MAX` back to 0).

**Solution**: 
- Use `u64` for jiffies (can run ~580 years at 100 Hz before overflow)
- Handle wraparound by detecting negative deltas:

```rust
fn safe_sub(a: u64, b: u64) -> u64 {
    if a >= b {
        a - b
    } else {
        // Wraparound detected
        u64::MAX - b + a + 1
    }
}

// Usage
let user_delta = safe_sub(stats.user, prev_stats.user);
```

#### 3. Clock Ticks (Jiffies Frequency)

On Linux, clock frequency varies by architecture:
- Most x86_64 systems: 100 Hz (100 jiffies/second)
- Modern systems: can be higher (250, 1000 Hz)

**Note**: Our delta calculation doesn't require knowing the frequency since we compute percentage.

#### 4. Multi-Core Systems

For aggregate usage across all cores, we use the system-wide `cpu` line. This is accurate for sleep inhibition decisions.

#### 5. Timer Interrupt Resolution

**Important Note**: CPU time is sampled at timer interrupt boundaries, which can cause slight inaccuracies for very short intervals. The default 5-second polling interval minimizes this effect.

**Reference**: [Linux Kernel CPU Load Documentation](https://www.kernel.org/doc/html/latest/admin-guide/cpu-load.html)

### Error Handling

#### Missing `/proc/stat`

If `/proc/stat` is unavailable (extremely rare), the daemon should:

1. Log an error with clear message
2. Return a sentinel value (e.g., `None`) or `0.0`
3. Allow systemd to restart the service according to restart policy

```rust
fn read_stats(&self) -> Result<CpuStats, CpuError> {
    let content = fs::read_to_string("/proc/stat")
        .map_err(|e| CpuError::IoError(e.to_string()))?;
    
    let first_line = content.lines()
        .find(|l| l.starts_with("cpu "))
        .ok_or(CpuError::InvalidFormat)?;
    
    // ... parsing logic
}

#[derive(Debug)]
pub enum CpuError {
    #[error("IO error: {0}")]
    IoError(String),
    
    #[error("Invalid /proc/stat format")]
    InvalidFormat,
    
    #[error("Missing required CPU fields")]
    MissingFields,
}
```

### Alternative Sources

#### `/proc/loadavg`

Alternative source for load average (1, 5, 15 minute averages):

```
$ cat /proc/loadavg
0.25 0.18 0.15 3/456 12345
```

**Pros**:
- Simpler to parse
- No need for two-sample calculation

**Cons**:
- Provides time-averaged load, not instantaneous usage
- Load average includes processes in uninterruptible sleep (D state)
- Not suitable for precise threshold detection

#### `sysinfo` Crate

For simpler implementation, consider using the `sysinfo` crate:

```toml
[dependencies]
sysinfo = "0.30"
```

```rust
use sysinfo::{System, SystemExt};

let mut system = System::new_all();
system.refresh_cpu_usage();
let cpu_usage = system.get_cpu_usage();
```

**Trade-offs**:
- **Pros**: Simpler API, cross-platform
- **Cons**: External dependency, more permissive license (MIT), larger binary size

**Recommendation**: Use `/proc/stat` directly for pure Rust implementation with no external dependencies.

### Performance Considerations

| Metric | Value |
|--------|-------|
| File I/O | None (all in kernel memory) |
| CPU overhead per read | ~10 microseconds |
| Parsing overhead | ~5 microseconds |
| Memory footprint | Negligible (< 1 KB) |
| Recommended polling interval | 1-10 seconds (default: 5s) |

### Testing

#### Unit Test Example

```rust
#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_parse_cpu_line() {
        let line = "cpu  100 20 300 4000 50 60 70 80 90 100";
        let stats = parse_cpu_line(line).unwrap();
        
        assert_eq!(stats.user, 100);
        assert_eq!(stats.nice, 20);
        assert_eq!(stats.system, 300);
        assert_eq!(stats.idle, 4000);
        assert_eq!(stats.iowait, 50);
        assert_eq!(stats.irq, 60);
        assert_eq!(stats.softirq, 70);
        assert_eq!(stats.steal, 80);
        assert_eq!(stats.guest, 90);
        assert_eq!(stats.guest_nice, 100);
    }
    
    #[test]
    fn test_cpu_usage_calculation() {
        let mut collector = CpuCollector::new();
        
        // Simulate first read - should return 0.0
        let usage1 = collector.collect_cpu_usage().unwrap();
        assert_eq!(usage1, 0.0);
        
        // Simulate second read with high CPU usage
        // (mock the /proc/stat content via test fixtures)
        let usage2 = collector.collect_cpu_usage().unwrap();
        assert!(usage2 >= 0.0 && usage2 <= 100.0);
    }
    
    #[test]
    fn test_jiffies_wraparound() {
        // Simulate jiffies overflow
        let prev = u64::MAX - 10;
        let curr = 5;
        
        let delta = safe_sub(curr, prev);
        assert_eq!(delta, 15); // Should handle wraparound correctly
    }
}
```

### References

- [Linux Kernel Documentation - /proc/stat](https://www.kernel.org/doc/html/latest/filesystems/proc.html#proc-stat)
- [Linux Kernel Documentation - CPU Load](https://www.kernel.org/doc/html/latest/admin-guide/cpu-load.html)
- [proc(5) man page](https://man7.org/linux/man-pages/man5/proc.5.html)

## See Also

- [GPU Metrics](gpu.md)
- [Network Metrics](network.md)
- [Disk Metrics](disk.md)
- [Memory Metrics](memory.md)
