pub mod cpu;
pub mod disk;
pub mod gpu;
pub mod network;

pub use cpu::{CpuCollector, CpuUsage};
pub use disk::DiskCollector;
pub use gpu::{GpuCollector, GpuData};
pub use network::NetworkCollector;

#[derive(Debug, Clone)]
pub struct Metrics {
    pub cpu_usage: CpuUsage,
    pub gpu_usage: Vec<GpuData>,
    pub network_io: f64,
    pub disk_activity: f64,
}
