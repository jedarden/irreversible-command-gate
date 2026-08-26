//! Assignment Repair Monitor Binary
//!
//! Continuous monitoring service that detects and repairs stuck bead assignment
//! states where beads remain assigned to inactive workers.
//!
//! ## Usage
//!
//! ```bash
//! assignment-repair-monitor [OPTIONS]
//! ```
//!
//! ## Environment Variables
//!
//! - `ICG_WORKSPACE_PATH`: Path to workspace root (default: current directory)
//! - `ICG_CHECK_INTERVAL_SECONDS`: Interval between checks (default: 300)
//! - `ICG_AUTO_REPAIR_ENABLED`: Enable auto-repair (default: true)
//! - `ICG_DRY_RUN`: Dry-run mode, no changes (default: false)

use anyhow::Result;
use clap::Parser;
use icg::assignment_repair::{AssignmentRepairMonitor, AssignmentRepairConfig};
use std::path::PathBuf;

#[derive(Parser, Debug)]
#[command(name = "assignment-repair-monitor")]
#[command(about = "Continuous bead assignment state repair service", long_about = None)]
#[command(author = "ICG Team")]
#[command(version)]
struct Args {
    /// Path to workspace root (default: current directory)
    #[arg(long, default_value = ".")]
    workspace_path: PathBuf,

    /// Interval between repair checks in seconds (default: 300)
    #[arg(long, default_value = "300")]
    check_interval: u64,

    /// Disable auto-repair when stale assignments are detected
    #[arg(long)]
    no_auto_repair: bool,

    /// Dry-run mode (report only, no changes)
    #[arg(long)]
    dry_run: bool,

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
    let workspace_path = if args.workspace_path == PathBuf::from(".") {
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
    let config = AssignmentRepairConfig {
        workspace_path: workspace_path.clone(),
        check_interval: std::time::Duration::from_secs(args.check_interval),
        auto_repair_enabled: !args.no_auto_repair,
        dry_run: args.dry_run,
        ..Default::default()
    };

    if args.verbose {
        eprintln!("🔧 Assignment Repair Monitor");
        eprintln!();
        eprintln!("Configuration:");
        eprintln!("  Workspace: {}", config.workspace_path.display());
        eprintln!("  Check interval: {} seconds", config.check_interval.as_secs());
        eprintln!("  Auto-repair: {}", config.auto_repair_enabled);
        eprintln!("  Dry-run: {}", config.dry_run);
        eprintln!();
    }

    // Create monitor
    let mut monitor = AssignmentRepairMonitor::with_config(config)?;

    if args.once {
        // Run single check and exit
        eprintln!("🔍 Running single assignment repair check...");

        let report = monitor.run_check()?;

        eprintln!();
        eprintln!("=== Assignment Repair Check Results ===");
        eprintln!("Total assigned beads: {}", report.total_assigned_beads);
        eprintln!("Active assignments: {}", report.active_assignments);
        eprintln!("Stale assignments: {}", report.stale_assignments);
        eprintln!("Repairs performed: {}", report.repairs_performed.len());
        eprintln!();

        // Print detailed summary
        monitor.print_summary(&report);

        // Print Prometheus metrics
        eprintln!();
        eprintln!("=== Prometheus Metrics ===");
        eprintln!("{}", monitor.export_prometheus(Some(&report)));

        // Exit with error code if stale assignments were detected but not repaired
        if report.stale_assignments > 0 && report.repairs_performed.is_empty() && !monitor.config().dry_run {
            eprintln!("⚠️  Stale assignments detected but not repaired");
            std::process::exit(1);
        }

        Ok(())
    } else {
        // Run continuous monitoring
        let rt = tokio::runtime::Runtime::new()?;
        rt.block_on(async {
            // Set up Ctrl+C handler
            let (shutdown_tx, mut shutdown_rx) = tokio::sync::oneshot::channel();

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
                if let Err(e) = monitor.run().await {
                    eprintln!("❌ Monitor error: {:#}", e);
                    std::process::exit(1);
                }
            });

            // Wait for shutdown signal
            let _ = shutdown_rx.await;
            monitor_task.abort();

            eprintln!("👋 Assignment repair monitor shut down gracefully");
            Ok(())
        })
    }
}
