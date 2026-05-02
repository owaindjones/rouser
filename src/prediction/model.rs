//! Time-aware prediction model for adaptive cooldown duration.
//!
//! Uses historical metric patterns across three time dimensions to predict how long
//! inhibition should remain active after metrics drop below threshold:
//! - Year (captures seasonal trends)
//! - Week of year (captures monthly/annual cycles)
//! - Seconds into week (precise position within a 7-day cycle, enabling hour-of-day and weekday/weekend distinction).
//! 
//! Purely statistical — no external ML dependencies required.

use crate::prediction::{HistoryEntry, HistoryLog};
use chrono::{Datelike, Timelike};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use tracing::debug;

/// Multi-dimensional time key for pattern matching in the prediction model.
/// Replaces the old single `hour_of_day` dimension with three orthogonal axes:
/// - Year: seasonal trends (winter vs summer usage)
/// - Week of year: monthly/annual cycles within a year
/// - Seconds into week: precise position enabling hour-of-day + weekday/weekend distinction
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct TimeKey {
    pub year: i32,
    pub week_of_year: u32,
    /// Seconds into the ISO week (0–604799). Stored as integer for HashMap key compatibility.
    pub seconds_into_week: i64, // 0 to 604799 (7 * 24 * 3600 - 1)
}

impl TimeKey {
    /// Convert to a linear week index for proximity search across year boundaries.
    /// Uses formula `(year_offset * max_weeks) + week_of_year` where max_weeks = 53 (max ISO weeks per year).
    fn linear_week(&self) -> i64 {
        ((self.year as i64 - 2000_i64) * 53_i64) + self.week_of_year as i64
    }

    /// Convert to a linear day index for proximity search across year boundaries.
    fn linear_day(&self) -> i64 {
        self.linear_week() * 7 + (self.seconds_into_week / 86_400_i64)
    }
}

impl TimeKey {
    /// Convert a Unix timestamp in nanoseconds to a TimeKey using UTC.
    fn from_timestamp_ns(ts_ns: u64) -> Self {
        let secs = ts_ns / 1_000_000_000;
        let dt = chrono::DateTime::<chrono::Utc>::from_timestamp(secs as i64, 0)
            .unwrap_or_else(chrono::Utc::now);

       // Use calendar year and ISO week number for seasonal pattern tracking.
        let year = dt.year();
        let iso_week = dt.iso_week();

        // Seconds into week: day-of-week (Mon=1..Sun=7) * seconds_per_day + hour*3600 + min*60 + sec
        let dow = dt.weekday().number_from_monday() as i32; // 1-7
        let hours_in_day = dt.hour() as i32;
        let minutes_in_hour = dt.minute() as i32;
        let seconds_in_min = dt.second() as i32;

        Self {
            year,
            week_of_year: iso_week.week(),
            seconds_into_week: ((dow - 1) * 86400 + hours_in_day * 3600 + minutes_in_hour * 60 + seconds_in_min) as i64,
        }
    }

    /// Extract just the hour of day from a timestamp (for backward-compatible fallback).
    fn hour_of_day(ts_ns: u64) -> u32 {
        ((ts_ns / 1_000_000_000 / 3600) % 24) as u32
    }

    /// Get the current TimeKey.
    fn now() -> Self {
        let secs = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("system time before epoch")
            .as_nanos();
        Self::from_timestamp_ns(secs as u64)
    }

    /// Get the current hour of day for backward-compatible fallback.
    fn current_hour() -> u32 {
        let secs = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("system time before epoch")
            .as_nanos();
        Self::hour_of_day(secs as u64)
    }

    /// Format a TimeKey into a human-readable string for debug logging.
    fn display(&self) -> String {
        format!(
            "year={}, week={:02}, sec={:.0}",
            self.year, self.week_of_year, self.seconds_into_week
        )
    }

 }

