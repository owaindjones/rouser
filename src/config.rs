use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::io::{self, Write};
use std::path::Path;
use std::time::Duration;
use tracing::{info, warn};

const DEFAULT_CONFIG_TOML: &str = include_str!("../config/rouser.toml");

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    #[serde(with = "humantime_serde")]
    pub update_interval: Duration,
    pub log_level: String,
    pub metrics: Metrics,
    pub timing: TimingConfig,
    pub inhibitor: InhibitionConfig,
    pub prediction: PredictionConfig,
}

/// CPU metrics configuration with per-core and total thresholds.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CpuConfig {
    /// Per-core CPU usage threshold (percentage). Exceeding this triggers inhibition.
    #[serde(default)]
    pub per_core_threshold: f64,
    /// Total averaged CPU usage threshold (percentage). Exceeding this triggers inhibition.
    #[serde(default)]
    pub total_threshold: f64,
    /// EMA smoothing factor for CPU readings.
    #[serde(default)]
    pub ema_alpha: f64,
}

impl Default for CpuConfig {
    fn default() -> Self {
        Self {
            per_core_threshold: 80.0,
            total_threshold: 25.0,
            ema_alpha: 0.7,
        }
    }
}

/// GPU metrics configuration with per-GPU and aggregate thresholds.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GpuConfig {
    /// GPU usage threshold per individual card (percentage). Any single GPU above this triggers inhibition.
    #[serde(default)]
    pub per_gpu_threshold: f64,
    /// System-wide aggregate GPU threshold (average across all GPUs, percentage). The average GPU load exceeding this triggers inhibition.
    #[serde(default)]
    pub total_threshold: f64,
    /// EMA smoothing factor for GPU readings.
    #[serde(default)]
    pub ema_alpha: f64,
}

impl Default for GpuConfig {
    fn default() -> Self {
        Self {
            per_gpu_threshold: 25.0,
            total_threshold: 40.0,
            ema_alpha: 0.7,
        }
    }
}

/// Network metrics configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NetworkConfig {
    /// Network throughput threshold (Mbps). Exceeding this triggers inhibition.
    #[serde(default)]
    pub threshold: f64,
    /// EMA smoothing factor for network I/O readings.
    #[serde(default)]
    pub ema_alpha: f64,
    #[serde(default)]
    pub exclude_interfaces: Vec<String>,
    #[serde(default)]
    pub include_interfaces: Vec<String>,
}

impl Default for NetworkConfig {
    fn default() -> Self {
        Self {
            threshold: 10.0,
            ema_alpha: 0.5,
            exclude_interfaces: Vec::new(),
            include_interfaces: Vec::new(),
        }
    }
}

/// Disk metrics configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiskConfig {
    /// Disk I/O threshold (MB/s). Exceeding this triggers inhibition.
    #[serde(default)]
    pub threshold: f64,
    /// EMA smoothing factor for disk activity readings.
    #[serde(default)]
    pub ema_alpha: f64,
    #[serde(default)]
    pub exclude_device_prefixes: Vec<String>,
}

impl Default for DiskConfig {
    fn default() -> Self {
        Self {
            threshold: 10.0,
            ema_alpha: 0.5,
            exclude_device_prefixes: Vec::new(),
        }
    }
}

/// Aggregated metrics configuration.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Metrics {
    #[serde(default)]
    pub cpu: CpuConfig,
    #[serde(default)]
    pub gpu: GpuConfig,
    #[serde(default)]
    pub network: NetworkConfig,
    #[serde(default)]
    pub disk: DiskConfig,
}

/// Timing configuration for threshold evaluation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TimingConfig {
    /// Minimum continuous time metrics must exceed threshold before inhibiting sleep.
    #[serde(with = "humantime_serde")]
    pub duration_threshold: Duration,
    /// Time after releasing inhibition during which the daemon won't re-inhibit even if thresholds are exceeded again.
    #[serde(default, with = "humantime_serde")]
    pub cooldown_duration: Duration,
}

impl Default for TimingConfig {
    fn default() -> Self {
        Self {
            duration_threshold: Duration::from_secs(30),
            cooldown_duration: Duration::from_secs(60),
        }
    }
}

/// Inhibition configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InhibitionConfig {
    /// Operations to inhibit (colon-separated). See D-Bus login1 API for options.
    #[serde(default)]
    pub what: String,
    /// Mode of inhibition: block, delay, or block-weak.
    #[serde(default)]
    pub mode: String,
}

