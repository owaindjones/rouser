# Quick Start Guide

This guide will help you get `rouser` running on your Linux system within 5 minutes.

## Prerequisites

- Linux system with systemd (systemd user instance)
- Your user logged in and active session (`loginctl enable-linger $USER` recommended for persistence after logout)
- Rust 1.70+ (for building from source only — pre-built binaries available on releases)
- D-Bus session bus available

## Installation Options

### Option 1: Build from Source

```bash
# Clone repository
git clone https://github.com/yourusername/rouser.git
cd rouser

# Build release binary and install to ~/.local/bin
cargo build --release
mkdir -p ~/.local/bin
cp target/release/rouser ~/.local/bin/rouser
chmod +x ~/.local/bin/rouser
```

### Option 2: Via Installer Script (Recommended)

The installer script fetches the latest release, installs the binary, config, and enables systemd user service automatically:

```bash
curl -fsSL https://raw.githubusercontent.com/yourusername/rouser/main/scripts/install.sh | bash
```

See `scripts/install.sh --help` for options.

### Option 3: Manual Download from Release

Download pre-built archives from GitHub Releases matching your architecture:

```bash
# Update URL with actual release version and arch (x86_64 or aarch64)
curl -LO https://github.com/yourusername/rouser/releases/download/v0.1.0/rouser-v0.1.0-linux-x86_64.tar.gz

# Extract — contains binary + config + systemd service file
tar -xzf rouser-v*.linux-*.tar.gz

# Install components manually:
cp rouser ~/.local/bin/rouser
mkdir -p ~/.config/rouser
cp config/rouser.toml ~/.config/rouser/config.toml
systemctl --user daemon-reload  # if installing service file
```

## Configuration

### Config File Discovery

When no `-c` flag is given, rouser searches **sequentially** for the first existing config:

| Priority | Path | Description |
|----------|------|-------------|
| 1 | `./config/rouser.toml` | Repo-packaged default in current directory |
| 2 | `~/.config/rouser/config.toml` | XDG user config (installed by installer script) |
| 3 | `/etc/rouser/config.toml` | System-wide config (requires root to create) |

### Create Configuration File

Copy the repo default into your XDG path:

```bash
mkdir -p ~/.config/rouser
cp ./config/rouser.toml ~/.config/rouser/config.toml
```

**Example configuration**:

```toml
name = "rouser"
update_interval = "5s"
log_level = "info"

[metrics.cpu]
threshold = 80.0       # CPU usage % above which to inhibit sleep
ema_alpha = 0.3        # EMA smoothing: higher = more responsive

[metrics.gpu]
threshold = 90.0       # GPU usage % per device
ema_alpha = 0.3

[metrics.network]
threshold = 100.0      # Network throughput in Mbps
ema_alpha = 0.2        # EMA smoothing for network I/O
exclude_interfaces = ["lo"]    # Exclude loopback from monitoring
include_interfaces = []        # Empty = monitor all interfaces

[metrics.disk]
threshold = 50.0       # Disk I/O in MB/s above which to inhibit sleep
ema_alpha = 0.2        # EMA smoothing for disk activity
exclude_device_prefixes = ["loop", "fd", "sr", "cdrom"]  # Exclude virtual devices

[timing]
duration_threshold = "30s"   # Min time metrics must exceed threshold before inhibiting
cooldown_duration = "60s"    # Time after releasing inhibition before re-inhibiting possible

[inhibitor]
what = "shutdown:idle"       # Lock types to inhibit (colon-separated)
mode = "block"               # Inhibition mode: block, delay, or block-weak
```

See [Configuration Reference](configuration.md) for full option descriptions.

## Testing Configuration

### Validate Configuration

```bash
# Uses sequential default config search
rouser --validate-config

# With explicit path
rouser -c ~/.config/rouser/config.toml --validate-config
```

### Dry Run Mode

Collect metrics and log readings without inhibiting sleep:

```bash
# Runs indefinitely until Ctrl+C
rouser --dry-run

# With debug logging to see per-device GPU readings
RUST_LOG=debug rouser -c ~/.config/rouser/config.toml --dry-run -l debug
```

Sample output in dry-run mode:
```
CPU threshold: 80%, EMA alpha: 0.30
GPU threshold: 90%, EMA alpha: 0.30
Network threshold: 100 Mbps, EMA alpha: 0.20
Disk threshold: 50 MB/s, EMA alpha: 0.20
Duration threshold: 30s
Cooldown duration: 60s
```

## Running the Daemon

### Manual Execution

```bash
# Run with default config search path
rouser

# Custom config path
rouser -c /path/to/config.toml

# Override log level at runtime
rouser -l debug --dry-run
```

The daemon runs indefinitely — press `Ctrl+C` to stop. When running as a systemd user service, use the service management commands below instead.

### Systemd User Service (Recommended)

After testing with dry-run:

