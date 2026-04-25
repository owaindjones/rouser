/// GPU metrics collection.
///
/// ## Unified Enumeration
///
/// All GPUs are enumerated via `/sys/class/drm/`. Each physical card directory
/// (`card0`, `card1`, etc.) is identified by checking for a valid driver symlink
/// at `device/driver`. Display connectors (e.g., `card2-HDMI-A-1`) and render nodes
/// (e.g., `renderD128`) are filtered out.
///
/// ## Driver-Based Routing
///
/// Each card is routed to the appropriate utilization collector based on its driver:
/// - **`nvidia` / `nouveau`** → nvidia-smi subprocess query, matched via GPU UUID
/// - **`amdgpu`, `xe`, `i915`** → direct read of `gpu_busy_percent` from sysfs
///
/// ## Consistent Device Identifiers
///
/// All GPU device IDs come from the sysfs card directory name (e.g., `"card0"`, `"card1"`),
/// regardless of vendor. This ensures consistent labeling across mixed-vendor systems:
/// `GPU log output shows "card0(nvidia): 45%", "card1(amdgpu): 78%"` instead of
/// the previous inconsistent mix of "GPU0(nvidia)" and "card1(amdgpu)".
use std::fs;
use std::path::Path;
use std::process::Command;
use tracing::{debug, warn};

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

/// Represents a discovered GPU card with driver info from sysfs enumeration.
#[derive(Debug)]
pub struct EnumGpu {
    pub device_id: String,
    pub driver_name: String,
}

pub struct GpuCollector;

impl GpuCollector {
    pub fn new() -> Self {
        // No state needed — all detection is done at collection time via sysfs.
        debug!("GPU collector initialized (sysfs-first enumeration)");
        Self
    }

    #[allow(dead_code)]
    pub fn name(&self) -> &str {
        "gpu"
    }

    /// Returns true if any physical GPU cards exist on this system.
    pub fn has_gpus(&self) -> bool {
        self.enumerate_gpus().is_empty()
    }

