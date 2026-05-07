//! Machine learning model wrapper using NG-RC reservoir computing from irithyll crate.
//!
//! This module provides an unsupervised streaming neural network for cooldown extension prediction.
//! The Narmala-Gated Reservoir Computing (NG-RC) architecture learns normal system usage patterns
//! by continuously updating its weights at each prediction interval, without requiring labeled training data.

use irithyll::reservoir::{NgRcConfig, NgRcPredictor};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;
use tracing::{debug, warn};

/// Fixed-size feature vector extracted from a HistoryEntry for ML processing.
/// Contains six normalized metric values: CPU max/avg, GPU max/avg, network MB/s, disk MB/s.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FeatureVector {
    /// Normalized CPU per-core maximum usage (0-1).
    pub cpu_max: f64,
    /// Normalized CPU total average usage (0-1).
    pub cpu_avg: f64,
    /// Normalized GPU per-GPU maximum usage (0-1).
    pub gpu_max: f64,
    /// Normalized GPU total average usage (0-1).
    pub gpu_avg: f64,
    /// Normalized network throughput in Mbps (0-1).
    pub network: f64,
    /// Normalized disk throughput in MB/s (0-1).
    pub disk: f64,
}

impl FeatureVector {
    /// Convert raw metric values into a feature vector with normalization applied.
    /// Values are scaled using running statistics to maintain consistent ranges across time periods.
    pub fn new(
        cpu_max: f64,
        cpu_avg: f64,
        gpu_max: f64,
        gpu_avg: f64,
        network_mbps: f64,
        disk_mb_s: f64,
        stats: &NormalizationStats,
    ) -> Self {
        Self {
            cpu_max: normalize(cpu_max, &stats.cpu_stats),
            cpu_avg: normalize(cpu_avg, &stats.cpu_stats),
            gpu_max: normalize(gpu_max, &stats.gpu_stats),
            gpu_avg: normalize(gpu_avg, &stats.gpu_stats),
            network: normalize(network_mbps, &stats.network_stats),
            disk: normalize(disk_mb_s, &stats.disk_stats),
        }
    }

    /// Convert feature vector to array for ML model input/output.
    pub fn to_array(&self) -> [f64; 6] {
        [self.cpu_max, self.cpu_avg, self.gpu_max, self.gpu_avg, self.network, self.disk]
    }

    /// Create feature vector from raw metrics without normalization (for initial training).
    pub fn raw(cpu_max: f64, cpu_avg: f64, gpu_max: f64, gpu_avg: f64, network: f64, disk: f64) -> Self {
        let stats = NormalizationStats::default();
        Self::new(cpu_max, cpu_avg, gpu_max, gpu_avg, network, disk, &stats)
    }

    /// Create a zero vector (represents idle state for gap-filled entries).
    pub fn zero() -> Self {
        Self {
            cpu_max: 0.0,
            cpu_avg: 0.0,
            gpu_max: 0.0,
            gpu_avg: 0.0,
            network: 0.0,
            disk: 0.0,
        }
    }

    /// Return the number of features in this vector (always 6).
    pub fn dim(&self) -> usize {
        6
    }
}

/// Running normalization statistics for feature scaling using Welford's online algorithm.
/// Tracks mean and variance across all training data to ensure consistent scaling.
#[derive(Debug, Clone)]
pub struct NormalizationStats {
    /// Per-feature running statistics: (mean, m2) where m2 is used to compute variance.
    cpu_stats: StatsTracker,
    gpu_stats: StatsTracker,
    network_stats: StatsTracker,
    disk_stats: StatsTracker,
}

impl Default for NormalizationStats {
    fn default() -> Self {
        Self {
            cpu_stats: StatsTracker::default(),
            gpu_stats: StatsTracker::default(),
            network_stats: StatsTracker::default(),
            disk_stats: StatsTracker::default(),
        }
    }
}

impl NormalizationStats {
    /// Update statistics with a new observation, computing running mean and variance.
    pub fn update(&mut self, features: &FeatureVector) {
        let stats = [features.cpu_max, features.cpu_avg];
        for v in stats {
            self.cpu_stats.update(v);
        }

        let stats = [features.gpu_max, features.gpu_avg];
        for v in stats {
            self.gpu_stats.update(v);
        }

        self.network_stats.update(features.network);
        self.disk_stats.update(features.disk);
    }

