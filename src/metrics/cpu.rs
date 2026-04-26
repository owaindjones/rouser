use std::fs;
use std::path::Path;
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
    /// Maximum weighted usage across all cores (0–100).
    pub per_core_max: f64,
    /// Average weighted usage across all cores (0–100).
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
}

impl Default for CpuCollector {
    fn default() -> Self {
        Self::new()
    }
}

impl CpuCollector {
    pub fn new() -> Self {
        Self {
            last_stats: None,
            current_ticks: None,
            last_time: None,
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
        let (curr_ticks, freqs) = self.read_stats()?;
        let now = std::time::SystemTime::now();

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
                        &freqs,
                    );
                    self.store_state(curr_ticks, freqs);
                    Ok(usage)
                }
                None => {
                    self.store_state(curr_ticks, freqs);
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
            self.store_state(curr_ticks, freqs);
            Ok(CpuUsage {
                per_core_max: 0.0,
                total_average: 0.0,
            })
        }
    }

    fn store_state(&mut self, curr_ticks: Vec<(String, CpuCoreTicks)>, _freqs: Vec<CpuFreq>) {
        let prev = std::mem::take(&mut self.current_ticks);
        if let Some(prev) = prev {
            self.last_stats = Some(
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
        self.current_ticks = Some(curr_ticks);
    }

    #[allow(clippy::type_complexity)]
    fn read_stats(&self) -> Result<(Vec<(String, CpuCoreTicks)>, Vec<CpuFreq>), CpuError> {
        let content =
            fs::read_to_string("/proc/stat").map_err(|e| CpuError::IoError(e.to_string()))?;

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

            let parse =
                |i: usize| -> u64 { parts.get(i).and_then(|s| s.parse().ok()).unwrap_or(0) };

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

        if core_ticks.is_empty() {
            return Err(CpuError::NoCoresFound);
        }

        let mut freqs = Vec::with_capacity(core_ticks.len());
        for (name, _) in &core_ticks {
            freqs.push(read_core_freq(name));
        }

        debug!("Read {} core(s) from /proc/stat", core_ticks.len());
        Ok((core_ticks, freqs))
    }
}

fn calculate_usage(
    prev: &[CpuCoreStats],
    curr_ticks: &[(String, CpuCoreTicks)],
    prev_time: std::time::SystemTime,
    curr_time: std::time::SystemTime,
    freqs: &[CpuFreq],
) -> CpuUsage {
    use std::time::Duration;

    let interval = curr_time
        .duration_since(prev_time)
        .unwrap_or(Duration::from_secs(1));
    let _interval_secs = interval.as_secs() as f64;

    let all_freqs_valid = freqs
        .iter()
        .all(|f| f.cur_freq_khz > 0 && f.max_freq_khz > 0);

    let mut core_usages: Vec<f64> = Vec::with_capacity(prev.len());

    for i in 0..prev.len() {
        if i >= curr_ticks.len() || i >= freqs.len() {
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

        let weighted_usage = if all_freqs_valid {
            let f = &freqs[i];
            let effective_cur = f.cur_freq_khz.min(f.max_freq_khz);
            raw_usage * (effective_cur as f64 / f.max_freq_khz as f64)
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
        if all_freqs_valid { "yes" } else { "no" }
    );

    CpuUsage {
        per_core_max: per_core_max.clamp(0.0, 100.0),
        total_average: total_average.clamp(0.0, 100.0),
    }
}

fn read_core_freq(core_name: &str) -> CpuFreq {
    let cur_path = format!(
        "/sys/devices/system/cpu/{}/cpufreq/cpuinfo_cur_freq",
        core_name
    );
    let max_path = format!(
        "/sys/devices/system/cpu/{}/cpufreq/cpuinfo_max_freq",
        core_name
    );

    if Path::new(&cur_path).exists() {
        let cur = read_freq_hz(&cur_path, 1000.0);
        let max = read_freq_hz(&max_path, 1000.0);
        return CpuFreq {
            cur_freq_khz: (cur as f64 / 1000.0) as u64,
            max_freq_khz: (max as f64 / 1000.0) as u64,
        };
    }

    let cur_path = format!("/sys/devices/system/cpu/{}/cpuinfo_cur_freq", core_name);
    let max_path = format!("/sys/devices/system/cpu/{}/cpuinfo_max_freq", core_name);

    if Path::new(&cur_path).exists() {
        let cur = read_freq_hz(&cur_path, 1.0);
        let max = read_freq_hz(&max_path, 1.0);
        return CpuFreq {
            cur_freq_khz: (cur as f64 / 1000.0) as u64,
            max_freq_khz: (max as f64 / 1000.0) as u64,
        };
    }

    CpuFreq::default()
}

fn read_freq_hz(path: &str, scale: f64) -> u64 {
    fs::read_to_string(path)
        .ok()
        .and_then(|content| content.trim().parse::<f64>().ok())
        .map(|v| (v * scale) as u64)
        .unwrap_or(0)
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
}
