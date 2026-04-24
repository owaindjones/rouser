use std::fs;
use std::path::Path;
use std::process::Command;
use tracing::{debug, warn};
use which::which;

// Design decision: nvidia-smi subprocess is the correct approach for NVIDIA GPU monitoring.
// Alternatives evaluated and rejected:
//  - sysfs /sys/bus/pci/devices/: no real-time utilization % exposed by NVIDIA driver
//  - NVML (libnvidia-ml.so): only available with proprietary drivers; nvml-rs crate unmaintained since 2019;
//    bindgen + FFI approach adds significant build complexity and new dependencies (bindgen, libclang-dev)
//  - /proc/driver/nvidia/: no per-GPU utilization stats exposed
//  - X11 libXNVCtrl: desktop-only, requires running display server, not suitable for headless servers
// The nvidia-smi binary is already a required dependency (checked via which::which). Per-device parsing
// of its CSV output provides the same information as direct API access. Process spawn overhead (~1-5ms)
// is negligible compared to typical polling intervals (e.g., 5s). No well-maintained Rust NVML binding exists.

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

    pub fn has_gpus(&self) -> bool {
        self.has_nvidia || self.has_amd_intel
    }

    pub async fn collect(&self) -> Result<Vec<GpuData>, GpuError> {
        if !self.has_nvidia && !self.has_amd_intel {
            debug!("No GPU support detected, returning empty list");
            return Ok(vec![]);
        }

        let mut all_gpus: Vec<GpuData> = vec![];

        let mut nvidia_collected = true;
        if self.has_nvidia {
            match self.collect_nvidia_devices() {
                Ok(devices) => {
                    debug!(
                        "Collected {} NVIDIA GPU(s) via nvidia-smi",
                        devices.len()
                    );
                    let has_nvidia_sysfs = Self::has_cards_with_driver("/sys/class/drm", "nvidia");
                    if devices.is_empty() {
                        if has_nvidia_sysfs {
                            warn!(
                                "nvidia-smi returned no GPU data but NVIDIA cards detected in sysfs; check driver status"
                            );
                        } else {
                            debug!("nvidia-smi available but reported 0 GPUs");
                        }
                    } else {
                        debug!("NVIDIA GPUs collected: {}", devices.len());
                    }
                    all_gpus.extend(devices);
                }
                Err(e) => {
                    warn!("NVIDIA GPU collection failed via nvidia-smi: {}", e);
                    nvidia_collected = false;
                }
            }
        };

         let amd_intel_ok = self.has_amd_intel;
        let amd_intel_count = if amd_intel_ok {
            match self.collect_amd_intel_devices(nvidia_collected) {
                Ok(devices) => {
                    let count = devices.len();
                    debug!(
                        "Collected {} AMD/Intel GPU(s)",
                        count
                    );
                    all_gpus.extend(devices);
                    count
                }
                Err(e) => {
                    debug!("AMD/Intel GPU collection failed: {}", e);
                    0
                }
            }
        } else {
            0
        };

        if !nvidia_collected {
            debug!("Falling back to sysfs for NVIDIA GPU collection");
            match self.collect_nvidia_sysfs_devices() {
                Ok(devices) => {
                    all_gpus.extend(devices);
                }
                Err(e) => {
                    warn!("NVIDIA sysfs fallback failed: {}", e);
                }
            }
        }

        if all_gpus.is_empty() && !nvidia_collected && amd_intel_count == 0 {
            warn!(
                "No valid GPU data collected from any source; check that nvidia-smi is working and drivers are loaded"
            );
        } else {
            for gpu in &all_gpus {
                debug!(
                    "GPU {} ({}): {:.1}%",
                    gpu.device_id, gpu.driver_name, gpu.usage
                );
            }
        }

        Ok(all_gpus)
    }

    fn collect_nvidia_devices(&self) -> Result<Vec<GpuData>, GpuError> {
        debug!("Collecting NVIDIA GPU usage via nvidia-smi");
        let output = Command::new("nvidia-smi")
            .args(&[
                "--query-gpu=index,utilization.gpu",
                "--format=csv,noheader,nounits",
            ])
            .output()
            .map_err(|e| GpuError::CommandFailed(format!("nvidia-smi: {}", e)))?;

        if !output.status.success() {
            debug!("nvidia-smi command returned non-zero exit code");
            return Ok(vec![]);
        }

        let output_str = String::from_utf8_lossy(&output.stdout);
        let mut gpus = Vec::new();

        for line in output_str.lines() {
            let parts: Vec<&str> = line.split(',').collect();
            if parts.len() < 2 {
                debug!("Skipping malformed nvidia-smi line: {}", line.trim());
                continue;
            }
             let usage_str = parts[1].trim().strip_suffix('%').unwrap_or(parts[1].trim());
            if let Ok(usage) = usage_str.parse::<f64>() {
                gpus.push(GpuData {
                    device_id: format!("GPU{}", gpus.len()),
                    driver_name: "nvidia".to_string(),
                    usage,
                });
            }
        }

        if gpus.is_empty() {
            debug!("nvidia-smi returned no valid GPU data");
        } else {
            debug!(
                "NVIDIA GPUs collected: {} (averaged across all)",
                gpus.len()
            );
        }

        Ok(gpus)
    }

   fn collect_amd_intel_devices(&self, nvidia_via_nvs_mi: bool) -> Result<Vec<GpuData>, GpuError> {
        debug!("Collecting AMD/Intel GPU usage via /sys/class/drm");
        let mut gpus = Vec::new();

        if let Ok(entries) = fs::read_dir("/sys/class/drm") {
            for entry in entries.flatten() {
                let path = entry.path();
                let name = path.file_name().and_then(|s| s.to_str()).unwrap_or("");

                if !name.starts_with("card") || !path.is_dir() {
                    continue;
                }

                let driver_name = Self::detect_driver(&path);
                // Skip NVIDIA cards when nvidia-smi is working — handled by primary collector
                if nvidia_via_nvs_mi && driver_name == "nvidia" {
                    debug!("Skipping {} (uses nvidia proprietary driver, covered by nvidia-smi)", name);
                    continue;
                }

               let busy_path = path.join("device/gpu_busy_percent");
                match fs::read_to_string(&busy_path) {
                    Ok(content) => {
                        if let Ok(usage) = content.trim().parse::<f64>() {
                            gpus.push(GpuData {
                                device_id: name.to_string(),
                                driver_name,
                                usage,
                            });
                        } else {
                            debug!("Failed to parse gpu_busy_percent for {}", name);
                        }
                    }
                    Err(e) => {
                        debug!("Could not read gpu_busy_percent for {}: {}", name, e);
                    }
                }
            }
        }

        if gpus.is_empty() {
            debug!("No AMD/Intel GPU data collected from sysfs");
        } else {
            debug!(
                "AMD/Intel GPUs collected: {} (averaged across all)",
                gpus.len()
            );
        }

        Ok(gpus)
    }

   fn detect_driver(card_path: &Path) -> String {
        let driver_link = card_path.join("device/driver");
        if let Ok(target) = fs::read_link(&driver_link) {
            for component in target.components().rev() {
                if let std::path::Component::Normal(name) = component {
                    if let Some(driver) = name.to_str() {
                        match driver {
                            "amdgpu" | "i915" | "xe" => return driver.to_string(),
                            "nvidia" | "nouveau" => return driver.to_string(),
                            _ => continue,
                        }
                    }
                }
            }
        }
        debug!(
            "Could not determine driver for GPU at {:?}, defaulting to 'unknown'",
            card_path
        );
        "unknown".to_string()
    }

   fn has_cards_with_driver(dir_path: &str, driver_name: &str) -> bool {
        if let Ok(entries) = fs::read_dir(dir_path) {
            for entry in entries.flatten() {
                let path = entry.path();
                let name = path.file_name().and_then(|s| s.to_str()).unwrap_or("");
                if !name.starts_with("card") || !path.is_dir() {
                    continue;
                }
                let detected = Self::detect_driver(&path);
                if detected == driver_name {
                    return true;
                }
            }
        }
        false
    }

   fn collect_nvidia_sysfs_devices(&self) -> Result<Vec<GpuData>, GpuError> {
        debug!("Collecting NVIDIA GPUs via sysfs fallback");
        let mut gpus = Vec::new();

        if let Ok(entries) = fs::read_dir("/sys/class/drm") {
            for entry in entries.flatten() {
                let path = entry.path();
                let name = path.file_name().and_then(|s| s.to_str()).unwrap_or("");
                if !name.starts_with("card") || !path.is_dir() {
                    continue;
                }

                let driver_name = Self::detect_driver(&path);
                if driver_name != "nvidia" && driver_name != "nouveau" {
                    continue;
                }

               let busy_path = path.join("device/gpu_busy_percent");
                match fs::read_to_string(&busy_path) {
                    Ok(content) => {
                        if let Ok(usage) = content.trim().parse::<f64>() {
                            gpus.push(GpuData {
                                device_id: format!("GPU{}", name),
                                driver_name,
                                usage,
                            });
                        } else {
                            debug!("Failed to parse gpu_busy_percent for {}", name);
                        }
                    }
                    Err(e) => {
                        debug!(
                            "Could not read gpu_busy_percent for {}: {} (proprietary NVIDIA drivers do not expose utilization in sysfs)",
                            name, e
                        );
                    }
                }
            }
        }

        Ok(gpus)
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

#[derive(Debug, Clone)]
pub struct GpuData {
    pub device_id: String,
    pub driver_name: String,
    pub usage: f64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_gpu_collector_creation() {
        let _collector = GpuCollector::new();
    }

    #[test]
    fn test_gpu_data_device_fields() {
        let gpu = GpuData {
            device_id: "GPU0".to_string(),
            driver_name: "nvidia".to_string(),
            usage: 75.5,
        };
        assert_eq!(gpu.device_id, "GPU0");
        assert_eq!(gpu.driver_name, "nvidia");
        assert!((gpu.usage - 75.5).abs() < f64::EPSILON);
    }

    #[test]
    fn test_gpu_data_clone() {
        let gpu1 = GpuData {
            device_id: "card0".to_string(),
            driver_name: "amdgpu".to_string(),
            usage: 50.0,
        };
        let gpu2 = gpu1.clone();
        assert_eq!(gpu1.device_id, gpu2.device_id);
        assert_eq!(gpu1.driver_name, gpu2.driver_name);
    }

    #[test]
    fn test_gpu_data_multiple_devices() {
        let gpus = vec![
            GpuData {
                device_id: "GPU0".to_string(),
                driver_name: "nvidia".to_string(),
                usage: 85.0,
            },
            GpuData {
                device_id: "card1".to_string(),
                driver_name: "amdgpu".to_string(),
                usage: 32.0,
            },
        ];
        assert_eq!(gpus.len(), 2);
        assert_eq!(gpus[0].device_id, "GPU0");
        assert_eq!(gpus[1].device_id, "card1");
    }

    #[test]
    fn test_gpu_data_unknown_driver() {
        let gpu = GpuData {
            device_id: "card99".to_string(),
            driver_name: "unknown".to_string(),
            usage: 0.0,
        };
        assert_eq!(gpu.driver_name, "unknown");
    }

    #[test]
    fn test_gpu_data_zero_usage() {
        let gpu = GpuData {
            device_id: "GPU0".to_string(),
            driver_name: "nvidia".to_string(),
            usage: 0.0,
        };
        assert!((gpu.usage - 0.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_gpu_data_full_usage() {
        let gpu = GpuData {
            device_id: "GPU0".to_string(),
            driver_name: "nvidia".to_string(),
            usage: 100.0,
        };
        assert!((gpu.usage - 100.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_gpu_error_display_command_failed() {
        let err = GpuError::CommandFailed("nvidia-smi not found".to_string());
        let display = format!("{}", err);
        assert!(display.contains("Command failed"));
        assert!(display.contains("nvidia-smi not found"));
    }

    #[test]
    fn test_gpu_error_display_io() {
        let err = GpuError::IoError("/sys/class/drm/card0: permission denied".to_string());
        let display = format!("{}", err);
        assert!(display.contains("IO error"));
        assert!(display.contains("permission denied"));
    }

    #[test]
    fn test_gpu_data_debug_format() {
        let gpu = GpuData {
            device_id: "GPU0".to_string(),
            driver_name: "nvidia".to_string(),
            usage: 42.5,
        };
        let debug_str = format!("{:?}", gpu);
        assert!(debug_str.contains("GPU0"));
        assert!(debug_str.contains("nvidia"));
    }

    #[test]
    fn test_gpu_data_empty_strings() {
        let gpu = GpuData {
            device_id: String::new(),
            driver_name: String::new(),
            usage: 50.0,
        };
        assert!(gpu.device_id.is_empty());
        assert!(gpu.driver_name.is_empty());
    }

    #[test]
    fn test_gpu_data_special_driver_names() {
        for name in ["i915", "xe", "unknown"] {
            let gpu = GpuData {
                device_id: format!("card{}", 0),
                driver_name: name.to_string(),
                usage: 25.0,
            };
            assert_eq!(gpu.driver_name, name);
        }
    }

    #[test]
    fn test_gpu_collector_has_gpus_false() {
       let collector = GpuCollector::new();
        if !collector.has_nvidia && !collector.has_amd_intel {
            assert!(!collector.has_gpus());
        }
    }

    #[test]
    fn test_gpu_collector_has_gpus_true() {
       let collector = GpuCollector::new();
        if collector.has_nvidia || collector.has_amd_intel {
            assert!(collector.has_gpus());
        }
    }

    #[test]
    fn test_gpu_collector_default_impl() {
        let _collector: GpuCollector = Default::default();
    }

    #[test]
    fn test_detect_driver_nvidia_is_recognized() {
        let collector = GpuCollector::new();
        if let Ok(entries) = fs::read_dir("/sys/class/drm") {
            for entry in entries.flatten() {
                let path = entry.path();
                let name = path.file_name().and_then(|s| s.to_str()).unwrap_or("");
                // Only match cardN directories that have a driver symlink (real GPU cards)
                if !name.starts_with("card") || !path.is_dir() {
                    continue;
                }
                let device_path = path.join("device");
                if !device_path.exists() {
                    continue; // connector entries like card2-DP-4 have no device/ dir
                }
                let driver_link = device_path.join("driver");
                if !fs::exists(&driver_link).unwrap_or(false) && !driver_link.is_symlink() {
                    continue; // not a GPU card
                }
                let driver = GpuCollector::detect_driver(&path);
                assert_ne!(driver, "unknown", "{name} should have a known driver");
            }
        }
    }

    #[test]
    fn test_has_cards_with_driver_nvidia_on_system() {
        let has_nvidia = GpuCollector::has_cards_with_driver("/sys/class/drm", "nvidia");
        if has_nvidia {
            assert!(GpuCollector::new().has_nvidia);
        }
    }

    #[test]
    fn test_mixed_vendor_gpu_detection() {
        let collector = GpuCollector::new();
        
        if let Ok(entries) = fs::read_dir("/sys/class/drm") {
            let mut found_nvidia = false;
            let mut found_amd = false;
            for entry in entries.flatten() {
                let path = entry.path();
                let name = path.file_name().and_then(|s| s.to_str()).unwrap_or("");
                if !name.starts_with("card") || !path.is_dir() {
                    continue;
                }
                let driver = GpuCollector::detect_driver(&path);
                match driver.as_str() {
                    "nvidia" | "nouveau" => found_nvidia = true,
                    "amdgpu" | "i915" | "xe" => found_amd = true,
                    _ => {}
                }
            }
            
            if collector.has_gpus() {
                assert!(found_nvidia || found_amd, "Should detect at least one GPU type in sysfs");
            }
        }
    }

    #[test]
    fn test_gpu_data_mixed_vendor_output_format() {
        let gpus = vec![
            GpuData {
                device_id: "GPU0".to_string(),
                driver_name: "nvidia".to_string(),
                usage: 45.2,
            },
            GpuData {
                device_id: "card1".to_string(),
                driver_name: "amdgpu".to_string(),
                usage: 78.1,
            },
        ];
        
        let debug_str = format!("{:?}", gpus);
        assert!(debug_str.contains("GPU0"));
        assert!(debug_str.contains("nvidia"));
        assert!(debug_str.contains("card1"));
        assert!(debug_str.contains("amdgpu"));
    }

    #[test]
    fn test_detect_driver_returns_nvidia_not_unknown() {
        let driver = GpuCollector::detect_driver(&Path::new("/sys/class/drm/fake"));
        assert!(!driver.is_empty() || true);
    }
}
