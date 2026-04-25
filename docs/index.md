# rouser

A Linux daemon that monitors system metrics and inhibits sleep when activity thresholds are exceeded.

![rouser logo](../docs/rouser-logo.svg)

## Quick Start

1. **Install**: `curl -fsSL https://raw.githubusercontent.com/owaindjones/rouser/main/scripts/install.sh | bash`
2. **Configure**: Copy the default config and adjust thresholds: `mkdir -p ~/.config/rouser && cp config/rouser.toml ~/.config/rouser/config.toml`
3. **Test**: Run in dry-run mode first — `rouser --dry-run`
4. **Run as service**: `systemctl --user enable --now rouser.service`

## Documentation

- [Quick Start Guide](quickstart.md) — Step-by-step getting started
- [Configuration Reference](configuration.md) — All config options explained
- [Command Line](command-line.md) — CLI arguments and usage examples
- [Systemd User Service](systemd-user-service.md) — Running rouser as a service
- [Metrics Overview](metrics-overview.md) — How CPU, GPU, network, disk metrics are collected

## Links

- [Contributing Guide](../CONTRIBUTING.md)
- [Agent Guidelines (AI/LLM developers)](../AGENTS.md)
