# Disk Metrics

## Overview

The `rouser` daemon collects disk I/O metrics by reading data from the `/proc/diskstats` file. This provides aggregate disk throughput across all monitored block devices.

## Metric Details

### Primary Metric

| Metric | Type | Unit | Description |
|--------|------|------|-------------|
| `disk_activity` | Throughput | MB/s | Disk read/write throughput |

### Calculated Metrics

| Metric | Type | Unit | Description |
|--------|------|------|-------------|
| `read_sectors` | Counter | sectors | Sectors read (cumulative) |
| `write_sectors` | Counter | sectors | Sectors written (cumulative) |
| `read_ios` | Counter | I/O operations | I/O operations completed |
| `write_ios` | Counter | I/O operations | I/O operations completed |

## Data Source: `/proc/diskstats`

### File Format

The `/proc/diskstats` file contains block device statistics:

```
loop0  7:0  loop 3000 10000 10000 400 200 200 0 0 0 200 0 0 0 0 0 0 0 100 200 0 0 0 0 0
sda    8:0  sda   5000 20000 20000 1000 500 500 0 0 0 500 0 0 0 0 0 0 0 100 300 0 0 0 0 0
nvme0n1 259:0 nvme0n1 10000 40000 40000 2000 1000 1000 0 0 0 1000 0 0 0 0 0 0 0 200 500 0 0 0 0 0
```

### Field Descriptions (Standard Block Device Stats)

| Field Index | Name | Description |
|-------------|------|-------------|
| 1 | `major` | Major device number |
| 2 | `minor` | Minor device number |
| 3 | `name` | Device name (e.g., `sda`, `nvme0n1`) |
| 4 | `reads_completed` | Total reads completed |
| 5 | `reads_merged` | Reads merged with pending reads |
| 6 | `sectors_read` | Total sectors read |
| 7 | `time_reading_ms` | Time spent reading (ms) |
| 8 | `writes_completed` | Total writes completed |
| 9 | `writes_merged` | Writes merged with pending writes |
| 10 | `sectors_written` | Total sectors written |
| 11 | `time_writing_ms` | Time spent writing (ms) |
| 12 | `io_in_progress` | I/O operations in progress |
| 13 | `time_io_ms` | Time spent doing I/O (ms) |
| 14 | `weighted_io_time_ms` | Weighted time spent doing I/O (ms) |

### Calculation of Disk Throughput

To calculate disk throughput:

1. **Read initial values** at time `t1`
2. **Wait for interval** (configurable, default 5 seconds)
3. **Read final values** at time `t2`
4. **Calculate differences** between t1 and t2
5. **Convert sectors to bytes** (sector size is typically 512 bytes)

#### Formula

```rust
// From two samples
sectors_read_delta = sectors_read_t2 - sectors_read_t1
sectors_written_delta = sectors_written_t2 - sectors_written_t1
total_sectors = sectors_read_delta + sectors_written_delta

// Convert to bytes (assuming 512-byte sectors)
sector_size = 512
total_bytes = total_sectors * sector_size

// Convert to MB/s
interval_seconds = t2 - t1
throughput_mbps = total_bytes / (interval_seconds * 1_000_000.0)
```

## Device Filtering

### Virtual Device Detection

#### Excluded Devices (Truly Virtual/Simulated)

The following device prefixes are **excluded** by default:

| Prefix | Type | Description |
|--------|------|-------------|
| `loop` | Loop device | File-backed block device |
| `fd` | File descriptor backend | Legacy file descriptor backends |
| `sr` | SCSI CD-ROM | CD/DVD drive |
| `cdrom` | CD-ROM | CD/DVD drive alternative name |

**Rationale**: These devices represent virtual or optical storage that shouldn't trigger sleep inhibition.

#### Included Devices (Real Storage)

The following device prefixes are **included** by default:

| Prefix | Type | Description |
|--------|------|-------------|
| `sd` | SCSI/SATA | Standard SATA/SAS drives |
| `vd` | VirtIO | KVM/virtio block devices |
| `xv` | Xen Block | Xen block devices |
| `nvme` | NVMe | NVMe SSDs |
| `dm-` | Device Mapper | LVM volumes, RAID arrays |
| `hda`, `sda`, `vda` | Various | Traditional disk interfaces |

**Important Note on Device Mapper (LVM)**:

Device mapper devices (prefix `dm-`) represent LVM volumes and are **included** in monitoring:

```toml
[disk]
# These are excluded by default
exclude_device_prefixes = ["loop", "fd", "sr", "cdrom"]

# dm- (LVM) is INCLUDED - do not add to exclusion list
```

**Rationale**: LVM volumes represent real storage devices that contribute to actual disk activity. Excluding them would undercount real I/O load.

### Implementation