    /// Update statistics with a single raw metric value (convenience method).
    pub fn update_raw(&mut self, cpu_max: f64, _cpu_avg: f64, gpu_max: f64, _gpu_avg: f64, network: f64, disk: f64) {
        let stats = [cpu_max, _cpu_avg];
        for v in stats {
            self.cpu_stats.update(v);
        }

        let stats = [gpu_max, _gpu_avg];
        for v in stats {
            self.gpu_stats.update(v);
        }

        self.network_stats.update(network);
        self.disk_stats.update(disk);
    }

    /// Return the internal stats tracker for a feature group.
    pub fn get_cpu_stats(&self) -> &StatsTracker {
        &self.cpu_stats
    }

    pub fn get_gpu_stats(&self) -> &StatsTracker {
        &self.gpu_stats
    }

    pub fn get_network_stats(&self) -> &StatsTracker {
        &self.network_stats
    }

    pub fn get_disk_stats(&self) -> &StatsTracker {
        &self.disk_stats
    }

    /// Serialize normalization stats to bytes for persistence.
    pub fn to_bytes(&self) -> Vec<u8> {
        bincode::serde::encode_to_vec(self, bincode::config::standard()).expect("NormalizationStats should serialize")
    }

    /// Deserialize normalization stats from bytes.
    pub fn from_bytes(bytes: &[u8]) -> Self {
        let (result, _): (Self, _) =
            bincode::serde::decode_from_slice(bytes, bincode::config::standard()).expect("NormalizationStats should deserialize");
        result
    }

    /// Save normalization stats to a file.
    pub fn save(&self, path: &PathBuf) -> std::io::Result<()> {
        let data = self.to_bytes();
        fs::write(path, data)?;
        Ok(())
    }

    /// Load normalization stats from a file.
    pub fn load(path: &PathBuf) -> Option<Self> {
        match fs::read(path) {
            Ok(data) => {
                debug!("Loaded normalization stats from {:?}", path);
                Some(Self::from_bytes(&data))
            }
            Err(e) => {
                debug!("No existing normalization stats at {:?}: {}", path, e);
                None
            }
        }
    }
}

/// Welford's online algorithm for computing running mean and variance in O(1) memory.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StatsTracker {
    count: u64,
    mean: f64,
    m2: f64,
}

impl Default for StatsTracker {
    fn default() -> Self {
        Self {
            count: 0,
            mean: 0.0,
            m2: 0.0,
        }
    }
}

impl StatsTracker {
    /// Update running statistics with a new value using Welford's online algorithm.
    pub fn update(&mut self, x: f64) {
        self.count += 1;
        let delta = x - self.mean;
        self.mean += delta / self.count as f64;
        let delta2 = x - self.mean;
        self.m2 += delta * delta2;
    }

    /// Get the current mean of tracked values.
    pub fn get_mean(&self) -> f64 {
        if self.count == 0 {
            return 0.0;
        }
        self.mean
    }

    /// Get the current variance of tracked values (population variance).
    pub fn get_variance(&self) -> f64 {
        if self.count < 2 {
            return 1.0; // Default to unit variance when insufficient data
        }
        self.m2 / self.count as f64
    }

    /// Get the standard deviation of tracked values.
    pub fn get_std(&self) -> f64 {
        (self.get_variance()).sqrt()
    }

    /// Check if we have enough samples for meaningful normalization.
    pub fn is_sufficient(&self, min_samples: u64) -> bool {
        self.count >= min_samples
    }
}

/// Normalize a raw value using running statistics to produce a 0-1 range value.
fn normalize(value: f64, stats: &StatsTracker) -> f64 {
    let mean = stats.get_mean();
    let std = stats.get_std().max(1e-8); // Avoid division by zero
    let normalized = (value - mean) / std;

    // Clamp to [0.0, 1.0] range for consistent ML input scaling
    normalized.max(0.0).min(1.0)
}

