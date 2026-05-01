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

fn resolve_initial_log_level(args: &Args) -> String {
    if let Some(ref cli_val) = args.log_level {
        return cli_val.to_string();
    }
    std::env::var("RUST_LOG").unwrap_or_else(|_| "info".to_string())
}

fn load_single_config(path: &std::path::Path) -> Result<config::Config> {
    let loader = ConfigLoader::new(path);
    loader
        .load()
        .map_err(|e| anyhow::anyhow!("Failed to load config from {}: {}", path.display(), e))
}

fn init_tracing(log_level: &str) {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_new(log_level).unwrap_or_else(|e| {
                eprintln!("Invalid log level '{}': {}. Using 'info'.", log_level, e);
                tracing_subscriber::EnvFilter::new("info")
            }),
        )
        .with_target(true)
        .with_level(true)
        .with_thread_ids(false)
        .with_thread_names(false)
        .init();
}

#[tokio::main]
async fn main() -> ExitCode {
    let args = Args::parse();

    // Initialize tracing early so that auto-install logs during config load are captured.
    init_tracing(&resolve_initial_log_level(&args));

    // --print-config: merge all configs and serialize back to TOML.
    if args.print_config {
        match ConfigLoader::load_merged() {
            Ok((config, _)) => {
                if let Err(e) = ConfigLoader::print_config_toml(&config, &mut std::io::stdout()) {
                    eprintln!("Error: {}", e);
                    return ExitCode::FAILURE;
                }
            }
            Err(e) => {
                error!("Failed to load and merge configuration: {}", e);
                return ExitCode::FAILURE;
            }
        }
        return ExitCode::SUCCESS;
    }

    // Load config with log_level for tracing init.
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
        "  - GPU threshold: {}%, EMA alpha: {:.2}",
        config.metrics.gpu.threshold, config.metrics.gpu.ema_alpha
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
