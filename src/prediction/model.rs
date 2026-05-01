//! Time-aware prediction model for adaptive cooldown duration.
//!
//! Uses historical metric patterns (hour-of-day analysis) to predict how long
//! inhibition should remain active after metrics drop below threshold.
//! Purely statistical — no external ML dependencies required.

use crate::prediction::{HistoryEntry, HistoryLog};
use std::collections::HashMap;
use tracing::debug;

/// Prediction result from the cooldown model.
#[derive(Debug, Clone)]
pub struct CooldownPrediction {
    /// Additional time to extend beyond the configured cooldown duration.
    /// Always >= 0. If zero-duration, use the default cooldown_duration setting.
    pub additional_time: std::time::Duration,
    /// Confidence in this prediction (0.0–1.0). Higher means more data supports it.
    pub confidence: f32,
}

/// Time-aware statistical model that predicts cooldown extension based on historical patterns.
pub struct PredictionModel {
    history: HistoryLog,
    /// Maximum additional time allowed for predictive cooldown extension.
    max_extension_time: std::time::Duration,
    // Per-hour high-activity counts for CPU and network (key: hour_of_day 0–23).
    cpu_high_count: HashMap<u32, u64>,
    network_high_count: HashMap<u32, u64>,
    data_points: u64,
}

impl PredictionModel {
    /// Create a new prediction model. Loads existing history if available.
    pub fn new(is_root: bool, max_extension_time: std::time::Duration) -> Self {
        let history = HistoryLog::new(is_root);
        let entries = history.read_all();
        debug!(
            "Prediction model initialized with {} historical data points",
            entries.len()
        );

        let mut cpu_high_count = HashMap::<u32, u64>::new();
        let mut network_high_count = HashMap::<u32, u64>::new();

        for entry in &entries {
            let hour_u32 = Self::hour_of_day(entry.timestamp_ns);

            // Track hours where metrics exceeded typical thresholds.
            if entry.cpu_usage.per_core_max > 50.0 {
                *cpu_high_count.entry(hour_u32).or_default() += 1;
            }
            if entry.network_mbps > 10.0 || entry.disk_mb_s > 5.0 {
                *network_high_count.entry(hour_u32).or_default() += 1;
            }
        }

        Self {
            history,
            max_extension_time,
            cpu_high_count,
            network_high_count,
            data_points: entries.len() as u64,
        }
    }

    /// Record a new metric snapshot. Called on each tick when metrics are collected.
    pub fn record(
        &mut self,
        cpu_per_core_max: f64,
        _cpu_total_average: f64,
        _gpu_usages: Vec<f64>,
        network_mbps: f64,
        disk_mb_s: f64,
        inhibited: bool,
    ) {
        let now = std::time::SystemTime::now();
        let ns = now
            .duration_since(std::time::UNIX_EPOCH)
            .expect("system time before epoch")
            .as_nanos() as u64;

        self.history.append(HistoryEntry::new(
            ns,
            cpu_per_core_max,
            _cpu_total_average,
            _gpu_usages,
            network_mbps,
            disk_mb_s,
            inhibited,
        ));
        debug!(
            "Recorded data point #{} (CPU max={:.1}%, net={:.2}MB/s, disk={:.2}MB/s, hour={})",
            self.data_points + 1,
            cpu_per_core_max,
            network_mbps,
            disk_mb_s,
            Self::hour_of_day(ns),
        );
        self.data_points += 1;
    }

    /// Predict the additional cooldown seconds based on current metrics and time of day.
    pub fn predict_cooldown(&self) -> CooldownPrediction {
        if self.data_points < 10 {
            return CooldownPrediction {
                additional_time: std::time::Duration::ZERO,
                confidence: 0.0,
            };
        }

        let hour_of_day = Self::current_hour();

        // Score each metric dimension (higher = more likely to stay active at this hour).
        let cpu_score = self.score_metric_hour(hour_of_day, &self.cpu_high_count);
        let network_score = self.score_metric_hour(hour_of_day, &self.network_high_count);

        // Weighted combination: CPU is primary signal; network is secondary.
        let combined_score = (cpu_score * 0.6 + network_score * 0.4).min(1.0);

        if combined_score < 0.3 {
            return CooldownPrediction {
                additional_time: std::time::Duration::ZERO,
                confidence: self.confidence_for_data_points(),
            };
        }

        // Map score to additional cooldown time (linear interpolation from 0–max_extension).
        let additional_time = std::time::Duration::from_secs_f64(
            (combined_score - 0.3) / 0.7 * self.max_extension_time.as_secs_f64(),
        );
        let confidence = self.confidence_for_data_points();

        debug!(
            "Predicted cooldown: +{:?} (score={:.2}, hour={}, data_points={}, confidence={:.2})",
            additional_time, combined_score, hour_of_day, self.data_points, confidence
        );

        CooldownPrediction {
            additional_time,
            confidence,
        }
    }

