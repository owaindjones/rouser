mod config;
mod inhibit;
mod metrics;
mod service;

use anyhow::Result;
use clap::Parser;
use std::path::PathBuf;
use std::process::ExitCode;
use tracing::{error, info, warn};

use config::ConfigLoader;
use service::DataService;

const DEFAULT_CONFIG_PATHS: &[&str] = &[
    "./config/rouser.toml",
    "~/.config/rouser/config.toml",
    "/etc/rouser/config.toml",
];

fn resolve_config_path(args: &Args) -> (PathBuf, Vec<String>) {
    let fallback = PathBuf::from(DEFAULT_CONFIG_PATHS.last().unwrap());
    if let Some(ref path) = args.config {
        return (path.clone(), vec![path.display().to_string()]);
    }
    let mut searched: Vec<String> = Vec::new();
    for pattern in DEFAULT_CONFIG_PATHS {
        let home = std::env::var("HOME").unwrap_or_else(|_| "~".to_string());
        let expanded = (*pattern).replace("~", &home);
        searched.push(expanded.clone());
        if std::path::Path::new(&expanded).exists() {
            return (PathBuf::from(expanded), searched);
        }
    }
    (fallback, searched)
}

/// rouser - A Linux daemon that monitors system metrics and inhibits sleep when activity thresholds are exceeded
#[derive(Parser)]
#[command(name = "rouser")]
#[command(author, version, about, long_about = None)]
struct Args {
    /// Path to configuration file [searches ./config/rouser.toml -> ~/.config/rouser/config.toml -> /etc/rouser/config.toml]
    #[arg(long, short)]
    config: Option<PathBuf>,

    /// Validate configuration and exit
    #[arg(long)]
    validate_config: bool,

    /// Dry run mode (don't actually inhibit sleep)
    #[arg(long)]
    dry_run: bool,

    /// Print the embedded default configuration and exit
    #[arg(long)]
    print_config: bool,

    /// Log level filter; overrides config.log_level and RUST_LOG env var.
    #[arg(long, short = 'l')]
    log_level: Option<String>,
}

#[tokio::main]
async fn main() -> ExitCode {
    let args = Args::parse();

    if args.print_config {
        config::ConfigLoader::print_default_config(&mut std::io::stdout()).ok();
        return ExitCode::SUCCESS;
    }

    // Load configuration to get log_level
    let (config_path, searched_paths) = resolve_config_path(&args);

    let config_loader = ConfigLoader::new(&config_path);
    let config_result = config_loader.clone().load();
    if config_result.is_err() {
        warn!(
            "No configuration file found at checked paths — using embedded defaults. \
             Checked: {}",
            searched_paths.join(", ")
        );
    }
    let should_validate = args.validate_config;

    // Resolve log level with precedence: CLI -l/--log-level > RUST_LOG env var > config.log_level > "info"
    let log_level = if let Some(ref cli_val) = args.log_level {
        cli_val.to_string()
    } else if let Ok(val) = std::env::var("RUST_LOG") {
        val
    } else {
        match &config_result {
            Ok(cfg) => cfg.log_level.clone(),
            Err(_) => "info".to_string(),
        }
    };

    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_new(&log_level).unwrap_or_else(|e| {
                eprintln!("Invalid log level '{}': {}. Using 'info'.", log_level, e);
                tracing_subscriber::EnvFilter::new("info")
            }),
        )
        .with_target(true)
        .with_level(true)
        .with_thread_ids(false)
        .with_thread_names(false)
        .init();

    info!("rouser starting...");

    // Validate configuration if requested — use full load() to catch serde errors
    if should_validate {
        match config_loader.load() {
            Ok(_) => {
                info!("Configuration validation passed");
                return ExitCode::SUCCESS;
            }
            Err(e) => {
                error!("Configuration validation failed: {}", e);
                return ExitCode::FAILURE;
            }
        }
    }

    // Load configuration for normal/dry-run operation (reload now that logging is initialized)
    let config = config_loader.load().unwrap_or_else(|e| {
        eprintln!("Failed to load configuration: {}", e);
        std::process::exit(1);
    });

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
        // Normal daemon mode
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
    if std::env::var("NOTIFY_SOCKET").is_err() && which::which("systemd-run").is_ok() {
        warn!("NOTIFY_SOCKET not set, consider running under systemd");
    }

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
