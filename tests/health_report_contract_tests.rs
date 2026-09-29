//! The health-report operator contract, held to the wire.
//!
//! `icg health` with no subcommand — the command the repository guidance
//! calls `health-report` — is the one-command installation check. Its
//! contract, documented in
//! `docs/operators/deployment-guide.md` ("The health-report operator
//! contract"):
//!
//! - every report invocation without a subcommand prints the same
//!   complete inventory; `--check-packs`, `--check-hooks` and
//!   `--verbose` declare what is verified, they do not narrow the output;
//! - a check flag combined with a subcommand reports that check and
//!   skips the subcommand;
//! - the report is line-oriented human text: one `✓` line per checked
//!   area, a `  - ` line per resolved pack file, and no JSON mode;
//! - packs resolve like every operator command's minus the explicit
//!   tier — health-report has no `--pack` flag: `ICG_PACK_DIR` when
//!   set, else the installed chain, with the checkout's `packs/` only
//!   as the final fallback; sources are never unioned;
//! - exit `0` when every check that ran passed; exit `1` when a
//!   resolved pack fails to load (stdout stays empty, stderr names the
//!   path), when no pack location resolves at all, or when
//!   `ICG_HOOK_CONFIG` is set and names a missing file (the report
//!   fails after the pack line); exit `2` on a usage error.
//!
//! Every test stages both pack tiers and clears the hook override, so
//! nothing here depends on whether the host running the suite has an
//! installation or hook configuration of its own.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use tempfile::TempDir;

const INSTALLED_OVERRIDE: &str = "ICG_INSTALLED_PACK_DIR";
const HOOK_OVERRIDE: &str = "ICG_HOOK_CONFIG";

/// A minimal valid pack, self-contained so shipped-pack drift cannot move
/// what these tests mean. Same shape as the pack-source drift fixtures:
/// one guarded pattern, keyword derived from the id.
fn pack_json(id: &str) -> String {
    let keyword = format!("{id}ctl");
    serde_json::to_string_pretty(&serde_json::json!({
        "id": id,
        "tool_keywords": [keyword],
        "guarded_patterns": [{
            "id": format!("{id}-destroy"),
            "enabled": true,
            "type": "command_regex",
            "regex": format!("^{keyword} destroy"),
            "tier": "tier1",
            "severity": "High",
            "explanation": format!("{id} destroy cannot be undone"),
            "destructive": true,
            "redirect": {
                "channel": "deny",
                "reason_template": format!("run {keyword} destroy --dry-run first")
            }
        }]
    }))
    .expect("fixture pack should serialize")
}

fn write_pack(dir: &Path, id: &str) {
    fs::write(dir.join(format!("{id}.json")), pack_json(id)).expect("fixture pack should write");
}

fn write_broken_pack(dir: &Path, name: &str) -> PathBuf {
    let path = dir.join(format!("{name}.json"));
    fs::write(&path, format!("{{ not json {name}")).expect("broken pack should write");
    path
}

/// A staged world: a working directory whose `packs/` carries
/// `repository_ids`, an "installed" trust directory with `installed_ids`,
/// and one extra directory for out-of-band fixtures. `installed_ids` may
/// be empty — an empty trust directory is how a test stages "nothing is
/// installed".
struct Staged {
    _dir: TempDir,
    installed: PathBuf,
    working_dir: PathBuf,
}

fn stage(installed_ids: &[&str], repository_ids: &[&str]) -> Staged {
    let dir = TempDir::new().expect("temporary directory");
    let installed = dir.path().join("installed");
    let checkout = dir.path().join("checkout");
    let repository = checkout.join("packs");
    fs::create_dir(&installed).expect("installed directory");
    fs::create_dir(&checkout).expect("checkout directory");
    fs::create_dir(&repository).expect("repository packs directory");
    for id in installed_ids {
        write_pack(&installed, id);
    }
    for id in repository_ids {
        write_pack(&repository, id);
    }
    Staged {
        _dir: dir,
        installed,
        working_dir: checkout,
    }
}

impl Staged {
    /// Run `icg health` inside the staged checkout with the staged trust
    /// directory, the staged extra environment, and no host hook
    /// configuration of its own.
    fn run(&self, args: &[&str], env: &[(&str, &Path)]) -> Output {
        let mut command = Command::new(env!("CARGO_BIN_EXE_icg"));
        command
            .args(args)
            .current_dir(&self.working_dir)
            .env(INSTALLED_OVERRIDE, &self.installed)
            .env_remove("ICG_PACK_DIR")
            .env_remove("ICG_RULE_PACK")
            .env_remove(HOOK_OVERRIDE);
        for (key, value) in env {
            command.env(key, value);
        }
        command.output().expect("icg should run")
    }

