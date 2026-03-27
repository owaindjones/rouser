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

    pub async fn collect(&self) -> Result<Vec<GpuData>, GpuError> {
        if !self.has_nvidia && !self.has_amd_intel {
            debug!("No GPU support detected, returning empty list");
            return Ok(vec![]);
        }

        let mut all_gpus: Vec<GpuData> = vec![];

        if self.has_nvidia {
            if let Ok(nvidia_usage) = self.collect_nvidia_all() {
                debug!("Collecting NVIDIA GPUs: usage {:.1}% across {} GPU(s)", nvidia_usage, 1);
                all_gpus.push(GpuData {
                    vendor_type: "NVIDIA",
                    usage: nvidia_usage,
                });
            }
        }

        if self.has_amd_intel {
            if let Ok(amd_intel_usage) = self.collect_sysfs_all() {
                debug!("Collecting AMD/Intel GPU sysfs: usage {:.1}% across detected GPUs", amd_intel_usage);
                all_gpus.push(GpuData {
                    vendor_type: "AMD/Intel",
                    usage: amd_intel_usage,
                });
            } else {
                debug!("AMD/Intel GPU sysfs collection failed or no data available");
            }
        }

        if all_gpus.is_empty() {
            debug!("No valid GPU data collected from all vendors");
        } else {
            debug!("Total GPUs collected: {}", all_gpus.len());
        }

        Ok(all_gpus)
    }

    fn collect_nvidia_all(&self) -> Result<f64, GpuError> {
        debug!("Collecting NVIDIA GPU usage via nvidia-smi");
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
            let line = line.trim();
            // nvidia-smi with --format=csv,noheader,nounits returns just the number
            // Without that flag, it returns "XX%" with the percentage symbol
            let usage_str = line.strip_suffix('%').unwrap_or(line);
            if let Ok(usage) = usage_str.parse::<f64>() {
                total_usage += usage;
                count += 1;
                debug!("Parsed NVIDIA GPU usage: {}% from line: {}", usage, line);
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
        debug!("Collecting AMD/Intel GPU usage via /sys/class/drm");
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

// New struct to hold GPU data with vendor info
#[derive(Debug, Clone)]
pub struct GpuData {
    pub vendor_type: &'static str,
    pub usage: f64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_gpu_collector_creation() {
        let collector = GpuCollector::new();
        let _ = collector; // Just test that we can create it without panicking
    }
}
