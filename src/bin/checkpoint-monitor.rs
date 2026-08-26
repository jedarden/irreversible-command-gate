//! Checkpoint Monitor - Automated Verification and Repair
//!
//! This binary provides continuous monitoring and automatic repair of the
//! bead checkpoint system, preventing invisible beads caused by sync failures.
//!
//! ## Usage
//!
//! ```bash
//! # Run once (verification mode)
//! cargo run --bin checkpoint-monitor
//!
//! # Run with custom configuration
//! cargo run --bin checkpoint-monitor -- --interval-secs 600 --stale-threshold-minutes 10
//!
//! # Install as systemd service
//! sudo cp systemd/icg-checkpoint-monitor.service /etc/systemd/system/
//! sudo systemctl enable icg-checkpoint-monitor
//! sudo systemctl start icg-checkpoint-monitor
//! ```

use anyhow::{Context, Result};
use chrono::Utc;
use clap::Parser;
use icg::checkpoint_monitor::{
    CheckpointMonitor, CheckpointMonitorConfig, CheckpointMonitorReport,
};
use std::fs;
use std::io::Write;
use std::path::PathBuf;
use std::time::Duration as StdDuration;
use tokio::time::interval;

/// Checkpoint monitor - automated verification and repair
#[derive(Parser, Debug)]
#[command(name = "checkpoint-monitor")]
#[command(about = "Automated checkpoint verification and repair tool", long_about = None)]
struct Args {
    /// Workspace path (default: current directory)
    #[arg(short, long, default_value = ".")]
    workspace: PathBuf,

    /// Interval between checks in seconds (default: 300)
    #[arg(short, long, default_value = "300")]
    interval_secs: u64,

    /// Threshold for considering checkpoint stale, in minutes (default: 5)
    #[arg(short, long, default_value = "5")]
    stale_threshold_minutes: i64,

    /// Disable automatic repair (verification only)
    #[arg(short, long, default_value = "false")]
    no_repair: bool,

    /// Run once and exit (don't loop)
    #[arg(short, long, default_value = "false")]
    once: bool,

    /// Path to bead CLI (default: "bead")
    #[arg(long, default_value = "bead")]
    bead_path: String,

    /// Verbose output
    #[arg(short, long)]
    verbose: bool,
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();

    // Build configuration
    let config = CheckpointMonitorConfig {
        workspace_path: args.workspace.clone(),
        check_interval: StdDuration::from_secs(args.interval_secs),
        stale_threshold_minutes: args.stale_threshold_minutes,
        auto_repair_enabled: !args.no_repair,
        bead_path: PathBuf::from(args.bead_path),
    };

    // Create monitor
    let mut monitor = CheckpointMonitor::with_config(config)
        .context("Failed to create checkpoint monitor")?;

    println!("🔍 Checkpoint Monitor started");
    println!("   Workspace: {}", args.workspace.display());
    println!("   Interval: {}s", args.interval_secs);
    println!("   Stale threshold: {} min", args.stale_threshold_minutes);
    println!("   Auto-repair: {}", !args.no_repair);
    println!();

    if args.once {
        // Run once and exit
        run_single_check(&mut monitor, args.verbose)?;
    } else {
        // Run continuously
        run_continuous_loop(&mut monitor, args.interval_secs, args.verbose).await?;
    }

    Ok(())
}

/// Run a single checkpoint health check
fn run_single_check(monitor: &mut CheckpointMonitor, verbose: bool) -> Result<()> {
    let timestamp = Utc::now();
    println!("🕥 Running checkpoint health check at {}", timestamp.to_rfc3339());

    let report = monitor
        .run_check()
        .context("Failed to run checkpoint health check")?;

    print_report(&report, verbose)?;

    // Save report to diagnostics
    let report_path = monitor.config().checkpoint_report_path();
    fs::create_dir_all(report_path.parent().unwrap())
        .context("Failed to create diagnostics directory")?;

    let report_json = serde_json::to_string_pretty(&report)
        .context("Failed to serialize report")?;
    let mut file = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&report_path)
        .context("Failed to open report file")?;
    writeln!(file, "{}", report_json)
        .context("Failed to write report")?;

    if verbose {
        println!("📝 Report saved to {}", report_path.display());
    }

    // Exit with error code if critical issues found
    if report.health_status == "critical" {
        std::process::exit(1);
    }

    Ok(())
}

