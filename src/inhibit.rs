use anyhow::{Result, anyhow};
use std::os::unix::io::RawFd;
use std::fs::File;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{debug, info, warn};

pub struct SleepInhibitor {
    fd: Option<File>,
    cookie: Option<String>,
    what: String,
    description: String,
}

impl SleepInhibitor {
    pub async fn new(
        what: &str,
        description: &str,
        mode: &str,
    ) -> Result<Self> {
        use std::ffi::CString;
        use std::ptr;

        let what_c = CString::new(what)
            .map_err(|e| anyhow!("Failed to convert what to C string: {}", e))?;
        let mode_c = CString::new(mode)
            .map_err(|e| anyhow!("Failed to convert mode to C string: {}", e))?;
        let description_c = CString::new(description)
            .map_err(|e| anyhow!("Failed to convert description to C string: {}", e))?;

        let sleep_type = "sleep";
        let sleep_type_c = CString::new(sleep_type)
            .map_err(|e| anyhow!("Failed to convert sleep_type to C string: {}", e))?;

        let connection = zbus::Connection::system()
            .await
            .map_err(|e| anyhow!("Failed to connect to system D-Bus: {}", e))?;

        let proxy = connection
            .object_proxy("org.freedesktop.login1", "/org/freedesktop/login1")
            .with_interface::<zbus::zvariant::PropertyStream<()>>(
                "org.freedesktop.login1.Manager",
            );

        // Use the zbus API with typed proxy
        let proxy: zbus::Proxy<'_> = proxy.typed::<(), (RawFd, String)>();
        
        let result = proxy
            .call("Inhibit", &(
                sleep_type_c.to_str().unwrap(),
                mode_c.to_str().unwrap(),
                what_c.to_str().unwrap(),
                description_c.to_str().unwrap()
            ))
            .await;

        match result {
            Ok((fd, cookie)) => {
                let file = unsafe { File::from_raw_fd(fd) };
                info!("Sleep inhibition acquired: {} (cookie: {})", what, cookie);
                
                Ok(Self {
                    fd: Some(file),
                    cookie: Some(cookie),
                    what: what.to_string(),
                    description: description.to_string(),
                })
            }
            Err(e) => {
                warn!("Failed to acquire sleep inhibition: {}", e);
                
                // Check for permission errors
                if let Some(dbus_err) = e.as_dbus_error() {
                    if dbus_err.1.contains("Access denied") {
                        return Err(anyhow!("D-Bus access denied. Add user to 'login' group or run as root."));
                    }
                }
                
                Err(anyhow!("Failed to inhibit sleeping: {}", e))
            }
        }
    }

    pub fn cookie(&self) -> Option<&str> {
        self.cookie.as_deref()
    }

    pub fn what(&self) -> &str {
        &self.what
    }
}

impl Drop for SleepInhibitor {
    fn drop(&mut self) {
        if let Some(ref cookie) = self.cookie {
            info!("Dropping sleep inhibition: {}", cookie);
            // File descriptor is automatically closed when File is dropped
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
        description: &str,
        mode: &str,
    ) -> Result<()> {
        if self.is_inhibited {
            debug!("Already inhibited");
            return Ok(());
        }

        let inhibitor = SleepInhibitor::new(what, description, mode).await?;

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

    pub fn inhibitor(&self) -> Option<&SleepInhibitor> {
        self.inhibitor.as_ref()
    }
}

impl Default for InhibitionState {
    fn default() -> Self {
        Self::new()
    }
}