```rust
pub struct DiskCollector {
    exclude_prefixes: Vec<String>,
    last_stats: HashMap<String, DiskStats>,
}

impl DiskCollector {
    pub fn new() -> Self {
        // Default exclusions: loop devices, fd, sr, cdrom
        // Note: dm- (LVM) is INCLUDED
        Self {
            exclude_prefixes: vec![
                "loop".to_string(),
                "fd".to_string(),
                "sr".to_string(),
                "cdrom".to_string(),
            ],
            last_stats: HashMap::new(),
        }
    }
    
    pub fn with_excludes(mut self, prefixes: Vec<String>) -> Self {
        self.exclude_prefixes = prefixes;
        self
    }
    
    fn should_monitor(&self, name: &str) -> bool {
        !self.exclude_prefixes.iter().any(|prefix| name.starts_with(prefix))
    }
    
    async fn read_disk_stats(&self) -> Result<HashMap<String, DiskStats>, DiskError> {
        let content = fs::read_to_string("/proc/diskstats")
            .map_err(|e| DiskError::IoError(e.to_string()))?;
        
        let mut stats_map = HashMap::new();
        
        for line in content.lines() {
            let parts: Vec<&str> = line.split_whitespace().collect();
            
            if parts.len() < 14 {
                continue;
            }
            
            let major = parts[0].parse::<u32>().ok();
            let minor = parts[1].parse::<u32>().ok();
            let name = parts[2].to_string();
            
            // Skip virtual devices
            if !self.should_monitor(&name) {
                continue;
            }
            
            let stats = DiskStats {
                major,
                minor,
                name: name.clone(),
                reads_completed: parts[3].parse().unwrap_or(0),
                reads_merged: parts[4].parse().unwrap_or(0),
                sectors_read: parts[6].parse().unwrap_or(0),
                reads_ms: parts[7].parse().unwrap_or(0),
                writes_completed: parts[8].parse().unwrap_or(0),
                writes_merged: parts[9].parse().unwrap_or(0),
                sectors_written: parts[10].parse().unwrap_or(0),
                writes_ms: parts[11].parse().unwrap_or(0),
            };
            
            stats_map.insert(name, stats);
        }
        
        Ok(stats_map)
    }
}
```

## Error Handling

### Missing `/proc/diskstats`

If `/proc/diskstats` is unavailable (extremely rare):
- Return `0.0` throughput
- Log error at debug level
- Continue operation

### No Monitored Devices

If no non-virtual devices are available:
- Return `0.0` throughput
- Log warning (unusual configuration)

### Device Name Stability

Device names in `/proc/diskstats` use major:minor numbers, which remain stable across reboots for most devices. However, some devices may change names.

**Strategy**: Monitor by major:minor rather than name:

```rust
let device_key = format!("{}:{}", major, minor);
```

### Implementation

```rust
#[derive(Debug)]
pub enum DiskError {
    #[error("IO error: {0}")]
    IoError(String),
    
    #[error("No disk devices found")]
    NoDevices,
    
    #[error("Invalid /proc/diskstats format")]
    InvalidFormat,
}
```

## Performance Considerations

| Metric | Value |
|--------|-------|
| File I/O | None (all in kernel memory) |
| CPU overhead per read | ~2 microseconds |
| Memory footprint | ~200 bytes per device |
| Device count | Supports 100+ devices |

### Sector Size Considerations

Most modern systems use 512-byte sectors, but some devices (Advanced Format drives) use 4096-byte sectors:

```rust
// Safe default: assume 512 bytes
const SECTOR_SIZE: u64 = 512;

// To detect sector size:
fn get_sector_size(major: u32, minor: u32) -> u64 {
    // Try to read from /sys/block/device/queue/logical_block_size
    // Fallback to 512 bytes
    512
}
```

## Configuration

### Threshold Configuration

```toml
[thresholds]
disk_activity = 50.0  # Default 50 MB/s
```

**Valid Range**: 0.0 - infinity
**Default**: 50.0 (MB/s)

### Device Filtering Configuration

```toml
[disk]
# Device prefixes to exclude
exclude_device_prefixes = ["loop", "fd", "sr", "cdrom"]

# Note: dm- (LVM) devices are INCLUDED by default
```

### Performance Configuration

```toml
[disk]
# Polling interval override (default: use daemon.update_interval)
poll_interval = "5s"

# Sector size (512 is typical, some drives use 4096)
sector_size = 512
```

## Testing

### Unit Test Example

```rust
#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_parse_diskstats_line() {
        let line = "loop0  7:0  loop 3000 10000 10000 400 200 200 0 0 0 200 0 0 0 0 0 0 0 100 200 0 0 0 0 0";
        let stats = parse_diskstats_line(line).unwrap();
        
        assert_eq!(stats.name, "loop");
        assert_eq!(stats.sectors_read, 10000);
        assert_eq!(stats.sectors_written, 200);
    }
    
    #[test]
    fn test_throughput_calculation() {
        let sector_delta = 100_000;
        let sector_size = 512;
        let interval_seconds = 5.0;
        
        let bytes = sector_delta as f64 * sector_size as f64;
        let throughput = bytes / (interval_seconds * 1_000_000.0);
        
        assert_eq!(throughput, 10.24); // 10.24 MB/s
    }
    
    #[test]
    fn test_loop_device_excluded() {
        let collector = DiskCollector::new();
        
        assert!(!collector.should_monitor("loop0"));
        assert!(!collector.should_monitor("loop1"));
    }
    
    #[test]
    fn test_lvm_device_included() {
        let collector = DiskCollector::new();
        
        assert!(collector.should_monitor("dm-0"));
        assert!(collector.should_monitor("dm-1"));
    }
}
```

## References

- [Linux Kernel Documentation - /proc/diskstats](https://www.kernel.org/doc/html/latest/filesystems/proc.html#diskstats)
- [blockdev(8) man page](https://man7.org/linux/man-pages/man8/blockdev.8.html)
- [blkid(8) man page](https://man7.org/linux/man-pages/man8/blkid.8.html)

## See Also

- [CPU Metrics](cpu.md)
- [GPU Metrics](gpu.md)
- [Network Metrics](network.md)
- [Memory Metrics](memory.md)
- [Configuration Reference](../configuration/reference.md)
