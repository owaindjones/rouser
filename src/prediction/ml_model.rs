//! Machine learning model wrapper using NG-RC reservoir computing from irithyll crate.
//!
//! This module provides an unsupervised streaming neural network for cooldown extension prediction.
//! The Next Generation Reservoir Computing (NG-RC) architecture learns normal system usage patterns
//! by continuously updating its weights at each prediction interval, without requiring labeled training data.

use irithyll::{
    reservoir::{NGRCConfig, NextGenRC},
    StreamingLearner,
};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;
use tracing::debug;

use crate::prediction::HistoryEntry;

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
        [
            self.cpu_max,
            self.cpu_avg,
            self.gpu_max,
            self.gpu_avg,
            self.network,
            self.disk,
        ]
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
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct NormalizationStats {
    cpu_stats: StatsTracker,
    gpu_stats: StatsTracker,
    network_stats: StatsTracker,
    disk_stats: StatsTracker,
}

impl NormalizationStats {
    /// Update statistics with a new observation.
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

    /// Serialize normalization stats to bytes for persistence.
    pub fn to_bytes(&self) -> Vec<u8> {
        bincode::serde::encode_to_vec(self, bincode::config::standard())
            .expect("NormalizationStats should serialize")
    }

    /// Deserialize normalization stats from bytes.
    pub fn from_bytes(bytes: &[u8]) -> Option<Self> {
        let (result, _): (Self, _) =
            bincode::serde::decode_from_slice(bytes, bincode::config::standard()).ok()?;
        Some(result)
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
                if let Some(stats) = Self::from_bytes(&data) {
                    debug!("Loaded normalization stats from {:?}", path);
                    return Some(stats);
                }
                debug!("Corrupted checkpoint data at {:?}", path);
                None
            }
            Err(e) => {
                debug!("No existing normalization stats at {:?}: {}", path, e);
                None
            }
        }
    }

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

    pub fn get_cpu_stats_mut(&mut self) -> &mut StatsTracker {
        &mut self.cpu_stats
    }

    pub fn get_gpu_stats_mut(&mut self) -> &mut StatsTracker {
        &mut self.gpu_stats
    }

    pub fn get_network_stats_mut(&mut self) -> &mut StatsTracker {
        &mut self.network_stats
    }

    pub fn get_disk_stats_mut(&mut self) -> &mut StatsTracker {
        &mut self.disk_stats
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
            return 1.0;
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

/// Normalize a raw value using running statistics to produce a scaled value.
fn normalize(value: f64, stats: &StatsTracker) -> f64 {
    let mean = stats.get_mean();
    let std = stats.get_std().max(1e-8);
    let normalized = (value - mean) / std;
    normalized.clamp(0.0, 1.0)
}

/// Unsupervised NG-RC predictor for cooldown extension estimation.
#[derive(Debug)]
pub struct MlPredictor {
    config: NGRCConfig,
    model: NextGenRC,
    stats: NormalizationStats,
    checkpoint_path: PathBuf,
    training_count: u64,
}

impl MlPredictor {
    /// Create a new ML predictor. The NG-RC model uses hardcoded constants tuned for system metrics anomaly detection:
    /// - k=5: looks back at the last 5 snapshots (temporal context window)
    /// - degree=2: quadratic polynomial features capture nonlinear relationships in CPU/GPU/network/disk patterns
    pub fn new(checkpoint_dir: PathBuf) -> Self {
        let config = NGRCConfig::builder()
            .k(5) // Look back at last 5 snapshots for temporal context
            .s(1)
            .degree(2) // Quadratic polynomial features (minimum allowed, sufficient for anomaly detection)
            .build()
            .expect("valid NGRC config");

        let model = NextGenRC::new(config.clone());

        debug!("Created ML predictor with temporal_window=5, degree=2");

        if let Err(e) = fs::create_dir_all(&checkpoint_dir) {
            debug!(
                "Failed to create checkpoint directory {:?}: {}",
                checkpoint_dir, e
            );
        }

        Self {
            config,
            model,
            stats: NormalizationStats::default(),
            checkpoint_path: checkpoint_dir.join("ml_checkpoint.bin"),
            training_count: 0,
        }
    }

    /// Train the model incrementally with a single new observation.
    pub fn train(&mut self, features: &FeatureVector) {
        let array = features.to_array();
        self.stats.update(features);
        let target = array[0];
        self.model.train_one(&array, target, 1.0);
        self.training_count += 1;

        if self.training_count.is_multiple_of(50) {
            debug!("Trained ML model on {} samples", self.training_count);
        }
    }

    pub fn predict_raw(&mut self, features: &[f64]) -> f64 {
        if self.training_count < 20 {
            debug!(
                "Insufficient training data for prediction: {} < {}",
                self.training_count, 20
            );
            return 0.5;
        }

        let predicted = self.model.predict_batch(&[features]).clone()[0];
        let actual = features[0];

        // Anomaly score is based on prediction error (residual) normalized to [0, 1]
        let residual = (actual - predicted).abs();
        let std = self.stats.get_cpu_stats().get_std().max(1e-8);

        // Normalize residual by training distribution's standard deviation
        let anomaly_score = (residual / std).min(3.0) / 3.0;

        debug!(
            "ML predict: actual={:.2}, predicted={:.2}, residual={:.2}, anomaly={:.3}",
            actual, predicted, residual, anomaly_score
        );

        anomaly_score
    }

    pub fn predict(&mut self, features: &FeatureVector) -> f64 {
        let array = features.to_array();
        self.predict_raw(&array)
    }

    /// Save the model state and normalization statistics to disk.
    pub fn save(&self) -> std::io::Result<()> {
        let stats_data = self.stats.to_bytes();
        fs::write(&self.checkpoint_path, &stats_data)?;
        debug!(
            "Saved ML checkpoint with {} training samples",
            self.training_count
        );
        Ok(())
    }

    pub fn load(&mut self) -> std::io::Result<()> {
        if let Some(stats) = NormalizationStats::load(&self.checkpoint_path) {
            self.stats = stats;
            debug!("Loaded existing normalization stats from checkpoint");
        }

        let _checkpoint_data = fs::read(&self.checkpoint_path);
        Ok(())
    }

    /// Get the number of training samples collected so far.
    pub fn get_training_count(&self) -> u64 {
        self.training_count
    }

    pub fn train_raw(&mut self, features: &[f64]) {
        self.model.train_one(features, features[0], 1.0);
        self.training_count += 1;

        if self.training_count.is_multiple_of(50) {
            debug!("Trained ML model on {} samples", self.training_count);
        }
    }

    pub fn update_stats(
        &mut self,
        cpu_max: f64,
        cpu_avg: f64,
        gpu_max: f64,
        gpu_avg: f64,
        network_mbps: f64,
        disk_mb_s: f64,
    ) {
        self.stats.get_cpu_stats_mut().update(cpu_max);
        self.stats.get_cpu_stats_mut().update(cpu_avg);
        self.stats.get_gpu_stats_mut().update(gpu_max);
        self.stats.get_gpu_stats_mut().update(gpu_avg);
        self.stats.get_network_stats_mut().update(network_mbps);
        self.stats.get_disk_stats_mut().update(disk_mb_s);
    }

    pub fn train_from_history(&mut self, entries: &[HistoryEntry]) {
        if entries.is_empty() {
            return;
        }

        let count = entries.len();
        for entry in entries {
            self.stats
                .get_cpu_stats_mut()
                .update(entry.cpu_usage.per_core_max);
            self.stats
                .get_cpu_stats_mut()
                .update(entry.cpu_usage.total_average);
            self.stats
                .get_gpu_stats_mut()
                .update(entry.gpu_usage.per_gpu_max);
            self.stats
                .get_gpu_stats_mut()
                .update(entry.gpu_usage.total_average);
            self.stats
                .get_network_stats_mut()
                .update(entry.network_mbps);
            self.stats.get_disk_stats_mut().update(entry.disk_mb_s);

            let array = [
                entry.cpu_usage.per_core_max,
                entry.cpu_usage.total_average,
                entry.gpu_usage.per_gpu_max,
                entry.gpu_usage.total_average,
                entry.network_mbps,
                entry.disk_mb_s,
            ];
            self.model.train_one(&array, array[0], 1.0);
            self.training_count += 1;
        }

        debug!("Trained ML predictor on {} historical entries", count);
    }

    /// Check if we have sufficient data to make meaningful predictions.
    pub fn has_sufficient_data(&self) -> bool {
        self.training_count >= 20
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_stats_tracker_welford() {
        let mut tracker = StatsTracker::default();

        let values: [f64; 5] = [1.0, 2.0, 3.0, 4.0, 5.0];
        for v in &values {
            tracker.update(*v);
        }

        assert_eq!(tracker.count, 5);
        assert!((tracker.get_mean() - 3.0).abs() < 1e-8);
        let variance = tracker.get_variance();
        assert!((variance - 2.0).abs() < 1e-8);

        let mut single = StatsTracker::default();
        single.update(42.0);
        assert_eq!(single.count, 1);
        assert!((single.get_mean() - 42.0).abs() < 1e-8);
    }

    #[test]
    fn test_normalization_stats_update() {
        let mut stats = NormalizationStats::default();

        for _ in 0..5 {
            let features = FeatureVector::zero();
            stats.update(&features);
        }

        assert_eq!(stats.get_cpu_stats().count, 10);
    }

    #[test]
    fn test_feature_vector_zero() {
        let zero = FeatureVector::zero();
        assert!((zero.cpu_max - 0.0).abs() < 1e-8);
        assert!((zero.network - 0.0).abs() < 1e-8);
        assert_eq!(zero.dim(), 6);
    }

    #[test]
    fn test_ml_predictor_creation() {
        let predictor = MlPredictor::new(PathBuf::from("/tmp/test_ml"));

        assert_eq!(predictor.get_training_count(), 0);
        assert!(!predictor.has_sufficient_data());
    }

    #[test]
    fn test_stats_tracker_sufficient_check() {
        let mut tracker = StatsTracker::default();
        assert!(!tracker.is_sufficient(1));

        tracker.update(1.0);
        assert!(tracker.is_sufficient(1));
    }

    #[test]
    fn test_normalization_stats_save_load() {
        let mut stats = NormalizationStats::default();

        for i in 1..=10u64 {
            let cpu_max = i as f64 * 5.0;
            let features = FeatureVector::new(cpu_max, cpu_max / 2.0, 0.0, 0.0, 0.0, 0.0, &stats);
            stats.update(&features);
        }

        let bytes = stats.to_bytes();
        let loaded =
            NormalizationStats::from_bytes(&bytes).expect("should deserialize valid bytes");

        assert_eq!(loaded.get_cpu_stats().count, 20);
    }
}
