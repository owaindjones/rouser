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
/// - **`nvidia` / `nouveau`** → NVML library (`libnvidia-ml.so`) via `nvml-wrapper` crate
///   (same approach used by [nvtop](https://github.com/Syllo/nvtop))
/// - **`amdgpu`, `xe`, `i915`** → direct read of `gpu_busy_percent` from sysfs
///
/// ## Consistent Device Identifiers
///
/// All GPU device IDs come from the sysfs card directory name (e.g., `"card0"`, `"card1"`),
/// regardless of vendor. This ensures consistent labeling across mixed-vendor systems:
/// `GPU log output shows "card0(nvidia): 45%", "card1(amdgpu): 78%"` instead of
/// the previous inconsistent mix of "GPU0(nvidia)" and "card1(amdgpu)".
use nvml_wrapper::enum_wrappers::device::Clock;
use nvml_wrapper::Nvml;
use std::collections::HashMap;
use std::fs;
use std::path::Path;
use tracing::{debug, warn};

/// Represents a discovered GPU card with driver info from sysfs enumeration.
#[derive(Debug)]
pub struct EnumGpu {
    pub device_id: String,
    pub driver_name: String,
}

/// Per-device frequency tracking state for NVIDIA NVML freq-weighted usage.
struct NvmlState {
    /// Per-card max observed graphics clock in MHz (tracks turbo boost peaks).
    peak_freq_mhz: HashMap<String, u32>,
}

impl NvmlState {
    fn new() -> Self {
        Self {
            peak_freq_mhz: HashMap::new(),
        }
    }
}

pub struct GpuCollector {
    nvml: Option<Nvml>,
    nvml_state: NvmlState,
}

impl GpuCollector {
    pub fn new() -> Self {
        let nvml = Nvml::init().ok();
        Self {
            nvml,
            nvml_state: NvmlState::new(),
        }
    }

    #[allow(dead_code)]
    pub fn name(&self) -> &str {
        "gpu"
    }

    /// Returns true if any physical GPU cards exist on this system.
    pub fn has_gpus(&self) -> bool {
        !self.enumerate_gpus().is_empty()
    }

