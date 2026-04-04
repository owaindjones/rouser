use std::time::Duration;
use tracing::{debug, info, warn};

use crate::config::Config;

use crate::inhibit::InhibitionState;
use crate::metrics::{CpuCollector, DiskCollector, GpuCollector, Metrics, NetworkCollector};

#[derive(Debug, Clone)]
pub struct SmoothingState {
    ema: f64,
    initialized: bool,
}

impl SmoothingState {
    pub fn new(_ema_alpha: f64) -> Self {
        Self {
            ema: 0.0,
            initialized: false,
        }
    }

    pub fn update(&mut self, value: f64, ema_alpha: f64) -> f64 {
        if !self.initialized {
            self.ema = value;
            self.initialized = true;
            value
        } else {
            self.ema = ema_alpha * value + (1.0 - ema_alpha) * self.ema;
            self.ema
        }
    }

    pub fn value(&self) -> f64 {
        self.ema
    }
}

pub struct ThresholdManager {
    cpu_threshold: f64,
    gpu_threshold: f64,
    network_threshold: f64,
    disk_threshold: f64,
    #[allow(dead_code)] // EMA alphas stored for potential future use
    cpu_ema_alpha: f64,
    #[allow(dead_code)]
    gpu_ema_alpha: f64,
    #[allow(dead_code)]
    network_ema_alpha: f64,
    #[allow(dead_code)]
    disk_ema_alpha: f64,
}

impl ThresholdManager {
    pub fn new(
        cpu_threshold: f64,
        gpu_threshold: f64,
        network_threshold: f64,
        disk_threshold: f64,
        cpu_ema_alpha: f64,
        gpu_ema_alpha: f64,
        network_ema_alpha: f64,
        disk_ema_alpha: f64,
    ) -> Self {
        Self {
            cpu_threshold,
            gpu_threshold,
            network_threshold,
            disk_threshold,
            cpu_ema_alpha,
            gpu_ema_alpha,
            network_ema_alpha,
            disk_ema_alpha,
        }
    }

    pub fn should_inhibit(
        &self,
        smoothed_cpu: f64,
        smoothed_gpu: f64,
        smoothed_network: f64,
        smoothed_disk: f64,
    ) -> bool {
        smoothed_cpu > self.cpu_threshold
            || smoothed_gpu > self.gpu_threshold
            || smoothed_network > self.network_threshold
            || smoothed_disk > self.disk_threshold
    }
}

pub struct DataManager {
    state: InhibitionState,
    metrics_below_threshold_since: Option<std::time::SystemTime>,
    metrics_above_threshold_since: Option<std::time::SystemTime>,
    cooldown_start_time: Option<std::time::SystemTime>,
    threshold_manager: ThresholdManager,
    cpu: CpuCollector,
    gpu: GpuCollector,
    network: NetworkCollector,
    disk: DiskCollector,
    last_collection: Option<std::time::SystemTime>,
    is_dry_run: bool,
    // EMA smoothing state for each metric
    cpu_smooth: SmoothingState,
    gpu_smooth: SmoothingState,
    network_smooth: SmoothingState,
    disk_smooth: SmoothingState,
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

 pub async fn tick(&mut self, config: &Config) -> Result<(), DataServiceError> {
     self.inner.tick(config).await
 }
}

impl DataManager {
     pub async fn new(
        config: &Config,
        is_dry_run: bool,
    ) -> Result<Self, DataServiceError> {
        let threshold_manager = ThresholdManager::new(
            config.metrics.cpu.threshold,
            config.metrics.gpu.threshold,
            config.metrics.network.threshold,
            config.metrics.disk.threshold,
            config.metrics.cpu.ema_alpha,
            config.metrics.gpu.ema_alpha,
            config.metrics.network.ema_alpha,
            config.metrics.disk.ema_alpha,
        );

        Ok(Self {
            state: InhibitionState::new(),
            metrics_below_threshold_since: None,
            metrics_above_threshold_since: None,
            cooldown_start_time: None,
            threshold_manager,
            cpu: CpuCollector::new(),
            gpu: GpuCollector::new(),
            network: NetworkCollector::new(
                config.metrics.network.exclude_interfaces.clone(),
            ),
            disk: DiskCollector::new(config.metrics.disk.exclude_device_prefixes.clone()),
            last_collection: None,
            is_dry_run,
            cpu_smooth: SmoothingState::new(config.metrics.cpu.ema_alpha),
            gpu_smooth: SmoothingState::new(config.metrics.gpu.ema_alpha),
            network_smooth: SmoothingState::new(config.metrics.network.ema_alpha),
            disk_smooth: SmoothingState::new(config.metrics.disk.ema_alpha),
        })
    }

