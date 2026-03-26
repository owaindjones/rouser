# Memory Metrics

## Overview

The `rouser` daemon collects memory metrics by reading from the `/proc/meminfo` file. This provides comprehensive memory usage statistics including RAM, swap, and cache information.

## Metric Details

### Primary Metrics

| Metric | Type | Unit | Description |
|--------|------|------|-------------|
| `memory_used` | Percentage | 0.0 - 100.0 | Percentage of RAM in use |
| `memory_total` | Absolute | Bytes | Total physical memory |
| `memory_free` | Absolute | Bytes | Free memory available |
| `memory_available` | Absolute | Bytes | Available memory (including reclaimable) |

### Derived Metrics

| Metric | Type | Unit | Description |
|--------|------|------|-------------|
| `memory_cached` | Absolute | Bytes | Memory used for page cache |
| `memory_buffers` | Absolute | Bytes | Memory used for buffers |
| `swap_used` | Absolute | Bytes | Swap space used |
| `swap_total` | Absolute | Bytes | Total swap space |
| `swap_free` | Absolute | Bytes | Free swap space |

## Data Source: `/proc/meminfo`

### File Format

The `/proc/meminfo` file contains memory statistics in a key-value format:

```
MemTotal:       16384000 kB
MemFree:         2048000 kB
MemAvailable:    8192000 kB
Buffers:          512000 kB
Cached:          3072000 kB
SwapCached:       102400 kB
Active:          8192000 kB
Inactive:        4096000 kB
Active(anon):    6144000 kB
Inactive(anon):  1024000 kB
Active(file):    2048000 kB
Inactive(file):  3072000 kB
Unevictable:         0 kB
Mlocked:             0 kB
SwapTotal:       8192000 kB
SwapFree:        4096000 kB
Dirty:             102400 kB
Writeback:             0 kB
AnonPages:       7168000 kB
Mapped:           512000 kB
Shmem:            256000 kB
KReclaimable:    1024000 kB
Slab:            1536000 kB
SReclaimable:    1024000 kB
SUnreclaim:       512000 kB
KernelStack:       65536 kB
PageTables:       131072 kB
NFS_Unstable:          0 kB
Bounce:                0 kB
WritebackTmp:          0 kB
CommitLimit:    16384000 kB
Committed_AS:    9216000 kB
VmallocTotal:   34359738367 kB
VmallocUsed:       26214 kB
VmallocChunk:          0 kB
Percpu:            32768 kB
HardwareCorrupted:     0 kB
AnonHugePages:   2097152 kB
ShmemHugePages:        0 kB
ShmemPmdMapped:        0 kB
FileHugePages:         0 kB
FilePmdMapped:         0 kB
HugePages_Total:       0
HugePages_Free:        0
HugePages_Surp:        0
Hugepagesize:       2048 kB
Hugetlb:               0 kB
DirectMap4k:      262144 kB
DirectMap2M:     4194304 kB
DirectMap1G:    12582912 kB
```

### Key Fields

#### Memory Usage Fields

| Field | Description |
|-------|-------------|
| `MemTotal` | Total installed physical memory |
| `MemFree` | Memory that's entirely unused |
| `MemAvailable` | Estimation of how much memory is available |
| `Buffers` | Memory used for block device buffers |
| `Cached` | Memory used for file page cache |

#### Swap Fields

| Field | Description |
|-------|-------------|
| `SwapTotal` | Total swap space |
| `SwapFree` | Free swap space |
| `SwapTotal` | Total swap space |
| `SwapCached` | Swap used for cached pages |

#### Cache Fields

| Field | Description |
|-------|-------------|
| `SReclaimable` | Reclaimable slab objects |
| `SUnreclaim` | Unreclaimable slab objects |
| `Slab` | Total slab memory |
| `Cached` | Page cache |

#### Active/Inactive Fields

| Field | Description |
|-------|-------------|
| `Active` | Memory that has been used more recently |
| `Inactive` | Memory not recently used |
| `Active(file)` | Active file-backed memory |
| `Inactive(file)` | Inactive file-backed memory |

### Parsing Algorithm

```rust
use std::collections::HashMap;
use std::fs;

#[derive(Debug, Clone, Default)]
pub struct MemInfo {
    pub mem_total: u64,
    pub mem_free: u64,
    pub mem_available: u64,
    pub buffers: u64,
    pub cached: u64,
    pub swap_total: u64,
    pub swap_free: u64,
    pub swap_cached: u64,
    pub slab_reclaimable: u64,
    pub slab_unreclaimable: u64,
    pub active: u64,
    pub inactive: u64,
}

fn parse_meminfo_line(line: &str) -> Result<(String, u64)> {
    // Format: "FieldName:     12345 kB"
    let parts: Vec<&str> = line.split_whitespace().collect();
    
    if parts.len() < 3 {
        return Err(MemoryError::InvalidFormat);
    }
    
    let field_name = parts[0].trim_end_matches(':').to_string();
    let value: u64 = parts[1].parse()?;
    
    Ok((field_name, value))
}
```

