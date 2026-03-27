use anyhow::{Result, anyhow};
use std::fs::File;
use std::os::fd::FromRawFd;
use std::os::unix::io::RawFd;
use tracing::{debug, info};


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
        let sleep_type = "sleep";
        
        let connection = zbus::Connection::system()
            .await
            .map_err(|e| anyhow!("Failed to connect to system D-Bus: {}", e))?;

        let proxy = zbus::Proxy::new(&connection, 
            "org.freedesktop.login1", 
            "/org/freedesktop/login1", 
            "org.freedesktop.login1.Manager")
            .await
            .map_err(|e| anyhow!("Failed to create proxy: {}", e))?;
    
        // Inhibit returns (reserved1, reserved2, reserved3, fd, cookie)
        // where fd is the int32 file descriptor
        let result: (u32, u32, u32, i32, String) = proxy
            .call("Inhibit", &(sleep_type, mode, what, description))
            .await
            .map_err(|e| anyhow!("Failed to call Inhibit: {}", e))?;
        
        let (_r1, _r2, _r3, fd, cookie) = result;
        Ok(Self {
            fd: Some(unsafe { File::from_raw_fd(fd as RawFd) }),
            cookie: Some(cookie),
            what: what.to_string(),
            description: description.to_string(),
        })
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
