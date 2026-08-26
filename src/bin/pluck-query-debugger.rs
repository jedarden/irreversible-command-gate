//! Pluck Query Debugger CLI
//!
//! Command-line tool for running the Pluck query debugging analysis.
//! Diagnoses why beads are invisible in the ready frontier by progressively
//! relaxing query filters and reporting which beads become visible at each level.

use anyhow::Result;
use clap::Parser;
use icg::pluck_query_debugger::{PluckQueryDebugger, PluckQueryDebuggerConfig};
use std::path::PathBuf;

#[derive(Parser, Debug)]
#[command(name = "pluck-query-debugger")]
#[command(about = "Debug Pluck query filters to diagnose bead invisibility")]
#[command(long_about = "
SQL-level diagnostic tool that replays Pluck's ready frontier query with
progressive filter relaxation to diagnose exactly which filters are excluding
beads from the ready frontier.

The debugger runs the query at 5 relaxation levels:
  Level 0: Exact Pluck query (all filters applied)
  Level 1: Without label exclusions
  Level 2: Without dependency checks
  Level 3: Without assignee filter
  Level 4: Raw base query (status only)

For each level, the debugger reports which beads become visible and which
filter was relaxed to make them visible.

Output is written to .beads/diagnostics/pluck-query-debug-report.jsonl
")]
struct Args {
    /// Path to the beads.db SQLite database
    #[arg(long, default_value = ".beads/beads.db")]
    db_path: PathBuf,

    /// Path to the diagnostics output directory
    #[arg(long, default_value = ".beads/diagnostics")]
    diagnostics_dir: PathBuf,

    /// Print human-readable summary to stdout
    #[arg(long, default_value = "false")]
    summary: bool,
}

fn main() -> Result<()> {
    let args = Args::parse();

    let config = PluckQueryDebuggerConfig {
        db_path: args.db_path,
        diagnostics_dir: args.diagnostics_dir,
        print_summary: args.summary,
    };

    println!("🔍 Starting Pluck query debug analysis...");
    println!("📁 Database: {}", config.db_path.display());

    let mut debugger = PluckQueryDebugger::with_config(config)?;

    let report = debugger.run_debug_analysis()?;

    // Print summary if requested or if starvation detected
    if args.summary || report.starvation_detected {
        debugger.print_summary(&report);
    }

    if report.starvation_detected {
        eprintln!("🚨 STARVATION DETECTED: See report for details");
        std::process::exit(1);
    }

    Ok(())
}