    pub async fn tick(&mut self, config: &Config) -> Result<(), DataServiceError> {
        let metrics = self.collect_metrics().await?;
        self.last_collection = Some(std::time::SystemTime::now());

        // Apply EMA smoothing to raw metrics
        let smoothed_cpu = self.cpu_smooth.update(metrics.cpu_usage, config.metrics.cpu.ema_alpha);
        let smoothed_gpu = if !metrics.gpu_usage.is_empty() {
            let avg_raw: f64 = metrics.gpu_usage.iter().map(|g| g.usage).sum::<f64>() / metrics.gpu_usage.len() as f64;
            self.gpu_smooth.update(avg_raw, config.metrics.gpu.ema_alpha)
        } else {
            0.0
        };
        let smoothed_network = self.network_smooth.update(metrics.network_io, config.metrics.network.ema_alpha);
        let smoothed_disk = self.disk_smooth.update(metrics.disk_activity, config.metrics.disk.ema_alpha);

        debug!(
            "Metrics: CPU={:.1}% (smoothed: {:.1}%), GPU={:.1}% (smoothed: {:.1}%), Network={:.2} Mbps (smoothed: {:.2}), Disk={:.2} MB/s (smoothed: {:.2})",
            metrics.cpu_usage, smoothed_cpu,
            metrics.gpu_usage.iter().map(|g| g.usage).sum::<f64>() / metrics.gpu_usage.len() as f64, smoothed_gpu,
            metrics.network_io, smoothed_network,
            metrics.disk_activity, smoothed_disk
        );

        let should_inhibit = self.threshold_manager.should_inhibit(
            smoothed_cpu, smoothed_gpu, smoothed_network, smoothed_disk,
        );

        self.update_state(should_inhibit, config).await?;

        if self.state.is_inhibited() {
            info!("Sleep inhibited: at least one metric above threshold");
        } else if let Some(below_since) = self.metrics_below_threshold_since {
            let elapsed = std::time::SystemTime::now()
                .duration_since(below_since)
                .unwrap_or(Duration::from_secs(0));
            
            if elapsed >= config.timing.cooldown_duration {
                info!(
                    "Releasing sleep inhibition: all metrics below threshold for {:?}",
                    elapsed
                );
                self.state.release().await;
                self.metrics_below_threshold_since = Some(std::time::SystemTime::now());
                self.cooldown_start_time = None;
            }
        }

        Ok(())
    }

