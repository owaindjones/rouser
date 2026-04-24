# Averaging and Threshold Calculations

This document explains how `rouser` calculates metric values using Exponential Moving Average (EMA) smoothing and how thresholds are evaluated to determine sleep inhibition.

## The Problem with Simple Averaging

### Rapid Inhibit/Release Cycles

Simple averaging causes jarring behavior with bursty workloads:

**Scenario**: CPU spikes from 20% to 100% and back down:

```
Sample | Raw Value | Simple Average | Result
-------|-----------|----------------|--------
   1   |    20%    |      20%       | Below threshold (80%)
   2   |   100%    |      60%       | Below threshold (80%)
   3   |    20%    |      46%       | Below threshold (80%)
   4   |    20%    |      35%       | Below threshold (80%)
```

**Problem**: The average drops too quickly when activity decreases, causing sleep to resume before the workload is truly complete.

### GPU Averaging Hides Individual Spikes

With multiple GPUs, simple averaging hides important spikes:

```
GPU 1: 100% usage (rendering)
GPU 2:   0% usage (idle)
Average: 50% (below 80% threshold)

Result: System sleeps despite GPU 1 being fully utilized!
```

## Solution: Asymmetric EMA Smoothing

### What is EMA?

Exponential Moving Average (EMA) gives more weight to recent values while retaining some history:

```
smoothed_value = alpha * new_value + (1 - alpha) * previous_smoothed
```

- **alpha**: Smoothing factor (0.0 to 1.0)
- **higher alpha**: More responsive to changes, less smoothing
- **lower alpha**: Smoother but slower response

### Asymmetric Approach

rouser uses **different smoothing factors** for rising vs. falling values:

```rust
if new_value > current_smoothed {
    // Fast response to increases - detect spikes quickly
    factor = min(alpha * 2, 1.0)
} else {
    // Slow decay for decreases - prevent rapid release
    factor = min(alpha, 0.5) / 2
}

smoothed_value = factor * new_value + (1 - factor) * smoothed_value
```

### Example with alpha = 0.1

| Event | Raw Value | Factor | Smoothed Value | Notes |
|-------|-----------|--------|----------------|-------|
| Start | - | - | 20.0 | Initial value |
| Spike | 100% | 0.20 | 36.0 | Fast response (20% of change) |
| Spike | 100% | 0.20 | 48.8 | Continues rising |
| Spike | 100% | 0.20 | 59.0 | Reaches ~63% in 1 sample |
| Drop | 20% | 0.025 | 58.5 | Slow decay (2.5% of change) |
| Drop | 20% | 0.025 | 57.4 | Takes ~14 samples for 63% change |

**Key Insight**: 
- **Rising edge**: Reaches 63% of change in 1 sample
- **Falling edge**: Reaches 63% of change in ~14 samples (for alpha=0.1)

This asymmetry ensures:
1. **Quick spike detection**: Inhibition engages within 2-3 samples
2. **Gradual cooldown**: Release only after sustained low activity

## Configuration

### Per-Metric EMA Alpha

Each metric section has its own `ema_alpha` value:

```toml
[metrics.cpu]
threshold = 80.0
ema_alpha = 0.1        # Default smoothing for CPU

[metrics.gpu]
threshold = 90.0
ema_alpha = 0.2        # More responsive for GPU

[metrics.network]
threshold = 100.0
ema_alpha = 0.05       # Smoother for network bursts

[metrics.disk]
threshold = 50.0
ema_alpha = 0.1        # Standard smoothing for disk I/O
```

### Choosing Alpha Values

| alpha | Response Time | Smoothing | Best For |
|-------|--------------|-----------|----------|
| 0.05 | Slow (~28 samples for 63% change) | High | Stable workloads, bursty traffic |
| 0.10 | Medium (~14 samples for 63% change) | Medium | General purpose (default) |
| 0.20 | Fast (~7 samples for 63% change) | Low | Responsive, consistent workloads |
| 0.50 | Very Fast (~3 samples for 63% change) | Minimal | Real-time, latency-critical |

### Per-GPU EMA Smoothing

