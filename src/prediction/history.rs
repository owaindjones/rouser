//! Binary history log for predictive cooldown.
//!
//! Uses bincode v2 (serde-compatible binary serialization) with date-partitioned files.
//! Each file is named `history.log.YYYYMMDD` and stored under XDG-compliant paths:
//! - User data dir: `$XDG_DATA_HOME/rouser/history.log.*` or `~/.local/share/rouser/history.log.*`
//! - Root path: `/var/lib/rouser/history.log.*`

use chrono::{DateTime, Local, Utc};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs::{self, File};
use std::io::{BufReader, BufWriter, Read, Write};
use std::path::{Path, PathBuf};
use tracing::{debug, info, warn};

/// A single data point recorded at each tick.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HistoryEntry {
    /// Unix epoch nanoseconds since 1970-01-01T00:00:00 UTC.
    pub timestamp_ns: u64,
    /// CPU usage metrics (per_core_max, total_average).
    pub cpu_usage: CpuSnapshot,
    /// GPU smoothed usages in order of device enumeration.
    #[serde(default)]
    pub gpu_usages: Vec<f64>,
    /// Network throughput (Mbps), aggregated across all interfaces.
    pub network_mbps: f64,
    /// Disk throughput (MB/s), aggregated across all devices.
    pub disk_mb_s: f64,
    /// Whether rouser currently holds the inhibition lock at this timestamp.
    pub inhibited: bool,
}

/// CPU metrics snapshot — serializable subset of CpuUsage.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CpuSnapshot {
    pub per_core_max: f64,
    pub total_average: f64,
}

impl HistoryEntry {
    /// Create a new history entry from tick metrics and current inhibition state.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        timestamp_ns: u64,
        cpu_per_core_max: f64,
        cpu_total_average: f64,
        gpu_usages: Vec<f64>,
        network_mbps: f64,
        disk_mb_s: f64,
        inhibited: bool,
    ) -> Self {
        Self {
            timestamp_ns,
            cpu_usage: CpuSnapshot {
                per_core_max: cpu_per_core_max,
                total_average: cpu_total_average,
            },
            gpu_usages,
            network_mbps,
            disk_mb_s,
            inhibited,
        }
    }

    /// Extract the date component for file partitioning (UTC day).
    pub fn entry_date(&self) -> chrono::NaiveDate {
        let secs = self.timestamp_ns / 1_000_000_000;
        match DateTime::<Utc>::from_timestamp(secs as i64, 0) {
            Some(dt) => dt.naive_utc().date(),
            None => Local::now().date_naive(),
        }
    }

    /// Serialize this entry to a binary buffer using bincode v2 standard config.
    pub fn to_bytes(&self) -> Vec<u8> {
        let encoded = bincode::serde::encode_to_vec(self, bincode::config::standard())
            .expect("HistoryEntry should serialize");
        // Prepend 4-byte length prefix for seekable streaming.
        let len = (encoded.len() as u32).to_le_bytes();
        let mut result = Vec::with_capacity(4 + encoded.len());
        result.extend_from_slice(&len);
        result.extend_from_slice(&encoded);
        result
    }

    /// Deserialize a single entry from bytes starting at offset 0.
    /// Returns `(entry, consumed_bytes)` or `None` if the buffer is too short/corrupt.
    pub fn from_bytes(buf: &[u8]) -> Option<(Self, usize)> {
        if buf.len() < 4 {
            return None;
        }
        let len = u32::from_le_bytes([buf[0], buf[1], buf[2], buf[3]]) as usize;
        if buf.len() < 4 + len {
            return None;
        }
        match bincode::serde::decode_from_slice::<Self, _>(
            &buf[4..4 + len],
            bincode::config::standard(),
        ) {
            Ok((entry, consumed)) => Some((entry, 4 + consumed)),
            Err(_) => None, // Corrupted entry.
        }
    }
}

/// XDG-compliant data directory path.
fn xdg_data_dir() -> PathBuf {
    std::env::var("XDG_DATA_HOME")
        .ok()
        .filter(|s| !s.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            std::env::var("HOME")
                .ok()
                .map(|h| PathBuf::from(h).join(".local/share"))
                .expect("XDG_DATA_HOME or HOME must be set for user data directory")
        })
}

/// Get the base history directory.
fn history_base_dir(is_root: bool) -> PathBuf {
    let path = if is_root {
        Path::new("/var/lib/rouser")
    } else {
        &xdg_data_dir().join("rouser")
    };

    // Ensure the parent directory exists for root paths.
    if is_root {
        let _ = fs::create_dir_all(path.parent().unwrap_or(path));
    }

    path.to_path_buf()
}

