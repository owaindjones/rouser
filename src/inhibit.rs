use dbus::blocking::Connection;
use tracing::{debug, warn};

/// The `what` parameter that works on desktop systems without polkit rules.
const FALLBACK_INHIBIT_TYPE: &str = "sleep";

/// Check if a D-Bus error indicates an interactive authentication requirement.
fn is_auth_error(error_msg: &str) -> bool {
    const AUTH_INDICATORS: &[&str] = &[
        "interactive authentication",
        "requires interactive authentication",
        "Access denied",
        "org.freedesktop.login1.NotAuthorized",
        "not authorized",
        "not authenticated",
    ];
    let lower = error_msg.to_lowercase();
    AUTH_INDICATORS
        .iter()
        .any(|indicator| lower.contains(indicator))
}

/// Sleep inhibitor using lower-level dbus crate.
pub struct SleepInhibitor {
    #[allow(dead_code)] // Connection kept for inhibitor lifetime
    conn: Connection,
    #[allow(dead_code)] // Keep the fd alive for inhibition
    _fd: dbus::arg::OwnedFd,
}

impl SleepInhibitor {
    /// Attempt D-Bus Inhibit call with the requested `what` type. Returns an OwnedFd that keeps
    /// inhibition active for the inhibitor's lifetime. Panics if mode is "block-weak" (use
    /// acquire_with_fallback() which handles this internally).
    async fn acquire_inhibition(
        what: &str,
        who: &str,
        why: &str,
        dbus_mode: &str,
    ) -> anyhow::Result<Self> {
        let conn = Connection::new_system()
            .map_err(|e| anyhow::anyhow!("Failed to connect to system D-Bus: {}", e))?;

        let proxy = conn.with_proxy(
            "org.freedesktop.login1",
            "/org/freedesktop/login1",
            std::time::Duration::from_millis(3000),
        );

        let result: (dbus::arg::OwnedFd,) = proxy
            .method_call(
                "org.freedesktop.login1.Manager",
                "Inhibit",
                (what, who, why, dbus_mode),
            )
            .map_err(|e| anyhow::anyhow!("Failed to call Inhibit: {}", e))?;

        let fd = result.0;

        Ok(Self { conn, _fd: fd })
    }

    /// Attempt inhibition with the requested `what` type. On desktop systems without polkit rules,
    /// `"shutdown:idle"` may fail with an authentication error — in that case this method falls back
    /// to using `"sleep"` which is less restrictive but more widely available. Only auth errors trigger fallback; other D-Bus failures propagate unchanged.
    pub async fn acquire_with_fallback(
        what: &str,
        who: &str,
        why: &str,
        mode: &str,
    ) -> anyhow::Result<Self> {
        let dbus_mode = match mode {
            "block-weak" => {
                warn!("D-Bus API does not support 'block-weak' mode. Using 'block' instead.");
                "block"
            }
            m => m,
        };

        // First attempt: try with the requested `what` type (e.g., "shutdown:idle").
        match Self::acquire_inhibition(what, who, why, dbus_mode).await {
            Ok(inhibitor) => Ok(inhibitor),
            Err(e) if is_auth_error(&e.to_string()) => {
                // Auth error — retry with the more widely-available "sleep" type.
                match Self::acquire_inhibition(FALLBACK_INHIBIT_TYPE, who, why, dbus_mode).await {
                    Ok(fb) => {
                        warn!(
                            "Requested inhibition type '{}' requires polkit interactive authentication — \
                             falling back to '{}'. To fix this, add a polkit rule or set inhibitor.what=sleep in config.",
                            what, FALLBACK_INHIBIT_TYPE
                        );
                        Ok(fb)
                    }
                    Err(e2) => {
                        warn!(
                            "Inhibition failed with '{}' (auth error indicator detected). \
                             Also tried fallback type '{}': {}",
                            what, FALLBACK_INHIBIT_TYPE, e2
                        );
                        Err(anyhow::anyhow!(
                            "Failed to acquire inhibition with both '{}' and fallback '{}'",
                            what,
                            FALLBACK_INHIBIT_TYPE
                        ))
                    }
                }
            }
            Err(e) => {
                // Not an auth error — report the original failure without masking it.
                Err(anyhow::anyhow!(
                    "Inhibition failed for type '{}': {} (not an auth error)",
                    what,
                    e
                ))
            }
        }
    }
}

pub struct InhibitionState {
    inhibitor: Option<SleepInhibitor>,
    is_inhibited: bool,
}

impl InhibitionState {
    pub fn new() -> Self {
        Self {
            inhibitor: None,
            is_inhibited: false,
        }
    }

    pub async fn acquire(
        &mut self,
        what: &str,
        who: &str,
        why: &str,
        mode: &str,
    ) -> anyhow::Result<()> {
        if self.is_inhibited {
            debug!("Already inhibited");
            return Ok(());
        }

        let inhibitor = SleepInhibitor::acquire_with_fallback(what, who, why, mode).await;

        match inhibitor {
            Ok(inh) => {
                self.inhibitor = Some(inh);
                self.is_inhibited = true;
                Ok(())
            }
            Err(e) => Err(e),
        }
    }

    pub async fn release(&mut self) {
        if !self.is_inhibited {
            debug!("Not currently inhibited");
            return;
        }

        self.inhibitor = None;
        self.is_inhibited = false;
    }

    pub fn is_inhibited(&self) -> bool {
        self.is_inhibited
    }

    #[allow(dead_code)]
    pub fn inhibitor(&self) -> Option<&SleepInhibitor> {
        self.inhibitor.as_ref()
    }
}

impl Drop for SleepInhibitor {
    fn drop(&mut self) {}
}

impl Default for InhibitionState {
    fn default() -> Self {
        Self::new()
    }
}
