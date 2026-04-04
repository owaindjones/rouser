use dbus::blocking::Connection;
use tracing::{debug, info};

/// Sleep inhibitor using lower-level dbus crate
/// The dbus crate properly handles file descriptors (h: UNIX_FD type)
pub struct SleepInhibitor {
    #[allow(dead_code)] // Connection kept for inhibitor lifetime
    conn: Connection,
    #[allow(dead_code)] // Keep the fd alive for inhibition
    _fd: dbus::arg::OwnedFd,
    #[allow(dead_code)]
    what: String,
    description: String,
}

impl SleepInhibitor {
  pub async fn new(
        what: &str,
        who: &str,
        why: &str,
        mode: &str,
    ) -> anyhow::Result<Self> {
        let dbus_mode = mode;
        
        // Connect to system D-Bus
        let conn = Connection::new_system()
            .map_err(|e| anyhow::anyhow!("Failed to connect to system D-Bus: {}", e))?;

        // Use with_proxy to create a wrapper for the target object
        let proxy = conn.with_proxy(
            "org.freedesktop.login1",
            "/org/freedesktop/login1",
            std::time::Duration::from_millis(3000),
        );

        // Call Inhibit - returns (file_descriptor,) tuple
        // The dbus crate handles file descriptors properly via OwnedFd
        let result: (dbus::arg::OwnedFd,) = proxy
            .method_call(
                "org.freedesktop.login1.Manager", 
                "Inhibit", 
                (what.to_string(), who.to_string(), why.to_string(), dbus_mode.to_string())
            )
            .map_err(|e| anyhow::anyhow!("Failed to call Inhibit: {}", e))?;

        // Keep the file descriptor alive for the lifetime of the inhibition
        // The fd is what keeps the inhibition active - it must not be dropped
        let fd = result.0;
        
        info!("Inhibition acquired successfully");

        Ok(Self {
            conn,
            _fd: fd,
            what: what.to_string(),
            description: why.to_string(),
        })
    }

    #[allow(dead_code)]
    pub fn what(&self) -> &str {
        &self.what
    }

    pub fn description(&self) -> &str {
        &self.description
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

    #[allow(dead_code)]
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

        let inhibitor = SleepInhibitor::new(what, who, why, mode).await?;

        self.inhibitor = Some(inhibitor);
        self.is_inhibited = true;

        Ok(())
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
    fn drop(&mut self) {
        info!("Sleep inhibition released for: {}", self.description());
    }
}

impl Default for InhibitionState {
    fn default() -> Self {
        Self::new()
    }
}
