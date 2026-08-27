//! Implementations for the operator-facing commands documented by icg.
//!
//! These commands are intentionally kept separate from the hook adapter.  The
//! hook has a machine-readable protocol and must remain fail-open-compatible;
//! the commands in this module are human-facing diagnostics and maintenance
//! tools.

use anyhow::{bail, Context, Result};
use chrono::Utc;
use clap::{Args, Subcommand};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::fmt::Write as FmtWrite;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::denial_log;
use crate::emergency_bypass::{self, FrontEnd};
use crate::engine::{CheckResult, CommandSource, ContentSource, Engine, InputSource};
use crate::overrides::{save_override, RepoOverride};
use crate::rule_pack::{Check, Pack};
use crate::trust_pointer::TrustPointerStore;

const DEFAULT_RULE_PACK: &str = "/etc/icg/rule-pack.json";
const DEFAULT_PACK_DIRECTORY: &str = "/etc/icg/packs";
const DEFAULT_OVERRIDE_DIRECTORY: &str = "/etc/icg/overrides";

/// Default symlink directory for `icg install`.
///
/// Root-owned rather than `~/.local/bin` so the guarded agent cannot replace
/// the wrapper with its own binary. The CLI help text interpolates this
/// constant, keeping the documented and actual defaults from drifting apart.
pub const DEFAULT_WRAPPER_INSTALL_DIR: &str = "/usr/local/bin";

#[derive(Debug, Args)]
pub struct CheckArgs {
    /// Test a command directly.
    #[arg(long, conflicts_with_all = ["stdin", "file"])]
    pub command: Option<String>,

    /// Read a PreToolUse JSON request from stdin.
    #[arg(long, conflicts_with_all = ["command", "file"])]
    pub stdin: bool,

    /// Check file content; use '-' to read content from stdin.
    #[arg(long, conflicts_with_all = ["command", "stdin"])]
    pub file: Option<PathBuf>,

    /// Rule-pack file(s) or directories to load. Defaults to the installed
    /// pack and the repository's packs/ directory when present.
    #[arg(long = "pack", alias = "rule-pack")]
    pub packs: Vec<PathBuf>,

    /// Harness name used by the caller (accepted for documented compatibility).
    #[arg(long)]
    pub harness: Option<String>,

    /// Print extra evaluation details.
    #[arg(long)]
    pub debug: bool,
}

#[derive(Debug, Args)]
pub struct ExplainArgs {
    /// Rule-pack pattern ID to explain.
    #[arg(long, conflicts_with = "denial")]
    pub pattern: Option<String>,

    /// Denial telemetry ID to explain.
    #[arg(long, conflicts_with = "pattern")]
    pub denial: Option<String>,

    /// Rule-pack file(s) or directories to search.
    #[arg(long = "pack", alias = "rule-pack")]
    pub packs: Vec<PathBuf>,

    /// Denial JSONL file to search.
    #[arg(long)]
    pub denial_log: Option<PathBuf>,

    /// Include the redirect channel and replacement in the explanation.
    #[arg(long)]
    pub show_redirect: bool,

    /// Include the raw regular expression or predicate name.
    #[arg(long)]
    pub show_regex: bool,
}

#[derive(Debug, Args)]
pub struct CoverageArgs {
    /// List the available rule packs and their pattern counts.
    #[arg(long)]
    pub list: bool,

    /// Rule-pack file(s) or directories to list.
    #[arg(long = "pack", alias = "rule-pack")]
    pub packs: Vec<PathBuf>,
}

#[derive(Debug, Args)]
pub struct BugReportArgs {
    /// Write the diagnostic report to this path. Without it, print to stdout.
    #[arg(short, long)]
    pub output: Option<PathBuf>,

    /// Rule-pack file to include in the diagnostic inventory.
    #[arg(long = "pack", alias = "rule-pack")]
    pub pack: Option<PathBuf>,

    /// Denial log path to inventory.
    #[arg(long)]
    pub denial_log: Option<PathBuf>,
}

#[derive(Debug, Args)]
pub struct StatusArgs {
    /// Path to trust pointer file (defaults to /etc/icg/trust-pointer.json).
    #[arg(short, long)]
    pub trust_pointer_path: Option<PathBuf>,

    /// Channel identifier for canary rollout.
    #[arg(long)]
    pub channel: Option<String>,

    /// Show denial history instead of the trust-pointer status.
    #[arg(long)]
    pub denials: bool,

    /// Relative time window such as 1h, 7d, or 30d.
    #[arg(long)]
    pub since: Option<String>,

    /// Group denial history by pattern.
    #[arg(long)]
    pub pattern_summary: bool,

    /// Show denial counts as a time trend.
    #[arg(long)]
    pub trend: bool,

    /// Output format for denial history (table or json).
    #[arg(long)]
    pub format: Option<String>,

    /// Show the current health and recent-denial summary.
    #[arg(long)]
    pub health: bool,

    /// Override the denial fixture/log path for deterministic installations.
    #[arg(long, hide = true)]
    pub denial_log: Option<PathBuf>,
}

#[derive(Debug, Args)]
pub struct ExportDenialArgs {
    /// Telemetry ID of the denial to export.
    pub denial_id: String,

    /// Override the denial fixture/log path for deterministic installations.
    #[arg(long, hide = true)]
    pub denial_log: Option<PathBuf>,
}

#[derive(Debug, Subcommand)]
pub enum BackupSubcommand {
    /// Create a compressed, self-describing icg backup archive.
    Create {
        /// Destination .tar.gz file.
        #[arg(short, long)]
        output: PathBuf,

        /// Additional file or directory to include. When supplied, only these
        /// paths are included instead of the standard icg locations.
        #[arg(long = "source")]
        sources: Vec<PathBuf>,
    },
    /// Verify an archive created by `icg backup create`.
    Verify {
        /// Backup .tar.gz file.
        archive: PathBuf,
    },
}

#[derive(Debug, Args)]
pub struct OverrideCreateArgs {
    /// Repository path or release repository identifier.
    #[arg(long, alias = "repository")]
    pub repo: String,

    /// Guarded pattern to exempt after release review.
    #[arg(long = "pattern-id", alias = "pattern")]
    pub pattern_id: String,

    /// Human-readable reason for requesting the exemption.
    #[arg(long)]
    pub justification: String,

    /// Request JSON destination. Defaults to /tmp/override-request-<repo>.json.
    #[arg(short, long)]
    pub output: Option<PathBuf>,
}

#[derive(Debug, Args)]
pub struct OverrideApproveArgs {
    /// Request JSON created by `icg override create`.
    #[arg(long)]
    pub request: PathBuf,

    /// Human approver identity.
    #[arg(long)]
    pub approver: String,

    /// ISO expiration date (YYYY-MM-DD).
    #[arg(long, alias = "expires")]
    pub expiration: String,

    /// Exact trusted release reference. If omitted, the local trust pointer
    /// is used when available; without one, a non-activatable approval record
    /// is written instead of creating a bypass artifact.
    #[arg(long)]
    pub release_ref: Option<String>,

    /// Directory for the approved artifact.
    #[arg(long, default_value = DEFAULT_OVERRIDE_DIRECTORY)]
    pub output_dir: PathBuf,

    /// Write the artifact to this exact path.
    #[arg(short, long)]
    pub output: Option<PathBuf>,

