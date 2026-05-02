# Prediction Model

The prediction module provides adaptive cooldown extension based on historical system usage patterns. When metrics drop below inhibition thresholds, rouser consults its learned patterns to determine whether it should extend the idle wait period before releasing sleep inhibition — reducing false-positive wake-ups during typical active-use hours.

## Overview

Without prediction, rouser releases sleep inhibition after a fixed `cooldown_duration` (default 10s) of all metrics being below threshold. With prediction enabled, if historical patterns indicate that similar times are usually followed by renewed activity, rouser extends this wait period by up to `max_extension_time`.

The model uses purely statistical pattern matching across three time dimensions — no external ML libraries or training pipelines required:
- **Year**: Captures seasonal trends (winter vs summer usage)
- **Week of year**: Captures monthly/annual cycles within a year
- **Seconds into week**: Precise position enabling hour-of-day + weekday/weekend distinction

## Data Collection

rouser collects metrics every `update_interval` seconds (root config, default 1s). Instead of writing each raw sample to the history log directly, it accumulates these per-tick samples in memory and writes an **averaged snapshot** at a longer interval defined by `[prediction].update_interval` (default 30s).

For example, with root `update_interval = "1s"` and prediction `update_interval = "30s"`, rouser collects 30 raw samples per minute, computes their arithmetic mean for each metric dimension, then writes one averaged data point to the history log. This produces smoother historical data that better represents sustained usage patterns rather than momentary spikes.

Each averaged snapshot contains:

| Field | Source | Description |
|-------|--------|-------------|
| Timestamp (nanoseconds) | System time | UTC epoch nanosecond precision of flush wall-clock time |
| CPU max per-core | `/proc/stat` | Average highest per-core usage across accumulated samples |
| GPU usages | NVML / sysfs | Per-GPU average utilization (averaged independently by slot index) |
| Network I/O | `/proc/net/dev` | Average throughput in Mbps across all monitored interfaces |
| Disk activity | `/proc/diskstats` | Average read + write throughput in MB/s |
| Inhibition state | Internal | Majority vote: true if rouser was inhibited for >50% of accumulated ticks |

### Rate-of-Change (Delta) Features

Each flushed snapshot also carries computed delta/rate-of-change fields that describe how metrics changed relative to the previous entry. These are calculated by comparing each averaged snapshot against its predecessor and stored alongside the raw metric values:

| Delta Field | Description |
|-------------|-------------|
| `elapsed_since_last_ns` | Nanoseconds elapsed since the previous flushed entry (None for first entry) |
| `cpu_delta_per_sec` | Rate of change of CPU per-core max in %/s (computed as delta / time_elapsed) |
| `network_delta_per_sec` | Rate of change of network throughput in Mbps/s |
| `disk_delta_per_sec` | Rate of change of disk throughput in MB/s/s |
| `gpu_deltas_per_sec` | Per-GPU rate of change array matching the order of GPU usages |

The first entry after startup has no predecessor and thus carries None/empty delta fields. Subsequent entries always have deltas computed from their immediate predecessor's metric values. These features enable trend-aware prediction (see [Trend-Aware Scoring](#trend-aware-scoring)).

### Gap Handling via Zero-Fill Interpolation

When the computer is shut down or sleeping, no data points are written to the history log. Without correction, this creates a temporal gap that causes the prediction model to be overfit on active-period data only — it would see high activity during those gaps and incorrectly predict future activity.

To address this, rouser detects large gaps (>5 minutes) between consecutive entries when loading history from disk and inserts **synthetic zero-value entries** at 30-second intervals within the gap. These synthetic records have all metric values set to 0 and `inhibited: false`, representing idle periods where no activity was recorded because the system was powered off or sleeping.

This approach ensures the prediction model sees a complete picture of both active and inactive periods, producing more accurate cooldown extensions that account for normal downtime patterns.

## Storage Layout

History files follow the naming pattern `history.log.YYYYMMDD` under:

- **User mode**: `$XDG_STATE_HOME/rouser/` (defaults to `~/.local/state/rouser/`)
- **Root mode**: `/var/lib/rouser/`

Each file contains only data points from that specific calendar day. Files are appended sequentially — new entries are written as binary blobs with a 4-byte length prefix followed by the bincode-encoded serde struct. This allows efficient streaming reads without loading entire files into memory for size estimation.

## How Prediction Works

### Step 1: Build Inhibition Histograms by Time Key

On initialization, rouser scans all existing history files and builds per-TimeKey inhibition histograms. Each data point is classified as inhibited or not based on the `inhibited` field (which reflects whether metrics exceeded thresholds at that time). The histogram counts how many times each `(year, week_of_year, seconds_into_week)` bucket was inhibited:

```
for entry in history_entries {
    if !entry.inhibited { continue; }
    let key = TimeKey::from_timestamp_ns(entry.timestamp_ns);  // (year, week, sec_in_week)
    inhibited_timekeys[key] += 1;
}
```

