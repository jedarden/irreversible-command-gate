mod coverage;
mod engine;
mod overrides;
mod regression;
mod rule_pack;
mod trust_pointer;
mod update;

use anyhow::Context;
use anyhow::Result;
use clap::{Parser, Subcommand};
use coverage::*;
use engine::{Engine, InputSource};
use overrides::*;
use regression::{generate_regression_suite_from_manifest, write_regression_suite};
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
        /// Explicit rationale required when the diff reports a regression
        #[arg(short, long)]
        justification: Option<String>,
        /// Previous release's per-repository override TOML, if present
        #[arg(long)]
        previous_override: Option<PathBuf>,
        /// Current release's per-repository override TOML, if present
        #[arg(long)]
        current_override: Option<PathBuf>,
    },
    /// Generate and validate the fixed deny-regression suite for a rule pack
    RegressionSuite {
        /// Path to the rule-pack JSON manifest
        manifest: PathBuf,
        /// Optional path for the generated JSON suite (stdout by default)
        #[arg(short, long)]
        output: Option<PathBuf>,
        /// Optional verified per-repository override to exercise at the gate
        #[arg(long)]
        override_file: Option<PathBuf>,
        /// Repository scope from the override manifest
        #[arg(long)]
        repository: Option<String>,
        /// Exact release reference trusted by Layer 4
        #[arg(long)]
        trusted_ref: Option<String>,
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
        /// Channel identifier for canary rollout (e.g., "canary", "stable")
        ///
        /// When set, uses a channel-specific trust pointer file
        /// (e.g., /etc/icg/trust-pointer-canary.json instead of /etc/icg/trust-pointer.json).
        /// This supports staged rollout patterns where different fleet segments
        /// track different release channels.
        #[arg(long)]
        channel: Option<String>,
    },
    /// Show current status and blind-spot self-report
    Status {
        /// Path to trust pointer file (defaults to /etc/icg/trust-pointer.json)
        #[arg(short, long)]
        trust_pointer_path: Option<PathBuf>,
        /// Channel identifier for canary rollout (e.g., "canary", "stable")
        ///
        /// When set, uses a channel-specific trust pointer file
        /// (e.g., /etc/icg/trust-pointer-canary.json instead of /etc/icg/trust-pointer.json).
        #[arg(long)]
        channel: Option<String>,
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
    Hook {
        /// Optional rule-pack file (defaults to /etc/icg/rule-pack.json)
        #[arg(long)]
        rule_pack: Option<PathBuf>,
        /// Release-bound per-repository override; requires repository and trusted-ref
        #[arg(long)]
        override_file: Option<PathBuf>,
        /// Repository scope for the override
        #[arg(long)]
        repository: Option<String>,
        /// Exact trusted release reference for the override
        #[arg(long)]
        trusted_ref: Option<String>,
    },
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
        /// Channel identifier for canary rollout (e.g., "canary", "stable")
        ///
        /// When set, uses a channel-specific trust pointer file
        /// (e.g., /etc/icg/trust-pointer-canary.json instead of /etc/icg/trust-pointer.json).
        #[arg(long)]
        channel: Option<String>,
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
        /// Channel identifier for canary rollout (e.g., "canary", "stable")
        ///
        /// When set, uses a channel-specific trust pointer file
        /// (e.g., /etc/icg/trust-pointer-canary.json instead of /etc/icg/trust-pointer.json).
        #[arg(long)]
        channel: Option<String>,
    },
    /// Check if a given reference is currently trusted
    Check {
        /// The reference to check against the trust pointer
        reference: String,
        /// Path to trust pointer file (defaults to /etc/icg/trust-pointer.json)
        #[arg(short, long)]
        path: Option<PathBuf>,
        /// Channel identifier for canary rollout (e.g., "canary", "stable")
        ///
        /// When set, uses a channel-specific trust pointer file
        /// (e.g., /etc/icg/trust-pointer-canary.json instead of /etc/icg/trust-pointer.json).
        #[arg(long)]
        channel: Option<String>,
    },
}

