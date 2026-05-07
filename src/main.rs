mod config;
mod inhibit;
mod metrics;
mod prediction;
mod service;

use anyhow::Result;
use clap::Parser;
use std::path::PathBuf;
use std::process::ExitCode;
use tracing::{error, info, warn};

// Import the prelude for .with() method on subscribers.
use tracing_subscriber::prelude::*;

use config::ConfigLoader;
use service::DataService;

/// rouser - A Linux daemon that monitors system metrics and inhibits sleep when activity thresholds are exceeded
#[derive(Parser)]
#[command(name = "rouser")]
#[command(author, version, about, long_about = None)]
struct Args {
    /// Path to a single configuration file (overrides default search/merge behavior)
    #[arg(long, short)]
    config: Option<PathBuf>,

    /// Validate configuration and exit
    #[arg(long)]
    validate_config: bool,

    /// Dry run mode (don't actually inhibit sleep)
    #[arg(long)]
    dry_run: bool,

    /// Print the final merged configuration as TOML and exit
    #[arg(long)]
    print_config: bool,

    /// Log level filter; overrides config.log_level and RUST_LOG env var.
    #[arg(long, short = 'l')]
    log_level: Option<String>,
}

/// Resolve the effective tracing log level after config is loaded.
/// Priority chain: CLI > RUST_LOG > config.log_level > 'info'.
fn resolve_tracing_log_level(args: &Args, config: &config::Config) -> String {
    if let Some(ref cli_val) = args.log_level {
        return cli_val.to_string();
    }

    // Environment variable is the next source — transient overrides persistent defaults.
    if let Ok(val) = std::env::var("RUST_LOG") {
        if !val.is_empty() {
            return val;
        }
    }

    // Config file log_level is the fallback for a persistent default.
    if !config.log_level.is_empty() {
        return config.log_level.clone();
    }

    "info".to_string()
}

fn load_single_config(path: &std::path::Path) -> Result<config::Config> {
    let loader = ConfigLoader::new(path);
    loader
        .load()
        .map_err(|e| anyhow::anyhow!("Failed to load config from {}: {}", path.display(), e))
}

