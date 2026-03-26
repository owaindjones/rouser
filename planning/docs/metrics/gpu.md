# GPU Metrics

## Overview

The `rouser` daemon collects GPU usage metrics to detect graphics-intensive workloads. Support varies by GPU vendor and driver availability.

## Metric Details

### Primary Metric

| Metric | Type | Unit | Description |
|--------|------|------|-------------|
| `gpu_usage` | Percentage | 0.0 - 100.0 | Aggregate GPU usage across all detected GPUs |

## Data Sources by GPU Type

### NVIDIA GPUs

**Primary Source**: `nvidia-smi` command-line tool

```bash
# Query all GPUs
nvidia-smi --query-gpu=utilization.gpu --format=csv,noheader,nounits

# Query specific GPU (index 0)
nvidia-smi -i 0 --query-gpu=utilization.gpu --format=csv,noheader,nounits
```

**Sample Output**:
```
95%
78%
```

**Implementation**:
```rust
fn get_nvidia_gpu_usage() -> Result<Vec<f64>> {
    let output = std::process::Command::new("nvidia-smi")
        .args(&["--query-gpu=utilization.gpu", "--format=csv,noheader,nounits"])
        .output()?;
    
    if !output.status.success() {
        return Err(GpuError::NvidiaNotAvailable);
    }
    
    let output_str = String::from_utf8_lossy(&output.stdout);
    output_str
        .lines()
        .filter_map(|line| {
            line.trim()
                .strip_suffix('%')
                .and_then(|v| v.parse::<f64>().ok())
        })
        .collect()
}

// Calculate average across all GPUs
fn calculate_average(usage_values: &[f64]) -> f64 {
    if usage_values.is_empty() {
        0.0
    } else {
        usage_values.iter().sum::<f64>() / usage_values.len() as f64
    }
}
```

**Fallback**: If `nvidia-smi` is unavailable or NVIDIA drivers are not installed, return 0% usage.

### AMD GPUs (ROCm)

**Primary Source**: `/sys/class/drm/cardX/device/gpu_busy_percent`

```bash
# Check GPU usage (if available)
cat /sys/class/drm/card0/device/gpu_busy_percent

# Or use rocm-smi tool (similar to nvidia-smi)
rocm-smi --showgpuutilization
```

**Implementation**:
```rust
fn get_amd_gpu_usage() -> Result<f64> {
    let mut total_usage = 0.0;
    let mut count = 0;
    
    // Try to find all GPU devices
    for entry in std::fs::read_dir("/sys/class/drm")? {
        let entry = entry?;
        let path = entry.path();
        
        if path.is_dir() && path.to_string_lossy().contains("card") {
            let busy_path = path.join("device/gpu_busy_percent");
            
            if let Ok(content) = std::fs::read_to_string(&busy_path) {
                if let Ok(usage) = content.trim().parse::<f64>() {
                    total_usage += usage;
                    count += 1;
                }
            }
        }
    }
    
    Ok(if count > 0 { total_usage / count as f64 } else { 0.0 })
}
```

**Requirements**:
- AMD ROCm drivers installed
- `uvm` (Unified Virtual Memory) subsystem loaded
- Access to `/sys/class/drm/cardX/device/`

**Important**: AMD GPU sysfs paths may not be available on all systems. Check for `/sys/class/drm/` directory existence first.

### Intel Integrated Graphics

**Primary Source**: `/sys/class/drm/cardX/device/gpu_busy_percent`

```bash
# Check GPU usage (if available)
cat /sys/class/drm/card0/device/gpu_busy_percent

# Alternative: intel_gpu_top tool
intel_gpu_top
```

**Implementation**:
```rust
fn get_intel_gpu_usage() -> Result<f64> {
    // Use same path as AMD (sysfs interface is similar)
    get_amd_gpu_usage()
}
```

**Requirements**:
- Intel i915 or newer display driver
- `/sys/class/drm/cardX/device/uvm/` directory may not exist on all systems
- `uvm` subsystem must be loaded for accurate measurements

**Important**: Intel integrated graphics sysfs paths may not be available on all systems. Many systems use `i915` kernel module without exposing `/sys/class/drm/cardX/device/uvm/`.

## Implementation Strategy

### Unified GPU Collector

```rust
pub struct GpuCollector {
    supported: bool,
    gpu_type: Option<GpuType>,
}

pub enum GpuType {
    Nvidia,
    AMD,
    Intel,
    Unknown,
}

impl GpuCollector {
    pub fn new() -> Self {
        Self {
            supported: Self::detect_gpu(),
            gpu_type: Self::detect_gpu_type(),
        }
    }
    
    fn detect_gpu() -> bool {
        // Check for nvidia-smi first (most reliable)
        if which::which("nvidia-smi").is_ok() {
            return true;
        }
        
        // Check for AMD/Intel sysfs
        Path::new("/sys/class/drm").exists()
    }
    
    fn detect_gpu_type() -> Option<GpuType> {
        if which::which("nvidia-smi").is_ok() {
            Some(GpuType::Nvidia)
        } else if Path::new("/sys/class/drm/card0/device/gpu_busy_percent").exists() {
            // Try to distinguish AMD vs Intel
            if Path::new("/sys/class/drm/card0/device/uvm").exists() {
                Some(GpuType::AMD)
            } else {
                Some(GpuType::Intel)
            }
        } else {
            None
        }
    }
    
    pub async fn collect_gpu_usage(&self) -> f64 {
        if !self.supported {
            log::warn!("No GPU support detected, returning 0% usage");
            return 0.0;
        }
        
        match self.gpu_type {
            Some(GpuType::Nvidia) => {
                match get_nvidia_gpu_usage() {
                    Ok(usage_values) => calculate_average(&usage_values),
                    Err(_) => {
                        log::warn!("nvidia-smi failed, returning 0% usage");
                        0.0
                    }
                }
            }
            Some(GpuType::AMD | GpuType::Intel) => {
                match get_amd_gpu_usage() {
                    Ok(usage) => usage,
                    Err(e) => {
                        log::warn!("GPU sysfs read failed: {:?}, returning 0% usage", e);
                        0.0
                    }
                }
            }
            None => 0.0,
        }
    }
}
```