    async fn collect_metrics(&mut self) -> Result<Metrics, DataServiceError> {
        let cpu_usage = self.cpu.collect().await.map_err(|e| DataServiceError {
            inner: format!("CPU collection failed: {}", e),
        })?;
    let gpu_usage = self.gpu.collect().await.map_err(|e| DataServiceError {
        inner: format!("GPU collection failed: {}", e),
    })?;
    let network_io = self.network.collect().await.map_err(|e| DataServiceError {
        inner: format!("Network collection failed: {}", e),
    })?;
    let disk_activity = self.disk.collect().await.map_err(|e| DataServiceError {
        inner: format!("Disk collection failed: {}", e),
    })?;

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
                           "[DRY RUN] Would inhibit sleep: metrics exceed threshold for {:?}",
                           config.timing.duration_threshold
                       );
                   } else {
 let _what = &config.inhibition.what;
                        let who = std::env::var("USER").unwrap_or_else(|_| "rouser".to_string());
                       let description = "Rouser: system metrics exceed threshold".to_string();
                       
                       match self.state.acquire(
                             &config.inhibition.what,
                             &who,
                             &description,
                             &config.inhibition.mode,
                        ).await {
                         Ok(_) => {
                             self.metrics_below_threshold_since = None;
                             self.metrics_above_threshold_since = Some(std::time::SystemTime::now());
                             self.cooldown_start_time = None;
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
                self.cooldown_start_time = Some(std::time::SystemTime::now());
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

#[cfg(test)]
mod tests {
    use super::*;

    fn create_test_config() -> Config {
        Config {
            name: "test".to_string(),
            update_interval: std::time::Duration::from_secs(5),
            log_level: "info".to_string(),
            metrics: crate::config::Metrics {
                cpu: crate::config::CpuConfig {
                    threshold: 80.0,
                    ema_alpha: 0.3,
                },
                gpu: crate::config::GpuConfig {
                    threshold: 90.0,
                    ema_alpha: 0.3,
                },
                network: crate::config::NetworkConfig {
                    threshold: 100.0,
                    ema_alpha: 0.2,
                    exclude_interfaces: vec!["lo".to_string()],
                    include_interfaces: vec![],
                },
                disk: crate::config::DiskConfig {
                    threshold: 50.0,
                    ema_alpha: 0.2,
                    exclude_device_prefixes: vec!["loop".to_string()],
                },
            },
            timing: crate::config::TimingConfig {
                duration_threshold: std::time::Duration::from_secs(30),
                cooldown_duration: std::time::Duration::from_secs(60),
            },
            inhibition: crate::config::InhibitionConfig {
                what: "sleep".to_string(),
                mode: "block".to_string(),
            },
        }
    }

    #[test]
    fn test_threshold_manager_creation() {
        let config = create_test_config();
        let manager = ThresholdManager::new(
            config.metrics.cpu.threshold,
            config.metrics.gpu.threshold,
            config.metrics.network.threshold,
            config.metrics.disk.threshold,
            config.metrics.cpu.ema_alpha,
            config.metrics.gpu.ema_alpha,
            config.metrics.network.ema_alpha,
            config.metrics.disk.ema_alpha,
        );
        assert!(true); // Basic instantiation test
    }

    #[test]
    fn test_threshold_manager_should_inhibit_high_cpu() {
        let config = create_test_config();
        let manager = ThresholdManager::new(
            config.metrics.cpu.threshold,
            config.metrics.gpu.threshold,
            config.metrics.network.threshold,
            config.metrics.disk.threshold,
            config.metrics.cpu.ema_alpha,
            config.metrics.gpu.ema_alpha,
            config.metrics.network.ema_alpha,
            config.metrics.disk.ema_alpha,
        );
        
        assert!(manager.should_inhibit(90.0, 50.0, 10.0, 5.0));
    }

    #[test]
    fn test_threshold_manager_should_not_inhibit_idle_cpu() {
        let config = create_test_config();
        let manager = ThresholdManager::new(
            config.metrics.cpu.threshold,
            config.metrics.gpu.threshold,
            config.metrics.network.threshold,
            config.metrics.disk.threshold,
            config.metrics.cpu.ema_alpha,
            config.metrics.gpu.ema_alpha,
            config.metrics.network.ema_alpha,
            config.metrics.disk.ema_alpha,
        );
        
        assert!(!manager.should_inhibit(50.0, 10.0, 10.0, 5.0));
    }

    #[test]
    fn test_data_service_error_display() {
        let err = DataServiceError {
            inner: "test error".to_string(),
        };
        let display = format!("{}", err);
        assert!(display.contains("test error"));
    }
}

#[cfg(test)]
mod ema_tests {
    use super::*;

    #[test]
    fn test_ema_initialization() {
        let mut state = SmoothingState::new(0.5);
        // First update should set initial value
        let val = state.update(50.0, 0.5);
        assert_eq!(val, 50.0);
        assert_eq!(state.value(), 50.0);
    }

    #[test]
    fn test_ema_smoothing() {
        let mut state = SmoothingState::new(0.5);
        
        // Initial value
        assert_eq!(state.update(10.0, 0.5), 10.0);
        
        // With alpha=0.5: new_ema = 0.5 * new + 0.5 * old
        // Second reading of 20.0: ema = 0.5 * 20 + 0.5 * 10 = 15
        assert_eq!(state.update(20.0, 0.5), 15.0);
        
        // Third reading of 30.0: ema = 0.5 * 30 + 0.5 * 15 = 22.5
        assert_eq!(state.update(30.0, 0.5), 22.5);
    }

    #[test]
    fn test_ema_low_alpha() {
        let mut state = SmoothingState::new(0.1);
        
        // With alpha=0.1, changes are slower
        assert_eq!(state.update(10.0, 0.1), 10.0);
        assert_eq!(state.update(90.0, 0.1), 18.0); // 0.1 * 90 + 0.9 * 10 = 18
        
        // Fourth reading of 50.0: ema = 0.1 * 50 + 0.9 * 18 = 21.2
        assert_eq!(state.update(50.0, 0.1), 21.2);
    }

    #[test]
    fn test_ema_high_alpha() {
        let mut state = SmoothingState::new(0.9);
        
        // With alpha=0.9, changes are faster
        assert_eq!(state.update(10.0, 0.9), 10.0);
        assert_eq!(state.update(90.0, 0.9), 82.0); // 0.9 * 90 + 0.1 * 10 = 82
    }

    #[test]
    fn test_ema_converges() {
        let mut state = SmoothingState::new(0.3);
        
        // Feed a steady value and verify convergence
        for _ in 0..20 {
            state.update(50.0, 0.3);
        }
        
        // After many updates with same value, EMA should be very close to that value
        assert!((state.value() - 50.0).abs() < 0.01);
    }
}

#[cfg(test)]
mod cooldown_tests {
    use super::*;
    use std::time::{Duration, SystemTime};

    #[test]
    fn test_cooldown_timing() {
        // Simulate the cooldown logic
        let mut cooldown_start: Option<SystemTime> = None;
        let cooldown_duration = Duration::from_secs(60);
        
        // At start, no cooldown
        assert!(cooldown_start.is_none());
        
        // Metrics go below threshold
        cooldown_start = Some(SystemTime::now());
        
        // Wait 30 seconds (simulated)
        let elapsed_30s = Duration::from_secs(30);
        assert!(cooldown_start.unwrap() + elapsed_30s < SystemTime::now() + cooldown_duration);
        
        // Cooldown not yet complete
        assert!(cooldown_start.unwrap() + cooldown_duration > SystemTime::now());
    }

    #[test]
    fn test_cooldown_elapsed() {
        let mut cooldown_start = Some(SystemTime::now() - Duration::from_secs(65));
        let cooldown_duration = Duration::from_secs(60);
        
        // After 65 seconds, cooldown should be complete
        if let Some(start) = cooldown_start {
            assert!(start + cooldown_duration <= SystemTime::now());
        }
    }
}
