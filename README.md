# rouser

[![Status](https://img.shields.io/badge/status-in%20development-blue)](https://github.com/yourusername/rouser)
[![Rust](https://img.shields.io/badge/rust-stable-orange)](https://www.rust-lang.org)
[![License](https://img.shields.io/badge/license-MIT-blue)](LICENSE)

A Linux daemon that monitors system metrics and inhibits sleep when activity thresholds are exceeded.

## Overview

`rouser` is designed for headless servers that need to automatically sleep when idle and stay awake during active use. Unlike desktop systems that use mouse/keyboard activity to detect user presence, headless servers require programmatic monitoring of system metrics to determine when sleep is appropriate.

### Key Features

- **Multi-metric monitoring**: CPU, GPU, network, and disk I/O usage
- **Configurable thresholds**: Set individual thresholds for each metric type
- **Hysteresis timing**: Prevents rapid sleep/inhibit cycling
- **D-Bus integration**: Uses systemd-logind for system-wide sleep inhibition
- **TOML configuration**: Secure, simple, pure Rust configuration format
- **Zero external dependencies**: Uses `/proc` filesystem for all metrics
- **Graceful error handling**: Continues operation even if some metrics are unavailable

### Use Case

Allow a headless server to:
- Sleep after 15 minutes of idle time
- Stay awake when CPU, GPU, network, or disk usage exceeds thresholds
- Be woken by WOL (Wake-on-LAN) packets
- Auto-inhibit sleep during active workloads

## Installation

### From Source

```bash
git clone https://github.com/yourusername/rouser.git
cd rouser
cargo build --release
sudo cp target/release/rouser /usr/local/bin/
```

### Configuration

Create `/etc/rouser/config.toml`:

```bash
sudo mkdir -p /etc/rouser
sudo cp /path/to/rouser/etc/rouser/config.toml.example /etc/rouser/config.toml
sudo chmod 0600 /etc/rouser/config.toml
```

## Usage

### Basic Usage

```bash
# Validate configuration
rouser --validate-config /etc/rouser/config.toml

# Dry run (don't inhibit sleep)
rouser --config /etc/rouser/config.toml --dry-run --duration 60s

# Start daemon
rouser --config /etc/rouser/config.toml
```

### Environment Variable Overrides

```bash
export ROUSER_THRESHOLDS_CPU_USAGE=75
export ROUSER_DAEMON_LOG_LEVEL=debug
rouser --config /etc/rouser/config.toml
```

### Systemd Service

1. Copy service file:
```bash
sudo cp rouser.service /etc/systemd/system/
sudo systemctl daemon-reload
```

2. Enable and start:
```bash
sudo systemctl enable rouser
sudo systemctl start rouser
```

3. Check status:
```bash
sudo systemctl status rouser
```

## Configuration

See the [Configuration Reference](planning/docs/configuration/reference.md) for complete documentation of all configuration options.

### Example Configuration

```toml
[daemon]
name = "rouser"
update_interval = "5s"
log_level = "info"

[thresholds]
cpu_usage = 80.0
gpu_usage = 90.0
network_io = 100.0
disk_activity = 50.0

[timing]
duration_threshold = "30s"
idle_duration = "60s"

[inhibition]
what = ["sleep", "hibernate", "shutdown"]
mode = "block"
```

## Architecture

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

## Documentation

- [Quick Start Guide](planning/docs/quickstart.md)
- [Configuration Reference](planning/docs/configuration/reference.md)
- [D-Bus Inhibition API](planning/docs/d-bus/inhibition.md)
- [Metric Collection](planning/docs/metrics/cpu.md)
- [Systemd Integration](planning/docs/systemd/service.md)
- [Security Best Practices](planning/docs/security.md)
- [Performance](planning/docs/performance.md)

## Technical Details

### Metrics Sources

- **CPU**: `/proc/stat` (system-wide)
- **GPU**: `nvidia-smi` (NVIDIA) or `/sys/class/drm/` (AMD/Intel)
- **Network**: `/proc/net/dev`
- **Disk**: `/proc/diskstats`

### D-Bus Integration

Uses `org.freedesktop.login1` D-Bus interface:
- Method: `Inhibit()`
- Purpose: Prevent system sleep
- Release: Close file descriptor

## Development

### Building

```bash
cargo build --release
```

### Running Tests

```bash
cargo test
```

### Code Formatting

```bash
cargo fmt
```

## License

This project is licensed under the MIT License - see the [LICENSE](LICENSE) file for details.

## Contributing

Contributions are welcome! Please read [CONTRIBUTING.md](CONTRIBUTING.md) before submitting PRs.
