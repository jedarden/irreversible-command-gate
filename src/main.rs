mod documented_commands;

use anyhow::Context;
use anyhow::Result;
use clap::{Args, Parser, Subcommand};
use coverage::*;
use engine::{Engine, InputSource};
use fail_closed::{PolicyStore, PolicyTransition, ReconcileOutcome};
use icg::{
    coverage, denial_log, emergency_bypass, engine, fail_closed, health, health_server, monitoring,
    new_pack, overrides, regex_safety, regression, rollback, rule_pack, state_store, telemetry,
    trust_pointer, update,
};
use overrides::*;
use regex_safety::{check_pack_for_redos, RedosConfig};
use regression::{
    generate_regression_suite_from_manifest, prune_recorded_cases,
    prune_recorded_cases_against_packs, record_denial_as_test, write_regression_suite,
    ExpectedVerdict, RecordOutcome, RegressionTestCase, DEFAULT_RECORDED_CASE_LIMIT,
};
use rollback::{check_and_rollback, PoisonPillConfig};
use std::ffi::{OsStr, OsString};
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
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
    /// Curate and bound traffic-derived deny regression cases.
    RegressionPrune {
        /// Directory containing `<pack-id>.cases` files.
        #[arg(short, long, default_value = "tests/regression")]
        path: PathBuf,
        /// Current rule-pack manifest(s) used to remove unreachable cases.
        /// Repeat the option once for each pack represented in the corpus.
        #[arg(long = "rule-pack", alias = "manifest", value_name = "PATH")]
        rule_packs: Vec<PathBuf>,
        /// Maximum number of unique cases to retain in each pack corpus.
        #[arg(long, default_value_t = DEFAULT_RECORDED_CASE_LIMIT)]
        max_cases: usize,
    },
    /// Trust pointer management (Layer 4 minimal form)
    #[command(subcommand)]
    Trust(TrustSubcommand),
    /// Atomically update the modular rule-pack directory from GitHub Releases
    Update {
        /// Path to trust pointer file (defaults to /etc/icg/trust-pointer.json)
        #[arg(short, long)]
        trust_pointer_path: Option<PathBuf>,
        /// Directory where the complete modular pack release is activated
        /// (defaults to /etc/icg/packs; --channel NAME uses /etc/icg/packs-NAME)
        #[arg(short = 'a', long = "pack-dir", alias = "artifact-path")]
        pack_directory: Option<PathBuf>,
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
    /// Build a merged rule-pack.json from individual pack files
    BuildPack {
        /// Directory containing individual pack JSON files (default: packs/)
        #[arg(short, long, default_value = "packs")]
        pack_dir: PathBuf,
        /// Output path for the merged rule-pack.json (default: rule-pack.json)
        #[arg(short, long, default_value = "rule-pack.json")]
        output: PathBuf,
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
    /// Graduated fail-open to fail-closed guard-availability policy
    #[command(subcommand)]
    Policy(PolicySubcommand),
    /// Health monitoring and crash tracking
    Health {
        #[command(flatten)]
        args: HealthArgs,
    },
    /// Serve health probes and Prometheus metrics for an external scraper.
    Monitor {
        /// Address to bind (use 127.0.0.1 when the endpoint is not isolated).
        #[arg(long, default_value = "0.0.0.0")]
        host: String,
        /// TCP port for `/health/live`, `/health/ready`, and `/metrics`.
        #[arg(long, default_value_t = 8080)]
        port: u16,
        /// Override the durable health state path.
        #[arg(long)]
        health_path: Option<PathBuf>,
        /// Override the rule pack file/directory used by the metrics scrape.
        #[arg(long)]
        rule_pack: Option<PathBuf>,
    },
    /// Hook mode: invoked by Claude Code/Codex's PreToolUse hook system
    Hook {
        /// Optional rule-pack JSON file or directory (defaults to /etc/icg/packs;
        /// the legacy /etc/icg/rule-pack.json is used when the directory is absent)
        #[arg(long)]
        rule_pack: Option<PathBuf>,
        /// Practice mode: report denials without blocking the tool call.
        #[arg(long)]
        practice: bool,
        /// Release-bound per-repository override; requires repository and trusted-ref
        #[arg(long)]
        override_file: Option<PathBuf>,
        /// Repository scope for the override
        #[arg(long)]
        repository: Option<String>,
        /// Exact trusted release reference for the override
        #[arg(long)]
        trusted_ref: Option<String>,
        /// Record denied hook inputs as JSONL cases under this directory.
        /// Supplying the flag without a value uses tests/regression.
        #[arg(
            long,
            value_name = "DIR",
            num_args = 0..=1,
            default_missing_value = "tests/regression",
        )]
        record_as_test: Option<PathBuf>,
    },
    /// Wrapper mode: invoked under a shadowed binary name (e.g., vault, git, docker)
    #[command(hide = true)]
    Wrapper {
        /// Practice mode: report denials without blocking the real binary.
        #[arg(long)]
        practice: bool,
        /// Command arguments (shadowed executable invocation)
        #[arg(required = true, trailing_var_arg = true)]
        args: Vec<String>,
    },
    /// Install PATH-wrapper symlinks for currently-loaded command-mode packs
    Install {
        /// Installation directory for symlinks
        #[arg(
            short,
            long,
            help = format!(
                "Installation directory for symlinks (defaults to {})",
                documented_commands::DEFAULT_WRAPPER_INSTALL_DIR
            )
        )]
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
        /// Path to the durable per-release state store (defaults to the ICG cache)
        #[arg(long)]
        state_store_path: Option<PathBuf>,
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
enum PolicySubcommand {
    /// Show the durable graduation policy state
    Status {
        /// Path to policy state (defaults to /etc/icg/fail-closed-policy.json)
        #[arg(short, long)]
        path: Option<PathBuf>,
    },
    /// Reconcile per-release telemetry and poison-pill rollback evidence
    Reconcile {
        /// Path to policy state (defaults to /etc/icg/fail-closed-policy.json)
        #[arg(short, long)]
        path: Option<PathBuf>,
        /// Path to the per-release runtime state store
        #[arg(long)]
        state_store_path: Option<PathBuf>,
        /// Path to the administrator-controlled trust pointer
        #[arg(long)]
        trust_pointer_path: Option<PathBuf>,
    },
    /// Change the clean-release threshold and restart qualification
    Configure {
        /// New positive number of consecutive clean releases
        #[arg(long)]
        threshold: u32,
        /// Path to policy state (defaults to /etc/icg/fail-closed-policy.json)
        #[arg(short, long)]
        path: Option<PathBuf>,
    },
    /// Explicitly demote Fail-Closed to Fail-Open during an incident
    Demote {
        /// Incident reason recorded in the policy state
        #[arg(long)]
        reason: String,
        /// Path to policy state (defaults to /etc/icg/fail-closed-policy.json)
        #[arg(short, long)]
        path: Option<PathBuf>,
    },
    /// Force Fail-Open to Fail-Closed through the administrator control plane
    #[command(alias = "graduate")]
    ForceGraduate {
        /// Change or incident reason recorded in the policy event telemetry
        #[arg(long)]
        reason: String,
        /// Path to policy state (defaults to /etc/icg/fail-closed-policy.json)
        #[arg(short, long)]
        path: Option<PathBuf>,
    },
    /// Force Fail-Closed back to Fail-Open during an incident
    #[command(alias = "revert")]
    ForceRevert {
        /// Incident reason recorded in the policy event telemetry
        #[arg(long)]
        reason: String,
        /// Path to policy state (defaults to /etc/icg/fail-closed-policy.json)
        #[arg(short, long)]
        path: Option<PathBuf>,
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

fn load_rule_pack(path: PathBuf) -> Result<icg::rule_pack::Pack> {
    icg::rule_pack::load_pack(&path)
}

const PRACTICE_MODE_ENV: &str = "ICG_PRACTICE";
const PRACTICE_MODE_BANNER: &str =
    "ICG PRACTICE MODE ACTIVE — ENFORCEMENT IS DISABLED for this check.";

fn practice_mode_enabled(cli_flag: bool) -> bool {
    cli_flag
        || std::env::var(PRACTICE_MODE_ENV)
            .map(|value| {
                value == "1"
                    || value.eq_ignore_ascii_case("true")
                    || value.eq_ignore_ascii_case("yes")
                    || value.eq_ignore_ascii_case("on")
            })
            .unwrap_or(false)
}

fn practice_denial_report(result: &engine::CheckResult, context: Option<&str>) -> Option<String> {
    let engine::CheckResult::Denied {
        reason,
        pack_id,
        pattern_id,
    } = result
    else {
        return None;
    };

    let suffix = context
        .map(|value| format!(", file={value}"))
        .unwrap_or_default();
    Some(format!(
        "WOULD DENY: {reason} [pack={pack_id}, pattern={pattern_id}{suffix}]"
    ))
}

fn practice_response_result(
    result: engine::CheckResult,
    practice_mode: bool,
) -> engine::CheckResult {
    if !practice_mode {
        return result;
    }

    match result {
        engine::CheckResult::Denied { .. } => engine::CheckResult::Allowed,
        other => other,
    }
}

/// Render the native Codex/Claude PreToolUse response envelope. Both hook
/// protocols consume the hook-specific decision under `hookSpecificOutput`;
/// Codex additionally requires `hookEventName` to identify the event.
fn render_hook_response(
    result: engine::CheckResult,
    original_input: Option<&serde_json::Value>,
    updated_input_key: &str,
    context: Option<&str>,
    practice_mode: bool,
) -> serde_json::Value {
    let details = |reason: &str, pack_id: &str, pattern_id: &str| {
        let suffix = context
            .map(|value| format!(", file={value}"))
            .unwrap_or_default();
        format!("{reason} [pack={pack_id}, pattern={pattern_id}{suffix}]")
    };

    let practice_message = practice_mode.then(|| {
        practice_denial_report(&result, context)
            .map(|report| format!("{PRACTICE_MODE_BANNER} {report}"))
            .unwrap_or_else(|| PRACTICE_MODE_BANNER.to_string())
    });
    let result = practice_response_result(result, practice_mode);

    let mut hook_output = serde_json::Map::new();
    hook_output.insert(
        "hookEventName".to_string(),
        serde_json::Value::String("PreToolUse".to_string()),
    );

    let mut response = match result {
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
    };

    if let Some(message) = practice_message {
        response["systemMessage"] = serde_json::Value::String(message);
    }

    response
}

/// Render the successful native-hook response for an explicit operator
/// emergency bypass. Do not parse or echo the request here: hook commands and
/// content may contain credentials, and the bypass audit intentionally records
/// only its activation and front end.
fn render_emergency_bypass_hook_response() -> serde_json::Value {
    serde_json::json!({
        "hookSpecificOutput": {
            "hookEventName": "PreToolUse",
            "permissionDecision": "allow"
        },
        "systemMessage": emergency_bypass::WARNING
    })
}

/// Best-effort persistence for the explicitly enabled traffic recorder.
/// Recording is auxiliary evidence and must never change the deny response or
/// make a hook fail open when its filesystem is unavailable.
fn record_hook_denial(
    directory: Option<&Path>,
    source: &InputSource,
    result: &engine::CheckResult,
) {
    let Some(directory) = directory else {
        return;
    };
    let engine::CheckResult::Denied {
        pack_id,
        pattern_id,
        ..
    } = result
    else {
        return;
    };

    let case = match source {
        InputSource::Command(engine::CommandSource::Hook(command)) => RegressionTestCase {
            pack_id: pack_id.clone(),
            pattern_id: pattern_id.clone(),
            command: command.clone(),
            file_path: None,
            content: None,
            expected: ExpectedVerdict::Deny,
        },
        InputSource::Command(engine::CommandSource::Argv(arguments)) => RegressionTestCase {
            pack_id: pack_id.clone(),
            pattern_id: pattern_id.clone(),
            command: arguments.join(" "),
            file_path: None,
            content: None,
            expected: ExpectedVerdict::Deny,
        },
        InputSource::Content(content) => RegressionTestCase {
            pack_id: pack_id.clone(),
            pattern_id: pattern_id.clone(),
            command: String::new(),
            file_path: Some(content.file_path().to_string()),
            content: Some(content.new_content().to_string()),
            expected: ExpectedVerdict::Deny,
        },
        // A batch is resolved by the caller with the source that produced the
        // aggregate deny. This fallback still records a fail-closed denial if
        // no individual source can be re-evaluated.
        InputSource::ContentBatch(contents) => {
            let Some(content) = contents.first() else {
                return;
            };
            RegressionTestCase {
                pack_id: pack_id.clone(),
                pattern_id: pattern_id.clone(),
                command: String::new(),
                file_path: Some(content.file_path().to_string()),
                content: Some(content.new_content().to_string()),
                expected: ExpectedVerdict::Deny,
            }
        }
    };

    match record_denial_as_test(directory, case) {
        Ok(RecordOutcome::Added) | Ok(RecordOutcome::Duplicate) => {}
        Ok(RecordOutcome::CapacityReached) => eprintln!(
            "icg: regression corpus for pack '{pack_id}' is full; run `icg regression-prune --path {}`",
            directory.display()
        ),
        Err(error) => eprintln!(
            "icg: could not record deny as a regression case in {}: {error:#}",
            directory.display()
        ),
    }
}

/// Locate the content source responsible for a multi-file patch's aggregate
/// denial before recording it. The engine intentionally returns only the most
/// severe result, so this small re-evaluation preserves the actual file and
/// content rather than recording an unrelated file from the same patch.
fn record_hook_batch_denial(
    directory: Option<&Path>,
    engine: &Engine,
    contents: &[engine::ContentSource],
    result: &engine::CheckResult,
) {
    let engine::CheckResult::Denied {
        pack_id,
        pattern_id,
        ..
    } = result
    else {
        return;
    };

    for content in contents {
        let candidate = engine.evaluate_content(content);
        if matches!(
            candidate,
            engine::CheckResult::Denied {
                pack_id: ref candidate_pack,
                pattern_id: ref candidate_pattern,
                ..
            } if candidate_pack == pack_id && candidate_pattern == pattern_id
        ) {
            record_hook_denial(
                directory,
                &InputSource::Content(content.clone()),
                &candidate,
            );
            return;
        }
    }

    record_hook_denial(
        directory,
        &InputSource::ContentBatch(contents.to_vec()),
        result,
    );
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
    state_store: Option<&state_store::StateStore>,
    trust_store_path: &Path,
) -> Result<()> {
    // Preserve the legacy per-process telemetry file for status/diagnostics.
    // The rollback decision below intentionally consumes the durable
    // per-release state store instead; the two signals must not be mixed.
    let store = telemetry_store
        .lock()
        .map_err(|e| anyhow::anyhow!("Failed to lock telemetry store: {}", e))?;
    let poison_pill_config = poison_pill_config_from_telemetry(store.config());
    if let Err(e) = store.persist() {
        eprintln!("⚠️  Failed to persist telemetry state: {}", e);
    }
    drop(store);

    let Some(state_store) = state_store else {
        return Ok(());
    };
    let trust_store = TrustPointerStore::new(trust_store_path);
    if let Err(error) = check_and_rollback(state_store, &trust_store, &poison_pill_config) {
        // Rollback failure is deliberately loud but does not rewrite the
        // already-emitted allow/deny hook response. Operators must use the
        // rollback runbook when this path cannot complete.
        eprintln!("🚨 POISON-PILL AUTO-ROLLBACK FAILED: {error:#}");
    }

    reconcile_fail_closed_policy(state_store, &trust_store, &poison_pill_config);

    Ok(())
}

/// Load runtime telemetry without making persistence a prerequisite for a
/// guard decision.  Hook and wrapper evaluation must retain their fail-open
/// contract when the configured cache is unavailable (for example, during an
/// unprivileged installation or a read-only filesystem).  The in-memory
/// store still lets this invocation collect metrics; its later persist attempt
/// remains best effort and reports the operational failure on stderr.
fn load_runtime_telemetry_store(path: PathBuf) -> telemetry::TelemetryStore {
    match telemetry::TelemetryStore::load_or_create(path.clone()) {
        Ok(store) => store,
        Err(error) => {
            eprintln!(
                "icg_monitoring_event event=telemetry_init_failed path={} error={error:#}",
                path.display()
            );
            telemetry::TelemetryStore::new(path)
        }
    }
}

/// Keep the operator-facing telemetry configuration connected to the durable
/// poison-pill reaction.  The release-count, baseline-volume, absolute-delta,
/// and early-window safeguards remain fixed conservative defaults; the legacy
/// names map only to their compatible reaction controls.
fn poison_pill_config_from_telemetry(config: &telemetry::TelemetryConfig) -> PoisonPillConfig {
    let mut poison_pill_config = PoisonPillConfig {
        enabled: config.auto_rollback_enabled,
        cooldown: config.rollback_cooldown,
        ..PoisonPillConfig::default()
    };
    poison_pill_config.policy.minimum_current_evaluations = config.minimum_samples as u64;
    poison_pill_config.policy.minimum_baseline_evaluations = (config.minimum_samples as u64)
        .saturating_mul(poison_pill_config.policy.minimum_baseline_releases as u64);
    if config.spike_threshold.is_finite() && config.spike_threshold > 0.0 {
        poison_pill_config.policy.baseline_sigma_multiplier = config.spike_threshold;
    }
    poison_pill_config
}

/// Load the configured reaction policy for wrapper invocations, which do not
/// otherwise need to keep the legacy telemetry window in memory.
fn configured_poison_pill_config() -> PoisonPillConfig {
    let telemetry_path = std::env::var_os("ICG_TELEMETRY_PATH")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/var/cache/icg/telemetry.json"));
    match telemetry::TelemetryStore::load_or_create(telemetry_path) {
        Ok(store) => poison_pill_config_from_telemetry(store.config()),
        Err(error) => {
            eprintln!(
                "⚠️  Using default poison-pill configuration; telemetry configuration unavailable: {error:#}"
            );
            PoisonPillConfig::default()
        }
    }
}

/// Reconcile policy state after the poison-pill reaction for either hook or
/// wrapper front-ends. Policy-store failures are operational alerts, not a
/// reason to rewrite an already-emitted operation decision.
fn reconcile_fail_closed_policy(
    state_store: &state_store::StateStore,
    trust_store: &TrustPointerStore,
    poison_pill_config: &PoisonPillConfig,
) {
    let policy_store = PolicyStore::from_env();
    match policy_store.reconcile_release_health(state_store, trust_store, poison_pill_config) {
        Ok(ReconcileOutcome::Clean(transition)) => {
            if let PolicyTransition::Graduated {
                ref release_ref,
                generation,
            } = transition
            {
                eprintln!(
                    "🚀 FAIL-CLOSED GRADUATION: release `{release_ref}` reached the clean-release threshold; policy generation {generation} is now Fail-Closed"
                );
            }
            eprintln!("ℹ️  Fail-closed policy reconciliation: {transition:?}");
        }
        Ok(ReconcileOutcome::PoisonPill(transition)) => {
            eprintln!("⚠️  Fail-closed graduation reset by poison-pill evidence: {transition:?}");
        }
        Ok(ReconcileOutcome::Invalidated(transition)) => {
            eprintln!("⚠️  Fail-closed graduation evidence invalidated: {transition:?}");
        }
        Ok(ReconcileOutcome::Pending { reason }) => {
            eprintln!("ℹ️  Fail-closed graduation pending: {reason}");
        }
        Ok(ReconcileOutcome::NoChange) => {}
        Err(error) => {
            eprintln!(
                "⚠️  Failed to reconcile fail-closed graduation policy {}: {error:#}",
                policy_store.path().display()
            );
        }
    }
}

/// Consume a crash recovered by the lifecycle marker before evaluating the
/// next operation.  The health store detects process disappearance on the
/// next invocation; this boundary turns that durable evidence into the same
/// idempotent poison-pill event used by policy reconciliation.
///
/// Returns true when the active policy requires this invocation to halt.  In
/// fail-open mode the event is still logged and persisted, but evaluation is
/// allowed to continue so the fleet retains the compatibility baseline.
fn recovered_guard_crash_requires_halt(
    engine: &Engine,
    lifecycle: Option<&health::GuardLifecycle>,
) -> bool {
    let Some(crash) = lifecycle.and_then(health::GuardLifecycle::startup_crash) else {
        return false;
    };

    let event_ref = format!("guard-crash:{}", crash.id);
    let policy_store = PolicyStore::from_env();
    match policy_store.record_poison_pill(&event_ref) {
        Ok(transition) => eprintln!(
            "⚠️  Recovered guard crash consumed as poison-pill event {event_ref}: {transition:?}"
        ),
        Err(error) => eprintln!(
            "⚠️  Failed to persist recovered guard crash poison-pill {event_ref}: {error:#}"
        ),
    }

    if engine.fail_closed() {
        eprintln!("🚨 Fail-Closed enforcement: guard crash {event_ref} halts this operation");
        true
    } else {
        eprintln!(
            "⚠️  Fail-Open enforcement: guard crash {event_ref} recorded; allowing this operation"
        );
        false
    }
}

fn guard_crash_result() -> engine::CheckResult {
    engine::CheckResult::Denied {
        reason: "Guard crash in fail-closed mode - rejecting all operations".to_string(),
        pack_id: "fail-closed".to_string(),
        pattern_id: "guard-crash".to_string(),
    }
}

/// Record an in-process guard availability failure without treating ordinary
/// rule denials as crashes.  Health persistence is best effort and never
/// changes the already computed policy decision.
fn record_engine_guard_failure(lifecycle: Option<&mut health::GuardLifecycle>, engine: &Engine) {
    if !engine.has_guard_failure() {
        return;
    }
    if let Some(lifecycle) = lifecycle {
        if let Err(error) = lifecycle.finish_error("guard availability failure during evaluation") {
            eprintln!("icg_health_event event=guard_failure_record_failed error={error:#}");
        }
    }
}

fn record_trust_pointer_observation(
    state_store: Option<&state_store::StateStore>,
    pointer: &TrustPointer,
) {
    if let Some(state_store) = state_store {
        if let Err(error) = state_store.save_trust_pointer(pointer) {
            eprintln!("⚠️  Failed to record trust-pointer history: {error:#}");
        }
    }
}

const DEFAULT_RULE_PACK_PATH: &str = "/etc/icg/rule-pack.json";
const DEFAULT_RULE_PACK_DIR: &str = "/etc/icg/packs";

fn configured_health_path() -> PathBuf {
    health::HealthStore::from_environment_or_default()
        .map(|store| store.path().to_path_buf())
        .unwrap_or_else(|_| PathBuf::from("/var/cache/icg/health-state.json"))
}

fn configured_trust_pointer_path(path: Option<PathBuf>, channel: Option<&str>) -> Result<PathBuf> {
    if let Some(path) = path {
        return Ok(path);
    }
    if let Some(channel) = channel {
        return Ok(TrustPointerStore::for_channel(channel).path().to_path_buf());
    }
    TrustPointerStore::default_path().context("Failed to determine trust pointer path")
}

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
            let directory = PathBuf::from(DEFAULT_RULE_PACK_DIR);
            directory.is_dir().then_some(directory)
        })
        .or_else(|| {
            let artifact = PathBuf::from(DEFAULT_RULE_PACK_PATH);
            artifact.is_file().then_some(artifact)
        })
}