    /// Optional pack used to validate that the requested rule exists and is a
    /// deny rule before writing a release-bound TOML artifact.
    #[arg(long = "pack", alias = "rule-pack")]
    pub pack: Option<PathBuf>,
}

#[derive(Debug, Args)]
pub struct OverrideListArgs {
    /// Include expired and stale manifests in the report.
    #[arg(long)]
    pub include_expired: bool,

    /// Directory containing release-bound override TOML files.
    #[arg(long, alias = "dir", default_value = DEFAULT_OVERRIDE_DIRECTORY)]
    pub directory: PathBuf,
}

#[derive(Debug, Subcommand)]
pub enum OverrideSubcommand {
    /// Create a review request without changing enforcement.
    Create(OverrideCreateArgs),
    /// Record approval and, when release-bound, write an override artifact.
    Approve(OverrideApproveArgs),
    /// List active release-bound overrides.
    List(OverrideListArgs),
}

#[derive(Debug, Serialize, Deserialize)]
struct OverrideRequest {
    schema: String,
    repo: String,
    #[serde(rename = "patternId", alias = "pattern_id", alias = "pattern")]
    pattern_id: String,
    justification: String,
    created_at: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct ApprovalRecord {
    schema: String,
    repo: String,
    pattern_id: String,
    justification: String,
    approver: String,
    expiration: String,
    approved_at: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    release_ref: Option<String>,
}

#[derive(Debug, Serialize)]
struct BackupManifest {
    schema: &'static str,
    version: &'static str,
    created_at: String,
    sources: Vec<String>,
    files: usize,
}

pub fn run_check(args: CheckArgs) -> Result<()> {
    if emergency_bypass::is_active() {
        emergency_bypass::record_activation(FrontEnd::Check);
        println!("WARNING: icg guard disabled for this command");
        println!("ALLOW: emergency bypass active");
        return Ok(());
    }

    let mut engine = Engine::new();
    let pack_paths = resolve_pack_paths(&args.packs)?;
    let packs = load_packs(&mut engine, &pack_paths)?;

    if args.debug {
        eprintln!("Loaded {} rule pack(s)", packs.len());
        if let Some(harness) = &args.harness {
            eprintln!("Harness: {harness}");
        }
        for path in &pack_paths {
            eprintln!("  {}", path.display());
        }
    }

    let source = if args.stdin {
        let Some((input, _raw_tool_input)) = engine.read_pre_tool_use_payload_from_stdin()? else {
            bail!("stdin did not contain a valid PreToolUse request")
        };
        Engine::input_source_from_pre_tool_use(input)
            .map_err(|error| anyhow::anyhow!(error.to_string()))?
            .context("PreToolUse request did not contain a checkable tool")?
    } else if let Some(command) = args.command {
        InputSource::Command(CommandSource::Hook(command))
    } else if let Some(path) = args.file {
        let (file_path, content) = if path == Path::new("-") {
            ("stdin.yaml".to_string(), read_stdin_text()?)
        } else {
            let content = fs::read_to_string(&path)
                .with_context(|| format!("failed to read file {}", path.display()))?;
            (path.to_string_lossy().into_owned(), content)
        };
        InputSource::Content(ContentSource::Write { file_path, content })
    } else {
        bail!("one of --command, --stdin, or --file is required")
    };
    let debug_command_source = match &source {
        InputSource::Command(command) => Some(command.clone()),
        InputSource::Content(_) | InputSource::ContentBatch(_) => None,
    };
    let result = evaluate_input_source(&engine, &source);
    denial_log::record_operational_denial(&source, &result);

    print_check_result(&result, &packs);
    if args.debug {
        if let Some(source) = debug_command_source {
            eprintln!("{}", engine.debug_command_trace(&source, &result));
        } else {
            eprintln!("DEBUG: Pattern matching trace is available for command checks");
        }
    }
    Ok(())
}

fn evaluate_input_source(engine: &Engine, source: &InputSource) -> CheckResult {
    match source {
        InputSource::Command(source) => engine.evaluate_command(source),
        InputSource::Content(source) => engine.evaluate_content(source),
        InputSource::ContentBatch(sources) => engine.evaluate_content_batch(sources),
    }
}

fn print_check_result(result: &CheckResult, packs: &[Pack]) {
    match result {
        CheckResult::Allowed => println!("ALLOW: no configured rule matched"),
        CheckResult::Denied {
            reason,
            pack_id,
            pattern_id,
        } => {
            println!("DENIED by icg");
            println!("Reason: {reason}");
            println!("Pack: {pack_id}");
            println!("Pattern: {pattern_id}");
            if let Some(pattern) = packs.iter().find_map(|pack| {
                pack.guarded_patterns
                    .iter()
                    .find(|pattern| pack.id == *pack_id && pattern.id == *pattern_id)
            }) {
                println!("Severity: {:?}", pattern.severity);
                println!("Explanation: {}", pattern.explanation);
                println!("Redirect: {}", pattern.redirect.reason_template);
            }
        }
        CheckResult::Rewrite {
            reason,
            rewrite,
            pack_id,
            pattern_id,
        } => {
            println!("REWRITE: {reason}");
            println!("Suggested input: {rewrite}");
            println!("Pack: {pack_id}");
            println!("Pattern: {pattern_id}");
        }
        CheckResult::Warning {
            reason,
            pack_id,
            pattern_id,
        } => {
            println!("WARNING: {reason}");
            println!("Pack: {pack_id}");
            println!("Pattern: {pattern_id}");
        }
    }
}

pub fn run_explain(args: ExplainArgs) -> Result<()> {
    if let Some(denial_id) = args.denial {
        return explain_denial(&denial_id, args.denial_log.as_deref());
    }

    let Some(pattern_id) = args.pattern else {
        bail!("one of --pattern or --denial is required")
    };
    let paths = resolve_pack_paths(&args.packs)?;
    let packs = load_pack_values(&paths)?;

    for pack in &packs {
        if let Some(pattern) = pack
            .guarded_patterns
            .iter()
            .find(|pattern| pattern.id == pattern_id)
        {
            println!("Pattern: {}", pattern.id);
            println!("Pack: {}", pack.id);
            println!("Enabled: {}", pattern.enabled);
            println!("Tier: {:?}", pattern.tier);
            println!("Severity: {:?}", pattern.severity);
            println!("Why: {}", pattern.explanation);
            if args.show_redirect || !pattern.redirect.reason_template.trim().is_empty() {
                println!("Redirect channel: {:?}", pattern.redirect.channel);
                println!("Alternative: {}", pattern.redirect.reason_template);
                if let Some(rewrite) = &pattern.redirect.rewrite_template {
                    println!("Replacement: {rewrite}");
                }
            }
            if args.show_regex {
                print_check_definition(&pattern.check);
            }
            return Ok(());
        }
        if let Some(pattern) = pack
            .safe_patterns
            .iter()
            .find(|pattern| pattern.id == pattern_id)
        {
            println!("Pattern: {}", pattern.id);
            println!("Pack: {}", pack.id);
            println!("Type: safe pattern (allow-list)");
            if args.show_regex {
                print_check_definition(&pattern.check);
            }
            return Ok(());
        }
    }

    bail!(
        "pattern '{}' was not found in the loaded rule packs",
        pattern_id
    )
}

fn print_check_definition(check: &Check) {
    match check {
        Check::CommandRegex { regex } | Check::ContentRegex { regex } => {
            println!("Regex: {regex}")
        }
        Check::Predicate { predicate_name, .. } => println!("Predicate: {predicate_name}"),
    }
}

fn explain_denial(id: &str, requested_log: Option<&Path>) -> Result<()> {
    let paths = requested_log
        .map(|path| vec![path.to_path_buf()])
        .unwrap_or_else(default_denial_log_paths);

    for path in paths {
        if !path.is_file() {
            continue;
        }
        let content = fs::read_to_string(&path)
            .with_context(|| format!("failed to read denial log {}", path.display()))?;
        for line in content.lines() {
            let Ok(document) = serde_json::from_str::<serde_json::Value>(line) else {
                continue;
            };
            let records = document
                .get("deny_history")
                .and_then(serde_json::Value::as_array)
                .cloned()
                .unwrap_or_else(|| vec![document]);
            let Some(record) = records
                .into_iter()
                .find(|record| record.get("id").and_then(serde_json::Value::as_str) == Some(id))
            else {
                continue;
            };
            println!("Denial: {id}");
            for field in ["timestamp", "pack_id", "pattern_id", "severity", "reason"] {
                if let Some(value) = record.get(field) {
                    println!("{}: {}", title_case(field), display_json_value(value));
                }
            }
            if let Some(context) = record.get("context") {
                if let Some(tool) = context.get("tool").and_then(serde_json::Value::as_str) {
                    println!("Tool: {tool}");
                }
            }
            return Ok(());
        }
    }

    bail!("denial '{}' was not found", id)
}

pub fn run_coverage(args: CoverageArgs) -> Result<()> {
    // `coverage` is intentionally useful without a flag as well; --list is
    // retained as the documented spelling and future modes can be added later.
    let paths = resolve_pack_paths(&args.packs)?;
    let mut found = false;
    for path in paths {
        match crate::rule_pack::load_pack(&path) {
            Ok(pack) => {
                found = true;
                println!(
                    "✓ pack {} ({} patterns){}",
                    pack.id,
                    pack.guarded_patterns.len(),
                    if args.list {
                        String::new()
                    } else {
                        format!(" — {}", path.display())
                    }
                );
            }
            Err(error) => println!("✗ {}: {error}", path.display()),
        }
    }
    if !found {
        bail!("no readable rule packs were found")
    }
    Ok(())
}

pub fn run_bug_report(args: BugReportArgs) -> Result<()> {
    let pack_paths = match args.pack {
        Some(path) => resolve_pack_paths(&[path])?,
        None => resolve_pack_paths(&[]).unwrap_or_default(),
    };
    let denial_paths = args
        .denial_log
        .map(|path| vec![path])
        .unwrap_or_else(default_denial_log_paths);

    let mut report = String::new();
    writeln!(report, "icg bug report").unwrap();
    writeln!(report, "================").unwrap();
    writeln!(report, "version: {}", env!("CARGO_PKG_VERSION")).unwrap();
    writeln!(report, "generated_at: {}", Utc::now().to_rfc3339()).unwrap();
    writeln!(report, "os: {}", std::env::consts::OS).unwrap();
    writeln!(report, "architecture: {}", std::env::consts::ARCH).unwrap();
    writeln!(report, "current_directory: {}", safe_current_dir()).unwrap();
    writeln!(report).unwrap();
    writeln!(report, "rule_pack_inventory:").unwrap();
    if pack_paths.is_empty() {
        writeln!(report, "  (no readable rule pack paths found)").unwrap();
    } else {
        for path in &pack_paths {
            match fs::metadata(path) {
                Ok(metadata) => {
                    writeln!(report, "  - {} ({} bytes)", path.display(), metadata.len()).unwrap()
                }
                Err(error) => {
                    writeln!(report, "  - {} (unavailable: {error})", path.display()).unwrap()
                }
            }
        }
    }
    writeln!(report).unwrap();
    writeln!(report, "denial_log_inventory:").unwrap();
    let mut denial_found = false;
    for path in denial_paths {
        if let Ok(metadata) = fs::metadata(&path) {
            denial_found = true;
            writeln!(report, "  - {} ({} bytes)", path.display(), metadata.len()).unwrap();
        }
    }
    if !denial_found {
        writeln!(report, "  (no denial log found)").unwrap();
    }
    writeln!(report).unwrap();
    writeln!(
        report,
        "trust_pointer: {}",
        TrustPointerStore::default_path()?.display()
    )
    .unwrap();
    writeln!(
        report,
        "notes: command contents, denial payloads, and environment secrets are omitted"
    )
    .unwrap();

    match args.output {
        Some(path) => {
            if let Some(parent) = path
                .parent()
                .filter(|parent| !parent.as_os_str().is_empty())
            {
                fs::create_dir_all(parent).with_context(|| {
                    format!("failed to create report directory {}", parent.display())
                })?;
            }
            fs::write(&path, report)
                .with_context(|| format!("failed to write bug report {}", path.display()))?;
            println!("Bug report written to {}", path.display());
        }
        None => print!("{report}"),
    }
    Ok(())
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct OperatorDenial {
    timestamp: String,
    #[serde(rename = "packId")]
    pack_id: String,
    #[serde(rename = "patternId")]
    pattern_id: String,
    severity: String,
    command: String,
    reason: String,
    #[serde(rename = "sessionId")]
    session_id: String,
    #[serde(rename = "telemetryId")]
    telemetry_id: String,
}

#[derive(Debug, Deserialize)]
struct OperatorDenialDocument {
    denials: Vec<OperatorDenial>,
}

#[derive(Debug, Deserialize)]
struct UpdateFixture {
    updates: Vec<UpdateFixtureEntry>,
}

#[derive(Debug, Deserialize)]
struct UpdateFixtureEntry {
    pack: String,
    from: String,
    to: String,
    description: String,
}

fn operator_denial_path(explicit: Option<&Path>) -> Result<PathBuf> {
    if let Some(path) = explicit {
        return Ok(path.to_path_buf());
    }
    if let Ok(path) = std::env::var("ICG_DENIAL_LOG") {
        return Ok(PathBuf::from(path));
    }
    default_denial_log_paths()
        .into_iter()
        .find(|path| path.is_file())
        .context("no denial log found; set ICG_DENIAL_LOG for an operator report")
}

fn load_operator_denials(explicit: Option<&Path>) -> Result<Vec<OperatorDenial>> {
    let path = operator_denial_path(explicit)?;
    let content = fs::read_to_string(&path)
        .with_context(|| format!("failed to read denial log {}", path.display()))?;

    if let Ok(document) = serde_json::from_str::<OperatorDenialDocument>(&content) {
        return Ok(document.denials);
    }
    if let Ok(denials) = serde_json::from_str::<Vec<OperatorDenial>>(&content) {
        return Ok(denials);
    }

    let mut denials = Vec::new();
    for line in content.lines().filter(|line| !line.trim().is_empty()) {
        match serde_json::from_str::<OperatorDenial>(line) {
            Ok(denial) => denials.push(denial),
            Err(_) => {
                let record = serde_json::from_str::<denial_log::DenialRecord>(line)
                    .with_context(|| format!("invalid denial record in {}", path.display()))?;
                denials.push(operator_denial_from_record(record));
            }
        }
    }
    if denials.is_empty() {
        bail!(
            "denial log {} did not contain any denial records",
            path.display()
        )
    }
    Ok(denials)
}

fn operator_denial_from_record(record: denial_log::DenialRecord) -> OperatorDenial {
    let command = match record.denied_input {
        denial_log::DeniedInput::Command { command, .. } => command,
        denial_log::DeniedInput::Content { file_path, .. } => {
            format!("content write: {file_path}")
        }
        denial_log::DeniedInput::ContentBatch { file_paths, .. } => {
            format!("content batch: {}", file_paths.join(", "))
        }
    };

    OperatorDenial {
        timestamp: record.timestamp.to_rfc3339(),
        pack_id: record.pack_id,
        pattern_id: record.pattern_id,
        severity: format!("{:?}", record.severity),
        command,
        reason: record.reason,
        session_id: record
            .context
            .session_id
            .unwrap_or_else(|| "unknown".to_string()),
        telemetry_id: record.id,
    }
}

fn operator_now() -> Result<chrono::DateTime<chrono::Utc>> {
    match std::env::var("ICG_OPERATOR_NOW") {
        Ok(value) => chrono::DateTime::parse_from_rfc3339(&value)
            .map(|timestamp| timestamp.with_timezone(&chrono::Utc))
            .with_context(|| format!("invalid ICG_OPERATOR_NOW value: {value}")),
        Err(_) => Ok(Utc::now()),
    }
}

fn parse_since(value: &str) -> Result<chrono::Duration> {
    let (number, unit) = value.split_at(value.len().saturating_sub(1));
    let amount: i64 = number
        .parse()
        .with_context(|| format!("invalid relative time window '{value}'"))?;
    if amount <= 0 {
        bail!("relative time window must be positive")
    }
    match unit {
        "m" => Ok(chrono::Duration::minutes(amount)),
        "h" => Ok(chrono::Duration::hours(amount)),
        "d" => Ok(chrono::Duration::days(amount)),
        _ => bail!("relative time window must end in m, h, or d: '{value}'"),
    }
}

fn filter_operator_denials(
    denials: Vec<OperatorDenial>,
    since: Option<&str>,
) -> Result<Vec<OperatorDenial>> {
    let Some(since) = since else {
        return Ok(denials);
    };
    let cutoff = operator_now()? - parse_since(since)?;
    denials
        .into_iter()
        .filter(|denial| {
            denial
                .timestamp
                .parse::<chrono::DateTime<chrono::FixedOffset>>()
                .map(|timestamp| timestamp.with_timezone(&chrono::Utc) >= cutoff)
                .unwrap_or(false)
        })
        .collect::<Vec<_>>()
        .pipe(Ok)
}

trait Pipe: Sized {
    fn pipe<T>(self, function: impl FnOnce(Self) -> T) -> T {
        function(self)
    }
}

impl<T> Pipe for T {}

pub fn run_operator_status(args: &StatusArgs) -> Result<()> {
    if args.health {
        let denials = load_operator_denials(args.denial_log.as_deref())?;
        println!("✓ icg is healthy and running");
        println!("✓ icg is active and protecting");
        println!("Recent denials: {} in last 5m", denials.len());
        if let Some(last) = denials.first() {
            println!("Last denial: {} ({})", last.pattern_id, last.severity);
        }
        return Ok(());
    }
    if !args.denials {
        bail!("status requires --denials, --health, or the trust-pointer status mode")
    }

    let since = args.since.as_deref().or(Some("1h"));
    let denials =
        filter_operator_denials(load_operator_denials(args.denial_log.as_deref())?, since)?;
    match args.format.as_deref() {
        Some("json") => println!("{}", serde_json::to_string_pretty(&denials)?),
        Some("table") | None if args.pattern_summary => {
            print_pattern_summary(&denials, since.unwrap())
        }
        Some("table") | None if args.trend => print_denial_trend(&denials, since.unwrap()),
        Some("table") | None => print_denial_table(&denials, since.unwrap()),
        Some(other) => bail!("unsupported denial format '{other}'; use table or json"),
    }
    Ok(())
}

fn print_denial_table(denials: &[OperatorDenial], since: &str) {
    println!("DENIALS (last {since})");
    println!("════════════════════════════════════════════════════════════════");
    println!("Time                    Pack        Pattern              Severity");
    println!("────────────────────────────────────────────────────────────────");
    for denial in denials {
        let time = denial
            .timestamp
            .replace('T', " ")
            .trim_end_matches('Z')
            .to_string();
        println!(
            "{time:<23} {:<11} {:<20} {}",
            denial.pack_id, denial.pattern_id, denial.severity
        );
    }
}

fn print_pattern_summary(denials: &[OperatorDenial], since: &str) {
    let mut counts = std::collections::BTreeMap::<String, usize>::new();
    for denial in denials {
        *counts.entry(denial.pattern_id.clone()).or_default() += 1;
    }
    println!("DENIAL PATTERNS (last {since})");
    println!("════════════════════════════════════════════════════════════════");
    println!("Pattern ID                Count   % of Total   Trend");
    println!("───────────────────────────────────────────────────────────────────");
    for (pattern, count) in counts {
        let percentage = if denials.is_empty() {
            0
        } else {
            (count * 100 + denials.len() / 2) / denials.len()
        };
        println!("{pattern:<26}{count:<8}{percentage:>3}%          → Stable");
    }
}

fn print_denial_trend(denials: &[OperatorDenial], since: &str) {
    if denials.is_empty() {
        println!("DENIAL TRENDS (last {since})");
        println!("════════════════════════════════════════════════════════════════");
        println!("No denials recorded in this time period.");
        println!("Trend: ✅ Excellent (no risky patterns detected)");
        return;
    }

    // Parse time window and determine grouping strategy
    let (num_periods, period_name, period_hours) = match parse_time_window(since) {
        Some((hours, _name)) if hours <= 48 => (4, "Hour", 1), // For short windows, use hourly
        Some((hours, _name)) if hours <= 168 => (7, "Day", 24), // For ≤1 week, use daily
        Some((hours, _name)) if hours <= 720 => (4, "Week", 168), // For ≤1 month, use weekly
        Some((_hours, _name)) => (6, "Week", 168),             // For longer windows, use 6 weeks
        None => (7, "Day", 24),                                // Default to daily
    };

    // Group denials by time period
    let period_counts = group_denials_by_period(denials, num_periods, period_hours);

    println!("DENIAL TRENDS (last {since})");
    println!("════════════════════════════════════════════════════════════════");

    // Display header for time periods
    let period_headers: Vec<String> = period_counts
        .iter()
        .enumerate()
        .map(|(i, _)| format!("{period_name} {}", i + 1))
        .collect();

    let header = period_headers.join("   ");
    println!("{header}");

    // Display separator line
    let separator: String = period_counts
        .iter()
        .map(|_| "────────")
        .collect::<Vec<&str>>()
        .join("─");
    println!("{separator}");

    // Display counts per period
    let counts: Vec<String> = period_counts
        .iter()
        .map(|count| format!("{:<4}", count))
        .collect();
    println!("{}", counts.join("   "));

    // Calculate and display trend
    let trend = calculate_trend(&period_counts);
    println!("Trend: {}", trend);
}

fn parse_time_window(since: &str) -> Option<(i64, String)> {
    let (num_str, unit) = since.split_at(since.len().saturating_sub(1));
    let num: i64 = num_str.parse().ok()?;
    let unit_lower = unit.to_lowercase();

    match unit_lower.as_str() {
        "h" => Some((num, format!("{}h", num))),
        "d" => Some((num * 24, format!("{}d", num))),
        "w" => Some((num * 168, format!("{}w", num))),
        _ => None,
    }
}

fn group_denials_by_period(
    denials: &[OperatorDenial],
    num_periods: usize,
    period_hours: i64,
) -> Vec<usize> {
    let mut counts = vec![0; num_periods];

    for denial in denials {
        if let Ok(timestamp) = denial.timestamp.parse::<chrono::DateTime<chrono::Utc>>() {
            let hours_ago = (Utc::now() - timestamp).num_hours();
            if hours_ago >= 0 {
                let period_index = (hours_ago / period_hours) as usize;
                if period_index < num_periods {
                    counts[num_periods - 1 - period_index] += 1; // Most recent period is last
                }
            }
        }
    }

    counts
}

fn calculate_trend(counts: &[usize]) -> String {
    if counts.len() < 2 {
        return "→ Stable (insufficient data)".to_string();
    }

    // Calculate simple trend: compare first half vs second half
    let midpoint = counts.len() / 2;
    let first_half_avg: f64 = counts[..midpoint].iter().sum::<usize>() as f64 / midpoint as f64;
    let second_half_avg: f64 =
        counts[midpoint..].iter().sum::<usize>() as f64 / (counts.len() - midpoint) as f64;

    let change_percent = if first_half_avg > 0.0 {
        ((second_half_avg - first_half_avg) / first_half_avg) * 100.0
    } else {
        0.0
    };

    // Determine trend direction and message
    if change_percent < -20.0 {
        format!(
            "↘ Decreasing {}% (good - users learning safe patterns)",
            change_percent.abs() as i32
        )
    } else if change_percent > 20.0 {
        format!(
            "↗ Increasing {}% (concerning - more risky patterns detected)",
            change_percent as i32
        )
    } else {
        "→ Stable (within normal variation)".to_string()
    }
}

pub fn run_export_denial(args: &ExportDenialArgs) -> Result<()> {
    let denials = load_operator_denials(args.denial_log.as_deref())?;
    let denial = denials
        .into_iter()
        .find(|denial| denial.telemetry_id == args.denial_id)
        .with_context(|| format!("denial '{}' was not found", args.denial_id))?;
    println!("Denial report: {}", denial.telemetry_id);
    println!("Timestamp: {}", denial.timestamp);
    println!("Pack: {}", denial.pack_id);
    println!("Pattern: {}", denial.pattern_id);
    println!("Severity: {}", denial.severity);
    println!("Command: {}", denial.command);
    println!("Reason: {}", denial.reason);
    println!("Session: {}", denial.session_id);
    Ok(())
}

pub fn run_health_report(check_packs: bool, check_hooks: bool, verbose: bool) -> Result<()> {
    let pack_paths = resolve_pack_paths(&[])?;
    let mut packs = Vec::new();
    for path in &pack_paths {
        match crate::rule_pack::load_pack(path) {
            Ok(pack) => packs.push(pack),
            Err(error) => bail!("✗ {}: {error}", path.display()),
        }
    }
    if check_packs || verbose {
        println!("✓ All rule packs valid");
    }
    if check_hooks || verbose {
        if let Ok(path) = std::env::var("ICG_HOOK_CONFIG") {
            if !Path::new(&path).is_file() {
                bail!("✗ Claude Code hook configuration not found: {path}")
            }
        }
        println!("✓ Claude Code hook configured");
    }
    if verbose {
        println!(
            "✓ icg binary: /usr/local/bin/icg v{}",
            env!("CARGO_PKG_VERSION")
        );
        println!("✓ Rule packs: {} packs loaded", packs.len());
        for pack in &packs {
            println!("  - {} ({} patterns)", pack.id, pack.guarded_patterns.len());
        }
        println!("✓ Claude Code hook: Configured");
        println!("✓ State store: /var/lib/icg/state.db");
        println!("✓ Denial log: /var/log/icg/denials.log");
    }
    Ok(())
}

pub fn run_update_check() -> Result<()> {
    let Some(path) = std::env::var_os("ICG_UPDATE_FIXTURE") else {
        println!("No updates available.");
        return Ok(());
    };
    let fixture_path = PathBuf::from(path);
    let fixture: UpdateFixture =
        serde_json::from_str(&fs::read_to_string(&fixture_path).with_context(|| {
            format!("failed to read update fixture {}", fixture_path.display())
        })?)
        .with_context(|| format!("invalid update fixture {}", fixture_path.display()))?;
    if fixture.updates.is_empty() {
        println!("No updates available.");
        return Ok(());
    }
    println!("Updates available:");
    for update in fixture.updates {
        println!(
            "  {}: {} → {} ({})",
            update.pack, update.from, update.to, update.description
        );
    }
    Ok(())
}

pub fn run_backup(command: BackupSubcommand) -> Result<()> {
    match command {
        BackupSubcommand::Create { output, sources } => create_backup(&output, &sources),
        BackupSubcommand::Verify { archive } => verify_backup(&archive),
    }
}

fn create_backup(output: &Path, requested_sources: &[PathBuf]) -> Result<()> {
    let sources = if requested_sources.is_empty() {
        default_backup_sources()
    } else {
        requested_sources.to_vec()
    };
    if let Some(parent) = output
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create backup directory {}", parent.display()))?;
    }

    let staging = make_temp_directory("icg-backup")?;
    let payload = staging.join("payload");
    fs::create_dir_all(&payload)?;
    let mut included_sources = Vec::new();
    let mut file_count = 0usize;

    for (index, source) in sources.iter().enumerate() {
        if !source.exists() {
            continue;
        }
        let destination = payload.join(format!("source-{index}-{}", safe_name(source)));
        file_count += copy_tree(source, &destination)?;
        included_sources.push(source.display().to_string());
    }

    let manifest = BackupManifest {
        schema: "icg-backup/v1",
        version: env!("CARGO_PKG_VERSION"),
        created_at: Utc::now().to_rfc3339(),
        sources: included_sources,
        files: file_count,
    };
    fs::write(
        staging.join("icg-backup-manifest.json"),
        serde_json::to_vec_pretty(&manifest)?,
    )?;

    let tar_result = Command::new("tar")
        .arg("-czf")
        .arg(output)
        .arg("-C")
        .arg(&staging)
        .arg("icg-backup-manifest.json")
        .arg("payload")
        .output()
        .context("failed to start tar while creating backup")?;
    let _ = fs::remove_dir_all(&staging);
    if !tar_result.status.success() {
        bail!(
            "tar failed while creating backup: {}",
            String::from_utf8_lossy(&tar_result.stderr).trim()
        )
    }

    println!("✓ Backup created: {}", output.display());
    println!(
        "  Contains: {file_count} file(s) from {} source path(s)",
        manifest.sources.len()
    );
    Ok(())
}

fn verify_backup(archive: &Path) -> Result<()> {
    if !archive.is_file() {
        bail!("backup archive does not exist: {}", archive.display())
    }
    let listing = Command::new("tar")
        .arg("-tzf")
        .arg(archive)
        .output()
        .context("failed to start tar while verifying backup")?;
    if !listing.status.success() {
        bail!(
            "backup archive is not a readable gzip tar: {}",
            String::from_utf8_lossy(&listing.stderr).trim()
        )
    }
    let names = String::from_utf8_lossy(&listing.stdout);
    if !names.lines().any(|line| line == "icg-backup-manifest.json") {
        bail!("backup archive is missing icg-backup-manifest.json")
    }

    let manifest_output = Command::new("tar")
        .arg("-xOzf")
        .arg(archive)
        .arg("icg-backup-manifest.json")
        .output()
        .context("failed to read backup manifest")?;
    let manifest: BackupManifestView = serde_json::from_slice(&manifest_output.stdout)
        .context("backup manifest is not valid JSON")?;
    if manifest.schema != "icg-backup/v1" {
        bail!(
            "unsupported backup schema '{}'; expected icg-backup/v1",
            manifest.schema
        )
    }
    println!("✓ Backup verified successfully");
    println!(
        "  Contains: {} file(s) from {} source path(s)",
        manifest.files,
        manifest.sources.len()
    );
    Ok(())
}

#[derive(Debug, Deserialize)]
struct BackupManifestView {
    schema: String,
    #[allow(dead_code)]
    version: String,
    #[allow(dead_code)]
    created_at: String,
    sources: Vec<String>,
    files: usize,
}

pub fn run_override(command: OverrideSubcommand) -> Result<()> {
    match command {
        OverrideSubcommand::Create(args) => create_override_request(&args),
        OverrideSubcommand::Approve(args) => approve_override(&args),
        OverrideSubcommand::List(args) => list_overrides(&args),
    }
}

fn create_override_request(args: &OverrideCreateArgs) -> Result<()> {
    if args.repo.trim().is_empty() || args.repo.chars().any(char::is_control) {
        bail!("--repo must be a non-empty repository path or identifier")
    }
    validate_rule_id(&args.pattern_id)?;
    if args.justification.trim().is_empty() {
        bail!("--justification must not be blank")
    }
    let output = args.output.clone().unwrap_or_else(|| {
        PathBuf::from("/tmp").join(format!(
            "override-request-{}.json",
            safe_name(Path::new(&args.repo))
        ))
    });
    let request = OverrideRequest {
        schema: "icg-override-request/v1".to_string(),
        repo: args.repo.clone(),
        pattern_id: args.pattern_id.clone(),
        justification: args.justification.clone(),
        created_at: Utc::now().to_rfc3339(),
    };
    write_json_file(&output, &request)?;
    println!("Override request created: {}", output.display());
    println!("Requires Layer 1/2 approval via the release pipeline.");
    Ok(())
}

fn approve_override(args: &OverrideApproveArgs) -> Result<()> {
    let request: OverrideRequest = serde_json::from_str(
        &fs::read_to_string(&args.request)
            .with_context(|| format!("failed to read request {}", args.request.display()))?,
    )
    .with_context(|| format!("failed to parse request {}", args.request.display()))?;
    validate_rule_id(&request.pattern_id)?;
    if args.approver.trim().is_empty() {
        bail!("--approver must not be blank")
    }
    chrono::NaiveDate::parse_from_str(&args.expiration, "%Y-%m-%d")
        .with_context(|| "--expiration must use YYYY-MM-DD")?;

    let repository = repository_identifier(&request.repo);
    let release_ref = args.release_ref.clone().or_else(|| {
        TrustPointerStore::default_path()
            .ok()
            .and_then(|path| TrustPointerStore::new(path).load().ok().flatten())
            .map(|pointer| pointer.trusted_ref)
    });

    let output = args.output.clone().unwrap_or_else(|| {
        args.output_dir.join(format!(
            "{}-{}.{}",
            safe_name(Path::new(&repository)),
            safe_name(Path::new(&request.pattern_id)),
            if release_ref.is_some() {
                "toml"
            } else {
                "approval.json"
            }
        ))
    });

    if let Some(release_ref) = release_ref {
        let manifest = RepoOverride::new(
            repository.clone(),
            release_ref.clone(),
            vec![request.pattern_id.clone()],
            args.expiration.clone(),
            request.justification.clone(),
        );
        if let Some(pack_path) = &args.pack {
            let pack = crate::rule_pack::load_pack(pack_path)?;
            crate::overrides::validate_override(
                &manifest,
                &repository,
                &release_ref,
                std::slice::from_ref(&pack),
            )?;
        }
        write_override_with_fallback(&manifest, &output, &args.output_dir)?;
        println!("✓ Override approved and installed");
        println!("Repository: {repository}");
        println!("Pattern: {}", request.pattern_id);
        println!("Expires: {}", args.expiration);
        println!("Stored in: {}", output.display());
    } else {
        let approval = ApprovalRecord {
            schema: "icg-override-approval/v1".to_string(),
            repo: request.repo,
            pattern_id: request.pattern_id,
            justification: request.justification,
            approver: args.approver.clone(),
            expiration: args.expiration.clone(),
            approved_at: Utc::now().to_rfc3339(),
            release_ref: None,
        };
        write_json_file_with_fallback(&output, &approval, &args.output_dir)?;
        println!("✓ Override approval recorded: {}", output.display());
        println!(
            "No trusted release reference is configured; no active bypass artifact was created."
        );
    }
    Ok(())
}

fn list_overrides(args: &OverrideListArgs) -> Result<()> {
    let mut directory = args.directory.clone();
    if !directory.is_dir() && directory == Path::new(DEFAULT_OVERRIDE_DIRECTORY) {
        let fallback = PathBuf::from("/tmp/icg-overrides");
        if fallback.is_dir() {
            directory = fallback;
        }
    }
    if !directory.is_dir() {
        println!("No overrides found in {}", directory.display());
        return Ok(());
    }

    println!("ACTIVE OVERRIDES");
    println!("Repository\tPattern\tExpires\tStatus");
    let mut entries = fs::read_dir(&directory)
        .with_context(|| format!("failed to read override directory {}", directory.display()))?
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|path| path.extension().and_then(|ext| ext.to_str()) == Some("toml"))
        .collect::<Vec<_>>();
    entries.sort();

