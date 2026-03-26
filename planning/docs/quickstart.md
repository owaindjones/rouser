# Quick Start Guide

This guide will help you get `rouser` running on your Linux system within 5 minutes.

## Prerequisites

- Linux system with systemd
- Root/sudo access
- Rust 1.70+ and Cargo installed (for building from source)

## Installation Options

### Option 1: Pre-built Binary (Recommended)

Download the latest release binary:

```bash
# Download binary (update version as needed)
curl -LO https://github.com/yourusername/rouser/releases/download/v0.1.0/rouser-x86_64-unknown-linux-gnu.tar.gz

# Extract
tar -xzf rouser-x86_64-unknown-linux-gnu.tar.gz

# Install
sudo install -m 755 -d /usr/local/bin
sudo install -m 755 rouser /usr/local/bin/rouser
```

### Option 2: Build from Source

```bash
# Clone repository
git clone https://github.com/yourusername/rouser.git
cd rouser

# Build release binary
cargo build --release

# Install
sudo install -m 755 target/release/rouser /usr/local/bin/rouser
```

### Option 3: Package Manager (if available)

```bash
# Example for Arch Linux (AUR)
yay -S rouser

# Example for Fedora (Copr repository)
sudo dnf copr enable yourusername/rouser
sudo dnf install rouser
```

## Configuration

### Step 1: Create Configuration Directory

```bash
sudo install -m 755 -d /etc/rouser
```

### Step 2: Create Configuration File

Create `/etc/rouser/config.toml`:

```toml
# /etc/rouser/config.toml

# Daemon configuration
[daemon]
name = "rouser"
update_interval = "5s"
log_level = "info"

# Thresholds (adjust based on your usage patterns)
[thresholds]
cpu_usage = 80.0
gpu_usage = 90.0
network_io = 100.0
disk_activity = 50.0

# Timing parameters
[timing]
duration_threshold = "30s"
idle_duration = "60s"

# D-Bus inhibition settings
[inhibition]
what = ["sleep", "hibernate", "shutdown"]
mode = "block"

# Network configuration
[network]
exclude_interfaces = ["lo"]

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

### Step 3: Set Secure Permissions

```bash
sudo chown root:root /etc/rouser/config.toml
sudo chmod 0600 /etc/rouser/config.toml
```

**Security Note**: The configuration file must be readable only by root to prevent unauthorized modification.

### Step 4: Create Log Directory

```bash
sudo install -m 755 -d /var/log/rouser
sudo chown root:root /var/log/rouser
```

## Testing Configuration

### Validate Configuration

```bash
rouser --validate-config /etc/rouser/config.toml
```

Expected output:

```
Configuration validated successfully
  - /etc/rouser/config.toml
  - All required fields present
  - Threshold values within valid range
  - File paths accessible
```

### Dry Run

Test with a dry run (won't actually inhibit sleep):

```bash
rouser --config /etc/rouser/config.toml --dry-run --duration 60s
```

This will:
- Parse and validate the configuration
- Collect metrics for 60 seconds
- Log what would trigger inhibition
- Exit without inhibiting sleep

## Systemd Service Setup

### Step 1: Copy Service File

```bash
sudo install -m 644 /path/to/rouser.service /etc/systemd/system/rouser.service
```

Or create `/etc/systemd/system/rouser.service`:

```ini
[Unit]
Description=rouser - Linux Sleep Inhibition Daemon
Documentation=https://github.com/yourusername/rouser
After=network.target

[Service]
Type=simple
User=root
Group=root
ExecStart=/usr/local/bin/rouser --config /etc/rouser/config.toml
Restart=on-failure
RestartSec=5s

# Security hardening
NoNewPrivileges=true
ProtectSystem=strict
PrivateTmp=true
CapabilityBoundingSet=CAP_SYS_ADMIN

[Install]
WantedBy=multi-user.target
```

### Step 2: Enable and Start Service

```bash
# Reload systemd daemon
sudo systemctl daemon-reload

# Enable on boot
sudo systemctl enable rouser

