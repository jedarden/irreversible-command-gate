//! Pack-source resolution is labeled, and the shadow is loud.
//!
//! Operator commands resolve the installed trust directory *and* the
//! working directory's `packs/` and report the union, while the hook reads
//! only the trust directory. A checkout ahead of the deployed release
//! therefore reports coverage the installed hook does not enforce — the
//! drift that hid codinghome's stuck v0.1.61 install (the checkout carried
//! a kubectl pack the trust directory did not). These tests pin the
//! labeling that makes that state visible: the `Pack source:` lines, the
//! shadow warning on the check and coverage front-ends, and `icg
//! pack-drift`, which compares the deployed set against the release
//! artifact.
//!
//! Every test stages both sides: `ICG_INSTALLED_PACK_DIR` replaces the
//! `/etc/icg` chain for operator commands, and the working directory is a
//! temporary checkout. Nothing here depends on whether the host running
//! the suite has an installation of its own.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use serde_json::Value;
use tempfile::TempDir;

const INSTALLED_OVERRIDE: &str = "ICG_INSTALLED_PACK_DIR";

/// A minimal valid pack, self-contained so shipped-pack drift cannot move
/// what the fixtures below mean. The keyword derives from the id, so the
/// same id stages byte-identical files on both sides of a comparison and a
/// command can be written against it: `<id>ctl destroy`.
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

fn write_pack(dir: &Path, id: &str) -> PathBuf {
    let path = dir.join(format!("{id}.json"));
    fs::write(&path, pack_json(id)).expect("fixture pack should write");
    path
}

/// A staged world: an "installed" trust directory with `installed_ids`, a
/// working directory whose `packs/` carries `repository_ids`, and the
/// binary to run against them.
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
    /// Run `icg` inside the staged checkout with the staged trust directory.
    fn run(&self, args: &[&str]) -> Output {
        let mut command = Command::new(env!("CARGO_BIN_EXE_icg"));
        command
            .args(args)
            .current_dir(&self.working_dir)
            .env(INSTALLED_OVERRIDE, &self.installed)
            .env_remove("ICG_PACK_DIR")
            .env_remove("ICG_RULE_PACK");
        command.output().expect("icg should run")
    }
}

fn stdout(output: &Output) -> String {
    String::from_utf8(output.stdout.clone()).expect("stdout should be UTF-8")
}

fn stderr(output: &Output) -> String {
    String::from_utf8(output.stderr.clone()).expect("stderr should be UTF-8")
}

// --- coverage --list: the source header and the shadow warning ----------

