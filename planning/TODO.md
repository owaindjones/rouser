# rouser - Task Tracker

## Phase 1: Planning & Research

### 1.1 Documentation
- [x] Create PROJECT.md with requirements
- [x] Create architecture overview
  - Created `docs/architecture/overview.md` (merged with ARCHITECTURE.md)
  - 500+ lines documenting components, design patterns, error handling
- [x] Create systemd service documentation
  - Created `docs/systemd/service.md`
  - Includes service file template, logging, security hardening
- [x] Create D-Bus inhibition documentation
  - Created `docs/d-bus/inhibition.md` (merged with DBUS_SLEEP_INHIBITION_API.md)
  - 800+ lines with API reference, examples, error handling
- [x] Create metric collection documentation
  - Documented in `docs/metrics/cpu.md`, `gpu.md`, `network.md`, `disk.md`, `memory.md`
  - Comprehensive coverage of data sources and implementation
- [x] Create configuration reference
  - Created `docs/configuration/reference.md`
  - All config options documented with TOML examples
- [x] Create quick start guide
  - Created `docs/quickstart.md`
  - Installation, testing, troubleshooting steps
- [x] Create security best practices
  - Created `docs/security.md`
  - Security hardening, file permissions, dependency management
- [x] Create performance documentation
  - Created `docs/performance.md`
  - Benchmarks, optimization guidelines, scaling considerations
- [x] Create doc-updates.md
  - Documented all issues found during research
  - 300+ lines of critical findings and recommendations

### 1.2 Research
- [x] Research Linux CPU metrics collection (/proc/stat, /proc/cpuinfo, perf)
  - Documented: `/proc/stat` for CPU usage calculation
  - Verified field order against kernel documentation
  - Addressed jiffies overflow handling
- [x] Research Linux GPU metrics collection (NVIDIA, AMD, Intel options)
  - Documented: `nvidia-smi` for NVIDIA, `/sys/class/drm/` for AMD/Intel
  - Addressed sysfs path availability issues
- [x] Research Linux network I/O metrics (/proc/net, iproute2, netlink)
  - Documented: `/proc/net/dev` for network I/O
  - Addressed interface filtering and loopback exclusion
- [x] Research Linux disk I/O metrics (/proc/diskstats, iostat)
  - Documented: `/proc/diskstats` for disk activity
  - Addressed virtual device filtering (LVM handling)
- [x] Research systemd-logind D-Bus API for sleep inhibition
  - Documented in `docs/d-bus/inhibition.md`
  - Service: `org.freedesktop.login1`, Interface: `org.freedesktop.login1.Manager`
  - Verified API parameter ordering
- [x] Research Rust crates for D-Bus communication (zbus)
  - Selected: `zbus` v4 for D-Bus bindings
- [x] Research Rust crates for metrics collection
  - No external crate needed - using `/proc` filesystem
- [x] Research Rust crates for configuration management
  - ��️ **Security Issue Found**: `serde_yaml` has vulnerability RUSTDEC-2025-0068
  - Changed decision: **TOML** (via `toml` crate) instead of YAML
  - Documented security rationale in multiple files
- [x] Research Rust crates for logging
  - Selected: `log` + `env_logger` or `tracing-subscriber`
- [x] Research Linux kernel documentation for /proc filesystem
  - Verified field orders and formats against official kernel docs
  - Added references to kernel documentation in metric files

### 1.3 Design Decisions
- [x] Choose Rust edition and version
  - Edition: 2021 (latest stable)
  - Version: Latest stable (1.75+)
- [x] Choose logging crate (tracing, log, etc.)
  - Decision: `tracing` + `tracing-subscriber` (better async support)
- [x] Choose configuration format (TOML, YAML, JSON)
  - ��️ **Updated Decision**: TOML instead of YAML
  - Rationale: Security (avoids RUSTSEC-2025-0068), pure Rust implementation
- [x] Choose configuration parsing crate
  - Decision: `toml` crate (0.8+)
- [x] Decide on metric aggregation strategy
  - Aggregation: Simple arithmetic mean across cores/interfaces
- [x] Decide on threshold configuration approach
  - Per-metric thresholds in TOML config
  - Support for individual metric enable/disable
- [x] Decide on idle timeout configuration
  - Dual timing: `duration_threshold` (time above threshold to inhibit)
  - `idle_duration` (time below threshold before releasing inhibition)
- [x] Design error handling strategy
  - Graceful degradation for missing metrics
  - Comprehensive error types using `thiserror`

## Phase 2: Project Setup

### 2.1 Rust Project Initialization
- [ ] Create Cargo.toml with dependencies
- [ ] Create src/main.rs structure
- [ ] Create src/config module
- [ ] Create src/metrics module
- [ ] Create src/inhibit module
- [ ] Create src/service module

### 2.2 Development Environment
- [ ] Create .gitignore
- [ ] Initialize git repository
- [ ] Create README.md
- [ ] Create CONTRIBUTING.md
- [ ] Create LICENSE

## Phase 3: Implementation

### 3.1 Configuration System
- [ ] Implement config parsing
- [ ] Implement default values
- [ ] Implement config validation
- [ ] Implement hot-reload capability (optional)

