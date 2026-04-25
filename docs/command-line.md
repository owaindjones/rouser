# Command Line Arguments

This document describes all command-line arguments and options for `rouser`.

## Usage

```bash
rouser [OPTIONS]
```

## Options

### Configuration

| Option | Description | Default Search Order |
|--------|-------------|---------------------|
| `-c, --config <CONFIG>` | Path to configuration file (overrides all defaults) | — |

When no `--config` flag is provided, rouser searches sequentially for the first existing config file:

1. `./config/rouser.toml` — repo-packaged default
2. `$HOME/.config/rouser/config.toml` — XDG user config
3. `/etc/rouser/config.toml` — system-wide config

If none of these paths exist, rouser uses built-in defaults and logs a warning:

```
No configuration file found at checked paths — using built-in defaults. Checked: ./config/rouser.toml, ~/.config/rouser/config.toml, /etc/rouser/config.toml
```

**Examples**:

```bash
# Use custom config path (takes priority over all defaults)
rouser --config /path/to/custom-config.toml
rouser -c /etc/my-custom-rouser.toml

# Let rouser search default paths sequentially
rouser
```

### Validation

| Option | Description |
|--------|-------------|
| `--validate-config` | Validate configuration and exit without running the daemon |

Validates that the config file exists, is valid TOML, and all fields match the expected schema. Does not start metric collection or inhibition.

**Example**:

```bash
# Validate using default path search
rouser --validate-config

# Validate explicit config
rouser -c /etc/rouser/config.toml --validate-config
```

Expected output on success: `Configuration validation passed`
Expected output on failure: `Configuration validation failed: <error details>`

### Runtime Mode

| Option | Description |
|--------|-------------|
| `--dry-run` | Test mode: collect metrics and log readings but never inhibit sleep. Runs indefinitely until interrupted (Ctrl+C). |

**Example**:

```bash
# Dry run with default config search — runs forever until Ctrl+C
rouser --dry-run

# Dry run with explicit config, debug logging to see per-device GPU readings
RUST_LOG=debug rouser -c /etc/rouser/config.toml --dry-run
```

### Logging Level Override

| Option | Description | Precedence |
|--------|-------------|------------|
| `-l, --log-level <LEVEL>` | Set log level at runtime, overriding config.log_level and RUST_LOG env var. Values: `debug`, `info`, `warn`, `error`. | Highest — overrides everything |

Log level precedence (highest to lowest):
1. CLI flag `-l/--log-level`
2. Environment variable `RUST_LOG`
3. Config file field `log_level` in `[rouser]` section
4. Hardcoded default `"info"`

**Examples**:

```bash
# Override log level via CLI (takes priority over RUST_LOG and config)
rouser --dry-run -l debug

# Set via environment variable (lower precedence than -l flag)
RUST_LOG=debug rouser --dry-run

# Both can be combined — CLI flag wins if both are set
RUST_LOG=error rouser --dry-run -l debug   # Results in debug level
```

### Help and Version

| Option | Description |
|--------|-------------|
| `-h, --help` | Print help information (all available options) |
| `-V, --version` | Print version information (from Cargo.toml package version) |

**Examples**:

```bash
rouser --version
# Output: rouser 0.1.0

rouser --help
```

## Complete Examples

### Basic Usage

```bash
# Run with default config search path
rouser

# Dry run to test configuration without inhibiting sleep
rouser --dry-run

# Validate config and exit (useful in CI or deployment scripts)
rouser --validate-config
```

### Custom Configuration Paths

```bash
# Use repo-packaged default if present in current directory
cp config/rouser.toml /tmp/test-dir/ && cd /tmp/test-dir && rouser

# User-level XDG config (if it exists at ~/.config/rouser/config.toml)
rouser

# System-wide override (must be created manually)
sudo tee /etc/rouser/config.toml > /dev/null <<EOF
name = "rouser"
update_interval = "10s"
log_level = "warn"
...
EOF
rouser  # will find and use /etc/rouser/config.toml

# Explicit path (always takes priority)
rouser -c /opt/custom/rouser-config.toml
```

### Debugging with Logging

```bash
# Enable debug logging to see per-device metric readings
RUST_LOG=debug rouser --dry-run

# Override log level at runtime via CLI flag
rouser -l debug --dry-run

# Crate-specific logging (rouser debug, zbus info)
RUST_LOG=rouser=debug,zbus=info rouser --dry-run
```

## Exit Codes

| Code | Description |
|------|-------------|
| `0` | Success — daemon ran until interrupted or dry run completed normally |
| `1` | Failure — invalid config, missing dependencies (e.g., no D-Bus), or other error |

## Environment Variables

In addition to configuration file options, rouser respects these environment variables:

| Variable | Description | Affects |
|----------|-------------|---------|
| `RUST_LOG` | Logging level filter (see [tracing-subscriber](https://docs.rs/tracing-subscriber/) for format) | Console logging output only |

There are no `ROUSER_*` environment variable overrides for configuration values. All settings must come from the TOML file or be overridden at runtime via CLI flags (`-l/--log-level`).

### RUST_LOG Format Examples

```bash
# Simple log level
RUST_LOG=debug rouser --dry-run

# Crate-specific levels
RUST_LOG=rouser=debug,zbus=info rouser --dry-run

# Module-level filtering (if applicable)
RUST_LOG=rouser::metrics=debug,rouser::service=warn rouser --dry-run
```

## Logging Output

### Console Logging

By default, rouser logs to stdout. Log format includes level, timestamp, and target module:

```bash
rouser -l debug --dry-run
# Sample output:
# 2026-04-24T10:00:00.123Z INFO  rouser::service [service.rs:45] Tick 1: CPU=45.2%, card0(nvidia)=92.1%, net=12.3Mbps, disk=0.5MB/s
# 2026-04-24T10:00:00.124Z INFO  rouser::inhibit [inhibit.rs:78] Sleep inhibited: GPU at 92% (threshold: 90%)
```

### Journalctl (systemd user service)

When installed via the installer script and running as a systemd user service:

```bash
journalctl --user -u rouser -f
```

## Error Handling

### Invalid Configuration File Format

If the config file has invalid TOML syntax:

```bash
rouser --validate-config /path/to/bad.toml
# Output: Configuration validation failed: Failed to parse TOML configuration: expected `=`, found end-of-file at line 1 column 10
```

### Missing Configuration File (non-fatal)

When no config file is found in any default path, rouser uses built-in defaults with a warning:

```bash
rouser --dry-run -l debug
# Output includes: No configuration file found at checked paths — using built-in defaults. Checked: ./config/rouser.toml, ~/.config/rouser/config.toml, /etc/rouser/config.toml
```

### Missing GPU Libraries (non-fatal)

When NVML (`libnvidia-ml.so`) is not available but NVIDIA hardware exists in sysfs, rouser logs a warning and continues with other metrics. No error exit occurs — the daemon degrades gracefully.

## Argument Precedence for Config Resolution

Config path resolution follows this order (highest to lowest):

1. **CLI flag** `--config` / `-c`
2. **Sequential default search**: `./config/rouser.toml` → `~/.config/rouser/config.toml` → `/etc/rouser/config.toml` (first existing file wins)
3. **Built-in defaults** (when none of the above exist)

## See Also

- [Configuration Reference](configuration.md) — Complete configuration options and format
- [Quick Start Guide](quickstart.md) — Getting started with rouser
- [Systemd User Service](systemd-user-service.md) — Running as a service
