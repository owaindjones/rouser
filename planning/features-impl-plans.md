# Implementation Plans — Q2 2026 Features

## Feature 1 (MINOR): State-change-only sleep inhibition logging

### Problem
In `service.rs:226`, the daemon emits an INFO log on **every polling cycle** while inhibited:
```rust
info!("Sleep inhibited: at least one metric above threshold");
```
This means logs like "Sleep inhibited: at least one metric above threshold" appear every 5 seconds (or whatever `update_interval` is set to).

### Solution
Track the previous inhibition state in a new field on `DataManager`. Only emit INFO logs when the state actually transitions. The existing release log at line ~234 already fires only on transition-out, so we need equivalent behavior for transition-in.

### Changes Required

#### File: `src/service.rs`

**1. Add field to `DataManager` struct (~line 102):**
```rust
// Track last tick's inhibition state for transition-only logging
previous_inhibited_state: bool,
```

**2. Initialize in `new()` (~line 160):**
```rust
previous_inhibited_state: false,
```

**3. Replace the per-tick log at line ~225-227:**
Remove the current code inside `tick()`:
```rust
// REMOVE THIS (lines ~225-227):
if self.state.is_inhibited() {
    info!("Sleep inhibited: at least one metric above threshold");
} else if let Some(below_since) = self.metrics_below_threshold_since { ... }
```

Replace with transition-aware logic that moves the inhibit log from inside `tick()` to after state transitions in `update_state()`. The cleanest approach is to track previous state and compare at end of tick:

At end of `tick()` (after `self.update_state(should_inhibit, config).await?`):
```rust
let current_inhibited = self.state.is_inhibited();
if !self.previous_inhibited_state && current_inhibited {
    info!("Sleep inhibited: at least one metric above threshold");
} else if self.previous_inhibited_state && !current_inhibited {
    // Release log already fires inside update_state() via the cooldown path
}
self.previous_inhibited_state = current_inhibited;
```

