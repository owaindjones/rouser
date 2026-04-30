# Introduction to rouser

## What is rouser?

`rouser` is a Linux daemon that monitors system metrics and automatically inhibits sleep when activity thresholds are exceeded. It enables systems to:

- Sleep automatically after idle time when no activity is detected
- Stay awake during high CPU/GPU usage (compiling, rendering, gaming)
- Remain active during network transfers or disk I/O

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

## CI/CD Coverage

rouser is built and packaged via GitHub Actions for multiple architectures on every release:

| Target | Build Status | Package Formats Available |
|--------|--------------|--------------------------|
| x86_64 Linux | Built & tested | Tarball + DEB (amd64) + RPM (x86_64) |
| aarch64 Linux | Cross-compiled & tested | Tarball + DEB (arm64) + RPM (aarch64) |
| Arch / Bazzite | Built on release | PKGBUILD archive |

> **Pre-release**: rouser is at `v0.0.0` (unreleased). No official releases yet. See [AGENTS.md](./../AGENTS.md) for versioning policy.

## System Requirements

### Minimum Requirements

| Resource | Value |
|----------|-------|
| OS | Linux with systemd (v219+) |
| CPU Architecture | x86_64 or aarch64 (pre-built binaries available via CI pipeline) |
| Memory | 64 MB free |
| Disk | 10 MB free |

### Recommended Requirements

| Resource | Value |
|----------|-------|
| OS | Latest stable systemd |
| CPU Architecture | x86_64 (native) or aarch64 (cross-compiled, same binaries work on both) |
| CPU | Dual-core or better |
| Memory | 128 MB free |
| Disk | 50 MB free |

### GPU Requirements (Optional — for GPU Monitoring)

GPU monitoring is optional; rouser operates fully without any GPU. When GPUs are present, the following drivers and data sources are supported:

| Vendor | Driver / Data Source | Notes |
|--------|---------------------|-------|
| NVIDIA | Proprietary driver with NVML library (`libnvidia-ml.so`) | Per-GPU utilization via NVML API (same approach as nvtop) |
| AMD    | Open-source `amdgpu` kernel module | Sysfs path `/sys/class/drm/cardN/device/gpu_busy_percent` |
| Intel  | `i915` or `xe` kernel module | Same sysfs path as AMD; driver detected via `device/driver` symlink target |

NVIDIA GPU monitoring uses the NVML library (`libnvidia-ml.so`) loaded dynamically at runtime — no separate binary required. The NVML library is included with NVIDIA proprietary drivers and is also used by tools like `nvidia-smi` and nvtop. AMD and Intel GPUs require no external binaries beyond standard kernel interfaces.

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

rouser is designed to run as a **systemd user service** under the unprivileged invoking user's account — no root access required for normal operation. The recommended installation path (via installer script or manual setup) places the binary and config in `~/.local/bin/` and `~/.config/rouser/config.toml`, respectively.

When running as a user service, rouser only needs:
- Read access to `/proc` filesystem entries (`/proc/stat`, `/proc/net/dev`, `/proc/diskstats`) — available to any process on the system
- D-Bus session bus access via `org.freedesktop.login1.Manager.Inhibit` for sleep inhibition (may require a polkit rule on some desktop environments like KDE Plasma)
- Optional: read access to sysfs paths (`/sys/class/drm/cardN/device/gpu_busy_percent`) for AMD/Intel GPU monitoring — typically world-readable

