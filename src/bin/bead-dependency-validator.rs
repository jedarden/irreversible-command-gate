//! Bead Dependency Validator Binary
//!
//! Standalone tool to detect and fix circular dependencies and orphaned
//! references in bead-rs databases.

use anyhow::Result;
use clap::Parser;
use icg::bead_dependency_validator::{DependencyValidator, ValidatorConfig};
use std::path::PathBuf;

#[derive(Parser, Debug)]
#[command(name = "bead-dependency-validator")]
#[command(about = "Validate and fix bead dependency issues", long_about = None)]
struct Args {
    /// Path to the beads.db SQLite database
    #[arg(long, default_value = ".beads/beads.db")]
    db_path: PathBuf,

    /// Path to events.jsonl for logging fixes
    #[arg(long, default_value = ".beads/events.jsonl")]
    events_path: PathBuf,

    /// Dry-run mode - detect issues but don't fix them
    #[arg(long)]
    dry_run: bool,

    /// Verbose output
    #[arg(long, short)]
    verbose: bool,
}

fn main() -> Result<()> {
    let args = Args::parse();

    // Validate paths
    if !args.db_path.exists() {
        eprintln!("❌ Database not found at: {}", args.db_path.display());
        eprintln!("Current directory: {:?}", std::env::current_dir());
        eprintln!("Make sure you're in a workspace with a .beads directory");
        std::process::exit(1);
    }

    let config = ValidatorConfig {
        db_path: args.db_path.clone(),
        events_path: args.events_path.clone(),
        dry_run: args.dry_run,
    };

    if args.verbose {
        eprintln!("Configuration:");
        eprintln!("  Database: {}", config.db_path.display());
        eprintln!("  Events: {}", config.events_path.display());
        eprintln!("  Dry run: {}", config.dry_run);
        eprintln!();
    }

    let mut validator = DependencyValidator::with_config(config)?;
    let result = validator.validate_and_fix()?;

    validator.print_summary(&result);

    // Exit with appropriate code
    if result.issues_found.is_empty() {
        Ok(())
    } else {
        // Return error if issues were found (even if fixed)
        // This helps scripts detect when fixes were applied
        std::process::exit(2);
    }
}
