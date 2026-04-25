use std::fs;
use tracing::debug;

#[derive(Debug, Clone)]
pub struct CpuStats {
    pub user: u64,
    pub nice: u64,
    pub system: u64,
    pub idle: u64,
    pub iowait: u64,
    pub irq: u64,
    pub softirq: u64,
    pub steal: u64,
    pub guest: u64,
    #[allow(dead_code)]
    guest_nice: u64,
}

pub struct CpuCollector {
    last_stats: Option<CpuStats>,
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
            last_time: None,
        }
    }

    #[allow(dead_code)]
    pub fn name(&self) -> &str {
        "cpu"
    }

    pub async fn collect(&mut self) -> Result<f64, CpuError> {
        let stats = self.read_stats()?;
        let now = std::time::SystemTime::now();

        match (&self.last_stats, &self.last_time) {
            (Some(prev_stats), Some(prev_time)) => {
                let usage = self.calculate_usage(prev_stats, &stats, *prev_time, now);
                self.last_stats = Some(stats);
                self.last_time = Some(now);
                Ok(usage)
            }
            _ => {
                self.last_stats = Some(stats);
                self.last_time = Some(now);
                Ok(0.0)
            }
        }
    }

    fn read_stats(&self) -> Result<CpuStats, CpuError> {
        let content =
            fs::read_to_string("/proc/stat").map_err(|e| CpuError::IoError(e.to_string()))?;

        let first_line = content
            .lines()
            .find(|l| l.starts_with("cpu "))
            .ok_or(CpuError::InvalidFormat)?;

        let fields: Vec<u64> = first_line
            .split_whitespace()
            .skip(1)
            .map(|s| s.parse().unwrap_or(0))
            .collect();

        if fields.len() < 9 {
            return Err(CpuError::MissingFields);
        }

        Ok(CpuStats {
            user: fields[0],
            nice: fields.get(1).copied().unwrap_or(0),
            system: fields.get(2).copied().unwrap_or(0),
            idle: fields.get(3).copied().unwrap_or(0),
            iowait: fields.get(4).copied().unwrap_or(0),
            irq: fields.get(5).copied().unwrap_or(0),
            softirq: fields.get(6).copied().unwrap_or(0),
            steal: fields.get(7).copied().unwrap_or(0),
            guest: fields.get(8).copied().unwrap_or(0),
            guest_nice: fields.get(9).copied().unwrap_or(0),
        })
    }

    fn calculate_usage(
        &self,
        prev: &CpuStats,
        curr: &CpuStats,
        prev_time: std::time::SystemTime,
        curr_time: std::time::SystemTime,
    ) -> f64 {
        use std::time::Duration;

        let interval = curr_time
            .duration_since(prev_time)
            .unwrap_or(Duration::from_secs(1));
        let interval_secs = interval.as_secs() as f64;

        let prev_total = prev.user as f64
            + prev.nice as f64
            + prev.system as f64
            + prev.idle as f64
            + prev.iowait as f64
            + prev.irq as f64
            + prev.softirq as f64
            + prev.steal as f64
            + prev.guest as f64;

        let curr_total = curr.user as f64
            + curr.nice as f64
            + curr.system as f64
            + curr.idle as f64
            + curr.iowait as f64
            + curr.irq as f64
            + curr.softirq as f64
            + curr.steal as f64
            + curr.guest as f64;

        let prev_idle = prev.idle as f64 + prev.iowait as f64;
        let curr_idle = curr.idle as f64 + curr.iowait as f64;

        let idle_delta = curr_idle - prev_idle;
        let total_delta = curr_total - prev_total;

        if total_delta <= 0.0 {
            debug!("CPU: No time elapsed in interval, returning 0%");
            return 0.0;
        }

        let usage = 100.0 * (1.0 - (idle_delta / total_delta));
        let usage = usage.clamp(0.0, 100.0);

        debug!("CPU usage: {:.1}% (interval: {}s)", usage, interval_secs);

        usage
    }
}

#[derive(Debug)]
pub enum CpuError {
    IoError(String),
    InvalidFormat,
    MissingFields,
}

impl std::fmt::Display for CpuError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CpuError::IoError(e) => write!(f, "IO error: {}", e),
            CpuError::InvalidFormat => write!(f, "Invalid /proc/stat format"),
            CpuError::MissingFields => write!(f, "Missing required CPU fields"),
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
        assert!(collector.last_time.is_none());
    }

    #[test]
    fn test_cpu_stats_display() {
        let stats = CpuStats {
            user: 100,
            nice: 10,
            system: 50,
            idle: 840,
            iowait: 5,
            irq: 1,
            softirq: 4,
            steal: 0,
            guest: 0,
            guest_nice: 0,
        };
        let display = format!("{:?}", stats);
        assert!(display.contains("CpuStats"));
    }
}
