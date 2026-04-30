# Running rouser as a Systemd User Service

This document describes how to configure `rouser` as a systemd user service for automatic startup and management.

## Overview

Systemd user services allow rouser to:
- Start automatically when you log in
- Run in your user context (no root required)
- Be managed with standard systemd commands
- Log to your user's journal

## Prerequisites

- Systemd user instance running (`systemctl --user daemon-reexec` may be needed)
- D-Bus session bus available
- User membership in `login` group (for D-Bus access) or polkit rule
- **logind lingering enabled** for the user, so the service persists after logout:

```bash
loginctl enable-linger $USER
```

## Installation Steps

### Step 1: Create Configuration Directory and File

Copy the repo-packaged default config into your XDG path:

```bash
mkdir -p ~/.config/rouser
cp ./config/rouser.toml ~/.config/rouser/config.toml
# Or create a custom one — see docs/configuration.md for full reference
cat > ~/.config/rouser/config.toml << 'EOF'
update_interval = "5s"
log_level = "info"

[metrics.cpu]
per_core_threshold = 80.0   # CPU max usage % (0–100) above which to inhibit sleep
total_threshold = 25.0      # Total averaged CPU usage % (default: 25.0)

[metrics.gpu]
threshold = 15.0            # GPU usage % per device (default: 15.0)
ema_alpha = 0.7             # EMA smoothing factor for GPU readings

[metrics.network]
threshold = 10.0            # Network throughput in Mbps (default: 10.0)
ema_alpha = 0.5             # EMA smoothing for network I/O
exclude_interfaces = ["lo"] # Exclude loopback from monitoring
include_interfaces = []     # Only monitor these; empty means all

[metrics.disk]
threshold = 10.0            # Disk I/O in MB/s (default: 10.0)
ema_alpha = 0.5             # EMA smoothing for disk activity
exclude_device_prefixes = ["loop", "fd", "sr", "cdrom"]  # Exclude virtual devices

[timing]
duration_threshold = "5s"   # Min time metrics must exceed threshold before inhibiting (default: 5s)
cooldown_duration = "10s"   # Time after releasing inhibition before re-inhibiting possible (default: 10s)

[inhibitor]
what = "shutdown:idle"       # Lock types: idle, sleep, suspend, shutdown (colon-separated)
mode = "block"               # Mode: block, delay, or block-weak
EOF
```

### Step 2: Create User Service File

Create `~/.config/systemd/user/rouser.service`:

```ini
[Unit]
Description=Rouser - User Sleep Inhibition Daemon
Documentation=https://github.com/owaindjones/rouser
After=graphical-session.target
Wants=network-online.target

[Service]
Type=simple
ExecStart=/home/%i/.local/bin/rouser --config /home/%i/.config/rouser/config.toml
Restart=on-failure
RestartSec=5s

# Security hardening
NoNewPrivileges=true
ProtectSystem=strict
PrivateTmp=true
ProtectHome=read-only
ReadWritePaths=%h/.config/rouser

[Install]
WantedBy=default.target
```

### Step 3: Create Log Directory

```bash
mkdir -p ~/.local/log/rouser
```

### Step 4: Configure and Start Service

```bash
# Reload user systemd daemon
systemctl --user daemon-reload

# Enable service to start on login
systemctl --user enable rouser

# Start the service
systemctl --user start rouser

# Check status
systemctl --user status rouser
```

Expected output:

```
● rouser.service - Rouser - User Sleep Inhibition Daemon
     Loaded: loaded (/home/username/.config/systemd/user/rouser.service; enabled)
     Active: active (running) since Mon 2026-03-26 10:00:00 UTC; 5min ago
   Main PID: 1234 (rouser)
      Tasks: 4 (limit: 4915)
     Memory: 2.5M
```

### Step 5: Verify Inhibition

Check active inhibitors:

```bash
# List active inhibitors
loginctl list-inhibitors

# Should show rouser as an inhibitor
```

## Service Management

### Start/Stop/Restart

```bash
# Start service
systemctl --user start rouser

# Stop service
systemctl --user stop rouser

# Restart service
systemctl --user restart rouser

# Reload configuration (without restart)
systemctl --user reload rouser
```

### Check Status

```bash
# Check if running
systemctl --user is-active rouser

# View detailed status
systemctl --user status rouser

# View logs
journalctl --user -u rouser -f

# View last 50 lines
journalctl --user -u rouser -n 50

# View logs for specific time range
journalctl --user -u rouser --since "2024-03-26 00:00:00" --until "2024-03-26 23:59:59"
```

### Enable/Disable

```bash
# Enable on login
systemctl --user enable rouser

# Disable (but keep file)
systemctl --user disable rouser

# Check if enabled
systemctl --user is-enabled rouser
```

## Alternative: Systemd System Service

For system-wide installation (requires root):

### Create System Service File

Create `/etc/systemd/system/rouser.service`:

```ini
[Unit]
Description=Rouser - System Sleep Inhibition Daemon
Documentation=https://github.com/owaindjones/rouser
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
ReadWritePaths=/var/log/rouser

[Install]
WantedBy=multi-user.target
```

### Install and Start

