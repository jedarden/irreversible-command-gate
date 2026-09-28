//! Pack-source resolution is labeled, and the shadow is loud.
//!
//! Operator commands choose one source by precedence, while the hook reads
//! only the installed trust directory. A checkout ahead of the deployed
//! release must not silently add checkout-only packs to operator coverage.
//! These tests pin the selected-source labels and `icg pack-drift`, which
//! remains the explicit comparison between the deployed set and the release
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

/// The installed tier wins as a whole, even when the checkout has a matching
/// pack. The report has one source label and no checkout tier.
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
    assert!(!text.contains("Pack source: packs"));
    assert!(!text.contains("WARNING (pack source)"));
    assert!(
        text.contains("✓ pack alpha"),
        "the packs still list: {text}"
    );
}

/// The working directory carries a pack the installed set lacks, but the
/// installed source wins and the checkout-only pack is absent.
#[test]
fn coverage_list_warns_when_the_working_directory_shadows_the_installed_set() {
    let staged = stage(&["alpha"], &["alpha", "kubectl"]);
    let output = staged.run(&["coverage", "--list"]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let text = stdout(&output);
    assert!(text.contains("Pack source:") && text.contains("(installed)"));
    assert!(
        !text.contains("kubectl"),
        "checkout-only pack leaked: {text}"
    );
    assert!(!text.contains("WARNING (pack source)"));
}

/// A bare checkout — nothing installed — has no installed set to shadow.
/// The working-directory tier still labels itself, and no warning fires.
#[test]
fn a_bare_checkout_names_only_the_working_directory_and_does_not_warn() {
    let staged = stage(&[], &["alpha"]);
    fs::remove_dir(&staged.installed).expect("missing installed override should be staged");
    let output = staged.run(&["coverage", "--list"]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let text = stdout(&output);
    assert!(text.contains("Pack source: packs (repository)"));
    assert!(
        !text.contains("(installed)"),
        "nothing counts as installed: {text}"
    );
    assert!(!text.contains("WARNING (pack source)"));
}

/// Explicit `--pack` paths are the caller's deliberate choice and are
/// labeled as an explicit source.
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
    assert!(text.contains("Pack source: explicit (explicit; --pack)"));
    assert!(!text.contains("WARNING (pack source)"));
    assert!(text.contains("✓ pack kubectl"));
}

/// `ICG_INSTALLED_PACK_DIR` replaces the `/etc/icg` chain for operator
/// commands and wins over the working-directory fallback.
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
    assert_eq!(by_id.keys().collect::<Vec<_>>(), vec!["alpha"]);
    assert!(
        by_id["alpha"].starts_with(&staged.installed.display().to_string()),
        "alpha resolves from the staged install: {:?}",
        by_id
    );
    assert_eq!(report["pack_source"]["origin"], "installed");
    assert_eq!(
        report["pack_source"]["root"],
        staged.installed.to_string_lossy().as_ref()
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

#[test]
fn catalog_json_labels_the_selected_installed_source() {
    let staged = stage(&["alpha"], &["checkout"]);
    let output = staged.run(&["catalog", "--json"]);
    assert!(output.status.success(), "{}", stderr(&output));
    let report: Value = serde_json::from_slice(&output.stdout).expect("catalog should be JSON");
    assert_eq!(report["format"], "icg-catalog/v2");
    assert_eq!(report["pack_source"]["origin"], "installed");
    assert_eq!(
        report["pack_source"]["root"],
        staged.installed.to_string_lossy().as_ref()
    );
    assert!(report["never"]
        .as_array()
        .unwrap()
        .iter()
        .any(|event| event["pack"] == "alpha"));
    assert!(!report["never"]
        .as_array()
        .unwrap()
        .iter()
        .any(|event| { event["pack"] == "checkout" }));
}

#[test]
fn missing_icg_pack_dir_is_authoritative_and_does_not_fall_back() {
    let staged = stage(&["alpha"], &["checkout"]);
    let missing = staged._dir.path().join("missing-override");
    let output = Command::new(env!("CARGO_BIN_EXE_icg"))
        .args(["coverage", "--list", "--format", "json"])
        .current_dir(&staged.working_dir)
        .env("ICG_PACK_DIR", &missing)
        .env(INSTALLED_OVERRIDE, &staged.installed)
        .env_remove("ICG_RULE_PACK")
        .output()
        .expect("icg should run");
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    assert!(stderr(&output).contains("ICG_PACK_DIR path does not exist"));
}

// --- check: the shadow warning is a --debug diagnostic ------------------

/// The check front-end names the selected pack source on stderr — but only
/// under `--debug`, where diagnostics belong.
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
    assert!(!err.contains("kubectl"));
    assert!(!err.contains("WARNING (pack source)"));
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

/// `icg status` states which pack source the operator commands selected, next
/// to the installed directory the hook reads.
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
    assert!(!text.contains("Pack source: packs (working directory)"));
    assert!(!text.contains("WARNING (pack source)"));
    assert!(!text.contains("✓ pack kubectl"));
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

/// An unparseable pack is a drift finding, not a usage fault and not a
/// silence: the report names the file with an `UNREADABLE` line and still
/// counts it, on whichever side it sits, while the readable packs around
/// it compare normally — a pack that fails to load must not masquerade as
/// missing from its own side.
#[test]
fn pack_drift_reports_an_unreadable_pack_as_drift_not_a_usage_fault() {
    let staged = stage(&["alpha"], &["alpha"]);
    // One syntactically broken file, one structurally wrong file: both are
    // "unreadable" — the second parses as JSON but not as a pack.
    let broken_installed = staged.installed.join("broken.json");
    fs::write(&broken_installed, "{ not json").expect("broken installed pack written");
    let broken_reference = staged.working_dir.join("packs").join("broken.json");
    fs::write(&broken_reference, "[]").expect("broken reference pack written");

    let output = staged.run(&["pack-drift"]);
    assert_eq!(
        output.status.code(),
        Some(1),
        "the check ran and found drift: {}",
        stderr(&output)
    );
    let text = stdout(&output);
    assert!(
        text.contains(&format!("UNREADABLE: {} (", broken_installed.display())),
        "the unreadable installed pack is named with its path: {text}"
    );
    assert!(
        text.contains("UNREADABLE: packs/broken.json ("),
        "the unreadable reference pack is named too, as resolved against \
         the default artifact: {text}"
    );
    assert!(text.contains("DRIFT: 2 difference(s)"), "{text}");
    assert!(
        !text.contains("MISSING FROM INSTALLED") && !text.contains("NOT IN REFERENCE"),
        "an unreadable pack is not misreported as set drift: {text}"
    );
}

/// A named reference artifact that does not exist cannot run the check
/// either: exit 2, the mirror of the nonexistent installed location —
/// "a named location is missing" faults whichever side is named.
#[test]
fn pack_drift_faults_when_the_named_reference_artifact_does_not_exist() {
    let staged = stage(&["alpha"], &[]);
    let missing = staged._dir.path().join("no-such-release");
    let output = staged.run(&[
        "pack-drift",
        "--installed",
        staged.installed.to_str().unwrap(),
        "--reference",
        missing.to_str().unwrap(),
    ]);
    assert_eq!(
        output.status.code(),
        Some(2),
        "a missing reference cannot run the check"
    );
    let err = stderr(&output);
    assert!(
        err.contains("reference pack location does not exist"),
        "the failure names the side and the path: {err}"
    );
    assert!(
        err.contains(missing.to_string_lossy().as_ref()),
        "the missing path is named: {err}"
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

/// A release archive is not itself a pack-drift reference. Passing an
/// archive (even one whose contents are malformed) instead of its extracted
/// root-level JSON directory is an unusable artifact, so the check cannot
/// run and must return the usage/error status rather than calling it drift.
#[test]
fn pack_drift_rejects_a_malformed_release_archive_as_unusable() {
    let staged = stage(&["alpha"], &[]);
    let archive = staged._dir.path().join("icg-packs.tar.gz");
    fs::write(&archive, b"not a gzip tar archive").expect("malformed archive written");
    let output = staged.run(&[
        "pack-drift",
        "--installed",
        staged.installed.to_str().unwrap(),
        "--reference",
        archive.to_str().unwrap(),
    ]);

    assert_eq!(
        output.status.code(),
        Some(2),
        "an archive is not a usable extracted reference: {}",
        stderr(&output)
    );
    assert!(
        stderr(&output).contains("no .json packs found in the reference location"),
        "the artifact shape is named: {}",
        stderr(&output)
    );
    assert!(
        stdout(&output).is_empty(),
        "no comparison report is emitted"
    );
}

/// The documented release layout has JSON pack manifests at the reference
/// root. A nested `packs/` directory is the malformed layout rejected by the
/// updater, and is likewise unusable as a pack-drift reference.
#[test]
fn pack_drift_rejects_a_nested_release_artifact_layout() {
    let staged = stage(&["alpha"], &[]);
    let nested = staged._dir.path().join("release");
    fs::create_dir(&nested).expect("release directory");
    fs::create_dir(nested.join("packs")).expect("nested packs directory");
    write_pack(&nested.join("packs"), "alpha");

    let output = staged.run(&[
        "pack-drift",
        "--installed",
        staged.installed.to_str().unwrap(),
        "--reference",
        nested.to_str().unwrap(),
    ]);

    assert_eq!(
        output.status.code(),
        Some(2),
        "a nested artifact layout cannot be compared: {}",
        stderr(&output)
    );
    assert!(
        stderr(&output).contains("no .json packs found in the reference location"),
        "the invalid layout is named: {}",
        stderr(&output)
    );
    assert!(
        stdout(&output).is_empty(),
        "no comparison report is emitted"
    );
}