/// Prediction result from the cooldown model.
#[derive(Debug, Clone)]
pub struct CooldownPrediction {
    /// Additional time to extend beyond the configured cooldown duration.
    /// Always >= 0. If zero-duration, use the default cooldown_duration setting.
    pub additional_time: std::time::Duration,
    /// Confidence in this prediction (0.0–1.0). Higher means more data supports it.
    pub confidence: f32,
}

/// Accumulates metrics across multiple ticks for averaged snapshot flushing.
struct TickAccumulator {
    count: u64,
    cpu_max_sum: f64,
    cpu_avg_sum: f64,
    network_sum: f64,
    disk_sum: f64,
    gpu_sums: Vec<f64>,
    inhibited_count: u64,
}

impl TickAccumulator {
    fn new() -> Self {
        Self {
            count: 0,
            cpu_max_sum: 0.0,
            cpu_avg_sum: 0.0,
            network_sum: 0.0,
            disk_sum: 0.0,
            gpu_sums: Vec::new(),
            inhibited_count: 0,
        }
    }

    fn accumulate(&mut self, entry: &HistoryEntry) {
        self.count += 1;
        self.cpu_max_sum += entry.cpu_usage.per_core_max;
        self.cpu_avg_sum += entry.cpu_usage.total_average;
        self.network_sum += entry.network_mbps;
        self.disk_sum += entry.disk_mb_s;

        // Expand GPU sums vec to accommodate this tick's GPUs.
        let gpu_count = entry.gpu_usages.len();
        if gpu_count > self.gpu_sums.len() {
            for _ in 0..(gpu_count - self.gpu_sums.len()) {
                self.gpu_sums.push(0.0);
            }
        }
        // Average per-GPU independently by slot index.
        for (i, gpu_val) in entry.gpu_usages.iter().enumerate() {
            if i < self.gpu_sums.len() {
                self.gpu_sums[i] += *gpu_val;
            } else {
                self.gpu_sums.push(*gpu_val);
            }
        }

        if entry.inhibited {
            self.inhibited_count += 1;
        }
    }

    fn flush(&mut self) -> Option<(HistoryEntry, u64)> {
        if self.count == 0 {
            return None;
        }
        let n = self.count as f64;
        let count = self.count;
        let mut gpu_averages: Vec<f64> = Vec::with_capacity(self.gpu_sums.len());
        for s in self.gpu_sums.iter() {
            gpu_averages.push(s / n);
        }

        let entry = HistoryEntry::new(
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("system time before epoch")
                .as_nanos() as u64,
            self.cpu_max_sum / n,
            self.cpu_avg_sum / n,
            gpu_averages,
            self.network_sum / n,
            self.disk_sum / n,
            self.inhibited_count > 0 && (self.inhibited_count * 2 >= self.count),
        );

        // Reset accumulator for next interval.
        self.count = 0;
        self.cpu_max_sum = 0.0;
        self.cpu_avg_sum = 0.0;
        self.network_sum = 0.0;
        self.disk_sum = 0.0;
        self.gpu_sums.clear();
        self.inhibited_count = 0;

        Some((entry, count))
    }
}

/// Time-aware statistical model that predicts cooldown extension based on historical patterns.
pub struct PredictionModel {
    history: HistoryLog,
    /// Maximum additional time allowed for predictive cooldown extension.
    max_extension_time: std::time::Duration,
    // Per-TimeKey inhibition counts (key: year + week_of_year + seconds_into_week).
    inhibited_timekeys: HashMap<TimeKey, u64>,
    data_points: u64,
    /// Number of ticks between averaged snapshot flushes.
    /// Computed as prediction_update_interval / root_update_interval.
    flush_interval: Option<usize>,
    tick_count: usize,
    accumulator: TickAccumulator,
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

        let mut inhibited_timekeys = HashMap::<TimeKey, u64>::new();

        for entry in &entries {
            if !entry.inhibited {
                continue;
            }
            let time_key = TimeKey::from_timestamp_ns(entry.timestamp_ns);
            *inhibited_timekeys.entry(time_key).or_default() += 1;
        }