```bash
# Install and enable the user service (if not done by installer script)
systemctl --user daemon-reload
systemctl --user enable --now rouser.service

# Check status
systemctl --user status rouser

# View live logs
journalctl --user -u rouser -f
```

### Service Management

```bash
systemctl --user start rouser    # Start service
systemctl --user stop rouser     # Stop service
systemctl --user restart rouser  # Restart (after config change)
systemctl --user disable rouser  # Disable auto-start on login
```

## Verifying Inhibition

Check active sleep inhibitors:

```bash
# List all current inhibitors
loginctl list-inhibitors

# Or use systemd-inhibit
systemd-inhibit --list
```

When rouser is inhibiting, you should see an entry with description "rouser".

### Quick Test

1. Start rouser in dry-run mode and verify metrics are collected:
   ```bash
   RUST_LOG=debug rouser --dry-run -l debug 2>&1 | head -30
   ```

2. If inhibition works, you can temporarily lower the CPU threshold to `1` to test:
   ```toml
   # In config.toml — set thresholds very low just for testing
   [metrics.cpu]
   threshold = 1.0    # Will trigger on any significant activity
   ema_alpha = 0.3

   [timing]
   duration_threshold = "5s"   # Short test window
   cooldown_duration = "10s"
   ```
   
   Then run `rouser` and check `loginctl list-inhibitors`. Restore thresholds after testing.

## Troubleshooting

### Service Won't Start After Logout

Ensure logind lingering is enabled for your user:

```bash
loginctl enable-linger $USER
# Verify: loginctl show-user $USER | grep Linger
```

Without lingering, systemd only starts the service during active login sessions.

### D-Bus Permission Errors

If rouser cannot acquire sleep inhibition:

1. **KDE Plasma**: Add a polkit rule (see [systemd user service docs](systemd-user-service.md))
2. **Check D-Bus session**: `echo $DBUS_SESSION_BUS_ADDRESS` should be non-empty
3. **Manual test**: `systemd-inhibit --what=sleep --mode=block --description="Test" sh -c "sleep 5"` — if this works but rouser doesn't, it's a config or threshold issue

### Inhibition Not Working on KDE Plasma

KDE Powerdevil may ignore inhibitors from unprivileged users. Create `/etc/polkit-1/rules.d/50-rouser.rules`:

```javascript
polkit.addRule(function(action, subject) {
    if (action.id == "org.freedesktop.login1.inhibit" &&
        subject.user == "your_username") {
        return polkit.Result.YES;
    }
});
```

Then `sudo systemctl restart polkit`.

### Debug Logging

Enable verbose logging to see per-device metric readings:

```toml
# In config.toml — set log_level at root level
name = "rouser"
update_interval = "5s"
log_level = "debug"     # NOT under [daemon] — rouser uses flat structure now
```

Or override via CLI (takes priority over config):
```bash
rouser --dry-run -l debug
```

## Example Configurations

### Home Server (low activity baseline)

```toml
name = "rouser"
update_interval = "5s"
log_level = "info"

[metrics.cpu]
threshold = 70.0
ema_alpha = 0.3

[metrics.gpu]
threshold = 85.0
ema_alpha = 0.3

[metrics.network]
threshold = 50.0
ema_alpha = 0.2
exclude_interfaces = ["lo"]

[metrics.disk]
threshold = 30.0
ema_alpha = 0.2
exclude_device_prefixes = ["loop", "fd", "sr", "cdrom"]

[timing]
duration_threshold = "60s"
cooldown_duration = "120s"

[inhibitor]
what = "sleep:idle"
mode = "block"
```

### Development Workstation (high activity tolerance)

```toml
[metrics.cpu]
threshold = 90.0       # Only inhibit during heavy compilation/builds
ema_alpha = 0.3

[metrics.gpu]
threshold = 95.0       # Gaming or GPU workloads
ema_alpha = 0.3

[metrics.network]
threshold = 200.0      # Large downloads/uploads
ema_alpha = 0.2
exclude_interfaces = ["lo"]

[metrics.disk]
threshold = 100.0      # Heavy I/O builds or VM operations
ema_alpha = 0.2
exclude_device_prefixes = ["loop", "fd", "sr", "cdrom"]

[timing]
duration_threshold = "30s"   # Shorter threshold for responsive inhibition
cooldown_duration = "60s"

[inhibitor]
what = "shutdown:idle:sleep:suspend"  # Block all sleep types
mode = "block"
```

## Next Steps

1. **Adjust thresholds** based on your system's typical activity patterns (use `RUST_LOG=debug` dry-run to baseline)
2. **Review [Configuration Reference](configuration.md)** for all available options and EMA smoothing details
3. **Read the full docs/** directory for metrics collection methods, inhibition flow, and security hardening

## See Also

- [Configuration Reference](configuration.md) — Complete configuration options with defaults
- [Command Line Arguments](command-line.md) — CLI usage and environment variables
- [Systemd User Service](systemd-user-service.md) — Detailed service setup and troubleshooting
- [Metrics Overview](metrics-overview.md) — How each metric type is collected
