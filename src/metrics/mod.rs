use anyhow::Result;
use std::collections::HashMap;
use std::fs;
use std::path::Path;
use std::time::{Duration, SystemTime};
use tracing::{debug, warn};

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
    pub gpu_usage: f64,
    pub network_io: f64,
    pub disk_activity: f64,
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
        // Collect all metrics
        let cpu_usage = self.cpu.collect().await?;
        let gpu_usage = self.gpu.collect().await?;
        let network_io = self.network.collect().await?;
        let disk_activity = self.disk.collect().await?;

        self.last_collection = Some(SystemTime::now());

        Ok(Metrics {
            cpu_usage,
            gpu_usage,
            network_io,
            disk_activity,
        })
    }
}

#[derive(Debug)]
pub struct CollectionError {
    pub source: CollectionErrorKind,
}

#[derive(Debug)]
pub enum CollectionErrorKind {
    CpuError,
    GpuError,
    NetworkError,
    DiskError,
}

impl std::fmt::Display for CollectionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.source {
            CollectionErrorKind::CpuError => write!(f, "CPU collection failed"),
            CollectionErrorKind::GpuError => write!(f, "GPU collection failed"),
            CollectionErrorKind::NetworkError => write!(f, "Network collection failed"),
            CollectionErrorKind::DiskError => write!(f, "Disk collection failed"),
        }
    }
}

impl std::error::Error for CollectionError {}
