mod coverage;
mod trust_pointer;

use anyhow::Context;
use anyhow::Result;
use clap::{Parser, Subcommand};
use coverage::*;
use std::path::PathBuf;
use trust_pointer::*;

#[derive(Parser)]
#[command(name = "icg")]
#[command(about = "irreversible-command-gate: coverage-diff CI tool", long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Compare rule packs and detect coverage regressions
    CoverageDiff {
        /// Path to previous release's rule pack manifest
        previous: PathBuf,
        /// Path to current release's rule pack manifest
        current: PathBuf,
    },
    /// Trust pointer management (Layer 4 minimal form)
    #[command(subcommand)]
    Trust(TrustSubcommand),
}

#[derive(Subcommand)]
enum TrustSubcommand {
    /// Show the currently trusted release reference
    Show {
        /// Path to trust pointer file (defaults to XDG_CONFIG_HOME/icg/trust-pointer.json)
        #[arg(short, long)]
        path: Option<PathBuf>,
    },
    /// Set a new trusted release reference
    Set {
        /// The release reference to trust (tag, commit SHA, or version)
        trusted_ref: String,
        /// Optional justification for why this ref is trusted
        #[arg(short, long)]
        justification: Option<String>,
        /// Path to trust pointer file (defaults to XDG_CONFIG_HOME/icg/trust-pointer.json)
        #[arg(short, long)]
        path: Option<PathBuf>,
    },
    /// Check if a given reference is currently trusted
    Check {
        /// The reference to check against the trust pointer
        reference: String,
        /// Path to trust pointer file (defaults to XDG_CONFIG_HOME/icg/trust-pointer.json)
        #[arg(short, long)]
        path: Option<PathBuf>,
    },
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Commands::CoverageDiff { previous, current } => {
            let diff = run_coverage_diff(previous, current)?;

            // Print structured report
            println!("# Coverage Diff Report\n");
            println!("## Removed Guarded Patterns");

            if diff.removed_guarded_patterns.is_empty() {
                println!("No removed guarded patterns\n");
            } else {
                for id in &diff.removed_guarded_patterns {
                    println!("- {}", id);
                }
                println!();
            }

            println!("## Widened Safe Patterns");

            if diff.widened_safe_patterns.is_empty() {
                println!("No widened safe patterns\n");
            } else {
                for change in &diff.widened_safe_patterns {
                    println!("- Pattern ID: {}", change.pattern_id);
                    println!("  Previous: {}", change.previous);
                    println!("  Current: {}", change.current);
                    println!();
                }
            }

            println!("## Narrowed Destructive Patterns");

            if diff.narrowed_destructive_patterns.is_empty() {
                println!("No narrowed destructive patterns\n");
            } else {
                for change in &diff.narrowed_destructive_patterns {
                    println!("- Pattern ID: {}", change.pattern_id);
                    println!("  Previous: {}", change.previous);
                    println!("  Current: {}", change.current);
                    println!();
                }
            }

            // Exit with error if regressions detected
            if diff.has_regressions() {
                eprintln!("\n❌ Coverage regressions detected!");
                eprintln!("This release contains changes that reduce protection coverage.");
                eprintln!("Explicit justification required for release approval.");
                std::process::exit(1);
            } else {
                println!("✅ No coverage regressions detected.");
            }

            Ok(())
        }
        Commands::Trust(subcommand) => match subcommand {
            TrustSubcommand::Show { path } => {
                let store_path = path.or_else(|| TrustPointerStore::default_path().ok())
                    .context("Failed to determine trust pointer path")?;
                let store = TrustPointerStore::new(store_path);

                match store.load()? {
                    Some(pointer) => {
                        println!("# Trust Pointer Status");
                        println!();
                        println!("**Trusted Reference:** `{}`", pointer.trusted_ref);
                        println!("**Last Updated:** {}", pointer.updated_at);
                        if let Some(justification) = pointer.justification {
                            println!("**Justification:** {}", justification);
                        }
                    }
                    None => {
                        println!("No trust pointer exists yet.");
                        println!();
                        println!("To set one, run:");
                        println!("  icg trust set <reference>");
                    }
                }

                Ok(())
            }
            TrustSubcommand::Set {
                trusted_ref,
                justification,
                path,
            } => {
                let store_path = path.or_else(|| TrustPointerStore::default_path().ok())
                    .context("Failed to determine trust pointer path")?;
                let store = TrustPointerStore::new(store_path);

                if let Some(justification) = justification {
                    store.set_trusted_ref_with_justification(&trusted_ref, justification)?;
                } else {
                    store.set_trusted_ref(&trusted_ref)?;
                }

                println!("✅ Trust pointer updated to: `{}`", trusted_ref);

                Ok(())
            }
            TrustSubcommand::Check { reference, path } => {
                let store_path = path.or_else(|| TrustPointerStore::default_path().ok())
                    .context("Failed to determine trust pointer path")?;
                let store = TrustPointerStore::new(store_path);

                let is_trusted = store.is_trusted(&reference)?;

                if is_trusted {
                    println!("✅ Reference `{}` is trusted.", reference);
                    std::process::exit(0);
                } else {
                    match store.get_trusted_ref()? {
                        Some(trusted) => {
                            println!("❌ Reference `{}` is NOT trusted.", reference);
                            println!("Current trusted reference: `{}`", trusted);
                        }
                        None => {
                            println!("❌ Reference `{}` is NOT trusted.", reference);
                            println!("No trust pointer exists yet.");
                        }
                    }
                    std::process::exit(1);
                }
            }
        },
    }
}
