use anyhow::Result;
use self::cpu::CpuError;
use self::disk::DiskError;
use self::gpu::GpuData;
use self::gpu::GpuError;
use self::network::NetworkError;
use std::time::SystemTime;
use tracing::debug;


pub mod cpu;
pub mod disk;
pub mod gpu;
pub mod network;

pub use cpu::CpuCollector;
pub use disk::DiskCollector;
pub use gpu::GpuCollector;
pub use network::NetworkCollector;

#[derive(Debug, Clone)]
pub struct Metrics {
    pub cpu_usage: f64,
    pub gpu_usage: Vec<GpuData>,
    pub network_io: f64,
    pub disk_activity: f64,
}

#[derive(Debug)]
pub struct CollectionError {
    source: CollectionErrorKind,
}

#[derive(Debug)]
pub enum CollectionErrorKind {
    Cpu(CpuError),
    Gpu(GpuError),
    Network(NetworkError),
    Disk(DiskError),
}

impl std::fmt::Display for CollectionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match &self.source {
            CollectionErrorKind::Cpu(e) => write!(f, "CPU collection failed: {}", e),
            CollectionErrorKind::Gpu(e) => write!(f, "GPU collection failed: {}", e),
            CollectionErrorKind::Network(e) => write!(f, "Network collection failed: {}", e),
            CollectionErrorKind::Disk(e) => write!(f, "Disk collection failed: {}", e),
        }
    }
}

impl std::error::Error for CollectionError {}

impl From<CpuError> for CollectionError {
    fn from(e: CpuError) -> Self {
        Self { source: CollectionErrorKind::Cpu(e) }
    }
}

impl From<GpuError> for CollectionError {
    fn from(e: GpuError) -> Self {
        Self { source: CollectionErrorKind::Gpu(e) }
    }
}

impl From<NetworkError> for CollectionError {
    fn from(e: NetworkError) -> Self {
        Self { source: CollectionErrorKind::Network(e) }
    }
}

impl From<DiskError> for CollectionError {
    fn from(e: DiskError) -> Self {
        Self { source: CollectionErrorKind::Disk(e) }
    }
}

pub struct MetricsCollector {
    pub cpu: CpuCollector,
    pub gpu: GpuCollector,
    pub network: NetworkCollector,
    pub disk: DiskCollector,
    pub last_collection: Option<SystemTime>,
}

impl MetricsCollector {
    pub fn new(
        exclude_interfaces: Vec<String>,
        exclude_disk_prefixes: Vec<String>,
    ) -> Self {
        Self {
            cpu: CpuCollector::new(),
            gpu: GpuCollector::new(),
            network: NetworkCollector::new(exclude_interfaces),
            disk: DiskCollector::new(exclude_disk_prefixes),
            last_collection: None,
        }
    }

 pub async fn collect(&mut self) -> Result<Metrics, CollectionError> {
    debug!("Collecting metrics");
    let cpu_usage = self.cpu.collect().await?;
    let gpu_data = self.gpu.collect().await?;
    let network_io = self.network.collect().await?;
    let disk_activity = self.disk.collect().await?;

    self.last_collection = Some(SystemTime::now());

    Ok(Metrics {
        cpu_usage,
        gpu_usage: gpu_data,
        network_io,
        disk_activity,
    })
}
}
