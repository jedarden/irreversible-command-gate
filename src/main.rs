mod coverage;
mod documented_commands;
mod engine;
mod new_pack;
mod overrides;
mod regression;
mod rule_pack;
mod trust_pointer;
mod update;
mod value_derivation;

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
    /// Check a command, file, or PreToolUse request without executing it.
    Check(documented_commands::CheckArgs),
    /// Explain a rule-pack pattern or recorded denial.
    Explain(documented_commands::ExplainArgs),
    /// List the rule packs available to the local installation.
    Coverage(documented_commands::CoverageArgs),
    /// Create a privacy-conscious diagnostic report.
    BugReport(documented_commands::BugReportArgs),
    /// Create or verify a maintenance backup.
    #[command(subcommand)]
    Backup(documented_commands::BackupSubcommand),
    /// Create, approve, or list repository override artifacts.
    #[command(subcommand)]
    Override(documented_commands::OverrideSubcommand),
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
        /// Type of pack: "command" (shell commands) or "content" (file contents)
        #[arg(short, long, default_value = "command")]
        pack_type: String,
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

/// Render the native Codex/Claude PreToolUse response envelope. Both hook
/// protocols consume the hook-specific decision under `hookSpecificOutput`;
/// Codex additionally requires `hookEventName` to identify the event.
fn render_hook_response(
    result: engine::CheckResult,
    original_input: Option<&serde_json::Value>,
    updated_input_key: &str,
    context: Option<&str>,
) -> serde_json::Value {
    let details = |reason: &str, pack_id: &str, pattern_id: &str| {
        let suffix = context
            .map(|value| format!(", file={value}"))
            .unwrap_or_default();
        format!("{reason} [pack={pack_id}, pattern={pattern_id}{suffix}]")
    };

    let mut hook_output = serde_json::Map::new();
    hook_output.insert(
        "hookEventName".to_string(),
        serde_json::Value::String("PreToolUse".to_string()),
    );

    match result {
        engine::CheckResult::Allowed => {
            hook_output.insert(
                "permissionDecision".to_string(),
                serde_json::Value::String("allow".to_string()),
            );
            serde_json::json!({"hookSpecificOutput": hook_output})
        }
        engine::CheckResult::Denied {
            reason,
            pack_id,
            pattern_id,
        } => {
            hook_output.insert(
                "permissionDecision".to_string(),
                serde_json::Value::String("deny".to_string()),
            );
            hook_output.insert(
                "permissionDecisionReason".to_string(),
                serde_json::Value::String(details(&reason, &pack_id, &pattern_id)),
            );
            serde_json::json!({"hookSpecificOutput": hook_output})
        }
        engine::CheckResult::Rewrite {
            reason,
            rewrite,
            pack_id,
            pattern_id,
        } => {
            hook_output.insert(
                "permissionDecision".to_string(),
                serde_json::Value::String("allow".to_string()),
            );
            let mut updated_input = original_input
                .and_then(serde_json::Value::as_object)
                .cloned()
                .unwrap_or_default();
            updated_input.insert(
                updated_input_key.to_string(),
                serde_json::Value::String(rewrite),
            );
            hook_output.insert(
                "updatedInput".to_string(),
                serde_json::Value::Object(updated_input),
            );
            hook_output.insert(
                "additionalContext".to_string(),
                serde_json::Value::String(details(&reason, &pack_id, &pattern_id)),
            );
            serde_json::json!({"hookSpecificOutput": hook_output})
        }
        engine::CheckResult::Warning {
            reason,
            pack_id,
            pattern_id,
        } => {
            hook_output.insert(
                "permissionDecision".to_string(),
                serde_json::Value::String("allow".to_string()),
            );
            hook_output.insert(
                "additionalContext".to_string(),
                serde_json::Value::String(details(&reason, &pack_id, &pattern_id)),
            );
            serde_json::json!({"hookSpecificOutput": hook_output})
        }
    }
}