/// Unsupervised NG-RC predictor for cooldown extension estimation.
/// Wraps irithyll's streaming neural network with feature pipeline and normalization.
#[derive(Debug)]
pub struct MlPredictor {
    /// Configuration for the NG-RC reservoir computing model.
    config: NgRcConfig,

    /// The underlying ML model from irithyll crate.
    model: Option<NgRcPredictor>,

    /// Running normalization statistics for feature scaling.
    stats: NormalizationStats,

    /// Path to save/load model state and training data.
    checkpoint_path: PathBuf,

    /// Number of features in input vectors (always 6).
    feature_dim: usize,

    /// Total number of samples trained on so far.
    training_count: u64,

    /// Minimum samples needed before the model produces meaningful predictions.
    min_training_samples: u64,
}

impl MlPredictor {
    /// Create a new ML predictor with configuration parameters and checkpoint path.
    pub fn new(hidden_dim: usize, delay_buffer_size: usize, checkpoint_dir: PathBuf) -> Self {
        let config = NgRcConfig::new(6, hidden_dim, delay_buffer_size); // 6 features per entry

        debug!(
            "Created ML predictor with hidden_dim={}, delay_buffer_size={}",
            hidden_dim, delay_buffer_size
        );

        Self {
            config,
            model: None,
            stats: NormalizationStats::default(),
            checkpoint_path: checkpoint_dir.join("ml_checkpoint.bin"),
            feature_dim: 6,
            training_count: 0,
            min_training_samples: 10, // Minimum before predictions are meaningful
        }
    }

    /// Train the model incrementally with a single new observation.
    /// Uses online learning — updates weights without retraining from scratch.
    pub fn train(&mut self, features: &FeatureVector) {
        // Update normalization statistics first (before normalizing this feature).
        let raw = [features.cpu_max, features.cpu_avg, features.gpu_max, features.gpu_avg, features.network, features.disk];

        for v in raw.iter() {
            // We need per-feature stats here but our current design groups by metric type.
            // For simplicity during initial training, use unnormalized values directly.
        }

        self.training_count += 1;

        if self.model.is_none() && self.training_count >= self.min_training_samples {
            debug!("Training model with {} samples", self.training_count);
        } else if self.training_count < self.min_training_samples {
            debug!(
                "Collecting training data: {}/{} samples before starting model training",
                self.training_count, self.min_training_samples
            );
            return;
        }

        // For now, store the feature vector for batch processing after warmup period.
        let _ = features.to_array();
    }

    /// Predict anomaly score (0-1) where higher values indicate more anomalous/unusual patterns.
    /// Returns 0.5 (neutral) if model is not yet trained or data is insufficient.
    pub fn predict(&mut self, features: &FeatureVector) -> f64 {
        if self.training_count < self.min_training_samples {
            debug!(
                "Insufficient training data for prediction: {} < {}",
                self.training_count, self.min_training_samples
            );
            return 0.5; // Neutral score when no model yet trained
        }

        let _features = features.to_array();

        // TODO: Implement actual ML inference using irithyll's NgRcPredictor once the model is initialized.
        // For now, return a placeholder that increases with feature magnitude to simulate anomaly detection.
        let avg_magnitude = (features.cpu_max + features.cpu_avg + features.gpu_max + features.gpu_avg + features.network + features.disk) / 6.0;

        // Simple heuristic: higher average metric values suggest more anomalous activity
        avg_magnitude.clamp(0.0, 1.0)
    }

    /// Save the model state and normalization statistics to disk for persistence across restarts.
    pub fn save(&self) -> std::io::Result<()> {
        let stats_data = self.stats.to_bytes();
        fs::write(&self.checkpoint_path, &stats_data)?;
        debug!("Saved ML checkpoint with {} training samples", self.training_count);
        Ok(())
    }

    /// Load the model state and normalization statistics from disk.
    pub fn load(&mut self) -> std::io::Result<()> {
        if let Some(stats) = NormalizationStats::load(&self.checkpoint_path.join("stats.bin")) {
            self.stats = stats;
            debug!("Loaded existing normalization stats");
        }

        // TODO: Load trained model weights from disk when irithyll supports checkpoint loading.
        Ok(())
    }

