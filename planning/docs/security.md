# Security Best Practices

## Overview

This document outlines security best practices for deploying and operating `rouser`. Following these guidelines ensures the daemon operates securely in your environment and doesn't introduce vulnerabilities.

## Configuration File Security

### File Permissions

The configuration file should have restrictive permissions to prevent unauthorized modification:

```bash
# Set owner and group to root
sudo chown root:root /etc/rouser/config.toml

# Set permissions to 0600 (owner read/write only)
sudo chmod 0600 /etc/rouser/config.toml
```

**Rationale**:
- Prevents unprivileged users from modifying thresholds to keep the system awake
- Avoids denial of service via configuration tampering
- Complies with principle of least privilege

### Sensitive Configuration

If you include sensitive data in configuration (e.g., API keys, tokens), consider:

1. **Environment Variable Overrides**: Use environment variables for sensitive values instead of hardcoding
2. **File Encryption**: Encrypt the configuration file at rest
3. **Separate Secrets**: Use a secrets manager instead of embedding credentials

## D-Bus Security

### System Bus Access

`rouser` requires access to the D-Bus system bus to inhibit sleep:

```bash
# Default D-Bus service
org.freedesktop.login1
```

**Security Considerations**:

1. **Root Privileges Required**: D-Bus system bus typically requires root access
2. **Polkit Rules**: On systems using PolicyKit, consider creating polkit rules to restrict access:

```bash
# /etc/polkit-1/rules.d/rouser.rules
polkit.addRule(function(action, subject) {
    if (action.id == "org.freedesktop.login1.inhibit" &&
        subject.isInGroup("systemd-journal")) {
        return polkit.Result.YES;
    }
});
```

**Warning**: Only grant access to users/groups that truly need it.

### Inhibition Best Practices

- **Inhibit Only What's Necessary**: Configure the `what` parameter to inhibit only needed operations (e.g., just `sleep`, not `shutdown`)
- **Use `block` Mode Sparingly**: The `block` mode completely prevents sleep, which may cause issues if combined with other power management policies
- **Monitor Inhibition State**: Use `systemd-inhibit --what=sleep list` to check what's currently inhibiting sleep

## Systemd Service Security

### Service Account

For improved security, consider running `rouser` as a dedicated user with minimal privileges:

```ini
# /etc/systemd/system/rouser.service
[Unit]
Description=rouser - Linux Sleep Inhibition Daemon
After=network.target

[Service]
Type=simple
User=rouser
Group=rouser
ExecStart=/usr/local/bin/rouser --config /etc/rouser/config.toml
Restart=on-failure
RestartSec=5s

# Security hardening
NoNewPrivileges=true
ProtectSystem=strict
ProtectHome=true
PrivateTmp=true
ReadWritePaths=/var/log/rouser
CapabilityBoundingSet=CAP_SYS_ADMIN

[Install]
WantedBy=multi-user.target
```

**Security Features**:

- `NoNewPrivileges=true`: Prevents privilege escalation
- `ProtectSystem=strict`: Makes filesystem read-only except specified paths
- `PrivateTmp=true`: Isolates `/tmp` for the service
- `CapabilityBoundingSet`: Limits capabilities to only `CAP_SYS_ADMIN` (needed for D-Bus)

**Note**: If running as non-root, ensure D-Bus permissions are correctly configured (see D-Bus Security section above).

## Input Validation

All configuration values undergo validation:

- **Numeric Bounds**: Thresholds validated against allowed ranges (0.0 - 100.0 for percentages)
- **String Values**: Validated against enumerated sets (e.g., valid `log_level` values)
- **File Paths**: Checked for existence and accessibility
- **Time Durations**: Parsed and validated for reasonable ranges

**Development Best Practices**:

1. Use Rust's type system to enforce valid values
2. Implement configuration validation at startup
3. Log validation errors clearly with line numbers
4. Fail fast on invalid configuration

## Dependency Management

### Security Auditing

Regularly audit dependencies for vulnerabilities:

```bash
# Install cargo-audit
cargo install cargo-audit

# Audit dependencies
cargo audit

# Audit with fix (if safe to do so)
cargo audit --fix
```

**Continuous Integration**:
- Add `cargo audit` to your CI pipeline
- Fail builds if critical/high severity vulnerabilities are found
- Set up automated alerts for new CVEs affecting your dependencies