## Memory Usage Calculation

### Available Memory Calculation

Linux provides a good approximation of available memory:

```rust
pub struct MemoryCollector;

impl MemoryCollector {
    pub fn collect_memory_stats(&self) -> Result<MemoryMetrics> {
        let meminfo = self.read_meminfo().await?;
        
        // Calculate used memory
        let used_bytes = meminfo.mem_total - meminfo.mem_available;
        let used_percentage = (used_bytes as f64 / meminfo.mem_total as f64) * 100.0;
        
        // Calculate swap usage
        let swap_used = meminfo.swap_total - meminfo.swap_free;
        let swap_used_percentage = if meminfo.swap_total > 0 {
            (swap_used as f64 / meminfo.swap_total as f64) * 100.0
        } else {
            0.0
        };
        
        // Calculate cache percentage
        let cache_bytes = meminfo.cached + meminfo.buffers + meminfo.slab_reclaimable;
        let cache_percentage = (cache_bytes as f64 / meminfo.mem_total as f64) * 100.0;
        
        Ok(MemoryMetrics {
            total_bytes: meminfo.mem_total,
            free_bytes: meminfo.mem_free,
            available_bytes: meminfo.mem_available,
            used_bytes: used_bytes,
            used_percentage,
            swap_used_bytes: swap_used,
            swap_used_percentage,
            cached_bytes: cache_bytes,
            cache_percentage,
        })
    }
    
    fn read_meminfo(&self) -> Result<MemInfo> {
        let content = fs::read_to_string("/proc/meminfo")
            .map_err(|e| MemoryError::IoError(e.to_string()))?;
        
        let mut meminfo = MemInfo::default();
        
        for line in content.lines() {
            match parse_meminfo_line(line) {
                Ok((field, value)) => {
                    match field.as_str() {
                        "MemTotal" => meminfo.mem_total = value,
                        "MemFree" => meminfo.mem_free = value,
                        "MemAvailable" => meminfo.mem_available = value,
                        "Buffers" => meminfo.buffers = value,
                        "Cached" => meminfo.cached = value,
                        "SwapTotal" => meminfo.swap_total = value,
                        "SwapFree" => meminfo.swap_free = value,
                        "SwapCached" => meminfo.swap_cached = value,
                        "SReclaimable" => meminfo.slab_reclaimable = value,
                        "SUnreclaim" => meminfo.slab_unreclaimable = value,
                        "Active" => meminfo.active = value,
                        "Inactive" => meminfo.inactive = value,
                        _ => {} // Ignore unknown fields
                    }
                }
                Err(_) => {
                    // Skip malformed lines
                    continue;
                }
            }
        }
        
        Ok(meminfo)
    }
}

#[derive(Debug, Clone, Default)]
pub struct MemoryMetrics {
    pub total_bytes: u64,
    pub free_bytes: u64,
    pub available_bytes: u64,
    pub used_bytes: u64,
    pub used_percentage: f64,
    pub swap_used_bytes: u64,
    pub swap_used_percentage: f64,
    pub cached_bytes: u64,
    pub cache_percentage: f64,
}
```

### Memory Pressure Calculation

For sleep inhibition purposes, we may want to calculate memory pressure:

```rust
impl MemoryCollector {
    pub fn calculate_memory_pressure(&self, metrics: &MemoryMetrics) -> MemoryPressure {
        // Memory pressure based on available memory percentage
        let available_percentage = (metrics.available_bytes as f64 / metrics.total_bytes as f64) * 100.0;
        
        // Swap usage as pressure indicator
        let swap_pressure = metrics.swap_used_percentage;
        
        // Combined pressure metric (0 = no pressure, 100 = critical)
        let pressure = if available_percentage < 25.0 {
            // Critical: less than 25% available
            100.0 - available_percentage
        } else if available_percentage < 50.0 {
            // High: less than 50% available
            75.0 - (available_percentage * 0.75)
        } else {
            // Moderate or low
            swap_pressure * 0.5
        };
        
        MemoryPressure {
            level: pressure,
            available_percentage,
            swap_pressure,
        }
    }
}

#[derive(Debug, Clone)]
pub struct MemoryPressure {
    pub level: f64,
    pub available_percentage: f64,
    pub swap_pressure: f64,
}
```

## Alternative Sources

### free command

```bash
# Quick memory overview
free -h

# Detailed memory stats
free -h --si  # SI units (KB, MB)

# In human-readable format
free -h
```

**Pros**: Human-readable, quick overview
**Cons**: Less detailed than /proc/meminfo, parsing required

### vmstat

```bash
# Virtual memory statistics
vmstat 1 5

# With detailed statistics
vmstat -s
```

**Pros**: Includes swap and I/O statistics
**Cons**: External dependency, more complex