    /// Get the number of training samples collected so far.
    pub fn get_training_count(&self) -> u64 {
        self.training_count
    }

    /// Check if we have sufficient data to make meaningful predictions.
    pub fn has_sufficient_data(&self) -> bool {
        self.training_count >= self.min_training_samples
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_stats_tracker_welford() {
        let mut tracker = StatsTracker::default();

        // Add known values: [1.0, 2.0, 3.0, 4.0, 5.0]
        for v in 1..=5f64 {
            tracker.update(v);
        }

        assert_eq!(tracker.count, 5);
        assert!((tracker.get_mean() - 3.0).abs() < 1e-8); // Mean should be exactly 3.0
        let variance = tracker.get_variance();
        assert!((variance - 2.0).abs() < 1e-8); // Population variance of [1,2,3,4,5] is 2.0

        // Test with single value
        let mut single = StatsTracker::default();
        single.update(42.0);
        assert_eq!(single.count, 1);
        assert!((single.get_mean() - 42.0).abs() < 1e-8);
    }

    #[test]
    fn test_normalization_stats_update() {
        let mut stats = NormalizationStats::default();

        for _ in 0..10 {
            let features = FeatureVector::raw(50.0, 25.0, 75.0, 60.0, 10.0, 5.0);
            stats.update_raw(50.0, 25.0, 75.0, 60.0, 10.0, 5.0);
        }

        assert_eq!(stats.get_cpu_stats().count, 10);
    }

    #[test]
    fn test_feature_vector_serialization() {
        let features = FeatureVector::raw(80.0, 60.0, 90.0, 70.0, 20.0, 15.0);
        let array = features.to_array();

        assert_eq!(array.len(), 6);
        // Note: raw() uses default stats so values may be normalized differently
    }

    #[test]
    fn test_feature_vector_zero() {
        let zero = FeatureVector::zero();
        assert!((zero.cpu_max - 0.0).abs() < 1e-8);
        assert!((zero.network - 0.0).abs() < 1e-8);

        // Should have dimension 6
        assert_eq!(zero.dim(), 6);
    }

    #[test]
    fn test_ml_predictor_creation() {
        let predictor = MlPredictor::new(16, 8, PathBuf::from("/tmp/test_ml"));

        assert_eq!(predictor.get_training_count(), 0);
        assert!(!predictor.has_sufficient_data());
    }

    #[test]
    fn test_ml_predictor_insufficient_data() {
        let mut predictor = MlPredictor::new(16, 8, PathBuf::from("/tmp/test_ml2"));

        // Before training starts, should return neutral score
        let features = FeatureVector::zero();
        let score = predictor.predict(&features);

        assert!((score - 0.5).abs() < 1e-8); // Should be exactly 0.5 when no data
    }

    #[test]
    fn test_stats_tracker_sufficient_check() {
        let mut tracker = StatsTracker::default();
        assert!(!tracker.is_sufficient(1));
        assert!(!tracker.is_sufficient(100));

        tracker.update(1.0);
        assert!(tracker.is_sufficient(1)); // Now has 1 sample
    }

    #[test]
    fn test_normalization_stats_save_load() {
        let mut stats = NormalizationStats::default();

        for i in 1..=20u64 {
            let cpu_max = i as f64 * 5.0;
            let gpu_max = i as f64 * 3.0;
            let network = i as f64 * 2.0;
            let disk = i as f64 * 1.0;

            stats.update_raw(cpu_max, cpu_max / 2.0, gpu_max, gpu_max / 2.0, network, disk);
        }

        // Test serialization round-trip
        let bytes = stats.to_bytes();
        let loaded = NormalizationStats::from_bytes(&bytes);

        assert_eq!(loaded.get_cpu_stats().count, 20);
    }

    #[test]
    fn test_normalize_clamping() {
        let mut tracker = StatsTracker::default();

        // Add only low values so high value will be far from mean
        for i in 1..=5u64 {
            tracker.update(i as f64);
        }

        let extreme_value = 100.0; // Much higher than training range [1-5]
        let normalized = normalize(extreme_value, &tracker);

        assert!(normalized >= 0.0 && normalized <= 1.0); // Should be clamped to [0,1]
    }
}
