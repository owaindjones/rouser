# Security Best Practices

This document outlines security best practices for deploying and operating `rouser`. Following these guidelines ensures the daemon operates securely in your environment.

## Configuration File Security

### File Permissions (System-Wide Install)

When installing system-wide at `/etc/rouser/config.toml`, restrict permissions:

```bash
sudo chown root:root /etc/rouser/config.toml
sudo chmod 0600 /etc/rouser/config.toml
```

**Rationale**: Prevents unprivileged users from modifying thresholds to keep the system awake (denial of service via configuration tampering).

### User-Level Config

User-level config files at `~/.config/rouser/config.toml` inherit standard home directory permissions. No special handling needed since they are already user-owned.

> **Note**: The configuration file does not store sensitive data (passwords, API keys), so the primary concern is preventing unauthorized modification rather than unauthorized reading.

## D-Bus Security

### Sleep Inhibition Access

`rouser` uses `org.freedesktop.login1.Manager.Inhibit` on the system D-Bus to prevent sleep. This requires access to the login1 service, which is typically available to any logged-in user via polkit policies.

**Security considerations**:
- An unprivileged user can inhibit sleep but cannot shut down or reboot without additional polkit rules (the `shutdown` lock type still requires elevated privileges on most systems)
- The default inhibitor configuration (`what = "shutdown:idle"`, `mode = "block"`) only blocks idle and shutdown — it does not prevent a privileged user from forcing a shutdown

### Polkit Rules for Desktop Environments