### top/htop

```bash
# Interactive process viewer
top

# Or htop for better interface
htop
```

**Pros**: Real-time, interactive, process-level details
**Cons**: External dependency, not suitable for programmatic monitoring

### psutil (Python)

```python
import psutil

memory = psutil.virtual_memory()
print(f"Total: {memory.total}")
print(f"Available: {memory.available}")
print(f"Used: {memory.percent}%")
```

**Pros**: Cross-platform, easy to use
**Cons**: Python dependency, slower than /proc/meminfo

## Edge Cases and Considerations

### 1. Memory Reclaimable

Linux aggressively uses free memory for cache, which can be reclaimed when needed. The `MemAvailable` field accounts for this:

```rust
// This is the correct metric to use for "available" memory
// Not MemFree, which only includes truly free memory
let available = meminfo.mem_available;
```

### 2. Huge Pages

Some systems use huge pages (2MB or 1GB pages). These are tracked separately:

```rust
// Check for huge pages if needed
if let Some(huge_pages) = meminfo.get("HugePages_Total") {
    if huge_pages > 0 {
        // Huge pages are configured
    }
}
```

### 3. Cgroups and Memory Limits

In containerized environments, memory may be limited:

```rust
// Check if memory limits are in place
let memory_limit = fs::read_to_string("/sys/fs/cgroup/memory/memory.limit_in_bytes")
    .ok()
    .and_then(|s| s.trim().parse().ok());

if let Some(limit) = memory_limit {
    if limit < meminfo.mem_total {
        // Container has memory limit
    }
}
```

### 4. NUMA Systems

On NUMA (Non-Uniform Memory Access) systems, memory may be partitioned:

```rust
// Check NUMA nodes
let numa_nodes = fs::read_dir("/sys/devices/system/node")
    .ok()
    .and_then(|dir| {
        dir.filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().starts_with("node"))
            .count()
    });
```

### 5. Memory Hot-Plug

On systems with memory hot-plugging, total memory can change:

```rust
// Re-read meminfo periodically to detect changes
// Most systems don't have memory hot-plug, so this is rare
```

## Performance Considerations

- **File I/O**: Reading `/proc/meminfo` is very fast (no disk I/O, all in memory)
- **Parsing**: Minimal overhead from string splitting and parsing
- **Memory**: O(1) - small fixed number of fields
- **CPU usage**: Negligible for typical polling intervals

## Testing

### Unit Test Example

```rust
#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_parse_meminfo_line() {
        let line = "MemTotal:       16384000 kB";
        let (name, value) = parse_meminfo_line(line).unwrap();
        
        assert_eq!(name, "MemTotal");
        assert_eq!(value, 16_384_000);
    }
    
    #[test]
    fn test_memory_usage_calculation() {
        let meminfo = MemInfo {
            mem_total: 16_384_000,
            mem_available: 8_192_000,
            ..Default::default()
        };
        
        let used_bytes = meminfo.mem_total - meminfo.mem_available;
        let used_percentage = (used_bytes as f64 / meminfo.mem_total as f64) * 100.0;
        
        assert!((used_percentage - 50.0).abs() < 0.01);
    }
    
    #[test]
    fn test_swap_usage_calculation() {
        let meminfo = MemInfo {
            swap_total: 8_192_000,
            swap_free: 4_096_000,
            ..Default::default()
        };
        
        let swap_used = meminfo.swap_total - meminfo.swap_free;
        let swap_used_percentage = (swap_used as f64 / meminfo.swap_total as f64) * 100.0;
        
        assert!((swap_used_percentage - 50.0).abs() < 0.01);
    }
}
```

## Configuration Reference

### Threshold Configuration

```yaml
thresholds:
  # Memory usage threshold in percentage
  memory_usage: 80.0
  
  # Optional: separate thresholds for swap
  swap_usage: 50.0
  
  # Optional: memory pressure threshold (0-100)
  memory_pressure: 70.0
```

### Memory Pressure Levels

```yaml
memory_pressure:
  # Pressure levels for different behaviors
  critical: 90.0  # < 10% available
  high: 70.0      # < 30% available
  moderate: 50.0  # < 50% available
  low: 25.0       # < 75% available
```

## Summary

Memory metrics are collected from `/proc/meminfo`, which provides comprehensive memory and swap statistics. The primary metrics include:

- **Used memory percentage**: Based on `MemAvailable` field (most accurate)
- **Swap usage**: Percentage of swap space in use
- **Cache percentage**: Memory used for page cache and buffers
- **Memory pressure**: Combined metric for sleep inhibition decisions

By using the `MemAvailable` field instead of `MemFree`, we account for Linux's memory management strategy where free memory is used for caching but can be reclaimed when needed. This provides a more accurate picture of actual available memory for applications.

The implementation requires no external dependencies and provides real-time memory statistics suitable for sleep inhibition decisions.
