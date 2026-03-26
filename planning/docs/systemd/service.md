# Systemd Service Configuration

This document describes how to configure and deploy `rouser` as a systemd service.

## Service File Template

### Default Configuration

Create `/etc/systemd/system/rouser.service`:

```ini
[Unit]
Description=rouser - Linux Sleep Inhibition Daemon
Documentation=https://github.com/yourusername/rouser
After=network.target
Wants=network-online.target

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
ProtectHome=true
PrivateTmp=true
PrivateDevices=true
ReadWritePaths=/var/log/rouser
CapabilityBoundingSet=CAP_SYS_ADMIN
SystemCallFilter=@system-service
SystemCallFilter=~@privileged @resources

[Install]
WantedBy=multi-user.target
```

### Minimal Configuration

For systems with less stringent security requirements:

```ini
[Unit]
Description=rouser - Linux Sleep Inhibition Daemon
After=network.target

[Service]
Type=simple
User=root
Group=root
ExecStart=/usr/local/bin/rouser --config /etc/rouser/config.toml
Restart=on-failure
RestartSec=5s

[Install]
WantedBy=multi-user.target
```

## Security Configuration

### Running as Dedicated User (Recommended for Production)

Create a dedicated user:

```bash
sudo useradd -r -s /usr/bin/false -U -M -r rouser
```

Update service file:

```ini
[Service]
Type=simple
User=rouser
Group=rouser
ExecStart=/usr/local/bin/rouser --config /etc/rouser/config.toml
Restart=on-failure
RestartSec=5s

# Grant only necessary permissions
CapabilityBoundingSet=CAP_SYS_ADMIN
AmbientCapabilities=CAP_SYS_ADMIN

# Restrict filesystem access
ProtectSystem=strict
ProtectHome=true
PrivateTmp=true
ReadWritePaths=/var/log/rouser
ReadOnlyPaths=/etc/rouser
```

Ensure log directory exists with correct permissions:

```bash
sudo install -m 755 -d /var/log/rouser
sudo chown rouser:rouser /var/log/rouser
```

## Environment Configuration

### Environment Variables

Override configuration with environment variables:

```ini
[Service]
Type=simple
User=root
ExecStart=/usr/local/bin/rouser --config /etc/rouser/config.toml
Environment=ROUSER_LOG_LEVEL=debug
Environment=ROUSER_THRESHOLDS_CPU_USAGE=75
```

### Drop-in Override Directory

Create custom configuration without modifying main service file:

```bash
sudo mkdir -p /etc/systemd/system/rouser.service.d
```

Create `/etc/systemd/system/rouser.service.d/override.conf`:

```ini
[Service]
# Override environment
Environment=ROUSER_LOG_LEVEL=debug

# Override restart policy
Restart=on-success

# Override timeout
TimeoutStartSec=30
TimeoutStopSec=30
```

## Resource Limits

### Memory and CPU Limits

Prevent resource exhaustion:

```ini
[Service]
Type=simple
User=root
ExecStart=/usr/local/bin/rouser --config /etc/rouser/config.toml

# Resource limits
MemoryLimit=256M
MemoryHigh=200M
CPUQuota=50%
TasksMax=10
```

### I/O Limits

Control disk I/O impact:

```ini
[Service]
Type=simple
User=root
ExecStart=/usr/local/bin/rouser --config /etc/rouser/config.toml

# I/O limits (optional, depends on systemd version)
IOWeight=100
IOSchedulingClass=best-effort
IOSchedulingPriority=4
```

## Logging Configuration

### Journal Logging (Recommended)

Use systemd journal for centralized logging:

```ini
[Service]
Type=simple
User=root
ExecStart=/usr/local/bin/rouser --config /etc/rouser/config.toml
StandardOutput=journal
StandardError=journal
```

Access logs:

```bash
# View logs
sudo journalctl -u rouser -f

# View specific time range
sudo journalctl -u rouser --since "2024-03-26 00:00:00" --until "2024-03-26 23:59:59"

# Filter by priority
sudo journalctl -u rouser -p warning

# Follow with tail
sudo journalctl -u rouser -n 100 -f
```

### File Logging

Write to dedicated log file:

```ini
[Service]
Type=simple
User=root
ExecStart=/usr/local/bin/rouser --config /etc/rouser/config.toml
StandardOutput=append:/var/log/rouser/rouser.log
StandardError=append:/var/log/rouser/rouser.log
```

### Syslog Integration

Log to syslog:

```ini
[Service]
Type=simple
User=root
ExecStart=/usr/local/bin/rouser --config /etc/rouser/config.toml
StandardOutput=syslog
StandardError=syslog
SyslogIdentifier=rouser
```

## Health Checks

### ExecStartPre Validation

Validate configuration before starting:

```ini
[Service]
Type=forking
ExecStartPre=/usr/local/bin/rouser --validate-config /etc/rouser/config.toml
ExecStart=/usr/local/bin/rouser --config /etc/rouser/config.toml
ExecReload=/usr/local/bin/rouser --reload-config
```

### ExecReload for Dynamic Config

Support configuration reload without restart:

```ini
[Service]
Type=simple
User=root
ExecStart=/usr/local/bin/rouser --config /etc/rouser/config.toml
ExecReload=/usr/local/bin/rouser --reload-config /etc/rouser/config.toml
KillMode=mixed
KillSignal=SIGTERM
TimeoutStopSec=30
```

## Restart Policy

### Automatic Restart on Failure

```ini
[Service]
Type=simple
User=root
ExecStart=/usr/local/bin/rouser --config /etc/rouser/config.toml
Restart=on-failure
RestartSec=5s
```

### Advanced Restart Options

```ini
[Service]
Type=simple
User=root
ExecStart=/usr/local/bin/rouser --config /etc/rouser/config.toml

# Restart on any exit code except 0
Restart=always
RestartSec=5s

# Maximum restart attempts
StartLimitBurst=5
StartLimitIntervalSec=300
```

## Service Dependencies

### Network Dependency

Ensure network is up before starting:

```ini
[Unit]
Description=rouser - Linux Sleep Inhibition Daemon
After=network.target
Wants=network-online.target
Requires=network-online.target
```

### Other Services

Coordinate with other services:

```ini
[Unit]
Description=rouser - Linux Sleep Inhibition Daemon
After=network.target
After=dbus.service
BindsTo=dbus.service
```

## Installation Commands

### Step 1: Create Service File

```bash
sudo install -m 644 /path/to/rouser.service /etc/systemd/system/rouser.service
```

### Step 2: Reload and Enable

```bash
# Reload systemd daemon
sudo systemctl daemon-reload

# Enable on boot
sudo systemctl enable rouser

# Start service
sudo systemctl start rouser
```

### Step 3: Verify

```bash
# Check status
sudo systemctl status rouser

# Check active state
sudo systemctl is-active rouser

# View unit file
systemctl cat rouser

# List dependencies
systemctl list-dependencies rouser
```

## Maintenance

### Reload Configuration

```bash
# Reload service configuration (not daemon config)
sudo systemctl daemon-reload

# Reload daemon configuration (if supported)
sudo systemctl reload rouser
```

### Update Service

```bash
# Stop service
sudo systemctl stop rouser

# Replace binary
sudo install -m 755 target/release/rouser /usr/local/bin/rouser

# Restart service
sudo systemctl restart rouser
```

### Diagnostics

```bash
# Check configuration
rouser --validate-config /etc/rouser/config.toml

# Check D-Bus permissions
loginctl list-inhibitors

# Test D-Bus inhibition
systemd-inhibit --what=sleep --mode=delay --why="Testing" sleep 10

# Monitor logs in real-time
sudo journalctl -u rouser -f
```

## Uninstallation

### Remove Service

```bash
# Stop and disable service
sudo systemctl stop rouser
sudo systemctl disable rouser

# Remove service file
sudo rm /etc/systemd/system/rouser.service

# Reload daemon
sudo systemctl daemon-reload

# Remove binary
sudo rm /usr/local/bin/rouser

# Remove config (optional)
sudo rm -rf /etc/rouser
```

## Security Hardening Checklist

- [x] Service runs as dedicated user (rouser)
- [x] Configuration file has mode 0600, owned by root
- [x] NoNewPrivileges=true
- [x] ProtectSystem=strict
- [x] ProtectHome=true
- [x] PrivateTmp=true
- [x] CapabilityBoundingSet limited to CAP_SYS_ADMIN
- [x] No unnecessary filesystem paths in ReadWritePaths
- [x] Log file permissions set to 0640

## References

- [systemd.service(5) man page](https://www.freedesktop.org/software/systemd/man/systemd.service.html)
- [systemd.exec(5) man page](https://www.freedesktop.org/software/systemd/man/systemd.exec.html)
- [systemd.resource-control(5) man page](https://www.freedesktop.org/software/systemd/man/systemd.resource-control.html)

## See Also

- [Quick Start Guide](../quickstart.md)
- [Configuration Reference](../configuration/reference.md)
- [Security Best Practices](../security.md)
