use std::fs;
use std::process::Command;

use icg::rule_pack::{load_pack, Check, Severity, Tier};
use tempfile::tempdir;

#[test]
fn new_pack_cli_generates_pack_and_regression_stub() {
    let directory = tempdir().unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_icg"))
        .args([
            "new-pack",
            "demo-tool",
            "--pack-type",
            "command",
            "--output-dir",
            &directory.path().to_string_lossy(),
        ])
        .output()
        .expect("icg should run");

    assert!(output.status.success(), "new-pack failed: {output:?}");

    let pack_path = directory.path().join("demo-tool.json");
    let test_path = directory.path().join("demo-tool_pack_tests.rs");
    let pack = load_pack(&pack_path).expect("generated pack should be valid");
    assert_eq!(pack.id, "demo-tool");
    assert_eq!(pack.tool_keywords, vec!["demo-tool"]);
    assert_eq!(pack.guarded_patterns.len(), 1);
    assert_eq!(pack.guarded_patterns[0].tier, Tier::Tier1);
    assert_eq!(pack.guarded_patterns[0].severity, Severity::Critical);
    assert!(matches!(
        pack.guarded_patterns[0].check,
        Check::CommandRegex { .. }
    ));

    let test_stub = fs::read_to_string(test_path).expect("generated test stub should exist");
    assert!(test_stub.contains("guarded_pattern_detects_dangerous_operations"));
    assert!(test_stub.contains("demo-tool-dangerous-operation"));
}

#[test]
fn new_pack_cli_rejects_invalid_names() {
    let directory = tempdir().unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_icg"))
        .args([
            "new-pack",
            "not_a_pack",
            "--output-dir",
            &directory.path().to_string_lossy(),
        ])
        .output()
        .expect("icg should run");

    assert!(!output.status.success());
    assert!(directory.path().read_dir().unwrap().next().is_none());
}