**4. Remove or adjust dry-run acquire logging:**
The `[DRY RUN] Would inhibit sleep` message at line ~298 should remain (it's a one-time per-acquisition), not changed.

### Verification
- No more INFO logs every 5 seconds while inhibited
- One INFO log when inhibition starts, one when it ends (after cooldown)  
- Dry-run mode still works with `[DRY RUN]` prefix messages
- All existing tests pass (`cargo test`)
- `cargo clippy -- -D warnings` passes

### Risk Assessment
**Low risk.** Simple field addition + conditional logic. Well-covered by existing code structure since the transition already happens in `update_state()`.

---

## Feature 2 (MEDIUM): Per-device GPU usage reporting

### Problem
GPU usage is reported at driver level: `GPU: NVIDIA: 0.0%, AMD/Intel: 0.0%` instead of per-GPU device: `GPU0(nvidia): 45.2%, card1(amdgpu): 78.1%`. The functional collection code aggregates all GPUs into a single average, which loses information about individual GPU activity.

### Solution
Refactor the GPU collector to return one `GpuData` entry per physical device with proper device ID and driver name. Keep nvidia-smi subprocess but change from averaged output to per-device parsing. For AMD/Intel, iterate sysfs entries individually instead of averaging them.

### Changes Required

#### File: `src/metrics/gpu.rs`

**1. Restructure `GpuData` struct:**
```rust
// OLD:
pub struct GpuData {
    pub vendor_type: &'static str,  // Static string like "NVIDIA", "AMD/Intel"  
    pub usage: f64,
}

// NEW:
#[derive(Debug, Clone)]
pub struct GpuData {
    pub device_id: String,   // e.g., "GPU0", "card1"
    pub driver_name: String, // e.g., "nvidia", "amdgpu", "i915", "xe", "unknown"
    pub usage: f64,          // utilization percentage 0.0-100.0
}

// Remove GpuStats and GpuVendor enums (unused in practice)
```

**2. Refactor `GpuCollector`:**

Change struct fields from boolean flags to proper tracking:
```rust
pub struct GpuCollector {
    nvidia_count: u32,      // Number of NVIDIA GPUs detected
}
```

**3. Implement per-device NVIDIA collection:**
Replace `collect_nvidia_all() -> Result<f64>` with `collect_nvidia_devices() -> Result<Vec<GpuData>>`:
```rust
fn collect_nvidia_devices(&self) -> Result<Vec<GpuData>, GpuError> {
    let output = Command::new("nvidia-smi")
        .args(&[
            "--query-gpu=index,utilization.gpu",
            "--format=csv,noheader,nounits",
        ])
        .output()
        .map_err(|e| GpuError::CommandFailed(format!("nvidia-smi: {}", e)))?;

    let output_str = String::from_utf8_lossy(&output.stdout);
    let mut gpus = Vec::new();

    for line in output_str.lines() {
        let usage_str = line.trim().strip_suffix('%').unwrap_or(line.trim());
        if let Ok(usage) = usage_str.parse::<f64>() {
            // We need the index from a separate query to get device_id
            gpus.push(GpuData {
                device_id: format!("GPU{}", gpus.len()),
                driver_name: "nvidia".to_string(),
                usage,
            });
        }
    }

    if gpus.is_empty() { debug!("No valid NVIDIA GPU data collected"); }
    Ok(gpus)
}
```

**4. Implement per-device AMD/Intel sysfs collection:**
Replace `collect_sysfs_all()` with proper per-device iteration:
```rust
fn collect_amd_intel_devices(&self) -> Result<Vec<GpuData>, GpuError> {
    let mut gpus = Vec::new();

    if let Ok(entries) = fs::read_dir("/sys/class/drm") {
        for entry in entries.flatten() {
            let path = entry.path();
            let name = path.file_name().and_then(|s| s.to_str()).unwrap_or("");
            
            // Only process cardN directories where N is numeric
            if !name.starts_with("card") || !path.is_dir() { continue; }
            
            // Extract card number
            let num: u32 = name[4..].parse().ok();
            let device_id = format!("GPU{}", num.unwrap_or(99));

            // Read gpu_busy_percent
            let busy_path = path.join("device/gpu_busy_percent");
            if let Ok(content) = fs::read_to_string(&busy_path) {
                if let Ok(usage) = content.trim().parse::<f64>() {
                    gpus.push(GpuData {
                        device_id,
                        driver_name: Self::detect_driver(&path),
                        usage,
                    });
                }
            }
        }
    }

    if gpus.is_empty() { debug!("No AMD/Intel GPU data collected"); }
    Ok(gpus)
}

fn detect_driver(card_path: &Path) -> String {
    let driver_link = card_path.join("device/driver");
    if let Ok(target) = fs::read_link(&driver_link) {
        // Symlink points to ../../pci/.../drivers/<name>
        // Walk back through the path or read target basename
        for component in target.components().rev() {
            if let std::path::Component::Normal(name) = component {
                if let Some(driver) = name.to_str() {
                    match driver {
                        "amdgpu" | "i915" | "xe" => return driver.to_string(),
                        _ => {} // Skip non-driver components like PCI names
                    }
                }
            }
        }
    }
    "unknown".to_string()
}
```

**5. Update `collect()` method:**
Replace the old aggregation approach with per-device collection:
```rust
pub async fn collect(&self) -> Result<Vec<GpuData>, GpuError> {
    let mut all_gpus: Vec<GpuData> = vec![];
    
    if self.has_nvidia() { /* ... */ }
    // Add nvidia devices...
    if self.has_amd_intel() { /* ... */ }
    // Add amd/intel devices...
    
    Ok(all_gpus)
}
```

**6. Remove obsolete structs:** `GpuStats`, `GpuVendor` (unused in practice, only `#[allow(dead_code)]`)

#### File: `src/metrics/mod.rs`

No changes needed — `GpuData` is already re-exported. Update the struct documentation comment if present.

#### File: `src/service.rs`

**1. Update GPU debug string formatting (~line 199-207):**
```rust
// OLD:
let gpu_debug: String = if !metrics.gpu_usage.is_empty() {
    metrics.gpu_usage.iter().enumerate()
        .map(|(i, g)| format!("{}: {:.1}%", g.vendor_type, g.usage))
        .collect::<Vec<_>>()
        .join(", ")
} else { "None".to_string() };

// NEW:
let gpu_debug: String = if !metrics.gpu_usage.is_empty() {
    metrics.gpu_usage.iter().map(|g| format!("{}({}): {:.1}%", g.device_id, g.driver_name, g.usage))
        .collect::<Vec<_>>()
        .join(", ")
} else { "None".to_string() };
```

**2. Update EMA smoothing initialization (~line 156-158):**
The current code initializes `gpu_smoothing` with a fixed size of 2 slots. With per-device collection, the number of GPUs is dynamic:
```rust
// Option A: Resize after first successful collection (preferred)
// In tick(), after collect():
let num_devices = metrics.gpu_usage.len();
while self.gpu_smoothing.len() < num_devices {
    self.gpu_smoothing.push(SmoothingState::new(config.metrics.gpu.ema_alpha));
}
self.gpu_smoothing.truncate(num_devices);

// Option B: Initialize with larger default (simpler but wastes memory)
let num_gpus = if has_gpu { 8 } else { 0 }; // More slots for multi-GPU systems
```

**3. Update threshold checking:** Already works per-device since `should_inhibit()` checks `gpu_smoothed_values.iter().any()`. No changes needed to the logic itself, but ensure it receives the full vec of smoothed values.

### Verification
- Debug string shows: `GPU0(nvidia): 45.2%, card1(amdgpu): 78.1%` format
- EMA smoothing works with variable GPU counts (0, 1, or many)
- All existing tests pass (`cargo test`)
- Dry-run mode shows correct per-device output
- `cargo clippy -- -D warnings` passes

### Risk Assessment
**Medium risk.** The GpuData struct change affects the API surface and requires updating all consumers (service.rs). However, the changes are straightforward string formatting + collection logic. Key risk is sysfs driver detection on unusual hardware configurations — mitigate with robust fallbacks to "unknown".

---

## Feature 3 (MAJOR): Investigate direct NVIDIA GPU access alternatives

### Problem
The current implementation spawns `nvidia-smi` as a subprocess and parses CSV output for every polling cycle. The user wants to know if there's a way to query NVIDIA GPU utilization without invoking external commands.

### Research Approach

**1. sysfs under `/sys/bus/pci/devices/`:**
NVIDIA drivers expose device info here but NOT real-time GPU compute utilization percentages in a reliable, well-documented interface. The `power/*` attributes show power draw on some cards but not usage %. This approach is **not viable**.

**2. NVML (NVIDIA Management Library):**
- Official C API via `libnvidia-ml.so`, shipped with proprietary drivers at `/usr/lib*/nvidia*/libnvidia-ml.so`
- Available crates:
  - `nvml-rs` — unmaintained since 2019, requires manual NVML linking
  - `opencl` + custom bindings — too heavy for this use case
  - `bindgen` + raw FFI to libnvidia-ml.so — feasible but adds build complexity and a new dependency (`bindgen`, `libclang-dev`)
- **Key constraint**: NVML is only available with NVIDIA proprietary drivers, not nouveau

**3. `/proc/driver/nvidia/`:**
Does NOT expose per-GPU utilization stats. Only has basic info about the driver state.

**4. X11 libXNVCtrl:**
Desktop-only (requires running display server), not suitable for headless servers. Also requires `libxnvctrl` package.

**5. Nouveau driver (`/sys/class/drm/`):**
The nouveau open-source driver does expose some utilization stats via sysfs but in a different format than AMD GPUs and with less reliable data.

### Decision: Keep nvidia-smi Subprocess, Improve Per-Device Parsing

**Rationale:**

1. **nvidia-smi is already required**: The `which::which("nvidia-smi")` check at `gpu.rs:42` means the daemon already requires this binary to function on NVIDIA systems. Keeping it as a subprocess doesn't add any new dependency.

2. **No good Rust NVML bindings exist**: The only crate (`nvml-rs`) is unmaintained (last update 2019). Using `bindgen` + raw FFI adds significant build complexity and maintenance burden for minimal gain — the subprocess approach works reliably.

3. **Per-device parsing via nvidia-smi is functionally equivalent**: The improved implementation (Feature 2) parses per-GPU data from nvidia-smi output, which provides the same information a direct API would. The only difference is process spawn overhead (~1-5ms per poll cycle), which is negligible compared to the polling interval (typically 5s).

4. **NVML requires additional packages**: `libnvidia-ml.so` comes with proprietary drivers but using it via FFI would require `bindgen` and potentially `libclang-dev` as build dependencies, increasing installation complexity for users on minimal systems.

**Recommendation:** Document this decision clearly in the codebase (add a comment block to `gpu.rs` explaining why nvidia-smi subprocess is acceptable), then proceed with Feature 2's per-device parsing approach which achieves the user's stated goals without adding NVML dependencies.

### Deliverable
- Decision documented as a module-level comment at top of `src/metrics/gpu.rs`
- Updated AGENTS.MD to reflect that nvidia-smi subprocess is the correct approach for NVIDIA GPUs

---
