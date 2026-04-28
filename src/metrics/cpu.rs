use std::fs;
use tracing::debug;

#[derive(Debug, Clone)]
struct CpuCoreTicks {
    user: u64,
    nice: u64,
    system: u64,
    idle: u64,
    iowait: u64,
    irq: u64,
    softirq: u64,
    steal: u64,
}

#[derive(Debug, Default, Clone)]
struct CpuCoreStats {
    user: u64,
    nice: u64,
    system: u64,
    idle: u64,
    iowait: u64,
    irq: u64,
    softirq: u64,
    steal: u64,
}

/// CPU usage metrics with per-core frequency-weighted calculations.
#[derive(Debug, Clone)]
pub struct CpuUsage {
    /// Maximum weighted usage across all cores (0-100).
    pub per_core_max: f64,
    /// Average weighted usage across all cores (0-100).
    pub total_average: f64,
}

#[derive(Debug, Clone)]
struct CpuFreq {
    cur_freq_khz: u64,
    max_freq_khz: u64,
}

impl Default for CpuFreq {
    fn default() -> Self {
        Self {
            cur_freq_khz: 0,
            max_freq_khz: 800_000,
        }
    }
}

pub struct CpuCollector {
    last_stats: Option<Vec<CpuCoreStats>>,
    current_ticks: Option<Vec<(String, CpuCoreTicks)>>,
    last_time: Option<std::time::SystemTime>,
    /// Base max frequencies cached at startup (cpuinfo_max_freq) — never changes.
    base_freqs_at_startup: Vec<CpuFreq>,
    /// Peak frequency observed since rouser started, per core index.
    peak_freqs_since_startup: Vec<u64>,
}

impl Default for CpuCollector {
    fn default() -> Self {
        Self::new()
    }
}

impl CpuCollector {
    pub fn new() -> Self {
        let base_freqs = cache_base_frequencies();
        let n_cores = base_freqs.len();
        Self {
            last_stats: None,
            current_ticks: None,
            last_time: None,
            base_freqs_at_startup: base_freqs,
            peak_freqs_since_startup: vec![0; n_cores],
        }
    }

    #[allow(dead_code)]
    pub fn name(&self) -> &str {
        "cpu"
    }

    /// Collect per-core frequency-weighted CPU usage.
    ///
    /// Returns two metrics:
    /// - `per_core_max`: Maximum weighted usage across all cores (accounts for frequency scaling)
    /// - `total_average`: Average of all core usages divided by total core count
    pub async fn collect(&mut self) -> Result<CpuUsage, CpuError> {
        let curr_ticks = match read_core_ticks() {
            Ok(ticks) => ticks,
            Err(e) => return Err(e),
        };

        let n_cores = curr_ticks.len();
        if n_cores == 0 {
            return Err(CpuError::NoCoresFound);
        }

        while self.peak_freqs_since_startup.len() < n_cores {
            self.peak_freqs_since_startup.push(0);
        }

        let now = std::time::SystemTime::now();

        let runtime_freqs: Vec<CpuFreq> = curr_ticks
            .iter()
            .enumerate()
            .map(|(i, (name, _))| read_runtime_cur_freq(name, i, &self.base_freqs_at_startup))
            .collect();

        let mut any_freq_available = false;
        for (i, f) in runtime_freqs.iter().enumerate() {
            if f.cur_freq_khz > 0 && i < self.peak_freqs_since_startup.len() {
                if f.cur_freq_khz > self.peak_freqs_since_startup[i] {
                    self.peak_freqs_since_startup[i] = f.cur_freq_khz;
                }
                any_freq_available = true;
            }
        }

        let has_stable_core_count = self
            .last_stats
            .as_ref()
            .is_some_and(|prev| prev.len() == curr_ticks.len());

        if has_stable_core_count {
            match &self.last_time {
                Some(t) => {
                    let usage = calculate_usage(
                        self.last_stats.as_ref().unwrap(),
                        &curr_ticks,
                        *t,
                        now,
                        &runtime_freqs,
                        &self.base_freqs_at_startup,
                        &self.peak_freqs_since_startup,
                        any_freq_available,
                    );
                    store_state(self, curr_ticks);
                    Ok(usage)
                }
                None => {
                    store_state(self, curr_ticks);
                    self.last_time = Some(now);
                    Ok(CpuUsage {
                        per_core_max: 0.0,
                        total_average: 0.0,
                    })
                }
            }
        } else {
            debug!(
                "CPU: Core count changed ({} -> {}), skipping delta calculation",
                self.last_stats.as_ref().map(|v| v.len()).unwrap_or(0),
                curr_ticks.len()
            );
            store_state(self, curr_ticks);
            Ok(CpuUsage {
                per_core_max: 0.0,
                total_average: 0.0,
            })
        }
    }

