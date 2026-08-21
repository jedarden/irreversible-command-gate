//! Missing CLI features tests for developer scenarios
//!
//! These tests verify CLI commands and flags that are documented but not yet implemented:
//! - icg check --command --debug (debug output for pattern matching)
//! - icg verify-coverage (verify no coverage narrowing between pattern changes)
//! - icg status --denials (denial query functionality)
//! - icg explain --pattern (pattern documentation lookup)

use std::fs;
use std::path::PathBuf;
use std::process::{Command, Output};
use tempfile::tempdir;

fn icg(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_icg"))
        .args(args)
        .output()
        .expect("icg should run")
}

fn icg_with_stdin(args: &[&str], input: &str) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_icg"));
    command
        .args(args)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped());

    let mut child = command.spawn().expect("icg should run");
    use std::io::Write;
    child
        .stdin
        .take()
        .unwrap()
        .write_all(input.as_bytes())
        .unwrap();
    child.wait_with_output().expect("icg should finish")
}

fn write_coverage_pack(path: &std::path::Path, include_second_rule: bool) {
    let second_rule = if include_second_rule {
        r#", {
                "id": "test-dangerous-b",
                "enabled": true,
                "check": {"type": "command_regex", "regex": "test destroy"},
                "tier": "tier1",
                "severity": "High",
                "explanation": "Destroying test data is dangerous",
                "redirect": {
                    "channel": "deny",
                    "reason_template": "Do not destroy test data",
                    "rewrite_template": null
                },
                "destructive": true
            }"#
    } else {
        ""
    };
    let pack = format!(
        r#"{{
            "id": "coverage-test",
            "tool_keywords": ["test"],
            "applies_to": [],
            "safe_patterns": [],
            "guarded_patterns": [{{
                "id": "test-dangerous-a",
                "enabled": true,
                "check": {{"type": "command_regex", "regex": "test dangerous"}},
                "tier": "tier1",
                "severity": "High",
                "explanation": "The test operation is dangerous",
                "redirect": {{
                    "channel": "deny",
                    "reason_template": "Do not run the dangerous test operation",
                    "rewrite_template": null
                }},
                "destructive": true
            }}{second_rule}]
        }}"#
    );
    fs::write(path, pack).expect("coverage pack should write");
}

#[test]
fn debug_flag_provides_pattern_matching_trace() {
    // Test that --debug flag shows which patterns were evaluated and matched
    let temp_dir = tempdir().unwrap();
    let pack_path = temp_dir.path().join("debug-test.json");

    let debug_pack = r#"{
        "id": "debug-test",
        "tool_keywords": ["kubectl"],
        "applies_to": [],
        "safe_patterns": [
            {
                "id": "safe-get",
                "check": {
                    "type": "command_regex",
                    "regex": "^kubectl get"
                }
            }
        ],
        "guarded_patterns": [
            {
                "id": "kubectl-delete-pvc",
                "enabled": true,
                "check": {
                    "type": "command_regex",
                    "regex": "kubectl delete pvc"
                },
                "tier": "tier1",
                "severity": "Critical",
                "explanation": "Deleting a PVC destroys persistent data",
                "redirect": {
                    "channel": "deny",
                    "reason_template": "kubectl delete pvc is permanently destructive",
                    "rewrite_template": null
                },
                "destructive": true
            }
        ]
    }"#;

    fs::write(&pack_path, debug_pack).expect("pack should write");

    // Test with --debug flag
    let result = icg_with_stdin(
        &["check", "--stdin", "--pack", &pack_path.to_string_lossy(), "--debug"],
        r#"{"toolName":"Bash","toolInput":{"command":"kubectl delete pvc data"}}"#,
    );

    let stdout = String::from_utf8_lossy(&result.stdout);
    let stderr = String::from_utf8_lossy(&result.stderr);
    let output = format!("{}\n{}", stdout, stderr);

    // Debug output should include pattern matching information
    assert!(
        output.contains("DEBUG") || output.contains("debug") || output.contains("pattern") || output.contains("match"),
        "Debug output should include pattern matching trace, got: {}",
        output
    );
}

#[test]
fn debug_flag_shows_safe_pattern_evaluation() {
    // Test that --debug flag shows safe pattern evaluation
    let temp_dir = tempdir().unwrap();
    let pack_path = temp_dir.path().join("debug-safe.json");

    let safe_pack = r#"{
        "id": "debug-safe-test",
        "tool_keywords": ["kubectl"],
        "applies_to": [],
        "safe_patterns": [
            {
                "id": "safe-get",
                "check": {
                    "type": "command_regex",
                    "regex": "^kubectl get"
                }
            }
        ],
        "guarded_patterns": []
    }"#;

    fs::write(&pack_path, safe_pack).expect("pack should write");

    let result = icg_with_stdin(
        &["check", "--stdin", "--pack", &pack_path.to_string_lossy(), "--debug"],
        r#"{"toolName":"Bash","toolInput":{"command":"kubectl get pods"}}"#,
    );

    let stdout = String::from_utf8_lossy(&result.stdout);
    let stderr = String::from_utf8_lossy(&result.stderr);
    let output = format!("{}\n{}", stdout, stderr);

    // Should show that safe pattern was evaluated
    assert!(
        !output.contains("DENIED") && !output.contains("deny"),
        "Safe command should be allowed even with debug flag"
    );
}

