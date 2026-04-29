pub mod cpu;
pub mod disk;
pub mod gpu;
pub mod network;

use std::collections::HashMap;
use std::fmt;

pub use cpu::{CpuCollector, CpuUsage};
pub use disk::{DiskCollector, DiskThroughput};
pub use gpu::{GpuCollector, GpuData};
pub use network::{NetworkCollector, NetworkThroughput};

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct Metrics {
    pub cpu_usage: CpuUsage,
    pub gpu_usage: Vec<GpuData>,
    pub network_io: f64,
    pub disk_activity: f64,
}

#[derive(Debug)]
pub struct TickMetrics {
    pub cpu_usage: CpuUsage,
    pub gpu_usage: Vec<GpuData>,
    pub network_throughput: NetworkThroughput,
    pub disk_throughput: DiskThroughput,
}

#[derive(Debug, Clone)]
pub struct GpuDisplayEntry {
    pub device_id: String,
    pub driver_name: String,
    pub raw_usage: f64,
    pub smoothed_usage: f64,
}

impl fmt::Display for GpuDisplayEntry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}({}): {:.1}% (smoothed: {:.1}%)",
            self.device_id, self.driver_name, self.raw_usage, self.smoothed_usage)
    }
}

pub fn sorted_gpu_display(
    gpu_data: &[GpuData],
    smoothed_values: &[f64],
) -> Vec<GpuDisplayEntry> {
    let mut entries: Vec<GpuDisplayEntry> = gpu_data
        .iter()
        .enumerate()
        .map(|(i, g)| GpuDisplayEntry {
            device_id: g.device_id.clone(),
            driver_name: g.driver_name.clone(),
            raw_usage: g.usage,
            smoothed_usage: *smoothed_values.get(i).unwrap_or(&0.0),
        })
        .collect();
    entries.sort_by(|a, b| a.device_id.cmp(&b.device_id));
    entries
}

pub fn gpu_display_string(entries: &[GpuDisplayEntry]) -> String {
    if entries.is_empty() {
        return "None".to_string();
    }
    let formatted: Vec<String> = entries.iter().map(|e| format!("{}", e)).collect();
    formatted.join(", ")
}

pub fn network_display_string(total_mbps: f64, per_interface: &HashMap<String, f64>) -> String {
    let parts: Vec<String> = per_interface.iter()
        .map(|(name, mbps)| format!("{}: {:.2}", name, mbps))
        .collect();
    if parts.is_empty() {
        return format!("{:.2} Mbps (total)", total_mbps);
    }
    let iface_str = parts.join(", ");
    format!("{:.2} Mbps (total), {}", total_mbps, iface_str)
}

pub fn disk_display_string(
    interval_secs: f64,
    total_mb_per_s: f64,
    per_device: &HashMap<String, f64>,
) -> String {
    let parts: Vec<String> = per_device.iter()
        .map(|(name, mbps)| format!("{}: {:.2}", name, mbps))
        .collect();
    if parts.is_empty() {
        return format!("interval: {:.0}s, {:.2} MB/s (total)", interval_secs, total_mb_per_s);
    }
    let dev_str = parts.join(", ");
    format!("interval: {:.0}s, {:.2} MB/s (total), {}", interval_secs, total_mb_per_s, dev_str)
}