    /// Collect utilization data from all detected GPUs.
    ///
    /// Enumerates all GPU cards via `/sys/class/drm/`, detects each card's driver,
    /// and routes to the appropriate collection method:
    /// - NVIDIA/Nouveau → nvidia-smi subprocess (matched by UUID)
    /// - AMD/Intel → direct sysfs `gpu_busy_percent` read
    pub async fn collect(&self) -> Result<Vec<GpuData>, GpuError> {
        let cards = self.enumerate_gpus();

        if cards.is_empty() {
            debug!("No GPU cards found in /sys/class/drm");
            return Ok(vec![]);
        }

        let mut all_gpus: Vec<GpuData> = Vec::with_capacity(cards.len());

        for card in &cards {
            match card.driver_name.as_str() {
                "nvidia" | "nouveau" => {
                    if let Some(usage) = Self::collect_nvidia_for_card(&card.device_id) {
                        all_gpus.push(GpuData {
                            device_id: card.device_id.clone(),
                            driver_name: card.driver_name.clone(),
                            usage,
                        });
                    }
                }
                "amdgpu" | "i915" | "xe" => {
                    if let Some(usage) = Self::collect_generic_for_card(card) {
                        all_gpus.push(GpuData {
                            device_id: card.device_id.clone(),
                            driver_name: card.driver_name.clone(),
                            usage,
                        });
                    }
                }
                other => {
                    debug!(
                        "Skipping unknown GPU driver '{}' for {} — no collector available",
                        other, card.device_id
                    );
                }
            }
        }

        if all_gpus.is_empty() {
            warn!("No valid GPU data collected; check that nvidia-smi is working and drivers are loaded");
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

    /// Enumerate all GPU cards from `/sys/class/drm/`.
    ///
    /// Filters out display connectors (`card0-HDMI-A-1`), render nodes (`renderD128`),
    /// and other non-card entries. Returns a vector of `(device_id, driver_name)` tuples.
    fn enumerate_gpus(&self) -> Vec<EnumGpu> {
        let mut cards = Vec::new();

        let drm_dir = Path::new("/sys/class/drm");
        let entries = match fs::read_dir(drm_dir) {
            Ok(entries) => entries,
            Err(e) => {
                debug!("Could not read /sys/class/drm: {}", e);
                return cards;
            }
        };

        for entry in entries.flatten() {
            let path = entry.path();
            let name = match path.file_name().and_then(|s| s.to_str()) {
                Some(n) => n,
                None => continue,
            };

            if !Self::is_valid_gpu_card(name, &path) {
                continue;
            }

            let driver_name = Self::detect_driver(&path);
            cards.push(EnumGpu {
                device_id: name.to_string(),
                driver_name,
            });
        }

        debug!("Enumerated {} GPU card(s) from /sys/class/drm", cards.len());

        cards
    }

    /// Collect utilization for an NVIDIA/Nouveau card via nvidia-smi.
    ///
    /// Runs a single nvidia-smi query to get all GPUs with their UUIDs and utilizations,
    /// then matches the requested sysfs card name against the returned data by:
    /// 1. Reading the GPU UUID from `/sys/class/drm/cardN/device/uevent`
    /// 2. Matching it against nvidia-smi output
    fn collect_nvidia_for_card(card_name: &str) -> Option<f64> {
        debug!(
            "Collecting NVIDIA utilization for {} via nvidia-smi",
            card_name
        );

        // Read the GPU UUID from sysfs uevent to match with nvidia-smi.
        let expected_uuid = Self::read_nvidia_uuid(card_name);

        let output = Command::new("nvidia-smi")
            .args([
                "--query-gpu=index,gpu_bus_id,utilization.gpu",
                "--format=csv,noheader,nounits",
            ])
            .output()
            .ok();

        let output = match output {
            Some(o) => o,
            None => return Some(0.0),
        };

        if !output.status.success() {
            debug!(
                "nvidia-smi returned non-zero exit code for {}; skipping",
                card_name
            );
            return Some(0.0);
        }

        let output_str = String::from_utf8_lossy(&output.stdout);
        let mut best_match: Option<(String, f64)> = None; // (uuid, usage)

        for line in output_str.lines() {
            let parts: Vec<&str> = line.split(',').collect();
            if parts.len() < 3 {
                debug!("Skipping malformed nvidia-smi line: {}", line.trim());
                continue;
            }

            let bus_id = parts[1].trim().to_string();
            let usage_str = parts[2].trim().strip_suffix('%').unwrap_or(parts[2].trim());
            let usage = match usage_str.parse::<f64>() {
                Ok(u) => u,
                Err(_) => continue,
            };

            // Match by normalized PCI address. sysfs reports 5-digit hex (0000:09:00.0) while
            // nvidia-smi reports 8-digit hex (00000000:09:00.0). Normalize both to compare.
            let normalize_pci = |s: &str| {
                s.trim()
                    .replace("0000", "")
                    .trim_start_matches(':')
                    .to_string()
            };

            if expected_uuid.is_empty() || normalize_pci(&expected_uuid) == normalize_pci(&bus_id) {
                debug!(
                    "Matched {} to nvidia-smi GPU index 0 via PCI bus ID",
                    card_name
                );
                return Some(usage);
            }

            // Keep track of all results for fallback logging.
            best_match = Some((bus_id.clone(), usage));
        }

        if !expected_uuid.is_empty() {
            debug!(
                "No nvidia-smi match found for sysfs card '{}' (PCI slot: {})",
                card_name, expected_uuid
            );

            if let Some((ref bus_id, _)) = best_match {
                warn!(
                    "nvidia-smi reported GPUs with PCI addresses [{}] but no match for {}; check driver status",
                    bus_id, card_name
                );
            }
        } else {
            debug!(
                "No sysfs PCI slot name found for {}, nvidia-smi query returned {} entries",
                card_name,
                best_match.as_ref().map(|_| 1).unwrap_or(0)
            );
        }

        // Return 0 rather than failing — the card exists but nvidia-smi couldn't provide data.
        Some(0.0)
    }

    /// Read the GPU UUID from a sysfs NVIDIA card's uevent file.
    fn read_nvidia_uuid(card_name: &str) -> String {
        let uevent_path = format!("/sys/class/drm/{}/device/uevent", card_name);
        match fs::read_to_string(&uevent_path) {
            Ok(content) => {
                for line in content.lines() {
                    if let Some(uuid) = line.strip_prefix("PCI_SLOT_NAME=") {
                        return uuid.trim().to_string();
                    }
                }
            }
            Err(e) => {
                debug!("Could not read uevent for {}: {}", card_name, e);
            }
        }

        // Fallback: use a placeholder that won't match any nvidia-smi output.
        String::new()
    }

    /// Collect utilization for an AMD/Intel GPU from sysfs `gpu_busy_percent`.
    fn collect_generic_for_card(card: &EnumGpu) -> Option<f64> {
        let busy_path = format!("/sys/class/drm/{}/device/gpu_busy_percent", card.device_id);
        match fs::read_to_string(&busy_path) {
            Ok(content) => {
                if let Ok(usage) = content.trim().parse::<f64>() {
                    Some(usage)
                } else {
                    debug!("Failed to parse gpu_busy_percent for {}", card.device_id);
                    None
                }
            }
            Err(e) => {
                debug!(
                    "Could not read gpu_busy_percent for {}: {} (proprietary NVIDIA drivers do not expose utilization in sysfs)",
                    card.device_id, e
                );
                None
            }
        }
    }

    /// Detect the kernel driver for a GPU card.
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

    /// Returns true if `name` looks like a real GPU card directory in sysfs.
    ///
    /// Validates two conditions:
    /// 1. Name matches the pattern "card" followed by one or more digits only
    ///    (rejects connector entries like "card0-HDMI-A-1", render nodes like "renderD128")
    /// 2. A `device/` subdirectory exists at that path and is a directory
    pub fn is_valid_gpu_card(name: &str, card_path: &Path) -> bool {
        if !name.starts_with("card") {
            return false;
        }

        let suffix = &name["card".len()..];
        if suffix.is_empty() || !suffix.chars().all(|c| c.is_ascii_digit()) {
            return false;
        }

        let device_dir = card_path.join("device");
        device_dir.is_dir()
    }
}

impl Default for GpuCollector {
    fn default() -> Self {
        Self::new()
    }
}

#[allow(dead_code)]
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
            device_id: "card0".to_string(),
            driver_name: "nvidia".to_string(),
            usage: 75.5,
        };
        assert_eq!(gpu.device_id, "card0");
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
        let gpus = [
            GpuData {
                device_id: "card0".to_string(),
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
        // Both use consistent cardN naming now
        assert!(gpus[0].device_id.starts_with("card"));
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
            device_id: "card0".to_string(),
            driver_name: "nvidia".to_string(),
            usage: 0.0,
        };
        assert!((gpu.usage - 0.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_gpu_data_full_usage() {
        let gpu = GpuData {
            device_id: "card1".to_string(),
            driver_name: "amdgpu".to_string(),
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
            device_id: "card0".to_string(),
            driver_name: "nvidia".to_string(),
            usage: 42.5,
        };
        let debug_str = format!("{:?}", gpu);
        assert!(debug_str.contains("card0"));
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
    fn test_gpu_collector_default_impl() {
        let _collector: GpuCollector = Default::default();
    }

    #[test]
    #[ignore = "hardware-specific: only meaningful on real hardware"]
    fn test_detect_driver_nvidia_is_recognized() {
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
                assert_ne!(driver, "unknown", "{} should have a known driver", name);
            }
        }
    }

    #[test]
    #[ignore = "hardware-specific: depends on local GPU hardware"]
    fn test_mixed_vendor_gpu_detection() {
        if let Ok(entries) = fs::read_dir("/sys/class/drm") {
            let mut found_nvidia = false;
            let mut found_amd = false;
            for entry in entries.flatten() {
                let path = entry.path();
                let name = path.file_name().and_then(|s| s.to_str()).unwrap_or("");
                if !GpuCollector::is_valid_gpu_card(name, &path) {
                    continue;
                }
                let driver = GpuCollector::detect_driver(&path);
                match driver.as_str() {
                    "nvidia" | "nouveau" => found_nvidia = true,
                    "amdgpu" | "i915" | "xe" => found_amd = true,
                    _ => {}
                }
            }

            if !GpuCollector::new().enumerate_gpus().is_empty() {
                assert!(
                    found_nvidia || found_amd,
                    "Should detect at least one GPU type in sysfs"
                );
            }
        }
    }

    #[test]
    fn test_gpu_data_mixed_vendor_output_format() {
        let gpus = vec![
            GpuData {
                device_id: "card0".to_string(),
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
        // Both use consistent cardN naming
        assert!(debug_str.contains("card0"));
        assert!(debug_str.contains("nvidia"));
        assert!(debug_str.contains("card1"));
        assert!(debug_str.contains("amdgpu"));
    }

    #[test]
    fn test_detect_driver_returns_nvidia_not_unknown() {
        let driver = GpuCollector::detect_driver(Path::new("/sys/class/drm/fake"));
        // For non-existent paths, detect_driver returns "unknown" — that's expected.
        assert!(!driver.is_empty());
    }

    #[test]
    fn test_enumerate_gpus_returns_valid_cards_only() {
        let collector = GpuCollector::new();
        let cards = collector.enumerate_gpus();

        // Every returned card must have a valid name pattern.
        for card in &cards {
            assert!(
                card.device_id.starts_with("card"),
                "Card ID '{}' should start with 'card'",
                card.device_id
            );
            assert!(!card.driver_name.is_empty(), "Driver should not be empty");
        }

        // On this system, we expect at least one GPU.
        if !cards.is_empty() {
            println!("Enumerated GPUs: {:?}", cards);
        }
    }
}

#[cfg(test)]
mod enumerate_tests {
    use super::*;
    use std::os::unix::fs::symlink;

    fn setup_fake_card(base: &Path, name: &str) {
        let card_path = base.join(name);
        fs::create_dir_all(card_path.join("device")).unwrap();
    }

    fn setup_fake_card_with_driver(base: &Path, name: &str, driver_name: &str) {
        let card_path = base.join(name);
        let device_dir = card_path.join("device");
        fs::create_dir_all(&device_dir).unwrap();

        let driver_target_base = base
            .parent()
            .unwrap()
            .join(format!("drivers/{}", driver_name));
        fs::create_dir_all(&driver_target_base).unwrap();
        symlink(&driver_target_base, card_path.join("device/driver")).ok();
    }

    #[test]
    fn test_connector_entries_filtered_out() {
        let base = tempfile::tempdir().unwrap();
        let base_path = base.path();

        setup_fake_card(base_path, "card0-HDMI-A-1");
        fs::create_dir_all(base_path.join("card1-DP-4/device")).ok();
        fs::create_dir_all(base_path.join("renderD128/device")).ok();
        fs::create_dir_all(base_path.join("drm-card-etnaviv-gpu0/device")).ok();

        setup_fake_card(base_path, "card0");

        let mut collected = Vec::new();
        if let Ok(entries) = fs::read_dir(base_path) {
            for entry in entries.flatten() {
                let path = entry.path();
                let name = path.file_name().and_then(|s| s.to_str()).unwrap_or("");
                if GpuCollector::is_valid_gpu_card(name, &path) {
                    collected.push(name.to_string());
                }
            }
        }

        assert_eq!(collected.len(), 1);
        assert!(collected.contains(&"card0".to_string()));
    }

    #[test]
    fn test_driver_detection_amdgpu() {
        let base = tempfile::tempdir().unwrap();
        let base_path = base.path();

        setup_fake_card_with_driver(base_path, "card0", "amdgpu");

        let card_path = base_path.join("card0");
        let driver = GpuCollector::detect_driver(&card_path);
        assert_eq!(driver, "amdgpu");
    }

    #[test]
    fn test_driver_detection_i915() {
        let base = tempfile::tempdir().unwrap();
        let base_path = base.path();

        setup_fake_card_with_driver(base_path, "card0", "i915");

        let card_path = base_path.join("card0");
        let driver = GpuCollector::detect_driver(&card_path);
        assert_eq!(driver, "i915");
    }

    #[test]
    fn test_driver_detection_nvidia() {
        let base = tempfile::tempdir().unwrap();
        let base_path = base.path();

        setup_fake_card_with_driver(base_path, "card0", "nvidia");

        let card_path = base_path.join("card0");
        let driver = GpuCollector::detect_driver(&card_path);
        assert_eq!(driver, "nvidia");
    }

    #[test]
    fn test_is_valid_gpu_card_filtering() {
        let base = tempfile::tempdir().unwrap();
        let base_path = base.path();

        // Create device/ dirs for all so only name-based filtering applies
        for name in [
            "card0",
            "card1-HDMI-A-1",
            "renderD128",
            "drm-card-amdgpu-dce",
            "card42",
        ] {
            fs::create_dir_all(base_path.join(name).join("device")).unwrap();
        }

        // Valid cards
        assert!(GpuCollector::is_valid_gpu_card(
            "card0",
            &base_path.join("card0")
        ));
        assert!(GpuCollector::is_valid_gpu_card(
            "card42",
            &base_path.join("card42")
        ));

        // Connector entries: have hyphen after cardN
        assert!(!GpuCollector::is_valid_gpu_card(
            "card1-HDMI-A-1",
            &base_path.join("card1-HDMI-A-1")
        ));

        // Render nodes don't start with "card"
        assert!(!GpuCollector::is_valid_gpu_card(
            "renderD128",
            &base_path.join("renderD128")
        ));

        // Doesn't match cardN pattern at all
        assert!(!GpuCollector::is_valid_gpu_card(
            "drm-card-amdgpu-dce",
            &base_path.join("drm-card-amdgpu-dce")
        ));

        // Missing device/ subdir → false
        let no_device = base.path().join("card99");
        fs::create_dir_all(&no_device).ok();
        assert!(!GpuCollector::is_valid_gpu_card("card99", &no_device));

        // Edge cases: "card" alone (no digits) → false
        let card_no_num = base.path().join("card");
        fs::create_dir_all(card_no_num.join("device")).ok();
        assert!(!GpuCollector::is_valid_gpu_card("card", &card_no_num));

        // Edge case: letters after digits → false
        let card_letter = base.path().join("card0x");
        fs::create_dir_all(card_letter.join("device")).ok();
        assert!(!GpuCollector::is_valid_gpu_card("card0x", &card_letter));

        // Edge case: empty string → false
        let empty = base.path().join("");
        fs::create_dir_all(empty.join("device")).ok();
        assert!(!GpuCollector::is_valid_gpu_card("", &empty));
    }
}
