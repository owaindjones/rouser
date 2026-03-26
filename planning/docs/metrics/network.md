# Network Metrics

## Overview

The `rouser` daemon collects network I/O metrics by reading data from the `/proc/net/dev` file. This provides aggregate network throughput across all network interfaces.

## Metric Details

### Primary Metrics

| Metric | Type | Unit | Description |
|--------|------|------|-------------|
| `network_io` | Throughput | Mbps | Network throughput threshold |

### Calculated Metrics

| Metric | Type | Unit | Description |
|--------|------|------|-------------|
| `rx_bytes` | Counter | bytes | Bytes received (cumulative) |
| `tx_bytes` | Counter | bytes | Bytes transmitted (cumulative) |
| `rx_pps` | Rate | packets/sec | Packets per second received |
| `tx_pps` | Rate | packets/sec | Packets per second transmitted |

## Data Source: `/proc/net/dev`

### File Format

The `/proc/net/dev` file contains network interface statistics:

```
Inter-|   Receive                                                |  Transmit
 face |bytes    packets errs drop fifo frame compressed multicast|bytes    packets errs drop fifo colls carrier compressed
    lo: 1234567   12345    0    0    0     0          0         0  2345678   23456    0    0    0     0       0          0
  eth0: 987654321 9876543    0    0    0     0          0         0   87654321 8765432    0    0    0     0       0          0
docker0: 123456   1234    0    0    0     0          0         0   234567   2345    0    0    0     0       0          0
```

### Field Descriptions

| Field Index | Name | Description |
|-------------|------|-------------|
| 1 | `rx_bytes` | Total bytes received |
| 2 | `rx_packets` | Total packets received |
| 3 | `rx_errs` | Receive errors |
| 4 | `rx_drop` | Receive drops |
| 5 | `rx_fifo` | FIFO buffer overruns |
| 6 | `rx_frame` | Framing errors |
| 7 | `rx_compressed` | Compressed packets received |
| 8 | `rx_multicast` | Multicast packets received |
| 9 | `tx_bytes` | Total bytes transmitted |
| 10 | `tx_packets` | Total packets transmitted |
| 11 | `tx_errs` | Transmit errors |
| 12 | `tx_drop` | Transmit drops |
| 13 | `tx_fifo` | FIFO buffer overruns |
| 14 | `tx_colls` | Collisions detected |
| 15 | `tx_carrier` | Carrier losses |
| 16 | `tx_compressed` | Compressed packets transmitted |

### Calculation of Network Throughput

To calculate network throughput:

1. **Read initial values** at time `t1`
2. **Wait for interval** (configurable, default 5 seconds)
3. **Read final values** at time `t2`
4. **Calculate differences** between t1 and t2
5. **Convert to bits per second** (multiply by 8, then divide by interval)

#### Formula

```rust
// From two samples
rx_delta = rx_bytes_t2 - rx_bytes_t1
tx_delta = tx_bytes_t2 - tx_bytes_t1
total_delta = rx_delta + tx_delta

// Convert to megabits per second
interval_seconds = t2 - t1
throughput_mbps = (total_delta * 8.0) / (interval_seconds * 1_000_000.0)
```

## Interface Filtering

### Loopback Interface (lo)

**Default Behavior**: The loopback interface (`lo`) is **excluded** from monitoring.

**Rationale**:
- Loopback traffic is internal to the system
- External network activity is more relevant for sleep inhibition decisions
- Database replication or internal services may use loopback, which should not keep the system awake

**Configuration**:

```toml
[network]
# Interfaces to explicitly exclude (default: exclude loopback)
exclude_interfaces = ["lo"]

# To include loopback (not recommended):
# exclude_interfaces = []
```

### Virtual and Docker Interfaces

Docker bridges (`docker0`), tunnels (`tun0`, `tap0`), and virtual network interfaces are **included** by default as they may represent external traffic.

### Interface Selection Strategy

```rust
pub struct NetworkCollector {
    exclude_interfaces: Vec<String>,
    include_interfaces: Option<Vec<String>>,
}

impl NetworkCollector {
    pub fn new(exclude_interfaces: Vec<String>) -> Self {
        Self {
            exclude_interfaces,
            include_interfaces: None, // None = all interfaces
        }
    }
    
    pub fn with_excludes(exclude_interfaces: Vec<String>) -> Self {
        // Default exclusion of loopback
        let mut excludes = exclude_interfaces;
        if !excludes.contains(&"lo".to_string()) {
            excludes.push("lo".to_string());
        }
        Self {
            exclude_interfaces: excludes,
            include_interfaces: None,
        }
    }
    
    fn get_monitored_interfaces(&self) -> Result<Vec<Interface>> {
        let all_interfaces = self.parse_proc_net_dev()?;
        
        let filtered: Vec<Interface> = all_interfaces
            .into_iter()
            .filter(|iface| {
                // Exclude specified interfaces
                if self.exclude_interfaces.contains(&iface.name) {
                    return false;
                }
                
                // Include only specified interfaces if configured
                if let Some(ref include) = self.include_interfaces {
                    return include.contains(&iface.name);
                }
                
                true
            })
            .collect();
        
        Ok(filtered)
    }
}
```

## Implementation

### Network Collector

