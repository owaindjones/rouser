# rouser

A Linux daemon that monitors system metrics and inhibits sleep when activity thresholds are exceeded.

![Status](https://img.shields.io/badge/status-planning-blue)
![Rust](https://img.shields.io/badge/rust-stable-orange)
![License](https://img.shields.io/badge/license-MIT-blue)

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

### Option 1: From Source

```bash
git clone https://github.com/yourusername/rouser.git
cd rouser
cargo build --release
sudo cp target/release/rouser /usr/local/bin/
```

### Option 2: Pre-built Binary

Download the latest release from the Releases page.

## Configuration

Create `/etc/rouser/config.toml`:

```toml
# /etc/rouser/config.toml

# Daemon configuration
[daemon]
name = "rouser"
update_interval = "5s"
log_level = "info"

# Thresholds (percentages and MB/s)
[thresholds]
cpu_usage = 80.0
gpu_usage = 90.0
network_io = 100.0
disk_activity = 50.0

# Timing parameters
[timing]
duration_threshold = "30s"  # Time above threshold to inhibit
idle_duration = "60s"        # Time below threshold before release

# D-Bus inhibition settings
[inhibition]
what = ["sleep", "hibernate", "shutdown"]
mode = "block"

# Network configuration
[network]
exclude_interfaces = ["lo"]  # Exclude loopback

# Disk configuration
[disk]
exclude_device_prefixes = ["loop", "fd", "sr", "cdrom"]

# Logging
[logging]
file = "/var/log/rouser/rouser.log"
rotation.max_size_mb = 10
rotation.max_files = 5
format = "text"
```

**Security Note**: Configuration file should have mode `0600` (owner read/write only).

```bash
sudo chmod 0600 /etc/rouser/config.toml
```

## Usage

### Systemd Service

1. **Copy service file**:
   ```bash
   sudo cp rouser.service /etc/systemd/system/
   sudo systemctl daemon-reload
   ```

2. **Enable and start**:
   ```bash
   sudo systemctl enable rouser
   sudo systemctl start rouser
   ```

3. **Check status**:
   ```bash
   sudo systemctl status rouser
   ```

### Command Line

```bash
# Validate configuration
rouser --validate-config /etc/rouser/config.toml

# Dry run (don't inhibit sleep)
rouser --config /etc/rouser/config.toml --dry-run --duration 60s

# Start daemon
rouser --config /etc/rouser/config.toml

# Environment variable overrides
ROUSER_LOG_LEVEL=debug rouser --config /etc/rouser/config.toml
```

## Architecture

```
┌──��������──��������──��������─┐     ����──��������──��������──��������─┐     ����──��������──��������──��������─┐
│  Config     ����──��������▶│    Core     ����──��������▶│  Metrics    ����
│  Loader     ����     ����   Logic     ����     ���� Collectors  ����
└──��������──��������──��������─┘     ����──��������──┬──��������──┘     ����──��������──��������──��������─┘
                           ����
                    ����──��������──▼──��������──┐
                    ���� Threshold   ����
                    ���� Manager     ����
                    ����──��������──┬──��������──┘
                           ����
                    ����──��������──▼──��������──┐
                    ����  D-Bus      ����
                    ���� Client      ����
                    ����──��������──��������──��������─┘
```

See [docs/architecture/overview.md](docs/architecture/overview.md) for detailed architecture documentation.

## Documentation

- [Quick Start Guide](docs/quickstart.md) - Installation and basic usage
- [Configuration Reference](docs/configuration/reference.md) - All configuration options
- [D-Bus Inhibition API](docs/d-bus/inhibition.md) - Sleep inhibition details
- [Metric Collection](docs/metrics/cpu.md) - How metrics are collected
- [Systemd Integration](docs/systemd/service.md) - Service configuration
- [Security Best Practices](docs/security.md) - Security hardening
- [Performance Characteristics](docs/performance.md) - Benchmarks and optimization

## Technical Details

### Metrics Sources

- **CPU**: `/proc/stat` (system-wide)
- **GPU**: `nvidia-smi` (NVIDIA) or `/sys/class/drm/` (AMD/Intel)
- **Network**: `/proc/net/dev`
- **Disk**: `/proc/diskstats`
- **Memory**: `/proc/meminfo`

### D-Bus Integration

Uses `org.freedesktop.login1` D-Bus interface:
- Method: `Inhibit()`
- Purpose: Prevent system sleep
- Release: Close file descriptor

### Error Handling

- Graceful degradation: Continues operation even if metrics are unavailable
- Logging: Clear error messages for troubleshooting
- Fallbacks: Zero values for missing metrics

## Configuration Format: TOML

`rouser` uses TOML instead of YAML for configuration:

- **Pure Rust**: No C dependencies (`serde_yaml` has security vulnerabilities)
- **Simple**: Easier to read and maintain
- **Secure**: Actively maintained in Rust ecosystem
- **Native support**: `toml` crate (v0.8+)

## Security

For production deployments, see [docs/security.md](docs/security.md) for:

- Configuration file permissions (mode 0600)
- Service account configuration
- D-Bus permission requirements
- Dependency vulnerability management

## Roadmap

- [x] Phase 1: Planning & Research (Complete)
- [ ] Phase 2: Project Setup (In Progress)
- [ ] Phase 3: Implementation
- [ ] Phase 4: Systemd Integration
- [ ] Phase 5: Testing
- [ ] Phase 6: Release

## Contributing

Contributions are welcome! Please read [CONTRIBUTING.md](CONTRIBUTING.md) before submitting PRs.

## License

This project is licensed under the MIT License - see the [LICENSE](LICENSE) file for details.

## Acknowledgments

- Linux kernel developers for `/proc` filesystem
- systemd team for D-Bus API
- zbus community for Rust D-Bus bindings

## References

- [Linux /proc Documentation](https://www.kernel.org/doc/html/latest/filesystems/proc.html)
- [systemd D-Bus API](https://www.freedesktop.org/software/systemd/man/org.freedesktop.login1.html)
- [zbus Documentation](https://docs.rs/zbus/)
- [TOML Specification](https://toml.io/)
