# rouser - Linux Daemon for Sleep Inhibition

## Purpose

A Linux daemon called `rouser` which will be run as a systemd service. Its purpose will be to continuously monitor operating system metrics, and inhibit sleep (using D-Bus APIs) whenever something remains above a threshold for a configurable amount of time.

## Metrics to Monitor

- Aggregate CPU usage (%)
- Aggregate GPU usage (%)
- Network I/O (Kbps/Mbps)
- Disk activity (KB/s / MB/s)

## Behavior

If any of these things go above a threshold, the daemon should inhibit sleep until it considers the system to be 'idle' again. When it considers the system to be idle (when no metrics are above threshold for a configurable amount of time), then it releases the D-Bus sleep inhibiting lock.

## Use Case

The purpose of this is to allow a headless server to go into standby when it's not being actively used, but to keep awake when it is busy. Because it is not a standard desktop, things like mouse movement or applications explicitly inhibiting sleep do not happen. But it's configured to sleep after 15 minutes of idle-time, and is woken again by WOL packets. `rouser` will provide the mechanism to keep it awake when it's busy, such that I don't have to keep it awake manually (I've been doing that in a hacky way with `systemd-inhibit --what=sleep --mode=delay cat` in a `tmux` session which I ctrl+c when I'm done; I really want to automate away the problem instead!)

## Configuration

**Note**: `rouser` uses TOML configuration format (not YAML) for security and simplicity:

- Pure Rust implementation with no C dependencies
- Native support via the `toml` crate
- Simpler, more readable format
- Well-maintained in the Rust ecosystem

Configuration file location: `/etc/rouser/config.toml`

## Security Considerations

For production deployments, refer to [SECURITY.md](docs/security.md) for important security best practices regarding:

- Configuration file permissions and ownership
- D-Bus permissions
- Systemd service account and privileges
- Dependency vulnerability management

## Technical Stack

- **Language**: Rust (stable)
- **D-Bus Library**: `zbus` v4
- **Configuration**: `toml` crate
- **Logging**: `tracing` + `tracing-subscriber`
- **Error Handling**: `anyhow` + `thiserror`
- **Service Type**: systemd (system service)