Some desktop environments (notably KDE Plasma's Powerdevil) ignore D-Bus inhibitors from unprivileged users. To grant inhibition permission:

Create `/etc/polkit-1/rules.d/50-rouser.rules`:

```javascript
polkit.addRule(function(action, subject) {
    if (action.id == "org.freedesktop.login1.inhibit" &&
        subject.user == "your_username") {
        return polkit.Result.YES;
    }
});
```

Only grant access to users or groups that truly need it. See [Introduction](introduction.md#desktop-environment-considerations) for more context.

### Inhibition Best Practices

- **Inhibit only what's necessary**: Configure the `what` parameter to inhibit only needed operations (e.g., just `"idle"` instead of `"shutdown:idle"`)
- **Use `block` mode sparingly**: The `block` mode completely blocks sleep, which may conflict with other power management policies
- **Monitor inhibition state**: Use `systemd-inhibit --list` to check what's currently inhibiting sleep

## Systemd User Service Security

### Hardened Service File

When running as a systemd user service, you can add security hardening directives:

```ini
[Service]
NoNewPrivileges=true
PrivateTmp=true
ProtectSystem=full
```

**Security features**:
- `NoNewPrivileges=true`: Prevents privilege escalation after startup
- `PrivateTmp=true`: Isolates `/tmp` for the service
- `ProtectSystem=full`: Makes the filesystem hierarchy read-only (except for explicitly writable paths)

### Resource Limits

Prevent resource exhaustion via systemd limits:

```ini
[Service]
MemoryMax=128M
CPUQuota=10%
```

**Best practices**:
- Set memory limits to prevent out-of-memory conditions (rouser typically uses 2–5 MB, so 128 MB is very generous)
- Limit CPU usage to prevent excessive resource consumption

## Dependency Management

### Security Auditing

Regularly audit dependencies for vulnerabilities:

```bash
# Install cargo-audit
cargo install cargo-audit

# Audit dependencies
cargo audit
```

**Continuous integration**: Add `cargo audit` to your CI pipeline and fail builds if critical/high severity vulnerabilities are found.

### Dependency Updates

- Pin dependency versions in `Cargo.toml`
- Check [RustSec advisories](https://rustsec.org/) regularly for known CVEs
- Update promptly: apply security patches within 48 hours for critical/high vulnerabilities
- Always run tests after updating dependencies

```bash
# Audit and update all dependencies
cargo audit
cargo update
cargo test
```

## Runtime Security

### Principle of Least Privilege

rouser is designed to run as a **systemd user service** under the unprivileged invoking user's account — no root access required. When running in system-wide mode (under root), apply additional hardening:

- Use `NoNewPrivileges=true`
- Restrict filesystem access with `ReadOnlyPaths=` and `ReadWritePaths=`
- Limit capabilities with `CapabilityBoundingSet=`

### Input Validation

All configuration values undergo validation at startup:
- **Numeric bounds**: Thresholds validated against allowed ranges (0.0–100.0 for percentages)
- **String values**: Validated against enumerated sets (e.g., valid log level and inhibition mode values)
- **Time durations**: Parsed via `humantime` with reasonable range checks

The daemon fails fast on invalid configuration rather than operating with potentially dangerous defaults.

## Logging Security

### Sensitive Data in Logs

rouser does not log sensitive data (passwords, API keys). Configuration values like thresholds and intervals are safe to log at debug level. However, if you extend rouser with custom logging:

- **Never** log passwords, tokens, or credentials
- **Avoid** logging full file contents of config files that may contain secrets
- Use structured logging via the `tracing` crate to control what fields are emitted

## Monitoring and Incident Response

### What to Monitor

1. **Service uptime**: Detect unexpected restarts or failures (`systemctl --user status rouser`)
2. **Inhibition state**: Unexpected sleep inhibition (check with `systemd-inhibit --list`)
3. **Resource usage**: Abnormal memory or CPU usage (`ps -p $(pgrep rouser) -o pid,pcpu,pmem,rss`)

### If Something Goes Wrong

1. Stop the service: `systemctl --user stop rouser.service`
2. Check logs: `journalctl --user -u rouser -n 50 --no-pager`
3. Validate config: `rouser --validate-config`
4. Test in dry-run mode: `rouser --dry-run -l debug`
5. Restart: `systemctl --user start rouser.service`

## CI/CD Pipeline Security

### GitHub Actions Permissions (Supply Chain Defense)

GitHub Actions workflows run in privileged environments that can exfiltrate secrets or inject malicious code into build artifacts. All workflow jobs follow the **principle of least privilege**: each job declares only the permissions it needs, scoped to the minimum required scope.

```yaml
jobs:
  lint-test:
    runs-on: ubuntu-latest
    permissions:
      contents: read        # Only read access — cannot push code or create releases
    steps: ...

  release-upload:
    runs-on: ubuntu-latest
    permissions:
      contents: write       # Write only for the job that uploads to GitHub Releases
      packages: write       # Required for publishing container/package registries
    steps: ...
```

**Never declare `permissions` at the workflow level with broad scopes.** This grants every job (including those that never need it) elevated access. If a downstream dependency in any job is compromised, all permissions are exposed. Always use job-level `permissions:` blocks.

### Workflow Linting with zizmor

Every PR to main runs [zizmor](https://woodruffw.github.io/zizmor/) — a GitHub Actions security linter that checks for:
- **dangerous-triggers**: Workflows triggered by untrusted events (PR forks, issue labels) without safeguards
- **cache-poisoning**: Malicious artifacts injected via cache keys derived from user input
- **unpinned-uses**: Actions referenced by mutable tags (e.g., `@v4`) instead of immutable SHAs
- **template-injection**: User-controlled data interpolated into workflow steps
- **excessive-permissions**: Jobs with more permissions than required

Add to your CI:
```yaml
zizmor-security-scan:
  runs-on: ubuntu-latest
  permissions: { contents: read }
  steps:
    - uses: actions/checkout@d632683dd7b4  # v4.3.1
      with:
        persist-credentials: false
    - name: Install zizmor
      uses: taiki-e/install-action@7769b73c2ec9  # v2
      with: { tool: zizmor }
    - run: zizmor .
```

### Dependency Pinning in Workflows

All `uses:` references are pinned to immutable commit SHAs. The following table documents every action reference used across workflow files:

| Action | Pinned SHA | Version | Notes |
|--------|-----------|---------|-------|
| actions/checkout | `@d632683dd7b4` | v4.3.1 | All steps include `persist-credentials: false` to prevent credential exposure via git config |
| dtolnay/rust-toolchain | `@d603d4afca40` | stable (master) | Immutable branch reference; CI installs cross-compilation targets at this SHA |
| Swatinem/rust-cache | `@cf9339e04bb8` | v2.7.0 | Cache keys use static strings or matrix values only — no user input in key derivation |
| taiki-e/install-action (v2) | `@7769b73c2ec9` | v2 | Used for zizmor, cargo-audit, and cross toolchain installation |
| actions/upload-artifact | `@ea165f8d65b6` | v4.6.2 | Release artifact uploads with scoped write permissions per job |
| actions/download-artifact | `@d3f86a106a0b` | v4.3.0 | Downloads CI artifacts for packaging jobs; patterns are deterministic |

When adding new action references, always look up the current commit SHA via GitHub API before committing workflow changes:

```bash
# Look up tag → SHA mapping
curl -sf "https://api.github.com/repos/OWNER/REPO/git/ref/tags/TAG" \
  | jq -r '.object.sha[:12]'
```

Document any new pinning in this table and add a comment on the `uses:` line matching the format above.

### Artifact Verification

All release artifacts are distributed as tarballs with embedded systemd service files and config. The PKGBUILD for Arch Linux computes SHA256 checksums from the actual GitHub release assets during CI generation — never using `'SKIP'`. This ensures package integrity verification at install time, preventing supply chain attacks where release binaries could be replaced.

### Known CI/CD Security Patterns to Avoid

| Anti-pattern | Risk | Fix |
|-------------|------|-----|
| `permissions: contents: write` at workflow level | Every job gets full repo access | Use per-job scoped permissions |
| `actions/checkout@v4` without pinning | Vulnerable to tag hijacking | Pin to commit SHA (see table above) |
| `${{ github.event.release.tag_name }}` interpolated into shell steps | Template injection via crafted release names | Use `$GITHUB_REF` env var with prefix stripping instead |
| Using `curl ... \| bash` in CI scripts | MITM during download + arbitrary code execution | Download to file, verify checksums, then execute |

### Rust Dependency Security

The project uses `cargo-audit` (via the `cargoaudit` crate's advisory database) in CI to scan for known CVEs:

```bash
cargo install cargo-audit  # Once per environment
cargo audit                 # Run before every commit / PR
```

Critical and high severity findings **fail the build**. See [Dependency Management](#dependency-management) above for runtime dependency policies.

## References

- [GitHub Actions security hardening guide](https://docs.github.com/en/actions/security-for-github-actions/security-hardening-your-workflows)
- [zizmor — GitHub Actions workflow linter](https://woodruffw.github.io/zizmor/)
- [RustSec advisories](https://rustsec.org/)
- [OWASP Linux Security Cheat Sheet](https://cheatsheetseries.owasp.org/cheatsheets/Linux_Security_Cheat_Sheet.html)