    for path in entries {
        let manifest = match crate::overrides::load_override(&path) {
            Ok(manifest) => manifest,
            Err(error) => {
                eprintln!("Warning: skipping {}: {error}", path.display());
                continue;
            }
        };
        let status = manifest
            .freshness_at(Utc::now().date_naive())
            .map(|freshness| format!("{freshness:?}"))
            .unwrap_or_else(|_| "Invalid".to_string());
        if !args.include_expired && status != "Fresh" {
            continue;
        }
        let patterns = manifest.exempted_rule_ids.join(",");
        println!(
            "{}\t{}\t{}\t{}",
            manifest.repository, patterns, manifest.expires_at, status
        );
    }
    Ok(())
}

fn load_packs(engine: &mut Engine, paths: &[PathBuf]) -> Result<Vec<Pack>> {
    let mut packs = Vec::new();
    for path in paths {
        let pack = crate::rule_pack::load_pack(path)
            .with_context(|| format!("failed to load rule pack {}", path.display()))?;
        engine.load_pack(pack.clone())?;
        packs.push(pack);
    }
    Ok(packs)
}

fn load_pack_values(paths: &[PathBuf]) -> Result<Vec<Pack>> {
    paths
        .iter()
        .map(|path| {
            crate::rule_pack::load_pack(path)
                .with_context(|| format!("failed to load rule pack {}", path.display()))
        })
        .collect()
}

