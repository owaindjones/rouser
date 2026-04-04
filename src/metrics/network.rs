use std::collections::HashMap;
use std::fs;
use std::time::{Duration, SystemTime};
use tracing::{debug, warn};

#[derive(Debug, Clone)]
pub struct NetworkStats {
    pub rx_bytes: u64,
    pub tx_bytes: u64,
}

pub struct NetworkCollector {
    last_stats: HashMap<String, NetworkStats>,
    last_time: Option<SystemTime>,
    exclude_interfaces: Vec<String>,
}

impl NetworkCollector {
    pub fn new(exclude_interfaces: Vec<String>) -> Self {
        // Ensure loopback is excluded by default unless explicitly in exclusion list
        let mut excludes = exclude_interfaces;
        if !excludes.contains(&"lo".to_string()) {
            excludes.push("lo".to_string());
        }
        Self {
            last_stats: HashMap::new(),
            last_time: None,
            exclude_interfaces: excludes,
        }
    }

    pub async fn collect(&mut self) -> Result<f64, NetworkError> {
        let current_stats = self.read_interface_stats()?;
        let now = SystemTime::now();

        match &self.last_time {
            Some(prev_time) => {
                let interval = now.duration_since(*prev_time)
                    .unwrap_or(Duration::from_secs(1));

                let mut total_delta = 0u64;

                for (name, stats) in &current_stats {
                    if let Some(prev) = self.last_stats.get(name) {
                        let rx_delta = stats.rx_bytes.saturating_sub(prev.rx_bytes);
                        let tx_delta = stats.tx_bytes.saturating_sub(prev.tx_bytes);
                        total_delta += rx_delta + tx_delta;
                    }
                }

                // Convert bytes to megabits per second
                // (bytes * 8 bits) / (seconds * 1,000,000)
                let throughput_mbps = (total_delta as f64 * 8.0)
                    / (interval.as_secs_f64() * 1_000_000.0);

                self.last_stats = current_stats;
                self.last_time = Some(now);

                debug!("Network usage: {:.2} Mbps", throughput_mbps);
                Ok(throughput_mbps)
            }
            None => {
                self.last_stats = current_stats;
                self.last_time = Some(now);
                debug!("Network: first sample, returning 0.0 Mbps");
                Ok(0.0)
            }
        }
    }

    fn read_interface_stats(&self) -> Result<HashMap<String, NetworkStats>, NetworkError> {
        let content = fs::read_to_string("/proc/net/dev")
            .map_err(|e| NetworkError::IoError(e.to_string()))?;

        let mut stats_map = HashMap::new();

        for line in content.lines() {
            // Skip header lines
            if !line.contains(':') {
                continue;
            }

            let parts: Vec<&str> = line.split(':').collect();
            if parts.len() != 2 {
                continue;
            }

            let name = parts[0].trim().to_string();

            // Skip excluded interfaces
            if self.exclude_interfaces.contains(&name) {
                debug!("Skipping excluded interface: {}", name);
                continue;
            }

            let values: Vec<u64> = parts[1]
                .split_whitespace()
                .map(|s| s.parse().unwrap_or(0))
                .collect();

           if values.len() < 16 {
            debug!("Invalid /proc/net/dev line: {} (expected 16 values, got {})", name, values.len());
            continue;
        }

            stats_map.insert(name, NetworkStats {
                rx_bytes: values[0],
                tx_bytes: values[8],
            });
        }

        if stats_map.is_empty() {
            warn!("No network interfaces found");
        }

        Ok(stats_map)
    }
}

#[derive(Debug)]
pub enum NetworkError {
    IoError(String),
    #[allow(dead_code)] // Reserved for future use
    InvalidFormat,
}

impl std::fmt::Display for NetworkError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            NetworkError::IoError(e) => write!(f, "IO error: {}", e),
            NetworkError::InvalidFormat => write!(f, "Invalid /proc/net/dev format"),
        }
    }
}

impl std::error::Error for NetworkError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_network_collector_creation() {
        let _collector = NetworkCollector::new(vec!["lo".to_string()]);
        assert!(true);
    }

    #[test]
    fn test_network_error_display() {
        let err = NetworkError::IoError("test error".to_string());
        let display = format!("{}", err);
        assert!(display.contains("test error"));
    }
}