# Start service
sudo systemctl start rouser

# Check status
sudo systemctl status rouser
```

Expected output:

```
● rouser.service - rouser - Linux Sleep Inhibition Daemon
     Loaded: loaded (/etc/systemd/system/rouser.service; enabled)
     Active: active (running) since Mon 2026-03-26 10:00:00 UTC; 5min ago
   Main PID: 1234 (rouser)
      Tasks: 4 (limit: 4915)
     Memory: 2.5M
     CGroup: /system.slice/rouser.service
             ���─1234 /usr/local/bin/rouser --config /etc/rouser/config.toml
```

### Step 3: Check Logs

```bash
# View logs
journalctl -u rouser -f

# Or check log file
sudo tail -f /var/log/rouser/rouser.log
```

## Basic Usage

### Start/Stop Service

```bash
# Start
sudo systemctl start rouser

# Stop
sudo systemctl stop rouser

# Restart
sudo systemctl restart rouser

# Reload configuration (without restart)
sudo systemctl reload rouser
```

### Check Service Status

```bash
sudo systemctl status rouser
sudo systemctl is-active rouser
```

### Test Inhibition

Verify sleep inhibition is working:

```bash
# Start rouser with high CPU threshold
rouser --config /etc/rouser/config.toml --dry-run &
PID=$!

# Generate load
yes > /dev/null &

# Try to suspend (should be delayed/blocked)
systemctl suspend

# Cleanup
kill $PID
```

## Troubleshooting

### Common Issues

#### 1. Permission Denied on D-Bus

**Error**: `Access denied to D-Bus system bus`

**Solution**:
```bash
# Run as root
sudo rouser --config /etc/rouser/config.toml

# Or add user to login group
sudo usermod -aG login $USER
```

#### 2. Configuration Validation Failed

**Error**: `Missing required field: thresholds.cpu_usage`

**Solution**: Check `/etc/rouser/config.toml` for all required fields.

#### 3. Service Not Starting

**Error**: `Failed to start rouser`

**Solution**:
```bash
# Check logs
sudo journalctl -u rouser -n 50

# Check config
rouser --validate-config /etc/rouser/config.toml

# Check file permissions
ls -l /etc/rouser/config.toml
```

#### 4. GPU Metrics Not Collected

**Error**: `GPU detection failed, using 0% as fallback`

**Solutions**:
- NVIDIA: `sudo apt install nvidia-utils`
- AMD: Check ROCm installation
- Intel: Verify i915 driver loaded

### Debug Mode

Enable verbose logging:

```toml
# In config.toml
[daemon]
log_level = "debug"
```

Then restart:

```bash
sudo systemctl restart rouser
sudo journalctl -u rouser -f
```

## Next Steps

1. **Adjust Thresholds**: Monitor usage and tune thresholds based on your patterns
2. **Configure Logging**: Set up log rotation and retention
3. **Add Monitoring**: Integrate with Prometheus/Grafana
4. **Security Review**: Ensure compliance with your security policies

## References

- [Configuration Reference](configuration/reference.md)
- [Architecture Overview](architecture/overview.md)
- [Security Best Practices](security.md)
- [D-Bus Inhibition API](d-bus/inhibition.md)

## Example Configurations

### Home Server

```toml
[thresholds]
cpu_usage = 75.0
gpu_usage = 85.0
network_io = 50.0
disk_activity = 30.0

[timing]
duration_threshold = "60s"
idle_duration = "120s"
```

### Development Workstation

```toml
[thresholds]
cpu_usage = 90.0
gpu_usage = 95.0
network_io = 200.0
disk_activity = 100.0

[timing]
duration_threshold = "30s"
idle_duration = "60s"
```

### Headless Database Server

```toml
[thresholds]
cpu_usage = 80.0
network_io = 100.0
disk_activity = 50.0

[timing]
duration_threshold = "45s"
idle_duration = "90s"
```

## See Also

- [Full Configuration Reference](configuration/reference.md)
- [Systemd Service Documentation](systemd/service.md)
- [Performance Tuning](performance.md)
