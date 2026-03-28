use std::collections::HashMap;
use std::fs;
use tracing::debug;




#[derive(Debug, Clone)]
pub struct DiskStats {
    pub name: String,
    pub sectors_read: u64,
    pub sectors_written: u64,
}

pub struct DiskCollector {
    exclude_prefixes: Vec<String>,
    last_stats: HashMap<String, DiskStats>,
    last_collection_time: Option<std::time::SystemTime>,
}

impl DiskCollector {
    // Default exclusions: loop devices, fd, sr, cdrom
    // Note: dm- (LVM) devices are INCLUDED
    pub fn new(exclude_prefixes: Vec<String>) -> Self {
        Self {
            exclude_prefixes,
            last_stats: HashMap::new(),
            last_collection_time: None,
        }
    }

pub async fn collect(&mut self) -> Result<f64, DiskError> {
        let current_time = std::time::SystemTime::now();
        let current_stats = self.read_disk_stats()?;

        if self.last_stats.is_empty() || self.last_collection_time.is_none() {
            self.last_stats = current_stats;
            self.last_collection_time = Some(current_time);
            debug!("Disk: first sample, returning 0.0 MB/s");
            return Ok(0.0);
        }

        let interval_seconds = current_time
            .duration_since(self.last_collection_time.unwrap())
            .unwrap_or(std::time::Duration::from_secs(5))
            .as_secs_f64();

        let mut total_sectors = 0u64;

        for (key, stats) in &current_stats {
            if let Some(prev) = self.last_stats.get(key) {
                let read_delta = stats.sectors_read.saturating_sub(prev.sectors_read);
                let write_delta = stats.sectors_written.saturating_sub(prev.sectors_written);
                total_sectors = total_sectors.saturating_add(read_delta);
                total_sectors = total_sectors.saturating_add(write_delta);
            }
        }

        // Convert sectors to bytes (assuming 512-byte sectors)
        const SECTOR_SIZE: u64 = 512;
        let total_bytes = total_sectors as f64 * SECTOR_SIZE as f64;

        // Convert to MB/s
        let throughput_mb_s = total_bytes / (interval_seconds * 1_000_000.0);

        self.last_stats = current_stats;
        self.last_collection_time = Some(current_time);

        debug!("Disk usage: {:.2} MB/s (interval: {:.2}s)", throughput_mb_s, interval_seconds);
        Ok(throughput_mb_s)
    }

    fn read_disk_stats(&self) -> Result<HashMap<String, DiskStats>, DiskError> {
        let content = fs::read_to_string("/proc/diskstats")
            .map_err(|e| DiskError::IoError(e.to_string()))?;

        let mut stats_map = HashMap::new();

        for line in content.lines() {
            let parts: Vec<&str> = line.split_whitespace().collect();

            if parts.len() < 14 {
                continue;
            }

            let name = parts[2];

            // Check exclusions
            if !self.should_monitor(name) {
                debug!("Excluding disk: {}", name);
                continue;
            }

            let sectors_read = parts[6].parse().unwrap_or(0);
            let sectors_written = parts[10].parse().unwrap_or(0);

            stats_map.insert(name.to_string(), DiskStats {
                name: name.to_string(),
                sectors_read,
                sectors_written,
            });
        }

        if stats_map.is_empty() {
            debug!("No disk devices found after filtering");
        }

        Ok(stats_map)
    }

    fn should_monitor(&self, name: &str) -> bool {
        !self.exclude_prefixes.iter().any(|prefix| name.starts_with(prefix))
    }
}

impl Default for DiskCollector {
    fn default() -> Self {
        Self::new(vec![
            "loop".to_string(),
            "fd".to_string(),
            "sr".to_string(),
            "cdrom".to_string(),
        ])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn test_disk_error_display() {
        let err = DiskError::IoError("test error".to_string());
        let display = format!("{}", err);
        assert!(display.contains("test error"));
    }
}

#[derive(Debug)]
pub enum DiskError {
    IoError(String),
    InvalidFormat,
}

impl std::fmt::Display for DiskError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DiskError::IoError(e) => write!(f, "IO error: {}", e),
            DiskError::InvalidFormat => write!(f, "Invalid /proc/diskstats format"),
        }
    }
}

impl std::error::Error for DiskError {}