## Error Handling

### Missing GPU Hardware

If no GPU is detected:
- Return 0% usage
- Log a warning only if logging level is `debug`
- Continue normal operation (GPU is optional metric)

### Driver or Kernel Module Not Available

**NVIDIA**:
- If `nvidia-smi` fails with "command not found", GPU not detected
- If `nvidia-smi` fails with "No devices detected", no NVIDIA GPU present

**AMD/Intel**:
- If `/sys/class/drm/` doesn't exist, GPU not detected
- If `gpu_busy_percent` file missing, fallback to 0%

### Graceful Degradation Strategy

```rust
impl GpuCollector {
    pub async fn collect_gpu_usage(&self) -> f64 {
        match self.collect_gpu_usage_attempt().await {
            Ok(usage) => usage,
            Err(GpuError::GpuNotFound) => {
                // Expected case - no GPU
                debug!("No GPU detected, returning 0% usage");
                0.0
            }
            Err(GpuError::DriverUnavailable) => {
                // Driver not loaded yet or failed to load
                warn!("GPU driver unavailable, retrying next poll");
                0.0
            }
            Err(e) => {
                // Unexpected error
                error!("Unexpected GPU error: {}", e);
                0.0
            }
        }
    }
}
```

## Multi-GPU Systems

### Support for Multiple GPUs

The daemon automatically detects and aggregates all GPUs:

```rust
// NVIDIA: Multiple GPUs via nvidia-smi
// Output: "95%\n78%\n100%" (one line per GPU)

// AMD/Intel: Multiple GPUs via /sys/class/drm/cardX
// Iterate through card0, card1, card2, etc.
```

### Aggregation Strategy

- **Average usage** across all GPUs (recommended)
- **Maximum usage** (alternative, more conservative)

```rust
// Average (default)
let avg = usage_values.iter().sum::<f64>() / usage_values.len() as f64;

// Maximum (for critical workloads)
let max = usage_values.iter().cloned().fold(0.0, f64::max);
```

## Configuration

### Threshold Configuration

```toml
[thresholds]
gpu_usage = 90.0  # Default 90%
```

**Valid Range**: 0.0 - 100.0
**Default**: 90.0

### GPU-Specific Configuration (Optional)

```toml
# Enable GPU monitoring
[gpu]
enabled = true

# Override aggregation method (average | maximum)
aggregation = "average"

# Which GPUs to monitor (by index)
# Empty array = all GPUs
monitor_gpus = []
```

## Performance Considerations

| Method | Overhead | Notes |
|--------|----------|-------|
| `nvidia-smi` | ~10-50ms per call | Caching recommended for frequent reads |
| sysfs read | <1ms per file | Very fast, no process spawn |
| `rocm-smi` | ~20-100ms per call | AMD alternative to nvidia-smi |

**Recommendation**: Cache `nvidia-smi` output for 1-second intervals to reduce overhead.

## Testing

### Unit Test Example

```rust
#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_average_calculation() {
        let usage = vec![90.0, 85.0, 95.0];
        let avg = calculate_average(&usage);
        assert!((avg - 90.0).abs() < 0.01);
    }
    
    #[test]
    fn test_empty_gpu_list() {
        let usage: Vec<f64> = vec![];
        let avg = calculate_average(&usage);
        assert_eq!(avg, 0.0);
    }
    
    #[test]
    fn test_detect_gpu_type() {
        // Test would require mocking file system
        // Placeholder for integration test
    }
}
```

### Integration Test

```rust
#[cfg(test)]
#[test]
fn test_gpu_collector_real() {
    let collector = GpuCollector::new();
    
    // Should not panic even if no GPU
    let usage = collector.collect_gpu_usage();
    
    assert!(usage >= 0.0 && usage <= 100.0);
}
```

## Security Considerations

- **nvidia-smi**: Requires read access to `/proc/driver/nvidia/` (typically restricted to root)
- **sysfs**: Read access to `/sys/class/drm/` (typically world-readable)

Run `rouser` as root or add user to appropriate groups:

```bash
# Add user to video group for GPU access
sudo usermod -aG video $USER
```

## References

- [NVIDIA nvidia-smi Documentation](https://developer.nvidia.com/nvidia-system-management-interface)
- [Linux Kernel Documentation - DRM Subsystem](https://www.kernel.org/doc/html/latest/driver-api/drm.html)
- [AMD ROCm Documentation](https://rocm.docs.amd.com/)

## See Also

- [CPU Metrics](cpu.md)
- [Network Metrics](network.md)
- [Disk Metrics](disk.md)
- [Memory Metrics](memory.md)
- [SECURITY.md](../security.md)
