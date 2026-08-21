mod coverage;
mod documented_commands;
mod engine;
mod health;
mod new_pack;
mod overrides;
mod regex_safety;
mod regression;
mod rule_pack;
mod state_store;
mod telemetry;
mod trust_pointer;
mod update;
mod value_derivation;

use anyhow::Context;
use anyhow::Result;
use clap::{Args, Parser, Subcommand};
use coverage::*;
use engine::{Engine, InputSource};
use overrides::*;
use regex_safety::{check_pack_for_redos, RedosConfig};
use regression::{generate_regression_suite_from_manifest, write_regression_suite};
use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::process::Command;
use trust_pointer::*;
use update::*;

#[derive(Parser)]
#[command(name = "icg", version = env!("CARGO_PKG_VERSION"))]
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
        /// Report available updates without downloading or installing one.
        #[arg(long)]
        check_only: bool,
    },
    /// Show current status and blind-spot self-report
    Status(documented_commands::StatusArgs),
    /// Export one denial record for incident or false-positive review.
    ExportDenial(documented_commands::ExportDenialArgs),
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
    /// Check rule pack for ReDoS (catastrophic backtracking) vulnerabilities
    RedosCheck {
        /// Path to the rule-pack JSON manifest
        manifest: PathBuf,
        /// Timeout per regex test in milliseconds (default: 100)
        #[arg(short, long, default_value = "100")]
        timeout_ms: u64,
        /// Skip dynamic fuzzing (only run static analysis)
        #[arg(long)]
        skip_dynamic: bool,
        /// Skip static analysis (only run dynamic fuzzing)
        #[arg(long)]
        skip_static: bool,
    },
    /// Telemetry management (rolling baseline monitoring and auto-rollback)
    #[command(subcommand)]
    Telemetry(TelemetrySubcommand),
    /// Health monitoring and crash tracking
    Health {
        #[command(flatten)]
        args: HealthArgs,
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
    /// Install PATH-wrapper symlinks for currently-loaded command-mode packs
    Install {
        /// Installation directory for symlinks (defaults to ~/.local/bin)
        #[arg(short, long)]
        dir: Option<PathBuf>,
        /// Rule-pack file(s) or directory to load for tool keyword discovery
        #[arg(long = "pack", alias = "rule-pack")]
        packs: Vec<PathBuf>,
        /// Force installation without confirmation
        #[arg(long)]
        force: bool,
        /// Remove existing symlinks instead of creating them
        #[arg(long)]
        uninstall: bool,
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

#[derive(Subcommand)]
enum TelemetrySubcommand {
    /// Show current telemetry status and baseline statistics
    Status {
        /// Path to telemetry file (defaults to /var/cache/icg/telemetry.json)
        #[arg(short, long)]
        path: Option<PathBuf>,
    },
    /// Reset all telemetry data (clear baseline history)
    Reset {
        /// Path to telemetry file (defaults to /var/cache/icg/telemetry.json)
        #[arg(short, long)]
        path: Option<PathBuf>,
        /// Skip confirmation prompt
        #[arg(long)]
        force: bool,
    },
    /// Configure telemetry thresholds and settings
    Configure {
        /// Path to telemetry file (defaults to /var/cache/icg/telemetry.json)
        #[arg(short, long)]
        path: Option<PathBuf>,
        /// Window size for rolling baseline (number of evaluations)
        #[arg(long)]
        window_size: Option<usize>,
        /// Spike threshold multiplier for anomaly detection
        #[arg(long)]
        spike_threshold: Option<f64>,
        /// Minimum samples before baseline is considered valid
        #[arg(long)]
        minimum_samples: Option<usize>,
        /// Rollback cooldown period in seconds
        #[arg(long)]
        cooldown_seconds: Option<u64>,
        /// Enable or disable automatic rollback
        #[arg(long)]
        auto_rollback: Option<bool>,
    },
}

#[derive(Subcommand)]
enum HealthSubcommand {
    /// Show current health status and crash metrics
    Status {
        /// Path to health state file (defaults to system cache directory)
        #[arg(short, long)]
        path: Option<PathBuf>,
    },
    /// Reset all health data (clear crash history)
    Reset {
        /// Path to health state file (defaults to system cache directory)
        #[arg(short, long)]
        path: Option<PathBuf>,
        /// Skip confirmation prompt
        #[arg(long)]
        force: bool,
    },
    /// Mark that the process has started (called by guard on startup)
    MarkStart {
        /// Path to health state file (defaults to system cache directory)
        #[arg(short, long)]
        path: Option<PathBuf>,
    },
    /// Mark that the process exited cleanly (called by guard on shutdown)
    MarkCleanExit {
        /// Path to health state file (defaults to system cache directory)
        #[arg(short, long)]
        path: Option<PathBuf>,
    },
    /// Record a crash event (called by watchdog or crash handler)
    RecordCrash {
        /// Path to health state file (defaults to system cache directory)
        #[arg(short, long)]
        path: Option<PathBuf>,
        /// Crash type (segfault, abort, oom, timeout, etc.)
        #[arg(long)]
        crash_type: String,
        /// Optional signal number (Unix only)
        #[arg(long)]
        signal: Option<i32>,
        /// Optional exit code
        #[arg(long)]
        exit_code: Option<i32>,
        /// Optional context about the crash
        #[arg(long)]
        context: Option<String>,
    },
}

#[derive(Args)]
struct HealthArgs {
    /// The detailed health-state operation to run.
    #[command(subcommand)]
    subcommand: Option<HealthSubcommand>,

    /// Validate every rule pack in the configured pack directory.
    #[arg(long)]
    check_packs: bool,

    /// Validate the configured Claude Code hook.
    #[arg(long)]
    check_hooks: bool,

    /// Print the complete operator health inventory.
    #[arg(long)]
    verbose: bool,
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

/// Check for anomalies and handle automatic rollback if needed
///
/// This function processes telemetry results after each evaluation and
/// triggers automatic rollback if a deny-rate spike is detected.
fn check_and_handle_anomaly(
    telemetry_store: &std::sync::Arc<std::sync::Mutex<telemetry::TelemetryStore>>,
    trust_store_path: &PathBuf,
) -> Result<()> {
    // Lock the telemetry store and process results
    let mut store = telemetry_store.lock().map_err(|e| {
        anyhow::anyhow!("Failed to lock telemetry store: {}", e)
    })?;

    // Create trust pointer store for rollback operations
    let trust_store = TrustPointerStore::new(trust_store_path.clone());

    // Process evaluation results and check for anomalies
    match telemetry::process_evaluation_results(&mut store, &trust_store) {
        Ok(Some(anomaly_report)) => {
            // Anomaly was detected
            eprintln!("🚨 Anomaly detected in deny-rate monitoring");
            eprintln!("   Severity: {:?}", anomaly_report.severity);
            eprintln!("   Current deny rate: {:.2}%", anomaly_report.current_deny_rate * 100.0);
            eprintln!("   Baseline mean: {:.2}%", anomaly_report.baseline.mean * 100.0);
            eprintln!("   Threshold: {:.2}%", anomaly_report.baseline.anomaly_threshold(store.config().spike_threshold) * 100.0);

            if anomaly_report.rollback_triggered {
                eprintln!("   ✅ Automatic rollback triggered");
                eprintln!("   Rolled back from: {}", anomaly_report.rolled_back_release.as_deref().unwrap_or("unknown"));
                eprintln!("   Rolled back to: {}", anomaly_report.previous_release.as_deref().unwrap_or("unknown"));
            } else {
                eprintln!("   ℹ️  Rollback not triggered (disabled or on cooldown)");
            }
        }
        Ok(None) => {
            // No anomaly - continue normally
        }
        Err(e) => {
            // Log error but don't fail the hook - telemetry failures should be non-blocking
            eprintln!("⚠️  Telemetry processing error: {}", e);
        }
    }

    // Persist telemetry state
    if let Err(e) = store.persist() {
        eprintln!("⚠️  Failed to persist telemetry state: {}", e);
    }

    Ok(())
}

const DEFAULT_RULE_PACK_PATH: &str = "/etc/icg/rule-pack.json";
const DEFAULT_RULE_PACK_DIR: &str = "/etc/icg/packs";

/// Return the tool name when `icg` was invoked through a PATH-wrapper symlink.
///
/// Administrative invocations use the basename `icg` and continue through
/// Clap's normal subcommand parser. Any other basename is a wrapped tool name;
/// this check must happen before Clap sees the tool's own arguments.
fn shadowed_tool_name(argv0: &OsStr) -> Option<String> {
    let name = Path::new(argv0).file_name()?.to_str()?;
    let name = name.strip_suffix(".exe").unwrap_or(name);
    (name != "icg").then(|| name.to_string())
}

fn wrapper_rule_pack_path() -> Option<PathBuf> {
    std::env::var_os("ICG_RULE_PACK")
        .filter(|path| !path.is_empty())
        .map(PathBuf::from)
        .or_else(|| {
            let artifact = PathBuf::from(DEFAULT_RULE_PACK_PATH);
            artifact.is_file().then_some(artifact)
        })
        .or_else(|| {
            let directory = PathBuf::from(DEFAULT_RULE_PACK_DIR);
            directory.is_dir().then_some(directory)
        })
}

fn load_wrapper_engine() -> Result<Engine> {
    let mut engine = Engine::new();

    if let Some(path) = wrapper_rule_pack_path() {
        if path.is_dir() {
            engine.load_packs_from_dir(path)?;
        } else {
            engine.load_pack_from_file(path)?;
        }
    }

    // The wrapper is a separate process for every command, so attach the
    // durable state store here as well as in hook mode. Telemetry is best
    // effort; inability to open the cache must not prevent normal commands
    // from reaching their real binary.
    if let Ok(state_path) = state_store::StateStore::default_path() {
        engine = engine.with_state_store(std::sync::Arc::new(
            state_store::StateStore::new(state_path),
        ));
    }
    if let Ok(trust_path) = TrustPointerStore::default_path() {
        if let Ok(Some(pointer)) = TrustPointerStore::new(trust_path).load() {
            engine = engine.with_release_ref(pointer.trusted_ref);
        }
    }

    Ok(engine)
}

/// Find the first executable named `tool` in PATH that is not this wrapper.
///
/// Comparing canonical paths is important when PATH contains several icg
/// symlinks: skipping only the first entry would let a second wrapper recurse.
fn real_binary_in_path(tool: &str, argv0: &OsStr) -> Result<PathBuf> {
    let current_exe = std::env::current_exe().ok();
    let invoked_path = Path::new(argv0);
    let invoked_path = if invoked_path.components().count() > 1 {
        std::fs::canonicalize(invoked_path).ok()
    } else {
        None
    };
    let path = std::env::var_os("PATH").context("PATH is not set; cannot locate the real binary")?;

    for directory in std::env::split_paths(&path) {
        let candidate = if directory.as_os_str().is_empty() {
            PathBuf::from(tool)
        } else {
            directory.join(tool)
        };

        let metadata = match std::fs::metadata(&candidate) {
            Ok(metadata) if metadata.is_file() => metadata,
            _ => continue,
        };

        #[cfg(unix)]
        if metadata.permissions().mode() & 0o111 == 0 {
            continue;
        }

        let canonical_candidate = std::fs::canonicalize(&candidate).ok();
        if current_exe
            .as_ref()
            .and_then(|path| std::fs::canonicalize(path).ok())
            .as_ref()
            == canonical_candidate.as_ref()
            || invoked_path.as_ref() == canonical_candidate.as_ref()
        {
            continue;
        }

        return Ok(candidate);
    }

    anyhow::bail!(
        "could not find the real `{tool}` binary after skipping icg wrapper entries in PATH"
    )
}

#[cfg(unix)]
fn rewritten_wrapper_args(tool: &str, rewrite: &str) -> Result<Vec<OsString>> {
    let parser = Engine::new();
    let source = engine::CommandSource::Hook(rewrite.to_string());
    let tokens = parser.segment_command(&source);
    let token = tokens
        .first()
        .filter(|token| token.executable == tool && tokens.len() == 1)
        .context("rule rewrite did not produce one command for the wrapped tool")?;

    Ok(token.args.iter().map(OsString::from).collect())
}

#[cfg(unix)]
fn run_shadowed_tool(argv0: &OsStr, tool: &str, original_args: &[OsString]) -> Result<()> {
    use std::os::unix::process::CommandExt;

    let engine = load_wrapper_engine()?;
    let mut check_argv = Vec::with_capacity(original_args.len() + 1);
    check_argv.push(tool.to_string());
    check_argv.extend(
        original_args
            .iter()
            .map(|arg| arg.to_string_lossy().into_owned()),
    );
    let source = engine.read_from_argv(check_argv);
    let result = engine.evaluate_command(&source);

    let exec_args = match result {
        engine::CheckResult::Allowed => original_args.to_vec(),
        engine::CheckResult::Warning {
            reason,
            pack_id,
            pattern_id,
        } => {
            eprintln!(
                "icg warning: {reason} [pack={pack_id}, pattern={pattern_id}]"
            );
            original_args.to_vec()
        }
        engine::CheckResult::Rewrite {
            reason,
            rewrite,
            pack_id,
            pattern_id,
        } => {
            eprintln!(
                "icg updated command: {reason} [pack={pack_id}, pattern={pattern_id}]"
            );
            rewritten_wrapper_args(tool, &rewrite)?
        }
        engine::CheckResult::Denied {
            reason,
            pack_id,
            pattern_id,
        } => {
            anyhow::bail!(
                "command denied: {reason} [pack={pack_id}, pattern={pattern_id}]"
            )
        }
    };

    let real_binary = real_binary_in_path(tool, argv0)?;
    let error = Command::new(&real_binary)
        .arg0(argv0)
        .args(&exec_args)
        .exec();
    anyhow::bail!(
        "failed to exec real `{tool}` binary `{}`: {error}",
        real_binary.display()
    )
}

#[cfg(not(unix))]
fn run_shadowed_tool(_argv0: &OsStr, _tool: &str, _original_args: &[OsString]) -> Result<()> {
    anyhow::bail!("PATH-wrapper mode is only supported on Unix platforms")
}

fn main() -> Result<()> {
    let mut argv = std::env::args_os();
    let argv0 = argv.next().context("process did not provide argv[0]")?;
    let original_args: Vec<OsString> = argv.collect();

    if let Some(tool) = shadowed_tool_name(&argv0) {
        return run_shadowed_tool(&argv0, &tool, &original_args);
    }

    let cli = Cli::parse_from(
        std::iter::once(argv0.clone()).chain(original_args.iter().cloned()),
    );

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
                    println!(
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

            // Load rule packs from the default path
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

            // Initialize telemetry store for rolling baseline monitoring
            let telemetry_path = PathBuf::from("/var/cache/icg/telemetry.json");
            let telemetry_store = std::sync::Arc::new(std::sync::Mutex::new(
                telemetry::TelemetryStore::load_or_create(telemetry_path)?
            ));

            // Generate session ID for this evaluation
            let session_id = format!("session-{}", uuid::Uuid::new_v4());

            // Get the current release reference from the trust pointer if available
            let trust_store_path = TrustPointerStore::default_path()?;
            let release_ref = if let Ok(Some(pointer)) = TrustPointerStore::new(&trust_store_path).load() {
                Some(pointer.trusted_ref)
            } else {
                None
            };

            // Configure engine with telemetry
            engine = engine
                .with_telemetry_store(telemetry_store.clone())
                .with_session_id(session_id.clone())
                .with_release_ref(release_ref.unwrap_or_else(|| "unknown".to_string()));
            if let Ok(state_path) = state_store::StateStore::default_path() {
                engine = engine.with_state_store(std::sync::Arc::new(
                    state_store::StateStore::new(state_path),
                ));
            }

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

                    // Check for anomalies and trigger rollback if needed
                    check_and_handle_anomaly(&telemetry_store, &trust_store_path)?;

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

                    // Check for anomalies and trigger rollback if needed
                    check_and_handle_anomaly(&telemetry_store, &trust_store_path)?;

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

                    // Check for anomalies and trigger rollback if needed
                    check_and_handle_anomaly(&telemetry_store, &trust_store_path)?;

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
            let (tool, args) = args
                .split_first()
                .context("wrapper mode requires the shadowed tool name")?;
            let argv0 = OsString::from(tool);
            let args = args.iter().map(OsString::from).collect::<Vec<_>>();
            run_shadowed_tool(&argv0, tool, &args)
        }
        Commands::Install {
            dir,
            packs,
            force,
            uninstall,
        } => documented_commands::run_install(dir, packs, force, uninstall),
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
                    println!(
                        "✅ Trust pointer for channel `{}` updated to: `{}`",
                        ch, trusted_ref
                    );
                } else {
                    println!("✅ Trust pointer updated to: `{}`", trusted_ref);
                }

                Ok(())
            }
            TrustSubcommand::Check {
                reference,
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

                let is_trusted = store.is_trusted(&reference)?;

                if is_trusted {
                    if let Some(ref ch) = channel {
                        println!(
                            "✅ Reference `{}` is trusted on channel `{}`.",
                            reference, ch
                        );
                    } else {
                        println!("✅ Reference `{}` is trusted.", reference);
                    }
                    std::process::exit(0);
                } else {
                    match store.get_trusted_ref()? {
                        Some(trusted) => {
                            if let Some(ref ch) = channel {
                                println!(
                                    "❌ Reference `{}` is NOT trusted on channel `{}`.",
                                    reference, ch
                                );
                            } else {
                                println!("❌ Reference `{}` is NOT trusted.", reference);
                            }
                            println!("Current trusted reference: `{}`", trusted);
                        }
                        None => {
                            if let Some(ref ch) = channel {
                                println!(
                                    "❌ Reference `{}` is NOT trusted on channel `{}`.",
                                    reference, ch
                                );
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
            check_only,
        } => {
            if check_only {
                return documented_commands::run_update_check();
            }
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

            let result = run_update(config).context("Failed to run update")?;

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
        Commands::Status(args) => {
            if args.denials || args.health || args.pattern_summary || args.trend || args.format.is_some() {
                return documented_commands::run_operator_status(&args);
            }
            let documented_commands::StatusArgs {
                trust_pointer_path,
                channel,
                ..
            } = args;
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
                        println!(
                            "  Run `icg update --channel {}` to initialize.",
                            channel.as_ref().unwrap()
                        );
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
                    println!(
                        "  Run `icg update --channel {}` to download the rule pack.",
                        channel.as_ref().unwrap()
                    );
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
                        println!(
                            "  Run `icg update --channel {}` to check for and download updates.",
                            channel.as_ref().unwrap()
                        );
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
        Commands::ExportDenial(args) => documented_commands::run_export_denial(&args),
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
        Commands::Telemetry(subcommand) => match subcommand {
            TelemetrySubcommand::Status { path } => {
                let telemetry_path = path.unwrap_or_else(|| PathBuf::from("/var/cache/icg/telemetry.json"));
                let store = telemetry::TelemetryStore::load_or_create(telemetry_path)?;

                println!("# Telemetry Status\n");
                println!("**Path:** {}", store.path().display());
                println!("**Window Size:** {}", store.config().window_size);
                println!("**Spike Threshold:** {:.2}x", store.config().spike_threshold);
                println!("**Minimum Samples:** {}", store.config().minimum_samples);
                println!("**Rollback Cooldown:** {:?}", store.config().rollback_cooldown);
                println!("**Auto-Rollback:** {}", if store.config().auto_rollback_enabled { "enabled" } else { "disabled" });
                println!();

                let window = store.window();
                let baseline = telemetry::compute_baseline(window);

                println!("## Current Baseline");
                println!("**Total Evaluations:** {}", baseline.total_evaluations);
                println!("**Deny Count:** {}", baseline.deny_count);
                println!("**Deny Rate:** {:.2}%", baseline.deny_rate * 100.0);
                println!("**Mean:** {:.2}%", baseline.mean * 100.0);
                println!("**Std Dev:** {:.4}", baseline.std_dev);
                println!("**Min:** {:.2}%", baseline.min * 100.0);
                println!("**Max:** {:.2}%", baseline.max * 100.0);

                if let Some(start) = baseline.window_start {
                    println!("**Window Start:** {}", start);
                }
                if let Some(end) = baseline.window_end {
                    println!("**Window End:** {}", end);
                }

                println!();
                println!("## Rollback Status");
                if store.is_rollback_on_cooldown() {
                    println!("**Status:** On cooldown");
                } else {
                    println!("**Status:** Ready to rollback");
                }

                Ok(())
            }
            TelemetrySubcommand::Reset { path, force } => {
                let telemetry_path = path.unwrap_or_else(|| PathBuf::from("/var/cache/icg/telemetry.json"));

                if !force {
                    println!("⚠️  This will clear all telemetry data and reset the baseline.");
                    println!("Type 'yes' to confirm:");
                    let mut confirmation = String::new();
                    std::io::stdin().read_line(&mut confirmation)?;
                    if confirmation.trim() != "yes" {
                        println!("Reset cancelled.");
                        return Ok(());
                    }
                }

                let mut store = telemetry::TelemetryStore::load_or_create(telemetry_path)?;
                store.clear();
                store.persist()?;

                println!("✅ Telemetry data cleared successfully.");

                Ok(())
            }
            TelemetrySubcommand::Configure {
                path,
                window_size,
                spike_threshold,
                minimum_samples,
                cooldown_seconds,
                auto_rollback,
            } => {
                let telemetry_path = path.unwrap_or_else(|| PathBuf::from("/var/cache/icg/telemetry.json"));
                let mut store = telemetry::TelemetryStore::load_or_create(telemetry_path)?;

                // Update configuration with provided values
                let mut config = store.config().clone();

                if let Some(size) = window_size {
                    println!("Setting window size to {}", size);
                    config.window_size = size;
                }
                if let Some(threshold) = spike_threshold {
                    println!("Setting spike threshold to {:.2}x", threshold);
                    config.spike_threshold = threshold;
                }
                if let Some(samples) = minimum_samples {
                    println!("Setting minimum samples to {}", samples);
                    config.minimum_samples = samples;
                }
                if let Some(seconds) = cooldown_seconds {
                    println!("Setting rollback cooldown to {} seconds", seconds);
                    config.rollback_cooldown = std::time::Duration::from_secs(seconds);
                }
                if let Some(enabled) = auto_rollback {
                    println!("Setting auto-rollback to {}", if enabled { "enabled" } else { "disabled" });
                    config.auto_rollback_enabled = enabled;
                }

                // Create new store with updated configuration
                let mut new_store = telemetry::TelemetryStore::with_config(
                    store.path().to_path_buf(),
                    config,
                );

                // Copy existing window data to new store
                let window_records = store.window().records().to_vec();
                for record in window_records {
                    new_store.record_evaluation(
                        record.verdict,
                        record.release_ref,
                        record.session_id,
                    );
                }

                // Persist the updated configuration
                new_store.persist()?;

                println!("✅ Telemetry configuration updated successfully.");

                Ok(())
            }
        },
        Commands::RedosCheck {
            manifest,
            timeout_ms,
            skip_dynamic,
            skip_static,
        } => {
            println!("# ReDoS Check for Rule Pack\n");
            println!("**Manifest:** {}", manifest.display());
            println!();

            let pack = load_rule_pack(manifest.clone()).with_context(|| {
                format!("Failed to load rule pack from: {}", manifest.display())
            })?;

            println!("**Pack ID:** `{}`", pack.id);
            println!();

            let config = RedosConfig {
                timeout_per_test: std::time::Duration::from_millis(timeout_ms),
                run_dynamic_tests: !skip_dynamic,
                run_static_analysis: !skip_static,
            };

            let report =
                check_pack_for_redos(&pack, &config).context("Failed to check pack for ReDoS")?;

            println!("## Results\n");
            println!("**Total patterns checked:** {}", report.total_patterns);
            println!(
                "**Unsafe patterns found:** {}",
                report.unsafe_patterns.len()
            );
            println!();

            if report.unsafe_patterns.is_empty() {
                println!("✅ **PASS:** All patterns passed ReDoS checks.\n");
                println!("The rule pack is safe from catastrophic backtracking vulnerabilities.");
                Ok(())
            } else {
                println!(
                    "❌ **FAIL:** Found {} unsafe pattern(s).\n",
                    report.unsafe_patterns.len()
                );

                for (i, unsafe_pattern) in report.unsafe_patterns.iter().enumerate() {
                    println!("### {}. Pattern: `{}`", i + 1, unsafe_pattern.pattern_id);
                    println!("**Check type:** {}", unsafe_pattern.check_type);
                    println!("**Reason:** {}", unsafe_pattern.reason);
                    println!();
                    println!("```regex");
                    println!("{}", unsafe_pattern.regex);
                    println!("```");
                    println!();

                    if let Some(findings) = &unsafe_pattern.static_findings {
                        println!("**Static analysis findings:** {}", findings);
                        println!();
                    }

                    if let Some(findings) = &unsafe_pattern.dynamic_findings {
                        println!("**Dynamic fuzzing results:** {}", findings);
                        println!();
                    }
                }

                println!("## Fixing ReDoS Vulnerabilities\n");
                println!("Common fixes:");
                println!("1. **Remove nested quantifiers:** Replace `(a+)+` with `a+`");
                println!("2. **Simplify alternation:** Replace `(a|a)+` with `a+`");
                println!(
                    "3. **Avoid overlapping patterns:** Use character classes instead: `[ab]+`"
                );
                println!("4. **Add anchors:** Use `^` and `$` to limit matching scope");
                println!(
                    "5. **Use possessive quantifiers:** Replace `.*` with `.*+` (if supported)"
                );
                println!();
                println!("For more information, see:");
                println!("- https://owasp.org/www-community/attacks/Regular_expression_Denial_of_Service_-_ReDoS");
                println!("- https://www.regular-expressions.info/redos.html");

                std::process::exit(1);
            }
        }
        Commands::Health { args } => {
            if args.check_packs || args.check_hooks || args.verbose || args.subcommand.is_none() {
                return documented_commands::run_health_report(
                    args.check_packs,
                    args.check_hooks,
                    args.verbose || args.subcommand.is_none(),
                );
            }
            let subcommand = args
                .subcommand
                .context("a health subcommand or health check flag is required")?;
            match subcommand {
            HealthSubcommand::Status { path } => {
                let health_path = path.unwrap_or_else(|| {
                    health::HealthStore::default_path()
                        .unwrap_or_else(|_| PathBuf::from("/var/cache/icg/health-state.json"))
                });
                let store = health::HealthStore::new(health_path);
                let metrics = store.health_metrics()?;

                println!("# Guard Health Status\n");
                println!("**Path:** {}", store.path().display());
                println!();

                println!("## Health Status");
                println!("**Status:** {:?}", metrics.status);
                println!("**Running:** {}", metrics.status.is_running());
                println!("**Healthy:** {}", metrics.status.is_healthy());
                println!("**Stable:** {}", metrics.is_stable);
                println!();

                println!("## Crash Metrics");
                println!("**Total Crashes:** {}", metrics.total_crashes);
                println!("**Recent Crashes (1h):** {}", metrics.recent_crashes);
                println!("**Crash Rate:** {:.2} crashes/hour", metrics.crash_rate);
                if let Some(last_crash) = metrics.last_crash_at {
                    println!("**Last Crash:** {}", last_crash);
                } else {
                    println!("**Last Crash:** (none)");
                }
                println!();

                println!("## Uptime & Stability");
                println!("**Consecutive Clean Runs:** {}", metrics.consecutive_clean_runs);
                if let Some(uptime) = metrics.current_uptime {
                    println!("**Current Uptime:** {:?}", uptime);
                } else {
                    println!("**Current Uptime:** (not running)");
                }
                if let Some(stable_time) = metrics.time_since_stable {
                    println!("**Time Since Stable:** {:?}", stable_time);
                }
                if let Some(last_start) = metrics.last_start_at {
                    println!("**Last Start:** {}", last_start);
                }

                Ok(())
            }
            HealthSubcommand::Reset { path, force } => {
                let health_path = path.unwrap_or_else(|| {
                    health::HealthStore::default_path()
                        .unwrap_or_else(|_| PathBuf::from("/var/cache/icg/health-state.json"))
                });

                if !force {
                    println!("⚠️  This will clear all health data and crash history.");
                    println!("Type 'yes' to confirm:");
                    let mut confirmation = String::new();
                    std::io::stdin().read_line(&mut confirmation)?;
                    if confirmation.trim() != "yes" {
                        println!("Reset cancelled.");
                        return Ok(());
                    }
                }

                let store = health::HealthStore::new(health_path);
                store.clear()?;

                println!("✅ Health data cleared successfully.");

                Ok(())
            }
            HealthSubcommand::MarkStart { path } => {
                let health_path = path.unwrap_or_else(|| {
                    health::HealthStore::default_path()
                        .unwrap_or_else(|_| PathBuf::from("/var/cache/icg/health-state.json"))
                });
                let store = health::HealthStore::new(health_path);
                store.mark_start()?;

                println!("✅ Process start marked successfully.");

                Ok(())
            }
            HealthSubcommand::MarkCleanExit { path } => {
                let health_path = path.unwrap_or_else(|| {
                    health::HealthStore::default_path()
                        .unwrap_or_else(|_| PathBuf::from("/var/cache/icg/health-state.json"))
                });
                let store = health::HealthStore::new(health_path);
                store.mark_clean_exit()?;

                println!("✅ Clean exit marked successfully.");

                Ok(())
            }
            HealthSubcommand::RecordCrash {
                path,
                crash_type,
                signal,
                exit_code,
                context,
            } => {
                let health_path = path.unwrap_or_else(|| {
                    health::HealthStore::default_path()
                        .unwrap_or_else(|_| PathBuf::from("/var/cache/icg/health-state.json"))
                });

                // Parse crash type from string
                let crash_type_enum = match crash_type.to_lowercase().as_str() {
                    "segfault" | "sigsegv" => health::CrashType::SegmentationFault,
                    "abort" | "sigabrt" => health::CrashType::Abort,
                    "bus" | "sigbus" => health::CrashType::BusError,
                    "fpe" | "sigfpe" => health::CrashType::FloatingPointException,
                    "oom" | "outofmemory" => health::CrashType::OutOfMemory,
                    "timeout" => health::CrashType::Timeout,
                    "panic" => health::CrashType::Panic,
                    "exit" | "exitcode" => health::CrashType::ExitCodeError,
                    _ => health::CrashType::Unknown,
                };

                let mut crash_record = health::CrashRecord::new(crash_type_enum);

                if let Some(sig) = signal {
                    crash_record = crash_record.with_signal(sig);
                }

                if let Some(code) = exit_code {
                    crash_record = crash_record.with_exit_code(code);
                }

                if let Some(ctx) = context {
                    crash_record = crash_record.with_context(ctx);
                }

                let store = health::HealthStore::new(health_path);
                store.record_crash(crash_record)?;

                println!("✅ Crash recorded successfully.");

                Ok(())
            }
        }
        },
    }
}