A system-wide installation under root is possible but not recommended; see [Running as a Service](systemd-user-service.md#alternative-systemd-system-service) for details.

### File Permissions (User-Mode, Recommended)

| Path | Permission | Notes |
|------|-----------|-------|
| Binary (`~/.local/bin/rouser`) | `0755` | User-owned executable |
| Config (`~/.config/rouser/config.toml`) | `0644` (or stricter) | Owned by user; no sensitive data is stored in config |
| Log directory (`~/.local/log/rouser`) | `0755` | Writable by daemon user only |

### File Permissions (System-Mode, Optional — Requires Root)

When installing system-wide under root:

| Path | Permission | Notes |
|------|-----------|-------|
| Binary (`/usr/local/bin/rouser`) | `0755` root-owned executable | Installed via DEB/RPM or manual copy as root |
| Config (`/etc/rouser/config.toml`) | `0644` root-owned, group-readable | No secrets in config; readable by all users is acceptable |
| Service file (`/lib/systemd/system/rouser.service`) | `0644` systemd-managed | Standard systemd service permissions |

### Desktop Environment Considerations

- **KDE Plasma**: Powerdevil may ignore D-Bus inhibitors from unprivileged users. A polkit rule at `/etc/polkit-1/rules.d/50-rouser.rules` is recommended (see [systemd-user-service.md](systemd-user-service.md#kde-plasma-issues)).

## Release Artifacts

On every tagged release, GitHub Actions publishes build artifacts matching your architecture (x86_64 or aarch64):

| Artifact | Format | Description |
|----------|--------|-------------|
| Tarball archive | `.tar.gz` | Contains binary + default config (`config/rouser.toml`) + systemd service file. Extract and install components manually, or use the installer script to handle it automatically. Available for x86_64 and aarch64. |
| DEB package | `.deb` | Debian/Ubuntu native package built in GitHub Actions. Installs binary to `/usr/local/bin/`, config to `/etc/rouser/config.toml`, service file to `/lib/systemd/system/`. Available for amd64 and arm64. |
| RPM package | `.rpm` | Red Hat/Fedora/CentOS native package built in a containerized `fedora:latest` image. Installs binary, config, and systemd service file at standard paths. Available for x86_64 and aarch64. |
| Arch PKGBUILD | `.tar.gz` (source archive) | Archive containing `PKGBUILD` with source URLs pointing to all release tarballs. Download from the releases page and build locally via `makepkg`. Suitable for Arch Linux, Bazzite, and other Arch-based distros. |

### Installer Script

The provided installer script (`scripts/install.sh`) automates manual installation: it downloads the latest release tarball matching your architecture, extracts the binary to `~/.local/bin/`, installs default config to `~/.config/rouser/config.toml`, and enables the systemd user service. Run `curl -fsSL <url>/install.sh | bash -- --help` to see available options before installing.

### Installation Methods Summary

| Method | Best For | Root Required? |
|--------|----------|----------------|
| Cargo build (`cargo build`) | Developers, custom targets | No (for user install) |
| Installer script (`install.sh`) | Quick setup on any distro | No (user-mode only) |
| DEB package (.deb) | Debian/Ubuntu servers and desktops | Yes (system-wide install via `dpkg -i`) |
| RPM package (.rpm) | Fedora/RHEL/CentOS/AlmaLinux | Yes (system-wide install via `dnf` or `rpm -i`) |
| Arch PKGBUILD | Arch Linux, Bazzite, and derivatives | No (user-mode build) |

## License

This project is licensed under the MIT License. See the LICENSE file for details.

## Getting Started

1. [Quick Start Guide](quickstart.md) — Get rouser running on your system within 5 minutes
2. [Configuration Reference](configuration.md) — Complete configuration options with defaults and examples
3. [Command Line Arguments](command-line.md) — CLI usage, flags, and environment variables
4. [Running as a Service](systemd-user-service.md) — Systemd user service setup, hardening, and troubleshooting
5. [Metrics Overview](metrics-overview.md) — How each metric type is collected (CPU, GPU, network, disk)
6. [Developer Guide](developer-guide.md) — Build process, code structure, and contribution guidelines

## See Also

- [CI Workflow](ci-workflow.md) — GitHub Actions pipeline: lint gates, cross-compilation, DEB/RPM/PKGBUILD packaging, release artifacts
- [AGENTS.md](./../AGENTS.md) — Developer conventions: versioning policy, commit format, error handling, async patterns
