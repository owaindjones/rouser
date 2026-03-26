# D-Bus Sleep Inhibition API

## Overview

This document describes the D-Bus API for inhibiting system sleep on Linux systems running systemd. The `rouser` daemon uses this API to prevent the system from entering sleep mode when metrics exceed configured thresholds.

## Background

Systemd provides a sleep inhibition mechanism that allows applications to register interest in preventing system suspend/hibernate. When an inhibitor is active, the system will delay or prevent sleep operations.

### Key Concepts

- **Inhibition Lock**: A file descriptor that keeps sleep inhibited
- **Inhibitors**: Applications that register to prevent sleep
- **what**: The type of sleep to inhibit (sleep, suspend, hibernate, shutdown)
- **mode**: How the inhibitor affects sleep (delay, block, etc.)

## D-Bus Service

### Service Name
`org.freedesktop.login1`

### Object Path
`/org/freedesktop/login1`

### Interface
`org.freedesktop.login1.Manager`

## Main Method

### `Inhibit()`

Prevents the system from entering sleep/shutdown until the inhibition lock is released.

#### Method Signature

```rust
Inhibit(sleep_type: s, mode: s, what: s, description: s)
  -> (fd: h, cookie: s)
```

#### Parameters

| Parameter | Type | Description |
|-----------|------|-------------|
| `sleep_type` | String | Type of sleep to inhibit |
| `mode` | String | How the inhibition should be handled |
| `what` | String | What operations to inhibit |
| `description` | String | Human-readable description of why sleep is inhibited |

#### Return Values

| Value | Type | Description |
|-------|------|-------------|
| `fd` | File Descriptor (h) | File descriptor to keep open while inhibiting |
| `cookie` | String | Unique identifier for this inhibition lock |

#### Parameter Details

**sleep_type** - One of:
- `"sleep"` - Inhibit suspend-to-RAM
- `"shutdown"` - Inhibit shutdown/reboot
- `"idle"` - Inhibit idle operations

**mode** - One of:
- `"block"` - Completely block the operation (default)
- `"delay"` - Delay the operation for a short period
- `"interact"` - Require user interaction before proceeding

**what** - One or more comma-separated values:
- `"sleep"` - Suspend-to-RAM
- `"hibernate"` - Suspend-to-disk
- `"shutdown"` - Shutdown or reboot
- `"idle"` - Idle operations

**description** - Any string describing the reason (e.g., `"Rouser daemon monitoring system activity"`)

#### Example Usage

```python
# Python example using dbus-python
import dbus

bus = dbus.SystemBus()
login1 = bus.get_object('org.freedesktop.login1', '/org/freedesktop/login1')
manager = dbus.Interface(login1, 'org.freedesktop.login1.Manager')

# Inhibit sleep - note parameter order: sleep_type, mode, what, description
fd, cookie = manager.Inhibit(
    sleep_type="sleep",
    mode="block",
    what="sleep,shutdown",
    description="Rouser daemon: system is active"
)

# Keep fd open while inhibiting
# When done, close the fd to release inhibition
```

```rust
// Rust example using zbus
use zbus::{Connection, Result};
use std::os::unix::io::RawFd;
use std::fs::File;

async fn inhibit_sleep(what: &str, description: &str) -> Result<(RawFd, String)> {
    let connection = Connection::system().await?;
    
    let proxy = connection
        .object_proxy("org.freedesktop.login1", "/org/freedesktop/login1")
        .typed::<(), (RawFd, String)>();
    
    let (fd, cookie) = proxy
        .call("Inhibit", &("sleep", "block", what, description))
        .await?;
    
    Ok((fd as RawFd, cookie))
}
```

## Active Inhibitors

### Method: `GetInhibitors()`

Retrieve currently active inhibitors for a specific operation.

#### Method Signature

```rust
GetInhibitors(sleep_type: s)
  -> (inhibitors: a{ssss})
```

#### Return Value

Map of inhibitors with keys:
- `what`: What is being inhibited
- `who`: Process name inhibiting
- `pid`: Process ID
- `mode`: Inhibition mode

#### Example

```python
# Get currently active sleep inhibitors
inhibitors = manager.GetInhibitors("sleep")
print(inhibitors)
# Output: {'sleep': {'what': 'sleep', 'who': 'rouser', 'pid': 1234, 'mode': 'block'}}
```

## Signals

### `InhibitorsChanged` Signal

Emit when inhibitors change.

#### Signal Signature

```rust
InhibitorsChanged(what: s)
```

#### Example