```bash
# Copy service file
sudo cp rouser.service /etc/systemd/system/

# Reload daemon
sudo systemctl daemon-reload

# Enable and start
sudo systemctl enable rouser
sudo systemctl start rouser

# Check status
sudo systemctl status rouser

# View logs
sudo journalctl -u rouser -f
```

## Security Hardening

### User Service (Recommended)

The user service runs with your user's privileges and has limited access:

```ini
[Service]
User=%u
NoNewPrivileges=true
ProtectSystem=strict
PrivateTmp=true
ProtectHome=true
ReadWritePaths=%h/.config/rouser %h/.local/log/rouser
```

### System Service (More Restrictive)

For system-wide installation with enhanced security:

```ini
[Service]
Type=simple
User=rouser
Group=rouser
ExecStart=/usr/local/bin/rouser --config /etc/rouser/config.toml
Restart=on-failure
RestartSec=5s

# Create dedicated user
# UserAdd -r -s /usr/bin/false -U -M -r rouser

# Grant only necessary capabilities
CapabilityBoundingSet=CAP_SYS_ADMIN
AmbientCapabilities=CAP_SYS_ADMIN

# Restrict filesystem access
ProtectSystem=strict
ProtectHome=true
PrivateTmp=true
PrivateDevices=true
ReadWritePaths=/var/log/rouser
ReadOnlyPaths=/etc/rouser

# System call filtering
SystemCallFilter=@system-service
SystemCallFilter=~@privileged @resources
```

## Environment Configuration

### Override Logging via systemd environment

Since rouser has no `ROUSER_*` config overrides, use systemd's `Environment=` to set `RUST_LOG`:

```bash
systemctl --user edit rouser.service
```

Add:
```ini
[Service]
Environment="RUST_LOG=debug"
```

Reload and restart:
```bash
systemctl --user daemon-reload
systemctl --user restart rouser
```

Or override the log level via CLI by editing `ExecStart` in the service file to include `-l debug`.

### Resource Limits

Prevent resource exhaustion:

```ini
[Service]
Type=simple
ExecStart=/usr/local/bin/rouser --config /etc/rouser/config.toml

# Resource limits
MemoryLimit=256M
MemoryHigh=200M
CPUQuota=50%
TasksMax=10
```

## Troubleshooting

### Service Won't Start

**Check if user systemd is running**:

```bash
systemctl --user status
```

If not running, log out and back in to start the user instance.

**Check D-Bus access**:

```bash
# Verify D-Bus session is available
echo $DBUS_SESSION_BUS_ADDRESS

# Check if rouser can reach D-Bus
dbus-send --session --print-reply \
  --dest=org.freedesktop.DBus \
  /org/freedesktop/DBus \
  org.freedesktop.DBus.Peer.Ping
```

**Check logs**:

```bash
journalctl --user -u rouser -n 100 --no-pager
```

### Inhibition Not Working

**Verify rouser is running**:

```bash
systemctl --user status rouser
```

**Check active inhibitors**:

```bash
loginctl list-inhibitors
```

**Test D-Bus access manually**:

```bash
# Try to inhibit sleep manually
systemd-inhibit --what=sleep --mode=block --description="Test" \
  -- sh -c "sleep 60; echo Done"
```

### KDE Plasma Issues

KDE Powerdevil may ignore inhibitors from unprivileged users.

**Add polkit rule**:

Create `/etc/polkit-1/rules.d/50-rouser.rules`:

```javascript
polkit.addRule(function(action, subject) {
    if (action.id == "org.freedesktop.login1.inhibit" &&
        subject.user == "your_username") {
        return polkit.Result.YES;
    }
});
```

Reload polkit:

```bash
sudo systemctl restart polkit
```

### Logging Issues

**Enable debug logging in config.toml**:

```toml
update_interval = "5s"
log_level = "debug"   # Set at root level
...
```

Then restart:

```bash
systemctl --user restart rouser
journalctl --user -u rouser -f
```

## Auto-Start on Login

The user service automatically starts when you log in if enabled. To verify:

```bash
# Check autostart status
systemctl --user is-enabled rouser

# Check what pulls it in
systemctl --user list-dependencies default.target | grep rouser
```

If needed, create a desktop autostart entry:

Create `~/.config/autostart/rouser.desktop`:

```ini
[Desktop Entry]
Type=Application
Name=Rouser
Exec=/usr/bin/systemctl --user start rouser
Hidden=false
NoDisplay=false
X-GNOME-Autostart-enabled=true
```

## Uninstallation

### Remove User Service

```bash
# Stop and disable service
systemctl --user stop rouser
systemctl --user disable rouser

# Remove service file
rm ~/.config/systemd/user/rouser.service

# Reload daemon
systemctl --user daemon-reload

# Clean up configuration
rm -rf ~/.config/rouser ~/.local/log/rouser
```

### Remove System Service

```bash
sudo systemctl stop rouser
sudo systemctl disable rouser
sudo rm /etc/systemd/system/rouser.service
sudo systemctl daemon-reload
sudo rm /etc/rouser/config.toml
sudo rm -rf /etc/rouser /var/log/rouser
```

## See Also

- [Installation](installation.md) - Getting started with rouser
- [Configuration Reference](configuration.md) - Complete configuration options
- [Command Line Arguments](command-line.md) - CLI usage details
