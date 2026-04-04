# Introduction to rouser

## What is rouser?

`rouser` is a Linux daemon that monitors system metrics and automatically inhibits sleep when activity thresholds are exceeded. It enables systems to:

- Sleep automatically after idle time when no activity is detected
- Stay awake during high CPU/GPU usage (compiling, rendering, gaming)
- Remain active during network transfers or disk I/O
- Be woken by Wake-on-LAN (WOL) packets while remaining asleep otherwise

## Purpose and Use Cases

### Headless Servers

Keep your headless server awake during:
- Large file downloads or uploads
- Database operations with high disk I/O
- Network backup jobs
- Compilation or build processes

### Desktop Systems

Prevent unwanted sleep during:
- Video rendering or 3D work
- Gaming sessions
- Long-running downloads
- Local network activity

### Development Workstations

Maintain system responsiveness during:
- Code compilation
- Container builds
- Docker operations
- Virtual machine execution

## How It Works

rouser operates on a simple principle:

1. **Monitor**: Collects CPU, GPU, network, and disk metrics at regular intervals (default: 5 seconds)
2. **Evaluate**: Compares metrics against configurable thresholds
3. **Decide**: Determines if activity warrants keeping the system awake
4. **Inhibit**: Uses systemd's login1 D-Bus API to prevent sleep when thresholds are exceeded

### Key Design Decisions

#### Why systemd login1?

rouser uses the `org.freedesktop.login1.Manager.Inhibit` D-Bus API because:

- Actively maintained and well-documented
- Works reliably across desktop environments
- Doesn't require session D-Bus (works without a graphical session)
- Standard interface for sleep inhibition on modern Linux systems

#### Why Rust?

- Memory safety without garbage collection
- Zero-cost abstractions for efficient metric collection
- Native async runtime support via Tokio
- Strong type system prevents configuration errors
- No external C dependencies

#### Why TOML Configuration?

TOML was chosen over YAML because:

- Pure Rust implementation with no C dependencies
- Simpler, more readable syntax
- Better security (avoids RUSTSEC-2025-0068 vulnerability in YAML parsers)
- Native support via the `toml` crate

## System Requirements

### Minimum Requirements

| Resource | Value |
|----------|-------|
| OS | Linux with systemd (v219+) |
| CPU | Any x86_64 or ARM64 |
| Memory | 64 MB free |
| Disk | 10 MB free |

### Recommended Requirements

| Resource | Value |
|----------|-------|
| OS | Latest stable systemd |
| CPU | Dual-core or better |
| Memory | 128 MB free |
| Disk | 50 MB free |

## Performance Characteristics

### Resource Usage (Typical)

| Metric | Value | Notes |
|--------|-------|-------|
| Memory | ~2-5 MB | Depends on interfaces/devices |
| CPU | <0.1% | With 5-second polling interval |
| Disk I/O | Negligible | Reads from /proc filesystem |
| Power Impact | ~1-2 mW | Additional daemon overhead |

### Latency

- **Response Time**: From threshold exceedance to sleep inhibition (~5-10 seconds)
- **Configurable**: Adjust polling interval and thresholds for faster response

## Architecture Overview

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

### Component Descriptions

- **Config Loader**: Parses TOML configuration and applies defaults
- **Core Logic**: Main event loop coordinating all components
- **Metrics Collectors**: Modular collectors for CPU, GPU, network, disk
- **Threshold Manager**: Tracks metrics over time and determines inhibition state
- **Inhibitor**: Interfaces with D-Bus to acquire/release sleep inhibition locks

## Security Considerations

### Principle of Least Privilege

rouser runs as root (or dedicated user) with minimal required capabilities:

- Reads from `/proc` filesystem (virtual, no disk I/O)
- D-Bus communication via login1 interface
- No network access required after startup

### File Permissions

- Configuration file: Mode `0600`, owned by root
- Log directory: Mode `0755`, writable by daemon user
- Service file: Standard systemd permissions

## License

This project is licensed under the MIT License. See the LICENSE file for details.

## Getting Started

1. [Installation](quickstart.md) - Get rouser running on your system
2. [Configuration](configuration.md) - Learn about configuration options
3. [Running as Service](systemd-user-service.md) - Set up automatic startup
4. [Developer Guide](developer-guide.md) - Contribute to the project