fn load_rule_pack(path: PathBuf) -> Result<crate::rule_pack::Pack> {
    crate::rule_pack::load_pack(&path)
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Commands::CoverageDiff {
            previous,
            current,
            justification,
            previous_override,
            current_override,
        } => {
            let has_override = previous_override.is_some() || current_override.is_some();
            let has_regressions;
            if has_override {
                let diff = run_release_integrity_diff(
                    previous.clone(),
                    current.clone(),
                    previous_override,
                    current_override,
                )?;
                let report = render_release_integrity_report(
                    &previous,
                    &current,
                    &diff,
                    justification.as_deref(),
                );
                print!("{report}");
                has_regressions = diff.has_regressions();
            } else {
                let diff = run_coverage_diff(previous.clone(), current.clone())?;
                let report = render_coverage_diff_report(
                    &previous,
                    &current,
                    &diff,
                    justification.as_deref(),
                );
                print!("{report}");
                has_regressions = diff.has_regressions();
            }

            // A regression may be approved only when the report carries an
            // explicit, non-blank rationale. The report is printed first so
            // the missing field is still visible in CI output for Layer 2.
            if has_regressions
                && !CoverageDiff::has_explicit_justification(justification.as_deref())
            {
                eprintln!(
                    "Coverage regressions detected; rerun with --justification <rationale>."
                );
                std::process::exit(2);
            }

            Ok(())
        }
        Commands::RegressionSuite {
            manifest,
            output,
            override_file,
            repository,
            trusted_ref,
        } => {
            let suite = generate_regression_suite_from_manifest(&manifest)?;
            match (override_file, repository, trusted_ref) {
                (None, None, None) => {}
                (Some(path), Some(repository), Some(trusted_ref)) => {
                    let pack = load_rule_pack(manifest.clone())?;
                    verify_override_regression_gate(
                        &pack,
                        &suite,
                        &load_verified_override(
                            &path,
                            &repository,
                            &trusted_ref,
                            std::slice::from_ref(&pack),
                        )?,
                        &repository,
                        &trusted_ref,
                    )?;
                }
                _ => anyhow::bail!(
                    "--override-file, --repository, and --trusted-ref must be supplied together"
                ),
            }
            match output {
                Some(path) => {
                    write_regression_suite(&suite, &path)?;
                    eprintln!(
                        "Generated {} regression test cases at {}",
                        suite.cases.len(),
                        path.display()
                    );
                }
                None => println!("{}", suite.to_json()?),
            }
            Ok(())
        }
        Commands::Hook {
            rule_pack,
            override_file,
            repository,
            trusted_ref,
        } => {
            // Hook mode: read PreToolUse JSON from stdin, route to appropriate engine, and return decision
            let mut engine = Engine::new();

            // Rule-pack failures are deliberately swallowed by the engine. A
            // broken pack must allow this invocation, never wedge the hook.
            let pack_path = rule_pack.unwrap_or_else(|| PathBuf::from("/etc/icg/rule-pack.json"));
            if pack_path.exists() {
                engine.load_pack_from_file(&pack_path)?;
            }
            match (override_file, repository, trusted_ref) {
                (None, None, None) => {}
                (Some(path), Some(repository), Some(trusted_ref)) => {
                    engine.load_verified_override_from_file(&path, &repository, &trusted_ref)?;
                }
                _ => anyhow::bail!(
                    "--override-file, --repository, and --trusted-ref must be supplied together"
                ),
            }
            // If no pack exists, we'll fail-open (allow everything)

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
            TrustSubcommand::Show { path, channel } => {
                let store_path = if let Some(ref ch) = channel {
                    // Use channel-specific trust pointer for canary rollout
                    TrustPointerStore::for_channel(ch).path().to_path_buf()
                } else {
                    path.or_else(|| TrustPointerStore::default_path().ok())
                        .context("Failed to determine trust pointer path")?
                };
                let store = TrustPointerStore::new(store_path.clone());

                match store.load()? {
                    Some(pointer) => {
                        println!("# Trust Pointer Status");
                        if let Some(ref ch) = channel {
                            println!("**Channel:** `{}`", ch);
                        }
                        println!();
                        println!("**Path:** {}", store_path.display());
                        println!("**Trusted Reference:** `{}`", pointer.trusted_ref);
                        println!("**Last Updated:** {}", pointer.updated_at);
                        if let Some(justification) = pointer.justification {
                            println!("**Justification:** {}", justification);
                        }
                    }
                    None => {
                        if let Some(ref ch) = channel {
                            println!("No trust pointer exists for channel `{}`.", ch);
                        } else {
                            println!("No trust pointer exists yet.");
                        }
                        println!();
                        if let Some(ref ch) = channel {
                            println!("To set one, run:");
                            println!("  icg trust set --channel {} <reference>", ch);
                        } else {
                            println!("To set one, run:");
                            println!("  icg trust set <reference>");
                        }
                    }
                }

                Ok(())
            }
            TrustSubcommand::Set {
                trusted_ref,
                justification,
                path,
                channel,
            } => {
                let store_path = if let Some(ref ch) = channel {
                    // Use channel-specific trust pointer for canary rollout
                    TrustPointerStore::for_channel(ch).path().to_path_buf()
                } else {
                    path.or_else(|| TrustPointerStore::default_path().ok())
                        .context("Failed to determine trust pointer path")?
                };
                let store = TrustPointerStore::new(store_path.clone());

                if let Some(justification) = justification {
                    store.set_trusted_ref_with_justification(&trusted_ref, justification)?;
                } else {
                    store.set_trusted_ref(&trusted_ref)?;
                }

                if let Some(ref ch) = channel {
                    println!("✅ Trust pointer for channel `{}` updated to: `{}`", ch, trusted_ref);
                } else {
                    println!("✅ Trust pointer updated to: `{}`", trusted_ref);
                }

                Ok(())
            }
            TrustSubcommand::Check { reference, path, channel } => {
                let store_path = if let Some(ref ch) = channel {
                    // Use channel-specific trust pointer for canary rollout
                    TrustPointerStore::for_channel(ch).path().to_path_buf()
                } else {
                    path.or_else(|| TrustPointerStore::default_path().ok())
                        .context("Failed to determine trust pointer path")?
                };
                let store = TrustPointerStore::new(store_path.clone());

                let is_trusted = store.is_trusted(&reference)?;

                if is_trusted {
                    if let Some(ref ch) = channel {
                        println!("✅ Reference `{}` is trusted on channel `{}`.", reference, ch);
                    } else {
                        println!("✅ Reference `{}` is trusted.", reference);
                    }
                    std::process::exit(0);
                } else {
                    match store.get_trusted_ref()? {
                        Some(trusted) => {
                            if let Some(ref ch) = channel {
                                println!("❌ Reference `{}` is NOT trusted on channel `{}`.", reference, ch);
                            } else {
                                println!("❌ Reference `{}` is NOT trusted.", reference);
                            }
                            println!("Current trusted reference: `{}`", trusted);
                        }
                        None => {
                            if let Some(ref ch) = channel {
                                println!("❌ Reference `{}` is NOT trusted on channel `{}`.", reference, ch);
                                println!("No trust pointer exists for channel `{}` yet.", ch);
                            } else {
                                println!("❌ Reference `{}` is NOT trusted.", reference);
                                println!("No trust pointer exists yet.");
                            }
                        }
                    }
                    std::process::exit(1);
                }
            }
        },
        Commands::Update {
            trust_pointer_path,
            artifact_path,
            channel,
        } => {
            let mut config = UpdateConfig::default();

            // Set channel if specified
            if let Some(ref ch) = channel {
                config.channel = Some(ch.clone());
                config.trust_pointer_path = TrustPointerStore::for_channel(ch).path().to_path_buf();
            } else if let Some(trust_path) = trust_pointer_path {
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
        Commands::Status { trust_pointer_path, channel } => {
            let store_path = if let Some(ref ch) = channel {
                // Use channel-specific trust pointer for canary rollout
                TrustPointerStore::for_channel(ch).path().to_path_buf()
            } else {
                trust_pointer_path
                    .or_else(|| TrustPointerStore::default_path().ok())
                    .context("Failed to determine trust pointer path")?
            };
            let store = TrustPointerStore::new(store_path.clone());

            println!("# icg Status\n");

            // Channel info if applicable
            if let Some(ref ch) = channel {
                println!("**Channel:** `{}`", ch);
                println!();
            }

            // Trust Pointer section
            println!("## Trust Pointer");
            println!("  **Path:** {}", store_path.display());
            match store.load()? {
                Some(pointer) => {
                    println!("  **Reference:** `{}`", pointer.trusted_ref);
                    println!("  **Last Updated:** {}", pointer.updated_at);
                    if let Some(justification) = pointer.justification {
                        println!("  **Justification:** {}", justification);
                    }
                }
                None => {
                    println!("  (not configured)");
                    if channel.is_some() {
                        println!("  Run `icg update --channel {}` to initialize.", channel.as_ref().unwrap());
                    } else {
                        println!("  Run `icg trust set <reference>` to configure.");
                    }
                }
            }
            println!();

            // Rule Pack Version section
            println!("## Rule Pack Version");
            let artifact_path = PathBuf::from("/etc/icg/rule-pack.json");
            if artifact_path.exists() {
                match crate::rule_pack::load_pack(&artifact_path) {
                    Ok(pack) => {
                        println!("  **Pack ID:** `{}`", pack.id);
                        println!("  **Path:** {}", artifact_path.display());
                    }
                    Err(e) => {
                        println!("  (failed to load: {})", e);
                    }
                }
            } else {
                println!("  (no rule pack found at {})", artifact_path.display());
                if channel.is_some() {
                    println!("  Run `icg update --channel {}` to download the rule pack.", channel.as_ref().unwrap());
                } else {
                    println!("  Run `icg update` to download the rule pack.");
                }
            }
            println!();

            // Last Successful Update Check section
            println!("## Last Successful Update Check");
            let state_path = PathBuf::from("/etc/icg/last-update-check.json");
            match UpdateCheckState::load(&state_path)? {
                Some(state) => {
                    println!("  **Timestamp:** {}", state.last_successful_check);
                    println!("  **Release Tag:** {}", state.release_tag);
                    println!("  **Trusted Ref:** {}", state.trusted_ref);
                }
                None => {
                    println!("  (no successful update checks recorded)");
                    if channel.is_some() {
                        println!("  Run `icg update --channel {}` to check for and download updates.", channel.as_ref().unwrap());
                    } else {
                        println!("  Run `icg update` to check for and download updates.");
                    }
                }
            }
            println!();

            // Known Limitations (blind-spot self-report)
            println!("## Known Limitations");
            println!();
            println!("This tool has known blind spots and coverage gaps. These are documented");
            println!("explicitly to avoid overselling protection capability:");
            println!();

            let limitations = vec![
                ("Cloud-hosted Codex", "Checks run locally on the agent's host; cloud-hosted Claude Code/Claude.ai sessions bypass the wrapper entirely. Protection requires those environments to invoke their own PreToolUse hook integration."),
                ("Absolute-path wrapper bypass", "If the user invokes a binary by its absolute path (e.g., `/usr/bin/git` instead of `git`), the shell resolves it directly and the wrapper is not triggered. PATH-order shadowing is not a security boundary."),
                ("Content-mode coverage gaps", "Only YAML files are checked for storage-class violations. Other formats (JSON, manifests with explicit storageClassName references) are not yet covered. Image-tag enforcement similarly has format gaps."),
                ("State-dependent checks (Tier 2)", "Patterns requiring cross-invocation state (e.g., \"did a git pull happen earlier in this session\") are not yet implemented. State-store infrastructure exists but no checks use it yet."),
                ("Tier 3 context-dependent patterns", "Patterns that depend on invocation context (e.g., git worktree add, which is legitimate in some contexts and dangerous in others) are not reliably decidable from command syntax alone. These may never be fully covered; at most heuristic warnings."),
                ("Race conditions on self-edit", "If a rule pack update narrows a safe_pattern, a concurrent agent that already loaded the old pack may still write through the widened gap. Mitigation: periodic process restart or explicit reload signal, not yet implemented."),
                ("No coverage for aliased commands", "If a user creates a shell alias that shadows a guarded command (e.g., `alias k=kubectl`), the wrapper sees the alias name, not the resolved binary. Coverage depends on alias keywords matching, not the resolved executable."),
            ];

            for (i, (title, description)) in limitations.iter().enumerate() {
                println!("{}. **{}**", i + 1, title);
                println!("   {}", description);
                println!();
            }

            println!("This list is kept current. If you discover a new gap, report it as");
            println!("a coverage bug so the limitation can be documented or fixed.");

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
