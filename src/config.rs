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
}

fn default_gpu_threshold() -> f64 {
    15.0
}

fn default_network_io() -> f64 {
    10.0
}

fn default_disk_activity() -> f64 {
    10.0
}

#[allow(dead_code)]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Thresholds {
    #[serde(default = "default_cpu_usage_threshold")]
    pub cpu_usage: f64,
    #[serde(default = "default_gpu_threshold")]
    pub gpu_usage: f64,
    #[serde(default = "default_network_io")]
    pub network_io: f64,
    #[serde(default = "default_disk_activity")]
    pub disk_activity: f64,
}

fn default_cpu_usage_threshold() -> f64 {
    80.0
}

#[allow(dead_code)]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MetricsConfig {
    #[serde(default = "default_ema_alpha_cpu")]
    pub ema_alpha: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CpuConfig {
    #[serde(default = "default_per_core_threshold")]
    pub per_core_threshold: f64,
    #[serde(default = "default_total_threshold")]
    pub total_threshold: f64,
    #[serde(default = "default_ema_alpha_cpu")]
    pub ema_alpha: f64,
}

fn default_per_core_threshold() -> f64 {
    80.0
}

fn default_total_threshold() -> f64 {
    25.0
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct GpuConfig {
    #[serde(default = "default_gpu_threshold")]
    pub threshold: f64,
    #[serde(default = "default_ema_alpha_gpu")]
    pub ema_alpha: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct NetworkConfig {
    #[serde(default = "default_network_io")]
    pub threshold: f64,
    #[serde(default = "default_ema_alpha_network")]
    pub ema_alpha: f64,
    #[serde(default)]
    pub exclude_interfaces: Vec<String>,
    #[serde(default)]
    pub include_interfaces: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct DiskConfig {
    #[serde(default = "default_disk_activity")]
    pub threshold: f64,
    #[serde(default = "default_ema_alpha_disk")]
    pub ema_alpha: f64,
    #[serde(default)]
    pub exclude_device_prefixes: Vec<String>,
}

fn default_cpu() -> CpuConfig {
    Default::default()
}

fn default_gpu() -> GpuConfig {
    Default::default()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Metrics {
    #[serde(default = "default_cpu")]
    pub cpu: CpuConfig,
    #[serde(default = "default_gpu")]
    pub gpu: GpuConfig,
    #[serde(default)]
    pub network: NetworkConfig,
    #[serde(default)]
    pub disk: DiskConfig,
}

fn default_duration_threshold() -> Duration {
    Duration::from_secs(30)
}

fn default_cooldown_duration() -> Duration {
    Duration::from_secs(60)
}

fn default_ema_alpha_cpu() -> f64 {
    0.7
}

fn default_ema_alpha_gpu() -> f64 {
    0.7
}

fn default_ema_alpha_network() -> f64 {
    0.5
}

fn default_ema_alpha_disk() -> f64 {
    0.5
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TimingConfig {
    #[serde(default = "default_duration_threshold", with = "humantime_serde")]
    pub duration_threshold: Duration,
    #[serde(default = "default_cooldown_duration", with = "humantime_serde")]
    pub cooldown_duration: Duration,
}

fn default_what() -> String {
    "shutdown:idle".to_string()
}

fn default_mode() -> String {
    "block".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InhibitionConfig {
    #[serde(default = "default_what")]
    pub what: String,
    #[serde(default = "default_mode")]
    pub mode: String,
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
        let metrics = Metrics {
            cpu: CpuConfig {
                per_core_threshold: default_per_core_threshold(),
                total_threshold: default_total_threshold(),
                ema_alpha: default_ema_alpha_cpu(),
            },
            gpu: GpuConfig {
                threshold: default_gpu_threshold(),
                ema_alpha: default_ema_alpha_gpu(),
            },
            network: NetworkConfig {
                threshold: default_network_io(),
                ema_alpha: default_ema_alpha_network(),
                exclude_interfaces: vec![],
                include_interfaces: vec![],
            },
            disk: DiskConfig {
                threshold: default_disk_activity(),
                ema_alpha: default_ema_alpha_disk(),
                exclude_device_prefixes: vec![],
            },
        };

        assert_eq!(metrics.cpu.per_core_threshold, 80.0);
        assert_eq!(metrics.cpu.total_threshold, 25.0);
        assert_eq!(metrics.gpu.threshold, 15.0);
        assert_eq!(metrics.network.threshold, 10.0);
        assert_eq!(metrics.disk.threshold, 10.0);
        assert_eq!(metrics.cpu.ema_alpha, 0.7);
        assert_eq!(metrics.gpu.ema_alpha, 0.7);
        assert_eq!(metrics.network.ema_alpha, 0.5);
        assert_eq!(metrics.disk.ema_alpha, 0.5);
    }

    #[test]
    fn test_timing_defaults() {
        let timing = TimingConfig {
            duration_threshold: default_duration_threshold(),
            cooldown_duration: default_cooldown_duration(),
        };

        assert_eq!(timing.duration_threshold.as_secs(), 30);
        assert_eq!(timing.cooldown_duration.as_secs(), 60);
    }
}