Each detected GPU applies the same `ema_alpha` from `[metrics.gpu]`, but independently. There is no per-GPU config override — the threshold and smoothing factor apply uniformly to all GPUs:

```toml
[metrics.gpu]
threshold = 90.0    # Applies to ALL detected GPUs
ema_alpha = 0.2     # Applied per-device, not globally averaged
```

This means GPU0(nvidia) at 95% and card1(amdgpu) at 87% are each compared against the same threshold independently — one exceeding it triggers inhibition regardless of the other's state.

## Threshold Evaluation

### Duration Threshold

Before inhibiting sleep, a metric must exceed its threshold for the configured duration:

```toml
[timing]
duration_threshold = "30s"  # Must exceed threshold for 30 seconds
```

**Process**:
1. Metric exceeds threshold
2. Start timer
3. If metric stays above threshold for `duration_threshold`, inhibit sleep
4. If metric drops below, reset timer

**Example** (5-second polling interval):

```
Time | CPU | Threshold | Duration Above | Action
-----|-----|-----------|----------------|--------
 0s  |  75%|    80%    | 0s             | Normal
 5s  |  85%|    80%    | 5s             | Timer started
10s  |  90%|    80%    | 10s            | Continue
15s  |  95%|    80%    | 15s            | Continue
20s  | 100%|    80%    | 20s            | Continue
25s  | 100%|    80%    | 25s            | Continue
30s  | 100%|    80%    | 30s            | INHIBIT SLEEP
```

### Idle Behavior — How Release Works

There is no `idle_duration` field. Instead, release behavior uses the EMA-smoothed value compared against the threshold with hysteresis timing:

1. All metrics fall below their respective thresholds (using smoothed values)
2. The `cooldown_duration` timer starts
3. If all metrics stay below for the full cooldown period, inhibition is released
4. If any metric exceeds its threshold during cooldown, the timer resets

**Example**:

```
Time | CPU  | Cooldown Timer | Action
-----|------|----------------|--------
30s  | 100% | N/A            | Already inhibiting (above 80%)
35s  |  70% | 5s             | Below threshold, cooldown starts
40s  |  65% | 10s            | Continue waiting
45s  |  85% | RESET          | Above threshold — timer resets
50s  |  90% | N/A            | Still inhibiting (above 80%)
60s  |  70% | 5s             | Below threshold, cooldown restarts
...  |  ... | ...            | ...
120s |  65% | 60s            | RELEASE INHIBITION
```

### Cooldown Duration — Re-inhibition Prevention

After releasing inhibition, rouser waits before re-inhibiting:

```toml
[timing]
cooldown_duration = "60s"  # Wait 60 seconds after release
```

**Purpose**: Prevents rapid inhibit/release cycles with bursty workloads.

**Example** (bursty CPU usage):

```
Time | CPU | State | Action
-----|-----|-------|--------
  0s |  75%| IDLE  | Normal
 10s |  95%| COUNTING | Above threshold, starting timer
 40s |  95%| INHIBITING | Duration met, inhibit sleep
 45s |  70%| HOLDING | Below threshold, start cooldown
105s |  70%| IDLE  | Cooldown complete, can re-inhibit
110s |  95%| COUNTING | Above threshold again
```

Without cooldown: Inhibit → Release → Inhibit → Release (jarring)
With cooldown: Inhibit → Hold → Release → Stable (smooth)

## Implementation Details

### Smoothing State

Each metric maintains its own smoothing state:

```rust
pub struct SmoothingState {
    smoothed: f64,      // Current smoothed value
    alpha: f64,         // Base smoothing factor
}

impl SmoothingState {
    pub fn update(&mut self, new_value: f64) -> f64 {
        let factor = if new_value > self.smoothed {
            // Rising edge: faster response
            (self.alpha * 2.0).min(1.0)
        } else {
            // Falling edge: slower decay
            self.alpha.min(0.5) / 2.0
        };
        
        self.smoothed = factor * new_value + (1.0 - factor) * self.smoothed;
        self.smoothed
    }
}
```

### Per-Metric Tracking

rouser tracks smoothing independently for each metric:

```rust
pub struct DataManager {
    cpu_state: SmoothingState,
    gpu_state: SmoothingState,
    network_state: SmoothingState,
    disk_state: SmoothingState,
    // ... per-GPU states if configured
}
```

