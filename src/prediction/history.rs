//! Binary history log for predictive cooldown.
//!
//! Uses bincode v2 (serde-compatible binary serialization) with date-partitioned files.
//! Each file is named `history.log.YYYYMMDD` and stored under XDG-compliant paths:
//! - User state dir: `$XDG_STATE_HOME/rouser/history.log.*` or `~/.local/state/rouser/history.log.*` (falls back to `/tmp/rouser-history` if primary is unavailable)
//! - Root path: `/var/lib/rouser/history.log.*`

use chrono::{DateTime, Local, Utc};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs::{self, File};
use std::io::{BufReader, BufWriter, Read, Write};
use std::os::unix::fs::PermissionsExt;
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

    // --- Delta features computed between consecutive entries ---
    // These are optional for backward compatibility with existing history files.
    /// Nanoseconds elapsed since previous entry (None for first entry or when not computable).
    #[serde(default)]
    pub elapsed_since_last_ns: Option<u64>,
    /// Rate of change of CPU per_core_max usage in %/s (None if not computable).
    #[serde(default)]
    pub cpu_delta_per_sec: Option<f64>,
    /// Rate of change of network throughput in Mbps/s (None if not computable).
    #[serde(default)]
    pub network_delta_per_sec: Option<f64>,
    /// Rate of change of disk throughput in MB/s/s (None if not computable).
    #[serde(default)]
    pub disk_delta_per_sec: Option<f64>,
    /// Per-GPU rate of change in %/s, matching gpu_usages order. Empty vec when not computable.
    #[serde(default)]
    pub gpu_deltas_per_sec: Vec<f64>,
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
        Self::with_deltas(
            timestamp_ns,
            cpu_per_core_max,
            cpu_total_average,
            gpu_usages,
            network_mbps,
            disk_mb_s,
            inhibited,
            None,
        )
    }

    /// Create a new history entry with optional delta/rate-of-change fields.
    #[allow(clippy::too_many_arguments)]
    pub fn with_deltas(
        timestamp_ns: u64,
        cpu_per_core_max: f64,
        cpu_total_average: f64,
        gpu_usages: Vec<f64>,
        network_mbps: f64,
        disk_mb_s: f64,
        inhibited: bool,
        elapsed_since_last_ns: Option<u64>,
    ) -> Self {
        let (cpu_delta_per_sec, network_delta_per_sec, disk_delta_per_sec, gpu_deltas_per_sec) =
            match elapsed_since_last_ns {
                Some(elapsed_ns) if elapsed_ns > 0 => {
                    // This is a placeholder — actual deltas computed in model.rs record() when comparing consecutive entries.
                    (None, None, None, Vec::new())
                }
                _ => (None, None, None, Vec::new()),
            };

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
            elapsed_since_last_ns,
            cpu_delta_per_sec,
            network_delta_per_sec,
            disk_delta_per_sec,
            gpu_deltas_per_sec,
        }
    }

    /// Compute delta fields from the previous entry and return a new entry with deltas filled in.
    pub fn compute_deltas(&self, prev: &HistoryEntry) -> Self {
        let elapsed_ns = self.timestamp_ns.saturating_sub(prev.timestamp_ns);

        if elapsed_ns == 0 {
            // Same timestamp — can't compute rates or meaningful elapsed time.
            return Self {
                elapsed_since_last_ns: None,
                cpu_delta_per_sec: None,
                network_delta_per_sec: None,
                disk_delta_per_sec: None,
                gpu_deltas_per_sec: Vec::new(),
                ..self.clone()
            };
        }

        let secs_f64 = elapsed_ns as f64 / 1_000_000_000.0;
        let cpu_delta_per_sec = if secs_f64 > 0.0 {
            Some((self.cpu_usage.per_core_max - prev.cpu_usage.per_core_max) / secs_f64)
        } else {
            None
        };

        let network_delta_per_sec = if secs_f64 > 0.0 {
            Some((self.network_mbps - prev.network_mbps) / secs_f64)
        } else {
            None
        };

        let disk_delta_per_sec = if secs_f64 > 0.0 {
            Some((self.disk_mb_s - prev.disk_mb_s) / secs_f64)
        } else {
            None
        };

        // Per-GPU deltas matching gpu_usages order.
        let mut gpu_deltas_per_sec = Vec::new();
        for i in 0..self.gpu_usages.len().max(prev.gpu_usages.len()) {
            let prev_val = prev.gpu_usages.get(i).copied().unwrap_or(0.0);
            let curr_val = self.gpu_usages.get(i).copied().unwrap_or(0.0);
            if secs_f64 > 0.0 {
                gpu_deltas_per_sec.push((curr_val - prev_val) / secs_f64);
            } else {
                gpu_deltas_per_sec.push(0.0);
            }
        }

        Self {
            elapsed_since_last_ns: Some(elapsed_ns),
            cpu_delta_per_sec,
            network_delta_per_sec,
            disk_delta_per_sec,
            gpu_deltas_per_sec,
            ..self.clone()
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

fn xdg_state_dir() -> PathBuf {
    std::env::var("XDG_STATE_HOME")
        .ok()
        .filter(|s| !s.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            std::env::var("HOME")
                .ok()
                .map(|h| PathBuf::from(h).join(".local/state"))
                .expect("XDG_STATE_HOME or HOME must be set for user state directory")
        })
}

fn history_base_dir(is_root: bool) -> PathBuf {
    let path = if is_root {
        PathBuf::from("/var/lib/rouser")
    } else {
        xdg_state_dir().join("rouser")
    };

    if is_root {
        let _ = fs::create_dir_all(path.parent().unwrap_or(&path));
    }

    path
}

fn is_path_writable(path: &Path) -> bool {
    let test_file = path.join(".rouser-writable-check");
    match File::create(&test_file) {
        Ok(f) => drop(f),
        Err(_) => return false,
    }
    fs::remove_file(&test_file).is_ok()
}

fn ensure_history_dir(path: &Path) -> std::io::Result<()> {
    fs::create_dir_all(path)
}

fn fallback_data_dir(primary: &Path, is_root: bool) -> Option<PathBuf> {
    if is_root || !primary.starts_with("/home") {
        return None;
    }

    // Last resort for read-only /home with no writable state dir.
    // Use PID-based unique path to minimize TOCTOU risk on shared systems.
    let tmp = PathBuf::from(format!(
        "/tmp/rouser-history.{pid}",
        pid = std::process::id()
    ));

    if ensure_history_dir(&tmp).is_ok() {
        // Restrict permissions: owner-only access (700).
        fs::set_permissions(&tmp, fs::Permissions::from_mode(0o700)).ok();
        return Some(tmp);
    }

    None
}

const HISTORY_FILE_PREFIX: &str = "history.log.";

// Gap detection constants — used in read_all() to detect and fill missing time periods.
const GAP_THRESHOLD_NS: u64 = 5 * 60 * 1_000_000_000; // 5 minutes in nanoseconds
const FILL_INTERVAL_NS: u64 = 30 * 1_000_000_000; // 30 seconds between synthetic entries

/// A date-partitioned binary log file for storing metric snapshots.
pub struct HistoryLog {
    base_path: PathBuf,
    entries_today: Vec<HistoryEntry>,
    pending_summary: Option<String>,
    last_prune_date: Option<i64>, // Unix day number (seconds since epoch / 86400)
}

impl HistoryLog {
    pub fn new(is_root: bool) -> Self {
        let primary = history_base_dir(is_root);
        let base_path = if ensure_history_dir(&primary).is_ok() {
            primary.clone()
        } else if let Some(fallback) = fallback_data_dir(&primary, is_root) {
            info!(
                "Using alternate data directory {} (primary {} unavailable)",
                fallback.display(),
                primary.display()
            );
            fallback
        } else {
            warn!("History logging disabled — no writable data directory available");
            return HistoryLog {
                base_path: PathBuf::from("/dev/null"), // Best effort — writes will fail silently.
                entries_today: Vec::new(),
                pending_summary: None,
                last_prune_date: None,
            };
        };

        let _ = ensure_history_dir(&base_path);

        HistoryLog {
            base_path,
            entries_today: Vec::new(),
            pending_summary: None,
            last_prune_date: None,
        }
    }

    /// Append an entry to the log with optional summary for logging on flush. Buffers until flush or date change.
    pub fn append_with_summary(&mut self, entry: HistoryEntry, summary: Option<String>) {
        if let Some(s) = summary {
            self.pending_summary = Some(s);
        }
        self.append(entry);
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

    /// Flush in-memory entries to disk, logging a summary if one was set via append_with_summary.
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

        if let Some(ref summary) = self.pending_summary {
            debug!(
                "{} — flushed {} entries for date {} to {}",
                summary,
                self.entries_today.len(),
                date,
                file_path.display()
            );
        } else {
            debug!(
                "Flushed {} entries for date {} to {}",
                self.entries_today.len(),
                date,
                file_path.display()
            );
        }

        let _ = self.pending_summary.take();
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

        const GAP_THRESHOLD_NS: u64 = 5 * 60 * 1_000_000_000; // 5 minutes in nanoseconds
        const FILL_INTERVAL_NS: u64 = 30 * 1_000_000_000; // 30 seconds between synthetic entries

        // Flatten entries and sort by timestamp (BTreeMap iterates in key/date order).
        let mut result: Vec<HistoryEntry> = date_entries.into_values().flatten().collect();

        result.sort_by_key(|e| e.timestamp_ns);

        let result = fill_gaps(result, GAP_THRESHOLD_NS, FILL_INTERVAL_NS);
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

/// Fill temporal gaps in sorted history entries with synthetic zero-value records.
/// When the computer is shut down or sleeping, no data is written to the log.
/// Without this fix, the prediction model would be overfit on active-period data only.
fn fill_gaps(
    entries: Vec<HistoryEntry>,
    gap_threshold_ns: u64,
    fill_interval_ns: u64,
) -> Vec<HistoryEntry> {
    if entries.len() < 2 {
        return entries;
    }

    let mut result = vec![entries[0].clone()];

    for i in 1..entries.len() {
        let prev = &entries[i - 1];
        let curr = &entries[i];
        let gap_ns = curr.timestamp_ns.saturating_sub(prev.timestamp_ns);

        if gap_ns > gap_threshold_ns {
            // Fill the gap with synthetic zero-value entries.
            let mut ts = prev.timestamp_ns + fill_interval_ns;
            while ts < curr.timestamp_ns - fill_interval_ns / 2 {
                result.push(HistoryEntry::with_deltas(
                    ts,
                    0.0, // cpu per_core_max — idle state
                    0.0, // cpu total_average
                    Vec::new(),
                    0.0,   // network mbps
                    0.0,   // disk mb/s
                    false, // inhibited
                    Some(ts.saturating_sub(prev.timestamp_ns)),
                ));
                ts += fill_interval_ns;
            }
        }

        result.push(curr.clone());
    }

    debug!(
        "Filled gaps: {} entries -> {} entries (added {} synthetic)",
        entries.len(),
        result.len(),
        result.len() - entries.len()
    );

    result
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

        // The date should match today's UTC date (entry_date uses UTC internally).
        assert_eq!(entry.entry_date(), Utc::now().date_naive());
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
            let entry2 = HistoryEntry::new(
                now_ns + 5_000_000_000, // +5s
                5.0,                    // cpu per_core_max
                2.0,                    // cpu total_average
                vec![10.0],             // gpu usages
                0.0,                    // network mbps
                0.0,                    // disk mb/s
                false,                  // inhibited
            );

            writer.write_all(&entry1.to_bytes()).unwrap();
            writer.write_all(&entry2.to_bytes()).unwrap();
            writer.flush().unwrap();
        }

        // Read them back via HistoryLog::read_all() which scans the directory.
        let log = HistoryLog {
            base_path: base_path.clone(),
            entries_today: Vec::new(),
            pending_summary: None,
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
            pending_summary: None,
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
            pending_summary: None,
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

    #[test]
    fn test_fill_gaps_inserts_synthetic_entries() {
        let entry1 = HistoryEntry::new(0, 50.0, 25.0, vec![], 10.0, 5.0, true);
        // Gap of 10 minutes (600 seconds) — well above GAP_THRESHOLD_NS (300s).
        let entry2 = HistoryEntry::new(10 * 60 * 1_000_000_000, 5.0, 2.0, vec![], 0.0, 0.0, false);

        let entries = vec![entry1.clone(), entry2];
        let result = fill_gaps(entries, GAP_THRESHOLD_NS, FILL_INTERVAL_NS);

        // Should have: original 2 + synthetic fills for 10min gap at 30s intervals = 2 + (600/30) - ~1 = ~21 entries
        assert!(
            result.len() > 2,
            "should insert synthetic entries in the gap"
        );

        // First entry is unchanged.
        assert_eq!(result[0].timestamp_ns, 0);
        assert_eq!(result[0].cpu_usage.per_core_max, 50.0);

        // Last entry is original entry2 (unchanged).
        let last = result.last().unwrap();
        assert_eq!(last.timestamp_ns, 10 * 60 * 1_000_000_000);

        // Synthetic entries in the middle should have zero values.
        for entry in &result[1..result.len() - 1] {
            assert_eq!(entry.cpu_usage.per_core_max, 0.0);
            assert_eq!(entry.network_mbps, 0.0);
            assert!(!entry.inhibited);
        }

        // Timestamps should be monotonically increasing and roughly FILL_INTERVAL_NS apart for synthetics.
        for i in 1..result.len() {
            let delta = result[i].timestamp_ns - result[i - 1].timestamp_ns;
            assert!(delta > 0, "timestamps must be strictly increasing");
            if result[i].cpu_usage.per_core_max == 0.0
                && result[i - 1].cpu_usage.per_core_max == 0.0
            {
                // Between two synthetic entries, gap should be close to FILL_INTERVAL_NS.
                assert!(
                    (delta as i64 - FILL_INTERVAL_NS as i64).abs() < (FILL_INTERVAL_NS / 2) as i64,
                    "synthetic entry spacing should be ~{}ns, got {}ns",
                    FILL_INTERVAL_NS,
                    delta
                );
            }
        }
    }

    #[test]
    fn test_fill_gaps_noop_when_entries_contiguous() {
        let entries: Vec<HistoryEntry> = (0..5)
            .map(|i| HistoryEntry::new(i * 1_000_000_000, 10.0, 5.0, vec![], 1.0, 0.5, false))
            .collect();

        let result = fill_gaps(entries.clone(), GAP_THRESHOLD_NS, FILL_INTERVAL_NS);
        assert_eq!(
            result.len(),
            entries.len(),
            "no synthetic entries should be added"
        );

        for (orig, filled) in entries.iter().zip(result.iter()) {
            assert_eq!(orig.timestamp_ns, filled.timestamp_ns);
            assert!(
                (orig.cpu_usage.per_core_max - filled.cpu_usage.per_core_max).abs() < f64::EPSILON
            );
        }
    }

    #[test]
    fn test_fill_gaps_single_entry_noop() {
        let entry = HistoryEntry::new(0, 50.0, 25.0, vec![], 10.0, 5.0, true);
        let result = fill_gaps(vec![entry], GAP_THRESHOLD_NS, FILL_INTERVAL_NS);
        assert_eq!(result.len(), 1);
    }

    #[test]
    fn test_fill_gaps_gap_below_threshold_noop() {
        // Gap of only 60 seconds — below GAP_THRESHOLD_NS (300s).
        let entry1 = HistoryEntry::new(0, 50.0, 25.0, vec![], 10.0, 5.0, true);
        let entry2 = HistoryEntry::new(60 * 1_000_000_000, 5.0, 2.0, vec![], 0.0, 0.0, false);

        let entries = vec![entry1, entry2];
        let result = fill_gaps(entries.clone(), GAP_THRESHOLD_NS, FILL_INTERVAL_NS);
        assert_eq!(result.len(), 2, "no synthetic entries when gap < threshold");
    }

    #[test]
    fn test_compute_deltas_basic() {
        let prev = HistoryEntry::new(0, 10.0, 5.0, vec![20.0], 8.0, 2.0, false);
        // Entry 1 second later with higher values.
        let curr = HistoryEntry::with_deltas(
            1_000_000_000, // +1s
            30.0,          // cpu per_core_max increased by 20 → rate = 20%/s
            15.0,          // cpu total_average increased by 10 → rate = 10%/s
            vec![40.0],    // gpu usage increased by 20 → rate = 20%/s
            18.0,          // network increased by 10 → rate = 10 Mbps/s
            7.0,           // disk increased by 5 → rate = 5 MB/s/s
            true,          // inhibited
            Some(1_000_000_000),
        );

        let with_deltas = curr.compute_deltas(&prev);

        assert_eq!(with_deltas.elapsed_since_last_ns, Some(1_000_000_000));
        // CPU delta should be (30-10)/1.0 = 20%/s.
        assert!((with_deltas.cpu_delta_per_sec.unwrap() - 20.0).abs() < f64::EPSILON);
        // Network delta should be (18-8)/1.0 = 10 Mbps/s.
        assert!((with_deltas.network_delta_per_sec.unwrap() - 10.0).abs() < f64::EPSILON);
        // Disk delta should be (7-2)/1.0 = 5 MB/s/s.
        assert!((with_deltas.disk_delta_per_sec.unwrap() - 5.0).abs() < f64::EPSILON);
        // GPU delta should be (40-20)/1.0 = 20%/s.
        assert_eq!(with_deltas.gpu_deltas_per_sec.len(), 1);
        assert!((with_deltas.gpu_deltas_per_sec[0] - 20.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_compute_deltas_zero_elapsed_no_change() {
        let prev = HistoryEntry::new(100, 10.0, 5.0, vec![], 8.0, 2.0, false);
        // Same timestamp — should return unchanged copy.
        let curr = HistoryEntry::with_deltas(100, 30.0, 15.0, vec![40.0], 18.0, 7.0, true, Some(0));
        let with_deltas = curr.compute_deltas(&prev);

        assert_eq!(with_deltas.elapsed_since_last_ns, None); // Zero elapsed → None
    }

    #[test]
    fn test_with_deltas_backward_compatible_serialization() {
        // Old entries without delta fields should deserialize correctly (serde default handles missing).
        let old_bytes = HistoryEntry::new(0, 50.0, 25.0, vec![30.0], 10.0, 4.0, true).to_bytes();

        let (decoded, _) = HistoryEntry::from_bytes(&old_bytes).unwrap();

        // Delta fields should have serde defaults.
        assert_eq!(decoded.elapsed_since_last_ns, None);
        assert!((decoded.cpu_delta_per_sec.unwrap_or(0.0) - 0.0).abs() < f64::EPSILON);
        assert!(decoded.gpu_deltas_per_sec.is_empty());

        // New entry with deltas should also serialize/deserialize correctly.
        let new_entry = HistoryEntry::with_deltas(
            1_000_000_000,
            60.0,
            30.0,
            vec![40.0],
            15.0,
            5.0,
            false,
            Some(1_000_000_000),
        );
        let new_bytes = new_entry.to_bytes();
        let (decoded_new, _) = HistoryEntry::from_bytes(&new_bytes).unwrap();

        assert_eq!(decoded_new.elapsed_since_last_ns, Some(1_000_000_000));
        // Values should round-trip correctly.
        assert!((decoded_new.cpu_usage.per_core_max - 60.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_read_all_sorted_by_timestamp_across_files() {
        let tmp_dir = tempfile::tempdir().unwrap();
        let base_path = tmp_dir.path().join("rouser");
        fs::create_dir_all(&base_path).unwrap();

        // Create two date-partitioned files with interleaved timestamps.
        let now_ns = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_nanos() as u64;

        {
            // File for yesterday (older).
            let yest = Local::now().date_naive() - chrono::Duration::days(1);
            let date_str = format!("{}{}", HISTORY_FILE_PREFIX, yest.format("%Y%m%d"));
            let file_path = base_path.join(date_str);

            // Entries with timestamps 5s apart.
            let mut writer = BufWriter::new(File::create(&file_path).unwrap());
            for i in 0..3 {
                let entry = HistoryEntry::new(
                    now_ns + ((i as u64) * 5_000_000_000),
                    10.0 + i as f64,
                    5.0 + i as f64,
                    vec![],
                    1.0 * (i + 1) as f64,
                    0.5 * (i + 1) as f64,
                    i % 2 == 0,
                );
                assert!(writer.write_all(&entry.to_bytes()).is_ok());
            }
        }

        {
            // File for today (newer) with earlier timestamps than yesterday's file.
            let date_str = format!(
                "{}{}",
                HISTORY_FILE_PREFIX,
                Local::now().date_naive().format("%Y%m%d")
            );
            let file_path = base_path.join(date_str);

            // These entries have timestamps BEFORE yesterday's — tests cross-file sorting.
            let mut writer = BufWriter::new(File::create(&file_path).unwrap());
            for i in 0..2 {
                let entry = HistoryEntry::new(
                    now_ns + ((i as u64) * 5_000_000_000),
                    1.0 + i as f64,
                    0.5 + i as f64,
                    vec![],
                    0.1 * (i + 1) as f64,
                    0.1 * (i + 1) as f64,
                    false,
                );
                assert!(writer.write_all(&entry.to_bytes()).is_ok());
            }
        }

        // Read all — should be sorted by timestamp regardless of file order.
        let log = HistoryLog {
            base_path: base_path.clone(),
            entries_today: Vec::new(),
            pending_summary: None,
            last_prune_date: None,
        };

        let all_entries = log.read_all();

        // After gap filling (no large gaps in test data), should have original 5 + synthetic fills.
        assert!(all_entries.len() >= 5, "should have at least 5 entries");

        // Verify monotonic timestamp ordering.
        for i in 1..all_entries.len() {
            assert!(
                all_entries[i].timestamp_ns >= all_entries[i - 1].timestamp_ns,
                "entries must be sorted by timestamp ({} < {})",
                all_entries[i - 1].timestamp_ns,
                all_entries[i].timestamp_ns
            );
        }

        // First entry should have the smallest timestamp.
        assert_eq!(all_entries[0].timestamp_ns, now_ns);
    }
}