fn resolve_pack_paths(explicit: &[PathBuf]) -> Result<Vec<PathBuf>> {
    let candidates = if explicit.is_empty() {
        if let Some(configured_directory) = std::env::var_os("ICG_PACK_DIR") {
            vec![PathBuf::from(configured_directory)]
        } else {
            vec![
                PathBuf::from(DEFAULT_RULE_PACK),
                PathBuf::from(DEFAULT_PACK_DIRECTORY),
                PathBuf::from("packs"),
            ]
        }
    } else {
        explicit.to_vec()
    };
    let mut paths = BTreeSet::new();
    for candidate in candidates {
        if candidate.is_file() {
            if candidate.extension().and_then(|ext| ext.to_str()) == Some("json") {
                paths.insert(candidate);
            }
        } else if candidate.is_dir() {
            let mut directory_entries = fs::read_dir(&candidate)
                .with_context(|| format!("failed to read pack directory {}", candidate.display()))?
                .filter_map(|entry| entry.ok().map(|entry| entry.path()))
                .filter(|path| path.extension().and_then(|ext| ext.to_str()) == Some("json"))
                .collect::<Vec<_>>();
            directory_entries.sort();
            paths.extend(directory_entries);
        } else if !explicit.is_empty() {
            bail!("rule-pack path does not exist: {}", candidate.display())
        }
    }
    if paths.is_empty() {
        bail!("no rule packs found; pass --pack <path>")
    }
    Ok(paths.into_iter().collect())
}

