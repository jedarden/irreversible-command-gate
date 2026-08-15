mod coverage;
mod engine;
mod rule_pack;
mod trust_pointer;
mod update;

use anyhow::Context;
use anyhow::Result;
use clap::{Parser, Subcommand};
use coverage::*;
use engine::{ContentSource, Engine, InputSource};
use std::path::PathBuf;
use trust_pointer::*;
use update::*;

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
    /// Update rule pack from GitHub Releases (per the trust pointer)
    Update {
        /// Path to trust pointer file (defaults to /etc/icg/trust-pointer.json)
        #[arg(short, long)]
        trust_pointer_path: Option<PathBuf>,
        /// Path where the rule pack artifact should be stored (defaults to /etc/icg/rule-pack.json)
        #[arg(short, long)]
        artifact_path: Option<PathBuf>,
    },
    /// Show current status and blind-spot self-report
    Status {
        /// Path to trust pointer file (defaults to /etc/icg/trust-pointer.json)
        #[arg(short, long)]
        trust_pointer_path: Option<PathBuf>,
    },
    /// Scaffold a new rule pack
    NewPack {
        /// Name for the new rule pack
        pack_name: String,
        /// Target output directory
        #[arg(short, long)]
        output_dir: Option<PathBuf>,
    },
    /// Hook mode: invoked by Claude Code/Codex's PreToolUse hook system
    Hook,
    /// Wrapper mode: invoked under a shadowed binary name (e.g., vault, git, docker)
    #[command(hide = true)]
    Wrapper {
        /// Command arguments (shadowed executable invocation)
        #[arg(required = true, trailing_var_arg = true)]
        args: Vec<String>,
    },
}

#[derive(Subcommand)]
enum TrustSubcommand {
    /// Show the currently trusted release reference
    Show {
        /// Path to trust pointer file (defaults to /etc/icg/trust-pointer.json)
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
        /// Path to trust pointer file (defaults to /etc/icg/trust-pointer.json)
        #[arg(short, long)]
        path: Option<PathBuf>,
    },
    /// Check if a given reference is currently trusted
    Check {
        /// The reference to check against the trust pointer
        reference: String,
        /// Path to trust pointer file (defaults to /etc/icg/trust-pointer.json)
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
        Commands::Hook => {
            // Hook mode: read PreToolUse JSON from stdin, segment commands, and evaluate
            let engine = Engine::new();

            // Read input from stdin (either command-mode or content-mode)
            match engine.read_from_stdin()? {
                Some(InputSource::Command(source)) => {
                    // Command-mode: Bash command
                    let tokens = engine.segment_command(&source);

                    // For now, just print what we found
                    // TODO: Dispatch to rule packs (bead irrevers-XXXX)
                    eprintln!("Engine: Hook mode (command-mode) - {} segments found", tokens.len());
                    for (i, token) in tokens.iter().enumerate() {
                        eprintln!("  Segment {}: executable='{}', args={:?}",
                                 i, token.executable, token.args);
                    }

                    // Allow by default until rule packs are implemented
                    Ok(())
                }
                Some(InputSource::Content(content)) => {
                    // Content-mode: Write/Edit operation
                    eprintln!("Engine: Hook mode (content-mode)");
                    eprintln!("  File path: {}", content.file_path());
                    eprintln!("  New content length: {} bytes", content.new_content().len());

                    // For now, just print what we found
                    // TODO: Implement content-mode checks for storage-class/image-tag packs
                    // TODO: Dispatch to rule packs (separate bead)

                    // Allow by default until content-mode packs are implemented
                    Ok(())
                }
                None => {
                    // Unrecognized tool - allow by default (fail-open)
                    Ok(())
                }
            }
        }
        Commands::Wrapper { args } => {
            // Wrapper mode: invoked as a shadowed binary (vault, git, docker, etc.)
            let engine = Engine::new();

            // Read from argv
            let source = engine.read_from_argv(args);
            let tokens = engine.segment_command(&source);

            // For now, just print what we found
            // TODO: Dispatch to rule packs, then exec the real binary if allowed
            eprintln!("Engine: Wrapper mode - {} segments found", tokens.len());
            for (i, token) in tokens.iter().enumerate() {
                eprintln!("  Segment {}: executable='{}', args={:?}",
                         i, token.executable, token.args);
            }

            // Allow by default until rule packs are implemented
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
        Commands::Update {
            trust_pointer_path,
            artifact_path,
        } => {
            let mut config = UpdateConfig::default();

            if let Some(trust_path) = trust_pointer_path {
                config.trust_pointer_path = trust_path;
            }

            if let Some(artifact_path_override) = artifact_path {
                config.artifact_path = artifact_path_override;
            }

            println!("🔄 icg update started");
            println!("📁 Trust pointer: {}", config.trust_pointer_path.display());
            println!("📁 Artifact path: {}", config.artifact_path.display());
            println!();

            let result = run_update(config)
                .context("Failed to run update")?;

            println!();
            println!("# Update Summary");
            println!();
            println!("**Trusted Reference:** {}", result.trusted_ref);
            println!("**Release Tag:** {}", result.release_tag);
            println!("**Artifact Path:** {}", result.artifact_path.display());
            if let Some(prev) = result.previous_version {
                println!("**Previous Version:** {}", prev);
            } else {
                println!("**Previous Version:** (none)");
            }

            Ok(())
        }
        Commands::Status { trust_pointer_path } => {
            // TODO: Implement full status reporting (bead irrevers-1cad33d2)
            let store_path = trust_pointer_path
                .or_else(|| TrustPointerStore::default_path().ok())
                .context("Failed to determine trust pointer path")?;
            let store = TrustPointerStore::new(store_path);

            println!("# icg Status");
            println!();

            match store.load()? {
                Some(pointer) => {
                    println!("**Trust Pointer:**");
                    println!("  Reference: `{}`", pointer.trusted_ref);
                    println!("  Last Updated: {}", pointer.updated_at);
                    if let Some(justification) = pointer.justification {
                        println!("  Justification: {}", justification);
                    }
                }
                None => {
                    println!("**Trust Pointer:** (not configured)");
                    println!("Run `icg trust set <reference>` to configure.");
                }
            }

            println!();
            println!("**Full status reporting coming soon** (bead irrevers-1cad33d2)");
            println!("This will include blind-spot self-report and detailed coverage status.");

            Ok(())
        }
        Commands::NewPack { pack_name, output_dir } => {
            // TODO: Implement pack scaffolding tool (bead irrevers-54b33e0c)
            println!("# icg new-pack");
            println!();
            println!("Pack name: `{}`", pack_name);

            let dest = output_dir.unwrap_or_else(|| PathBuf::from("."));
            println!("Output directory: {}", dest.display());
            println!();
            println!("**Pack scaffolding coming soon** (bead irrevers-54b33e0c)");
            println!("This will generate a new rule pack template with examples.");
            println!();
            println!("For now, create packs manually following the pattern in existing packs.");

            Ok(())
        }
    }
}
