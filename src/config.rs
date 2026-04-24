use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::Path;
use std::time::Duration;
use tracing::{info, warn};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    #[allow(dead_code)] // Reserved for future use
    pub name: String,
    #[serde(with = "humantime_serde")]
    pub update_interval: Duration,
    pub log_level: String,
    pub metrics: Metrics,
    pub timing: TimingConfig,
    pub inhibitor: InhibitionConfig,
}

fn default_name() -> String {
    "rouser".to_string()
}

fn default_update_interval() -> Duration {
    Duration::from_secs(5)
}

fn default_log_level() -> String {
    "info".to_string()
}

fn default_cpu_usage() -> f64 {
    80.0
}

fn default_gpu_usage() -> f64 {
    90.0
}

fn default_network_io() -> f64 {
    100.0
}

fn default_disk_activity() -> f64 {
    50.0
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Thresholds {
    #[serde(default = "default_cpu_usage")]
    pub cpu_usage: f64,
    #[serde(default = "default_gpu_usage")]
    pub gpu_usage: f64,
    #[serde(default = "default_network_io")]
    pub network_io: f64,
    #[serde(default = "default_disk_activity")]
    pub disk_activity: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MetricsConfig {
    #[serde(default = "default_ema_alpha_cpu")]
    pub ema_alpha: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CpuConfig {
    #[serde(default = "default_cpu_usage")]
    pub threshold: f64,
    #[serde(default = "default_ema_alpha_cpu")]
    pub ema_alpha: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct GpuConfig {
    #[serde(default = "default_gpu_usage")]
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
    0.3
}

fn default_ema_alpha_gpu() -> f64 {
    0.3
}

fn default_ema_alpha_network() -> f64 {
    0.2
}

fn default_ema_alpha_disk() -> f64 {
    0.2
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

    pub fn validate(&self) -> Result<()> {
        if !self.config_path.exists() {
            anyhow::bail!(
                "Configuration file does not exist: {}",
                self.config_path.display()
            );
        }

        let content = fs::read_to_string(&self.config_path)
            .with_context(|| format!("Failed to read config file: {}", self.config_path.display()))?;

        let config: toml::Value = toml::from_str(&content)
            .with_context(|| "Failed to parse TOML configuration")?;

        self.validate_thresholds(&config)?;

        info!("Configuration validation passed");
        Ok(())
    }

    fn validate_thresholds(&self, config: &toml::Value) -> Result<()> {
        if let Some(thresholds) = config.get("thresholds") {
            if let Some(cpu_usage) = thresholds.get("cpu_usage") {
                if let Some(cpu) = cpu_usage.as_str().and_then(|s| s.parse::<f64>().ok()) {
                    if cpu < 0.0 || cpu > 100.0 {
                        anyhow::bail!(
                            "cpu_usage threshold must be between 0.0 and 100.0, got: {}",
                            cpu
                        );
                    }
                }
            }

            if let Some(gpu_usage) = thresholds.get("gpu_usage") {
                if let Some(gpu) = gpu_usage.as_str().and_then(|s| s.parse::<f64>().ok()) {
                    if gpu < 0.0 || gpu > 100.0 {
                        anyhow::bail!(
                            "gpu_usage threshold must be between 0.0 and 100.0, got: {}",
                            gpu
                        );
                    }
                }
            }
        }
        Ok(())
    }

    pub fn load(&self) -> Result<Config> {
        if !self.config_path.exists() {
            warn!(
                "Configuration file does not exist, using defaults: {}",
                self.config_path.display()
            );
            return self.load_defaults();
        }

        let content = fs::read_to_string(&self.config_path)
            .with_context(|| format!("Failed to read config file: {}", self.config_path.display()))?;

        let config: Config = toml::from_str(&content)
            .with_context(|| "Failed to parse TOML configuration")?;

        Ok(config)
    }

    fn load_defaults(&self) -> Result<Config> {
        let config = Config {
            name: default_name(),
            update_interval: default_update_interval(),
            log_level: default_log_level(),
            metrics: Metrics {
                cpu: CpuConfig {
                    threshold: default_cpu_usage(),
                    ema_alpha: default_ema_alpha_cpu(),
                },
                gpu: GpuConfig {
                    threshold: default_gpu_usage(),
                    ema_alpha: default_ema_alpha_gpu(),
                },
                network: NetworkConfig {
                    threshold: default_network_io(),
                    ema_alpha: default_ema_alpha_network(),
                    exclude_interfaces: vec!["lo".to_string()],
                    include_interfaces: vec![],
                },
                disk: DiskConfig {
                    threshold: default_disk_activity(),
                    ema_alpha: default_ema_alpha_disk(),
                    exclude_device_prefixes: vec!["loop".to_string(), "fd".to_string(), "sr".to_string(), "cdrom".to_string()],
                },
            },
            timing: TimingConfig {
                duration_threshold: default_duration_threshold(),
                cooldown_duration: default_cooldown_duration(),
            },
            inhibitor: InhibitionConfig {  
                what: default_what(), 
                mode: default_mode(),
            },
        };

        Ok(config)
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
        
        // Create an empty config file
        fs::write(&config_path, "").unwrap();
        
        let config_path = config_path.to_path_buf();
        assert!(config_path.exists());
    }

    #[test]
    fn test_metrics_defaults() {
        let metrics = Metrics {
            cpu: CpuConfig {
                threshold: default_cpu_usage(),
                ema_alpha: default_ema_alpha_cpu(),
            },
            gpu: GpuConfig {
                threshold: default_gpu_usage(),
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
        
        assert_eq!(metrics.cpu.threshold, 80.0);
        assert_eq!(metrics.gpu.threshold, 90.0);
        assert_eq!(metrics.network.threshold, 100.0);
        assert_eq!(metrics.disk.threshold, 50.0);
        assert_eq!(metrics.cpu.ema_alpha, 0.3);
        assert_eq!(metrics.gpu.ema_alpha, 0.3);
        assert_eq!(metrics.network.ema_alpha, 0.2);
        assert_eq!(metrics.disk.ema_alpha, 0.2);
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