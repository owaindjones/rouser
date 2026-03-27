use std::time::Duration;
use tracing::{debug, info, warn};

use crate::config::{Config, Thresholds, TimingConfig};
use crate::metrics::{CpuCollector, DiskCollector, GpuCollector, Metrics, NetworkCollector};
use crate::inhibit::InhibitionState;

pub struct ThresholdManager {
    thresholds: Thresholds,
    timing: TimingConfig,
}

impl ThresholdManager {
    pub fn new(thresholds: &Thresholds, timing: &TimingConfig) -> Self {
        Self {
            thresholds: thresholds.clone(),
            timing: timing.clone(),
        }
    }

    pub fn should_inhibit(&self, metrics: &Metrics) -> bool {
        metrics.cpu_usage > self.thresholds.cpu_usage
            || metrics.gpu_usage > self.thresholds.gpu_usage
            || metrics.network_io > self.thresholds.network_io
            || metrics.disk_activity > self.thresholds.disk_activity
    }
}

pub struct DataService {
    inner: DataManager,
}

impl DataService {
    pub async fn new(
        config: &Config,
        is_dry_run: bool,
    ) -> anyhow::Result<Self> {
        let inner = DataManager::new(config, is_dry_run).await?;
        Ok(Self { inner })
    }

    pub async fn tick(&mut self, config: &Config) -> anyhow::Result<()> {
        self.inner.tick(config).await
    }
}

pub struct DataManager {
    state: InhibitionState,
    metrics_below_threshold_since: Option<std::time::SystemTime>,
    metrics_above_threshold_since: Option<std::time::SystemTime>,
    threshold_manager: ThresholdManager,
    cpu: CpuCollector,
    gpu: GpuCollector,
    network: NetworkCollector,
    disk: DiskCollector,
    last_collection: Option<std::time::SystemTime>,
    is_dry_run: bool,
}

impl DataManager {
    pub async fn new(
        config: &Config,
        is_dry_run: bool,
    ) -> Result<Self, DataServiceError> {
        let threshold_manager = ThresholdManager::new(&config.thresholds, &config.timing);

        Ok(Self {
            state: InhibitionState::new(),
            metrics_below_threshold_since: Some(std::time::SystemTime::now()),
            metrics_above_threshold_since: None,
            threshold_manager,
            cpu: CpuCollector::new(),
            gpu: GpuCollector::new(),
            network: NetworkCollector::new(
                config.network.exclude_interfaces.clone(),
            ),
            disk: DiskCollector::new(config.disk.exclude_device_prefixes.clone()),
            last_collection: None,
            is_dry_run,
        })
    }

    pub async fn tick(&mut self, config: &Config) -> Result<(), DataServiceError> {
        let metrics = self.collect_metrics().await?;
        self.last_collection = Some(std::time::SystemTime::now());

        debug!(
            "Metrics: CPU={:.1}%, GPU={:.1}%, Network={:.2} Mbps, Disk={:.2} MB/s",
            metrics.cpu_usage, metrics.gpu_usage, metrics.network_io, metrics.disk_activity
        );

        let should_inhibit = self.threshold_manager.should_inhibit(&metrics);

        self.update_state(should_inhibit, &metrics, config).await?;

        if self.state.is_inhibited() {
            info!("Sleep inhibited: at least one metric above threshold");
        } else if let Some(below_since) = self.metrics_below_threshold_since {
            let elapsed = std::time::SystemTime::now()
                .duration_since(below_since)
                .unwrap_or(Duration::from_secs(0));
            
            if elapsed >= config.timing.idle_duration {
                info!(
                    "Releasing sleep inhibition: all metrics below threshold for {:?}",
                    elapsed
                );
                self.state.release().await;
                self.metrics_below_threshold_since = Some(std::time::SystemTime::now());
            }
        }

        Ok(())
    }

    async fn collect_metrics(&mut self) -> Result<Metrics, DataServiceError> {
        let cpu_usage = self.cpu.collect().await?;
        let gpu_usage = self.gpu.collect().await?;
        let network_io = self.network.collect().await?;
        let disk_activity = self.disk.collect().await?;

        Ok(Metrics {
            cpu_usage,
            gpu_usage,
            network_io,
            disk_activity,
        })
    }

    async fn update_state(
        &mut self,
        should_inhibit: bool,
        metrics: &Metrics,
        config: &Config,
    ) -> Result<(), DataServiceError> {
        if should_inhibit {
            debug!("Metrics exceed threshold, checking inhibition status");
            
            self.metrics_above_threshold_since = 
                self.metrics_above_threshold_since
                .or_else(|| Some(std::time::SystemTime::now()));

            if let Some(above_since) = self.metrics_above_threshold_since {
                let elapsed = std::time::SystemTime::now()
                    .duration_since(above_since)
                    .unwrap_or(Duration::from_secs(0));

                if elapsed < config.timing.duration_threshold {
                    debug!(
                        "Below duration_threshold: {}/{} seconds above threshold",
                        elapsed.as_secs(),
                        config.timing.duration_threshold.as_secs()
                    );
                    return Ok(());
                }
            }

            if !self.state.is_inhibited() {
                if self.is_dry_run {
                    info!(
                        "[DRY RUN] Would inhibit sleep: {} above threshold for {:?}",
                        metrics_exceeded_desc(metrics, &config.thresholds),
                        config.timing.duration_threshold
                    );
                } else {
                    let what: String = config.inhibition.what.join(",");
                    let description = "Rouser: system metrics exceed threshold".to_string();
                    
                    match self.state.acquire(
                        &what,
                        &description,
                        &config.inhibition.mode,
                    ).await {
                        Ok(_) => {
                            self.metrics_below_threshold_since = None;
                            self.metrics_above_threshold_since = Some(std::time::SystemTime::now());
                        }
                        Err(e) => {
                            warn!("Failed to acquire inhibition: {}", e);
                        }
                    }
                }
            }
        } else {
            debug!("All metrics below threshold");
            
            if self.metrics_above_threshold_since.is_some() {
                self.metrics_below_threshold_since = 
                    Some(std::time::SystemTime::now());
                self.metrics_above_threshold_since = None;
            }

            if self.state.is_inhibited() {
                if self.is_dry_run {
                    info!("[DRY RUN] Would release sleep inhibition");
                } else {
                    self.state.release().await;
                }
            }
        }

        Ok(())
    }
}

fn metrics_exceeded_desc(metrics: &Metrics, thresholds: &Thresholds) -> String {
    let mut parts = vec![];
    if metrics.cpu_usage > thresholds.cpu_usage {
        parts.push(format!("CPU {}% > {}%", metrics.cpu_usage, thresholds.cpu_usage));
    }
    if metrics.gpu_usage > thresholds.gpu_usage {
        parts.push(format!("GPU {}% > {}%", metrics.gpu_usage, thresholds.gpu_usage));
    }
    if metrics.network_io > thresholds.network_io {
        parts.push(format!("Network {:.1} Mbps > {:.1} Mbps", metrics.network_io, thresholds.network_io));
    }
    if metrics.disk_activity > thresholds.disk_activity {
        parts.push(format!("Disk {:.1} MB/s > {:.1} MB/s", metrics.disk_activity, thresholds.disk_activity));
    }
    parts.join(", ")
}

#[derive(Debug)]
pub struct DataServiceError {
    inner: String,
}

impl std::fmt::Display for DataServiceError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Data service error: {}", self.inner)
    }
}

impl std::error::Error for DataServiceError {}

impl From<std::io::Error> for DataServiceError {
    fn from(e: std::io::Error) -> Self {
        Self { inner: e.to_string() }
    }
}
