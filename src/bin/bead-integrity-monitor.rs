//! Bead Integrity Monitor Binary
//!
//! Continuous monitoring service for bead database health with automatic
//! repair and Prometheus metrics export.
//!
//! ## Usage
//!
//! ```bash
//! bead-integrity-monitor [OPTIONS]
//! ```
//!
//! ## Environment Variables
//!
//! - `ICG_WORKSPACE_PATH`: Path to workspace root (default: current directory)
//! - `ICG_CHECK_INTERVAL_SECONDS`: Interval between checks (default: 300)
//! - `ICG_ALERT_THRESHOLD`: Alert threshold for stuck beads (default: 10)
//! - `ICG_AUTO_REPAIR_ENABLED`: Enable auto-repair (default: true)
//! - `ICG_MONITOR_HOST`: HTTP server host (default: 0.0.0.0)
//! - `ICG_MONITOR_PORT`: HTTP server port (default: 9095)

use anyhow::Result;
use clap::Parser;
use icg::bead_integrity_monitor::{IntegrityMonitor, IntegrityMonitorConfig};
use std::path::PathBuf;

#[derive(Parser, Debug)]
#[command(name = "bead-integrity-monitor")]
#[command(about = "Continuous bead integrity verification service", long_about = None)]
#[command(author = "ICG Team")]
#[command(version)]
struct Args {
    /// Path to workspace root (default: current directory)
    #[arg(long, default_value = ".")]
    workspace_path: PathBuf,

    /// Interval between health checks in seconds (default: 300)
    #[arg(long, default_value = "300")]
    check_interval: u64,

    /// Alert threshold for stuck beads (default: 10)
    #[arg(long, default_value = "10")]
    alert_threshold: usize,

    /// Disable auto-repair when issues are detected
    #[arg(long)]
    no_auto_repair: bool,

    /// HTTP server host (default: 0.0.0.0)
    #[arg(long, default_value = "0.0.0.0")]
    host: String,

    /// HTTP server port (default: 9095)
    #[arg(long, default_value = "9095")]
    port: u16,

    /// Run a single check and exit (useful for testing)
    #[arg(long)]
    once: bool,

    /// Verbose output
    #[arg(long, short)]
    verbose: bool,
}

fn main() -> Result<()> {
    let args = Args::parse();

    // Validate workspace path
    let workspace_path = if args.workspace_path == *"." {
        std::env::current_dir()?
    } else {
        args.workspace_path
    };

    if !workspace_path.exists() {
        eprintln!("❌ Workspace path not found: {}", workspace_path.display());
        std::process::exit(1);
    }

    let beads_db = workspace_path.join(".beads").join("beads.db");
    if !beads_db.exists() {
        eprintln!("❌ Beads database not found: {}", beads_db.display());
        eprintln!("Make sure you're in a workspace with a .beads directory");
        std::process::exit(1);
    }

    // Build configuration
    let mut config = IntegrityMonitorConfig {
        workspace_path: workspace_path.clone(),
        check_interval: std::time::Duration::from_secs(args.check_interval),
        alert_threshold: args.alert_threshold,
        auto_repair_enabled: !args.no_auto_repair,
        ..Default::default()
    };

    // Override HTTP config from CLI args
    config.http_config.host = args.host;
    config.http_config.port = args.port;

    if args.verbose {
        eprintln!("🩺 Bead Integrity Monitor");
        eprintln!();
        eprintln!("Configuration:");
        eprintln!("  Workspace: {}", config.workspace_path.display());
        eprintln!(
            "  Check interval: {} seconds",
            config.check_interval.as_secs()
        );
        eprintln!("  Alert threshold: {} stuck beads", config.alert_threshold);
        eprintln!("  Auto-repair: {}", config.auto_repair_enabled);
        eprintln!("  HTTP server: {}", config.http_config.bind_address());
        eprintln!();
    }

    // Create monitor
    let mut monitor = IntegrityMonitor::new(config);

    if args.once {
        // Run single check and exit
        eprintln!("🔍 Running single integrity check...");

        let report = monitor.run_check()?;

        eprintln!();
        eprintln!("=== Integrity Check Results ===");
        eprintln!("Status: {}", report.status);
        eprintln!("Total checks: {}", report.total_checks);
        eprintln!("Passed: {}", report.passed_checks);
        eprintln!("Failed: {}", report.failed_checks);
        eprintln!("Warnings: {}", report.warning_checks);
        eprintln!();

        if report.repair_triggered {
            if let Some(ref repair) = report.repair_result {
                eprintln!("🔧 Auto-repair performed:");
                eprintln!("  Success: {}", repair.success);
                eprintln!("  Issues repaired: {}", repair.issues_repaired);
                eprintln!("  {}", repair.message);
                eprintln!();
            }
        }

        if report.alert_triggered {
            if let Some(ref reason) = report.alert_reason {
                eprintln!("🚨 ALERT TRIGGERED: {}", reason);
                eprintln!();
            }
        }

        // Print Prometheus metrics
        eprintln!("=== Prometheus Metrics ===");
        eprintln!("{}", monitor.export_prometheus());

        // Exit with error code if checks failed
        if report.failed_checks > 0 {
            std::process::exit(1);
        }

        Ok(())
    } else {
        // Run continuous monitoring with HTTP server
        let rt = tokio::runtime::Runtime::new()?;
        rt.block_on(async {
            // Set up Ctrl+C handler
            let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel();

            tokio::spawn(async move {
                tokio::signal::ctrl_c()
                    .await
                    .expect("failed to install Ctrl+C handler");
                eprintln!();
                eprintln!("🛑 Received shutdown signal");
                // Send will fail if receiver already dropped, which is fine
                let _ = shutdown_tx.send(());
            });

            // Spawn monitor task
            let monitor_task = tokio::spawn(async move {
                if let Err(e) = monitor.run_with_http_server().await {
                    eprintln!("❌ Monitor error: {:#}", e);
                    std::process::exit(1);
                }
            });

            // Wait for shutdown signal
            let _ = shutdown_rx.await;
            monitor_task.abort();

            eprintln!("👋 Bead integrity monitor shut down gracefully");
            Ok(())
        })
    }
}
