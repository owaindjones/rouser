use std::fs;
use std::path::Path;
use tracing::{debug, warn};
use which::which;

#[derive(Debug, Clone)]
pub struct GpuStats {
    pub usage: f64,
    pub vendor: GpuVendor,
}

#[derive(Debug, Clone)]
pub enum GpuVendor {
    Nvidia,
    Amdgpu,
    Intel,
    Unknown,
}

pub struct GpuCollector {
    vendor: Option<GpuVendor>,
}

impl GpuCollector {
    pub fn new() -> Self {
        let vendor = Self::detect_gpu();
        Self { vendor }
    }

    fn detect_gpu() -> Option<GpuVendor> {
        // Check for NVIDIA first
        if which("nvidia-smi").is_ok() {
            debug!("Detected NVIDIA GPU (nvidia-smi available)");
            return Some(GpuVendor::Nvidia);
        }

        // Check for AMD/Intel
        if Path::new("/sys/class/drm").exists() {
            // Try to distinguish AMD vs Intel
            if Path::new("/sys/class/drm/card0/device/uvm").exists() {
                debug!("Detected AMD/Intel GPU found (uvm present)");
                // Heuristic: if /sys/class/kfd/kfd exists, it's more likely AMD
                if Path::new("/sys/class/kfd/kfd").exists() {
                    Some(GpuVendor::Amdgpu)
                } else {
                    Some(GpuVendor::Intel)
                }
            } else {
                Some(GpuVendor::Unknown)
            }
        } else {
            None
        }
    }

    pub async fn collect(&self) -> Result<f64, GpuError> {
        debug!("Collecting GPU usage (vendor: {:?})", self.vendor);

        match self.vendor {
            Some(GpuVendor::Nvidia) => self.collect_nvidia(),
            Some(GpuVendor::Amdgpu | GpuVendor::Intel) => self.collect_sysfs(),
            _ => {
                debug!("No GPU support detected, returning 0%");
                Ok(0.0)
            }
        }
        .map(|usage| usage)
    }

    fn collect_nvidia(&self) -> Result<f64, GpuError> {
        use std::process::Command;

        let output = Command::new("nvidia-smi")
            .args(&["--query-gpu=utilization.gpu", "--format=csv,noheader,nounits"])
            .output()
            .map_err(|e| GpuError::CommandFailed(format!("nvidia-smi: {}", e)))?;

        if !output.status.success() {
            return Ok(0.0);
        }

        let output_str = String::from_utf8_lossy(&output.stdout);
        let mut total_usage: f64 = 0.0;
        let mut count: u32 = 0;

        for line in output_str.lines() {
            if let Some(usage_str) = line.trim().strip_suffix('%') {
                if let Ok(usage) = usage_str.parse::<f64>() {
                    total_usage += usage;
                    count += 1;
                }
            }
        }

        if count == 0 {
            debug!("nvidia-smi returned no valid GPU usage");
            return Ok(0.0);
        }

        let avg = total_usage / count as f64;
        debug!("NVIDIA GPU usage: {:.1}% ({} GPU(s))", avg, count);
        Ok(avg)
    }

    fn collect_sysfs(&self) -> Result<f64, GpuError> {
        let mut total_usage: f64 = 0.0;
        let mut count: u32 = 0;

        if let Ok(entries) = fs::read_dir("/sys/class/drm") {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() && path.to_string_lossy().contains("card") {
                    let busy_path = path.join("device/gpu_busy_percent");
                    if let Ok(content) = fs::read_to_string(&busy_path) {
                        if let Ok(usage) = content.trim().parse::<f64>() {
                            total_usage += usage;
                            count += 1;
                        }
                    }
                }
            }
        }

        if count == 0 {
            debug!("No GPU sysfs entries found");
            return Ok(0.0);
        }

        let avg = total_usage / count as f64;
        debug!("AMD/Intel GPU usage: {:.1}% ({} GPU(s))", avg, count);
        Ok(avg)
    }
}

impl Default for GpuCollector {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug)]
pub enum GpuError {
    CommandFailed(String),
    IoError(String),
}

impl std::fmt::Display for GpuError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            GpuError::CommandFailed(e) => write!(f, "Command failed: {}", e),
            GpuError::IoError(e) => write!(f, "IO error: {}", e),
        }
    }
}

impl std::error::Error for GpuError {}
