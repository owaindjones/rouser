# Command Line Arguments

This document describes all command-line arguments and options for `rouser`.

## Usage

```bash
rouser [OPTIONS]
```

## Options

### Configuration

| Option | Description | Default |
|--------|-------------|---------|
| `-c, --config <CONFIG>` | Path to configuration file | `/etc/rouser/config.toml` |

**Example**:

```bash
rouser --config /path/to/custom-config.toml
```

### Validation

| Option | Description |
|--------|-------------|
| `--validate-config` | Validate configuration and exit without running |

**Example**:

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

### Testing

| Option | Description |
|--------|-------------|
| `--dry-run` | Test mode: collect metrics but don't inhibit sleep |
| `--duration <SECONDS>` | Run for specified duration (requires --dry-run) |

**Example**:

```bash
# Dry run for 60 seconds
rouser --config /etc/rouser/config.toml --dry-run --duration 60s

# Dry run with custom config
rouser --config /path/to/config.toml --dry-run --duration 120s
```

### Help and Version

| Option | Description |
|--------|-------------|
| `-h, --help` | Print help information |
| `-V, --version` | Print version information |

**Example**:

```bash
rouser --version
# Output: rouser 0.1.0

rouser --help
```

## Complete Examples

### Basic Usage

```bash
# Run with default configuration
rouser

# Run with custom configuration
rouser --config /etc/rouser/config.toml
```

### Validation

```bash
# Validate configuration before deploying
rouser --validate-config /etc/rouser/config.toml
```

### Dry Run Testing

```bash
# Test for 5 minutes (300 seconds)
rouser --config /etc/rouser/config.toml --dry-run --duration 300s

# Test with debug logging
ROUSER_LOG_LEVEL=debug rouser --config /etc/rouser/config.toml \
  --dry-run --duration 300s
```

### Running as Root (for systemd)

```bash
# Manual run as root
sudo rouser --config /etc/rouser/config.toml

# Or set up as systemd service (recommended)
# See systemd-user-service.md for details
```

## Exit Codes

| Code | Description |
|------|-------------|
| `0` | Success |
| `1` | General error (invalid config, missing file, etc.) |
| `2` | Validation failed |

**Example**:

```bash
rouser --validate-config /etc/rouser/config.toml
echo $?  # 0 if valid, 2 if invalid
```

## Environment Variables

In addition to configuration file options, rouser respects these environment variables:

| Variable | Description |
|----------|-------------|
| `RUST_LOG` | Logging level filter (e.g., `debug`, `info`, `warn`) |
| `ROUSER_*` | Configuration overrides (see Configuration Reference) |

### RUST_LOG

Set the logging level via environment variable:

```bash
# Debug logging
RUST_LOG=debug rouser

# Info level (default)
RUST_LOG=info rouser

# Warning level
RUST_LOG=warn rouser

# Error level only
RUST_LOG=error rouser
```

You can also specify crate-specific logging:

```bash
# Enable debug for rouser only
RUST_LOG=rouser=debug rouser

# Enable debug for rouser and zbus (D-Bus client)
RUST_LOG=rouser=debug,zbus=info rouser
```

### Configuration Overrides

Configuration values can be overridden via environment variables:

```bash
# Override CPU threshold to 75%
ROUSER_THRESHOLDS_CPU_USAGE=75 rouser

# Set multiple overrides
ROUSER_THRESHOLDS_CPU_USAGE=75 \
  ROUSER_DAEMON_LOG_LEVEL=debug \
  rouser
```

| Environment Variable | Config Path |
|---------------------|-------------|
| `ROUSER_DAEMON_NAME` | `daemon.name` |
| `ROUSER_DAEMON_UPDATE_INTERVAL` | `daemon.update_interval` |
| `ROUSER_DAEMON_LOG_LEVEL` | `daemon.log_level` |
| `ROUSER_THRESHOLDS_CPU_USAGE` | `thresholds.cpu_usage` |
| `ROUSER_THRESHOLDS_GPU_USAGE` | `thresholds.gpu_usage` |
| `ROUSER_THRESHOLDS_NETWORK_IO` | `thresholds.network_io` |
| `ROUSER_THRESHOLDS_DISK_ACTIVITY` | `thresholds.disk_activity` |
| `ROUSER_TIMING_DURATION_THRESHOLD` | `timing.duration_threshold` |
| `ROUSER_TIMING_IDLE_DURATION` | `timing.idle_duration` |
| `ROUSER_TIMING_COOLDOWN_DURATION` | `timing.cooldown_duration` |

## Argument Precedence

When multiple sources specify the same configuration value, the following precedence applies (highest to lowest):

1. **Environment variables** (`ROUSER_*`)
2. **Command-line arguments** (`--config`, `--dry-run`)
3. **Configuration file** (`/etc/rouser/config.toml`)
4. **Hardcoded defaults**

**Example**:

```toml
# /etc/rouser/config.toml
[thresholds]
cpu_usage = 80.0
```

```bash
ROUSER_THRESHOLDS_CPU_USAGE=75 rouser
# Result: CPU threshold will be 75% (env var takes precedence)
```

## Interactive Mode

rouser is designed as a daemon and does not support interactive mode. All operations run in the background with logging to stdout/stderr or a configured log file.

## Logging Output

### Console Logging

By default, rouser logs to stdout:

```bash
rouser --config /etc/rouser/config.toml
# Logs appear in terminal
```

### File Logging

Configure log file in `config.toml`:

```toml
[logging]
file = "/var/log/rouser/rouser.log"
format = "text"  # or "json"
```

### JSON Format

For log aggregation systems:

```toml
[logging]
file = "/var/log/rouser/rouser.log"
format = "json"
```

Example output:

```json
{"level":"info","time":"2026-03-26T10:00:00Z","message":"Sleep inhibited: CPU at 85% (threshold: 80%)"}
{"level":"warn","time":"2026-03-26T10:00:05Z","message":"GPU metrics unavailable, using 0%"}
```

## Error Handling

### Invalid Configuration

If the configuration file is invalid:

```bash
rouser --config /etc/rouser/config.toml
# Output: Error: Failed to parse config: missing field `thresholds.cpu_usage` at line 5
```

### Missing Configuration File

```bash
rouser --config /nonexistent.toml
# Output: Error: Config file not found: /nonexistent.toml
```

### Permission Errors

```bash
rouser --config /etc/rouser/config.toml
# If config is not readable:
# Output: Error: Permission denied: /etc/rouser/config.toml
```

## Debugging

### Enable Verbose Logging

```bash
# Using environment variable
RUST_LOG=debug rouser --config /etc/rouser/config.toml

# Or in config file
[daemon]
log_level = "debug"
```

### Check Active Inhibitors

While rouser is running, check if inhibition is active:

```bash
loginctl list-inhibitors
```

### Monitor Logs in Real-time

```bash
# Via journalctl (systemd service)
sudo journalctl -u rouser -f

# Or from log file
tail -f /var/log/rouser/rouser.log
```

## See Also

- [Quick Start Guide](quickstart.md) - Getting started with rouser
- [Configuration Reference](configuration.md) - Complete configuration options
- [Systemd User Service](systemd-user-service.md) - Running as a service