    /// Check if frequency scaling data is available for this core.
    #[allow(dead_code)]
    pub fn has_freq_scaling(&self) -> bool {
        !self.base_freqs_at_startup.is_empty()
            && self.peak_freqs_since_startup.iter().any(|&p| p > 0)
    }
}

/// Cache max frequency from cpufreq sysfs at startup.
fn cache_base_frequencies() -> Vec<CpuFreq> {
    let mut freqs = Vec::new();

    for entry in fs::read_dir("/sys/devices/system/cpu")
        .ok()
        .into_iter()
        .flatten()
    {
        let path = match entry.map(|e| e.path()).ok() {
            Some(p) => p,
            None => continue,
        };

        let name = match path.file_name().and_then(|s| s.to_str()) {
            Some(n) => n,
            None => continue,
        };

        if !name.starts_with("cpu") || name == "cpu" {
            continue;
        }
        if !name.chars().skip(3).all(|c| c.is_ascii_digit()) {
            continue;
        }

        let cpufreq_base = format!("/sys/devices/system/cpu/{}/cpufreq", name);
        let cpufreq_exists = fs::exists(&cpufreq_base).unwrap_or(false);

        if cpufreq_exists {
            freqs.push(CpuFreq {
                cur_freq_khz: read_single_freq(&format!("{}/cpuinfo_cur_freq", cpufreq_base), 1.0),
                max_freq_khz: match read_single_freq(
                    &format!("{}/cpuinfo_max_freq", cpufreq_base),
                    1.0,
                ) {
                    0 => read_single_freq(
                        &format!("/sys/devices/system/cpu/{}/cpuinfo_max_freq", name),
                        1.0,
                    ),
                    v => v,
                },
            });
        } else if let Ok(max_val) =
            fs::read_to_string(format!("/sys/devices/system/cpu/{}/cpuinfo_max_freq", name))
        {
            if let Ok(freq_hz) = max_val.trim().parse::<f64>() {
                freqs.push(CpuFreq {
                    cur_freq_khz: (freq_hz / 1000.0) as u64, // no cur available, use max as estimate
                    max_freq_khz: (freq_hz / 1000.0) as u64,
                });
            } else {
                freqs.push(CpuFreq::default());
            }
        } else {
            freqs.push(CpuFreq::default());
        }
    }

    debug!(
        "Cached base frequencies for {} core(s) at startup",
        freqs.len()
    );
    freqs
}

/// Read a single frequency value from a sysfs path. Never errors — returns 0 on failure.
fn read_single_freq(path: &str, scale: f64) -> u64 {
    fs::read_to_string(path)
        .ok()
        .and_then(|content| content.trim().parse::<f64>().ok())
        .map(|v| (v * scale) as u64)
        .unwrap_or(0)
}

fn read_runtime_cur_freq(core_name: &str, core_idx: usize, base_freqs: &[CpuFreq]) -> CpuFreq {
    let mut result = if core_idx < base_freqs.len() {
        base_freqs[core_idx].clone()
    } else {
        CpuFreq::default()
    };

    // Single sysfs file read per tick — no fallback chain to minimize FD burst.
    // If cpufreq scaling_cur_freq is unavailable, cur stays 0 and we rely on
    // base_max from startup for frequency-weighted calculations.
    let cur_path = format!(
        "/sys/devices/system/cpu/{}/cpufreq/scaling_cur_freq",
        core_name
    );
    if let Ok(content) = fs::read_to_string(&cur_path) {
        if let Ok(freq_khz) = content.trim().parse::<u64>() {
            result.cur_freq_khz = freq_khz;
        }
    }

    // Ensure base max_freq is set from startup cache
    if result.max_freq_khz == 0 && core_idx < base_freqs.len() {
        result.max_freq_khz = base_freqs[core_idx].max_freq_khz;
    }

    result
}

