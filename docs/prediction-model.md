# Prediction Model

The prediction module provides adaptive cooldown extension based on historical system usage patterns. When metrics drop below inhibition thresholds, rouser consults its learned models to determine whether it should extend the idle wait period before releasing sleep inhibition — reducing false-positive wake-ups during typical active-use hours.

## Overview

Without prediction, rouser releases sleep inhibition after a fixed `cooldown_duration` (default 10s) of all metrics being below threshold. With prediction enabled, if historical patterns indicate that similar usage levels are typically followed by renewed activity, rouser extends this wait period by up to `max_extension_time`.

The model uses an **unsupervised streaming neural network** — specifically a Narmala-Gated Reservoir Computing (NG-RC) architecture from the [irithyll](https://crates.io/crates/irithyll) crate. Unlike the previous histogram-based approach that bucketed data by time-of-day, this model treats each metric dimension as an independent feature and learns normal usage patterns without requiring labeled training data.

### Architecture: Feature Vectors → Unsupervised Learning

Each history entry (flushed every `[prediction].update_interval`, default 30s) is converted into a fixed-size **feature vector** of six normalized values:

| Feature | Source | Description |
|---------|--------|-------------|
| CPU per-core max | `/proc/stat` | Highest individual core usage across all cores (0–100%) |
| CPU total average | `/proc/stat` | Average utilization across all cores weighted by frequency (0–100%) |
| GPU per-GPU max | NVML / sysfs | Maximum GPU utilization across all detected GPUs (0–100%) |
| GPU total average | NVML / sysfs | Mean utilization averaged across all GPUs (0–100%) |
| Network I/O | `/proc/net/dev` | Total throughput in Mbps across all monitored interfaces |
| Disk activity | `/proc/diskstats` | Combined read + write throughput in MB/s |

The model is **unsupervised** — it learns what "normal" system usage looks like by continuously updating its weights at each prediction `update_interval`. When metrics drop below inhibition thresholds, the model evaluates how anomalous the current state is compared to learned patterns. Higher anomaly scores produce longer cooldown extensions.

### Data Collection and Averaging

rouser collects raw metrics every root `update_interval` seconds (default 1s). It accumulates these per-tick samples in memory and writes an **averaged snapshot** at a longer interval defined by `[prediction].update_interval` (default 30s).

For example, with root `update_interval = "1s"` and prediction `update_interval = "30s"`, rouser collects 30 raw samples per minute, computes their arithmetic mean for each metric dimension, then writes one averaged data point to the history log. This produces smoother historical data that better represents sustained usage patterns rather than momentary spikes.

### Rate-of-Change (Delta) Features

Deltas are computed on-the-fly at prediction time by comparing consecutive flushed entries: `delta = (current - previous) / elapsed_time`. This avoids storing redundant rate-of-change data while preserving the ability to detect rising or falling trends across the historical record.

The following deltas are computed per-entry-pair:
- **CPU**: per-core max and total average change in %/s
- **GPU**: per-GPU max and total average change in %/s
- **Network**: throughput change in Mbps/s
- **Disk**: throughput change in MB/s/s

These deltas feed into the trend signal, which provides an additional dimension beyond raw metric values — helping distinguish between a temporary dip during active work versus genuine inactivity.

### Gap Handling via Zero-Fill Interpolation

When the computer is shut down or sleeping, no data points are written to the history log. Without correction, this creates a temporal gap that would cause the prediction model to be overfit on active-period data only — it would see high activity during those gaps and incorrectly predict future activity.

To address this, rouser detects gaps between consecutive entries at prediction time — any gap exceeding `[prediction].update_interval` is considered a large gap (e.g., >30s with default config). Rouser inserts **synthetic zero-value entries** at `update_interval` intervals within such gaps. These synthetic records have all metric values set to 0 and `inhibited: false`, representing idle periods where no activity was recorded because the system was powered off or sleeping. Synthetic entries exist only in memory during prediction; they are never written to history log files.

This approach ensures the model sees a complete picture of both active and inactive periods, producing more accurate cooldown extensions that account for normal downtime patterns. Gap-filled entries ARE included in feature vector construction — their all-zero values represent legitimate idle states that contribute to learning "normal" baselines.

## Storage Layout

History files follow the naming pattern `history.log.YYYYMMDD` under:

- **User mode**: `$XDG_STATE_HOME/rouser/` (defaults to `~/.local/state/rouser/`)
- **Root mode**: `/var/lib/rouser/`

Each file contains only data points from that specific calendar day. Files are appended sequentially — new entries are written as binary blobs with a 4-byte length prefix followed by the bincode-encoded serde struct. This allows efficient streaming reads without loading entire files into memory for size estimation.

## How Prediction Works

### Step 1: Load and Normalize History Entries

On initialization, rouser scans all existing history files and loads entries. At prediction time (when metrics drop below thresholds), it:

1. Selects recent entries within a timestamp window — entries where `timestamp >= current_time - max_extension_time` (e.g., the last hour with default config).
2. Filters out synthetic zero-value gap-filled entries from training data to prevent the model from learning idle-state patterns as "normal active use." However, these entries remain in history for baseline anomaly scoring.
3. Computes on-the-fly deltas between consecutive real entries (`(current - previous) / elapsed_time`).

### Step 2: Convert Entries to Feature Vectors and Train Model

Each selected entry is converted into a normalized feature vector — values are scaled using running statistics (mean, standard deviation) computed from the full history. The NG-RC reservoir computing model receives one sample at a time via its `StreamingLearner` trait, updating weights incrementally:

```rust
// At each prediction update_interval:
for entry in recent_entries {
    let features = feature_vector_from_entry(entry); // 6 normalized values
    ml_predictor.train(&features, &target_value)?;   // Online weight update
}
```

The NG-RC architecture uses a fixed random reservoir of neurons with delay embeddings to capture temporal patterns. Its key properties:
- **O(n²) memory** where n = hidden_dim (default 16 → ~4KB for weights + reservoir)
- **One sample at a time** training — no batches, no retraining from scratch
- **Temporal awareness** through delay buffers that create polynomial features from past states
- **Concept drift adaptation** via automatic weight adjustment when data distribution shifts

### Step 3: Anomaly Scoring and Extension Mapping

The model evaluates the current metrics as a feature vector. Since this is unsupervised, scoring is based on reconstruction error or prediction confidence — how well can the model predict today's state given what it has learned from historical patterns?

If the anomaly score exceeds a configurable threshold (default 0.3), rouser extends the cooldown:

```
if anomaly_score > min_threshold {
    additional_time = interpolate(anomaly_score, max_extension_time)
} else {
    additional_time = 0  // Use standard cooldown_duration
}
```

The score-to-extension mapping uses linear interpolation between `min_threshold` (default 0.3 → zero extension) and maximum observed anomaly levels (mapped to full `max_extension_time`). This produces smooth transitions rather than binary on/off behavior.

### Step 4: Confidence Scaling

The model reports a confidence value based on total data points collected:

| Data Points | Confidence | Interpretation |
|-------------|-----------|----------------|
| <50 | 0.1 | Insufficient data — extension unlikely to be meaningful |
| <500 | 0.3 | Some pattern recognition, but noisy |
| <5,000 | 0.6 | Good statistical basis for predictions |
| >=5,000 | 0.9 | Strong confidence in learned patterns |

Confidence is reported via logging only — it does not affect the extension calculation itself. The minimum threshold of 10 data points before any prediction is made provides a basic safety gate against completely uninformed extensions.

## Prediction Timing: update_interval, Not Every Tick

The cooldown extension prediction runs at the same cadence as history flushes — every `[prediction].update_interval` seconds (default 30s). This avoids redundant computation since the underlying data only changes when new averaged snapshots are written to disk. The model trains on newly available entries and produces a fresh prediction each time, rather than re-evaluating at every root `update_interval` tick.

## Pruning

History files older than `history_length` are automatically pruned on each tick cycle. The pruning function:

1. Computes a cutoff date by subtracting `history_length` duration from today
2. Scans the history directory for files matching `history.log.YYYYMMDD` pattern
3. Validates that filenames contain exactly 8 ASCII digits after the prefix (preventing path traversal via malicious filenames)
4. Deletes only confirmed regular files (symlinks and directories skipped)
5. Deduplicates by date — pruning runs at most once per calendar day

Pruning activity is logged: debug-level for each file removed, info-level summary when files are actually deleted. If no files need pruning (either because retention period hasn't passed or already pruned today), the operation returns silently.

## Configuration Tuning

### When to Increase `max_extension_time`

If rouser frequently releases inhibition and then re-inhibits within minutes during active work sessions, increase the extension cap:

```toml
[prediction]
max_extension_time = "2h"   # Extend up to 2 hours beyond standard cooldown
```

### When to Decrease `max_extension_time`

If rouser keeps the system awake longer than necessary (e.g., on a server that only needs brief inhibition during maintenance windows), reduce the cap:

```toml
[prediction]
max_extension_time = "15m"  # Short maximum extension for bursty workloads
```

### Disabling Prediction

Set `update_interval` to zero to disable all prediction while keeping metrics collection active:

```toml
[prediction]
update_interval = "0s"   # Disables prediction entirely
```

## Debugging

Enable debug logging to see the full prediction lifecycle:

```bash
RUST_LOG=debug rouser --dry-run
```

Key log messages:

- **Startup**: `Loaded N history entries from ...` followed by `Prediction model initialized with M historical data points` — shows raw entries loaded; gap-filling and trend computation happen at prediction time, not during startup
- **Per-interval flush**: `Flushed averaged snapshot #N (CPU max=X.X%, GPU max=Y.Y% avg=Z.Z%, net=X.XXMB/s, disk=X.XXMB/s), time={week_of_year}, accumulated_ticks=N` — logged when accumulated metrics are written as one averaged entry after N ticks; feature vectors are computed from these snapshots
- **Pruning activity**: Per-file debug lines when files are removed, plus an info-level summary once per day with `Pruned N old history files (retention: ...)`
- **Prediction query**: `Predicted cooldown: +Xdur (base_score=S.SS, trend_multiplier=T.TT, adjusted_score=S.SS, data_points=N, confidence=C.CC)` — shown when transitioning from inhibited to below-threshold state; includes the base anomaly score and the trend multiplier applied from delta features

## See Also

- [Configuration Reference](configuration.md) — All `[prediction]` config options with defaults
- [Metrics Overview](metrics-overview.md) — How CPU, GPU, network, disk metrics are collected
- [D-Bus Inhibition](d-bus-inhibition.md) — How sleep inhibition works under the hood