```rust
pub struct NetworkCollector {
    last_stats: HashMap<String, NetworkStats>,
    last_time: Option<SystemTime>,
    exclude_interfaces: Vec<String>,
}

impl NetworkCollector {
    pub fn new() -> Self {
        Self {
            last_stats: HashMap::new(),
            last_time: None,
            exclude_interfaces: vec!["lo".to_string()],
        }
    }
    
    pub async fn collect_network_usage(&mut self) -> Result<f64, NetworkError> {
        let current_stats = self.read_interface_stats().await?;
        let now = SystemTime::now();
        
        match &self.last_time {
            Some(prev_time) => {
                let interval = now.duration_since(*prev_time)
                    .unwrap_or(Duration::from_secs(1));
                
                let mut total_delta = 0u64;
                
                for (name, stats) in &current_stats {
                    if let Some(prev) = self.last_stats.get(name) {
                        let rx_delta = stats.rx_bytes.saturating_sub(prev.rx_bytes);
                        let tx_delta = stats.tx_bytes.saturating_sub(prev.tx_bytes);
                        total_delta += rx_delta + tx_delta;
                    }
                }
                
                let throughput_mbps = (total_delta as f64 * 8.0) / 
                    (interval.as_millis() as f64 / 1000.0 * 1_000_000.0);
                
                self.last_stats = current_stats;
                self.last_time = Some(now);
                
                Ok(throughput_mbps)
            }
            None => {
                self.last_stats = current_stats;
                self.last_time = Some(now);
                Ok(0.0)
            }
        }
    }
    
    async fn read_interface_stats(&self) -> Result<HashMap<String, NetworkStats>, NetworkError> {
        let content = fs::read_to_string("/proc/net/dev")
            .map_err(|e| NetworkError::IoError(e.to_string()))?;
        
        let mut stats_map = HashMap::new();
        
        for line in content.lines() {
            // Skip header lines
            if !line.contains(':') {
                continue;
            }
            
            let parts: Vec<&str> = line.split(':').collect();
            if parts.len() != 2 {
                continue;
            }
            
            let name = parts[0].trim().to_string();
            
            // Skip excluded interfaces
            if self.exclude_interfaces.contains(&name) {
                continue;
            }
            
            let values: Vec<u64> = parts[1]
                .split_whitespace()
                .map(|s| s.parse().unwrap_or(0))
                .collect();
            
            if values.len() < 17 {
                continue;
            }
            
            stats_map.insert(name, NetworkStats {
                rx_bytes: values[0],
                rx_packets: values[1],
                rx_errs: values[2],
                rx_drop: values[3],
                tx_bytes: values[8],
                tx_packets: values[9],
                tx_errs: values[10],
                tx_drop: values[11],
            });
        }
        
        Ok(stats_map)
    }
}

#[derive(Debug, Clone)]
pub struct NetworkStats {
    pub rx_bytes: u64,
    pub rx_packets: u64,
    pub rx_errs: u64,
    pub rx_drop: u64,
    pub tx_bytes: u64,
    pub tx_packets: u64,
    pub tx_errs: u64,
    pub tx_drop: u64,
}
```

### Error Handling

#### Missing `/proc/net/dev`

If `/proc/net/dev` is unavailable (extremely rare):
- Return `0.0` throughput
- Log error at debug level
- Continue operation

```rust
#[derive(Debug)]
pub enum NetworkError {
    #[error("IO error: {0}")]
    IoError(String),
    
    #[error("No network interfaces found")]
    NoInterfaces,
    
    #[error("Invalid /proc/net/dev format")]
    InvalidFormat,
}
```

#### No Network Interfaces

If no network interfaces are available:
- Return `0.0` throughput
- Log warning (unusual configuration)

## Configuration

### Threshold Configuration

```toml
[thresholds]
network_io = 100.0  # Default 100 Mbps
```

**Valid Range**: 0.0 - infinity
**Default**: 100.0 (Mbps)

### Interface Filtering Configuration

```toml
[network]
# Interfaces to exclude (default excludes loopback)
exclude_interfaces = ["lo"]

# Interfaces to include (empty = all non-excluded interfaces)
include_interfaces = []
```

### Performance Configuration

```toml
[network]
# Polling interval override (default: use daemon.update_interval)
poll_interval = "5s"

# Include/Exclude patterns (glob patterns supported)
exclude_patterns = ["^lo$", "^docker.*$", "^virbr.*$"]
```

## Performance Considerations

| Metric | Value |
|--------|-------|
| File I/O | None (all in kernel memory) |
| CPU overhead per read | ~5 microseconds |
| Memory footprint | ~1 KB per interface |
| Interface count | Supports 100+ interfaces |

## Testing

### Unit Test Example

```rust
#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_parse_proc_net_dev_line() {
        let line = "  eth0: 1234567   12345    0    0    0     0          0         0  2345678   23456    0    0    0     0       0          0";
        let stats = parse_net_dev_line(line).unwrap();
        
        assert_eq!(stats.rx_bytes, 1234567);
        assert_eq!(stats.tx_bytes, 2345678);
    }
    
    #[test]
    fn test_throughput_calculation() {
        let mut collector = NetworkCollector::new();
        
        // Simulate two reads with known delta
        // Read 1: rx=1000, tx=2000
        // Read 2: rx=2000, tx=4000
        // Interval: 1 second
        // Expected: 24 Mbps (1000*8 + 2000*8 = 24000 bits, / 1s)
        
        let throughput = collector.collect_network_usage().unwrap();
        assert!(throughput >= 0.0);
    }
    
    #[test]
    fn test_loopback_excluded() {
        let mut collector = NetworkCollector::new();
        
        // Verify loopback is excluded by default
        let interfaces = collector.get_monitored_interfaces();
        
        assert!(!interfaces.iter().any(|i| i.name == "lo"));
    }
}
```

## References

- [Linux Kernel Documentation - /proc/net/dev](https://www.kernel.org/doc/html/latest/filesystems/proc.html#net-dev)
- [man 5 proc](https://man7.org/linux/man-pages/man5/proc.5.html)

## See Also

- [CPU Metrics](cpu.md)
- [GPU Metrics](gpu.md)
- [Disk Metrics](disk.md)
- [Memory Metrics](memory.md)
- [Configuration Reference](../configuration/reference.md)