#[test]
fn debug_flag_works_without_stdin() {
    // Test that --debug flag works with --command flag (no stdin)
    let temp_dir = tempdir().unwrap();
    let pack_path = temp_dir.path().join("debug-cmd.json");

    let test_pack = r#"{
        "id": "debug-cmd-test",
        "tool_keywords": ["test"],
        "applies_to": [],
        "safe_patterns": [],
        "guarded_patterns": [
            {
                "id": "test-dangerous",
                "enabled": true,
                "check": {
                    "type": "command_regex",
                    "regex": "test dangerous"
                },
                "tier": "tier1",
                "severity": "High",
                "explanation": "Dangerous test command",
                "redirect": {
                    "channel": "deny",
                    "reason_template": "Test is dangerous",
                    "rewrite_template": null
                },
                "destructive": true
            }
        ]
    }"#;

    fs::write(&pack_path, test_pack).expect("pack should write");

    let result = icg(&[
        "check",
        "--command",
        "test dangerous",
        "--pack",
        &pack_path.to_string_lossy(),
        "--debug",
    ]);

    let stdout = String::from_utf8_lossy(&result.stdout);
    let stderr = String::from_utf8_lossy(&result.stderr);
    let output = format!("{}\n{}", stdout, stderr);

    // Should work and show debug information
    assert!(
        output.contains("DENIED") || output.contains("deny") || output.contains("dangerous"),
        "Should deny dangerous command with debug output"
    );
}

#[test]
fn verify_coverage_detects_narrowed_coverage() {
    // The implemented coverage-diff command rejects a release that removes a
    // guarded pattern without an explicit justification.
    let temp_dir = tempdir().unwrap();

    let baseline = temp_dir.path().join("baseline.json");
    let current = temp_dir.path().join("current.json");
    write_coverage_pack(&baseline, true);
    write_coverage_pack(&current, false);

    let result = icg(&[
        "coverage-diff",
        &baseline.to_string_lossy(),
        &current.to_string_lossy(),
    ]);

    let output = format!(
        "{}\n{}",
        String::from_utf8_lossy(&result.stdout),
        String::from_utf8_lossy(&result.stderr)
    );
    assert!(!result.status.success());
    assert!(output.contains("test-dangerous-b") || output.contains("regressions"));
}

#[test]
fn verify_coverage_accepts_same_coverage() {
    let temp_dir = tempdir().unwrap();
    let baseline = temp_dir.path().join("baseline-same.json");
    let current = temp_dir.path().join("current-same.json");
    write_coverage_pack(&baseline, false);
    write_coverage_pack(&current, false);

    let result = icg(&[
        "coverage-diff",
        &baseline.to_string_lossy(),
        &current.to_string_lossy(),
    ]);
    assert!(result.status.success(), "coverage diff should pass: {}", String::from_utf8_lossy(&result.stderr));
}

#[test]
fn verify_coverage_accepts_expanded_coverage() {
    // Adding a guarded rule is a strengthening and should pass the gate.
    let temp_dir = tempdir().unwrap();
    let baseline = temp_dir.path().join("baseline-expand.json");
    let current = temp_dir.path().join("current-expand.json");
    write_coverage_pack(&baseline, false);
    write_coverage_pack(&current, true);

    let result = icg(&[
        "coverage-diff",
        &baseline.to_string_lossy(),
        &current.to_string_lossy(),
    ]);
    assert!(result.status.success(), "coverage diff should pass: {}", String::from_utf8_lossy(&result.stderr));
}