fn updated_input_key(
    tool_name: Option<&str>,
    original_input: Option<&serde_json::Value>,
) -> &'static str {
    match tool_name {
        Some("Write") => "content",
        Some("Edit") => {
            if original_input
                .and_then(serde_json::Value::as_object)
                .is_some_and(|input| input.contains_key("new_string"))
            {
                "new_string"
            } else {
                "newString"
            }
        }
        _ => "command",
    }
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Commands::Check(args) => documented_commands::run_check(args),
        Commands::Explain(args) => documented_commands::run_explain(args),
        Commands::Coverage(args) => documented_commands::run_coverage(args),
        Commands::BugReport(args) => documented_commands::run_bug_report(args),
        Commands::Backup(command) => documented_commands::run_backup(command),
        Commands::Override(command) => documented_commands::run_override(command),
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

            // Retain the original tool input so an updatedInput response can
            // replace one field without dropping the other tool arguments.
            let hook_payload = engine.read_pre_tool_use_payload_from_stdin()?;
            let (hook_input, original_input) = match hook_payload {
                Some((input, original_input)) => (Some(input), Some(original_input)),
                None => (None, None),
            };
            let input_key = updated_input_key(
                hook_input.as_ref().map(|input| input.tool_name.as_str()),
                original_input.as_ref(),
            );
            let input_source = match hook_input {
                Some(input) => match Engine::input_source_from_pre_tool_use(input) {
                    Ok(source) => source,
                    Err(error) => {
                        eprintln!("Engine: invalid hook input: {error}");
                        None
                    }
                },
                None => None,
            };

            // Read input from stdin (either command-mode or content-mode)
            match input_source {
                Some(InputSource::Command(source)) => {
                    // Command-mode: Bash command
                    let result = engine.evaluate_command(&source);
                    println!(
                        "{}",
                        render_hook_response(result, original_input.as_ref(), input_key, None)
                    );

                    Ok(())
                }
                Some(InputSource::Content(content)) => {
                    // Content-mode: Write/Edit operation
                    // Evaluate against content-mode packs (storage-class, image-tag, beads)
                    let result = engine.evaluate_content(&content);
                    println!(
                        "{}",
                        render_hook_response(
                            result,
                            original_input.as_ref(),
                            input_key,
                            Some(content.file_path()),
                        )
                    );

                    Ok(())
                }
                Some(InputSource::ContentBatch(contents)) => {
                    let result = engine.evaluate_content_batch(&contents);
                    let files = contents
                        .iter()
                        .map(|content| content.file_path())
                        .collect::<Vec<_>>()
                        .join(",");
                    println!(
                        "{}",
                        render_hook_response(
                            result,
                            original_input.as_ref(),
                            input_key,
                            Some(&files),
                        )
                    );

                    Ok(())
                }
                None => {
                    // Unrecognized tool - allow by default (fail-open)
                    println!(
                        "{}",
                        render_hook_response(
                            engine::CheckResult::Allowed,
                            original_input.as_ref(),
                            input_key,
                            None,
                        )
                    );
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
        Commands::NewPack {
            pack_name,
            pack_type,
            output_dir,
        } => {
            let dest = output_dir.unwrap_or_else(|| PathBuf::from("."));

            println!("# icg new-pack");
            println!();
            println!("Pack name: `{}`", pack_name);
            println!("Pack type: `{}`", pack_type);
            println!("Output directory: {}", dest.display());
            println!();
            match new_pack::generate_pack_scaffolding(&pack_name, &pack_type, &dest) {
                Ok((pack_path, test_path)) => {
                    println!("✓ Pack scaffold created: {}", pack_path.display());
                    println!("✓ Test stub created: {}", test_path.display());
                    println!();
                    println!("Next steps:");
                    println!("  1. Edit the pack file to add your specific patterns");
                    println!("  2. Implement the test cases in the test file");
                    println!("  3. Run `cargo test` to verify your implementation");
                    Ok(())
                }
                Err(e) => {
                    println!("✗ Failed to generate scaffolding: {}", e);
                    Err(e)
                }
            }
        }
    }
}