/// Both tiers consulted, no shadow: the header names each location and its
/// tier, and no warning runs — a reader can see the report describes two
/// sources and that they agree.
#[test]
fn coverage_list_labels_each_pack_source() {
    let staged = stage(&["alpha"], &["alpha"]);
    let output = staged.run(&["coverage", "--list"]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let text = stdout(&output);
    assert!(
        text.contains(&format!(
            "Pack source: {} (installed)",
            staged.installed.display()
        )),
        "the installed tier is named: {text}"
    );
    assert!(
        text.contains("Pack source: packs (working directory)"),
        "the working-directory tier is named: {text}"
    );
    assert!(
        !text.contains("WARNING (pack source)"),
        "matching sets must not warn: {text}"
    );
    assert!(
        text.contains("✓ pack alpha"),
        "the packs still list: {text}"
    );
}

/// The incident, pinned: the working directory carries a pack the
/// installed set lacks, so the union reports coverage the installed hook
/// does not enforce — and the report says so, naming the offending pack.
#[test]
fn coverage_list_warns_when_the_working_directory_shadows_the_installed_set() {
    let staged = stage(&["alpha"], &["alpha", "kubectl"]);
    let output = staged.run(&["coverage", "--list"]);
    assert!(
        output.status.success(),
        "a shadow is a warning, not a refusal: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let text = stdout(&output);
    assert!(
        text.contains("WARNING (pack source)"),
        "the shadow is announced: {text}"
    );
    assert!(
        text.contains("shadows the installed set with: kubectl"),
        "the warning names the pack: {text}"
    );
    assert!(
        text.contains("not what the installed hook enforces"),
        "the warning states the consequence: {text}"
    );
}

/// A bare checkout — nothing installed — has no installed set to shadow.
/// The working-directory tier still labels itself, and no warning fires.
#[test]
fn a_bare_checkout_names_only_the_working_directory_and_does_not_warn() {
    let staged = stage(&[], &["alpha"]);
    let output = staged.run(&["coverage", "--list"]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let text = stdout(&output);
    assert!(
        text.contains("Pack source: packs (working directory)"),
        "the working-directory tier is named: {text}"
    );
    assert!(
        !text.contains("(installed)"),
        "nothing counts as installed: {text}"
    );
    assert!(!text.contains("WARNING (pack source)"));
}

/// Explicit `--pack` paths are the caller's deliberate choice: no source
/// header, no shadow warning, exactly the packs they named.
#[test]
fn explicit_pack_paths_are_never_labeled_or_warned() {
    let staged = stage(&["alpha"], &["alpha", "kubectl"]);
    let output = staged.run(&["coverage", "--list", "--pack", "packs/kubectl.json"]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let text = stdout(&output);
    assert!(
        !text.contains("Pack source:"),
        "an explicit selection carries no source header: {text}"
    );
    assert!(!text.contains("WARNING (pack source)"));
    assert!(text.contains("✓ pack kubectl"));
}

/// `ICG_INSTALLED_PACK_DIR` replaces the `/etc/icg` chain for operator
/// commands but not the working-directory tier: both sides of the staged
/// world report, each resolved from its own location.
#[test]
fn the_installed_override_replaces_the_etc_chain_but_not_the_working_directory() {
    let staged = stage(&["alpha"], &["beta"]);
    let output = staged.run(&["coverage", "--list", "--format", "json"]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let report: Value = serde_json::from_slice(&output.stdout).expect("the document should parse");
    let mut by_id = BTreeMap::new();
    for pack in report["packs"].as_array().unwrap() {
        by_id.insert(
            pack["id"].as_str().unwrap().to_string(),
            pack["path"].as_str().unwrap().to_string(),
        );
    }
    assert_eq!(by_id.keys().collect::<Vec<_>>(), vec!["alpha", "beta"]);
    assert!(
        by_id["alpha"].starts_with(&staged.installed.display().to_string()),
        "alpha resolves from the staged install: {:?}",
        by_id
    );
    assert!(
        by_id["beta"].starts_with("packs/"),
        "beta resolves from the working directory: {:?}",
        by_id
    );
}

/// `ICG_PACK_DIR` still replaces the whole search — the explicit tier wins
/// over the installed override, and nothing else is consulted.
#[test]
fn icg_pack_dir_still_replaces_every_default_source() {
    let staged = stage(&["alpha"], &["beta"]);
    let solo = staged._dir.path().join("solo");
    fs::create_dir(&solo).expect("solo directory");
    write_pack(&solo, "gamma");
    let mut command = Command::new(env!("CARGO_BIN_EXE_icg"));
    command
        .args(["coverage", "--list", "--format", "json"])
        .current_dir(&staged.working_dir)
        .env("ICG_PACK_DIR", &solo)
        .env(INSTALLED_OVERRIDE, &staged.installed)
        .env_remove("ICG_RULE_PACK");
    let output = command.output().expect("icg should run");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let report: Value = serde_json::from_slice(&output.stdout).expect("the document should parse");
    let ids: Vec<&str> = report["packs"]
        .as_array()
        .unwrap()
        .iter()
        .map(|pack| pack["id"].as_str().unwrap())
        .collect();
    assert_eq!(ids, vec!["gamma"], "only the ICG_PACK_DIR pack loads");
}

// --- check: the shadow warning is a --debug diagnostic ------------------

/// The check front-end names the pack source and the shadow on stderr —
/// but only under `--debug`, where diagnostics belong.
#[test]
fn check_debug_names_the_pack_source_and_warns_on_shadow() {
    let staged = stage(&["alpha"], &["alpha", "kubectl"]);
    let output = staged.run(&[
        "check",
        "--debug",
        "--command",
        "alphactl destroy everything",
    ]);
    assert!(output.status.success());
    let err = stderr(&output);
    assert!(
        err.contains(&format!(
            "Pack source: {} (installed)",
            staged.installed.display()
        )),
        "check --debug names the installed tier: {err}"
    );
    assert!(
        err.contains("Pack source: packs (working directory)"),
        "check --debug names the working-directory tier: {err}"
    );
    assert!(
        err.contains("WARNING (pack source)") && err.contains("kubectl"),
        "check --debug carries the shadow warning: {err}"
    );
}

/// The plain check keeps its pinned stream contract: the verdict opens
/// stdout, stderr carries no source chatter, shadow or not.
#[test]
fn a_plain_check_stays_silent_about_the_pack_source() {
    let staged = stage(&["alpha"], &["alpha", "kubectl"]);
    let output = staged.run(&["check", "--command", "alphactl destroy everything"]);
    assert!(output.status.success());
    let out = stdout(&output);
    assert!(
        out.starts_with("DENIED by icg"),
        "the verdict still opens stdout: {out}"
    );
    assert!(
        !out.contains("Pack source:") && !out.contains("WARNING (pack source)"),
        "stdout carries no source chatter: {out}"
    );
    assert!(
        stderr(&output).is_empty(),
        "plain check keeps stderr fault-only: {}",
        stderr(&output)
    );
}

// --- status -------------------------------------------------------------

/// `icg status` states which pack source the operator commands loaded and
/// warns on the shadow, next to the installed directory the hook reads.
#[test]
fn status_names_the_operator_pack_source_and_warns() {
    let staged = stage(&["alpha"], &["alpha", "kubectl"]);
    let output = staged.run(&["status"]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let text = stdout(&output);
    assert!(
        text.contains("## Operator Pack Source"),
        "status has a pack-source section: {text}"
    );
    assert!(
        text.contains(&format!(
            "Pack source: {} (installed)",
            staged.installed.display()
        )),
        "the installed tier is named: {text}"
    );
    assert!(
        text.contains("Pack source: packs (working directory)")
            && text.contains("WARNING (pack source)")
            && text.contains("kubectl"),
        "the shadow is announced: {text}"
    );
}

// --- pack-drift ---------------------------------------------------------

/// Identical sets: exit 0, and the report says so with the pack count.
#[test]
fn pack_drift_passes_when_the_installed_set_matches_the_artifact() {
    let staged = stage(&["alpha", "beta"], &["alpha", "beta"]);
    let output = staged.run(&[
        "pack-drift",
        "--installed",
        staged.installed.to_str().unwrap(),
    ]);
    assert_eq!(
        output.status.code(),
        Some(0),
        "matching sets exit 0: {}",
        stderr(&output)
    );
    let text = stdout(&output);
    assert!(text.contains(&format!("Installed: {}", staged.installed.display())));
    assert!(text.contains("Reference: packs"));
    assert!(
        text.contains("OK: no drift — 2 pack(s) byte-identical"),
        "{text}"
    );
}

/// The incident shape: the release artifact ships a pack the installed
/// trust directory does not carry.
#[test]
fn pack_drift_reports_a_pack_missing_from_the_installed_set() {
    let staged = stage(&["alpha"], &["alpha", "kubectl"]);
    let output = staged.run(&[
        "pack-drift",
        "--installed",
        staged.installed.to_str().unwrap(),
    ]);
    assert_eq!(
        output.status.code(),
        Some(1),
        "drift exits 1: {}",
        stderr(&output)
    );
    let text = stdout(&output);
    assert!(
        text.contains("MISSING FROM INSTALLED: kubectl"),
        "the missing pack is named: {text}"
    );
    assert!(text.contains("DRIFT: 1 difference(s)"), "{text}");
}

/// The other direction: the installed set carries a pack the release
/// artifact does not — deployed enforcement beyond the reviewed release.
#[test]
fn pack_drift_reports_an_installed_pack_the_artifact_lacks() {
    let staged = stage(&["alpha", "stray"], &["alpha"]);
    let output = staged.run(&[
        "pack-drift",
        "--installed",
        staged.installed.to_str().unwrap(),
    ]);
    assert_eq!(output.status.code(), Some(1));
    let text = stdout(&output);
    assert!(
        text.contains("NOT IN REFERENCE: stray"),
        "the extra installed pack is named: {text}"
    );
}

/// Same pack id, different bytes: the changed pack is named with both
/// sides' rule counts.
#[test]
fn pack_drift_reports_a_changed_pack() {
    let dir = TempDir::new().expect("temporary directory");
    let installed = dir.path().join("installed");
    let checkout = dir.path().join("checkout");
    let repository = checkout.join("packs");
    fs::create_dir(&installed).expect("installed directory");
    fs::create_dir(&checkout).expect("checkout directory");
    fs::create_dir(&repository).expect("repository packs directory");
    write_pack(&installed, "alpha");
    let reference = write_pack(&repository, "alpha");
    // One extra guarded pattern on the reference side: same id, drift.
    let mut pack: Value =
        serde_json::from_str(&fs::read_to_string(&reference).expect("pack readable")).unwrap();
    pack["guarded_patterns"]
        .as_array_mut()
        .expect("guarded_patterns array")
        .push(serde_json::json!({
            "id": "alpha-wipe",
            "enabled": true,
            "type": "command_regex",
            "regex": "^alphactl wipe",
            "tier": "tier1",
            "severity": "High",
            "explanation": "alphactl wipe cannot be undone",
            "destructive": true,
            "redirect": {
                "channel": "deny",
                "reason_template": "run alphactl wipe --dry-run first"
            }
        }));
    fs::write(&reference, serde_json::to_string_pretty(&pack).unwrap())
        .expect("reference pack rewritten");

    let mut command = Command::new(env!("CARGO_BIN_EXE_icg"));
    command
        .args([
            "pack-drift",
            "--installed",
            installed.to_str().unwrap(),
            "--reference",
            repository.to_str().unwrap(),
        ])
        .env_remove("ICG_PACK_DIR")
        .env_remove("ICG_RULE_PACK")
        .env_remove(INSTALLED_OVERRIDE);
    let output = command.output().expect("icg should run");
    assert_eq!(output.status.code(), Some(1));
    let text = stdout(&output);
    assert!(text.contains("CHANGED: alpha"), "{text}");
    assert!(
        text.contains("(1 guarded patterns) vs reference") && text.contains("(2 guarded patterns)"),
        "both sides' rule counts are reported: {text}"
    );
}

/// The legacy single-file artifact is a legal installed side: it compares
/// as the one pack it carries.
#[test]
fn pack_drift_accepts_a_legacy_single_file_install() {
    let staged = stage(&[], &["rule-pack"]);
    let legacy = staged._dir.path().join("rule-pack.json");
    fs::write(&legacy, pack_json("rule-pack")).expect("legacy artifact written");
    let output = staged.run(&[
        "pack-drift",
        "--installed",
        legacy.to_str().unwrap(),
        "--reference",
        "packs",
    ]);
    assert_eq!(
        output.status.code(),
        Some(0),
        "one legacy pack matching one artifact pack: {}",
        stderr(&output)
    );
    assert!(stdout(&output).contains("OK: no drift — 1 pack(s)"));
}

/// A nonexistent installed location cannot run the check at all: that is
/// a usage fault (exit 2), distinct from a drift finding.
#[test]
fn pack_drift_reports_a_nonexistent_installed_location_as_a_usage_fault() {
    let staged = stage(&[], &["alpha"]);
    let missing = staged._dir.path().join("no-such-install");
    let output = staged.run(&["pack-drift", "--installed", missing.to_str().unwrap()]);
    assert_eq!(
        output.status.code(),
        Some(2),
        "a nonexistent location cannot run"
    );
    let err = stderr(&output);
    assert!(
        err.contains("installed pack location does not exist"),
        "the failure names the side and the path: {err}"
    );
    assert!(stdout(&output).is_empty(), "no report is emitted");
}

/// The default installed side is the hook's chain: `ICG_RULE_PACK`, as the
/// registered hook resolves it — the drift check compares what the hook
/// enforces, not an arbitrary directory.
#[test]
fn pack_drift_defaults_to_the_hook_chain() {
    let dir = TempDir::new().expect("temporary directory");
    let hook_pack_dir = dir.path().join("hook-packs");
    let checkout = dir.path().join("checkout");
    let repository = checkout.join("packs");
    fs::create_dir(&hook_pack_dir).expect("hook pack directory");
    fs::create_dir(&checkout).expect("checkout directory");
    fs::create_dir(&repository).expect("repository packs directory");
    write_pack(&hook_pack_dir, "alpha");
    write_pack(&repository, "alpha");
    write_pack(&repository, "kubectl");

    let mut command = Command::new(env!("CARGO_BIN_EXE_icg"));
    command
        .args(["pack-drift"])
        .current_dir(&checkout)
        .env("ICG_RULE_PACK", &hook_pack_dir)
        .env_remove("ICG_PACK_DIR")
        .env_remove(INSTALLED_OVERRIDE);
    let output = command.output().expect("icg should run");
    assert_eq!(
        output.status.code(),
        Some(1),
        "the drift is found: {}",
        stdout(&output)
    );
    let text = stdout(&output);
    assert!(
        text.contains(&format!("Installed: {}", hook_pack_dir.display())),
        "the installed side is the hook chain's resolution: {text}"
    );
    assert!(
        text.contains("MISSING FROM INSTALLED: kubectl"),
        "the comparison is against what the hook reads: {text}"
    );
}

/// `pack-drift` honors the staged override when no `--installed` path is
/// named: the staged trust directory stands in for the hook chain, so the
/// check is stageable on a host that cannot touch `/etc/icg`.
#[test]
fn pack_drift_reads_the_installed_override_when_no_path_is_named() {
    let staged = stage(&["alpha"], &["alpha", "kubectl"]);
    let output = staged.run(&["pack-drift"]);
    assert_eq!(
        output.status.code(),
        Some(1),
        "the staged install lacks kubectl, so the checkout's extra pack is \
         drift: {}",
        stderr(&output)
    );
    let text = stdout(&output);
    assert!(
        text.contains(&format!("Installed: {}", staged.installed.display())),
        "the installed side is the staged override, not the host's chain: {text}"
    );
    assert!(
        text.contains("MISSING FROM INSTALLED: kubectl"),
        "the comparison is against the staged install: {text}"
    );
}

/// Usage faults exit 2 with the reason on stderr, keeping "could not run"
/// distinct from "drift found".
#[test]
fn pack_drift_usage_faults_exit_two() {
    let staged = stage(&["alpha"], &[]);
    // A reference directory with no .json packs.
    let empty = staged._dir.path().join("empty-reference");
    fs::create_dir(&empty).expect("empty reference directory");
    let output = staged.run(&[
        "pack-drift",
        "--installed",
        staged.installed.to_str().unwrap(),
        "--reference",
        empty.to_str().unwrap(),
    ]);
    assert_eq!(output.status.code(), Some(2));
    assert!(stderr(&output).starts_with("Error: "));
    assert!(
        stdout(&output).is_empty(),
        "no report is emitted: {}",
        stdout(&output)
    );
}