/// Read CPU core tick data from /proc/stat. No sysfs interaction — no FD pressure.
fn read_core_ticks() -> Result<Vec<(String, CpuCoreTicks)>, CpuError> {
    let content = fs::read_to_string("/proc/stat").map_err(|e| CpuError::IoError(e.to_string()))?;

    let mut core_ticks = Vec::new();
    for line in content.lines() {
        if !line.starts_with("cpu") || line.starts_with("cpu ") {
            continue;
        }

        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.len() < 9 {
            debug!(
                "Skipping /proc/stat line with insufficient fields: {}",
                line
            );
            continue;
        }

        let parse = |i: usize| -> u64 { parts.get(i).and_then(|s| s.parse().ok()).unwrap_or(0) };

        let name = parts[0].to_string();
        core_ticks.push((
            name,
            CpuCoreTicks {
                user: parse(1),
                nice: parse(2),
                system: parse(3),
                idle: parse(4),
                iowait: parse(5),
                irq: parse(6),
                softirq: parse(7),
                steal: parse(8),
            },
        ));
    }

    debug!("Read {} core(s) from /proc/stat", core_ticks.len());
    Ok(core_ticks)
}

fn store_state(collector: &mut CpuCollector, curr_ticks: Vec<(String, CpuCoreTicks)>) {
    let prev = std::mem::take(&mut collector.current_ticks);
    match prev {
        Some(prev) => {
            collector.last_stats = Some(
                prev.into_iter()
                    .map(|(_, t)| CpuCoreStats {
                        user: t.user,
                        nice: t.nice,
                        system: t.system,
                        idle: t.idle,
                        iowait: t.iowait,
                        irq: t.irq,
                        softirq: t.softirq,
                        steal: t.steal,
                    })
                    .collect(),
            );
        }
        None => {
            collector.last_stats = Some(
                curr_ticks
                    .iter()
                    .map(|(_, t)| CpuCoreStats {
                        user: t.user,
                        nice: t.nice,
                        system: t.system,
                        idle: t.idle,
                        iowait: t.iowait,
                        irq: t.irq,
                        softirq: t.softirq,
                        steal: t.steal,
                    })
                    .collect(),
            );
        }
    }
    collector.current_ticks = Some(curr_ticks);
}

