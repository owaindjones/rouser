# Architecture Overview

This document describes the system architecture and design decisions for `rouser`, a Linux daemon that monitors system metrics and inhibits sleep when activity thresholds are exceeded.

## System Architecture

```
┌──�����──�����──�����──�����──�����──�����──�����──�����──�����──�����──�����──�����──�����──�����──�����──�����─┐
│                      rouser Daemon                              ���
│                                                                   ���
│  ���──�����──�����──�����─┐    ���──�����──�����──�����─┐    ���──�����──�����──�����─┐         ���
│  ���   Config    ���───▶│    Core     ���◀───│  Metrics    ���         ���
│  ���   Loader    ���    ���   Logic     ���    ���  Collectors ��         ���
│  ���──�����──�����──�����─┘    ���──�����──┬──�����──┘    ���──�����──�����──�����─┘         ���
│                            ���                                     ���
│                  ���──�����──�����▼──�����──�����┐                            ���
│                  ���  Threshold      ���                            ���
│                  ���  Manager        ���                            ���
│                  ���──�����──�����┬──�����──�����┘                            ���
│                           ���                                     ���
│                  ���──�����──�����▼──�����──�����┐                            ���
│                  ���  Sleep          ���                            ���
│                  ���  Inhibitor      ���                            ���
│                  ���──�����──�����┬──�����──�����┘                            ���
│                           ���                                     ���
│                  ���──�����──�����▼──�����──�����┐                            ���
│                  ���  D-Bus Client   ���                            ���
│                  ���  (zbus)         ���                            ���
│                  ���──�����──�����┬──�����──�����┘                            ���
└──�����──�����──�����──�����──�����──�����───┼──�����──�����──�����──�����──�����──�����──�����──�����──�����─┘
                            ���
                            ���
                   org.freedesktop.login1
```

## Core Components

### 1. Configuration Loader

**Purpose**: Parse and validate configuration files.

**Input**: TOML configuration file (default: `/etc/rouser/config.toml`)

**Key Responsibilities**:
- Parse TOML configuration using `toml` crate
- Validate all threshold values (0.0 - 100.0 for percentages)
- Resolve environment variable overrides
- Apply default values for optional fields
- Return immutable configuration

```rust
pub struct ConfigLoader {
    config_path: PathBuf,
    env_overrides: HashMap<String, String>,
}

impl ConfigLoader {
    pub fn load(&self) -> Result<Config> {
        let toml_content = fs::read_to_string(&self.config_path)?;
        let mut config: Config = toml::from_str(&toml_content)?;
        
        // Apply environment variable overrides
        self.apply_env_overrides(&mut config);
        
        // Validate configuration
        config.validate()?;
        
        Ok(config)
    }
}
```

### 2. Core Logic

**Purpose**: Main event loop and state management.

**Key Responsibilities**:
- Initialize metric collectors
- Run metric collection loop
- Check thresholds and update inhibition state
- Handle errors and failures gracefully

```rust
pub struct Core {
    config: Config,
    cpu_collector: CpuCollector,
    gpu_collector: GpuCollector,
    network_collector: NetworkCollector,
    disk_collector: DiskCollector,
    sleep_inhibitor: Option<SleepInhibitor>,
}

impl Core {
    pub async fn run(&mut self) -> Result<()> {
        let mut interval = tokio::time::interval(Duration::from_secs(
            self.config.daemon.update_interval.as_secs()
        ));
        
        loop {
            interval.tick().await;
            
            // Collect all metrics
            let cpu_usage = self.cpu_collector.collect_cpu_usage().await;
            let gpu_usage = self.gpu_collector.collect_gpu_usage().await;
            let network_io = self.network_collector.collect_network_usage().await;
            let disk_activity = self.disk_collector.collect_disk_usage().await;
            
            // Check thresholds
            let should_inhibit = self.check_thresholds(
                cpu_usage, gpu_usage, network_io, disk_activity
            );
            
            // Update inhibition state
            self.update_inhibition_state(should_inhibit).await?;
            
            // Log metric snapshot
            debug!(
                "Metrics: CPU={:.1}%, GPU={:.1}%, Network={:.1} Mbps, Disk={:.1} MB/s",
                cpu_usage, gpu_usage, network_io, disk_activity
            );
        }
    }
}
```

### 3. Metric Collectors

Modular collectors for different system metrics:

#### CPU Collector

- **Source**: `/proc/stat`
- **Polling**: Every 5 seconds (configurable)
- **Calculation**: Two-sample delta with wraparound handling
- **Error handling**: Graceful fallback to 0% on failure

#### GPU Collector

- **Sources**: 
  - NVIDIA: `nvidia-smi` command
  - AMD/Intel: `/sys/class/drm/cardX/device/gpu_busy_percent`
