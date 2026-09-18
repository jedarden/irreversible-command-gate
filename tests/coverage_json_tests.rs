//! `icg coverage --format json` -- the machine-readable policy surface.
//!
//! Agents, bots and doc generators need to know what is enforced without
//! scraping `coverage --list`'s text. This pins the shape of that output and
//! keeps it in agreement with the packs on disk.

use serde_json::Value;
use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use tempfile::TempDir;

fn packs_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("packs")
}

fn icg(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_icg"))
        .args(args)
        .output()
        .expect("icg should run")
}

/// Run `coverage --list --format json` against explicit `--pack` paths.
fn coverage_json_against(packs: &[&Path]) -> Output {
    let mut args = vec!["coverage", "--list", "--format", "json"];
    for pack in packs {
        args.push("--pack");
        args.push(pack.to_str().expect("pack path is utf-8"));
    }
    icg(&args)
}

/// The truncated-pack fixture every "this pack does not load" test shares.
fn corrupt_pack_contents() -> String {
    fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/corrupt-rule-pack.json"),
    )
    .expect("corrupt fixture should be readable")
}

fn coverage_json() -> Value {
    let packs = packs_dir();
    let output = icg(&[
        "coverage",
        "--list",
        "--format",
        "json",
        "--pack",
        packs.to_str().unwrap(),
    ]);
    assert!(
        output.status.success(),
        "coverage --format json should succeed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).expect("coverage --format json should emit valid JSON")
}

#[test]
fn coverage_json_reports_every_shipped_pack_and_rule() {
    let report = coverage_json();
    assert_eq!(report["format"], "coverage/v1");

    let mut on_disk_packs = BTreeSet::new();
    let mut on_disk_rules = BTreeSet::new();
    for entry in fs::read_dir(packs_dir()).expect("packs/ readable") {
        let path = entry.expect("entry").path();
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        let pack: Value = serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        on_disk_packs.insert(pack["id"].as_str().unwrap().to_owned());
        for rule in pack["guarded_patterns"].as_array().unwrap() {
            on_disk_rules.insert(rule["id"].as_str().unwrap().to_owned());
        }
    }

    let reported_packs: BTreeSet<String> = report["packs"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| p["id"].as_str().unwrap().to_owned())
        .collect();
    let reported_rules: BTreeSet<String> = report["packs"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|p| p["guarded_patterns"].as_array().unwrap())
        .map(|r| r["id"].as_str().unwrap().to_owned())
        .collect();

    assert_eq!(reported_packs, on_disk_packs);
    assert_eq!(reported_rules, on_disk_rules);
    assert_eq!(
        report["pack_count"].as_u64().unwrap() as usize,
        on_disk_packs.len()
    );
    assert_eq!(
        report["guarded_pattern_count"].as_u64().unwrap() as usize,
        on_disk_rules.len()
    );
    assert!(report["unreadable"].as_array().unwrap().is_empty());
}

#[test]
fn coverage_json_carries_the_fields_an_integrator_needs() {
    let report = coverage_json();
    let git = report["packs"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["id"] == "git")
        .expect("the git pack should be reported");

    assert!(git["tool_keywords"]
        .as_array()
        .unwrap()
        .contains(&Value::from("git")));
    assert!(!git["safe_patterns"].as_array().unwrap().is_empty());

    let force_push = git["guarded_patterns"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["id"] == "git-force-push")
        .expect("git-force-push should be reported");

    // A rewrite rule must be distinguishable from a deny rule without
    // running a command through the engine.
    assert_eq!(force_push["channel"], "UpdatedInput");
    assert_eq!(force_push["check"], "command_regex");
    assert_eq!(force_push["enabled"], true);
    for field in ["tier", "severity", "explanation", "redirect"] {
        assert!(
            force_push[field].as_str().is_some_and(|s| !s.is_empty()),
            "git-force-push should report a non-empty {field}"
        );
    }
}

#[test]
fn coverage_rejects_an_unknown_format() {
    let output = icg(&["coverage", "--format", "yaml"]);
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("unsupported --format"));
}

// --- the coverage/v1 contract ------------------------------------------
//
// docs/notes/coverage-json-api.md specifies this API. The tests below pin
// it: a shape change must fail here until the format version moves with it.