#[tokio::main]
async fn main() -> ExitCode {
    let args = Args::parse();

    // Phase 1 — init tracing at DEBUG so auto-install logs during config load are captured.
    // RUST_LOG takes priority, then CLI flag, then fallback to debug.
    let startup_level = std::env::var("RUST_LOG")
        .ok()
        .filter(|s| !s.is_empty())
        .or_else(|| args.log_level.clone())
        .unwrap_or_else(|| "debug".to_string());

    // Build reloadable filter and install subscriber inline to avoid complex type annotations.
    let env_filter = match tracing_subscriber::EnvFilter::try_new(&startup_level) {
        Ok(f) => f,
        Err(e) => {
            eprintln!(
                "Invalid log level '{}': {}. Using 'info'.",
                startup_level, e
            );
            tracing_subscriber::EnvFilter::new("info")
        }
    };

    let (env_filter, reload_handle) = tracing_subscriber::reload::Layer::new(env_filter);

    let tracing_installed = match tracing_subscriber::registry()
        .with(
            tracing_subscriber::fmt::layer()
                .with_target(true)
                .with_level(true)
                .with_thread_ids(false)
                .with_thread_names(false),
        )
        .with(env_filter)
        .try_init()
    {
        Ok(_) => true,
        Err(e) if e.to_string().contains("global default") => false,
        Err(e) => {
            eprintln!("Failed to install tracing subscriber: {}", e);
            false
        }
    };

    // --print-config: serialize config as TOML and exit.
    if args.print_config {
        let config = if let Some(ref path) = args.config {
            match load_single_config(path) {
                Ok(cfg) => cfg,
                Err(e) => {
                    error!(
                        "Failed to load configuration from {}: {}",
                        path.display(),
                        e
                    );
                    return ExitCode::FAILURE;
                }
            }
        } else {
            let (cfg, _) = ConfigLoader::load_merged().unwrap_or_else(|e| {
                error!("Failed to load and merge configuration: {}", e);
                std::process::exit(1);
            });
            cfg
        };
        if let Err(e) = ConfigLoader::print_config_toml(&config, &mut std::io::stdout()) {
            eprintln!("Error: {}", e);
            return ExitCode::FAILURE;
        }
        return ExitCode::SUCCESS;
    }

    // Load configuration.
    let (config, _searched): (config::Config, Vec<String>) = if let Some(ref path) = args.config {
        match load_single_config(path) {
            Ok(cfg) => (cfg, vec![]),
            Err(e) => {
                eprintln!(
                    "Failed to load configuration from {}: {}",
                    path.display(),
                    e
                );
                return ExitCode::FAILURE;
            }
        }
    } else {
        ConfigLoader::load_merged().unwrap_or_else(|e| {
            eprintln!("Failed to load configuration: {}", e);
            std::process::exit(1);
        })
    };

    // Phase 2 — swap the log level filter to match config.log_level if our subscriber is active.
    let final_level = resolve_tracing_log_level(&args, &config);
    if tracing_installed {
        match tracing_subscriber::EnvFilter::try_new(&final_level) {
            Ok(new_filter) => {
                reload_handle
                    .modify(|filter| *filter = new_filter)
                    .unwrap_or_else(|e| {
                        warn!("Failed to modify tracing filter: {}", e);
                    });
                info!("Log level reconfigured to: {}", final_level);
            }
            Err(e) => {
                eprintln!("Invalid log level '{}': {}. Using 'info'.", final_level, e);
                reload_handle
                    .modify(|filter| *filter = tracing_subscriber::EnvFilter::new("info"))
                    .unwrap_or_else(|e| {
                        warn!("Failed to modify tracing filter: {}", e);
                    });
            }
        }
    } else {
        warn!(
            "Tracing was pre-initialized externally (likely by RUST_LOG). config.log_level will not take effect."
        );
    }

    let should_validate = args.validate_config;

    info!("rouser starting...");

    // Validate configuration if requested.
    if should_validate {
        if let Some(ref path) = args.config {
            match load_single_config(path) {
                Ok(_) => {
                    info!("Configuration validation passed");
                    return ExitCode::SUCCESS;
                }
                Err(e) => {
                    error!("Configuration validation failed: {}", e);
                    return ExitCode::FAILURE;
                }
            }
        } else {
            // Already loaded and parsed successfully — nothing to validate further.
            info!("Configuration validation passed");
            return ExitCode::SUCCESS;
        }
    }

    if args.dry_run {
        match run_dry_run(&config).await {
            Ok(_) => {
                info!("Dry run completed successfully");
                return ExitCode::SUCCESS;
            }
            Err(e) => {
                error!("Dry run failed: {}", e);
                return ExitCode::FAILURE;
            }
        }
    } else {
        match run_daemon(&config).await {
            Ok(_) => {
                info!("rouser stopped normally");
                ExitCode::SUCCESS
            }
            Err(e) => {
                error!("rouser encountered an error: {}", e);
                ExitCode::FAILURE
            }
        }
    }
}

async fn run_dry_run(config: &config::Config) -> Result<()> {
    info!("Running in dry-run mode indefinitely");
    info!("Configuration:");
    info!(
        "  - CPU per-core threshold: {}%, total threshold: {}%, EMA alpha: {:.2}",
        config.metrics.cpu.per_core_threshold,
        config.metrics.cpu.total_threshold,
        config.metrics.cpu.ema_alpha
    );
    info!(
        "  - GPU per-GPU threshold: {}%, total threshold: {}%, EMA alpha: {:.2}",
        config.metrics.gpu.per_gpu_threshold,
        config.metrics.gpu.total_threshold,
        config.metrics.gpu.ema_alpha
    );
    info!(
        "  - Network threshold: {} Mbps, EMA alpha: {:.2}",
        config.metrics.network.threshold, config.metrics.network.ema_alpha
    );
    info!(
        "  - Disk threshold: {} MB/s, EMA alpha: {:.2}",
        config.metrics.disk.threshold, config.metrics.disk.ema_alpha
    );
    info!(
        "  - Duration threshold: {:?}",
        config.timing.duration_threshold
    );
    info!(
        "  - Cooldown duration: {:?}",
        config.timing.cooldown_duration
    );

    let mut service = DataService::new(config, true).await?;

    loop {
        service.tick(config).await?;
        tokio::time::sleep(config.update_interval).await;
    }
}

async fn run_daemon(config: &config::Config) -> Result<()> {
    // Create service
    let mut service = DataService::new(config, false).await?;

    // Handle shutdown signals
    let shutdown_signal = async {
        tokio::signal::ctrl_c().await.ok();
        info!("Received shutdown signal");
    };

    tokio::select! {
        result = async {
          loop {
                if let Err(e) = service.tick(config).await {
                    warn!("Tick failed: {}", e);
                }
                tokio::time::sleep(config.update_interval).await;
            }
        } => {
            result
        }
        _ = shutdown_signal => {
            info!("Shutting down daemon...");
            Ok(())
        }
    }
}