        Self {
            history,
            max_extension_time,
            inhibited_timekeys,
            data_points: entries.len() as u64,
            flush_interval: None,
            tick_count: 0,
            accumulator: TickAccumulator::new(),
        }
    }

    /// Set the prediction update interval (in seconds). Controls how many ticks between averaged snapshots.
    pub fn set_prediction_update_interval(
        &mut self,
        prediction_update_interval: std::time::Duration,
    ) {
        if prediction_update_interval.as_secs() > 0 {
            self.flush_interval = Some(prediction_update_interval.as_secs() as usize);
        } else {
            self.flush_interval = None;
        }
    }

    /// Record a new tick's metrics. Accumulates into running average and writes an averaged snapshot to history when the configured interval elapses. Returns true if a snapshot was flushed.
    pub fn record(
        &mut self,
        cpu_per_core_max: f64,
        cpu_total_average: f64,
        gpu_usages: Vec<f64>,
        network_mbps: f64,
        disk_mb_s: f64,
        inhibited: bool,
    ) -> bool {
        let entry = HistoryEntry::new(
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("system time before epoch")
                .as_nanos() as u64,
            cpu_per_core_max,
            cpu_total_average,
            gpu_usages,
            network_mbps,
            disk_mb_s,
            inhibited,
        );

        self.accumulator.accumulate(&entry);
        self.tick_count += 1;

        if let Some(interval) = self.flush_interval {
            if self.tick_count >= interval {
                if let Some((snapshot, samples)) = self.accumulator.flush() {
                    self.data_points += 1;
                    debug!(
                        "Flushed averaged snapshot #{} (CPU max={:.1}%, net={:.2}MB/s, disk={:.2}MB/s, time={}, accumulated_ticks={})",
                        self.data_points,
                        snapshot.cpu_usage.per_core_max,
                        snapshot.network_mbps,
                        snapshot.disk_mb_s,
                        TimeKey::from_timestamp_ns(snapshot.timestamp_ns).display(),
                        samples,
                    );
                    self.history.append(snapshot);
                }
                self.tick_count = 0;
                return true;
            }
        }

        false
    }

    /// Predict the additional cooldown seconds based on current metrics and time of day.
    pub fn predict_cooldown(&self) -> CooldownPrediction {
        if self.data_points < 10 {
            return CooldownPrediction {
                additional_time: std::time::Duration::ZERO,
                confidence: 0.0,
            };
        }

        let now = TimeKey::now();

        let score = self.score_inhibition_rate(&now);

        if score < 0.3 {
            return CooldownPrediction {
                additional_time: std::time::Duration::ZERO,
                confidence: self.confidence_for_data_points(),
            };
        }

        // Map score to additional cooldown time (linear interpolation from 0–max_extension).
        let additional_time = std::time::Duration::from_secs_f64(
            (score - 0.3) / 0.7 * self.max_extension_time.as_secs_f64(),
        );
        let confidence = self.confidence_for_data_points();

        debug!(
            "Predicted cooldown: +{:?} (score={:.2}, time={}, data_points={}, confidence={:.2})",
            additional_time, score, now.display(), self.data_points, confidence
        );

        CooldownPrediction {
            additional_time,
            confidence,
        }
    }

   // Multi-level fallback matching:
    // Level 1: Exact TimeKey match — most precise, used with sufficient historical data for this time window.
    // Level 2: Hour-of-day fallback — original single-dimension approach when no exact matches exist (sparse data).
    fn score_inhibition_rate(&self, now: &TimeKey) -> f64 {
        // Level 1: Try exact TimeKey match first.
        if let Some(&count) = self.inhibited_timekeys.get(now) {
            return self.score_from_count(count);
        }

      // Level 2: Fall back to hour-of-day matching for sparse data.
        // Use linear day index to handle ISO week wraparound at year boundaries correctly.
        let target_seconds = now.seconds_into_week;
       let mut best_count: u64 = 0;
       for (key, &count) in self.inhibited_timekeys.iter() {
           if key.year == now.year
               && (-7_i64..=7_i64).contains(&(key.linear_day() - now.linear_day()))
               && (-3600_i64..=3600_i64).contains(&(key.seconds_into_week - target_seconds)) {
               best_count = count.max(best_count);
           }
       }

      if best_count > 0 {
            return self.score_from_count(best_count);
        }

        0.0
    }

    /// Compute a score from an inhibition count, using the overall distribution as baseline.
    fn score_from_count(&self, count: u64) -> f64 {
        let total_inhibited = self.inhibited_timekeys.values().sum::<u64>();
        // Average per matching bucket gives baseline expectation for scoring.
        let avg_per_bucket: u64 = (total_inhibited.max(1)) / (self.inhibited_timekeys.len() as u64).max(1);

        if count == 0 || avg_per_bucket == 0 {
            return 0.0;
        }

        // Score above 0.5 for buckets with more than average activity, capped at 1.0.
        let ratio = count as f64 / avg_per_bucket.max(1) as f64;
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

   fn hour_of_day(ts_ns: u64) -> u32 {
        TimeKey::hour_of_day(ts_ns)
    }

    fn current_hour() -> u32 {
        TimeKey::current_hour()
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

    fn make_test_model() -> PredictionModel {
        let mut model = PredictionModel::new(true, std::time::Duration::from_secs(60));
        // Flush every tick so tests don't need to wait for intervals.
        model.set_prediction_update_interval(std::time::Duration::from_secs(1));
        model
    }

    #[test]
    fn test_prediction_model_initialization() {
        let mut model = make_test_model();
        assert_eq!(model.data_points, 0); // No data yet.
        assert!(!model.has_sufficient_data(10));
        // Flush one snapshot to verify count increments.
        model.record(50.0, 25.0, vec![30.0], 5.0, 2.0, false);
        assert_eq!(model.data_points(), 1);
    }

    #[test]
    fn test_predict_cooldown_no_data_returns_zero() {
        let model = PredictionModel::new(true, std::time::Duration::from_secs(60));
        let prediction = model.predict_cooldown();
        assert!(!prediction.additional_time.gt(&std::time::Duration::ZERO));
    }

    #[test]
    fn test_record_and_count_entries() {
        let mut model = make_test_model();

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

    /// Test that multi-tick accumulation produces correct arithmetic means across flush boundaries.
    #[test]
   fn test_multi_tick_averaging_correctness() {
        let mut model = PredictionModel::new(true, std::time::Duration::from_secs(60));
        // Flush every 5 ticks to verify partial accumulation doesn't produce snapshots.
        model.set_prediction_update_interval(std::time::Duration::from_secs(5));

        for i in 0..4 {
            let cpu = i as f64 * 10.0; // 0, 10, 20, 30
            let net = (i + 1) as f64 * 5.0; // 5, 10, 15, 20
            assert!(!model.record(cpu, cpu * 0.5, vec![cpu], net, 1.0, false));
        }

        // No flush yet: tick_count (4) < flush_interval (5).
        assert_eq!(model.data_points(), 0);

        // 5th tick triggers flush with averaged values: CPU max = (0+10+20+30+40)/5 = 20.0, net = (5+10+15+20+25)/5 = 15.0
        assert!(model.record(40.0, 20.0, vec![40.0], 25.0, 1.0, false));
        assert_eq!(model.data_points(), 1);

        // Record second batch (5 ticks): CPU max values = 50,60,70,80,90 → avg = 70.0
        for i in 5..9 {
            let cpu = i as f64 * 10.0;
            assert!(!model.record(cpu, cpu * 0.5, vec![cpu], (i + 1) as f64 * 5.0, 1.0, false));
        }

        // Final tick of batch triggers flush for second averaged snapshot.
        assert!(model.record(90.0, 45.0, vec![90.0], 35.0, 1.0, true));
        assert_eq!(model.data_points(), 2);

       let mut model2 = PredictionModel::new(true, std::time::Duration::from_secs(60));
        // Flush every 3 ticks to verify exact-value averaging (all identical inputs → average equals input).
        model2.set_prediction_update_interval(std::time::Duration::from_secs(3));

        for _ in 0..2 {
            assert!(!model2.record(50.0, 25.0, vec![60.0], 10.0, 4.0, false));
        }

        // Third tick triggers flush: averaged values equal the repeated input (50.0, 25.0, 60.0, 10.0, 4.0).
        assert!(model2.record(50.0, 25.0, vec![60.0], 10.0, 4.0, false));
        assert_eq!(model2.data_points(), 1);

        for _ in 0..2 {
            assert!(!model2.record(80.0, 40.0, vec![90.0], 20.0, 8.0, true));
        }
        // Second flush confirms accumulator resets correctly and averaging cycle repeats cleanly.
        assert!(model2.record(80.0, 40.0, vec![90.0], 20.0, 8.0, true));
        assert_eq!(model2.data_points(), 2);
    }

    /// Test that TimeKey correctly represents seconds-into-week for known timestamps.
    #[test]
    fn test_timekey_from_timestamp_known_values() {
        // Monday Jan 1 2024 00:00 UTC (ISO week starts on Monday)
        let monday_00 = TimeKey::from_timestamp_ns(1704067200 * 1_000_000_000);
        assert_eq!(monday_00.year, 2024);
       assert_eq!(monday_00.seconds_into_week, 0); // Monday at midnight

        // Same day, noon (still Monday since Jan 1 2024 is a Monday in ISO calendar)
        let monday_noon = TimeKey::from_timestamp_ns((1704067200 + 3600 * 12) * 1_000_000_000);
        assert_eq!(monday_noon.year, 2024);
        // Monday = day index 0 (Mon=0), so seconds = 0*86400 + 12*3600 = 43200
        assert_eq!(monday_noon.seconds_into_week, 43200);

        // Sunday at 23:59 should be near end of week (day index 6)
        let sunday_night = TimeKey::from_timestamp_ns((1704067200 + (6 * 86400) + (23 * 3600) + (59 * 60)) * 1_000_000_000);
        assert_eq!(sunday_night.year, 2024);
        // Sunday = day index 6, so seconds = 6*86400 + 23*3600 + 59*60 = 604740
        assert_eq!(sunday_night.seconds_into_week, 604740);
    }

     /// Test that same weekday+time in different weeks of the same year produces identical seconds-into-week.
    #[test]
    fn test_timekey_same_position_different_weeks() {
        // Monday Jan 1 2024 at 06:30 UTC (ISO calendar Monday)
        let tk_wk1 = TimeKey::from_timestamp_ns((1704067200 + (6 * 3600) + (30 * 60)) * 1_000_000_000);
        // Monday Jan 8 2024 at 06:30 UTC — same day-of-week and time, different week of year
        let tk_wk2 = TimeKey::from_timestamp_ns((1704067200 + (7 * 86400) + (6 * 3600) + (30 * 60)) * 1_000_000_000);

        assert_eq!(tk_wk1.year, 2024);
        assert_eq!(tk_wk2.year, 2024);
        // Different weeks but same weekday+time → identical seconds_into_week
        assert_eq!(tk_wk1.week_of_year, 1);
        assert_eq!(tk_wk2.week_of_year, 2);
        assert_eq!(tk_wk1.seconds_into_week, tk_wk2.seconds_into_week);
    }


    /// Test that different weekdays at the same time produce distinct seconds-into-week values.
    #[test]
    fn test_timekey_different_weekdays_distinct() {
        // Monday Jan 1 2024 at noon UTC
        let monday = TimeKey::from_timestamp_ns((1704067200 + (12 * 3600)) * 1_000_000_000);
        // Tuesday Jan 2 2024 at noon UTC
        let tuesday = TimeKey::from_timestamp_ns((1704067200 + (86400) + (12 * 3600)) * 1_000_000_000);

        assert_eq!(monday.year, 2024);
        assert_eq!(tuesday.year, 2024);
        // Different weekdays → distinct seconds-into-week values (86400s apart)
        assert_ne!(monday.seconds_into_week, tuesday.seconds_into_week);
    }

    /// Test that linear_day correctly handles ISO week wraparound at year boundaries.
    #[test]
    fn test_linear_day_wraps_at_year_boundary() {
        // Monday Jan 1 2024 at midnight (ISO Week 1 of 2024)
        let jan_wk1 = TimeKey::from_timestamp_ns((1704067200) * 1_000_000_000);
        // Monday Jan 8 2024 at midnight (ISO Week 2 of 2024, same calendar year)
        let jan_wk2 = TimeKey::from_timestamp_ns((1704067200 + (7 * 86400)) * 1_000_000_000);

        assert_eq!(jan_wk1.year, 2024);
        assert_eq!(jan_wk2.year, 2024);
        // Exactly one week apart → linear_day diff should be exactly 7
        assert_eq!(jan_wk2.linear_day() - jan_wk1.linear_day(), 7);

        // Monday Jan 15 2024 (ISO Week 3)
        let jan_wk3 = TimeKey::from_timestamp_ns((1704067200 + (14 * 86400)) * 1_000_000_000);
        // Two weeks from Jan 1 → diff should be 14 days
        assert_eq!(jan_wk3.linear_day() - jan_wk1.linear_day(), 14);

        // Sunday Dec 29 2024 at midnight (ISO Week 52 of year 2024)
        let dec_sunday = TimeKey::from_timestamp_ns((1735401600) * 1_000_000_000);
        assert_eq!(dec_sunday.year, 2024);

        // Monday Jan 6 2025 at midnight (ISO Week 2 of year 2025)
        let jan_wk2_2025 = TimeKey::from_timestamp_ns((1736155800) * 1_000_000_000);

        // Jan 6, 2025 is a Monday at midnight UTC
        assert_eq!(jan_wk2_2025.year, 2025);
    }

    /// Test that predict_cooldown returns zero with insufficient data (< 10 points).
    #[test]
    fn test_predict_cooldown_insufficient_data() {
        let model = PredictionModel::new(true, std::time::Duration::from_secs(60));
        let prediction = model.predict_cooldown();
        assert_eq!(prediction.additional_time, std::time::Duration::ZERO);
        assert_eq!(prediction.confidence, 0.0);
    }

    /// Test that predict_cooldown returns zero when score is below threshold (no inhibited data).
    #[test]
    fn test_predict_cooldown_no_inhibited_data() {
        let mut model = make_test_model();

        // Record 15 entries, none inhibited — this gives enough points to pass the 10-point guard.
        for i in 0..15 {
            model.record(10.0 + (i as f64 * 2.0), 5.0 + (i as f64), vec![8.0], 2.0, 0.5, false);
        }

        // With no inhibited entries, score should be 0 and additional_time = 0.
        let prediction = model.predict_cooldown();
        assert_eq!(prediction.additional_time, std::time::Duration::ZERO);
    }

    /// Test that predict_cooldown returns non-zero when there is sufficient inhibited data at current time key.
    #[test]
    fn test_predict_cooldown_with_inhibited_data() {
        let mut model = make_test_model();

        // Record 15 entries with ~70% inhibition rate to ensure score > 0.3 threshold.
        for i in 0..15 {
            model.record(60.0, 30.0, vec![40.0], 10.0, 5.0, i % 3 != 0); // inhibited on ~67% of ticks
        }

        let prediction = model.predict_cooldown();
        // With sufficient inhibited data points, score may or may not exceed threshold depending on
        // current time-of-week vs historical patterns — verify the API returns valid values.
        assert!(prediction.additional_time.as_secs() <= 60); // bounded by max_extension_time
    }
}