impl Default for InhibitionConfig {
    fn default() -> Self {
        Self {
            what: "shutdown:idle".to_string(),
            mode: "block".to_string(),
        }
    }
}

/// Predictive cooldown configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PredictionConfig {
    /// Seconds between averaged snapshots written to history log; must be >= root update_interval.
    #[serde(default, with = "humantime_serde")]
    pub update_interval: Duration,
    /// Keep this much historical data; older entries are pruned periodically.
    #[serde(default = "default_history_length", with = "humantime_serde")]
    pub history_length: Duration,
    /// Maximum additional time for predictive cooldown extension.
    #[serde(default, with = "humantime_serde")]
    pub max_extension_time: Duration,
}

fn default_history_length() -> Duration {
    // 30 days — matches config/rouser.toml. Kept because humantime_serde
    // requires a Duration-typed function (can't use bare "default").
    Duration::from_secs(30 * 24 * 60 * 60)
}

impl Default for PredictionConfig {
    fn default() -> Self {
        Self {
            update_interval: Duration::from_secs(30),
            history_length: Duration::from_secs(30 * 24 * 60 * 60),
            max_extension_time: Duration::from_secs(3600),
        }
    }
}

#[derive(Clone)]
pub struct ConfigLoader {
    config_path: std::path::PathBuf,
}

impl ConfigLoader {
    pub fn new(config_path: &Path) -> Self {
        Self {
            config_path: config_path.to_path_buf(),
        }
    }

    #[allow(dead_code)]
    pub fn validate(&self) -> Result<()> {
        if !self.config_path.exists() {
            anyhow::bail!(
                "Configuration file does not exist: {}",
                self.config_path.display()
            );
        }

        let content = fs::read_to_string(&self.config_path).with_context(|| {
            format!("Failed to read config file: {}", self.config_path.display())
        })?;

        let _config: Config =
            toml::from_str(&content).with_context(|| "Failed to parse TOML configuration")?;

        info!("Configuration validation passed");
        Ok(())
    }

    pub fn load(&self) -> Result<Config> {
        if !self.config_path.exists() {
            warn!(
                "Configuration file does not exist, using defaults: {}",
                self.config_path.display()
            );
            return ConfigLoader::load_defaults();
        }

        let content = fs::read_to_string(&self.config_path).with_context(|| {
            format!("Failed to read config file: {}", self.config_path.display())
        })?;

        let config: Config =
            toml::from_str(&content).with_context(|| "Failed to parse TOML configuration")?;

        Ok(config)
    }

    pub fn load_defaults() -> Result<Config> {
        let config: Config = toml::from_str(DEFAULT_CONFIG_TOML)
            .context("Failed to parse embedded default configuration")?;
        Ok(config)
    }

    #[allow(dead_code)]
    fn load_fallback(&self) -> Result<Config> {
        ConfigLoader::load_defaults()
    }

    pub(super) fn deep_merge(base: &mut toml::Value, override_val: &toml::Value) {
        if let (Some(b), Some(o)) = (base.as_table_mut(), override_val.as_table()) {
            for (k, v) in o.iter() {
                if let Some(existing) = b.get_mut(k) {
                    Self::deep_merge(existing, v);
                } else {
                    b.insert(k.clone(), v.clone());
                }
            }
        } else {
            *base = override_val.clone();
        }
    }

    fn config_to_toml_value(config: &Config) -> Result<toml::Value> {
        let serialized = toml::to_string(config).context("Failed to serialize config to TOML")?;
        toml::from_str(&serialized).context("Failed to parse serialized config as Value")
    }

    pub fn print_config_toml(config: &Config, out: &mut dyn Write) -> io::Result<()> {
        let serialized = toml::to_string_pretty(config)
            .map_err(|e| io::Error::other(format!("Failed to serialize config: {}", e)))?;

        writeln!(out, "{}", serialized)?;
        Ok(())
    }

    fn install_default_if_missing(path: &std::path::Path) -> Result<()> {
        use std::fs;

        if path.exists() {
            return Ok(());
        }

        let parent = path
            .parent()
            .context("Config path has no parent directory")?;
        fs::create_dir_all(parent)
            .with_context(|| format!("Failed to create config directory: {}", parent.display()))?;

        fs::write(path, DEFAULT_CONFIG_TOML)
            .with_context(|| format!("Failed to write default config to {}", path.display()))?;

        info!("Installed default configuration to {}", path.display());
        Ok(())
    }

