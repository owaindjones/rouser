# D-Bus Sleep Inhibition API Reference

This document describes the D-Bus API used by `rouser` for inhibiting system sleep on Linux systems running systemd. It serves as a technical reference for developers extending or debugging inhibition behavior.

## Overview

Systemd provides a sleep inhibition mechanism that allows applications to register interest in preventing system suspend/hibernate. When an inhibitor is active, the system delays or prevents sleep operations.

rouser uses this via the `org.freedesktop.login1.Manager.Inhibit` method on the **system** D-Bus. Unlike session-bus APIs (e.g., the deprecated FreeDesktop PowerManagement API), systemd login1 works without a graphical session and is actively maintained.

### Key Concepts

- **Inhibition Lock**: A file descriptor that, when kept open, holds sleep inhibited
- **cookie**: A unique identifier returned by `Inhibit()` for tracking individual inhibition requests
- **what**: The type of operation to inhibit (sleep, shutdown, etc.)
- **mode**: How the inhibitor affects the operation (block, delay, block-weak)

## D-Bus Service Details

| Property | Value |
|----------|-------|
| Bus | System bus (`org.freedesktop.login1`) |
| Object Path | `/org/freedesktop/login1` |
| Interface | `org.freedesktop.login1.Manager` |
| Method | `Inhibit(string what, string mode, string who, string reason)` → `(handle cookie)` |

### Inhibition Parameters

**what** — Colon-separated lock types:

| Lock Type | ACPI State | Description |
|-----------|-----------|-------------|
| `idle` | S5 (software power off) / idle suspend | Prevents the system from entering an idle/off state |
| `sleep` | S3 (Suspend-to-RAM) / S4 (Suspend-to-disk) | Prevents sleep/hibernate operations |

**mode** — How the inhibitor affects the operation:

| Mode | Behavior | Equivalent systemd directive |
|------|----------|-------------------------------|
| `block` | Completely blocks the operation while lock is held | Default behavior |
| `delay` | Delays the operation for up to `InhibitDelayMaxUSec()` | `InhibitDelayMaxUSec=` |
| `block-weak` | Blocks but can be overridden by privileged processes | N/A (internal) |

**who** — A short string identifying the process (e.g., `"rouser"`). Appears in `systemd-inhibit --list`.

**reason** — A human-readable description of why sleep is inhibited. Appears in inhibitor listings for operators.

### Return Values

| Value | Type | Description |
|-------|------|-------------|
| `fd` (handle) | Unix file descriptor (`h`) | Keep open to maintain inhibition; close to release |
| `cookie` | String | Unique identifier for this lock, used for tracking/debugging |

## Lock Types Reference

The `what` parameter accepts these values individually or combined with colons:

| Value | Description | Use Case |
|-------|-------------|----------|
| `idle` | Prevents idle/off state transitions | Headless servers that should stay awake during background work even when no user is logged in |
| `sleep` | Prevents S3 (Suspend-to-RAM) and S4 (Suspend-to-disk) | Standard use case for desktop/laptop systems |

**Default**: rouser defaults to `"shutdown:idle"`. The choice between `"sleep"` and `"shutdown:idle"` affects how desktop environments behave:

- **`"sleep"`** — Simple, good for headless servers. Blocks sleep/hibernate but does not interfere with DE idle timers. Observed issue on KDE: may cause the system to never automatically sleep after inhibition is released, as it interprets the lock as a persistent "don't touch my power management" signal.
- **`"shutdown:idle"`** — Conservative default, better for workstations/home-labs running DEs like KDE/GNOME. Prevents powered-off/suspended state transitions while metrics are active but lets the DE respect its configured idle delay (e.g., if KDE is set to sleep after 15 minutes of inactivity, it will put the system to sleep 15 minutes after rouser releases locks).

To change between options:

```toml
[inhibitor]
what = "idle:sleep"   # Block both idle and sleep
mode = "block"
```

> **Note**: The `shutdown` lock type in the default config does not prevent reboot/shutdown — it prevents idle-to-shutdown transitions. A privileged user can still force shutdown with `systemctl --force reboot`.

## Rust Implementation (zbus v4)

rouser uses `zbus` version 4 for D-Bus communication. The inhibition logic is in `src/inhibit.rs`.

### Core Pattern: RAII Wrap

The file descriptor returned by `Inhibit()` must be kept open while inhibition should be active, and closed when inhibition ends. rouser wraps this using Rust's Drop trait for automatic cleanup:

```rust
use zbus::{Connection, Result};
use std::os::unix::io::RawFd;

pub struct SleepInhibitor {
    fd: RawFd,
    cookie: String,
}

impl SleepInhibitor {
    pub async fn new(what: &str, mode: &str, reason: &str) -> Result<Self> {
        let connection = Connection::system().await?;
        
        // Call Inhibit() on the system bus
        let (fd, cookie): (RawFd, String) = connection.call_method(
            Some("org.freedesktop.login1"),
            "/org/freedesktop/login1",
            Some("org.freedesktop.login1.Manager"),
            "Inhibit",
            &(what, mode, "rouser", reason),
        ).await?;

        Ok(Self { fd, cookie })
    }
}

impl Drop for SleepInhibitor {
    fn drop(&mut self) {
        // File descriptor is automatically closed when dropped.
        // This releases the inhibition lock and allows sleep again.
    }
}
```

When `SleepInhibitor` goes out of scope (or is explicitly dropped), the file descriptor closes, releasing inhibition immediately — no explicit D-Bus release call needed.

### Usage in rouser's Service Loop

The inhibitor is acquired/released within the main service loop (`src/service.rs`):

1. Each tick collects metrics and evaluates thresholds
2. If any metric exceeds its threshold continuously for `duration_threshold`, a new `SleepInhibitor` is created
3. While inhibited, inhibition state persists across ticks (the `SleepInhibitor` struct lives in `DataManager`)
4. When all metrics stay below their thresholds for `cooldown_duration`, the inhibitor is dropped and sleep is released

### State-Change Logging

Following best practices, rouser only logs INFO-level messages when the inhibition state **transitions** — not on every polling tick:

```rust
// After evaluating state in a tick:
let current_inhibited = self.state.is_inhibited();
if !self.previous_inhibited_state && current_inhibited {
    info!("Sleep inhibited: at least one metric above threshold");
} else if self.previous_inhibited_state && !current_inhibited {
    info!("Releasing sleep inhibition: all metrics below thresholds for cooldown duration");
}
self.previous_inhibited_state = current_inhibited;
```

This prevents log spam (previously, "Sleep inhibited" was logged every 5 seconds while active). See [AGENTS.md](./../AGENTS.md) → Logging Conventions for the state-change-only logging rule.

## Debugging and Verification

### Check Active Inhibitors

```bash
# List all current inhibitors system-wide
systemd-inhibit --list

# Show detailed info about a specific inhibitor
systemd-inhibit --list --what=sleep

# From rouser's perspective — check if your user has inhibition permission
loginctl list-sessions | grep $(whoami)
```

### Check systemd-logind Status

```bash
# Verify the login1 D-Bus service is running
systemctl status systemd-logind

# Check for errors in logind journal
journalctl -u systemd-logind --since "5 min ago"
```

### Common Errors

| Error | Cause | Solution |
|-------|-------|----------|
| `AccessDenied` (org.freedesktop.login1) | User lacks permission to inhibit via polkit | Add polkit rule for the user, or run as root. See [Security Best Practices](security.md#polkit-rules-for-desktop-environments). |
| `Failed` (org.freedesktop.DBus.Error.ServiceUnknown) | systemd-logind not running on system bus | Ensure systemd is active: `systemctl --user start systemd-logind` or check host-level service. |
| No inhibitors showing in `systemd-inhibit --list` | Inhibition was released before command ran | Check timing — inhibition may have been released between polling cycles. Run rouser with debug logging to track state transitions. |

## Deprecated APIs (Do Not Use)

### FreeDesktop PowerManagement API

The old `/org/freedesktop/PowerManagement.Inhibit` API is **obsolete** and should never be used:
- Deprecated around 2014 (systemd 183 era)
- Specs no longer available in current freedesktop.org documentation
- Does not work reliably on modern desktop environments
- KDE Powerdevil ignores inhibitors from unprivileged users regardless of the API used

Always use `org.freedesktop.login1.Manager.Inhibit` instead. See [AGENTS.md](./../AGENTS.md) → Lessons Learned for more context on why this approach was chosen over alternatives.

## References

- [systemd login1 D-Bus API docs](https://www.freedesktop.org/software/systemd/man/latest/org.freedesktop.login1.html#Inhibit%20)
- [systemd inhibitor lock types](https://systemd.io/INHIBITOR_LOCKS/)
- [zbus documentation](https://docs.rs/zbus/)
