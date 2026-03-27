mod config;
mod inhibit;
mod metrics;
mod service;

use anyhow::Result;
use clap::Parser;
use humantime::Duration;
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
    /// Path to configuration file
    #[arg(long, short)]
    config: Option<PathBuf>,

    /// Validate configuration and exit
    #[arg(long)]
    validate_config: bool,

    /// Dry run mode (don't actually inhibit sleep)
    #[arg(long)]
    dry_run: bool,

    /// Duration to run in dry run mode (e.g., "60s", "5m"). Use "forever" to run indefinitely.
    #[arg(long, default_value = "30s")]
    duration: Duration,
    /// Run dry mode indefinitely (overrides --duration)
    #[arg(long)]
    forever: bool,

    /// Run in foreground (default: daemon mode)
    #[arg(long)]
    foreground: bool,
}

#[tokio::main]
async fn main() -> ExitCode {
    let args = Args::parse();

    // Initialize tracing subscriber
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive("rouser=debug".parse().unwrap()),
        )
        .with_target(true)
        .with_level(true)
        .with_thread_ids(false)
        .with_thread_names(false)
        .init();

    info!("rouser starting...");

    // Load configuration
    let config_path = args.config.clone().unwrap_or_else(|| {
        PathBuf::from("/etc/rouser/config.toml")
    });

    let config_loader = ConfigLoader::new(&config_path);

    // Validate configuration if requested
    if args.validate_config {
        match config_loader.validate() {
            Ok(_) => {
                info!("Configuration is valid");
                return ExitCode::SUCCESS;
            }
            Err(e) => {
                error!("Configuration validation failed: {}", e);
                return ExitCode::FAILURE;
            }
        }
    }

    // Load configuration for normal operation
    let config = match config_loader.load() {
        Ok(cfg) => {
            info!("Loaded configuration from {}", config_path.display());
            cfg
        }
        Err(e) => {
            error!("Failed to load configuration: {}", e);
            return ExitCode::FAILURE;
        }
    };

    // Run in dry-run mode if requested
    if args.dry_run {
        match run_dry_run(&config, args.duration, args.forever).await {
            Ok(_) => {
                info!("Dry run completed successfully");
                ExitCode::SUCCESS
            }
            Err(e) => {
                error!("Dry run failed: {}", e);
                ExitCode::FAILURE
            }
        }
    } else {
        // Normal daemon mode
        match run_daemon(&config, args.foreground).await {
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

async fn run_dry_run(
    config: &config::Config,
    duration: humantime::Duration,
    forever: bool,
) -> Result<()> {
    info!("Running in dry-run mode (forever: {})", forever);
    if !forever {
        info!("Duration: {:?}", std::time::Duration::from(duration));
    }
    info!("Configuration:");
    info!("  - CPU threshold: {}%", config.thresholds.cpu_usage);
    info!("  - GPU threshold: {}%", config.thresholds.gpu_usage);
    info!("  - Network threshold: {} Mbps", config.thresholds.network_io);
    info!("  - Disk threshold: {} MB/s", config.thresholds.disk_activity);
    info!("  - Duration threshold: {:?}", config.timing.duration_threshold);
    info!("  - Idle duration: {:?}", config.timing.idle_duration);

    let mut service = DataService::new(config, true).await?;

    let start = std::time::Instant::now();
    loop {
        if !forever && start.elapsed() >= duration.into() {
            info!("Dry run completed after {:?}", start.elapsed());
            break;
        }
        tokio::time::sleep(config.daemon.update_interval).await;
        service.tick(config).await?;
    }
    Ok(())
}

async fn run_daemon(config: &config::Config, foreground: bool) -> Result<()> {
    info!("Starting rouser daemon in {} mode", if foreground { "foreground" } else { "daemon" });

    // Check for required systemd/logind environment
    if std::env::var("NOTIFY_SOCKET").is_err() && !foreground && which::which("systemd-run").is_ok() {
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
                    tokio::time::sleep(config.daemon.update_interval).await;
                }
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