/// The exact top-level key set. Adding a field for a new feature without
/// bumping the format version would silently pass every lenient consumer
/// and break the strict ones; this makes it a build failure instead.
#[test]
fn coverage_json_pins_its_top_level_key_set() {
    let report = coverage_json();
    let keys: BTreeSet<&str> = report
        .as_object()
        .expect("coverage/v1 should be an object")
        .keys()
        .map(String::as_str)
        .collect();
    let expected: BTreeSet<&str> = [
        "format",
        "packs",
        "unreadable",
        "pack_count",
        "guarded_pattern_count",
    ]
    .into_iter()
    .collect();
    assert_eq!(keys, expected, "coverage/v1 top-level keys moved");
}

/// Every rule, in every shipped pack, must be fully described: the
/// integrator reading coverage/v1 gets the same metadata `icg explain`
/// prints, and every guarded rule still owes its caller an alternative
/// (repo rule 2).
#[test]
fn every_reported_rule_carries_complete_metadata() {
    let report = coverage_json();
    let channels = ["Deny", "UpdatedInput", "AdditionalContext"];
    let checks = ["command_regex", "content_regex", "predicate"];

    let packs = report["packs"].as_array().unwrap();
    assert!(!packs.is_empty(), "the shipped packs should all report");
    for pack in packs {
        let pack_id = pack["id"].as_str().unwrap();
        assert!(!pack_id.is_empty());
        assert!(!pack["path"].as_str().unwrap().is_empty());
        for field in [
            "tool_keywords",
            "applies_to",
            "safe_patterns",
            "guarded_patterns",
        ] {
            assert!(
                pack[field].is_array(),
                "pack {pack_id} should report {field} as an array"
            );
        }
        for rule in pack["guarded_patterns"].as_array().unwrap() {
            let rule_id = rule["id"].as_str().unwrap();
            assert!(!rule_id.is_empty(), "rule ids are never blank");
            assert!(rule["enabled"].is_boolean(), "{rule_id}: enabled");
            assert!(rule["destructive"].is_boolean(), "{rule_id}: destructive");
            for field in ["tier", "severity", "explanation", "redirect"] {
                assert!(
                    rule[field]
                        .as_str()
                        .is_some_and(|value| !value.trim().is_empty()),
                    "{rule_id} should report a non-empty {field}"
                );
            }
            assert!(
                checks.contains(&rule["check"].as_str().unwrap()),
                "{rule_id}: unknown check kind {:?}",
                rule["check"]
            );
            assert!(
                channels.contains(&rule["channel"].as_str().unwrap()),
                "{rule_id}: unknown channel {:?}",
                rule["channel"]
            );
        }
    }
}

/// `--list` is the documented spelling but is optional under
/// `--format json`; both spellings are part of the contract.
#[test]
fn coverage_json_works_without_the_optional_list_flag() {
    let output = icg(&[
        "coverage",
        "--format",
        "json",
        "--pack",
        packs_dir().to_str().unwrap(),
    ]);
    assert!(output.status.success());
    let report: Value =
        serde_json::from_slice(&output.stdout).expect("json without --list is still coverage/v1");
    assert_eq!(report["format"], "coverage/v1");
}

