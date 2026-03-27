use std::fs;
use std::path::Path;
use std::process::Command;
use tracing::debug;
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
    has_nvidia: bool,
    has_amd_intel: bool,
}

impl GpuCollector {
    pub fn new() -> Self {
        let (has_nvidia, has_amd_intel) = Self::detect_gpus();
        debug!(
            "GPU detection complete - NVIDIA: {}, AMD/Intel: {}",
            has_nvidia, has_amd_intel
        );
        Self {
            has_nvidia,
            has_amd_intel,
        }
    }

    fn detect_gpus() -> (bool, bool) {
        let has_nvidia = which("nvidia-smi").is_ok();
        let has_amd_intel = Path::new("/sys/class/drm").exists();
        (has_nvidia, has_amd_intel)
    }

    pub async fn collect(&self) -> Result<f64, GpuError> {
        if !self.has_nvidia && !self.has_amd_intel {
            debug!("No GPU support detected, returning 0%");
            return Ok(0.0);
        }

        let mut total_usage: f64 = 0.0;
        let mut gpu_count: f64 = 0.0;

        if self.has_nvidia {
            if let Ok(usage) = self.collect_nvidia_all() {
                total_usage += usage;
                gpu_count += 1.0;
            }
        }

        if self.has_amd_intel {
            if let Ok(usage) = self.collect_sysfs_all() {
                total_usage += usage;
                gpu_count += 1.0;
            }
        }

        if gpu_count == 0.0 {
            debug!("No GPU entries found across all vendors");
            return Ok(0.0);
        }

        let avg = total_usage / gpu_count;
        debug!(
            "All GPU usage: {:.1}% ({} vendor types)",
            avg, gpu_count as u32
        );
        Ok(avg)
    }

    fn collect_nvidia_all(&self) -> Result<f64, GpuError> {
        let output = Command::new("nvidia-smi")
            .args(&["--query-gpu=utilization.gpu", "--format=csv,noheader,nounits"])
            .output()
            .map_err(|e| GpuError::CommandFailed(format!("nvidia-smi: {}", e)))?;

        if !output.status.success() {
            debug!("nvidia-smi command failed");
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

  fn collect_sysfs_all(&self) -> Result<f64, GpuError> {
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
            debug!("No AMD/Intel GPU sysfs entries found");
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_gpu_collector_creation() {
        let collector = GpuCollector::new();
        let _ = collector; // Just test that we can create it without panicking
    }

    #[test]
    fn test_gpu_vendor_display() {
        use std::fmt::Write;
        
        let mut output = String::new();
        write!(&mut output, "{:?}", GpuVendor::Nvidia).unwrap();
        assert!(output.contains("Nvidia"));
        
        output.clear();
        write!(&mut output, "{:?}", GpuVendor::Amdgpu).unwrap();
        assert!(output.contains("Amdgpu"));
        
        output.clear();
        write!(&mut output, "{:?}", GpuVendor::Intel).unwrap();
        assert!(output.contains("Intel"));
    }

    #[test]
    fn test_gpu_error_display() {
        let err = GpuError::CommandFailed("test error".to_string());
        let display = format!("{}", err);
        assert!(display.contains("test error"));
    }
}