/// Ensure the history directory exists.
fn ensure_history_dir(path: &Path) -> std::io::Result<()> {
    fs::create_dir_all(path)
}

const HISTORY_FILE_PREFIX: &str = "history.log.";

/// A date-partitioned binary log file for storing metric snapshots.
pub struct HistoryLog {
    base_path: PathBuf,
    entries_today: Vec<HistoryEntry>,
    last_prune_date: Option<i64>, // Unix day number (seconds since epoch / 86400)
}

impl HistoryLog {
    /// Create a new history log writer.
    pub fn new(is_root: bool) -> Self {
        let base_path = history_base_dir(is_root);
        if let Err(e) = ensure_history_dir(&base_path) {
            warn!(
                "Failed to create history directory {}: {}",
                base_path.display(),
                e
            );
        }

        HistoryLog {
            base_path,
            entries_today: Vec::new(),
            last_prune_date: None,
        }
    }

    /// Append an entry to the log. Buffers in memory until flush or date change.
    pub fn append(&mut self, entry: HistoryEntry) {
        let entry_date = entry.entry_date();

        if self.entries_today.is_empty() {
            self.entries_today.push(entry);
        } else {
            // Check if this entry is for the same day as our buffer.
            let first_date = self.entries_today.first().map(|e| e.entry_date());
            match first_date {
                Some(d) if d == entry_date => {
                    self.entries_today.push(entry);
                }
                _ => {
                    // Different date — flush previous day and start new buffer.
                    self.flush();
                    self.entries_today = vec![entry];
                }
            }
        }
    }

    /// Flush in-memory entries to disk.
    pub fn flush(&mut self) {
        if self.entries_today.is_empty() {
            return;
        }

        let date = self.entries_today[0].entry_date();
        let file_path =
            self.base_path
                .join(format!("{}{}", HISTORY_FILE_PREFIX, date.format("%Y%m%d")));

        match File::options().create(true).append(true).open(&file_path) {
            Ok(file) => {
                let mut writer = BufWriter::new(file);
                for entry in &self.entries_today {
                    let bytes = entry.to_bytes();
                    if let Err(e) = writer.write_all(&bytes) {
                        warn!("Failed to write history entry: {}", e);
                    }
                }
                if let Err(e) = writer.flush() {
                    warn!("Failed to flush history buffer: {}", e);
                }
            }
            Err(e) => {
                warn!("Failed to open history log {}: {}", file_path.display(), e);
            }
        }

        debug!(
            "Flushed {} entries for date {} to {}",
            self.entries_today.len(),
            date,
            file_path.display()
        );

        self.entries_today.clear();
    }

    /// Read all entries from the history files, sorted by timestamp.
    pub fn read_all(&self) -> Vec<HistoryEntry> {
        if !self.base_path.exists() {
            return vec![];
        }

        let mut date_entries: BTreeMap<String, Vec<HistoryEntry>> = BTreeMap::new();

        let dir = match fs::read_dir(&self.base_path) {
            Ok(d) => d,
            Err(_) => return vec![], // Directory doesn't exist or can't be read.
        };

        for entry_result in dir {
            let path = match entry_result {
                Ok(e) => e.path(),
                Err(_) => continue,
            };

            if !path.is_file() || !is_history_file(&path) {
                continue;
            }

            let entries = read_entries_from_file(&path);
            // Use filename as sort key for BTreeMap (YYYYMMDD sorts lexicographically).
            if let Some(date_str) = extract_date_str(&path) {
                date_entries.entry(date_str).or_default().extend(entries);
            } else {
                // Skip files we can't parse the date from.
                warn!("Skipping unparseable history file: {}", path.display());
            }
        }

        // Flatten entries and sort by timestamp (BTreeMap iterates in key/date order).
        let mut result: Vec<HistoryEntry> = date_entries.into_values().flatten().collect();

        result.sort_by_key(|e| e.timestamp_ns);
        debug!(
            "Loaded {} history entries from {}",
            result.len(),
            self.base_path.display()
        );

        result
    }

