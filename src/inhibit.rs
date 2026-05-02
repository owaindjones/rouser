use dbus::blocking::Connection;
use tracing::{debug, warn};

/// The default `what` parameter for D-Bus inhibition. On desktop systems without polkit rules,
/// `"shutdown:idle"` requires interactive authentication — this fallback is used when that fails.
const FALLBACK_INHIBIT_TYPE: &str = "sleep";

/// Check if a D-Bus error message indicates an interactive authentication requirement.
fn is_auth_error(error_msg: &str) -> bool {
    const AUTH_INDICATORS: &[&str] = &[
        "interactive authentication",
        "requires interactive authentication",
        "Access denied",
        "org.freedesktop.login1.dismiss",
    ];
    let lower = error_msg.to_lowercase();
    AUTH_INDICATORS.iter().any(|indicator| lower.contains(indicator))
}

/// Sleep inhibitor using lower-level dbus crate.
pub struct SleepInhibitor {
    #[allow(dead_code)] // Connection kept for inhibitor lifetime
    conn: Connection,
    #[allow(dead_code)] // Keep the fd alive for inhibition
    _fd: dbus::arg::OwnedFd,
}

impl SleepInhibitor {
    /// Attempt inhibition with the requested `what` type. On desktop systems without polkit rules,
    /// `"shutdown:idle"` may fail with an authentication error — use `acquire_with_fallback()` for that case.
    pub async fn acquire_with_fallback(what: &str, who: &str, why: &str, mode: &str) -> anyhow::Result<Self> {
        Self::acquire_inhibition(what, who, why, mode).await
    }

    /// Core D-Bus Inhibit call. Returns an OwnedFd that keeps inhibition active for the inhibitor's lifetime.
    async fn acquire_inhibition(what: &str, who: &str, why: &str, mode: &str) -> anyhow::Result<Self> {
        let dbus_mode = match mode {
            "block-weak" => {
                warn!(
                    "D-Bus API does not support 'block-weak' mode. Using 'block' instead."
                );
                "block"
            }
            m => m,
        };

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
                (
                    what.to_string(),
                    who.to_string(),
                    why.to_string(),
                    dbus_mode.to_string(),
                ),
            )
            .map_err(|e| anyhow::anyhow!("Failed to call Inhibit: {}", e))?;

        let fd = result.0;

        Ok(Self { conn, _fd: fd })
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
            Err(e) if is_auth_error(&e.to_string()) => {
                let fallback_msg = format!(
                    "{} (falling back to '{}')",
                    e, FALLBACK_INHIBIT_TYPE
                );

                match SleepInhibitor::acquire_inhibition(FALLBACK_INHIBIT_TYPE, who, why, mode).await {
                    Ok(fb) => {
                        warn!("{}", fallback_msg);
                        self.inhibitor = Some(fb);
                        self.is_inhibited = true;
                        Ok(())
                    }
                    Err(fb_err) => Err(anyhow::anyhow!("{} (fallback also failed: {})", e, fb_err)),
                }
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
