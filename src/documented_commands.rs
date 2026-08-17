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

use crate::engine::{CheckResult, CommandSource, ContentSource, Engine, InputSource};
use crate::overrides::{save_override, RepoOverride};
use crate::rule_pack::{Check, Pack};
use crate::trust_pointer::TrustPointerStore;

const DEFAULT_RULE_PACK: &str = "/etc/icg/rule-pack.json";
const DEFAULT_PACK_DIRECTORY: &str = "/etc/icg/packs";
const DEFAULT_OVERRIDE_DIRECTORY: &str = "/etc/icg/overrides";

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

    let result = if args.stdin {
        let Some((input, _raw_tool_input)) = engine.read_pre_tool_use_payload_from_stdin()? else {
            bail!("stdin did not contain a valid PreToolUse request")
        };
        let source = Engine::input_source_from_pre_tool_use(input)
            .map_err(|error| anyhow::anyhow!(error.to_string()))?
            .context("PreToolUse request did not contain a checkable tool")?;
        evaluate_input_source(&engine, source)
    } else if let Some(command) = args.command {
        engine.evaluate_command(&CommandSource::Hook(command))
    } else if let Some(path) = args.file {
        let (file_path, content) = if path == Path::new("-") {
            ("stdin.yaml".to_string(), read_stdin_text()?)
        } else {
            let content = fs::read_to_string(&path)
                .with_context(|| format!("failed to read file {}", path.display()))?;
            (path.to_string_lossy().into_owned(), content)
        };
        engine.evaluate_content(&ContentSource::Write { file_path, content })
    } else {
        bail!("one of --command, --stdin, or --file is required")
    };

    print_check_result(&result);
    Ok(())
}

fn evaluate_input_source(engine: &Engine, source: InputSource) -> CheckResult {
    match source {
        InputSource::Command(source) => engine.evaluate_command(&source),
        InputSource::Content(source) => engine.evaluate_content(&source),
        InputSource::ContentBatch(sources) => engine.evaluate_content_batch(&sources),
    }
}

fn print_check_result(result: &CheckResult) {
    match result {
        CheckResult::Allowed => println!("ALLOW: no configured rule matched"),
        CheckResult::Denied {
            reason,
            pack_id,
            pattern_id,
        } => {
            println!("DENIED: {reason}");
            println!("Pack: {pack_id}");
            println!("Pattern: {pattern_id}");
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
        Check::Predicate { predicate_name } => println!("Predicate: {predicate_name}"),
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
                    "✓ {} ({} patterns){}",
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
        vec![
            PathBuf::from(DEFAULT_RULE_PACK),
            PathBuf::from(DEFAULT_PACK_DIRECTORY),
            PathBuf::from("packs"),
        ]
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
