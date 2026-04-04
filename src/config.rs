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
    pub thresholds: Thresholds,
    pub timing: TimingConfig,
    pub inhibition: InhibitionConfig,
    pub network: NetworkConfig,
    pub disk: DiskConfig,
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

fn default_duration_threshold() -> Duration {
    Duration::from_secs(30)
}

fn default_idle_duration() -> Duration {
    Duration::from_secs(60)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TimingConfig {
    #[serde(default = "default_duration_threshold", with = "humantime_serde")]
    pub duration_threshold: Duration,
    #[serde(default = "default_idle_duration", with = "humantime_serde")]
    pub idle_duration: Duration,
}

fn default_what() -> String {
    "sleep".to_string()
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



#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NetworkConfig {
    #[serde(default)]
    pub exclude_interfaces: Vec<String>,
    #[serde(default)]
    pub include_interfaces: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiskConfig {
    #[serde(default)]
    pub exclude_device_prefixes: Vec<String>,
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
            thresholds: Thresholds {
                cpu_usage: default_cpu_usage(),
                gpu_usage: default_gpu_usage(),
                network_io: default_network_io(),
                disk_activity: default_disk_activity(),
            },
            timing: TimingConfig {
                duration_threshold: default_duration_threshold(),
                idle_duration: default_idle_duration(),
            },
           inhibition: InhibitionConfig {  
                what: default_what(), 
                mode: default_mode(),
            },

            network: NetworkConfig {
                exclude_interfaces: vec!["lo".to_string()],
                include_interfaces: vec![],
            },
            disk: DiskConfig {
                exclude_device_prefixes: vec!["loop".to_string(), "fd".to_string(), "sr".to_string(), "cdrom".to_string()],
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
    fn test_threshold_defaults() {
        let thresholds = Thresholds {
            cpu_usage: default_cpu_usage(),
            gpu_usage: default_gpu_usage(),
            network_io: default_network_io(),
            disk_activity: default_disk_activity(),
        };
        
        assert_eq!(thresholds.cpu_usage, 80.0);
        assert_eq!(thresholds.gpu_usage, 90.0);
        assert_eq!(thresholds.network_io, 100.0);
        assert_eq!(thresholds.disk_activity, 50.0);
    }

    #[test]
    fn test_timing_defaults() {
        let timing = TimingConfig {
            duration_threshold: default_duration_threshold(),
            idle_duration: default_idle_duration(),
        };
        
        assert_eq!(timing.duration_threshold.as_secs(), 30);
        assert_eq!(timing.idle_duration.as_secs(), 60);
    }
}