    /// Prune old files beyond the given retention period. Called periodically (e.g., every 12 hours).
    #[allow(dead_code)]
    pub fn prune(&mut self, max_age: std::time::Duration) {
        let base_path = &self.base_path;

        if !base_path.exists() || !base_path.is_dir() {
            return;
        }

        // Compute today's YYYYMMDD string and an approximate cutoff date.
        let today_naive = Local::now().date_naive();
        let days_to_subtract: i32 = (max_age.as_secs() / 86400) as i32;

        // Convert NaiveDate to a comparable YYYYMMDD integer (lexical sort == chronological for this format).
        fn date_as_ymd_int(date: chrono::NaiveDate) -> Option<i32> {
            let ymd_str = date.format("%Y%m%d").to_string();
            ymd_str.parse::<i32>().ok()
        }

        // Convert YYYYMMDD string to NaiveDate for precise age comparison.
        fn parse_ymd(s: &str) -> Option<chrono::NaiveDate> {
            let year = s[0..4].parse().ok()?;
            let month = s[4..6].parse().ok()?;
            let day = s[6..8].parse().ok()?;
            chrono::NaiveDate::from_ymd_opt(year, month, day)
        }

        // Compute cutoff date using NaiveDate arithmetic.
        let cutoff_date = today_naive - chrono::TimeDelta::days(i64::from(days_to_subtract));

        if let Some(today_ymd) = date_as_ymd_int(today_naive) {
            // Only prune once per day (use the YYYYMMDD as a dedup key).
            if self.last_prune_date == Some(today_ymd as i64) {
                return;
            }

            let mut pruned_count: u32 = 0;

            let dir = match fs::read_dir(base_path) {
                Ok(d) => d,
                Err(_) => return, // Can't read directory — skip pruning.
            };

            for entry_result in dir {
                let path = match entry_result {
                    Ok(e) => e.path(),
                    Err(_) => continue,
                };

                if !path.is_file() || !is_history_file(&path) {
                    continue;
                }

                // Extract YYYYMMDD from filename.
                let file_name = path.file_name().and_then(|s| s.to_str()).unwrap_or("");
                let date_part = file_name.strip_prefix(HISTORY_FILE_PREFIX).unwrap_or("");

                if date_part.len() == 8 && date_part.chars().all(|c| c.is_ascii_digit()) {
                    if let Some(file_date) = parse_ymd(date_part) {
                        if file_date < cutoff_date {
                            match fs::remove_file(&path) {
                                Ok(_) => {
                                    pruned_count += 1;
                                    debug!(
                                        "Pruned old history file {} (date: {})",
                                        path.display(),
                                        date_part
                                    );
                                }
                                Err(e) => {
                                    warn!(
                                        "Failed to prune old history file {}: {}",
                                        path.display(),
                                        e
                                    );
                                }
                            }
                        }
                    }
                }
            }

            self.last_prune_date = Some(today_ymd as i64);

            if pruned_count > 0 {
                info!(
                    "Pruned {} old history files (retention: {:?})",
                    pruned_count, max_age
                );
            }
        } // Can't compute today's date — skip pruning.
    }

    /// Check if the log has any data.
    #[allow(dead_code)]
    pub fn is_empty(&self) -> bool {
        self.entries_today.is_empty() && !has_existing_files(&self.base_path)
    }
}

impl Drop for HistoryLog {
    fn drop(&mut self) {
        self.flush();
    }
}

#[allow(dead_code)]
fn has_existing_files(base: &Path) -> bool {
    let dir = match fs::read_dir(base) {
        Ok(d) => d,
        Err(_) => return false, // Directory doesn't exist or can't be read.
    };

    dir.flatten().any(|entry| is_history_file(&entry.path()))
}

fn is_history_file(path: &Path) -> bool {
    let name = match path.file_name().and_then(|s| s.to_str()) {
        Some(n) => n,
        None => return false,
    };
    if !name.starts_with(HISTORY_FILE_PREFIX) {
        return false;
    }
    // Ensure date portion is at least 8 chars (YYYYMMDD).
    let after_prefix = &name[HISTORY_FILE_PREFIX.len()..];
    after_prefix.len() >= 8 && after_prefix.chars().all(|c| c.is_ascii_digit())
}

/// Extract YYYYMMDD string from a history file path for BTreeMap sorting.
fn extract_date_str(path: &Path) -> Option<String> {
    let name = path.file_name()?.to_str()?;
    if let Some(date_part) = name.strip_prefix(HISTORY_FILE_PREFIX) {
        if date_part.len() == 8 && date_part.chars().all(|c| c.is_ascii_digit()) {
            return Some(date_part.to_string());
        }
    }
    None
}