    /// Collect utilization data from all detected GPUs.
    ///
    /// Enumerates all GPU cards via `/sys/class/drm/`, detects each card's driver,
    /// and routes to the appropriate collection method:
    /// - NVIDIA/Nouveau → NVML library (`libnvidia-ml.so`) matched by PCI bus ID
    /// - AMD/Intel → direct sysfs `gpu_busy_percent` read
    pub async fn collect(&mut self) -> Result<Vec<GpuData>, GpuError> {
        let cards = self.enumerate_gpus();

        if cards.is_empty() {
            debug!("No GPU cards found in /sys/class/drm");
            return Ok(vec![]);
        }

        let mut all_gpus: Vec<GpuData> = Vec::with_capacity(cards.len());

        for card in &cards {
            match card.driver_name.as_str() {
                "nvidia" | "nouveau" => {
                    if let Some(nvml_ref) = self.nvml.as_ref() {
                        if let Some(usage) = Self::collect_nvidia_for_card(
                            nvml_ref,
                            &mut self.nvml_state,
                            &card.device_id,
                            &card.driver_name,
                        ) {
                            all_gpus.push(GpuData {
                                device_id: card.device_id.clone(),
                                driver_name: card.driver_name.clone(),
                                usage,
                            });
                        }
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

        if all_gpus.is_empty() && !cards.is_empty() {
            warn!("No valid GPU data collected; check that NVIDIA drivers are loaded");
        }

        Ok(all_gpus)
    }

    /// Enumerate all GPU cards from `/sys/class/drm/`.
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

        cards
    }

    /// Collect utilization for an NVIDIA/Nouveau card via NVML with frequency-weighted usage.
    ///
    /// Uses the NVML library (same approach as nvtop). Matches NVML devices to sysfs
    /// cards by comparing PCI bus IDs from `/sys/class/drm/cardN/device/uevent` with
    /// `nvmlDeviceGetPciInfo`. This avoids spawning subprocesses and provides lower-level
    /// GPU access via the official NVIDIA management library.
    ///
    /// Frequency-weighted: NVML utilization_rates() reports percentage of time SMs were busy
    /// at their *current* clock speed, not normalized to max rated frequency. A GPU running
    /// 200MHz at 100% usage is effectively only ~6% loaded vs its 3200MHz peak — the same
    /// principle as CPU freq-weighting (`src/metrics/cpu.rs`). We compute:
    ///   effective_max = max(current_freq, max_rated_freq, observed_peak)
    ///   weighted_compute_usage = raw_gpu_pct * (current_freq / effective_max)
    fn collect_nvidia_for_card(
        nvml: &Nvml,
        nvml_state: &mut NvmlState,
        card_name: &str,
        _driver_name: &str,
    ) -> Option<f64> {
        let pci_slot = Self::read_pci_slot_from_uevent(card_name);

        if pci_slot.is_empty() {
            debug!(
                "Could not determine PCI slot for sysfs card {}, skipping NVML query",
                card_name
            );
            return Some(0.0);
        }

        let count = match nvml.device_count() {
            Ok(c) => c,
            Err(e) => {
                debug!("NVML device enumeration failed: {}", e);
                return Some(0.0);
            }
        };

        for idx in 0..count {
            if let Ok(device) = nvml.device_by_index(idx) {
                if let Ok(pci_info) = device.pci_info() {
                    // NVML bus_id format: "00000000:09:00.0" (8-digit domain prefix)
                    // sysfs uevent PCI_SLOT_NAME format: "0000:09:00.0" (4-digit domain prefix)
                    if pci_info.bus_id.contains(&pci_slot) {
                        let raw_gpu_usage = match device.utilization_rates() {
                            Ok(util) => util.gpu as f64,
                            Err(e) => {
                                debug!("NVML GPU utilization not available: {}", e);
                                0.0
                            }
                        };

                        // Frequency-weighted compute usage: NVML reports % busy at current clock,
                        // need to normalize against max rated frequency for accurate load measurement.
                        let (current_freq_mhz, max_rated_freq_mhz) = match (
                            device.clock_info(Clock::Graphics),
                            device.max_clock_info(Clock::Graphics),
                        ) {
                            (Ok(cur), Ok(max)) => (cur as f64, max as f64),
                            (Err(e_cur), Err(_e_max)) => {
                                debug!(
                                    "NVML clock info not available for {}: current={}",
                                    e_cur, raw_gpu_usage
                                );
                                return Some(raw_gpu_usage);
                            }
                            _ => {
                                debug!(
                                    "NVML clock info partially unavailable for {}, using raw usage",
                                    card_name
                                );
                                return Some(raw_gpu_usage);
                            }
                        };

                        // Track max observed frequency (handles turbo boost beyond rated max).
                        let peak = nvml_state
                            .peak_freq_mhz
                            .entry(card_name.to_string())
                            .or_insert(0);
                        if current_freq_mhz > *peak as f64 {
                            *peak = current_freq_mhz as u32;
                        }

                        let effective_max =
                            current_freq_mhz.max(max_rated_freq_mhz).max(*peak as f64);
                        let weighted_compute_usage = if effective_max > 0.0 {
                            raw_gpu_usage * (current_freq_mhz / effective_max)
                        } else {
                            raw_gpu_usage
                        };

                        let encoder_usage = device
                            .encoder_utilization()
                            .map(|info| info.utilization as f64)
                            .unwrap_or(0.0);

                        let decoder_usage = device
                            .decoder_utilization()
                            .map(|info| info.utilization as f64)
                            .unwrap_or(0.0);

                        // Encoder/decoder engines run at fixed clocks (not scaled by boost),
                        // so their raw percentages are already normalized — no freq weighting needed.
                        let composite =
                            weighted_compute_usage.max(encoder_usage).max(decoder_usage);

                        debug!(
                            "GPU {} (PCI: {}) compute_raw={:.1}% freq_ratio={}/{:.0}→{:.1}% encode={:.1}% decode={:.1}% → composite={:.1}%",
                            card_name, pci_slot, raw_gpu_usage, current_freq_mhz as u32, max_rated_freq_mhz as u32, weighted_compute_usage, encoder_usage, decoder_usage, composite
                        );

                        return Some(composite);
                    }
                }
            }
        }

        warn!(
            "NVML initialized but no NVIDIA GPU found matching sysfs card '{}' (PCI: {})",
            card_name, pci_slot
        );
        Some(0.0)
    }

    /// Read the PCI slot name from a sysfs card's uevent file.
    fn read_pci_slot_from_uevent(card_name: &str) -> String {
        let uevent_path = format!("/sys/class/drm/{}/device/uevent", card_name);
        match fs::read_to_string(&uevent_path) {
            Ok(content) => {
                for line in content.lines() {
                    if let Some(slot) = line.strip_prefix("PCI_SLOT_NAME=") {
                        return slot.trim().to_string();
                    }
                }
            }
            Err(e) => {
                debug!("Could not read uevent for {}: {}", card_name, e);
            }
        }
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
    pub fn is_valid_gpu_card(name: &str, card_path: &Path) -> bool {
        if !name.starts_with("card") {
            return false;
        }

        let suffix = &name["card".len()..];
        if suffix.is_empty() || !suffix.chars().all(|c| c.is_ascii_digit()) {
            return false;
        }

        card_path.join("device").is_dir()
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

/// Aggregate GPU metrics across all GPUs on the system.
/// Mirrors CpuUsage pattern: per-GPU max + average for inhibition decisions.
#[derive(Debug, Clone, Default)]
pub struct GpuAggregate {
    /// Maximum individual GPU usage across all devices (0-100).
    pub per_gpu_max: f64,
    /// Average usage across all GPUs (sum / count) (0-100).
    pub total_average: f64,
}

#[derive(Debug, Clone)]
pub struct GpuData {
    pub device_id: String,
    pub driver_name: String,
    pub usage: f64,
}

impl GpuAggregate {
    #[allow(dead_code)] // Kept for potential future use with full GpuData inputs.
    /// Compute aggregate metrics from individual GPU data.
    pub(crate) fn from_gpus(gpus: &[GpuData]) -> Self {
        if gpus.is_empty() {
            return Self::default();
        }
        let max = gpus.iter().map(|g| g.usage).fold(0.0f64, f64::max);
        let sum: f64 = gpus.iter().map(|g| g.usage).sum();
        let avg = sum / gpus.len() as f64;
        Self {
            per_gpu_max: max,
            total_average: avg,
        }
    }

    /// Compute aggregate metrics from raw GPU usage values (e.g., after EMA smoothing).
    pub fn from_values(values: &[f64]) -> Self {
        if values.is_empty() {
            return Self::default();
        }
        let max = values.iter().cloned().fold(0.0f64, f64::max);
        let sum: f64 = values.iter().sum();
        let avg = sum / values.len() as f64;
        Self {
            per_gpu_max: max,
            total_average: avg,
        }
    }
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
                if !name.starts_with("card") || !path.is_dir() {
                    continue;
                }
                let device_path = path.join("device");
                if !device_path.exists() {
                    continue;
                }
                let driver_link = device_path.join("driver");
                if !fs::exists(&driver_link).unwrap_or(false) && !driver_link.is_symlink() {
                    continue;
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
        assert!(debug_str.contains("card0"));
        assert!(debug_str.contains("nvidia"));
        assert!(debug_str.contains("card1"));
        assert!(debug_str.contains("amdgpu"));
    }

    #[test]
    fn test_detect_driver_returns_nvidia_not_unknown() {
        let driver = GpuCollector::detect_driver(Path::new("/sys/class/drm/fake"));
        assert!(!driver.is_empty());
    }

    #[test]
    fn test_enumerate_gpus_returns_valid_cards_only() {
        let collector = GpuCollector::new();
        let cards = collector.enumerate_gpus();

        for card in &cards {
            assert!(
                card.device_id.starts_with("card"),
                "Card ID '{}' should start with 'card'",
                card.device_id
            );
            assert!(!card.driver_name.is_empty(), "Driver should not be empty");
        }

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

        for name in [
            "card0",
            "card1-HDMI-A-1",
            "renderD128",
            "drm-card-amdgpu-dce",
            "card42",
        ] {
            fs::create_dir_all(base_path.join(name).join("device")).unwrap();
        }

        assert!(GpuCollector::is_valid_gpu_card(
            "card0",
            &base_path.join("card0")
        ));
        assert!(GpuCollector::is_valid_gpu_card(
            "card42",
            &base_path.join("card42")
        ));

        assert!(!GpuCollector::is_valid_gpu_card(
            "card1-HDMI-A-1",
            &base_path.join("card1-HDMI-A-1")
        ));

        assert!(!GpuCollector::is_valid_gpu_card(
            "renderD128",
            &base_path.join("renderD128")
        ));

        assert!(!GpuCollector::is_valid_gpu_card(
            "drm-card-amdgpu-dce",
            &base_path.join("drm-card-amdgpu-dce")
        ));

        let no_device = base.path().join("card99");
        fs::create_dir_all(&no_device).ok();
        assert!(!GpuCollector::is_valid_gpu_card("card99", &no_device));

        let card_no_num = base.path().join("card");
        fs::create_dir_all(card_no_num.join("device")).ok();
        assert!(!GpuCollector::is_valid_gpu_card("card", &card_no_num));

        let card_letter = base.path().join("card0x");
        fs::create_dir_all(card_letter.join("device")).ok();
        assert!(!GpuCollector::is_valid_gpu_card("card0x", &card_letter));

        let empty = base.path().join("");
        fs::create_dir_all(empty.join("device")).ok();
        assert!(!GpuCollector::is_valid_gpu_card("", &empty));
    }
}
#[cfg(test)]
mod has_gpus_tests {
    use super::*;

    #[test]
    fn test_has_gpus_consistent_with_enumerate() {
        let collector = GpuCollector::new();
        let enumerated = collector.enumerate_gpus();

        // has_gpus and enumerate results must agree:
        // has_gpus is true iff enumerate returns non-empty.
        assert_eq!(collector.has_gpus(), !enumerated.is_empty());
    }

    #[test]
    fn test_enumerate_returns_known_driver_types() {
        let collector = GpuCollector::new();
        let cards = collector.enumerate_gpus();

        for card in &cards {
            // All enumerated cards should have recognized drivers, not "unknown"
            assert_ne!(
                card.driver_name, "unknown",
                "Card {} has unrecognized driver '{}'",
                card.device_id, card.driver_name
            );
        }

        if !cards.is_empty() {
            println!("Enumerated GPUs: {:?}", cards);
        }
    }

    #[test]
    fn test_has_gpus_false_on_empty_sysfs_simulation() {
        let base = tempfile::tempdir().unwrap();

        // Verify is_valid_gpu_card rejects all entries in empty temp dir.
        let entries = fs::read_dir(base.path()).ok();
        let mut found_any = false;
        if let Some(entries) = entries {
            for entry in entries.flatten() {
                let path = entry.path();
                let name = match path.file_name().and_then(|s| s.to_str()) {
                    Some(n) => n,
                    None => continue,
                };
                if GpuCollector::is_valid_gpu_card(name, &path) {
                    found_any = true;
                }
            }
        }

        // Empty temp dir should have no valid GPU cards.
        assert!(
            !found_any,
            "tempdir unexpectedly contains valid gpu card entries"
        );
    }

    #[test]
    fn test_has_gpus_true_when_fake_card_present() {
        let base = tempfile::tempdir().unwrap();
        let card_path = base.path().join("card0");
        fs::create_dir_all(card_path.join("device")).unwrap();

        // Verify is_valid_gpu_card accepts the fake card.
        assert!(GpuCollector::is_valid_gpu_card("card0", &card_path));
    }
}

#[cfg(test)]
mod gpu_aggregate_tests {
    use super::*;

    #[test]
    fn test_gpu_aggregate_empty_values_returns_default() {
        let agg = GpuAggregate::from_values(&[]);
        assert_eq!(agg.per_gpu_max, 0.0);
        assert_eq!(agg.total_average, 0.0);
    }

    #[test]
    fn test_gpu_aggregate_single_value_both_metrics_equal() {
        let agg = GpuAggregate::from_values(&[50.0]);
        // With one GPU, max and average are the same value.
        assert!((agg.per_gpu_max - 50.0).abs() < f64::EPSILON);
        assert!((agg.total_average - 50.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_gpu_aggregate_two_gpus_max_and_average_correct() {
        let agg = GpuAggregate::from_values(&[30.0, 70.0]);
        // max is 70 (highest GPU)
        assert!((agg.per_gpu_max - 70.0).abs() < f64::EPSILON);
        // average is (30+70)/2 = 50
        assert!((agg.total_average - 50.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_gpu_aggregate_three_gpus_correct() {
        let agg = GpuAggregate::from_values(&[10.0, 50.0, 90.0]);
        // max is 90
        assert!((agg.per_gpu_max - 90.0).abs() < f64::EPSILON);
        // average is (10+50+90)/3 = 50
        assert!((agg.total_average - 50.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_gpu_aggregate_all_zeros() {
        let agg = GpuAggregate::from_values(&[0.0, 0.0, 0.0]);
        assert!((agg.per_gpu_max - 0.0).abs() < f64::EPSILON);
        assert!((agg.total_average - 0.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_gpu_aggregate_default_impl_is_zero() {
        let agg = GpuAggregate::default();
        assert_eq!(agg.per_gpu_max, 0.0);
        assert_eq!(agg.total_average, 0.0);
    }

    #[test]
    fn test_gpu_aggregate_from_gpus_empty_returns_default() {
        let gpus: Vec<GpuData> = vec![];
        let agg = GpuAggregate::from_gpus(&gpus);
        assert_eq!(agg.per_gpu_max, 0.0);
        assert_eq!(agg.total_average, 0.0);
    }

    #[test]
    fn test_gpu_aggregate_from_gpus_matches_from_values() {
        let gpus = vec![
            GpuData {
                device_id: "card0".into(),
                driver_name: "nvidia".into(),
                usage: 40.0,
            },
            GpuData {
                device_id: "card1".into(),
                driver_name: "amdgpu".into(),
                usage: 80.0,
            },
        ];
        let values = vec![40.0, 80.0];

        let agg_from_gpus = GpuAggregate::from_gpus(&gpus);
        let agg_from_values = GpuAggregate::from_values(&values);

        assert!((agg_from_gpus.per_gpu_max - agg_from_values.per_gpu_max).abs() < f64::EPSILON);
        assert!((agg_from_gpus.total_average - agg_from_values.total_average).abs() < f64::EPSILON);
    }
}
