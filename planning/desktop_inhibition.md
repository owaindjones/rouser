# Desktop Sleep Inhibition - DEPRECATED

## Status: Dead End

The FreeDesktop PowerManagement API (`/org/freedesktop/PowerManagement.Inhibit`) was researched as an alternative to systemd login1 inhibition, but is **obsolete** and should not be used.

### Why abandoned

- Deprecated around 2014 (systemd 183)
- Specs no longer available in current FreeDesktop documentation
- Does not work reliably on modern desktop environments
- KDE Powerdevil has known issues ignoring inhibitors from unprivileged users anyway (KDE Bug 457859)

### Recommended approach

Use **systemd login1 inhibition** via `org.freedesktop.login1.Manager.Inhibit` on the system D-Bus. This is:

- Actively maintained and documented
- Works without requiring session bus access
- Supported by all modern Linux distributions

Reference: https://www.freedesktop.org/software/systemd/man/latest/org.freedesktop.login1.html#Inhibit%20

### Lock types

The `what` field in inhibition accepts these lock types (from systemd):
- `idle` - prevents idle suspend
- `sleep` - prevents sleep/hibernate  
- `suspend` - prevents suspend
- `shutdown` - prevents shutdown

Valid `mode` values: `block`, `delay`, `block-weak`

Example config:
```toml
[inhibitor]
what = "idle"    # or "sleep", "suspend", "shutdown"
mode = "block"   # or "delay", "block-weak"
```