### Dependency Updates

```bash
# Update all dependencies to latest compatible versions
cargo update

# Update a specific dependency
cargo update -p package-name
```

**Best Practices**:

1. **Pin Dependency Versions**: Use specific versions in `Cargo.toml`
2. **Review Security Advisories**: Regularly check https://rustsec.org/
3. **Update Promptly**: Apply security patches within 48 hours for critical/high vulnerabilities
4. **Test After Updates**: Always run tests after updating dependencies

## Logging Security

### Sensitive Data in Logs

**Never log** in your configuration:

- Passwords or API keys
- Sensitive user data
- Security tokens

**Example of what NOT to do**:

```toml
# DON'T do this
[auth]
api_key = "secret123"  # Will be logged if debug logging enabled
```

**Do this instead**:

```toml
# Use environment variables for sensitive data
[auth]
api_key_env = "ROUSER_API_KEY"  # Read from environment instead
```

### Log File Permissions

```bash
sudo chown rouser:rouser /var/log/rouser/rouser.log
sudo chmod 0640 /var/log/rouser/rouser.log
```

**Rationale**:
- Prevents unauthorized users from reading logs
- Limits exposure if the log file is compromised
- Compliance with security best practices

## Runtime Security

### Resource Limits

Prevent memory or CPU exhaustion via resource limits:

```ini
# /etc/systemd/system/rouser.service
[Service]
MemoryLimit=256M
CPUQuota=50%
```

**Best Practices**:

1. Set memory limits to prevent out-of-memory conditions
2. Limit CPU usage to prevent denial of service
3. Monitor resource usage in production

### Filesystem Access

**Restrict Filesystem Access**:

```ini
[Service]
ReadWritePaths=/var/log/rouser
ReadOnlyPaths=/etc/rouser
```

**Rationale**:
- Only grant read/write access to necessary directories
- Make configuration files read-only
- Prevent accidental modification of critical files

## Incident Response

### What to Do If Compromised

1. **Stop the Service**:
   ```bash
   sudo systemctl stop rouser
   ```

2. **Preserve Evidence**:
   ```bash
   sudo cp /var/log/rouser/rouser.log /tmp/rouser-incident-$(date +%Y%m%d).log
   ```

3. **Review Logs**:
   ```bash
   grep -i "error\|warning\|invalid" /tmp/rouser-incident-*.log
   ```

4. **Reset Configuration**:
   - Replace configuration file with known-good backup
   - Reset service to default state

5. **Investigate**:
   - Check system logs (`journalctl -u rouser`)
   - Review D-Bus activity
   - Audit user access to configuration files

## Compliance

### Common Security Standards

| Standard | Requirement | Implementation |
|----------|-------------|----------------|
| CIS Benchmark | Service isolation | `PrivateTmp=true`, `ProtectSystem` |
| NIST 800-53 | Access control | Root-only config file access |
| SOC 2 | Audit logging | Comprehensive logging with rotation |
| GDPR | Data protection | No sensitive data in logs |

## Monitoring Security

### What to Monitor

1. **Configuration Changes**: Alert on any modification to `/etc/rouser/config.toml`
2. **Inhibition State**: Monitor unexpected inhibition of sleep
3. **Service Uptime**: Detect unexpected restarts or failures
4. **Resource Usage**: Alert on abnormal memory or CPU usage

### Example Alert Rules (Prometheus/Grafana)

```promql
# Configuration file modification
changes(file:rouser_config_modified{job="rouser"}[5m]) > 0

# Unexpected inhibition state
rouser_inhibition_state{state="inhibited"} == 1
```

## References

- [Linux Security Module](https://www.kernel.org/doc/html/latest/admin-guide/LSM/index.html)
- [systemd Security Features](https://www.freedesktop.org/software/systemd/man/systemd.exec.html#Security)
- [OWASP Cheat Sheet: Linux Security](https://cheatsheetseries.owasp.org/cheatsheets/Linux_Security_Cheat_Sheet.html)
- [Rust Security Best Practices](https://rust-lang.github.io/wg-security-advisories/)

## See Also

- [Configuration Reference](configuration/reference.md)
- [Systemd Service Configuration](systemd/service.md)
- [D-Bus Inhibition API](d-bus/inhibition.md)