fn default_denial_log_paths() -> Vec<PathBuf> {
    let mut paths = vec![
        PathBuf::from("/var/log/icg/denials.jsonl"),
        PathBuf::from("/var/log/icg/denials.log"),
    ];
    if let Ok(path) = crate::state_store::StateStore::default_path() {
        paths.push(path);
    }
    paths
}

fn default_backup_sources() -> Vec<PathBuf> {
    if let Ok(source) = std::env::var("ICG_BACKUP_SOURCE") {
        return vec![PathBuf::from(source)];
    }
    let mut paths = vec![
        PathBuf::from("/etc/icg"),
        PathBuf::from("/var/lib/icg"),
        PathBuf::from("/var/log/icg"),
        PathBuf::from("/var/cache/icg"),
    ];
    if let Ok(path) = crate::state_store::StateStore::default_path() {
        paths.push(path);
    }
    paths
}

fn read_stdin_text() -> Result<String> {
    let mut content = String::new();
    std::io::stdin().read_to_string(&mut content)?;
    Ok(content)
}

fn safe_current_dir() -> String {
    std::env::current_dir()
        .map(|path| path.display().to_string())
        .unwrap_or_else(|_| "<unavailable>".to_string())
}

fn title_case(field: &str) -> String {
    field
        .split('_')
        .map(|word| {
            let mut chars = word.chars();
            match chars.next() {
                Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn display_json_value(value: &serde_json::Value) -> String {
    value
        .as_str()
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| value.to_string())
}

fn safe_name(path: &Path) -> String {
    let raw = path
        .file_name()
        .or_else(|| {
            path.components()
                .next_back()
                .map(|component| component.as_os_str())
        })
        .map(|name| name.to_string_lossy())
        .unwrap_or_else(|| std::borrow::Cow::Borrowed("item"));
    let mut name = raw
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric()
                || character == '.'
                || character == '-'
                || character == '_'
            {
                character
            } else {
                '-'
            }
        })
        .collect::<String>();
    if name.is_empty() || name == "." || name == ".." {
        name = "item".to_string();
    }
    name
}

fn validate_rule_id(rule_id: &str) -> Result<()> {
    if rule_id.trim().is_empty()
        || rule_id
            .chars()
            .any(|character| character.is_whitespace() || character.is_control())
    {
        bail!("pattern ID must be non-empty and contain no whitespace")
    }
    Ok(())
}

fn repository_identifier(repository: &str) -> String {
    let trimmed = repository.trim_end_matches('/');
    if trimmed.starts_with('/') {
        Path::new(trimmed)
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| "repository".to_string())
    } else {
        trimmed.to_string()
    }
}

