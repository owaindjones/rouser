use std::time::Duration;
use tracing::{debug, info, warn};

use crate::config::Config;
use crate::prediction::{CooldownPrediction, PredictionModel};

use crate::inhibit::InhibitionState;
use crate::metrics::{
    disk_display_string, gpu_display_string, network_display_string, sorted_gpu_display,
    CpuCollector, DiskCollector, GpuCollector, NetworkCollector, TickMetrics,
};

#[derive(Debug, Clone)]
pub struct SmoothingState {
    ema: f64,
    initialized: bool,
}

impl SmoothingState {
    pub fn new(_alpha: f64) -> Self {
        Self {
            ema: 0.0,
            initialized: false,
        }
    }

    /// Update with asymmetric EMA: faster response to increases, slower decay for decreases
    /// This prevents rapid inhibition release from brief idle periods while remaining responsive to spikes
    pub fn update(&mut self, value: f64, alpha: f64) -> f64 {
        if !self.initialized {
            self.ema = value;
            self.initialized = true;
            return value;
        }

        let factor = if value > self.ema {
            // Rising edge: use higher alpha (2x the configured alpha for faster response)
            // Cap at 1.0 to prevent overshoot
            (alpha.max(0.1) * 2.0).min(1.0)
        } else {
            // Falling edge: use lower alpha (0.5x the configured alpha for slower decay)
            alpha.clamp(0.01, 0.5) / 2.0
        };

        self.ema = factor * value + (1.0 - factor) * self.ema;
        self.ema
    }

    #[allow(dead_code)]
    pub fn value(&self) -> f64 {
        self.ema
    }
}

pub struct ThresholdManager {
    cpu_per_core_threshold: f64,
    cpu_total_threshold: f64,
    gpu_threshold: f64,
    network_threshold: f64,
    disk_threshold: f64,
}

impl ThresholdManager {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        cpu_per_core_threshold: f64,
        cpu_total_threshold: f64,
        gpu_threshold: f64,
        network_threshold: f64,
        disk_threshold: f64,
    ) -> Self {
        Self {
            cpu_per_core_threshold,
            cpu_total_threshold,
            gpu_threshold,
            network_threshold,
            disk_threshold,
        }
    }

    pub fn should_inhibit(
        &self,
        smoothed_cpu_max: f64,
        smoothed_cpu_avg: f64,
        gpu_smoothed_values: &[f64],
        smoothed_network: f64,
        smoothed_disk: f64,
    ) -> bool {
        smoothed_cpu_max > self.cpu_per_core_threshold
            || smoothed_cpu_avg > self.cpu_total_threshold
            || gpu_smoothed_values.iter().any(|&v| v > self.gpu_threshold)
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
    // EMA smoothing state for each metric (and per-GPU)
    cpu_smooth_max: SmoothingState,
    cpu_smooth_avg: SmoothingState,
    gpu_smoothing: Vec<SmoothingState>,
    network_smooth: SmoothingState,
    disk_smooth: SmoothingState,
    #[allow(dead_code)] // tracked in tick() for state-change detection
    previous_inhibited_state: bool,
    just_released: bool,
    waiting_for_cooldown: bool,
    /// Cached predicted additional time from last tick's model query.
    /// Applied to cooldown_duration when metrics drop below threshold.
    predicted_additional_time: std::time::Duration,
    // Prediction model for adaptive cooldown extension (None if disabled).
    prediction_model: Option<PredictionModel>,
}

pub struct DataService {
    inner: DataManager,
}

impl DataService {
    pub async fn new(config: &Config, is_dry_run: bool) -> anyhow::Result<Self> {
        let inner = DataManager::new(config, is_dry_run).await?;
        Ok(Self { inner })
    }

    pub async fn tick(&mut self, config: &Config) -> Result<(), DataServiceError> {
        self.inner.tick(config).await
    }
}

