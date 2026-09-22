# Developer Guide

This guide covers architecture, extending rouser, and coding standards for contributors.

## Table of Contents

1. [Architecture Overview](#architecture-overview)
2. [Project Structure](#project-structure)
3. [Adding New Metrics Modules](#adding-new-metrics-modules)
4. [Implementing New Inhibitor Modules](#implementing-new-inhibitor-modules)
5. [Testing](#testing)
6. [Coding Standards](#coding-standards)
7. [Build and Release](#build-and-release)

## Architecture Overview

### High-Level Architecture

```
┌──────────────┐    ┌───────────┐    ┌───────────┐
│ Config       │───▶│ Core      │◀───│ Metrics   │
│ Loader       │    │ Logic     │    │ Collectors│
└──────────────┘    └─────┬─────┘    └───────────┘
                          │
                     ┌────▼────┐
                     │Threshold│
                     │Manager  │
                     └────┬────┘
                          │
                     ┌────▼────┐
                     │Inhibitor│
                     └────┬────┘
                          │
                     org.freedesktop.login1
```

### Core Components

#### Config Loader (`src/config.rs`)

Parses and validates TOML configuration:

- Reads configuration from file
- Applies environment variable overrides
- Validates all values against constraints
- Returns immutable `Config` struct

```rust
pub struct Config {
    pub daemon: DaemonConfig,
    pub thresholds: ThresholdsConfig,
    pub timing: TimingConfig,
    pub inhibition: InhibitionConfig,
    // ... other sections
}
```

#### Core Logic (`src/service.rs`)

Main event loop coordinating all components:

- Initializes metric collectors
- Runs periodic metric collection
- Checks thresholds and updates inhibition state
- Handles errors gracefully

```rust
pub struct Service {
    config: Config,
    cpu_collector: CpuCollector,
    gpu_collector: GpuCollector,
    network_collector: NetworkCollector,
    disk_collector: DiskCollector,
    threshold_manager: ThresholdManager,
    inhibitor: Option<Login1Inhibitor>,
}

impl Service {
    pub async fn run(&mut self) -> Result<()> {
        loop {
            let metrics = self.collect_all_metrics().await;
            let should_inhibit = self.threshold_manager.check(&metrics);
            self.update_inhibition_state(should_inhibit).await?;
        }
    }
}
```

#### Metric Collectors (`src/metrics/`)

Modular collectors for different system metrics:

- `cpu.rs` - CPU usage from `/proc/stat`
- `gpu.rs` - GPU utilization via NVML (NVIDIA) or sysfs (AMD/Intel)
- `network.rs` - Network I/O from `/proc/net/dev`
- `disk.rs` - Disk activity from `/proc/diskstats`

Each collector implements the `MetricCollector` trait:

```rust
pub trait MetricCollector: Send + Sync {
    async fn collect(&self) -> Result<f64>;
    fn name(&self) -> &str;
}
```

#### Threshold Manager (`src/service.rs`)

Tracks metrics over time and determines inhibition state:

- Maintains smoothing state for each metric
- Applies asymmetric EMA smoothing
- Tracks duration above/below thresholds
- Manages cooldown periods

```rust
pub struct ThresholdManager {
    cpu_state: SmoothingState,
    gpu_state: SmoothingState,
    network_state: SmoothingState,
    disk_state: SmoothingState,
    // ... per-GPU states
    duration_threshold: Duration,
    cooldown_duration: Duration,
}
```

#### Inhibitor (`src/inhibit.rs`)

Interfaces with systemd login1 D-Bus API:

- Acquires sleep inhibition lock
- Releases lock when dropped (RAII pattern)
- Tracks inhibition cookie for debugging

```rust
pub struct Login1Inhibitor {
    fd: Option<RawFd>,
    cookie: Option<String>,
}

impl Login1Inhibitor {
    pub async fn acquire(what: &str, mode: &str) -> Result<Self> {
        // Call org.freedesktop.login1.Manager.Inhibit
    }
}

impl Drop for Login1Inhibitor {
    fn drop(&mut self) {
        // Close fd to release inhibition
    }
}
```

## Project Structure

```
rouser/
├── Cargo.toml              # Project manifest and dependencies
├── README.md               # Project overview
├── docs/                   # Documentation (this file)
│   ├── introduction.md
│   ├── installation.md
│   ├── configuration.md
│   ├── command-line.md
│   ├── systemd-user-service.md
│   ├── metrics-overview.md
│   ├── averaging.md
│   └── developer-guide.md
├── etc/                    # Example configuration files
│   └── rouser/
│       └── config.toml.example
├── src/
│   ├── main.rs             # Entry point
│   ├── config.rs           # Configuration parsing
│   ├── service.rs          # Core service logic
│   ├── inhibit.rs          # D-Bus inhibition
│   └── metrics/            # Metric collectors
│       ├── mod.rs          # Module exports
│       ├── cpu.rs          # CPU metrics
│       ├── gpu.rs          # GPU metrics
│       ├── network.rs      # Network metrics
│       └── disk.rs         # Disk metrics
└── tests/                  # Integration tests (if any)
```

### Key Files

| File | Purpose |
|------|---------|
| `src/main.rs` | Application entry point, CLI parsing |
| `src/config.rs` | TOML configuration loading and validation |
| `src/service.rs` | Main service loop, threshold management |
| `src/inhibit.rs` | D-Bus inhibition implementation |
| `src/metrics/mod.rs` | Metric collector trait and exports |
| `src/metrics/*.rs` | Individual metric collectors |

## Adding New Metrics Modules

### Step 1: Create the Collector Module

Create a new file in `src/metrics/`:

```rust
// src/metrics/memory.rs
use crate::config::Config;
use anyhow::Result;

pub struct MemoryCollector {
    // Configuration if needed
}

impl MemoryCollector {
    pub fn new(_config: &Config) -> Self {
        Self {}
    }
}

#[async_trait::async_trait]
impl super::MetricCollector for MemoryCollector {
    async fn collect(&self) -> Result<f64> {
        // Read from /proc/meminfo or other source
        let content = tokio::fs::read_to_string("/proc/meminfo").await?;
        
        // Parse memory values
        let total = parse_meminfo_value(&content, "MemTotal:");
        let available = parse_meminfo_value(&content, "MemAvailable:");
        
        // Calculate usage percentage
        if total > 0 {
            let used = total - available;
            Ok((used as f64 / total as f64) * 100.0)
        } else {
            Ok(0.0)
        }
    }
    
    fn name(&self) -> &str {
        "memory"
    }
}

fn parse_meminfo_value(content: &str, key: &str) -> u64 {
    content
        .lines()
        .find(|line| line.starts_with(key))
        .and_then(|line| {
            line.split_whitespace()
                .nth(1)
                .and_then(|v| v.parse().ok())
        })
        .unwrap_or(0)
}
```

### Step 2: Add to Module Exports

Update `src/metrics/mod.rs`:

```rust
pub mod cpu;
pub mod gpu;
pub mod network;
pub mod disk;
pub mod memory;  // New module

pub use cpu::CpuCollector;
pub use gpu::GpuCollector;
pub use network::NetworkCollector;
pub use disk::DiskCollector;
pub use memory::MemoryCollector;

pub trait MetricCollector: Send + Sync {
    async fn collect(&self) -> Result<f64>;
    fn name(&self) -> &str;
}
```

### Step 3: Update Configuration

Add threshold configuration in `src/config.rs`:

```rust
#[derive(Debug, Deserialize)]
pub struct ThresholdsConfig {
    pub cpu_usage: f64,
    pub gpu_usage: f64,
    pub network_io: f64,
    pub disk_activity: f64,
    pub memory_usage: Option<f64>,  // Optional new metric
}
```

### Step 4: Update Core Service

Add collector to `Service` in `src/service.rs`:

```rust
pub struct Service {
    // ... existing fields
    memory_collector: Option<MemoryCollector>,  // New field
}

impl Service {
    pub fn new(config: Config) -> Result<Self> {
        Ok(Self {
            // ... existing initializers
            memory_collector: config.thresholds.memory_usage.map(|_| {
                MemoryCollector::new(&config)
            }),
        })
    }
    
    async fn collect_all_metrics(&self) -> Metrics {
        let metrics = Metrics {
            cpu: self.cpu_collector.collect().await,
            gpu: self.gpu_collector.collect().await,
            network: self.network_collector.collect().await,
            disk: self.disk_collector.collect().await,
            memory: self.memory_collector.as_ref()
                .map(|c| futures::poll!(c.collect()))
                .flatten()
                .unwrap_or(0.0),  // Fallback if collector unavailable
        };
        
        debug!("Collected metrics: {:?}", metrics);
        metrics
    }
}
```

### Step 5: Update Threshold Manager

Add smoothing state for the new metric:

```rust
pub struct ThresholdManager {
    // ... existing fields
    memory_state: Option<SmoothingState>,
}

impl ThresholdManager {
    pub fn new(config: &Config) -> Self {
        Self {
            // ... existing initializers
            memory_state: config.thresholds.memory_usage.map(|_| {
                SmoothingState::new(config.thresholds.memory_ema_alpha.unwrap_or(0.1))
            }),
        }
    }
    
    pub fn check(&self, config: &Config) -> bool {
        // Check each metric against its threshold using smoothed values
        let cpu_ok = self.check_metric(&self.cpu_state, metrics.cpu.usage(), config.metrics.cpu.per_core_threshold);
        let gpu_agg = GpuAggregate::from_gpus(&metrics.gpu_usage);
        let gpu_ok = gpu_agg.per_gpu_max > config.metrics.gpu.per_gpu_threshold
            || gpu_agg.total_average > config.metrics.gpu.total_threshold;
        // ... similar for network and disk
        cpu_ok || gpu_ok || /* others */ false
    }
}
```

## Implementing New Inhibitor Modules

### Step 1: Define the Inhibitor Trait

Create a trait for inhibitor implementations:

```rust
// src/inhibit.rs

pub trait SleepInhibitor: Send + Sync {
    /// Acquire the inhibition lock
    async fn acquire(&mut self, what: &str, description: &str) -> Result<()>;
    
    /// Release the inhibition lock
    async fn release(&mut self);
    
    /// Check if currently inhibited
    fn is_inhibited(&self) -> bool;
}
```

### Step 2: Implement a New Inhibitor

Example: Custom inhibitor for a specific use case:

```rust
// src/inhibit/custom.rs
use super::SleepInhibitor;
use anyhow::Result;

pub struct CustomInhibitor {
    inhibited: bool,
    cookie: Option<String>,
}

impl CustomInhibitor {
    pub fn new() -> Self {
        Self {
            inhibited: false,
            cookie: None,
        }
    }
}

impl Default for CustomInhibitor {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait::async_trait]
impl SleepInhibitor for CustomInhibitor {
    async fn acquire(&mut self, what: &str, description: &str) -> Result<()> {
        // Custom inhibition logic here
        // Could write to a file, make HTTP request, etc.
        
        self.inhibited = true;
        self.cookie = Some(format!("custom-{}", Uuid::new_v4()));
        
        info!("Acquired custom inhibition: {} - {}", what, description);
        Ok(())
    }
    
    async fn release(&mut self) {
        if self.inhibited {
            info!("Released custom inhibition: {}", self.cookie.as_ref().unwrap());
            self.inhibited = false;
            self.cookie = None;
        }
    }
    
    fn is_inhibited(&self) -> bool {
        self.inhibited
    }
}
```

### Step 3: Update Configuration

Add inhibitor selection to config:

```rust
#[derive(Debug, Deserialize)]
pub struct InhibitionConfig {
    #[serde(default)]
    pub inhibitor_type: String,  // "login1", "custom", etc.

    #[serde(default)]
    pub what: String,

    #[serde(default)]
    pub mode: String,
}

impl Default for InhibitionConfig {
    fn default() -> Self {
        Self {
            inhibitor_type: "login1".to_string(),
            what: "shutdown:idle".to_string(),
            mode: "block".to_string(),
        }
    }
}
```

### Step 4: Factory Pattern for Inhibitors

Create a factory to instantiate inhibitors:

```rust
// src/inhibit.rs

pub enum InhibitorType {
    Login1,
    Custom,
}

impl SleepInhibitorFactory {
    pub fn create(config: &InhibitionConfig) -> Result<Box<dyn SleepInhibitor>> {
        match config.inhibitor_type.as_str() {
            "login1" => Ok(Box::new(Login1Inhibitor::new())),
            "custom" => Ok(Box::new(CustomInhibitor::new())),
            other => Err(anyhow!("Unknown inhibitor type: {}", other)),
        }
    }
}
```

## Testing

### Unit Tests

Write unit tests for individual components:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_ema_smoothing_rising() {
        let mut state = SmoothingState::new(0.1);
        
        // Rising edge should respond quickly
        let result = state.update(20.0);  // Initial
        assert!((result - 20.0).abs() < 0.01);
        
        let result = state.update(100.0);  // Spike
        assert!((result - 36.0).abs() < 0.1);  // Should be ~36% (fast response)
    }
    
    #[test]
    fn test_ema_smoothing_falling() {
        let mut state = SmoothingState::new(0.1);
        
        // Start at high value
        for _ in 0..10 {
            state.update(100.0);
        }
        
        // Falling edge should be slow
        let result = state.update(20.0);
        assert!(result > 90.0);  // Should still be high
    }
    
    #[test]
    fn test_threshold_manager_duration() {
        let config = Config::default();
        let mut manager = ThresholdManager::new(&config);
        
        // Feed values above threshold
        for _ in 0..6 {  // 6 samples at 5s = 30s
            manager.update_cpu(85.0);  // Above 80% threshold
        }
        
        assert!(manager.should_inhibit());
    }
}
```

### Integration Tests

Test the full service loop:

```rust
#[cfg(test)]
mod integration_tests {
    use tokio::time::{timeout, Duration};
    
    #[tokio::test]
    async fn test_service_runs() {
        let config = Config {
            daemon: DaemonConfig {
                update_interval: Duration::from_millis(100),  // Fast for testing
                ..Default::default()
            },
            metrics: MetricsConfig {
                cpu: CpuConfig { threshold: 50.0, ..Default::default() },
                gpu: GpuConfig::default(),
                network: NetworkConfig::default(),
                disk: DiskConfig::default(),
            },
            timing: TimingConfig {
                duration_threshold: Duration::from_millis(200),
                cooldown_duration: Duration::from_millis(100),
            },
            inhibition: InhibitionConfig {
                what: "sleep".to_string(),
                mode: "block".to_string(),
                ..Default::default()
            },
            ..Default::default()
        };
        
        let mut service = Service::new(config).unwrap();
        
        // Run for a short time with timeout
        let result = timeout(
            Duration::from_secs(5),
            service.run()
        ).await;
        
        assert!(result.is_ok());
    }
}
```

### Running Tests

```bash
# Run all tests
cargo test

# Run specific test
cargo test test_ema_smoothing_rising

# Run with output
cargo test -- --nocapture

# Run integration tests only
cargo test --test '*'
```

## Coding Standards

### Code Style

rouser follows Rust's official style guidelines with minor additions:

#### Formatting

```bash
# Format code
cargo fmt

# Check formatting
cargo fmt --check
```

#### Linting

```bash
# Run clippy
cargo clippy -- -D warnings

# Fix auto-fixable warnings
cargo clippy --fix
```

### Naming Conventions

| Type | Convention | Example |
|------|------------|---------|
| Structs | PascalCase | `ThresholdManager`, `CpuCollector` |
| Types/Traits | PascalCase | `MetricCollector`, `InhibitorType` |
| Functions | snake_case | `collect_cpu_usage`, `should_inhibit` |
| Variables | snake_case | `cpu_usage`, `duration_threshold` |
| Constants | SCREAMING_SNAKE_CASE | `DEFAULT_ALPHA`, `MAX_RETRIES` |
| Modules | snake_case | `threshold_manager`, `sleep_inhibitor` |

### Error Handling

Use `anyhow::Result` for error handling:

```rust
use anyhow::{Context, Result};

fn parse_config(path: &Path) -> Result<Config> {
    let content = std::fs::read_to_string(path)
        .with_context(|| format!("Failed to read config: {}", path.display()))?;
    
    let config: Config = toml::from_str(&content)
        .context("Failed to parse TOML")?;
    
    Ok(config)
}
```

Use `thiserror` for custom error types:

```rust
use thiserror::Error;

#[derive(Error, Debug)]
pub enum GpuError {
    #[error("NVIDIA drivers not installed")]
    NvidiaNotAvailable,
    
    #[error("AMD/Intel sysfs not available")]
    SysfsNotAvailable,
    
    #[error("Failed to parse GPU usage: {0}")]
    ParseError(String),
}
```

### Async/Await

Use `async`/`await` for I/O operations:

```rust
// Good: Proper async/await usage
pub async fn collect_metrics(&self) -> Result<f64> {
    let content = tokio::fs::read_to_string("/proc/stat").await?;
    Ok(parse_cpu_usage(&content))
}

// Bad: Blocking in async context
pub async fn collect_metrics_bad(&self) -> Result<f64> {
    let content = std::fs::read_to_string("/proc/stat")?;  // Blocks!
    Ok(parse_cpu_usage(&content))
}
```

Use `tokio::spawn` for parallel operations:

```rust
let (cpu, gpu) = tokio::join!(
    cpu_collector.collect(),
    gpu_collector.collect()
);
```

### Logging

Use the `tracing` crate for logging:

```rust
use tracing::{debug, info, warn, error};

pub fn process_metrics(metrics: &Metrics) {
    debug!("Collected metrics: {:?}", metrics);
    
    if metrics.cpu > 90.0 {
        warn!("High CPU usage: {:.1}%", metrics.cpu);
    }
    
    if should_inhibit {
        info!("Inhibiting sleep due to high activity");
    }
    
    if error_occurred {
        error!("Failed to collect GPU metrics: {}", error);
    }
}
```

### Documentation

Document public APIs:

```rust
/// CPU usage collector from /proc/stat
pub struct CpuCollector {
    last_stats: Option<CpuStats>,
}

impl CpuCollector {
    /// Create a new CPU collector
    pub fn new() -> Self {
        Self { last_stats: None }
    }
    
    /// Collect current CPU usage percentage (0-100)
    /// 
    /// # Returns
    /// 
    /// CPU usage as a percentage (0.0 to 100.0)
    pub async fn collect(&self) -> Result<f64> {
        // ... implementation
    }
}
```

### Tests

Write tests for all public functions:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_valid_input() {
        // Test happy path
    }
    
    #[test]
    fn test_edge_cases() {
        // Test boundary values
    }
    
    #[test]
    fn test_error_handling() {
        // Test error cases
    }
}
```

## Build and Release

### Building

```bash
# Debug build (default)
cargo build

# Release build
cargo build --release

# Cross-compile for Linux musl
rustup target add x86_64-unknown-linux-musl
cargo build --target x86_64-unknown-linux-musl --release
```

### Running

```bash
# Run with default config
cargo run

# Run with custom config
cargo run -- --config /path/to/config.toml

# Dry run mode
cargo run -- --dry-run --duration 60s

# Validate config
cargo run -- --validate-config /etc/rouser/config.toml
```

### Testing

```bash
# Run all tests
cargo test

# Run with coverage (requires cargo-tarpaulin)
cargo tarpaulin --out Html

# Integration tests
cargo test --test integration
```

### Code Quality

```bash
# Format code
cargo fmt

# Lint with clippy
cargo clippy -- -D warnings

# Check documentation
cargo doc --open
```

### Releasing

1. Update version in `Cargo.toml`
2. Update `CHANGELOG.md` with changes
3. Tag release: `git tag v0.1.0 && git push --tags`
4. Build release binary:
   ```bash
   cargo build --release
   ```
5. Create tarball:
   ```bash
   tar -czvf rouser-0.1.0-x86_64-unknown-linux-gnu.tar.gz \
       target/release/rouser README.md LICENSE
   ```
6. Upload to GitHub releases

## Contributing

### Getting Started

1. Fork the repository
2. Clone locally: `git clone https://github.com/owaindjones/rouser.git`
3. Create a branch: `git checkout -b feature/your-feature`
4. Make changes and write tests
5. Run `cargo test && cargo fmt && cargo clippy -- -D warnings`
6. Commit and push: `git push origin feature/your-feature`
7. Open a pull request

### Pull Request Guidelines

- Follow the coding standards above
- Include tests for new functionality
- Update documentation as needed
- Keep PRs focused and small
- Write clear commit messages

## See Also

- [Installation](installation.md) - Getting started with rouser
- [Configuration Reference](configuration.md) - Configuration options
- [Metrics Overview](metrics-overview.md) - How metrics are collected
