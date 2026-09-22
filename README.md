# rouser

![rouser logo — eye/radar with animated metric bars](docs/rouser-logo.svg)

A Linux daemon that monitors system metrics (CPU, GPU, network, disk) and inhibits sleep when activity thresholds are exceeded.

[![CI](https://github.com/owaindjones/rouser/actions/workflows/ci.yml/badge.svg)](https://github.com/owaindjones/rouser/actions/workflows/ci.yml)
[![Rust](https://img.shields.io/badge/rust-stable-orange?style=flat&logo=rust)](https://www.rust-lang.org)
[![License](https://img.shields.io/badge/license-MIT-blue?style=flat)](LICENSE)

## Overview

rouser keeps headless servers and desktops awake during active use. It monitors CPU, GPU, network I/O, and disk activity using configurable thresholds with hysteresis timing to prevent rapid sleep/inhibit cycling. When metrics stay below thresholds for the configured cooldown duration, rouser releases its D-Bus inhibition lock and lets the system sleep naturally.

**Note: Rouser does not put your machine into sleep or hibernation — it only prevents other things from doing so via the SystemD login1 D-Bus API.**

### Key Features

- **Multi-metric monitoring**: CPU (per-core frequency-weighted), GPU (NVIDIA/AMD/Intel), network I/O, disk activity
- **Configurable thresholds**: Independent per-core and total-CPU thresholds, per-GPU reporting
- **EMA smoothing**: Per-metric exponential moving average for stable readings
- **Predictive cooldown**: Learns from historical usage patterns to extend idle cooldown duration, reducing false-positive sleep inhibition during typical active-use hours
- **Systemd integration**: Uses `org.freedesktop.login1.Manager.Inhibit` D-Bus API
- **TOML configuration**: Embedded default config; auto-installs to user or system paths on first run, merges `/etc/rouser/config.toml` and `~/.config/rouser/config.toml` if present
- **Dry-run mode**: Test without inhibiting sleep

## Install

### Build from source

```bash
git clone https://github.com/owaindjones/rouser.git && cd rouser
cargo build --release
./target/release/rouser --dry-run    # test first
./target/release/rouser              # run as daemon (Ctrl+C to stop)
```

### Install via script (recommended)

The installer fetches the latest release, installs the binary + config + systemd service:

```bash
curl -fsSL https://raw.githubusercontent.com/owaindjones/rouser/main/scripts/install.sh | bash
```

Or download and verify before running:

```bash
curl -fSL -o install.sh https://raw.githubusercontent.com/owaindjones/rouser/main/scripts/install.sh
sha256sum install.sh  # inspect before executing
bash install.sh
```

See [Installation](docs/installation.md) for full installation, configuration, and systemd service setup.

## Configuration

The default config is embedded in the binary from `config/rouser.toml`. Print it with:

```bash
./target/release/rouser --print-config > ~/.config/rouser/config.toml
# or after install script: rouser --print-config > ~/.config/rouser/config.toml
```

See [Configuration Reference](docs/configuration.md) for all options with defaults and descriptions.

## Documentation

| Document | Description |
|----------|-------------|
| [Installation](docs/installation.md) | Step-by-step getting started |
| [Configuration Reference](docs/configuration.md) | All config options with embedded-default values |
| [Command Line](docs/command-line.md) | CLI arguments and usage examples |
| [Metrics Overview](docs/metrics-overview.md) | How CPU, GPU, network, disk metrics are collected |
| [Prediction Model](docs/prediction-model.md) | How adaptive cooldown extension works from historical patterns |
| [GPU Usage Measurement](docs/gpu-usage-measurement.md) | What NVML, amdgpu, and i915 actually measure |
| [D-Bus Inhibition](docs/d-bus-inhibition.md) | How sleep inhibition works under the hood |

## Development

This project is 100% vibecoded: humans provide high-level designs and technical recommendations, but write zero lines of code. Every detail - planning, architecture, implementation - comes from a large language model running in the terminal. See [VIBECODED.md](VIBECODED.md) for more.

For coding standards, testing conventions, and documentation sync rules: [CONTRIBUTING.md](./CONTRIBUTING.md)
For AI/LLM agent-specific guidelines (commit format, error handling, async patterns): [AGENTS.md](./AGENTS.md)

## License

MIT — see [LICENSE](LICENSE) for details.
