use icg::regression::{
    prune_recorded_cases, record_denial_as_test, ExpectedVerdict, RecordOutcome, RegressionTestCase,
};
use serde_json::{json, Value};
use std::io::Write;
use std::process::{Command, Stdio};
use tempfile::tempdir;

fn test_pack() -> Value {
    json!({
        "id": "recorded-pack",
        "tool_keywords": ["git"],
        "applies_to": [],
        "safe_patterns": [],
        "guarded_patterns": [{
            "id": "deny-reset",
            "type": "command_regex",
            "regex": "git reset --hard",
            "tier": "tier1",
            "severity": "Critical",
            "explanation": "Reset discards work",
            "redirect": {
                "channel": "deny",
                "reason_template": "Do not discard work",
                "rewrite_template": null
            },
            "destructive": true
        }]
    })
}

fn run_hook(pack: &std::path::Path, corpus: &std::path::Path, command: &str) -> Value {
    let mut child = Command::new(env!("CARGO_BIN_EXE_icg"))
        .args([
            "hook",
            "--rule-pack",
            pack.to_str().expect("pack path should be UTF-8"),
            "--record-as-test",
            corpus.to_str().expect("corpus path should be UTF-8"),
        ])
        .env("ICG_TELEMETRY_PATH", corpus.join("telemetry.json"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("hook should start");
    child
        .stdin
        .take()
        .expect("hook stdin should be available")
        .write_all(
            json!({
                "tool_name": "Bash",
                "tool_input": {"command": command}
            })
            .to_string()
            .as_bytes(),
        )
        .expect("hook input should be written");
    let output = child.wait_with_output().expect("hook should finish");
    assert!(
        output.status.success(),
        "hook failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).expect("hook response should be JSON")
}

#[test]
fn opt_in_hook_recording_appends_and_deduplicates_a_denial() {
    let temp = tempdir().expect("temporary directory should exist");
    let pack_path = temp.path().join("pack.json");
    let corpus = temp.path().join("regression");
    std::fs::write(
        &pack_path,
        serde_json::to_vec_pretty(&test_pack()).expect("pack should serialize"),
    )
    .expect("pack should be written");

    for _ in 0..2 {
        let response = run_hook(&pack_path, &corpus, "git reset --hard HEAD");
        assert_eq!(response["hookSpecificOutput"]["permissionDecision"], "deny");
    }

    let cases_path = corpus.join("recorded-pack.cases");
    let cases = std::fs::read_to_string(&cases_path).expect("denial should create the pack corpus");
    let lines = cases.lines().collect::<Vec<_>>();
    assert_eq!(lines.len(), 1, "repeated denial should be deduplicated");
    let case: RegressionTestCase = serde_json::from_str(lines[0]).expect("case should be JSON");
    assert_eq!(case.pack_id, "recorded-pack");
    assert_eq!(case.pattern_id, "deny-reset");
    assert_eq!(case.command, "git reset --hard HEAD");
    assert_eq!(case.expected, ExpectedVerdict::Deny);
}

#[test]
fn recorder_is_bounded_and_explicit_pruning_removes_duplicates() {
    let temp = tempdir().expect("temporary directory should exist");
    let corpus = temp.path().join("regression");
    let first = RegressionTestCase {
        pack_id: "pack".to_string(),
        pattern_id: "rule".to_string(),
        command: "git reset --hard HEAD".to_string(),
        file_path: None,
        content: None,
        expected: ExpectedVerdict::Deny,
    };
    let second = RegressionTestCase {
        command: "git reset --hard other".to_string(),
        ..first.clone()
    };

    assert_eq!(
        record_denial_as_test(&corpus, first.clone()).expect("first case should record"),
        RecordOutcome::Added
    );
    assert_eq!(
        icg::regression::record_denial_as_test_with_limit(&corpus, second.clone(), 1)
            .expect("full corpus should be reported"),
        RecordOutcome::CapacityReached
    );

    let path = corpus.join("pack.cases");
    let line = serde_json::to_string(&first).expect("case should serialize");
    std::fs::write(&path, format!("{line}\n{line}\n")).expect("duplicate corpus should be written");
    let report = prune_recorded_cases(&corpus, 1).expect("pruning should succeed");
    assert_eq!(report.files_rewritten, 1);
    assert_eq!(report.cases_removed, 1);
    assert_eq!(std::fs::read_to_string(path).unwrap().lines().count(), 1);
}