/// Run checkpoint monitoring in a continuous loop
async fn run_continuous_loop(
    monitor: &mut CheckpointMonitor,
    interval_secs: u64,
    verbose: bool,
) -> Result<()> {
    let mut timer = interval(StdDuration::from_secs(interval_secs));
    timer.tick().await; // Skip first immediate tick

    loop {
        timer.tick().await;

        if let Err(e) = run_single_check(monitor, verbose) {
            eprintln!("❌ Check failed: {}", e);
            // Continue loop despite errors
        }
    }
}

/// Print a formatted report to stdout
fn print_report(report: &CheckpointMonitorReport, verbose: bool) -> std::io::Result<()> {
    // Health status with emoji
    let status_emoji = match report.health_status.as_str() {
        "healthy" => "✅",
        "degraded" => "⚠️",
        "warning" => "🟡",
        "critical" => "🔴",
        _ => "❓",
    };

    println!("{} Health Status: {}", status_emoji, report.health_status);
    println!();

    // Checkpoint sync status
    println!("📋 Checkpoint Sync:");
    println!("   Exists: {}", report.checkpoint_sync.checkpoint_exists);
    if let Some(ts) = &report.checkpoint_sync.checkpoint_timestamp {
        println!("   Timestamp: {}", ts.to_rfc3339());
    }
    if let Some(count) = report.checkpoint_sync.checkpoint_issue_count {
        println!("   Issues: {}", count);
    }
    println!("   Status: {}", report.checkpoint_sync.sync_status);
    println!("   Stale: {}", report.checkpoint_sync.stale);
    if let Some(mins) = report.checkpoint_sync.stale_minutes {
        println!("   Stale by: {} min", mins);
    }
    println!("   Corrupted: {}", report.checkpoint_sync.corrupted);
    if let Some(details) = &report.checkpoint_sync.corruption_details {
        println!("   Corruption: {}", details);
    }

    // Bead-level drift information
    if report.checkpoint_sync.drift_count > 0 {
        println!("   Drift: {} beads differ", report.checkpoint_sync.drift_count);
        if !report.checkpoint_sync.beads_missing_in_database.is_empty() {
            println!("   Missing in database: {} beads",
                report.checkpoint_sync.beads_missing_in_database.len());
            if verbose {
                for id in report.checkpoint_sync.beads_missing_in_database.iter().take(5) {
                    println!("     - {}", id);
                }
                if report.checkpoint_sync.beads_missing_in_database.len() > 5 {
                    println!("     ... and {} more",
                        report.checkpoint_sync.beads_missing_in_database.len() - 5);
                }
            }
        }
        if !report.checkpoint_sync.beads_missing_in_checkpoint.is_empty() {
            println!("   Missing in checkpoint: {} beads",
                report.checkpoint_sync.beads_missing_in_checkpoint.len());
            if verbose {
                for id in report.checkpoint_sync.beads_missing_in_checkpoint.iter().take(5) {
                    println!("     - {}", id);
                }
                if report.checkpoint_sync.beads_missing_in_checkpoint.len() > 5 {
                    println!("     ... and {} more",
                        report.checkpoint_sync.beads_missing_in_checkpoint.len() - 5);
                }
            }
        }
    }
    println!();

    // Database health status
    println!("💾 Database Health:");
    println!("   Exists: {}", report.database_health.exists);
    println!("   Readable: {}", report.database_health.readable);
    println!("   Schema Valid: {}", report.database_health.schema_valid);
    println!("   Corrupted: {}", report.database_health.corrupted);
    if let Some(details) = &report.database_health.error_details {
        println!("   Error: {}", details);
    }
    println!();

    // Repair actions
    if report.repair_triggered {
        println!("🔧 Repairs Performed:");
        for repair in &report.repairs_performed {
            let emoji = if repair.success { "✅" } else { "❌" };
            println!("   {} {}: {}", emoji, repair.repair_type, repair.message);
        }
        println!();
    }

    // Recommended actions
    if !report.recommended_actions.is_empty() {
        println!("💡 Recommended Actions:");
        for action in &report.recommended_actions {
            println!("   • {}", action);
        }
        println!();
    }

    // Verbose details
    if verbose {
        println!("📊 Check Details:");
        println!("   Interval: {}s", report.check_interval_seconds);
        println!("   Timestamp: {}", report.timestamp.to_rfc3339());
        println!("   Repair Triggered: {}", report.repair_triggered);
    }

    Ok(())
}
