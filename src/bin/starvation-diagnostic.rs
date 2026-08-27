//! Starvation diagnostic binary
//!
//! Automated diagnostic tool for bead starvation detection and root cause analysis.
//! Runs when Pluck reports zero candidates despite open beads existing.
//!
//! Usage:
//!   cargo run --bin starvation-diagnostic
//!   cargo run --bin starvation-diagnostic -- --report-only
//!   cargo run --bin starvation-diagnostic -- --db-path /path/to/.beads/beads.db

use anyhow::Result;
use clap::Parser;
use icg::starvation_diagnostic::{StarvationDiagnostic, StarvationDiagnosticConfig};
use std::path::PathBuf;

/// Starvation diagnostic tool for bead database
#[derive(Parser, Debug)]
#[command(name = "starvation-diagnostic")]
#[command(about = "Automated diagnostic for bead starvation detection", long_about = None)]
struct Args {
    /// Path to the beads.db SQLite database
    #[arg(long, default_value = ".beads/beads.db")]
    db_path: PathBuf,

    /// Generate report only (no auto-repair)
    #[arg(long, default_value = "false")]
    report_only: bool,

    /// Automatically repair stale assignees (clear assignee from open beads with dead workers)
    #[arg(long, default_value = "false")]
    auto_repair: bool,

    /// Checkpoint stale threshold in minutes
    #[arg(long, default_value = "5")]
    checkpoint_stale_threshold: i64,

    /// Output JSON report only (no human-readable output)
    #[arg(long, default_value = "false")]
    json: bool,
}

fn main() -> Result<()> {
    let args = Args::parse();

    let workspace_path = args
        .db_path
        .parent()
        .and_then(|p| p.parent())
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));

    let config = StarvationDiagnosticConfig {
        db_path: args.db_path.clone(),
        diagnostics_dir: workspace_path.join(".beads/diagnostics"),
        report_only: args.report_only,
        auto_repair: args.auto_repair,
        events_path: workspace_path.join(".beads/events.jsonl"),
        checkpoint_stale_threshold_minutes: args.checkpoint_stale_threshold,
    };

    let mut diagnostic = StarvationDiagnostic::with_config(config)?;

    eprintln!("🔍 Running starvation diagnostic...");
    eprintln!("📁 Database: {}", args.db_path.display());

    if args.auto_repair {
        eprintln!("🔧 Auto-repair ENABLED - will clear stale assignees");
    }

    let report = diagnostic.run_diagnostic()?;

    if args.json {
        // Output JSON report only
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else {
        // Output human-readable summary
        diagnostic.print_summary(&report);
    }

    // Exit with non-zero if starvation detected
    if report.summary.starvation_detected {
        eprintln!("\n🚨 Starvation detected - investigate recommended actions");
        std::process::exit(1);
    }

    eprintln!("\n✅ Diagnostic completed successfully");
    Ok(())
}