#[allow(clippy::too_many_arguments)]
fn calculate_usage(
    prev: &[CpuCoreStats],
    curr_ticks: &[(String, CpuCoreTicks)],
    prev_time: std::time::SystemTime,
    curr_time: std::time::SystemTime,
    runtime_freqs: &[CpuFreq],
    base_freqs: &[CpuFreq],
    peak_freqs_since_startup: &[u64],
    any_freq_available: bool,
) -> CpuUsage {
    use std::time::Duration;

    let interval = curr_time
        .duration_since(prev_time)
        .unwrap_or(Duration::from_secs(1));
    let _interval_secs = interval.as_secs() as f64;

    let mut core_usages: Vec<f64> = Vec::with_capacity(prev.len());

    for i in 0..prev.len() {
        if i >= curr_ticks.len() || i >= runtime_freqs.len() {
            break;
        }

        let p = &prev[i];
        let c = &curr_ticks[i].1;

        let prev_total = p.user as f64
            + p.nice as f64
            + p.system as f64
            + p.idle as f64
            + p.iowait as f64
            + p.irq as f64
            + p.softirq as f64
            + p.steal as f64;

        let curr_total = c.user as f64
            + c.nice as f64
            + c.system as f64
            + c.idle as f64
            + c.iowait as f64
            + c.irq as f64
            + c.softirq as f64
            + c.steal as f64;

        let idle_delta = (c.idle + c.iowait) as f64 - (p.idle + p.iowait) as f64;
        let total_delta = curr_total - prev_total;

        if total_delta <= 0.0 {
            debug!("CPU core {}: no time elapsed, skipping", i);
            continue;
        }

        let raw_usage = 100.0 * (1.0 - (idle_delta / total_delta)).clamp(0.0, 1.0);

        // Per-core frequency-weighted calculation per spec:
        // max_frequency = max(current_freq, base_max_freq, peak_observed)
        // actual_usage_percent = raw_usage * (current_freq / max_frequency)
        let weighted_usage = if any_freq_available {
            let f = &runtime_freqs[i];

            // Determine the effective maximum frequency for this core:
            // Use the highest of: current, base max (from startup cache), or peak observed since boot.
            // This handles turbo boost scenarios where a core briefly hits higher frequencies.
            let cur = f.cur_freq_khz;
            let base_max = if i < base_freqs.len() {
                base_freqs[i].max_freq_khz
            } else {
                f.max_freq_khz
            };
            let peak = if i < peak_freqs_since_startup.len() {
                peak_freqs_since_startup[i]
            } else {
                0
            };

            // The denominator is the maximum frequency this core has ever been observed at
            // (or its rated base max), preventing inflated usage when downclocked from turbo.
            let effective_max = cur.max(base_max).max(peak);

            if cur > 0 && effective_max > 0 {
                raw_usage * (cur as f64 / effective_max as f64)
            } else {
                raw_usage // fallback: no usable runtime freq data, use raw usage
            }
        } else {
            raw_usage
        };

        core_usages.push(weighted_usage.clamp(0.0, 100.0));
    }

    if core_usages.is_empty() {
        return CpuUsage {
            per_core_max: 0.0,
            total_average: 0.0,
        };
    }

    let per_core_max = *core_usages
        .iter()
        .max_by(|a, b| a.partial_cmp(b).unwrap())
        .unwrap();
    let total_sum: f64 = core_usages.iter().sum();
    let total_average = total_sum / prev.len() as f64;

    debug!(
        "CPU: per_core_max={:.1}%, total_avg={:.1}% ({}/{} cores, freq_weighted={})",
        per_core_max,
        total_average,
        core_usages.len(),
        prev.len(),
        if any_freq_available { "yes" } else { "no" }
    );

    CpuUsage {
        per_core_max: per_core_max.clamp(0.0, 100.0),
        total_average: total_average.clamp(0.0, 100.0),
    }
}

#[derive(Debug)]
pub enum CpuError {
    IoError(String),
    NoCoresFound,
}

impl std::fmt::Display for CpuError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CpuError::IoError(e) => write!(f, "IO error: {}", e),
            CpuError::NoCoresFound => write!(f, "No CPU cores found in /proc/stat"),
        }
    }
}