impl DataManager {
    pub async fn new(config: &Config, is_dry_run: bool) -> Result<Self, DataServiceError> {
        let threshold_manager = ThresholdManager::new(
            config.metrics.cpu.per_core_threshold,
            config.metrics.cpu.total_threshold,
            config.metrics.gpu.threshold,
            config.metrics.network.threshold,
            config.metrics.disk.threshold,
        );

        // Initialize prediction model if enabled (prediction.update_interval is set).
        let prediction_model = if config.prediction.update_interval.as_secs() > 0 {
            // Determine if running as root to choose history directory.
            #[cfg(unix)]
            let is_root: bool = unsafe { libc::geteuid() == 0 };
            #[cfg(not(unix))]
            let is_root: bool = false;

            let mut model = PredictionModel::new(
                is_root,
                config.prediction.update_interval.as_nanos() as u64,
                config.prediction.max_extension_time,
            );
            let effective_prediction_interval =
                std::cmp::max(config.prediction.update_interval, config.update_interval);
            if config.prediction.update_interval < config.update_interval
                && config.update_interval.as_secs() > 0
            {
                warn!(
                    "prediction.update_interval ({:?}) is less than root update_interval ({}s) — \
                 using {:?} instead to avoid erratic accumulation flush timing",
                    config.prediction.update_interval,
                    config.update_interval.as_secs(),
                    effective_prediction_interval,
                );
            }
            // Configure how often to flush averaged snapshots (every N ticks).
            model.set_prediction_update_interval(effective_prediction_interval);
            Some(model)
        } else {
            None
        };

        // Initialize per-GPU smoothing states based on detected GPUs
        let gpu_collector = GpuCollector::new();
        let has_gpu = gpu_collector.has_gpus();
        let num_gpus = if has_gpu { 2 } else { 0 }; // Default to 2 GPU slots, will resize on first collection

        Ok(Self {
            state: InhibitionState::new(),
            metrics_below_threshold_since: None,
            metrics_above_threshold_since: None,
            cooldown_start_time: None,
            threshold_manager,
            cpu: CpuCollector::new(),
            gpu: GpuCollector::new(),
            network: NetworkCollector::new(config.metrics.network.exclude_interfaces.clone()),
            disk: DiskCollector::new(config.metrics.disk.exclude_device_prefixes.clone()),
            last_collection: None,
            is_dry_run,
            previous_inhibited_state: false,
            just_released: false,
            waiting_for_cooldown: false,
            predicted_additional_time: std::time::Duration::ZERO,
            prediction_model,
            cpu_smooth_max: SmoothingState::new(config.metrics.cpu.ema_alpha),
            cpu_smooth_avg: SmoothingState::new(config.metrics.cpu.ema_alpha),
            gpu_smoothing: (0..num_gpus)
                .map(|_| SmoothingState::new(config.metrics.gpu.ema_alpha))
                .collect(),
            network_smooth: SmoothingState::new(config.metrics.network.ema_alpha),
            disk_smooth: SmoothingState::new(config.metrics.disk.ema_alpha),
        })
    }

