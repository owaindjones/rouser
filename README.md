# rouser

[![Status](https://img.shields.io/badge/status-in%20development-blue)](https://github.com/yourusername/rouser)
[![Rust](https://img.shields.io/badge/rust-stable-orange)](https://www.rust-lang.org)
[![License](https://img.shields.io/badge/license-MIT-blue)](LICENSE)

A Linux daemon that monitors system metrics (CPU, GPU, network, disk) and inhibits sleep when activity thresholds are exceeded.

## Overview

`rouser` is designed for headless servers and desktop systems that need to automatically sleep when idle and stay awake during active use. It monitors multiple system metrics and uses systemd's login1 D-Bus API to prevent unwanted suspend or hibernation.

### Key Features

- **Multi-metric monitoring**: CPU, GPU (NVIDIA/AMD/Intel), network I/O, and disk activity
- **Configurable thresholds**: Set individual percentage thresholds for each metric
- **Hysteresis timing**: Prevents rapid sleep/inhibit cycling with configurable delays
- **Systemd integration**: Uses `org.freedesktop.login1.Manager.Inhibit` for reliable sleep inhibition
- **TOML configuration**: Pure Rust, secure configuration format
- **Dry-run mode**: Test configuration without inhibiting sleep
- **Graceful error handling**: Continues operation even if some metrics are unavailable

### Use Case

Allow a system to:
- Sleep after idle time when no activity is detected
- Stay awake during high CPU/GPU usage (compiling, rendering, gaming)
- Remain active during network transfers or disk I/O
- Be woken by WOL (Wake-on-LAN) packets

## Installation

### From Source

```bash
git clone https://github.com/yourusername/rouser.git
cd rouser
cargo build --release
sudo cp target/release/rouser /usr/local/bin/
```

### Dependencies

- Rust 1.70+
- Systemd (for login1 D-Bus API)
- Optional: NVIDIA drivers with `nvidia-smi` for GPU monitoring
- Linux kernel with `/sys/class/drm` for AMD/Intel GPU monitoring

## Quick Start

1. **Create configuration directory**:
   ```bash
   sudo mkdir -p /etc/rouser
   ```

2. **Create config file** (see Configuration below):
   ```bash
   sudo nano /etc/rouser/config.toml
   ```

3. **Validate configuration**:
   ```bash
   rouser --validate-config /etc/rouser/config.toml
   ```

4. **Test in dry-run mode**:
   ```bash
   rouser --config /etc/rouser/config.toml --dry-run
   ```

5. **Run the daemon**:
   ```bash
   rouser --config /etc/rouser/config.toml
   ```

## Configuration

Create `/etc/rouser/config.toml`:

```toml
name = "rouser"
update_interval = "5s"
log_level = "info"

[thresholds]
cpu_usage = 80.0       # CPU usage percentage (0-100)
gpu_usage = 90.0       # GPU usage percentage (0-100)
network_io = 100.0     # Network throughput in Mbps
disk_activity = 50.0   # Disk I/O in MB/s

[timing]
duration_threshold = "30s"   # Min time above threshold before inhibiting
idle_duration = "60s"        # Time below threshold before releasing

[inhibitor]
what = "sleep"     # Lock type: idle, sleep, suspend, shutdown
mode = "block"     # Mode: block, delay, block-weak

[network]
exclude_interfaces = ["lo"]
include_interfaces = []

[disk]
exclude_device_prefixes = ["loop", "fd", "sr", "cdrom"]
```

### Configuration Options

#### `inhibitor.what` - Lock Type
Controls what type of sleep the inhibitor blocks:
- `idle` - Prevents idle suspend (default behavior)
- `sleep` - Prevents sleep/hibernate
- `suspend` - Prevents suspend-to-RAM
- `shutdown` - Prevents shutdown

Reference: https://systemd.io/INHIBITOR_LOCKS/

#### `inhibitor.mode` - Inhibition Mode
- `block` - Completely blocks sleep
- `delay` - Delays sleep for the duration of inhibition
- `block-weak` - Blocks sleep but can be overridden by privileged processes

#### `thresholds`
Percentage or rate thresholds that trigger sleep inhibition:
- `cpu_usage`: CPU usage percentage (0-100)
- `gpu_usage`: GPU usage percentage (0-100), averaged across all GPUs
- `network_io`: Network throughput in Mbps
- `disk_activity`: Disk I/O in MB/s

#### `timing`
- `duration_threshold`: Minimum time metrics must exceed threshold before inhibiting
- `idle_duration`: Time all metrics must stay below threshold before releasing inhibition

## Command Line Arguments

```
Usage: rouser [OPTIONS]

Options:
  -c, --config <CONFIG>          Path to configuration file [default: /etc/rouser/config.toml]
      --validate-config          Validate configuration and exit
      --dry-run                  Dry run mode (don't actually inhibit sleep)
  -h, --help                     Print help
  -V, --version                  Print version
```

## Running as a Service

### Using systemd (recommended)

Create `/etc/systemd/system/rouser.service`:

```ini
[Unit]
Description=Rouser System Metrics Daemon
After=network.target

[Service]
Type=simple
ExecStart=/usr/local/bin/rouser -c /etc/rouser/config.toml
Restart=always

[Install]
WantedBy=multi-user.target
```

Enable and start:
```bash
sudo systemctl daemon-reload
sudo systemctl enable rouser
sudo systemctl start rouser
```

Check status:
```bash
sudo systemctl status rouser
journalctl -u rouser -f
```

### Manual execution

```bash
# Run with default configuration
rouser

# Custom config path
rouser --config /path/to/config.toml

# Dry run mode
rouser --dry-run

# Validate only
rouser --validate-config
```

## Environment Variables

- `RUST_LOG`: Logging level filter (e.g., `info`, `debug`, `rouser=debug`)
  ```bash
  RUST_LOG=debug rouser
  ```

## Troubleshooting

### Check active inhibitors
```bash
# List active sleep inhibitors
loginctl show-sessions | grep -i inhibited
systemd-inhibit --list
```

### Inhibition not working on KDE Plasma

KDE Powerdevil may ignore inhibitors from unprivileged users. Solutions:

1. **Add polkit rule** (recommended):
   Create `/etc/polkit-1/rules.d/50-rouser.rules`:
   ```javascript
   polkit.addRule(function(action, subject) {
       if (action.id == "org.freedesktop.login1.inhibit" &&
           subject.user == "your_username") {
           return polkit.Result.YES;
       }
   });
   ```

2. **Run as root** (not recommended for security):
   ```bash
   sudo rouser --config /etc/rouser/config.toml
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

### Metrics Collection

- **CPU**: `/proc/stat` (system-wide)
- **GPU**: `nvidia-smi` (NVIDIA) or `/sys/class/drm/device/gpu_busy_percent` (AMD/Intel)
- **Network**: `/proc/net/dev`
- **Disk**: `/proc/diskstats`

### Inhibition Flow

1. Collect metrics every `update_interval` seconds
2. If any metric exceeds threshold, wait for `duration_threshold`
3. Acquire sleep inhibition lock via D-Bus
4. When all metrics below threshold, wait for `idle_duration`
5. Release inhibition lock (file descriptor closes automatically)

## Development

### Building

```bash
# Debug build
cargo build

# Release build
cargo build --release
```

### Running Tests

```bash
cargo test
```

### Code Quality

```bash
# Format code
cargo fmt

# Lint with clippy
cargo clippy -- -D warnings
```

## License

This project is licensed under the MIT License - see the [LICENSE](LICENSE) file for details.