    fn run_plain(&self, args: &[&str]) -> Output {
        self.run(args, &[])
    }
}

fn stdout(output: &Output) -> String {
    String::from_utf8(output.stdout.clone()).expect("stdout should be UTF-8")
}

fn stderr(output: &Output) -> String {
    String::from_utf8(output.stderr.clone()).expect("stderr should be UTF-8")
}

// --- the report mode prints the same complete inventory every time ------

/// `icg health`, `--verbose`, `--check-packs` and `--check-hooks` — every
/// report invocation without a subcommand — print the identical complete
/// inventory. The flags declare what the report verifies; they do not
/// select a subset of it.
#[test]
fn every_report_invocation_without_a_subcommand_prints_the_same_inventory() {
    let staged = stage(&["alpha", "beta"], &[]);
    let bare = staged.run_plain(&["health"]);
    let verbose = staged.run_plain(&["health", "--verbose"]);
    let check_packs = staged.run_plain(&["health", "--check-packs"]);
    let check_hooks = staged.run_plain(&["health", "--check-hooks"]);

    for output in [&bare, &verbose, &check_packs, &check_hooks] {
        assert!(
            output.status.success(),
            "a healthy staged world should report success: {}",
            stderr(output)
        );
    }
    assert_eq!(stdout(&bare), stdout(&verbose));
    assert_eq!(stdout(&bare), stdout(&check_packs));
    assert_eq!(stdout(&bare), stdout(&check_hooks));

    let text = stdout(&bare);
    assert!(text.contains("✓ All rule packs valid"), "{text}");
    assert!(text.contains("✓ Claude Code hook configured"), "{text}");
    assert!(
        text.contains("✓ Rule packs: 2 packs loaded"),
        "the inventory counts resolved pack files: {text}"
    );
}

/// The inventory names the binary and its version, lists each resolved
/// pack with its pattern count, and names the default state-store and
/// denial-log locations.
#[test]
fn the_inventory_names_the_binary_each_pack_and_the_state_locations() {
    let staged = stage(&["alpha", "beta"], &[]);
    let output = staged.run_plain(&["health"]);
    assert!(output.status.success(), "{}", stderr(&output));
    let text = stdout(&output);

    let expected_version = env!("CARGO_PKG_VERSION");
    assert!(
        text.contains(&format!(
            "✓ icg binary: /usr/local/bin/icg v{expected_version}"
        )),
        "the inventory names the installed binary path and version: {text}"
    );
    assert!(
        text.contains("  - alpha (1 patterns)"),
        "each resolved pack is listed with its pattern count: {text}"
    );
    assert!(
        text.contains("  - beta (1 patterns)"),
        "each resolved pack is listed with its pattern count: {text}"
    );
    assert!(text.contains("✓ Claude Code hook: Configured"), "{text}");
    assert!(
        text.contains("✓ State store: /var/lib/icg/state.db"),
        "{text}"
    );
    assert!(
        text.contains("✓ Denial log: /var/log/icg/denials.log"),
        "{text}"
    );
}

/// A check flag before a subcommand is the one narrow report form: it
/// runs that check and skips the subcommand entirely. Pinned because it
/// is the only invocation whose output is a subset — and because a
/// reader combining the two should know the subcommand never ran.
#[test]
fn a_check_flag_before_a_subcommand_reports_that_check_and_skips_it() {
    let staged = stage(&["alpha"], &[]);
    let output = staged.run_plain(&["health", "--check-packs", "status"]);
    assert!(
        output.status.success(),
        "the pack check itself passes: {}",
        stderr(&output)
    );
    assert_eq!(
        stdout(&output),
        "✓ All rule packs valid\n",
        "the narrow report prints only the named check"
    );
}

// --- failure semantics --------------------------------------------------

