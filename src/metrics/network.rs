use std::collections::HashMap;
use std::fs;
use std::time::{Duration, SystemTime};
use tracing::{debug, warn};

#[derive(Debug, Clone)]
pub struct NetworkStats {
    pub rx_bytes: u64,
    pub tx_bytes: u64,
}

/// Per-interface throughput breakdown returned by collect().
#[derive(Debug, Default)]
pub struct NetworkThroughput {
    /// Total throughput across all monitored interfaces (Mbps).
    pub total_mbps: f64,
    /// Throughput per interface name (Mbps).
    pub per_interface: HashMap<String, f64>,
}

pub struct NetworkCollector {
    last_stats: HashMap<String, NetworkStats>,
    last_time: Option<SystemTime>,
    exclude_interfaces: Vec<String>,
}

impl NetworkCollector {
    pub fn new(exclude_interfaces: Vec<String>) -> Self {
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

    #[allow(dead_code)]
    pub fn name(&self) -> &str {
        "network"
    }

    /// Collect network I/O throughput.
    /// Returns total Mbps plus per-interface breakdown for logging.
    pub async fn collect(&mut self) -> Result<NetworkThroughput, NetworkError> {
        let current_stats = self.read_interface_stats()?;
        let now = SystemTime::now();

        match &self.last_time {
            Some(prev_time) => {
                let interval = now
                    .duration_since(*prev_time)
                    .unwrap_or(Duration::from_secs(1));

                let mut total_delta: u64 = 0;
                let mut per_interface_mbps: HashMap<String, f64> = HashMap::new();

                for (name, stats) in &current_stats {
                    if let Some(prev) = self.last_stats.get(name) {
                        let rx_delta = stats.rx_bytes.saturating_sub(prev.rx_bytes);
                        let tx_delta = stats.tx_bytes.saturating_sub(prev.tx_bytes);
                        total_delta += rx_delta + tx_delta;

                         if interval.as_secs_f64() > 0.0 {
                            let iface_mbps = ((rx_delta + tx_delta) as f64 * 8.0)
                                / (interval.as_secs_f64() * 1_000_000.0);
                            per_interface_mbps.insert(name.clone(), iface_mbps);
                        }
                    }
                }

                let total_mbps = if interval.as_secs_f64() > 0.0 {
                    (total_delta as f64 * 8.0) / (interval.as_secs_f64() * 1_000_000.0)
                } else {
                    0.0
                };

                self.last_stats = current_stats;
                self.last_time = Some(now);

                Ok(NetworkThroughput {
                    total_mbps,
                    per_interface: per_interface_mbps,
                })
            }
            None => {
                self.last_stats = current_stats;
                self.last_time = Some(now);
                debug!("Network: first sample, returning 0.0 Mbps");
                Ok(NetworkThroughput::default())
            }
        }
    }

    fn read_interface_stats(&self) -> Result<HashMap<String, NetworkStats>, NetworkError> {
        let content = fs::read_to_string("/proc/net/dev")
            .map_err(|e| NetworkError::IoError(e.to_string()))?;

        let mut stats_map = HashMap::new();

        for line in content.lines() {
            if !line.contains(':') {
                continue;
            }

            let parts: Vec<&str> = line.split(':').collect();
            if parts.len() != 2 {
                continue;
            }

            let name = parts[0].trim().to_string();

            // Skip excluded interfaces silently.
            if self.exclude_interfaces.contains(&name) {
                continue;
            }

            let values: Vec<u64> = parts[1]
                .split_whitespace()
                .map(|s| s.parse().unwrap_or(0))
                .collect();

            if values.len() < 16 {
                debug!(
                    "Invalid /proc/net/dev line: {} (expected 16 values, got {})",
                    name,
                    values.len()
                );
                continue;
            }

            stats_map.insert(
                name,
                NetworkStats {
                    rx_bytes: values[0],
                    tx_bytes: values[8],
                },
            );
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
        let collector = NetworkCollector::new(vec!["lo".to_string()]);
        assert_eq!(collector.name(), "network");
    }

    #[test]
    fn test_network_error_display() {
        let err = NetworkError::IoError("test error".to_string());
        let display = format!("{}", err);
        assert!(display.contains("test error"));
    }
}