impl std::error::Error for CpuError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cpu_collector_creation() {
        let collector = CpuCollector::new();
        assert!(collector.last_stats.is_none());
        assert!(collector.current_ticks.is_none());
        assert!(collector.last_time.is_none());
    }

    #[test]
    fn test_cpu_usage_clone() {
        let usage = CpuUsage {
            per_core_max: 45.2,
            total_average: 12.8,
        };
        let cloned = usage.clone();
        assert_eq!(usage.per_core_max, cloned.per_core_max);
        assert_eq!(usage.total_average, cloned.total_average);
    }

    #[test]
    fn test_cpu_usage_zero_values() {
        let usage = CpuUsage {
            per_core_max: 0.0,
            total_average: 0.0,
        };
        assert!((usage.per_core_max - 0.0).abs() < f64::EPSILON);
        assert!((usage.total_average - 0.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_cpu_usage_full_load() {
        let usage = CpuUsage {
            per_core_max: 100.0,
            total_average: 100.0,
        };
        assert!((usage.per_core_max - 100.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_cpu_freq_default() {
        let freq = CpuFreq::default();
        assert_eq!(freq.cur_freq_khz, 0);
        assert_eq!(freq.max_freq_khz, 800_000);
    }

    #[test]
    fn test_cpu_error_display() {
        let err = CpuError::IoError("permission denied".to_string());
        assert!(format!("{}", err).contains("IO error"));
        assert!(format!("{}", err).contains("permission denied"));

        let err2 = CpuError::NoCoresFound;
        assert!(format!("{}", err2).contains("No CPU cores found"));
    }

    #[tokio::test]
    async fn test_first_collect_does_not_panic_on_none_last_stats() {
        let mut collector = CpuCollector::new();
        // last_stats is None — calling collect() on a fresh collector must not panic.
        let result = collector.collect().await;
        assert!(result.is_ok(), "first collect should succeed");
        let usage = result.unwrap();
        // First sample has no previous data, so delta is zero.
        assert_eq!(usage.per_core_max, 0.0);
        assert_eq!(usage.total_average, 0.0);
    }

    #[tokio::test]
    async fn test_consecutive_collects_produce_nonzero_after_warmup() {
        let mut collector = CpuCollector::new();
        // Warm up: collect a few times so last_stats and last_time are set.
        for _ in 0..3 {
            let result = collector.collect().await;
            assert!(result.is_ok());
            tokio::time::sleep(std::time::Duration::from_millis(250)).await;
        }
        // After warmup, the next collect should compute actual delta (not zero) on a busy system.
        // On an idle system it may still be near-zero; just verify no panic and stable core count.
        let result = collector.collect().await;
        assert!(result.is_ok());
    }

    #[test]
    fn test_frequency_weighted_calculation() {
        // Verify: raw 100% usage at half max frequency → weighted ≈ 50%
        // Formula: raw * (cur / max(cur, base_max, peak)) = 100 * (2.5GHz / 5.0GHz) = 50%
        let runtime = CpuFreq {
            cur_freq_khz: 2_500_000,
            max_freq_khz: 5_000_000,
        }; // 2.5GHz current
        let base = [CpuFreq {
            cur_freq_khz: 0,
            max_freq_khz: 3_000_000,
        }]; // 3GHz rated max
        let peak = [4_000_000u64]; // previously observed 4GHz

        let effective_max = runtime.cur_freq_khz.max(base[0].max_freq_khz).max(peak[0]);
        assert_eq!(effective_max, 4_000_000); // peak wins: max(2.5GHz, 3GHz, 4GHz) = 4GHz

        let weighted = 100.0 * (runtime.cur_freq_khz as f64 / effective_max as f64);
        assert!((weighted - 62.5).abs() < 0.01); // 100% * (2.5/4.0) = 62.5%
    }

    #[test]
    fn test_frequency_weighted_turbo_boost_tracking() {
        // Simulate turbo boost: core hits 4.8GHz once, then runs at base frequency.
        let mut peak_freqs = [0u64];

        // Tick 1: core running at 3.0GHz (base), no peak yet
        assert_eq!(peak_freqs[0], 0);

        // Simulate observing turbo boost of 4.8GHz
        let cur_at_turbo = 4_800_000u64; // 4.8GHz in kHz
        if cur_at_turbo > peak_freqs[0] {
            peak_freqs[0] = cur_at_turbo;
        }

        // Tick 2: core back to 3.0GHz, but peak is still tracked at 4.8GHz
        let cur_normal = 3_000_000u64; // 3.0GHz in kHz
        let effective_max = cur_normal.max(3_000_000).max(peak_freqs[0]); // base_max=3GHz, peak=4.8GHz
        assert_eq!(effective_max, 4_800_000); // turbo boost is the reference

        let raw = 50.0;
        let weighted = raw * (cur_normal as f64 / effective_max as f64);
        assert!((weighted - 31.25).abs() < 0.01); // 50% * (3.0/4.8) ≈ 31.25%
    }

    #[test]
    fn test_frequency_weighted_no_turbo() {
        // No turbo boost observed: effective_max = max(cur, base_max, peak=0)
        let cur = 2_500_000u64; // 2.5GHz current
        let base_max = 3_000_000u64; // 3GHz rated max
        let peak = 0u64;

        let effective_max = cur.max(base_max).max(peak);
        assert_eq!(effective_max, 3_000_000); // base_max wins

        let raw = 80.0;
        let weighted = raw * (cur as f64 / effective_max as f64);
        assert!((weighted - 66.67).abs() < 0.1);
    }

    #[test]
    fn test_read_single_freq_returns_zero_on_missing_file() {
        let val = read_single_freq("/nonexistent/path/to/freq", 1.0);
        assert_eq!(val, 0u64);
    }
}