- **Polling**: Every 5 seconds
- **Aggregation**: Average across all GPUs
- **Error handling**: Returns 0% if GPU not detected

#### Network Collector

- **Source**: `/proc/net/dev`
- **Polling**: Every 5 seconds
- **Filtering**: Loopback interface excluded by default
- **Error handling**: Continue with available interfaces

#### Disk Collector

- **Source**: `/proc/diskstats`
- **Polling**: Every 5 seconds
- **Filtering**: Virtual devices excluded (loop, fd, sr, cdrom)
- **Error handling**: Continue with available devices

```rust
pub trait MetricCollector: Send + Sync {
    async fn collect(&self) -> Result<f64>;
    fn name(&self) -> &str;
}

impl MetricCollector for CpuCollector {
    async fn collect(&self) -> Result<f64> {
        // Collect from /proc/stat
        Ok(cpu_usage)
    }
    
    fn name(&self) -> &str {
        "cpu"
    }
}
```

### 4. Threshold Manager

**Purpose**: Track metrics over time and determine when thresholds are exceeded.

**Key Logic**:

```rust
pub struct ThresholdManager {
    cpu_usage_history: Vec<(SystemTime, f64)>,
    gpu_usage_history: Vec<(SystemTime, f64)>,
    // ... other metrics
    duration_threshold: Duration,
    idle_duration: Duration,
}

impl ThresholdManager {
    pub fn should_inhibit(
        &self,
        current_cpu: f64,
        current_gpu: f64,
        // ... other metrics
    ) -> bool {
        // Check if any metric exceeds threshold for duration_threshold
        self.check_metric_threshold(
            &self.cpu_usage_history,
            self.config.thresholds.cpu_usage
        ) || self.check_metric_threshold(
            &self.gpu_usage_history,
            self.config.thresholds.gpu_usage
        )
        // ... other metrics
    }
    
    pub fn is_idle(&self) -> bool {
        // Check if all metrics have been below threshold for idle_duration
        self.check_all_below_threshold_for_duration()
    }
}
```

### 5. Sleep Inhibitor

**Purpose**: Interface with D-Bus to inhibit system sleep.

**Implementation**:
- Uses `zbus` crate for D-Bus communication
- Implements RAII pattern for automatic lock release
- Tracks inhibition cookie for debugging

```rust
pub struct SleepInhibitor {
    fd: Option<File>,
    cookie: Option<String>,
}

impl SleepInhibitor {
    pub async fn acquire(what: &str, description: &str) -> Result<Self> {
        let connection = Connection::system().await?;
        
        let proxy = connection
            .object_proxy("org.freedesktop.login1", "/org/freedesktop/login1")
            .typed::<(), (RawFd, String)>();
        
        let (fd, cookie) = proxy
            .call("Inhibit", &("sleep", "block", what, description))
            .await?;
        
        Ok(Self {
            fd: Some(unsafe { File::from_raw_fd(fd) }),
            cookie: Some(cookie),
        })
    }
}

impl Drop for SleepInhibitor {
    fn drop(&mut self) {
        // File descriptor automatically closed, releasing inhibition lock
        log::info!("Releasing sleep inhibition: {}", self.cookie);
    }
}
```

## Data Flow

```
┌──�����──�����──┐    ���──�����──�����──┐    ���──�����──�����──┐    ���──�����──�����──┐
│ Config   ���───▶│ Core     ���───▶│ Collect  ���───▶│ Metrics  ���
│ Loader   ���    ���          ���    ���          ���    ���          ���
└──�����──�����──┘    ���──�����┬──�����─┘    ���──�����┬──�����─┘    ���──�����──�����──┘
                     ���               ���
                     ���    ���──�����──�����──▼──�����──�����──┐
                     ���    ��� Threshold Manager   ���
                     ���    ���──�����──�����──┬──�����──�����──┘
                     ���               ���
                     ���    ���──�����──�����──▼──�����──�����──┐
                     ���    ��� Sleep Inhibitor     ���
                     ���    ���──�����──�����──┬──�����──�����──┘
                     ���               ���
                     ���               ���
              org.freedesktop.login1  (system sleep)
```

## State Machine

```
┌──�����──�����──�����─┐
│   IDLE      ���◀──�����──�����──�����──�����──�����──�����───┐
└──�����──┬──�����──┘                            ���
       ���                                    ���
       ��� Any metric exceeds threshold       ���
       ��� for duration_threshold             ���
       ���                                    ���
┌──�����──�����──�����─┐                            ���
│ INHIBITING  ���──�����──�����──�����──�����──�����──�����──�����┘
└──�����──┬──�����──┘
       ���
       ��� All metrics below threshold      ���
       ��� for idle_duration                ���
       ���
┌──�����──�����──�����─┐
│   IDLE      ���
└──�����──�����──�����─┘
```