```python
# Listen for inhibitor changes
bus.add_signal_receiver(
    on_inhibitors_changed,
    signal_name="InhibitorsChanged",
    dbus_interface="org.freedesktop.login1.Manager",
    bus_name="org.freedesktop.login1",
    path="/org/freedesktop/login1"
)

def on_inhibitors_changed(what):
    print(f"Inhibitors changed for: {what}")
```

## Rust Implementation with zbus

### Complete Example with RAII

```rust
use zbus::{Connection, Result};
use std::os::unix::io::{AsRawFd, RawFd};
use std::fs::File;

pub struct SleepInhibitor {
    fd: File,
    cookie: String,
    what: String,
}

impl SleepInhibitor {
    pub async fn new(what: &str, description: &str) -> Result<Self> {
        let connection = Connection::system().await?;
        
        let proxy = connection
            .object_proxy("org.freedesktop.login1", "/org/freedesktop/login1")
            .typed::<(), (RawFd, String)>();
        
        let (fd, cookie) = proxy
            .call("Inhibit", &("sleep", "block", what, description))
            .await?;
        
        // Convert fd to File for RAII cleanup
        let file = unsafe { File::from_raw_fd(fd) };
        
        Ok(Self {
            fd: file,
            cookie,
            what: what.to_string(),
        })
    }
    
    pub fn cookie(&self) -> &str {
        &self.cookie
    }
    
    pub fn what(&self) -> &str {
        &self.what
    }
}

impl Drop for SleepInhibitor {
    fn drop(&mut self) {
        // File descriptor automatically closed when File is dropped
        // This releases the inhibition lock
        log::info!("Releasing sleep inhibition: {}", self.cookie);
    }
}
```

### Usage in rouser

```rust
pub async fn inhibit_if_active() -> Result<Option<SleepInhibitor>> {
    if metrics_exceed_threshold() {
        let inhibitor = SleepInhibitor::new(
            "sleep,hibernate,shutdown",
            "Rouser: system metrics above threshold"
        ).await?;
        
        log::info!("Sleep inhibited: {}", inhibitor.cookie());
        Ok(Some(inhibitor))
    } else {
        log::info!("All metrics below threshold, releasing inhibition");
        Ok(None)
    }
}
```

## Dependencies

### Rust Crates

```toml
[dependencies]
zbus = "4"  # D-Bus bindings for Rust
tokio = { version = "1", features = ["full"] }
log = "0.4"
```

### System Requirements

- Linux with systemd
- D-Bus system bus accessible
- User in `login` group or root privileges

## Error Handling

### Common Errors

| Error | Cause | Solution |
|-------|-------|----------|
| `AccessDenied` | Insufficient permissions | Add user to `login` group or run as root |
| `InvalidArg` | Invalid parameter values | Check sleep_type, mode, what values |
| `Failed` | D-Bus service unavailable | Ensure systemd is running |

### Error Handling Example

```rust
async fn safe_inhibit() -> Result<SleepInhibitor, InhibitError> {
    match SleepInhibitor::new("sleep,shutdown", "Rouser daemon").await {
        Ok(inhibitor) => Ok(inhibitor),
        Err(zbus::Error::DBus { name: _, message }) => {
            if message.contains("Access denied") {
                Err(InhibitError::PermissionDenied)
            } else {
                Err(InhibitError::DbusError(message))
            }
        }
        Err(e) => Err(InhibitError::Other(e.into())),
    }
}
```

## Best Practices

1. **Use RAII**: Wrap file descriptor in Rust struct with `Drop` trait
2. **Log Inhibition**: Record when sleep is inhibited and why
3. **Monitor Inhibitors**: Check existing inhibitors before creating new ones
4. **Graceful Release**: Ensure inhibition is released on program exit
5. **Describe Clearly**: Use meaningful descriptions for the `description` parameter

## Security Considerations

- Only inhibit when necessary (metrics exceed thresholds)
- Use descriptive names to identify your inhibitor
- Consider rate limiting inhibition requests
- Monitor for multiple inhibitors from the same application

## Testing

### Test Commands

```bash
# Check if you have permission to inhibit
loginctl list-inhibitors

# List all inhibitors
loginctl list-inhibitors --all

# Check systemd login service status
systemctl status systemd-logind
```

### Manual Testing

```bash
# Start a background process that inhibits sleep
systemd-inhibit --what=sleep --mode=delay "sleep 300"

# While running, try to suspend
systemctl suspend

# Should be delayed/blocked
```

## References

- [systemd-login1 D-Bus API](https://www.freedesktop.org/software/systemd/man/org.freedesktop.login1.html)
- [zbus Documentation](https://docs.rs/zbus/)
- [Rust D-Bus Binding Guide](https://dbus.freedesktop.org/doc/dbus-binding-api.html)

## Version History

- **v1.0** (2026-03-25): Initial documentation for rouser project