/// A resolved pack that fails to load aborts the report before any line
/// is printed — stdout stays empty, stderr names the path with `✗` — and
/// it wins over every later check: even a also-missing hook configuration
/// is never reached.
#[test]
fn an_unreadable_pack_aborts_the_report_before_any_line_is_printed() {
    let staged = stage(&["alpha"], &[]);
    let broken = write_broken_pack(&staged.installed, "broken");

    let output = staged.run_plain(&["health"]);
    assert_eq!(
        output.status.code(),
        Some(1),
        "an unreadable pack fails the report"
    );
    assert_eq!(stdout(&output), "", "no line is printed before the failure");
    let err = stderr(&output);
    assert!(
        err.contains("✗") && err.contains(broken.to_string_lossy().as_ref()),
        "stderr names the unreadable path: {err}"
    );

    // Pack loading runs first, so the hook check never runs.
    let missing_hook = staged.working_dir.join("no-hook-config.json");
    let output = staged.run(
        &["health", "--check-hooks"],
        &[("ICG_HOOK_CONFIG", &missing_hook)],
    );
    assert_eq!(output.status.code(), Some(1));
    assert!(
        stderr(&output).contains(broken.to_string_lossy().as_ref()),
        "the pack failure is the one reported: {}",
        stderr(&output)
    );
}

/// `ICG_HOOK_CONFIG` set and naming a missing file fails the report — on
/// every report invocation, because every subcommand-less invocation is
/// the full report — after the pack line has printed.
#[test]
fn a_missing_hook_config_fails_the_report_after_the_pack_line() {
    let staged = stage(&["alpha"], &[]);
    let missing = staged.working_dir.join("no-hook-config.json");

    for args in [&["health"][..], &["health", "--check-hooks"][..]] {
        let output = staged.run(args, &[("ICG_HOOK_CONFIG", &missing)]);
        assert_eq!(
            output.status.code(),
            Some(1),
            "a missing hook configuration fails {args:?}: {}",
            stderr(&output)
        );
        assert_eq!(
            stdout(&output),
            "✓ All rule packs valid\n",
            "the pack line has printed when the hook check fails"
        );
        let err = stderr(&output);
        assert!(
            err.contains("✗ Claude Code hook configuration not found")
                && err.contains(missing.to_string_lossy().as_ref()),
            "stderr names the missing path: {err}"
        );
    }
}

/// The hook check is a presence check on `ICG_HOOK_CONFIG` alone: a file
/// at the path passes, and an unset variable prints the hook line
/// unconditionally — the report does not look for any default hook
/// configuration.
#[test]
fn the_hook_check_is_a_presence_check_on_icg_hook_config() {
    let staged = stage(&["alpha"], &[]);

    let present = staged.working_dir.join("hook-config.json");
    fs::write(&present, "{}").expect("hook configuration should write");
    let output = staged.run(
        &["health", "--check-hooks"],
        &[("ICG_HOOK_CONFIG", &present)],
    );
    assert!(output.status.success(), "{}", stderr(&output));
    assert!(
        stdout(&output).contains("✓ Claude Code hook configured"),
        "{}",
        stdout(&output)
    );

    let output = staged.run_plain(&["health", "--check-hooks"]);
    assert!(output.status.success(), "{}", stderr(&output));
    assert!(
        stdout(&output).contains("✓ Claude Code hook configured"),
        "unset ICG_HOOK_CONFIG skips the check and prints the line: {}",
        stdout(&output)
    );
}

/// No resolvable pack location at all — an empty trust chain, a checkout
/// without `packs/` — fails the report.
#[test]
fn no_resolvable_pack_location_fails_the_report() {
    let staged = stage(&[], &[]);
    let output = staged.run_plain(&["health"]);
    assert_eq!(output.status.code(), Some(1));
    assert_eq!(stdout(&output), "");
    let err = stderr(&output);
    assert!(
        err.contains("no rule packs found"),
        "the failure says no packs resolved: {err}"
    );
}

// --- pack source resolution ---------------------------------------------

/// `ICG_PACK_DIR` is the one location consulted: neither the installed
/// chain nor the working directory's `packs/` is read while it is set.
#[test]
fn icg_pack_dir_is_the_only_location_consulted_when_set() {
    let staged = stage(&["installed-only"], &["checkout-only"]);
    let mut command = Command::new(env!("CARGO_BIN_EXE_icg"));
    command
        .args(["health"])
        .current_dir(&staged.working_dir)
        .env("ICG_PACK_DIR", &staged.installed)
        .env_remove(INSTALLED_OVERRIDE)
        .env_remove("ICG_RULE_PACK")
        .env_remove(HOOK_OVERRIDE);
    let output = command.output().expect("icg should run");

    assert!(output.status.success(), "{}", stderr(&output));
    let text = stdout(&output);
    assert!(text.contains("✓ Rule packs: 1 packs loaded"), "{text}");
    assert!(text.contains("  - installed-only (1 patterns)"), "{text}");
    assert!(
        !text.contains("checkout-only"),
        "the working directory is not consulted: {text}"
    );
}