The `seconds_into_week` field encodes precise position within a 7-day cycle (0–604799.999 seconds, millisecond resolution), enabling fine-grained discrimination between Saturday morning vs Monday afternoon even though both share the same wall-clock hour. Combined with year and week-of-year axes, this captures seasonal, monthly, weekly, and weekday/weekend patterns in historical data.

### Step 2: Score Current Time Window on Cooldown Transition

When metrics drop below all thresholds and rouser is about to release inhibition, the model evaluates:

1. **Get current TimeKey** from system clock (year + week_of_year + seconds_into_week)
2. **Score via multi-level fallback matching**:
   - **Level 1 (exact match)**: Look up inhibition count at this exact `(year, week, second_position)` bucket — most precise when sufficient historical data exists for this specific time window.
   - **Level 2 (hour-of-day fallback)**: If no exact match, search all buckets within ±3600 seconds of the target `seconds_into_week` value. This recovers hour-of-day pattern matching behavior for sparse data.

The scoring formula normalizes each bucket's historical inhibition frequency against its average across all time keys:

```
ratio = count_at_timekey / avg_per_bucket
score = min(ratio * 0.5, 1.0)    # Scales above 0.5 for above-average hours
```

#### Trend-Aware Scoring (Delta Features)

In addition to the histogram-based inhibition scoring, rouser examines rate-of-change patterns from recent history entries when making predictions. This trend signal provides an additional dimension beyond pure time-key matching — it captures whether system activity is currently **rising** or **falling**, which helps distinguish between a temporary dip during active work versus genuine inactivity.

When `predict_cooldown()` is called, rouser reads the 20 most recent history entries and computes trend signals from their delta features:

1. Collects up to 20 most recent entries with populated delta fields
2. Computes average CPU rate-of-change (delta per second) across entries that have deltas
3. Computes average network I/O rate-of-change similarly
4. Normalizes both trends to a -0.2..=+0.2 adjustment range
5. Multiplies the base inhibition score by `(1 + cpu_trend + net_trend)`

The trend multiplier is bounded between 0.5 and 1.4, meaning rising activity can increase the prediction extension by up to 40%, while falling activity can reduce it by up to 50%. If metrics are trending upward during a period that was historically active at this time of day, rouser extends the cooldown further — anticipating renewed activity is likely. Conversely, if usage is declining toward idle, the extension is reduced since a release from inhibition is less risky.

This trend-aware approach complements the histogram-based scoring: it adds temporal momentum awareness to the static historical pattern matching, making predictions more responsive to current system behavior while still being grounded in learned patterns.

### Step 3: Map Score to Extension Time

If the score is below 0.3 (insufficient evidence of activity at this time window), no extension is applied — rouser uses the standard `cooldown_duration`.

For scores above 0.3, linear interpolation maps the score to an extension time between 0 and `max_extension_time`:

```
additional_time = ((score - 0.3) / 0.7) * max_extension_time
```

This produces a smooth curve: a score of 0.3 gives zero extension, while a score of 1.0 (very high historical inhibition at this time window) yields the full `max_extension_time`.

### Step 4: Confidence Scaling

The model reports a confidence value based on total data points collected:

| Data Points | Confidence | Interpretation |
|-------------|-----------|----------------|
| <50 | 0.1 | Insufficient data — extension unlikely to be meaningful |
| <500 | 0.3 | Some pattern recognition, but noisy |
| <5,000 | 0.6 | Good statistical basis for predictions |
| >=5,000 | 0.9 | Strong confidence in learned patterns |

Confidence is reported via logging only — it does not affect the extension calculation itself. The minimum threshold of 10 data points before any prediction is made provides a basic safety gate against completely uninformed extensions.

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

- **Startup**: `Loaded N history entries from ...` followed by `Prediction model initialized with M historical data points` — shows raw entries loaded and post-gap-filling count (M >= N since synthetic zero-fill entries are inserted for sleep/shutdown gaps)
- **Per-interval flush**: `Flushed averaged snapshot #N (CPU max=X.X%, net=X.XXMB/s, disk=X.XXMB/s, time=year=Y week=W sec=S, accumulated_ticks=N)` — logged when accumulated metrics are written as one averaged entry after N ticks; delta fields are computed from the previous flushed entry
- **Pruning activity**: Per-file debug lines when files are removed, plus an info-level summary once per day with `Pruned N old history files (retention: ...)`
- **Prediction query**: `Predicted cooldown: +Xdur (base_score=S.SS, trend_multiplier=T.TT, adjusted_score=S.SS, time=year=Y week=W sec=S, data_points=N, confidence=C.CC)` — shown when transitioning from inhibited to below-threshold state; includes the base inhibition score and the trend multiplier applied from delta features

## See Also

- [Configuration Reference](configuration.md) — All `[prediction]` config options with defaults
- [Metrics Overview](metrics-overview.md) — How CPU, GPU, network, disk metrics are collected
- [D-Bus Inhibition](d-bus-inhibition.md) — How sleep inhibition works under the hood