### Convergence Behavior

The asymmetric EMA converges to stable values:

**Steady State** (constant input):
```
Input = 80% for all samples
Alpha = 0.1

Sample | Smoothed
-------|----------
   1   |  80.0  (initial)
   2   |  80.0  (no change when equal)
   3   |  80.0  (stable)
```

**Oscillation Dampening** (fluctuating input):
```
Input: 70% → 90% → 70% → 90%
Alpha = 0.1

Sample | Raw | Smoothed | Behavior
-------|-----|----------|----------
   1   | 70% |  70.0    | Start
   2   | 90% |  74.0    | Slow rise (+4%)
   3   | 70% |  73.5    | Slow fall (-0.5%)
   4   | 90% |  76.8    | Slow rise (+3.3%)
   5   | 70% |  76.4    | Slow fall (-0.4%)

Result: Oscillation dampened from ±20% to ±3.2%
```

## Best Practices

### Choosing Alpha Values

**For bursty workloads** (video rendering, downloads):
- Use lower alpha (0.05-0.10)
- Longer `cooldown_duration` (60-120s)
- Prevents rapid cycling during brief pauses

**For consistent workloads** (compilation, database):
- Use medium alpha (0.10-0.15)
- Standard timing values
- Responsive to actual changes

**For real-time requirements** (gaming, streaming):
- Use higher alpha (0.15-0.25)
- Shorter `duration_threshold` (10-20s)
- Faster response to spikes

### Tuning Recommendations

1. **Start with defaults**: `alpha=0.1`, `duration_threshold=30s`, `cooldown_duration=60s`
2. **Monitor logs**: Watch for frequent inhibit/release cycles
3. **Adjust alpha**: If too sensitive, lower alpha; if too sluggish, increase
4. **Tune timing**: If cycles persist, increase `cooldown_duration`
5. **Per-metric tuning**: Different metrics may need different alphas

### Example Configurations

**Home Server** (bursty network/disk):
```toml
name = "rouser"
update_interval = "5s"
log_level = "info"

[metrics.cpu]
threshold = 80.0
ema_alpha = 0.1        # Default smoothing for CPU

[metrics.gpu]
threshold = 90.0
ema_alpha = 0.2

[metrics.network]
threshold = 50.0
ema_alpha = 0.05       # Smoother for network bursts

[metrics.disk]
threshold = 30.0
ema_alpha = 0.05       # Smooth disk readings during bursty I/O

[timing]
duration_threshold = "60s"   # Longer to avoid inhibiting on brief idle gaps
cooldown_duration = "120s"   # Extended cooldown for workloads with natural pauses

[inhibitor]
what = "sleep:idle"
mode = "block"
```

**Development Workstation** (consistent CPU/GPU):
```toml
[metrics.cpu]
threshold = 90.0
ema_alpha = 0.15       # More responsive for compilation bursts

[metrics.gpu]
threshold = 95.0
ema_alpha = 0.2        # Responsive for GPU workloads

[metrics.network]
threshold = 100.0
ema_alpha = 0.2

[metrics.disk]
threshold = 50.0
ema_alpha = 0.1

[timing]
duration_threshold = "30s"   # Standard threshold timing
cooldown_duration = "60s"    # Standard cooldown period

[inhibitor]
what = "shutdown:idle:sleep:suspend"
mode = "block"
```

**Gaming System** (quick response):
```toml
[metrics.cpu]
threshold = 85.0
ema_alpha = 0.2        # Quick spike detection

[metrics.gpu]
threshold = 90.0
ema_alpha = 0.25       # Very responsive for gaming GPU activity

[timing]
duration_threshold = "15s"   # Shorter threshold — gamers prefer instant response
cooldown_duration = "30s"    # Quick cooldown between game sessions

[inhibitor]
what = "shutdown:idle:sleep:suspend"
mode = "block"
```

## See Also

- [Metrics Overview](metrics-overview.md) - How metrics are collected
- [Configuration Reference](configuration.md) - Full configuration options
- [Quick Start Guide](quickstart.md) - Getting started with rouser