    pub fn load_merged() -> Result<(Config, Vec<String>)> {
        use std::path::PathBuf;

        let mut searched = Vec::new();

        // Start with embedded defaults as the base.
        let mut merged_value: toml::Value =
            ConfigLoader::config_to_toml_value(&ConfigLoader::load_defaults()?)?;
        searched.push("embedded defaults".to_string());

        // Check if running as root (euid == 0).
        let is_root = unsafe { libc::geteuid() == 0 };

        // System-wide config path: /etc/rouser/config.toml
        let etc_path = std::path::PathBuf::from("/etc/rouser/config.toml");
        searched.push(etc_path.display().to_string());

        if etc_path.exists() {
            let content = fs::read_to_string(&etc_path)
                .with_context(|| format!("Failed to read {}", etc_path.display()))?;
            let user_config: toml::Value =
                toml::from_str(&content).context("Failed to parse /etc/rouser/config.toml")?;
            Self::deep_merge(&mut merged_value, &user_config);
        } else if is_root {
            // Root user + missing system config → auto-install.
            if let Err(e) = ConfigLoader::install_default_if_missing(&etc_path) {
                warn!(
                    "Could not install default config to {}: {}",
                    etc_path.display(),
                    e
                );
            }
        }

        // User config path: $XDG_CONFIG_HOME/rouser/config.toml or ~/.config/rouser/config.toml
        let xdg_config_home = std::env::var("XDG_CONFIG_HOME")
            .ok()
            .filter(|s| !s.is_empty())
            .map(PathBuf::from)
            .or_else(|| {
                std::env::var("HOME")
                    .ok()
                    .map(|h| PathBuf::from(h).join(".config"))
            });

        // Resolve the actual user config path for file operations.
        let user_config_path: PathBuf = match xdg_config_home {
            Some(base) => base.join("rouser").join("config.toml"),
            None => std::env::var("HOME")
                .ok()
                .map(|h| PathBuf::from(h).join(".config/rouser/config.toml"))
                .unwrap_or_else(|| PathBuf::from("~/.config/rouser/config.toml")),
        };

        searched.push(user_config_path.display().to_string());

        if user_config_path.exists() {
            let content = fs::read_to_string(&user_config_path)
                .with_context(|| format!("Failed to read {}", user_config_path.display()))?;
            let user_config: toml::Value =
                toml::from_str(&content).context("Failed to parse user config file")?;
            Self::deep_merge(&mut merged_value, &user_config);
        } else if !is_root {
            // Non-root user + missing user config → auto-install.
            if let Err(e) = ConfigLoader::install_default_if_missing(&user_config_path) {
                warn!(
                    "Could not install default config to {}: {}",
                    user_config_path.display(),
                    e
                );
            }
        }

        // Deserialize the merged TOML value into our Config struct.
        let config_str = toml::to_string(&merged_value)
            .context("Failed to serialize merged config for final deserialization")?;
        let config: Config =
            toml::from_str(&config_str).context("Failed to deserialize merged configuration")?;

        Ok((config, searched))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn test_default_config_values() {
        let dir = TempDir::new().unwrap();
        let config_path = dir.path().join("config.toml");

        fs::write(&config_path, "").unwrap();

        let config_path = config_path.to_path_buf();
        assert!(config_path.exists());
    }

    #[test]
    fn test_metrics_defaults() {
        let metrics = Metrics::default();

        assert_eq!(metrics.cpu.per_core_threshold, 80.0);
        assert_eq!(metrics.cpu.total_threshold, 25.0);
        assert_eq!(metrics.gpu.per_gpu_threshold, 25.0);
        assert_eq!(metrics.gpu.total_threshold, 40.0);
        assert_eq!(metrics.network.threshold, 10.0);
        assert_eq!(metrics.disk.threshold, 10.0);
        assert_eq!(metrics.cpu.ema_alpha, 0.7);
        assert_eq!(metrics.gpu.ema_alpha, 0.7);
        assert_eq!(metrics.network.ema_alpha, 0.5);
        assert_eq!(metrics.disk.ema_alpha, 0.5);
    }

    #[test]
    fn test_timing_defaults() {
        let timing = TimingConfig::default();

        assert_eq!(timing.duration_threshold.as_secs(), 30);
        assert_eq!(timing.cooldown_duration.as_secs(), 60);
    }
}