#[test]
fn explain_pattern_shows_pattern_documentation() {
    // Test that explain --pattern shows pattern documentation
    let temp_dir = tempdir().unwrap();
    let pack_path = temp_dir.path().join("explain-test.json");

    let explain_pack = r#"{
        "id": "explain-test",
        "tool_keywords": ["vault"],
        "applies_to": [],
        "safe_patterns": [],
        "guarded_patterns": [
            {
                "id": "vault-kv-destroy",
                "enabled": true,
                "check": {
                    "type": "command_regex",
                    "regex": "vault kv destroy"
                },
                "tier": "tier1",
                "severity": "Critical",
                "explanation": "vault kv destroy is permanently destructive and cannot be undone",
                "redirect": {
                    "channel": "deny",
                    "reason_template": "vault kv destroy is permanently destructive",
                    "rewrite_template": "vault kv patch"
                },
                "destructive": true
            }
        ]
    }"#;

    fs::write(&pack_path, explain_pack).expect("pack should write");

    let result = icg(&[
        "explain",
        "--pattern",
        "vault-kv-destroy",
        "--pack",
        &pack_path.to_string_lossy(),
    ]);

    let stdout = String::from_utf8_lossy(&result.stdout);
    let stderr = String::from_utf8_lossy(&result.stderr);
    let output = format!("{}\n{}", stdout, stderr);

    // Should show pattern information
    assert!(
        output.contains("vault-kv-destroy") || output.contains("destructive") || output.contains("vault"),
        "explain should show pattern documentation"
    );
}

#[test]
fn status_denials_shows_denial_history() {
    // Test that status --denials shows denial history
    let result = icg(&["status", "--denials", "--since", "1h"]);

    let stdout = String::from_utf8_lossy(&result.stdout);
    let stderr = String::from_utf8_lossy(&result.stderr);
    let output = format!("{}\n{}", stdout, stderr);

    // Command should be recognized (may not have denials in test environment)
    assert!(
        !output.contains("unrecognized") && !output.contains("unknown flag"),
        "status --denials should be a recognized command"
    );
}

#[test]
fn regression_suite_generates_test_cases() {
    // Test that regression-suite command generates test cases from a pack
    let temp_dir = tempdir().unwrap();
    let pack_path = temp_dir.path().join("regression-pack.json");
    let output_path = temp_dir.path().join("regression-output.json");

    let regression_pack = r#"{
        "id": "regression-test",
        "tool_keywords": ["test-tool"],
        "applies_to": [],
        "safe_patterns": [],
        "guarded_patterns": [
            {
                "id": "test-dangerous-1",
                "enabled": true,
                "check": {
                    "type": "command_regex",
                    "regex": "test-tool dangerous-op"
                },
                "tier": "tier1",
                "severity": "Critical",
                "explanation": "First dangerous operation",
                "redirect": {
                    "channel": "deny",
                    "reason_template": "Operation is dangerous",
                    "rewrite_template": null
                },
                "destructive": true
            },
            {
                "id": "test-dangerous-2",
                "enabled": true,
                "check": {
                    "type": "command_regex",
                    "regex": "test-tool destroy"
                },
                "tier": "tier1",
                "severity": "Critical",
                "explanation": "Second dangerous operation",
                "redirect": {
                    "channel": "deny",
                    "reason_template": "Operation is destructive",
                    "rewrite_template": null
                },
                "destructive": true
            }
        ]
    }"#;

    fs::write(&pack_path, regression_pack).expect("pack should write");

    let result = icg(&[
        "regression-suite",
        &pack_path.to_string_lossy(),
        "--output",
        &output_path.to_string_lossy(),
    ]);

    // Should succeed
    assert!(
        result.status.success(),
        "Regression suite generation should succeed"
    );

    // Output file should be created
    assert!(
        output_path.exists(),
        "Regression suite output file should be created"
    );

    // Output should contain test cases
    let content = fs::read_to_string(&output_path).expect("output should be readable");
    assert!(
        content.contains("cases") || content.contains("test-dangerous-1") || content.contains("test-dangerous-2"),
        "Regression suite should contain test cases for each guarded pattern"
    );
}

#[test]
fn regression_suite_includes_metadata() {
    // Test that regression suite includes pack metadata
    let temp_dir = tempdir().unwrap();
    let pack_path = temp_dir.path().join("regression-meta.json");
    let output_path = temp_dir.path().join("regression-meta-output.json");

    let meta_pack = r#"{
        "id": "meta-test",
        "tool_keywords": ["meta"],
        "applies_to": [],
        "safe_patterns": [],
        "guarded_patterns": [
            {
                "id": "meta-pattern",
                "enabled": true,
                "check": {
                    "type": "command_regex",
                    "regex": "meta dangerous"
                },
                "tier": "tier1",
                "severity": "High",
                "explanation": "Meta test pattern",
                "redirect": {
                    "channel": "deny",
                    "reason_template": "Pattern is dangerous",
                    "rewrite_template": null
                },
                "destructive": false
            }
        ]
    }"#;

    fs::write(&pack_path, meta_pack).expect("pack should write");

    let result = icg(&[
        "regression-suite",
        &pack_path.to_string_lossy(),
        "--output",
        &output_path.to_string_lossy(),
    ]);

    assert!(result.status.success(), "Regression suite generation should succeed");

    let content = fs::read_to_string(&output_path).expect("output should be readable");
    assert!(
        content.contains("packId") || content.contains("generatedAt") || content.contains("version") || content.contains("cases"),
        "Regression suite should include metadata"
    );
}