fn read_entries_from_file(path: &Path) -> Vec<HistoryEntry> {
    let mut entries = Vec::new();

    let file = match File::open(path) {
        Ok(f) => f,
        Err(e) => {
            warn!("Failed to open history file {}: {}", path.display(), e);
            return entries;
        }
    };

    let mut reader = BufReader::new(file);
    let mut buf = Vec::new();

    if let Err(e) = reader.read_to_end(&mut buf) {
        warn!("Failed to read history file {}: {}", path.display(), e);
        return entries;
    }

    let mut offset = 0usize;
    while offset < buf.len() {
        match HistoryEntry::from_bytes(&buf[offset..]) {
            Some((entry, next_offset)) => {
                entries.push(entry);
                offset += next_offset;
            }
            None => break, // Corrupted or truncated entry at end.
        }
    }

    debug!("Read {} entries from {}", entries.len(), path.display());
    entries
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, SystemTime};

    fn sample_entry(timestamp_ns: u64) -> HistoryEntry {
        HistoryEntry::new(
            timestamp_ns,
            25.0,             // cpu per_core_max
            12.0,             // cpu total_average
            vec![45.0, 78.0], // gpu usages (2 GPUs)
            15.5,             // network mbps
            3.2,              // disk mb/s
            true,             // inhibited
        )
    }

    #[test]
    fn test_history_entry_serialization_roundtrip() {
        let now = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap();
        let entry = sample_entry(now.as_nanos() as u64);
        let bytes = entry.to_bytes();

        assert!(!bytes.is_empty(), "serialized entry should not be empty");

        let (decoded, consumed) =
            HistoryEntry::from_bytes(&bytes).expect("should decode valid entry");

        assert_eq!(consumed, bytes.len(), "should consume all bytes");
        assert_eq!(entry.timestamp_ns, decoded.timestamp_ns);
        assert!(
            (entry.cpu_usage.per_core_max - decoded.cpu_usage.per_core_max).abs() < f64::EPSILON
        );
        assert_eq!(
            entry.cpu_usage.total_average,
            decoded.cpu_usage.total_average
        );
        assert_eq!(entry.gpu_usages, decoded.gpu_usages);
        assert!((entry.network_mbps - decoded.network_mbps).abs() < f64::EPSILON);
        assert!((entry.disk_mb_s - decoded.disk_mb_s).abs() < f64::EPSILON);
        assert_eq!(entry.inhibited, decoded.inhibited);
    }

    #[test]
    fn test_history_entry_date_extraction() {
        let now = SystemTime::now();
        let ns = now
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_nanos() as u64;
        let entry = sample_entry(ns);

        // The date should match today's date.
        assert_eq!(entry.entry_date(), Local::now().date_naive());
    }

    #[test]
    fn test_history_log_file_operations() {
        let tmp_dir = tempfile::tempdir().unwrap();
        let base_path = tmp_dir.path().join("rouser");
        fs::create_dir_all(&base_path).unwrap();

        let now_ns = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_nanos() as u64;

        // Write entries directly to file.
        {
            let date_str = format!(
                "{}{}",
                HISTORY_FILE_PREFIX,
                Local::now().date_naive().format("%Y%m%d")
            );
            let file_path = base_path.join(date_str);

            let mut writer = BufWriter::new(File::create(&file_path).unwrap());
            let entry1 = sample_entry(now_ns);
            let entry2 = HistoryEntry {
                timestamp_ns: now_ns + 5_000_000_000, // +5s
                cpu_usage: CpuSnapshot {
                    per_core_max: 5.0,
                    total_average: 2.0,
                },
                gpu_usages: vec![10.0],
                network_mbps: 0.0,
                disk_mb_s: 0.0,
                inhibited: false,
            };

            writer.write_all(&entry1.to_bytes()).unwrap();
            writer.write_all(&entry2.to_bytes()).unwrap();
            writer.flush().unwrap();
        }

        // Read them back via HistoryLog::read_all() which scans the directory.
        let log = HistoryLog {
            base_path: base_path.clone(),
            entries_today: Vec::new(),
            last_prune_date: None,
        };

        let all_entries = log.read_all();
        assert_eq!(all_entries.len(), 2);
    }

    #[test]
    fn test_history_log_pruning() {
        let tmp_dir = tempfile::tempdir().unwrap();
        let base_path = tmp_dir.path().join("rouser");
        fs::create_dir_all(&base_path).unwrap();

        // Create an old history file (35 days ago, well within 8-digit YYYYMMDD format).
        let old_date = Local::now().date_naive() - chrono::Duration::days(35);
        let date_str_old = format!("{}{}", HISTORY_FILE_PREFIX, old_date.format("%Y%m%d"));
        let old_file = base_path.join(&date_str_old);
        File::create(&old_file).unwrap();

        // Create a recent history file (2 days ago).
        let recent_date = Local::now().date_naive() - chrono::Duration::days(2);
        let date_str_recent = format!("{}{}", HISTORY_FILE_PREFIX, recent_date.format("%Y%m%d"));
        let recent_file = base_path.join(&date_str_recent);
        File::create(&recent_file).unwrap();

        // Create a non-history file (should be skipped).
        let _ = File::create(base_path.join("other.txt")).unwrap();

        let mut log = HistoryLog {
            base_path: base_path.clone(),
            entries_today: Vec::new(),
            last_prune_date: None,
        };

        // Prune with 30-day retention.
        log.prune(Duration::from_secs(30 * 24 * 60 * 60));

        assert!(!old_file.exists(), "old file should be pruned");
        assert!(recent_file.exists(), "recent file should remain");
    }

    #[test]
    fn test_history_log_is_empty_initially() {
        let tmp_dir = tempfile::tempdir().unwrap();
        let log = HistoryLog {
            base_path: tmp_dir.path().join("rouser"),
            entries_today: Vec::new(),
            last_prune_date: None,
        };

        assert!(log.is_empty());
    }

    #[test]
    fn test_from_bytes_handles_short_buffer() {
        let result = HistoryEntry::from_bytes(&[1, 2]); // Less than 4 bytes for length prefix.
        assert!(result.is_none(), "should return None for too-short buffer");
    }

    #[test]
    fn test_from_bytes_handles_truncated_entry() {
        let now = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap();
        let entry = sample_entry(now.as_nanos() as u64);
        let bytes = entry.to_bytes();

        // Truncate to only first 10 bytes (less than total length + header for most entries).
        let truncated: Vec<u8> = bytes[..bytes.len().min(10)].to_vec();
        let result = HistoryEntry::from_bytes(&truncated);
        assert!(result.is_none(), "should return None for truncated entry");
    }

    #[test]
    fn test_is_history_file() {
        let tmp_dir = tempfile::tempdir().unwrap();

        let valid_path = tmp_dir.path().join("history.log.20250615");
        assert!(is_history_file(&valid_path));

        let invalid_prefix = tmp_dir.path().join("other.log.20250615");
        assert!(!is_history_file(&invalid_prefix));

        let no_date = tmp_dir.path().join("history.log.txt");
        assert!(
            !is_history_file(&no_date),
            "non-numeric date should be invalid"
        );
    }

    #[test]
    fn test_multiple_entries_serialization() {
        let now_ns = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_nanos() as u64;

        let entries: Vec<HistoryEntry> = (0..10)
            .map(|i| {
                HistoryEntry::new(
                    now_ns + i * 5_000_000_000, // 5s apart
                    (i as f64) * 10.0,
                    (i as f64) * 5.0,
                    vec![(i as f64) * 20.0],
                    i as f64,
                    (i as f64) / 10.0,
                    i % 3 == 0,
                )
            })
            .collect();

        // Write all to a temp file.
        let tmp_dir = tempfile::tempdir().unwrap();
        let file_path = tmp_dir.path().join("test.bin");

        {
            let mut writer = BufWriter::new(File::create(&file_path).unwrap());
            for entry in &entries {
                let bytes = entry.to_bytes();
                assert!(writer.write_all(&bytes).is_ok());
            }
            writer.flush().unwrap();
        }

        // Read back.
        let read_entries = read_entries_from_file(&file_path);
        assert_eq!(read_entries.len(), 10, "should have all entries");

        for (orig, decoded) in entries.iter().zip(read_entries.iter()) {
            assert_eq!(orig.timestamp_ns, decoded.timestamp_ns);
            assert!(
                (orig.cpu_usage.per_core_max - decoded.cpu_usage.per_core_max).abs() < f64::EPSILON
            );
            assert_eq!(orig.inhibited, decoded.inhibited);
        }
    }

    #[test]
    fn test_history_entry_gpu_usages_empty_vec() {
        let entry = HistoryEntry::new(0, 0.0, 0.0, vec![], 0.0, 0.0, false);
        assert!(entry.gpu_usages.is_empty());

        // Should serialize/deserialize fine with empty GPU array.
        let bytes = entry.to_bytes();
        let (decoded, _) = HistoryEntry::from_bytes(&bytes).unwrap();
        assert_eq!(decoded.gpu_usages.len(), 0);
    }

    #[test]
    fn test_history_entry_timestamp_ordering() {
        let mut entries: Vec<HistoryEntry> = (0..5)
            .rev() // Reverse order to test sorting.
            .map(|i| {
                HistoryEntry::new(
                    i as u64 * 1_000_000_000,
                    10.0,
                    20.0,
                    vec![],
                    0.0,
                    0.0,
                    false,
                )
            })
            .collect();

        entries.sort_by_key(|e| e.timestamp_ns);

        for i in 1..entries.len() {
            assert!(
                entries[i].timestamp_ns >= entries[i - 1].timestamp_ns,
                "entries should be sorted by timestamp"
            );
        }
    }
}