/// A checkout can be ahead of the installed release without making the
/// installed report unhealthy. The selected installed source is loaded as a
/// whole, so checkout-only files and duplicate ids in the checkout are not
/// loaded; `pack-drift` is the explicit comparison for that state.
#[test]
fn a_drifted_checkout_does_not_change_the_selected_health_source() {
    let staged = stage(&["alpha", "installed-only"], &["alpha", "checkout-only"]);
    let output = staged.run_plain(&["health"]);
    assert!(output.status.success(), "{}", stderr(&output));

    let text = stdout(&output);
    assert!(
        text.contains("✓ Rule packs: 2 packs loaded"),
        "the selected installed source counts: {text}"
    );
    assert_eq!(text.matches("  - alpha (1 patterns)").count(), 1);
    assert!(!text.contains("checkout-only"));
}

/// An authoritative but unavailable `ICG_PACK_DIR` is a failed health
/// source, not a reason to fall back to the installed or checkout packs.
#[test]
fn an_unavailable_explicit_pack_source_does_not_fall_back() {
    let staged = stage(&["installed-only"], &["checkout-only"]);
    let missing = staged.working_dir.join("missing-packs");
    let output = staged.run(&["health"], &[("ICG_PACK_DIR", &missing)]);

    assert_eq!(output.status.code(), Some(1));
    assert_eq!(stdout(&output), "");
    let err = stderr(&output);
    assert!(
        err.contains("ICG_PACK_DIR") && err.contains("does not exist"),
        "the authoritative source failure is named: {err}"
    );
    assert!(!err.contains("installed-only"), "{err}");
    assert!(!err.contains("checkout-only"), "{err}");
}

// --- machine-readable output ---------------------------------------------

/// The report is line-oriented human text: every line is a `✓` verdict or
/// a `  - ` pack listing, and there is no JSON mode — a structured-output
/// flag is rejected as a usage error. Automation reads `coverage --list
/// --format json`, `catalog --json` or `icg monitor` instead.
#[test]
fn the_report_is_line_oriented_text_with_no_json_mode() {
    let staged = stage(&["alpha"], &[]);
    let output = staged.run_plain(&["health"]);
    assert!(output.status.success(), "{}", stderr(&output));
    for line in stdout(&output).lines() {
        assert!(
            line.starts_with("✓ ") || line.starts_with("  - "),
            "every report line is a verdict or a pack listing: {line:?}"
        );
        assert!(
            !line.contains('{'),
            "no line carries a JSON object: {line:?}"
        );
    }

    let output = staged.run_plain(&["health", "--json"]);
    assert_eq!(
        output.status.code(),
        Some(2),
        "a structured-output flag is a usage error: {}",
        stderr(&output)
    );
    assert!(
        stderr(&output).contains("--json"),
        "the usage error names the rejected flag: {}",
        stderr(&output)
    );
}

// --- the documented contract matches the report -------------------------

/// The deployment guide's health-report contract section describes the
/// report this suite pins. Hold the section's transcript to the wire: the
/// stable verdict lines it prints must be exactly the lines a real report
/// prints, and the surface names it documents must exist. The pack-count
/// line is held to packs/ by the shared pack-count guard, not here.
#[test]
fn the_operator_contract_doc_matches_the_report_it_documents() {
    let doc = fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("docs/operators/deployment-guide.md"),
    )
    .expect("deployment guide should be readable");
    let start = doc
        .find("### The health-report operator contract")
        .expect("the deployment guide should carry the health-report contract section");
    let section = &doc[start..];

    let staged = stage(&["alpha"], &[]);
    let report = stdout(&staged.run_plain(&["health"]));
    for verdict in [
        "✓ All rule packs valid",
        "✓ Claude Code hook configured",
        "✓ Claude Code hook: Configured",
        "✓ State store: /var/lib/icg/state.db",
        "✓ Denial log: /var/log/icg/denials.log",
    ] {
        assert!(
            section.contains(verdict),
            "the documented transcript should show {verdict:?}"
        );
        assert!(
            report.contains(verdict),
            "the documented verdict {verdict:?} should be what the report prints"
        );
    }

    for surface in [
        "`--check-packs`",
        "`--check-hooks`",
        "`--verbose`",
        "`ICG_PACK_DIR`",
        "`ICG_INSTALLED_PACK_DIR`",
        "`ICG_HOOK_CONFIG`",
    ] {
        assert!(
            section.contains(surface),
            "the contract section should document {surface}"
        );
    }
}