### 3.2 Metrics Collection
- [ ] Implement CPU metrics collector
- [ ] Implement GPU metrics collector (with fallbacks)
- [ ] Implement network I/O collector
- [ ] Implement disk I/O collector
- [ ] Implement metric smoothing/averaging

### 3.3 Sleep Inhibition
- [ ] Implement D-Bus connection
- [ ] Implement inhibit method
- [ ] Implement un-inhibit method
- [ ] Handle D-Bus errors gracefully
- [ ] Implement inhibition state tracking

### 3.4 Core Logic
- [ ] Implement threshold checking
- [ ] Implement idle timeout logic
- [ ] Implement main monitoring loop
- [ ] Implement graceful shutdown
- [ ] Implement signal handling (SIGTERM, SIGINT)

## Phase 4: Systemd Integration

### 4.1 Service File
- [ ] Create rouser.service file
- [ ] Configure proper systemd options
- [ ] Create logrotate config (optional)
- [ ] Create systemd timer (if needed)

### 4.2 Installation Scripts
- [ ] Create install script
- [ ] Create uninstall script
- [ ] Create configuration template

## Phase 5: Testing

### 5.1 Unit Tests
- [ ] Test config parsing
- [ ] Test metric collectors (mocked)
- [ ] Test threshold logic
- [ ] Test idle timeout logic

### 5.2 Integration Tests
- [ ] Test actual metric collection
- [ ] Test D-Bus inhibition (on test system)
- [ ] Test full monitoring loop

### 5.3 Manual Testing
- [ ] Test on target system
- [ ] Test various load scenarios
- [ ] Test sleep inhibition behavior
- [ ] Test wake from sleep behavior

## Phase 6: Documentation & Release

### 6.1 User Documentation
- [x] Complete README.md
- [x] Create installation guide
- [x] Create configuration guide
- [x] Create troubleshooting guide
  - All documentation merged into `docs/` directory

### 6.2 Code Documentation
- [ ] Add inline documentation
- [ ] Generate rustdoc
- [ ] Document public API

### 6.3 Release Preparation
- [ ] Create release notes
- [ ] Tag first release
- [ ] Create binary release (optional)

---

## Current Status

**Completed**: Phase 1 (Planning & Research) - 100% ��
- All documentation created and organized in `docs/` folder
- All research completed and documented
- All design decisions made and documented
- Security issues identified and addressed
- Performance characteristics documented
- 10 documentation files created/updated

**In Progress**: Phase 2 (Project Setup)
- Next tasks: Initialize git repo, create Cargo.toml, set up project structure

**Next Steps**:
1. Initialize git repository
2. Create Cargo.toml with dependencies (TOML instead of YAML)
3. Create project directory structure
4. Write README.md
5. Begin Phase 3 implementation

---

## Research Summary

### Security Issues Identified
1. **RUSTSEC-2025-0068**: `serde_yaml` crate vulnerability
   - Impact: Supply chain risk, unmaintained dependencies
   - Resolution: Switched to TOML format with `toml` crate
   - Documentation: Updated in PROJECT.md, configuration/reference.md, quickstart.md

2. **D-Bus API Parameter Order**
   - Impact: Inconsistent examples in documentation
   - Resolution: Standardized on zbus API parameter order
   - Documentation: Fixed in docs/d-bus/inhibition.md

3. **Virtual Device Detection**
   - Impact: LVM devices incorrectly excluded
   - Resolution: Changed to include dm- (LVM), exclude only truly virtual
   - Documentation: Updated in docs/metrics/disk.md

### Documentation Updates Completed
- Updated all files to use TOML format
- Added security documentation (security.md)
- Added performance documentation (performance.md)
- Fixed D-Bus API examples
- Corrected metric calculation descriptions
- Added kernel documentation references
- Standardized parameter order across all examples

### Files Updated in This Session
1. `PROJECT.md` - Updated configuration format to TOML
2. `docs/configuration/reference.md` - Complete TOML configuration guide
3. `docs/d-bus/inhibition.md` - Fixed API parameter order
4. `docs/metrics/cpu.md` - Added kernel references, jiffies overflow handling
5. `docs/metrics/gpu.md` - Updated sysfs paths, vendor-specific docs
6. `docs/metrics/network.md` - Loopback exclusion rationale
7. `docs/metrics/disk.md` - LVM device inclusion rationale
8. `docs/quickstart.md` - TOML configuration examples
9. `docs/systemd/service.md` - Security hardening guide
10. `docs/performance.md` - Benchmarks and optimization guidelines
11. `docs/architecture/overview.md` - Architecture with TOML config
12. `docs/security.md` - NEW: Security best practices

### Files Created in This Session
- `docs/security.md` - Security best practices
- `docs/performance.md` - Performance characteristics and benchmarks

### Archive Created
- `rouser-planning.zip` - Contains all planning documentation (300+ pages)

---

## Notes

### Configuration Format Decision
**Before**: YAML with `serde_yaml`
**After**: TOML with `toml` crate

**Reasoning**:
- Security vulnerability in `serde_yaml` (RUSTSEC-2025-0068)
- Pure Rust implementation with no C dependencies
- Simpler, more readable syntax
- Well-maintained in Rust ecosystem
- Native support via `toml` crate

All documentation has been updated to reflect this change.

### Next Major Milestone
**Phase 3 Start**: Begin implementation after project setup is complete.

Estimated completion: 2-4 weeks depending on complexity.
