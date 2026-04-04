# Quick Start Guide

This guide will help you get `rouser` running on your Linux system within 5 minutes.

## Prerequisites

- Linux system with systemd
- Root/sudo access
- Rust 1.70+ and Cargo installed (for building from source)

## Installation Options

### Option 1: Build from Source (Recommended)

```bash
# Clone repository
git clone https://github.com/yourusername/rouser.git
cd rouser

# Build release binary
cargo build --release

# Install to /usr/local/bin
sudo install -m 755 target/release/rouser /usr/local/bin/rouser
```

### Option 2: Pre-built Binary

Download the latest release binary from GitHub releases and extract:

```bash
# Download binary (update version as needed)
curl -LO https://github.com/yourusername/rouser/releases/download/v0.1.0/rouser-x86_64-unknown-linux-gnu.tar.gz

# Extract
tar -xzf rouser-x86_64-unknown-linux-gnu.tar.gz

# Install
sudo install -m 755 rouser /usr/local/bin/rouser
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

# Metric thresholds (adjust based on your usage patterns)
[thresholds]
cpu_usage = 80.0
gpu_usage = 90.0
network_io = 100.0
disk_activity = 50.0

# Timing parameters
[timing]
duration_threshold = "30s"
idle_duration = "60s"

# Inhibition settings
[inhibition]
what = "shutdown:idle"
mode = "block"

# Network configuration
[network]
exclude_interfaces = ["lo"]

# Disk configuration
[disk]
exclude_device_prefixes = ["loop", "fd", "sr", "cdrom"]
```

### Step 3: Set Secure Permissions

```bash
sudo chown root:root /etc/rouser/config.toml
sudo chmod 0600 /etc/rouser/config.toml
```

**Security Note**: The configuration file should be readable only by root to prevent unauthorized modification.

### Step 4: Create Log Directory (Optional)

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

### Dry Run Mode

Test with a dry run (won't actually inhibit sleep):

```bash
rouser --config /etc/rouser/config.toml --dry-run --duration 60s
```

This will:
- Parse and validate the configuration
- Collect metrics for 60 seconds
- Log what would trigger inhibition
- Exit without inhibiting sleep

## Running the Daemon

### Manual Execution

```bash
# Run with default configuration
rouser --config /etc/rouser/config.toml

# Custom config path
rouser --config /path/to/config.toml

# Dry run mode
rouser --config /etc/rouser/config.toml --dry-run
```

### Testing Inhibition

Verify sleep inhibition is working:

```bash
# Start rouser with high CPU threshold in background
rouser --config /etc/rouser/config.toml &
PID=$!

# Generate load
yes > /dev/null &

# Try to suspend (should be delayed/blocked)
systemctl suspend

# Cleanup
kill $PID
```

## Systemd Service Setup

### Step 1: Copy Service File

Create `/etc/systemd/system/rouser.service`:

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
             └─1234 /usr/local/bin/rouser --config /etc/rouser/config.toml
```

### Step 3: Check Logs

```bash
# View logs
journalctl -u rouser -f

# Or check log file
sudo tail -f /var/log/rouser/rouser.log
```

## Basic Service Management

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

## Verifying Inhibition

Check active sleep inhibitors:

```bash
# List active inhibitors
loginctl list-inhibitors

# Or use systemd-inhibit
systemd-inhibit --list
```

Expected output when running:

```
2 inhibitors listed.
```

## Troubleshooting

### Common Issues

#### 1. Permission Denied on D-Bus

**Error**: `Access denied to D-Bus system bus`

**Solution**:
```bash
# Run as root (recommended for systemd service)
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

### Inhibition Not Working on KDE Plasma

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

2. **Run as root** (default for systemd service)

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

## Next Steps

1. **Adjust Thresholds**: Monitor usage and tune thresholds based on your patterns
2. **Configure Logging**: Set up log rotation and retention
3. **Security Review**: Ensure compliance with your security policies

## See Also

- [Configuration Reference](configuration.md) - Complete configuration options
- [Command Line Arguments](command-line.md) - CLI usage details
- [Systemd User Service](systemd-user-service.md) - Detailed service setup
- [Metrics Overview](metrics-overview.md) - How metrics are collected
- [Averaging Explained](averaging.md) - Understanding threshold calculations