/// Resolve the pack location for a native hook invocation.
///
/// A pack directory is the production shape: it keeps packs independently
/// dispatchable, which is required for empty-keyword packs such as secrets and
/// content-mode packs such as image-tag and storage-class. The single-file
/// artifact remains a compatibility fallback for existing installations.
fn hook_rule_pack_path(explicit: Option<PathBuf>) -> Option<PathBuf> {
    explicit.or_else(wrapper_rule_pack_path)
}

/// Load either one legacy pack artifact or every JSON pack in a modular
/// production directory.
fn load_rule_packs_at_path(engine: &mut Engine, path: &Path) -> Result<()> {
    if path.is_dir() {
        engine.load_packs_from_dir(path)
    } else {
        engine.load_pack_from_file(path)
    }
}

fn load_wrapper_engine() -> Result<Engine> {
    let mut engine = Engine::new();

    if let Some(path) = wrapper_rule_pack_path() {
        load_rule_packs_at_path(&mut engine, &path)?;
    }

    // The wrapper is a separate process for every command, so attach the
    // durable state store here as well as in hook mode. Telemetry is best
    // effort; inability to open the cache must not prevent normal commands
    // from reaching their real binary.
    let durable_state_store = state_store::StateStore::default_path()
        .ok()
        .map(|path| std::sync::Arc::new(state_store::StateStore::new(path)));
    if let Some(state_store) = &durable_state_store {
        engine = engine.with_state_store(std::sync::Arc::clone(state_store));
    }
    if let Ok(trust_path) = TrustPointerStore::default_path() {
        if let Ok(Some(pointer)) = TrustPointerStore::new(&trust_path).load() {
            record_trust_pointer_observation(durable_state_store.as_deref(), &pointer);
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
    let path =
        std::env::var_os("PATH").context("PATH is not set; cannot locate the real binary")?;

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
    // Rule packs use command text for rewrites because that is the native
    // representation on the hook front-end.  The wrapper must realize the
    // same decision as argv, without handing the rewrite to a shell.
    let parser = Engine::new();
    let source = engine::CommandSource::Hook(rewrite.to_string());
    let tokens = parser.segment_command(&source);
    if tokens.len() != 1 || tokens[0].executable != tool {
        anyhow::bail!("rule rewrite did not produce one command for the wrapped tool `{tool}`");
    }

    Ok(tokens[0].args.iter().map(OsString::from).collect())
}

/// Execute the real binary under the explicit emergency policy without
/// constructing an engine or command source. In particular, do not turn argv
/// into loggable command text: wrapper arguments can contain credentials.
#[cfg(unix)]
fn exec_emergency_bypass(
    argv0: &OsStr,
    tool: &str,
    original_args: &[OsString],
    lifecycle: Option<&mut health::GuardLifecycle>,
) -> Result<()> {
    use std::os::unix::process::CommandExt;

    eprintln!("{}", emergency_bypass::WARNING);
    emergency_bypass::record_activation(emergency_bypass::FrontEnd::Wrapper);
    let real_binary = real_binary_in_path(tool, argv0)?;
    if let Some(run) = lifecycle {
        if let Err(error) = run.finish_clean() {
            eprintln!("icg_health_event event=finish_failed error={error:#}");
        }
    }
    let error = Command::new(&real_binary)
        .arg0(argv0)
        .args(original_args)
        .exec();
    anyhow::bail!(
        "failed to exec real `{tool}` binary `{}`: {error}",
        real_binary.display()
    )
}

#[cfg(unix)]
fn run_shadowed_tool(
    argv0: &OsStr,
    tool: &str,
    original_args: &[OsString],
    practice_mode: bool,
    emergency_bypass_active: bool,
    mut lifecycle: Option<&mut health::GuardLifecycle>,
) -> Result<()> {
    use std::os::unix::process::CommandExt;

    if emergency_bypass_active {
        return exec_emergency_bypass(argv0, tool, original_args, lifecycle);
    }

    if practice_mode {
        eprintln!("{PRACTICE_MODE_BANNER}");
    }

    let engine = load_wrapper_engine()?;
    let halt_for_recovered_crash =
        recovered_guard_crash_requires_halt(&engine, lifecycle.as_deref());
    let mut check_argv = Vec::with_capacity(original_args.len() + 1);
    check_argv.push(tool.to_string());
    check_argv.extend(
        original_args
            .iter()
            .map(|arg| arg.to_string_lossy().into_owned()),
    );
    let source = engine.read_from_argv(check_argv);
    let result = if halt_for_recovered_crash {
        guard_crash_result()
    } else {
        engine.evaluate_command(&source)
    };
    record_engine_guard_failure(lifecycle.as_deref_mut(), &engine);
    denial_log::record_operational_denial(&InputSource::Command(source.clone()), &result);

    if practice_mode {
        if let Some(report) = practice_denial_report(&result, None) {
            eprintln!("icg practice: {report}");
        }
    }

    let result = practice_response_result(result, practice_mode);

    if let Ok(state_path) = state_store::StateStore::default_path() {
        let state_store = state_store::StateStore::new(state_path);
        if let Ok(trust_path) = TrustPointerStore::default_path() {
            let trust_store = TrustPointerStore::new(&trust_path);
            let poison_pill_config = configured_poison_pill_config();
            if let Err(error) = check_and_rollback(&state_store, &trust_store, &poison_pill_config)
            {
                eprintln!("🚨 POISON-PILL AUTO-ROLLBACK FAILED: {error:#}");
            }
            reconcile_fail_closed_policy(&state_store, &trust_store, &poison_pill_config);
        }
    }

    let exec_args = match result {
        engine::CheckResult::Allowed => original_args.to_vec(),
        engine::CheckResult::Warning {
            reason,
            pack_id,
            pattern_id,
        } => {
            eprintln!("icg warning: {reason} [pack={pack_id}, pattern={pattern_id}]");
            original_args.to_vec()
        }
        engine::CheckResult::Rewrite {
            reason,
            rewrite,
            pack_id,
            pattern_id,
        } => {
            eprintln!("icg updated command: {reason} [pack={pack_id}, pattern={pattern_id}]");
            rewritten_wrapper_args(tool, &rewrite)?
        }
        engine::CheckResult::Denied {
            reason,
            pack_id,
            pattern_id,
        } => {
            if let Some(run) = lifecycle.as_deref_mut() {
                let finish = if halt_for_recovered_crash || engine.has_guard_failure() {
                    run.finish_error("guard availability failure triggered fail-closed halt")
                } else {
                    // A normal rule denial is a successful guard decision,
                    // not a crash or an unhealthy invocation.
                    run.finish_clean()
                };
                if let Err(error) = finish {
                    eprintln!("icg_health_event event=finish_failed error={error:#}");
                }
            }
            anyhow::bail!("command denied: {reason} [pack={pack_id}, pattern={pattern_id}]")
        }
    };

    let real_binary = real_binary_in_path(tool, argv0)?;

    // `exec` replaces this process, so the guard must close its own run
    // marker before handing control to the real tool.  A failure to exec is
    // still reported by the caller as an abnormal guard exit.
    if let Some(run) = lifecycle {
        if halt_for_recovered_crash {
            if let Err(error) = run.finish_error("recovered guard crash triggered fail-closed halt")
            {
                eprintln!("icg_health_event event=finish_failed error={error:#}");
            }
        }
        if let Err(error) = run.finish_clean() {
            eprintln!("icg_health_event event=finish_failed error={error:#}");
        }
    }
    let error = Command::new(&real_binary)
        .arg0(argv0)
        .args(&exec_args)
        .exec();
    anyhow::bail!(
        "failed to exec real `{tool}` binary `{}`: {error}",
        real_binary.display()
    )
}

/// Run the standalone monitoring endpoint used by a supervisor or sidecar.
/// The endpoint owns a durable lifecycle marker so a probe can distinguish a
/// responsive monitor from a guard process that disappeared.
fn run_monitor(
    host: String,
    port: u16,
    health_path: Option<PathBuf>,
    rule_pack: Option<PathBuf>,
) -> Result<()> {
    let mut monitoring_config = monitoring::MonitoringConfig::from_environment();
    if let Some(path) = rule_pack {
        monitoring_config.rule_pack_path = path;
    }
    let health_path = health_path.unwrap_or_else(|| monitoring_config.health_path.clone());
    monitoring_config = monitoring_config.with_health_path(health_path.clone());

    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .context("failed to create monitoring runtime")?;
    runtime.block_on(async move {
        let health_store = health::HealthStore::new(&health_path);
        health_store.mark_start()?;
        let server_config = health_server::HealthServerConfig {
            host,
            port,
            ..Default::default()
        };
        let mut server =
            health_server::HealthServer::with_health_store(server_config, health_store.clone())
                .with_monitoring_config(monitoring_config);
        if let Err(error) = server.spawn_background_task().await {
            let _ = health_store.mark_clean_exit();
            return Err(error);
        }

        if let Some(address) = server.local_addr() {
            println!("icg monitor listening on http://{address}");
        }
        let signal_result = tokio::signal::ctrl_c().await;
        let shutdown_result = server.shutdown().await;
        let clean_exit_result = health_store.mark_clean_exit();
        signal_result.context("monitoring signal handler failed")?;
        shutdown_result?;
        clean_exit_result
    })
}

#[cfg(not(unix))]
fn run_shadowed_tool(
    _argv0: &OsStr,
    _tool: &str,
    _original_args: &[OsString],
    _practice_mode: bool,
    _emergency_bypass_active: bool,
    _lifecycle: Option<&mut health::GuardLifecycle>,
) -> Result<()> {
    anyhow::bail!("PATH-wrapper mode is only supported on Unix platforms")
}

fn main() -> Result<()> {
    let mut argv = std::env::args_os();
    let argv0 = argv.next().context("process did not provide argv[0]")?;
    let original_args: Vec<OsString> = argv.collect();

    if let Some(tool) = shadowed_tool_name(&argv0) {
        let emergency_bypass_active = emergency_bypass::is_active();
        let mut lifecycle = if emergency_bypass_active {
            None
        } else {
            match health::GuardLifecycle::start() {
                Ok(lifecycle) => Some(lifecycle),
                Err(error) => {
                    eprintln!("icg_health_event event=start_failed error={error:#}");
                    None
                }
            }
        };
        let result = run_shadowed_tool(
            &argv0,
            &tool,
            &original_args,
            practice_mode_enabled(false),
            emergency_bypass_active,
            lifecycle.as_mut(),
        );
        if let Some(run) = lifecycle.as_mut() {
            run.finish_result(&result);
        }
        return result;
    }

    let cli = Cli::parse_from(std::iter::once(argv0.clone()).chain(original_args.iter().cloned()));

    let tracks_lifecycle = matches!(
        &cli.command,
        Commands::Hook { .. } | Commands::Wrapper { .. }
    );
    let emergency_bypass_active = tracks_lifecycle && emergency_bypass::is_active();
    let mut lifecycle = if tracks_lifecycle && !emergency_bypass_active {
        match health::GuardLifecycle::start() {
            Ok(lifecycle) => Some(lifecycle),
            Err(error) => {
                // Health tracking is best effort and must not turn a guard
                // availability problem into a fleet-wide outage.
                eprintln!("icg_health_event event=start_failed error={error:#}");
                None
            }
        }
    } else {
        None
    };

    let result = match cli.command {
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
            let has_regressions = if has_override {
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
                diff.has_regressions()
            } else {
                let diff = run_coverage_diff(previous.clone(), current.clone())?;
                let report = render_coverage_diff_report(
                    &previous,
                    &current,
                    &diff,
                    justification.as_deref(),
                );
                print!("{report}");
                diff.has_regressions()
            };

            // A regression may be approved only when the report carries an
            // explicit, non-blank rationale. The report is printed first so
            // the missing field is still visible in CI output for Layer 2.
            if has_regressions
                && !CoverageDiff::has_explicit_justification(justification.as_deref())
            {
                eprintln!("Coverage regressions detected; rerun with --justification <rationale>.");
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
        Commands::RegressionPrune {
            path,
            rule_packs,
            max_cases,
        } => {
            let packs = rule_packs
                .iter()
                .map(|path| {
                    load_rule_pack(path.clone()).with_context(|| {
                        format!("failed to load curation rule pack {}", path.display())
                    })
                })
                .collect::<Result<Vec<_>>>()?;
            let report = if packs.is_empty() {
                prune_recorded_cases(&path, max_cases)?
            } else {
                prune_recorded_cases_against_packs(&path, &packs, max_cases)?
            };
            println!(
                "Curated {} regression corpus file(s), removed {} case(s)",
                report.files_rewritten, report.cases_removed
            );
            Ok(())
        }
        Commands::Hook {
            rule_pack,
            practice,
            override_file,
            repository,
            trusted_ref,
            record_as_test,
        } => {
            if emergency_bypass_active {
                // An explicit incident escape hatch wins before pack loading,
                // request parsing, and the fail-open/fail-closed availability
                // boundary. Its telemetry deliberately contains no tool input.
                emergency_bypass::record_activation(emergency_bypass::FrontEnd::Hook);
                println!("{}", render_emergency_bypass_hook_response());
                let result = Ok(());
                if let Some(run) = lifecycle.as_mut() {
                    run.finish_result(&result);
                }
                return result;
            }
            // Hook mode: read PreToolUse JSON from stdin, route to appropriate engine, and return decision
            let practice_mode = practice_mode_enabled(practice);
            let mut engine = Engine::new();

            // A modular directory is the production shape. It preserves the
            // dispatch semantics of unconditional and content-mode packs that
            // cannot be represented in the merged compatibility artifact.
            if let Some(pack_path) = hook_rule_pack_path(rule_pack) {
                load_rule_packs_at_path(&mut engine, &pack_path)?;
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
            let telemetry_path = std::env::var_os("ICG_TELEMETRY_PATH")
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from("/var/cache/icg/telemetry.json"));
            let telemetry_store = std::sync::Arc::new(std::sync::Mutex::new(
                load_runtime_telemetry_store(telemetry_path),
            ));

            // Generate session ID for this evaluation
            let session_id = format!("session-{}", uuid::Uuid::new_v4());

            // Get the current release reference from the trust pointer if available
            let trust_store_path = TrustPointerStore::default_path()?;
            let durable_state_store = state_store::StateStore::default_path()
                .ok()
                .map(|path| std::sync::Arc::new(state_store::StateStore::new(path)));
            let release_ref =
                if let Ok(Some(pointer)) = TrustPointerStore::new(&trust_store_path).load() {
                    record_trust_pointer_observation(durable_state_store.as_deref(), &pointer);
                    Some(pointer.trusted_ref)
                } else {
                    None
                };

            // Configure engine with telemetry
            engine = engine
                .with_telemetry_store(telemetry_store.clone())
                .with_session_id(session_id.clone())
                .with_release_ref(release_ref.unwrap_or_else(|| "unknown".to_string()));
            if let Some(state_store) = &durable_state_store {
                engine = engine.with_state_store(std::sync::Arc::clone(state_store));
            }

            let halt_for_recovered_crash =
                recovered_guard_crash_requires_halt(&engine, lifecycle.as_ref());

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
                    let result = if halt_for_recovered_crash {
                        guard_crash_result()
                    } else {
                        engine.evaluate_command(&source)
                    };
                    record_engine_guard_failure(lifecycle.as_mut(), &engine);
                    record_hook_denial(
                        record_as_test.as_deref(),
                        &InputSource::Command(source.clone()),
                        &result,
                    );
                    denial_log::record_operational_denial(
                        &InputSource::Command(source.clone()),
                        &result,
                    );
                    println!(
                        "{}",
                        render_hook_response(
                            result,
                            original_input.as_ref(),
                            input_key,
                            None,
                            practice_mode,
                        )
                    );

                    // Check for anomalies and trigger rollback if needed
                    check_and_handle_anomaly(
                        &telemetry_store,
                        durable_state_store.as_deref(),
                        &trust_store_path,
                    )?;

                    Ok(())
                }
                Some(InputSource::Content(content)) => {
                    // Content-mode: Write/Edit operation
                    // Evaluate against content-mode packs (storage-class, image-tag, beads)
                    let result = if halt_for_recovered_crash {
                        guard_crash_result()
                    } else {
                        engine.evaluate_content(&content)
                    };
                    record_engine_guard_failure(lifecycle.as_mut(), &engine);
                    record_hook_denial(
                        record_as_test.as_deref(),
                        &InputSource::Content(content.clone()),
                        &result,
                    );
                    denial_log::record_operational_denial(
                        &InputSource::Content(content.clone()),
                        &result,
                    );
                    println!(
                        "{}",
                        render_hook_response(
                            result,
                            original_input.as_ref(),
                            input_key,
                            Some(content.file_path()),
                            practice_mode,
                        )
                    );

                    // Check for anomalies and trigger rollback if needed
                    check_and_handle_anomaly(
                        &telemetry_store,
                        durable_state_store.as_deref(),
                        &trust_store_path,
                    )?;

                    Ok(())
                }
                Some(InputSource::ContentBatch(contents)) => {
                    let result = if halt_for_recovered_crash {
                        guard_crash_result()
                    } else {
                        engine.evaluate_content_batch(&contents)
                    };
                    record_engine_guard_failure(lifecycle.as_mut(), &engine);
                    record_hook_batch_denial(
                        record_as_test.as_deref(),
                        &engine,
                        &contents,
                        &result,
                    );
                    denial_log::record_operational_denial(
                        &InputSource::ContentBatch(contents.clone()),
                        &result,
                    );
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
                            practice_mode,
                        )
                    );

                    // Check for anomalies and trigger rollback if needed
                    check_and_handle_anomaly(
                        &telemetry_store,
                        durable_state_store.as_deref(),
                        &trust_store_path,
                    )?;

                    Ok(())
                }
                None => {
                    // Unrecognized tools remain fail-open unless a previously
                    // crashed guard has latched the fail-closed boundary.
                    let result = if halt_for_recovered_crash {
                        guard_crash_result()
                    } else {
                        engine::CheckResult::Allowed
                    };
                    println!(
                        "{}",
                        render_hook_response(
                            result,
                            original_input.as_ref(),
                            input_key,
                            None,
                            practice_mode,
                        )
                    );
                    Ok(())
                }
            }
        }
        Commands::Wrapper { practice, args } => {
            let (tool, args) = args
                .split_first()
                .context("wrapper mode requires the shadowed tool name")?;
            let argv0 = OsString::from(tool);
            let args = args.iter().map(OsString::from).collect::<Vec<_>>();
            run_shadowed_tool(
                &argv0,
                tool,
                &args,
                practice_mode_enabled(practice),
                emergency_bypass_active,
                lifecycle.as_mut(),
            )
        }
        Commands::Install {
            dir,
            packs,
            force,
            uninstall,
        } => documented_commands::run_install(dir, packs, force, uninstall),
        Commands::Trust(subcommand) => match subcommand {
            TrustSubcommand::Show { path, channel } => {
                let store_path = configured_trust_pointer_path(path, channel.as_deref())?;
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
                let store_path = configured_trust_pointer_path(path, channel.as_deref())?;
                let store = TrustPointerStore::new(store_path.clone());

                if let Some(justification) = justification {
                    store.set_trusted_ref_with_justification(&trusted_ref, justification)?;
                } else {
                    store.set_trusted_ref(&trusted_ref)?;
                }

                // Keep the exact previous reference in durable runtime state
                // so the poison-pill reaction can roll back without guessing.
                // Channel pointers have separate rollout histories and are
                // intentionally left for their channel-specific integration.
                if channel.is_none() {
                    if let Ok(Some(pointer)) = store.load() {
                        if let Ok(state_path) = state_store::StateStore::default_path() {
                            record_trust_pointer_observation(
                                Some(&state_store::StateStore::new(state_path)),
                                &pointer,
                            );
                        }
                    }
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
                let store_path = configured_trust_pointer_path(path, channel.as_deref())?;
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
            pack_directory,
            channel,
            check_only,
        } => {
            if check_only {
                return documented_commands::run_update_check();
            }
            let mut config = UpdateConfig::default();

            // Channel-specific paths are derived by the updater only when the
            // caller leaves the corresponding default path unchanged.
            if let Some(ref ch) = channel {
                config.channel = Some(ch.clone());
            }
            if let Some(trust_path) = trust_pointer_path {
                config.trust_pointer_path = trust_path;
            }

            if let Some(pack_directory_override) = pack_directory {
                config.pack_directory = pack_directory_override;
            }

            println!("🔄 icg update started");
            println!("📁 Trust pointer: {}", config.trust_pointer_path.display());
            println!("📁 Pack directory: {}", config.pack_directory.display());
            println!();

            let result = run_update(config).context("Failed to run update")?;

            println!();
            println!("# Update Summary");
            println!();
            println!("**Trusted Reference:** {}", result.trusted_ref);
            println!("**Release Tag:** {}", result.release_tag);
            println!("**Pack Directory:** {}", result.pack_directory.display());
            if let Some(rollback) = result.rollback_directory {
                println!("**Rollback Directory:** {}", rollback.display());
            }
            if let Some(prev) = result.previous_version {
                println!("**Previous Version:** {}", prev);
            } else {
                println!("**Previous Version:** (none)");
            }

            Ok(())
        }
        Commands::BuildPack { pack_dir, output } => {
            println!("🔨 Building merged rule pack");
            println!("📁 Pack directory: {}", pack_dir.display());
            println!("📁 Output: {}", output.display());
            println!();

            let merged_pack = icg::rule_pack::load_and_merge_packs_from_dir(&pack_dir)
                .context("Failed to load and merge packs")?;

            println!("✅ Merged pack created:");
            println!("  - ID: {}", merged_pack.id);
            println!("  - Tool keywords: {}", merged_pack.tool_keywords.len());
            println!("  - Safe patterns: {}", merged_pack.safe_patterns.len());
            println!(
                "  - Guarded patterns: {}",
                merged_pack.guarded_patterns.len()
            );
            println!();

            icg::rule_pack::save_pack(&merged_pack, &output)
                .context("Failed to save merged pack")?;

            println!("✅ Merged pack saved to: {}", output.display());

            Ok(())
        }
        Commands::Status(args) => {
            if args.denials
                || args.health
                || args.pattern_summary
                || args.trend
                || args.format.is_some()
            {
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
                    if let Some(channel) = &channel {
                        println!("  Run `icg update --channel {}` to initialize.", channel);
                    } else {
                        println!("  Run `icg trust set <reference>` to configure.");
                    }
                }
            }
            println!();

            // Rule Pack Version section
            println!("## Rule Pack Version");
            if let Some(artifact_path) = hook_rule_pack_path(None) {
                if artifact_path.is_dir() {
                    let mut pack_paths = match std::fs::read_dir(&artifact_path) {
                        Ok(entries) => entries
                            .filter_map(|entry| entry.ok().map(|entry| entry.path()))
                            .filter(|path| {
                                path.extension().and_then(|ext| ext.to_str()) == Some("json")
                            })
                            .collect::<Vec<_>>(),
                        Err(error) => {
                            println!("  (failed to read {}: {error})", artifact_path.display());
                            Vec::new()
                        }
                    };
                    pack_paths.sort();
                    match pack_paths
                        .iter()
                        .map(icg::rule_pack::load_pack)
                        .collect::<Result<Vec<_>>>()
                    {
                        Ok(packs) if !packs.is_empty() => {
                            let ids = packs
                                .iter()
                                .map(|pack| pack.id.as_str())
                                .collect::<Vec<_>>()
                                .join(", ");
                            println!("  **Pack Directory:** {}", artifact_path.display());
                            println!("  **Packs ({}):** `{ids}`", packs.len());
                        }
                        Ok(_) => println!(
                            "  (no JSON rule packs found in {})",
                            artifact_path.display()
                        ),
                        Err(error) => println!("  (failed to load: {error})"),
                    }
                } else {
                    match icg::rule_pack::load_pack(&artifact_path) {
                        Ok(pack) => {
                            println!("  **Pack ID:** `{}`", pack.id);
                            println!("  **Path:** {}", artifact_path.display());
                        }
                        Err(e) => {
                            println!("  (failed to load: {})", e);
                        }
                    }
                }
            } else {
                println!(
                    "  (no rule-pack directory found at {}; legacy fallback: {})",
                    DEFAULT_RULE_PACK_DIR, DEFAULT_RULE_PACK_PATH
                );
                println!("  Install the approved modular pack directory before enabling the hook.");
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
                    if let Some(channel) = &channel {
                        println!(
                            "  Run `icg update --channel {}` to check for and download updates.",
                            channel
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

            let limitations = [
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
            TelemetrySubcommand::Status {
                path,
                state_store_path,
            } => {
                let telemetry_path =
                    path.unwrap_or_else(|| PathBuf::from("/var/cache/icg/telemetry.json"));
                let store = telemetry::TelemetryStore::load_or_create(telemetry_path)?;

                println!("# Telemetry Status\n");
                println!("**Path:** {}", store.path().display());
                println!("**Window Size:** {}", store.config().window_size);
                println!(
                    "**Spike Threshold:** {:.2}x",
                    store.config().spike_threshold
                );
                println!("**Minimum Samples:** {}", store.config().minimum_samples);
                println!(
                    "**Rollback Cooldown:** {:?}",
                    store.config().rollback_cooldown
                );
                println!(
                    "**Auto-Rollback:** {}",
                    if store.config().auto_rollback_enabled {
                        "enabled"
                    } else {
                        "disabled"
                    }
                );
                println!(
                    "**Emergency Bypasses (retained):** {}",
                    store.emergency_bypasses().len()
                );
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

                // Poison-pill rollback consumes the durable per-release state
                // store, not the legacy invocation window above. Show both so
                // operators can inspect the exact signal used by rollback.
                let state_path = match state_store_path {
                    Some(path) => path,
                    None => state_store::StateStore::default_path()?,
                };
                let durable_store = state_store::StateStore::new(&state_path);
                let durable_baseline = durable_store.rolling_deny_rate_baseline()?;

                println!();
                println!("## Durable Release Baseline");
                println!("**State Store:** {}", durable_store.path().display());
                println!("**Retained Releases:** {}", durable_baseline.release_count);
                println!(
                    "**Total Evaluations:** {}",
                    durable_baseline.evaluation_count
                );
                println!("**Deny Count:** {}", durable_baseline.deny_count);
                println!(
                    "**Mean Deny Rate:** {:.2}%",
                    durable_baseline.mean_deny_rate * 100.0
                );
                println!("**Std Dev:** {:.4}", durable_baseline.std_dev);
                println!(
                    "**Range:** {:.2}%–{:.2}%",
                    durable_baseline.min_deny_rate * 100.0,
                    durable_baseline.max_deny_rate * 100.0
                );

                if let Some(deviation) = durable_store.current_deny_rate_deviation()? {
                    println!();
                    println!("### Current Release Signal");
                    println!("**Release:** {}", deviation.release_ref);
                    println!(
                        "**Current Deny Rate:** {:.2}%",
                        deviation.current_deny_rate * 100.0
                    );
                    println!(
                        "**Absolute Deviation:** {:.2} percentage points",
                        deviation.absolute_deviation * 100.0
                    );
                    println!(
                        "**Baseline Releases:** {}",
                        deviation.baseline.release_count
                    );
                    println!(
                        "**Current Evaluations:** {}",
                        deviation.current_evaluation_count
                    );
                } else {
                    println!("**Current Release Signal:** unavailable (no release telemetry)");
                }

                let rollback = durable_store.rollback_state()?;
                println!();
                println!("## Durable Rollback Status");
                println!("**Rollback Count:** {}", rollback.rollback_count);
                println!(
                    "**Last Rollback:** {}",
                    rollback.last_rollback_at.as_deref().unwrap_or("(none)")
                );
                if let Some(reason) = rollback.last_rollback_reason {
                    println!("**Last Rollback Reason:** {}", reason);
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
                let telemetry_path =
                    path.unwrap_or_else(|| PathBuf::from("/var/cache/icg/telemetry.json"));

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
                let telemetry_path =
                    path.unwrap_or_else(|| PathBuf::from("/var/cache/icg/telemetry.json"));
                let store = telemetry::TelemetryStore::load_or_create(telemetry_path)?;

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
                    println!(
                        "Setting auto-rollback to {}",
                        if enabled { "enabled" } else { "disabled" }
                    );
                    config.auto_rollback_enabled = enabled;
                }

                // Create new store with updated configuration
                let mut new_store =
                    telemetry::TelemetryStore::with_config(store.path().to_path_buf(), config);

                // Copy existing window data to new store
                let window_records = store.window().records().to_vec();
                let rule_metrics = store.rule_metrics().cloned().collect::<Vec<_>>();
                let emergency_bypasses = store.emergency_bypasses().to_vec();
                for record in window_records {
                    new_store.record_evaluation(
                        record.verdict,
                        record.release_ref,
                        record.session_id,
                    );
                }
                new_store.restore_rule_metrics(rule_metrics);
                new_store.restore_emergency_bypasses(emergency_bypasses);

                // Persist the updated configuration
                new_store.persist()?;

                println!("✅ Telemetry configuration updated successfully.");

                Ok(())
            }
        },
        Commands::Policy(subcommand) => match subcommand {
            PolicySubcommand::Status { path } => {
                let store = path
                    .map(PolicyStore::new)
                    .unwrap_or_else(PolicyStore::from_env);
                let state = store.load()?;
                println!("# Fail-Closed Policy\n");
                println!("**Path:** {}", store.path().display());
                println!("**Generation:** {}", state.generation);
                println!("**Mode:** {:?}", state.mode);
                println!(
                    "**Clean Release Streak:** {}/{}",
                    state.clean_release_streak, state.graduation_threshold
                );
                println!("**Counted Releases:** {}", state.counted_releases.len());
                println!(
                    "**Last Poison-Pill Event:** {}",
                    state.last_poison_pill_event.as_deref().unwrap_or("(none)")
                );
                if let Some(reason) = state.last_transition_reason {
                    println!("**Last Transition:** {}", reason);
                }
                if let Some(event) = state.events.last() {
                    println!(
                        "**Last Event:** {:?} (generation {})",
                        event.event_type, event.generation
                    );
                }
                Ok(())
            }
            PolicySubcommand::Reconcile {
                path,
                state_store_path,
                trust_pointer_path,
            } => {
                let policy_store = path
                    .map(PolicyStore::new)
                    .unwrap_or_else(PolicyStore::from_env);
                let state_path = match state_store_path {
                    Some(path) => path,
                    None => state_store::StateStore::default_path()?,
                };
                let trust_path = match trust_pointer_path {
                    Some(path) => path,
                    None => TrustPointerStore::default_path()?,
                };
                let runtime_store = state_store::StateStore::new(state_path);
                let trust_store = TrustPointerStore::new(trust_path);
                let outcome = policy_store.reconcile_release_health(
                    &runtime_store,
                    &trust_store,
                    &PoisonPillConfig::default(),
                )?;
                println!("Fail-closed policy reconciliation: {outcome:?}");
                Ok(())
            }
            PolicySubcommand::Configure { threshold, path } => {
                let store = path
                    .map(PolicyStore::new)
                    .unwrap_or_else(PolicyStore::from_env);
                store.set_threshold(threshold)?;
                println!(
                    "Configured fail-closed graduation threshold to {} at {}",
                    threshold,
                    store.path().display()
                );
                Ok(())
            }
            PolicySubcommand::Demote { reason, path } => {
                let store = path
                    .map(PolicyStore::new)
                    .unwrap_or_else(PolicyStore::from_env);
                let transition = store.emergency_demote(reason)?;
                println!("Fail-closed policy demoted: {transition:?}");
                Ok(())
            }
            PolicySubcommand::ForceGraduate { reason, path } => {
                let store = path
                    .map(PolicyStore::new)
                    .unwrap_or_else(PolicyStore::from_env);
                let transition = store.force_graduate(reason)?;
                println!("Fail-closed policy force-graduated: {transition:?}");
                Ok(())
            }
            PolicySubcommand::ForceRevert { reason, path } => {
                let store = path
                    .map(PolicyStore::new)
                    .unwrap_or_else(PolicyStore::from_env);
                let transition = store.force_revert(reason)?;
                println!("Fail-closed policy force-reverted: {transition:?}");
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
                    let health_path = path.unwrap_or_else(configured_health_path);
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
                    println!(
                        "**Consecutive Clean Runs:** {}",
                        metrics.consecutive_clean_runs
                    );
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
                    let health_path = path.unwrap_or_else(configured_health_path);

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
                    let health_path = path.unwrap_or_else(configured_health_path);
                    let store = health::HealthStore::new(health_path);
                    store.mark_start()?;

                    println!("✅ Process start marked successfully.");

                    Ok(())
                }
                HealthSubcommand::MarkCleanExit { path } => {
                    let health_path = path.unwrap_or_else(configured_health_path);
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
                    let health_path = path.unwrap_or_else(configured_health_path);

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
        }
        Commands::Monitor {
            host,
            port,
            health_path,
            rule_pack,
        } => run_monitor(host, port, health_path, rule_pack),
    };

    if let Some(run) = lifecycle.as_mut() {
        run.finish_result(&result);
    }
    result
}
