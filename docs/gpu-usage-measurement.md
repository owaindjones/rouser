# GPU Usage Measurement

This document describes how rouser measures GPU utilization across different vendors, what each driver actually reports, and why these measurements are not directly comparable as percentages.

## Supported Drivers

rouser supports three families of GPU drivers:

| Driver | Vendor | Data Source | Collection Method |
|--------|--------|-------------|-------------------|
| **NVML** (`libnvidia-ml.so`) | NVIDIA proprietary / Nouveau (via NVML) | NVML library | `nvmlDeviceGetUtilizationRates()` via PCI bus ID matching to sysfs cards |
| **amdgpu** | AMD GPUs | `/sys/class/drm/cardX/device/gpu_busy_percent` | Direct sysfs file read, computed by SMU firmware |
| **i915 / xe** | Intel integrated/discrete GPUs (Arc) | `/sys/class/drm/cardX/device/gpu_busy_percent` | PMU engine busy ticks exposed via kernel counter |

NVIDIA is the only vendor where rouser uses an external library. AMD and Intel are read directly from sysfs with no additional dependencies. The NVML library (`libnvidia-ml.so`) is loaded dynamically at runtime — if it fails to load (no NVIDIA driver installed), rouser falls back gracefully without error.

## What Each Driver Measures

Despite all exposing a value in the range 0–100%, **the three drivers measure different things**. The percentage means something slightly different depending on which GPU vendor is being monitored.

### NVIDIA NVML (`gpu_busy_percent`)

**Definition**: *"Percent of time over the past sample period during which one or more kernels was executing on the GPU."* — [NVML API Reference](https://docs.nvidia.com/deploy/nvml-api/structnvmlUtilization__t.html)

The sample window is adaptive: 1 second for older products, down to ~167ms (1/6s) on newer architectures. This means NVIDIA readings can be noisier at short polling intervals compared to the more stable AMD/Intel firmware counters.

NVML measures **SM (Streaming Multiprocessor) compute activity** — specifically whether CUDA or OpenCL kernels are running shader cores. It does not directly account for:
- Video encode/decode engine usage (handled separately via `nvmlDeviceGetEncoderUtilization()` / `nvmlDeviceGetDecoderUtilization()`)
- Display compositing without kernel execution
- Memory controller activity (reported as a separate metric)

### AMD amdgpu (`gpu_busy_percent`)

**Definition**: *"The SMU firmware computes a percentage of load based on the aggregate activity level in the IP cores."* — [Kernel Documentation](https://docs.kernel.org/gpu/amdgpu/pm.html)

The System Management Unit (SMU) is a dedicated microcontroller on the AMD GPU that tracks **all GPU engines collectively** — GFX, Compute, SDMA, VCN/UVD encoders and decoders. The percentage represents aggregate hardware block busy time across all IP cores as seen by the SMU firmware.

This means amdgpu's `gpu_busy_percent` will register activity from media workloads (video playback, encoding) that NVML might report near-zero for on an NVIDIA card.

### Intel i915 / xe (`gpu_busy_percent`)

**Definition**: *"Percentage of time that the GPU was active."* — [i915 Kernel Documentation](https://docs.kernel.org/gpu/i915/index.html) / [xe Driver Docs](https://docs.xe.cloud/)

Measured via PMU (Performance Monitoring Unit) counters from the GuC (Graphics Microcontroller). The counter tracks total GT (Graphics Technology) engine activity including render, copy, video decode, and compute engines. Similar in scope to amdgpu's aggregate approach but implemented at the hardware PMU level rather than a separate firmware coprocessor.

## Key Differences

| Driver | What is Measured | Sample Period | Granularity |
|--------|-----------------|---------------|-------------|
| NVML | SM compute kernels only (~1s to 167ms) | Short, adaptive | Compute-focused |
| amdgpu | All IP cores aggregate via SMU firmware | ~1 second | Overall engine activity |
| i915 / xe | GT engine ticks via PMU counters | Varies by workload | Overall engine activity |

### Practical Implications

**NVML is compute-centric.** A GPU running only video decode (H.264/HEVC) or display compositing may show near-zero NVML utilization, whereas the same workload would register on AMD/Intel since those count all engines including encoders, copy engines, and media blocks.

**Sample windows differ.** NVIDIA's adaptive 1s to ~167ms window means its readings are inherently noisier at short polling intervals compared to the more stable firmware-level counters used by AMD and Intel. This can cause slightly different threshold crossing behavior when comparing mixed-vendor GPU workloads.

### Why This Matters for Sleep Inhibition

rouser applies a single configurable GPU utilization threshold across all GPUs regardless of vendor:

```toml
[metrics.gpu]
threshold = 20   # Inhibit sleep if any GPU exceeds this percentage
ema_alpha = 0.3
```

Because the three drivers measure different things, a "20% on NVIDIA" reading does not represent the same workload as "20% on AMD". However, for rouser's purpose — detecting whether the GPU is actively in use rather than precise benchmarking — this inconsistency is acceptable. All three drivers answer the question reliably enough: *"Is something using this GPU right now?"*

If per-vendor thresholds become necessary (e.g., `gpu_threshold_nvidia = 20` vs `gpu_threshold_amd = 40`), that can be addressed in a future enhancement to the configuration format.

## No Frequency Weighting on GPU Usage

Unlike CPU usage, which applies per-core frequency weighting (`raw_usage * current_freq / max_frequency` at `src/metrics/cpu.rs:404-436`), GPU utilization is reported as raw values from the driver with only EMA smoothing applied in the service loop. This is intentional — all three drivers normalize their busy percentage to a 0–100% scale regardless of current clock speed, so frequency weighting would be redundant.

## See Also

- [Metrics Overview](metrics-overview.md) — How rouser collects system metrics
- [Averaging and Thresholds](averaging.md) — EMA smoothing explained