/// `ICG_PACK_DIR` replaces the default pack search path when no `--pack`
/// is given — part of the documented resolution order. A single-pack
/// directory must yield exactly one pack: if the variable were ignored or
/// merely additive, the repository's own `packs/` (found from the working
/// directory) would report too.
#[test]
fn icg_pack_dir_replaces_the_default_search_path() {
    let dir = TempDir::new().expect("tempdir");
    fs::copy(packs_dir().join("tmux.json"), dir.path().join("only.json"))
        .expect("shipped tmux pack should copy");
    let output = Command::new(env!("CARGO_BIN_EXE_icg"))
        .args(["coverage", "--format", "json"])
        .env("ICG_PACK_DIR", dir.path())
        .output()
        .expect("icg should run");
    assert!(
        output.status.success(),
        "ICG_PACK_DIR at a readable pack should load: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let report: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["format"], "coverage/v1");
    assert_eq!(report["pack_count"].as_u64(), Some(1));
    assert_eq!(report["packs"][0]["id"], "tmux");

    let empty = TempDir::new().expect("tempdir");
    let output = Command::new(env!("CARGO_BIN_EXE_icg"))
        .args(["coverage", "--format", "json"])
        .env("ICG_PACK_DIR", empty.path())
        .output()
        .expect("icg should run");
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("no rule packs found"),
        "an empty ICG_PACK_DIR is refused like an empty --pack dir: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

/// An unreadable pack is reported in `unreadable` — path plus reason —
/// while every readable pack in the same directory still reports, the
/// command exits 0, and the counts describe only what loaded.
#[test]
fn an_unreadable_pack_is_reported_alongside_the_readable_ones() {
    let dir = TempDir::new().expect("tempdir");
    fs::copy(
        packs_dir().join("tmux.json"),
        dir.path().join("readable.json"),
    )
    .expect("shipped tmux pack should copy");
    let corrupt_path = dir.path().join("corrupt.json");
    fs::write(&corrupt_path, corrupt_pack_contents()).expect("write corrupt pack");

    let output = coverage_json_against(&[dir.path()]);
    assert!(
        output.status.success(),
        "a partially readable directory still succeeds: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let report: Value =
        serde_json::from_slice(&output.stdout).expect("partial readability still emits a document");

    let packs = report["packs"].as_array().unwrap();
    assert_eq!(packs.len(), 1, "only the readable pack reports");
    assert_eq!(packs[0]["id"], "tmux");
    assert_eq!(report["pack_count"].as_u64(), Some(1));
    assert_eq!(
        report["guarded_pattern_count"].as_u64(),
        Some(packs[0]["guarded_patterns"].as_array().unwrap().len() as u64),
        "counts describe only what loaded"
    );

    let unreadable = report["unreadable"].as_array().unwrap();
    assert_eq!(unreadable.len(), 1, "the corrupt pack is named");
    assert!(
        unreadable[0]["path"]
            .as_str()
            .unwrap()
            .ends_with("corrupt.json"),
        "unreadable entries carry the path: {:?}",
        unreadable[0]
    );
    assert!(
        !unreadable[0]["error"].as_str().unwrap().trim().is_empty(),
        "unreadable entries carry the reason"
    );
}

/// A directory where nothing loads must not produce an empty coverage/v1
/// document — an empty report reads as "nothing is enforced".
#[test]
fn a_directory_where_every_pack_is_unreadable_fails_without_a_document() {
    let dir = TempDir::new().expect("tempdir");
    fs::write(dir.path().join("corrupt.json"), corrupt_pack_contents())
        .expect("write corrupt pack");

    let output = coverage_json_against(&[dir.path()]);
    assert!(!output.status.success());
    assert!(
        output.stdout.is_empty(),
        "no document is emitted when nothing loaded: {}",
        String::from_utf8_lossy(&output.stdout)
    );
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("no readable rule packs"),
        "stderr names the failure: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

/// Consumers cache and byte-diff this document; two runs over an unchanged
/// pack tree must agree byte for byte, and packs must be path-ordered.
#[test]
fn coverage_json_is_byte_stable_across_runs() {
    let packs = packs_dir();
    let first = coverage_json_against(&[&packs]);
    let second = coverage_json_against(&[&packs]);
    assert!(first.status.success() && second.status.success());
    assert_eq!(
        first.stdout, second.stdout,
        "identical invocations must produce identical bytes"
    );

    let report: Value = serde_json::from_slice(&first.stdout).unwrap();
    let paths: Vec<PathBuf> = report["packs"]
        .as_array()
        .unwrap()
        .iter()
        .map(|pack| PathBuf::from(pack["path"].as_str().unwrap()))
        .collect();
    let mut sorted = paths.clone();
    sorted.sort();
    assert_eq!(paths, sorted, "packs are ordered by resolved path");
}

/// An empty directory is refused, not reported as zero coverage.
#[test]
fn an_empty_pack_directory_is_an_error_not_an_empty_report() {
    let dir = TempDir::new().expect("tempdir");
    let output = coverage_json_against(&[dir.path()]);
    assert!(!output.status.success());
    assert!(
        output.stdout.is_empty(),
        "no document for an empty directory"
    );
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("no rule packs found"),
        "stderr names the failure: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

/// A typo'd --pack path must fail loudly rather than quietly reporting
/// only the packs that did resolve.
#[test]
fn a_missing_pack_path_is_an_error() {
    let dir = TempDir::new().expect("tempdir");
    let missing = dir.path().join("does-not-exist");
    let output = coverage_json_against(&[&missing]);
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("rule-pack path does not exist"),
        "stderr names the failure: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}