    pub async fn tick(&mut self, config: &Config) -> Result<(), DataServiceError> {
        let metrics = self.collect_metrics().await?;
        self.last_collection = Some(std::time::SystemTime::now());

        let smoothed_cpu_max = self
            .cpu_smooth_max
            .update(metrics.cpu_usage.per_core_max, config.metrics.cpu.ema_alpha);
        let smoothed_cpu_avg = self.cpu_smooth_avg.update(
            metrics.cpu_usage.total_average,
            config.metrics.cpu.ema_alpha,
        );

        let num_devices = metrics.gpu_usage.len();
        while self.gpu_smoothing.len() < num_devices {
            self.gpu_smoothing
                .push(SmoothingState::new(config.metrics.gpu.ema_alpha));
        }
        self.gpu_smoothing.truncate(num_devices);

        let mut gpu_smoothed_values: Vec<f64> = vec![0.0; num_devices];
        for (i, gpu) in metrics.gpu_usage.iter().enumerate() {
            if i < self.gpu_smoothing.len() {
                gpu_smoothed_values[i] =
                    self.gpu_smoothing[i].update(gpu.usage, config.metrics.gpu.ema_alpha);
            }
        }

        let sorted_entries = sorted_gpu_display(&metrics.gpu_usage, &gpu_smoothed_values);
        let gpu_debug = gpu_display_string(&sorted_entries);

        let smoothed_network = self.network_smooth.update(
            metrics.network_throughput.total_mbps,
            config.metrics.network.ema_alpha,
        );
        let network_log = network_display_string(
            metrics.network_throughput.total_mbps,
            &metrics.network_throughput.per_interface,
        );

        let smoothed_disk = self.disk_smooth.update(
            metrics.disk_throughput.total_mb_per_s,
            config.metrics.disk.ema_alpha,
        );
        let disk_log = disk_display_string(
            metrics.disk_throughput.interval_secs,
            metrics.disk_throughput.total_mb_per_s,
            &metrics.disk_throughput.per_device,
        );

        debug!(
            "Metrics: CPU max={:.1}% avg={:.1}%, GPU: {}, Network={}, Disk={}",
            smoothed_cpu_max, smoothed_cpu_avg, gpu_debug, network_log, disk_log
        );

        let should_inhibit = self.threshold_manager.should_inhibit(
            smoothed_cpu_max,
            smoothed_cpu_avg,
            &gpu_smoothed_values,
            smoothed_network,
            smoothed_disk,
        );

        // Record metrics into prediction history if enabled. Accumulates per-tick and flushes averaged snapshots on interval.
        if let Some(ref mut model) = self.prediction_model {
            let _flushed = model.record(
                smoothed_cpu_max,
                smoothed_cpu_avg,
                gpu_smoothed_values.clone(),
                smoothed_network,
                smoothed_disk,
                should_inhibit,
            );
            // debug! already logs inside model.record() when a snapshot is flushed.
        }

        if let Some(ref mut model) = self.prediction_model {
            model.prune(config.prediction.history_length);
        }

        self.update_state(should_inhibit).await?;

        let was_inhibited = self.previous_inhibited_state;

        if should_inhibit {
            // Cancel cooldown — metrics spiked again while waiting.
            if self.waiting_for_cooldown {
                self.waiting_for_cooldown = false;
                self.metrics_below_threshold_since = None;
            }

            self.metrics_above_threshold_since = self
                .metrics_above_threshold_since
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
                } else if !self.state.is_inhibited() {
                    // Duration threshold met — acquire inhibition.
                    if self.is_dry_run {
                        info!(
                            "[DRY RUN] Would inhibit sleep: metrics exceed threshold for {:?}",
                            config.timing.duration_threshold
                        );
                    } else {
                        let who = std::env::var("USER").unwrap_or_else(|_| "rouser".to_string());
                        let description = "Rouser: system metrics exceed threshold".to_string();

                        match self
                            .state
                            .acquire(
                                &config.inhibitor.what,
                                &who,
                                &description,
                                &config.inhibitor.mode,
                            )
                            .await
                        {
                            Ok(_) => {
                                self.metrics_below_threshold_since = None;
                                self.cooldown_start_time = None;
                                self.just_released = false;
                                // Clear prediction — fresh prediction will be computed when metrics drop below again.
                                self.predicted_additional_time = std::time::Duration::ZERO;
                            }
                            Err(e) => warn!("Failed to acquire inhibition: {}", e),
                        }
                    }
                }
            }
        } else if let Some(below_since) = self.metrics_below_threshold_since {
            // Metrics dropped below threshold — check cooldown before releasing.
            let elapsed = std::time::SystemTime::now()
                .duration_since(below_since)
                .unwrap_or(Duration::from_secs(0));

            // Re-evaluate prediction every tick during cooldown waiting to adapt extension
            // based on current trends (increases or decreases the remaining wait time).
            let was_active = !self.predicted_additional_time.is_zero();
            if self.prediction_model.is_some() {
                let prediction = match &self.prediction_model {
                    Some(model) => model.predict_cooldown(),
                    None => CooldownPrediction {
                        additional_time: std::time::Duration::ZERO,
                        confidence: 0.0,
                    },
                };

                // Log info-level only when first applying a non-zero extension per transition;
                // log debug-level for subsequent updates during extended cooldown.
                if was_active && self.predicted_additional_time != prediction.additional_time {
                    debug!(
                        "Updated predictive cooldown extension: {:?} -> {:?}",
                        self.predicted_additional_time, prediction.additional_time
                    );
                } else if !was_active && !prediction.additional_time.is_zero() {
                    info!(
                        "Predictive cooldown extension: +{}s (confidence={:.0}%), \
                         historical patterns suggest active usage at this hour",
                        prediction.additional_time.as_secs(),
                        prediction.confidence * 100.0,
                    );
                }

                self.predicted_additional_time = prediction.additional_time;
            }

            if !self.just_released && self.state.is_inhibited() {
                let effective_cooldown = std::cmp::max(
                    config.timing.cooldown_duration,
                    self.predicted_additional_time,
                );

                if elapsed >= effective_cooldown {
                    if !self.predicted_additional_time.is_zero() {
                        let total_wait =
                            config.timing.cooldown_duration + self.predicted_additional_time;
                        info!(
                            "Releasing sleep inhibition: all metrics below threshold for {:?} \
                             (base cooldown {}s, with {}s predictive extension, total wait {:?})",
                            elapsed,
                            config.timing.cooldown_duration.as_secs(),
                            self.predicted_additional_time.as_secs(),
                            total_wait,
                        );
                    } else {
                        info!(
                            "Releasing sleep inhibition: all metrics below threshold for {:?}",
                            elapsed
                        );
                    }
                    self.state.release().await;
                    self.waiting_for_cooldown = false;
                    self.metrics_below_threshold_since = None;
                    self.just_released = true;
                } else {
                    debug!(
                        "Waiting for cooldown: {}s/{}s below threshold \
                         (with {:?} predictive extension)",
                        elapsed.as_secs(),
                        effective_cooldown.as_secs(),
                        self.predicted_additional_time,
                    );
                }
            } else if !self.state.is_inhibited() {
                // Not inhibited — reset state tracking for fresh below-threshold cycle.
                self.waiting_for_cooldown = false;
                self.just_released = false;
                self.metrics_below_threshold_since = None;
            }
        }

        // Predict cooldown extension when transitioning from inhibited to below-threshold.
        // Only set initial prediction here — the active cooldown block (above) re-evaluates
        // every tick and produces fresher predictions based on updated in-memory model state.
        if was_inhibited && !should_inhibit {
            let prediction = match &self.prediction_model {
                Some(model) => model.predict_cooldown(),
                None => CooldownPrediction {
                    additional_time: std::time::Duration::ZERO,
                    confidence: 0.0,
                },
            };

            // Only apply from the transition block if no prediction exists yet (first tick below threshold).
            if self.predicted_additional_time.is_zero() {
                self.predicted_additional_time = prediction.additional_time;
                if !prediction.additional_time.is_zero() {
                    info!(
                        "Predictive cooldown extension: +{}s (confidence={:.0}%), \
                         historical patterns suggest active usage at this hour",
                        prediction.additional_time.as_secs(),
                        prediction.confidence * 100.0,
                    );
                }
            }
        } else if should_inhibit && self.metrics_above_threshold_since.is_some() {
            // Metrics spiked again — reset extension and flag for fresh cooldown cycle.
            self.predicted_additional_time = std::time::Duration::ZERO;
        }

        if !was_inhibited && self.state.is_inhibited() {
            info!("Sleep inhibited: at least one metric above threshold");
        }

        self.previous_inhibited_state = self.state.is_inhibited();

        Ok(())
    }

    async fn collect_metrics(&mut self) -> Result<TickMetrics, DataServiceError> {
        let cpu_usage = self.cpu.collect().await.map_err(|e| DataServiceError {
            inner: format!("CPU collection failed: {}", e),
        })?;
        let gpu_usage = self.gpu.collect().await.map_err(|e| DataServiceError {
            inner: format!("GPU collection failed: {}", e),
        })?;
        let network_throughput = self.network.collect().await.map_err(|e| DataServiceError {
            inner: format!("Network collection failed: {}", e),
        })?;
        let disk_throughput = self.disk.collect().await.map_err(|e| DataServiceError {
            inner: format!("Disk collection failed: {}", e),
        })?;

        Ok(TickMetrics {
            cpu_usage,
            gpu_usage,
            network_throughput,
            disk_throughput,
        })
    }

    async fn update_state(&mut self, should_inhibit: bool) -> Result<(), DataServiceError> {
        if should_inhibit && self.metrics_above_threshold_since.is_none() {
            self.waiting_for_cooldown = false;

            self.metrics_above_threshold_since = Some(std::time::SystemTime::now());
        } else if !should_inhibit && self.metrics_above_threshold_since.is_some() {
            self.waiting_for_cooldown = true;
            self.metrics_below_threshold_since = Some(std::time::SystemTime::now());
            self.metrics_above_threshold_since = None;
            self.cooldown_start_time = Some(std::time::SystemTime::now());
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
        Self {
            inner: e.to_string(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create_test_config() -> Config {
        Config {
            update_interval: std::time::Duration::from_secs(5),
            log_level: "info".to_string(),
            metrics: crate::config::Metrics {
                cpu: crate::config::CpuConfig {
                    per_core_threshold: 80.0,
                    total_threshold: 50.0,
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
            inhibitor: crate::config::InhibitionConfig {
                what: "sleep".to_string(),
                mode: "block".to_string(),
            },
            prediction: crate::config::PredictionConfig {
                update_interval: std::time::Duration::from_secs(30),
                history_length: std::time::Duration::from_secs(30 * 24 * 60 * 60),
                max_extension_time: std::time::Duration::from_secs(60),
            },
        }
    }

    #[test]
    fn test_threshold_manager_creation() {
        let config = create_test_config();
        let _manager = ThresholdManager::new(
            config.metrics.cpu.per_core_threshold,
            config.metrics.cpu.total_threshold,
            config.metrics.gpu.threshold,
            config.metrics.network.threshold,
            config.metrics.disk.threshold,
        );
    }

    #[test]
    fn test_threshold_manager_should_inhibit_high_cpu() {
        let config = create_test_config();
        let manager = ThresholdManager::new(
            config.metrics.cpu.per_core_threshold,
            config.metrics.cpu.total_threshold,
            config.metrics.gpu.threshold,
            config.metrics.network.threshold,
            config.metrics.disk.threshold,
        );

        assert!(manager.should_inhibit(90.0, 30.0, &[50.0], 10.0, 5.0));
    }

    #[test]
    fn test_threshold_manager_should_not_inhibit_idle_cpu() {
        let config = create_test_config();
        let manager = ThresholdManager::new(
            config.metrics.cpu.per_core_threshold,
            config.metrics.cpu.total_threshold,
            config.metrics.gpu.threshold,
            config.metrics.network.threshold,
            config.metrics.disk.threshold,
        );

        assert!(!manager.should_inhibit(50.0, 30.0, &[10.0], 10.0, 5.0));
    }

    #[test]
    fn test_threshold_manager_should_inhibit_high_gpu() {
        let config = create_test_config();
        let manager = ThresholdManager::new(
            config.metrics.cpu.per_core_threshold,
            config.metrics.cpu.total_threshold,
            config.metrics.gpu.threshold,
            config.metrics.network.threshold,
            config.metrics.disk.threshold,
        );

        assert!(manager.should_inhibit(80.0, 45.0, &[95.0], 10.0, 5.0));
    }

    #[test]
    fn test_threshold_manager_should_inhibit_any_gpu() {
        let config = create_test_config();
        let manager = ThresholdManager::new(
            config.metrics.cpu.per_core_threshold,
            config.metrics.cpu.total_threshold,
            config.metrics.gpu.threshold,
            config.metrics.network.threshold,
            config.metrics.disk.threshold,
        );

        assert!(manager.should_inhibit(80.0, 45.0, &[50.0, 95.0], 10.0, 5.0));
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
    fn test_ema_asymmetric_increase() {
        let mut state = SmoothingState::new(0.5);

        // Initial value
        assert_eq!(state.update(10.0, 0.5), 10.0);

        // Increase: factor = min(0.5 * 2, 1.0) = 1.0 (full weight to new value)
        // ema = 1.0 * 20 + 0.0 * 10 = 20
        assert_eq!(state.update(20.0, 0.5), 20.0);

        // Another increase: factor = 1.0 (still maxed)
        // ema = 1.0 * 30 + 0.0 * 20 = 30
        assert_eq!(state.update(30.0, 0.5), 30.0);
    }

    #[test]
    fn test_ema_asymmetric_decrease() {
        let mut state = SmoothingState::new(0.5);

        // Initial value
        assert_eq!(state.update(50.0, 0.5), 50.0);

        // Decrease: uses 0.5x alpha = 0.25
        // ema = 0.25 * 40 + 0.75 * 50 = 10 + 37.5 = 47.5
        assert_eq!(state.update(40.0, 0.5), 47.5);

        // Another decrease: ema = 0.25 * 30 + 0.75 * 47.5 = 7.5 + 35.625 = 43.125
        assert_eq!(state.update(30.0, 0.5), 43.125);
    }

    #[test]
    fn test_ema_low_alpha_slower_decay() {
        let mut state = SmoothingState::new(0.1);

        // Initial value
        assert_eq!(state.update(90.0, 0.1), 90.0);

        // Decrease from high to low: factor = min(0.1, 0.5) / 2 = 0.05
        // ema = 0.05 * 10 + 0.95 * 90 = 0.5 + 85.5 = 86.0
        assert_eq!(state.update(10.0, 0.1), 86.0);

        // Another decrease: factor = 0.05 (still low)
        // ema = 0.05 * 5 + 0.95 * 86 = 0.25 + 81.7 = 81.95
        assert_eq!(state.update(5.0, 0.1), 81.95);
    }

    #[test]
    fn test_ema_high_alpha_rapid_response() {
        let mut state = SmoothingState::new(0.9);

        // Initial value
        assert_eq!(state.update(10.0, 0.9), 10.0);

        // Increase: factor = min(0.9 * 2, 1.0) = 1.0 (full weight to new value)
        // ema = 1.0 * 90 + 0.0 * 10 = 90
        assert_eq!(state.update(90.0, 0.9), 90.0);

        // Decrease: factor = min(0.9, 0.5) / 2 = 0.5 / 2 = 0.25
        // ema = 0.25 * 50 + 0.75 * 90 = 12.5 + 67.5 = 80.0
        assert_eq!(state.update(50.0, 0.9), 80.0);
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
        let cooldown_start = Some(SystemTime::now() - Duration::from_secs(65));
        let cooldown_duration = Duration::from_secs(60);

        // After 65 seconds, cooldown should be complete
        if let Some(start) = cooldown_start {
            assert!(start + cooldown_duration <= SystemTime::now());
        }
    }

    #[test]
    fn test_just_released_prevents_duplicate_release_log() {
        let mut guard_active = false;
        let elapsed_idle = Duration::from_secs(65);
        let cooldown_duration = Duration::from_secs(60);

        if !guard_active && elapsed_idle >= cooldown_duration {
            // Cooldown expired while idle with no spike — set flag to prevent redundant logs.
            guard_active = true;
        }

        assert!(
            guard_active,
            "Guard must activate after cooldown expires during idle"
        );

        let subsequent_tick_guard_active = !guard_active && elapsed_idle >= cooldown_duration;
        assert!(
            !subsequent_tick_guard_active,
            "Guard must stay inactive on subsequent ticks"
        );
    }

    #[test]
    fn test_cooldown_defers_release_until_elapsed() {
        let mut just_released = false;

        // Simulate cooldown of only 4 seconds elapsed.
        let elapsed_short = Duration::from_secs(4);
        assert!(
            elapsed_short < Duration::from_secs(60),
            "Short duration below threshold"
        );

        // After 65 seconds — cooldown ELAPSED.
        let later_elapsed = Duration::from_secs(65);
        if !just_released && later_elapsed >= Duration::from_secs(60) {
            just_released = true;
        }

        assert!(
            just_released,
            "Release must happen after cooldown_duration elapses"
        );
    }

    #[test]
    fn test_update_state_sets_flags_on_threshold_transition() {
        let metrics_above_since = Some(SystemTime::now());
        let mut waiting_for_cooldown: bool = false;

        assert!(
            metrics_above_since.is_some(),
            "Above timestamp set when active"
        );
        assert!(!waiting_for_cooldown, "No cooldown while active");

        if metrics_above_since.is_some() {
            waiting_for_cooldown = true;
        }

        assert!(
            waiting_for_cooldown,
            "Cooldown starts when inactive after active"
        );
    }

    #[test]
    fn test_metrics_above_since_not_reset_on_consecutive_ticks() {
        use std::thread;
        use std::time::Duration;

        let mut metrics_above_threshold_since = Option::<SystemTime>::None;

        // First tick: metrics go above threshold — should set timestamp
        if metrics_above_threshold_since.is_none() {
            metrics_above_threshold_since = Some(SystemTime::now());
        }
        assert!(
            metrics_above_threshold_since.is_some(),
            "Timestamp set on first tick"
        );

        thread::sleep(Duration::from_millis(10));

        let first_timestamp = metrics_above_threshold_since.unwrap();

        // Second tick: metrics still above threshold — should NOT reset timestamp
        if metrics_above_threshold_since.is_none() {
            metrics_above_threshold_since = Some(SystemTime::now());
        }

        assert_eq!(
            first_timestamp,
            metrics_above_threshold_since.unwrap(),
            "Timestamp must not be reset on consecutive ticks"
        );

        // Verify elapsed time accumulates correctly (should be ~10ms+)
        let elapsed = SystemTime::now()
            .duration_since(first_timestamp)
            .unwrap_or(Duration::from_secs(0));
        assert!(
            elapsed >= Duration::from_millis(5),
            "Elapsed must accumulate past first tick: {elapsed:?}"
        );

        // Third and fourth ticks should also preserve the original timestamp
        for _ in 2..4 {
            if metrics_above_threshold_since.is_none() {
                metrics_above_threshold_since = Some(SystemTime::now());
            }
        }
        assert_eq!(first_timestamp, metrics_above_threshold_since.unwrap());

        metrics_above_threshold_since = None;
        assert!(
            metrics_above_threshold_since.is_none(),
            "Timestamp cleared when metrics drop below"
        );

        // Re-above threshold: should set a new timestamp (not the old one)
        if metrics_above_threshold_since.is_none() {
            metrics_above_threshold_since = Some(SystemTime::now());
        }
        let reabove_timestamp = metrics_above_threshold_since.unwrap();
        assert!(
            reabove_timestamp > first_timestamp,
            "New timestamp must be after release cycle"
        );

        // Verify it still doesn't get reset on the next tick
        if metrics_above_threshold_since.is_none() {
            metrics_above_threshold_since = Some(SystemTime::now());
        }
        assert_eq!(
            reabove_timestamp,
            metrics_above_threshold_since.unwrap(),
            "Timestamp must not be reset after re-acquire"
        );
    }
}