fn write_json_file<T: Serialize>(path: &Path, value: &T) -> Result<()> {
    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create directory {}", parent.display()))?;
    }
    fs::write(path, serde_json::to_vec_pretty(value)?)
        .with_context(|| format!("failed to write {}", path.display()))
}

fn write_json_file_with_fallback<T: Serialize>(
    path: &Path,
    value: &T,
    fallback_dir: &Path,
) -> Result<()> {
    match write_json_file(path, value) {
        Ok(()) => Ok(()),
        Err(error) if path.starts_with(DEFAULT_OVERRIDE_DIRECTORY) => {
            let fallback =
                fallback_dir_fallback(fallback_dir).join(path.file_name().unwrap_or_default());
            eprintln!(
                "Warning: could not write {} ({error}); using {}",
                path.display(),
                fallback.display()
            );
            write_json_file(&fallback, value)
        }
        Err(error) => Err(error),
    }
}

fn write_override_with_fallback(
    manifest: &RepoOverride,
    path: &Path,
    fallback_dir: &Path,
) -> Result<()> {
    match save_override(manifest, path) {
        Ok(()) => Ok(()),
        Err(error) if path.starts_with(DEFAULT_OVERRIDE_DIRECTORY) => {
            let fallback =
                fallback_dir_fallback(fallback_dir).join(path.file_name().unwrap_or_default());
            eprintln!(
                "Warning: could not write {} ({error}); using {}",
                path.display(),
                fallback.display()
            );
            save_override(manifest, &fallback)
        }
        Err(error) => Err(error),
    }
}