    /// Score a metric dimension based on historical frequency at this hour.
    fn score_metric_hour(&self, hour: u32, counts: &HashMap<u32, u64>) -> f64 {
        let count = counts.get(&hour).copied().unwrap_or(0);
        if count == 0 {
            return 0.0;
        }

        // Average per hour across all data points gives baseline expectation.
        let avg_per_hour: u64 =
            self.data_points / 24.max(self.cpu_high_count.values().sum::<u64>() + 1);
        if avg_per_hour == 0 {
            return 0.0;
        }

        // Score above 0.5 for hours with more than average activity, capped at 1.0.
        let ratio = count as f64 / avg_per_hour.max(1) as f64;
        (ratio * 0.5).min(1.0)
    }

    /// Compute confidence based on total data points available.
    fn confidence_for_data_points(&self) -> f32 {
        match self.data_points {
            n if n < 50 => 0.1,
            n if n < 500 => 0.3,
            n if n < 5_000 => 0.6,
            _ => 0.9,
        }
    }

    /// Extract hour of day (0–23 UTC) from a Unix timestamp in nanoseconds.
    fn hour_of_day(ts_ns: u64) -> u32 {
        ((ts_ns / 1_000_000_000 / 3600) % 24) as u32
    }

    /// Get the current hour of day (UTC).
    fn current_hour() -> u32 {
        Self::hour_of_day(
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("system time before epoch")
                .as_nanos() as u64,
        )
    }

    /// Get the current history log reference for manual writes (e.g., during integration).
    #[allow(dead_code)]
    pub fn get_history(&self) -> &HistoryLog {
        &self.history
    }

    pub fn prune(&mut self, max_age: std::time::Duration) {
        debug!("Running history pruning (max age: {:?})", max_age);
        self.history.prune(max_age);
    }

    /// Check if we have enough data to make meaningful predictions.
    #[allow(dead_code)] // Used in service.rs
    pub fn has_sufficient_data(&self, min_points: u64) -> bool {
        self.data_points >= min_points
    }

    /// Return the number of historical data points collected so far.
    #[allow(dead_code)]
    pub fn data_points(&self) -> u64 {
        self.data_points
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_prediction_model_initialization() {
        let model = PredictionModel::new(true, std::time::Duration::from_secs(60));
        assert_eq!(model.data_points, 0); // No data yet.
        assert!(!model.has_sufficient_data(10));
    }

    #[test]
    fn test_predict_cooldown_no_data_returns_zero() {
        let model = PredictionModel::new(true, std::time::Duration::from_secs(60));
        let prediction = model.predict_cooldown();
        assert!(!prediction.additional_time.gt(&std::time::Duration::ZERO));
    }

    #[test]
    fn test_record_and_count_entries() {
        let mut model = PredictionModel::new(true, std::time::Duration::from_secs(60));

        for i in 0..5 {
            model.record(
                60.0 + (i as f64 * 2.0),
                30.0 + (i as f64),
                vec![70.0],
                15.0,
                8.0,
                i % 2 == 0, // alternate inhibited/not-inhibited
            );
        }

        assert_eq!(model.data_points(), 5);
    }

    #[test]
    fn test_predict_cooldown_with_insufficient_data() {
        let model = PredictionModel::new(true, std::time::Duration::from_secs(60));
        let prediction = model.predict_cooldown();
        // Should return zero additional time and low confidence with no data.
        assert_eq!(prediction.additional_time, std::time::Duration::ZERO);
        assert!(prediction.confidence < 0.5);
    }

    #[test]
    fn test_hour_of_day() {
        // Unix epoch (Jan 1, 1970 00:00:00 UTC) is hour 0.
        assert_eq!(PredictionModel::hour_of_day(0), 0);
        // Jan 1, 1970 12:00:00 UTC = 43200 seconds.
        assert_eq!(PredictionModel::hour_of_day(43_200_000_000_000), 12);
    }

    #[test]
    fn test_current_hour_valid_range() {
        let hour = PredictionModel::current_hour();
        assert!((0..=23).contains(&hour));
    }
}
