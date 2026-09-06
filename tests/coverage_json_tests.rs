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

fn packs_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("packs")
}

fn icg(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_icg"))
        .args(args)
        .output()
        .expect("icg should run")
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