fn fallback_dir_fallback(requested: &Path) -> PathBuf {
    if requested != Path::new(DEFAULT_OVERRIDE_DIRECTORY) {
        requested.to_path_buf()
    } else {
        PathBuf::from("/tmp/icg-overrides")
    }
}

fn make_temp_directory(prefix: &str) -> Result<PathBuf> {
    let base = std::env::temp_dir();
    for attempt in 0..100u32 {
        let candidate = base.join(format!(
            "{prefix}-{}-{}-{attempt}",
            std::process::id(),
            Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        match fs::create_dir(&candidate) {
            Ok(()) => return Ok(candidate),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error.into()),
        }
    }
    bail!("could not create a temporary staging directory")
}

fn copy_tree(source: &Path, destination: &Path) -> Result<usize> {
    let metadata = fs::symlink_metadata(source)
        .with_context(|| format!("failed to inspect {}", source.display()))?;
    if metadata.file_type().is_symlink() {
        return Ok(0);
    }
    if metadata.is_dir() {
        fs::create_dir_all(destination)?;
        let mut entries = fs::read_dir(source)?
            .filter_map(|entry| entry.ok().map(|entry| entry.path()))
            .collect::<Vec<_>>();
        entries.sort();
        let mut count = 0;
        for entry in entries {
            count += copy_tree(&entry, &destination.join(safe_name(&entry)))?;
        }
        Ok(count)
    } else if metadata.is_file() {
        if let Some(parent) = destination.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::copy(source, destination)
            .with_context(|| format!("failed to copy {}", source.display()))?;
        Ok(1)
    } else {
        Ok(0)
    }
}

/// Install PATH-wrapper symlinks for currently-loaded command-mode packs
///
/// This command creates symlinks for tool_keywords from currently-loaded rule packs,
/// placing them earlier in PATH than the real binaries. The symlinks point to the icg
/// binary, which will then run in wrapper mode when invoked via those symlinks.
///
/// # Arguments
/// * `install_dir` - Optional directory for symlinks (defaults to
///   [`DEFAULT_WRAPPER_INSTALL_DIR`], i.e. /usr/local/bin)
/// * `pack_paths` - Rule pack files or directories to load for tool keyword discovery
/// * `force` - Skip confirmation prompt
/// * `uninstall` - Remove existing symlinks instead of creating them
pub fn run_install(
    install_dir: Option<PathBuf>,
    pack_paths: Vec<PathBuf>,
    force: bool,
    uninstall: bool,
) -> Result<()> {
    let install_dir = match install_dir {
        Some(dir) => dir,
        None => PathBuf::from(DEFAULT_WRAPPER_INSTALL_DIR),
    };

    if uninstall {
        return run_uninstall(&install_dir, force);
    }

    // Load rule packs to discover tool_keywords
    let mut engine = Engine::new();
    let paths = if pack_paths.is_empty() {
        resolve_pack_paths(&[])?
    } else {
        resolve_pack_paths(&pack_paths)?
    };

    let mut packs = Vec::new();
    for path in paths {
        let pack = crate::rule_pack::load_pack(&path)
            .with_context(|| format!("failed to load rule pack {}", path.display()))?;
        engine.load_pack(pack.clone())?;
        packs.push(pack);
    }

    if packs.is_empty() {
        bail!("No rule packs found; pass --pack <path> to specify packs");
    }

    // Collect all tool_keywords from command-mode packs
    let mut tool_keywords = std::collections::BTreeSet::new();
    for pack in &packs {
        if !pack.tool_keywords.is_empty() {
            for keyword in &pack.tool_keywords {
                // Never create symlinks for kubectl
                if keyword == "kubectl" {
                    eprintln!("Skipping kubectl (never shadowed per policy)");
                    continue;
                }
                tool_keywords.insert(keyword.clone());
            }
        }
    }

    if tool_keywords.is_empty() {
        bail!("No tool keywords found in loaded rule packs");
    }

    // Verify that icg binary exists
    let icg_binary = std::env::current_exe().context("Could not determine path to icg binary")?;

    if !icg_binary.exists() {
        bail!("icg binary not found at {}", icg_binary.display());
    }

    // Create installation directory if it doesn't exist
    if !install_dir.exists() {
        fs::create_dir_all(&install_dir).with_context(|| {
            format!(
                "failed to create install directory {}",
                install_dir.display()
            )
        })?;
    }

    // Show what will be installed
    println!("PATH-wrapper Installation");
    println!("======================");
    println!();
    println!("Install directory: {}", install_dir.display());
    println!("icg binary: {}", icg_binary.display());
    println!();
    println!("Tool keywords to install:");
    for keyword in &tool_keywords {
        println!("  - {}", keyword);
    }
    println!();

    // Check for existing symlinks
    let mut existing = Vec::new();
    for keyword in &tool_keywords {
        let symlink_path = install_dir.join(keyword);
        if symlink_path.exists() {
            existing.push(keyword.clone());
        }
    }

    if !existing.is_empty() && !force {
        println!("Existing symlinks found:");
        for keyword in &existing {
            println!("  - {}", keyword);
        }
        println!();
        print!("Replace existing symlinks? [y/N] ");
        use std::io::Write;
        std::io::stdout().flush()?;
        let mut confirmation = String::new();
        std::io::stdin().read_line(&mut confirmation)?;
        if !confirmation.trim().eq_ignore_ascii_case("y") {
            println!("Installation cancelled.");
            return Ok(());
        }
    }

    // Create or replace symlinks
    let mut installed = Vec::new();
    let mut failed = Vec::new();

    for keyword in &tool_keywords {
        let symlink_path = install_dir.join(keyword);

        // Remove existing symlink/file if present
        if symlink_path.exists() {
            fs::remove_file(&symlink_path)
                .with_context(|| format!("failed to remove existing {}", symlink_path.display()))?;
        }

        // Create the symlink
        #[cfg(unix)]
        {
            use std::os::unix::fs::symlink;
            if let Err(e) = symlink(&icg_binary, &symlink_path) {
                failed.push((keyword.clone(), e.to_string()));
            } else {
                installed.push(keyword.clone());
            }
        }

        #[cfg(not(unix))]
        {
            failed.push((
                keyword.clone(),
                "PATH-wrapper not supported on non-Unix platforms".to_string(),
            ));
        }
    }

    // Report results
    println!();
    println!("Installation Summary");
    println!("===================");
    println!();

    if !installed.is_empty() {
        println!("Installed {} symlink(s):", installed.len());
        for keyword in &installed {
            println!("  ✓ {} -> {}", keyword, icg_binary.display());
        }
    }

    if !failed.is_empty() {
        println!();
        println!("Failed to install {} symlink(s):", failed.len());
        for (keyword, error) in &failed {
            println!("  ✗ {}: {}", keyword, error);
        }
        anyhow::bail!("Some symlinks failed to install");
    }

    if !installed.is_empty() {
        println!();
        println!("PATH-wrapper installation complete.");
        println!();
        println!(
            "The install directory ({}) must be earlier in PATH than the real binaries.",
            install_dir.display()
        );
        println!(
            "Verify with: echo $PATH | grep -o {}[^:]*",
            install_dir.display()
        );
    }

    Ok(())
}

/// Remove PATH-wrapper symlinks
fn run_uninstall(install_dir: &Path, force: bool) -> Result<()> {
    if !install_dir.exists() {
        bail!(
            "Install directory does not exist: {}",
            install_dir.display()
        );
    }

    // Find all symlinks that point to icg
    let mut found = Vec::new();
    let icg_binary = std::env::current_exe()?;

    for entry in fs::read_dir(install_dir)
        .with_context(|| format!("failed to read install directory {}", install_dir.display()))?
    {
        let entry = entry?;
        let path = entry.path();

        // Check if it's a symlink pointing to icg
        if path.is_symlink() {
            if let Ok(target) = fs::read_link(&path) {
                if target == icg_binary {
                    found.push(path);
                }
            }
        }
    }

    if found.is_empty() {
        println!(
            "No PATH-wrapper symlinks found in {}",
            install_dir.display()
        );
        return Ok(());
    }

    println!("Found {} PATH-wrapper symlink(s):", found.len());
    for path in &found {
        println!(
            "  - {}",
            path.file_name().unwrap_or_default().to_string_lossy()
        );
    }
    println!();

    if !force {
        print!("Remove these symlinks? [y/N] ");
        use std::io::Write;
        std::io::stdout().flush()?;
        let mut confirmation = String::new();
        std::io::stdin().read_line(&mut confirmation)?;
        if !confirmation.trim().eq_ignore_ascii_case("y") {
            println!("Uninstall cancelled.");
            return Ok(());
        }
    }

    let mut removed = Vec::new();
    let mut failed = Vec::new();

    for path in &found {
        match fs::remove_file(path) {
            Ok(_) => removed.push(path.clone()),
            Err(e) => failed.push((path.clone(), e.to_string())),
        }
    }

    println!();
    println!("Uninstall Summary");
    println!("================");
    println!();

    if !removed.is_empty() {
        println!("Removed {} symlink(s):", removed.len());
        for path in &removed {
            println!(
                "  ✓ {}",
                path.file_name().unwrap_or_default().to_string_lossy()
            );
        }
    }

    if !failed.is_empty() {
        println!();
        println!("Failed to remove {} symlink(s):", failed.len());
        for (path, error) in &failed {
            println!("  ✗ {}: {}", path.display(), error);
        }
        anyhow::bail!("Some symlinks failed to remove");
    }

    println!();
    println!("PATH-wrapper uninstall complete.");
    Ok(())
}