## Error Handling Strategy

### Graceful Degradation

```rust
impl CpuCollector {
    async fn collect_cpu_usage(&mut self) -> f64 {
        match self.read_stats() {
            Ok(stats) => calculate_usage(&self.last_stats, &stats),
            Err(e) => {
                warn!("CPU metrics collection failed: {}", e);
                0.0  // Graceful fallback
            }
        }
    }
}
```

### Error Propagation

```rust
#[derive(Debug)]
pub enum RouserError {
    ConfigError(ConfigError),
    IoError(#[from] std::io::Error),
    DbusError(#[from] zbus::Error),
    ParseError(ParseError),
    #[error("No metric sources available")]
    NoMetricsAvailable,
}
```

## Configuration Management

### Configuration Format

`rouser` uses **TOML** format for configuration (not YAML):

```toml
[daemon]
name = "rouser"
update_interval = "5s"

[thresholds]
cpu_usage = 80.0
```

**Rationale for TOML**:
- Pure Rust implementation with no C dependencies
- Simpler syntax than YAML
- Better security (avoids RUSTSEC-2025-0068 vulnerability in YAML parsers)
- Native support via `toml` crate

### Environment Variable Overrides

```bash
export ROUSER_THRESHOLDS_CPU_USAGE=75
export ROUSER_DAEMON_LOG_LEVEL=debug
rouser
```

Environment variables take precedence over configuration file values.

## Logging Strategy

### Log Levels

| Level | Usage |
|-------|-------|
| `debug` | Detailed metric collection logs |
| `info` | State transitions and significant events |
| `warn` | Recoverable errors (e.g., metric collection failure) |
| `error` | Critical errors that may require intervention |

### Log Format

```rust
// JSON format for log aggregation
[logging]
format = "json"

// Text format for readability
format = "text"
```

### Example Logs

```json
{"level":"info","time":"2026-03-26T10:00:00Z","message":"Sleep inhibited: CPU at 85% (threshold: 80%)"}
{"level":"warn","time":"2026-03-26T10:00:05Z","message":"GPU metrics unavailable, using 0%"}
{"level":"info","time":"2026-03-26T10:01:00Z","message":"Releasing sleep inhibition: all metrics below threshold"}
```

## Security Considerations

### Principle of Least Privilege

```ini
# Run as dedicated user
User=rouser
Group=rouser

# Restrict capabilities
CapabilityBoundingSet=CAP_SYS_ADMIN
```

### File Permissions

```bash
# Configuration file: 0600 (owner read/write only)
sudo chmod 0600 /etc/rouser/config.toml

# Log file: 0640 (owner read/write, group read)
sudo chmod 0640 /var/log/rouser/rouser.log
```

## Performance Characteristics

### Memory Usage

| Component | Memory |
|-----------|--------|
| Binary | ~2 MB |
| Runtime data | ~500 KB |
| Per-interface state | ~100 bytes |
| **Total** | **~2-5 MB** |

### CPU Usage

| Interval | CPU Usage |
|----------|-----------|
| 1 second | 0.2-0.5% |
| 5 seconds | 0.05-0.1% |
| 10 seconds | 0.02-0.05% |

See [Performance](../performance.md) for detailed benchmarks.

## Dependencies

### External Crates

```toml
[dependencies]
tokio = { version = "1", features = ["full"] }
zbus = "4"
toml = "0.8"
tracing = "0.1"
tracing-subscriber = "0.3"
anyhow = "1.0"
thiserror = "1.0"
chrono = "0.4"
which = "5.0"
```

### System Dependencies

- Linux with systemd
- D-Bus system bus
- `/proc` filesystem
- Root or `login` group membership for D-Bus access

## Future Considerations

### Planned Features

- [ ] Prometheus metrics endpoint
- [ ] Web UI for monitoring
- [ ] Multi-instance support
- [ ] Plugin architecture for custom metric collectors

### Potential Improvements

- [ ] More efficient metric sampling (event-based instead of polling)
- [ ] Machine learning-based threshold prediction
- [ ] Integration with systemd-resolved for network metrics

## References

- [Linux Kernel Documentation](https://www.kernel.org/doc/html/latest/)
- [zbus Documentation](https://docs.rs/zbus/)
- [TOML Specification](https://toml.io/en/v1.0.0)
- [systemd D-Bus API](https://www.freedesktop.org/software/systemd/man/org.freedesktop.login1.html)

## See Also

- [Quick Start Guide](../quickstart.md)
- [Configuration Reference](../configuration/reference.md)
- [D-Bus Inhibition API](../d-bus/inhibition.md)
- [Performance](../performance.md)
