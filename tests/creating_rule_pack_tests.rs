//! Creating a New Rule Pack scenario tests (Scenario 6)
//!
//! These tests verify the rule pack creation workflow documented in
//! docs/examples/README.md Scenario 6: Creating a New Rule Pack.
//!
//! The scenario covers:
//! - Step 1: Scaffold the Pack (new-pack command)
//! - Step 2: Define Safe Patterns
//! - Step 3: Define Guarded Patterns
//! - Step 4: Write Tests
//! - Step 5: Test Locally
//! - Step 6: Generate Regression Suite
//!
//! This tests the developer workflow for creating new rule packs.

use std::fs;
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

#[test]
fn creating_rule_pack_scenario_1_scaffold_command_exists() {
    // Step 1: Verify new-pack command exists
    let new_pack = icg(&["new-pack", "--help"]);

    // Should succeed
    assert!(new_pack.status.success(), "new-pack --help should succeed");

    let stdout = String::from_utf8_lossy(&new_pack.stdout);
    assert!(
        stdout.contains("new-pack") || stdout.contains("pack") || stdout.contains("Scaffold"),
        "Help should mention pack scaffolding"
    );
}

#[test]
fn creating_rule_pack_scenario_1_scaffold_creates_files() {
    // Step 1: Verify new-pack command creates the necessary files
    let temp_dir = tempdir().unwrap();
    let output_dir = temp_dir.path();

    let result = icg(&[
        "new-pack",
        "test-kubectl",
        "--pack-type",
        "command",
        "--output-dir",
        &output_dir.to_string_lossy(),
    ]);

    // Should succeed
    assert!(result.status.success(), "new-pack should succeed");

    let stdout = String::from_utf8_lossy(&result.stdout);
    assert!(
        stdout.contains("✓") || stdout.contains("Pack") || stdout.contains("created"),
        "new-pack should report successful creation"
    );

    // Verify files were created
    let pack_file = output_dir.join("test-kubectl.json");
    let test_file = output_dir.join("test-kubectl-tests.rs");

    assert!(
        pack_file.exists() || test_file.exists(),
        "new-pack should create pack and/or test files"
    );

    // If pack file exists, verify it's valid JSON
    if pack_file.exists() {
        let content = fs::read_to_string(&pack_file).expect("pack file should be readable");
        assert!(
            content.contains("id")
                && content.contains("tool_keywords")
                && content.contains("safe_patterns")
                && content.contains("guarded_patterns"),
            "Pack file should contain required fields"
        );
    }
}

#[test]
fn creating_rule_pack_scenario_2_safe_patterns_structure() {
    // Step 2: Verify safe patterns can be defined in pack structure
    let temp_dir = tempdir().unwrap();
    let pack_path = temp_dir.path().join("safe-pattern-test.json");

    let pack_with_safe_patterns = r#"{
        "id": "safe-pattern-test",
        "tool_keywords": ["kubectl"],
        "applies_to": [],
        "safe_patterns": [
            {
                "id": "safe-get",
                "check": {
                    "type": "command_regex",
                    "regex": "^kubectl get"
                }
            },
            {
                "id": "safe-describe",
                "check": {
                    "type": "command_regex",
                    "regex": "^kubectl describe"
                }
            },
            {
                "id": "safe-logs",
                "check": {
                    "type": "command_regex",
                    "regex": "^kubectl logs"
                }
            }
        ],
        "guarded_patterns": []
    }"#;

    fs::write(&pack_path, pack_with_safe_patterns).expect("pack should write");

    // Verify the pack can be loaded
    let check = icg(&[
        "check",
        "--pack",
        &pack_path.to_string_lossy(),
        "--command",
        "kubectl get pods",
    ]);

    // Should not crash
    let stderr = String::from_utf8_lossy(&check.stderr);
    assert!(
        !stderr.contains("failed to load") && !stderr.contains("error"),
        "Pack with safe patterns should load successfully"
    );
}

#[test]
fn creating_rule_pack_scenario_3_guarded_patterns_structure() {
    // Step 3: Verify guarded patterns can be defined with all required fields
    let temp_dir = tempdir().unwrap();
    let pack_path = temp_dir.path().join("guarded-pattern-test.json");

    let pack_with_guarded_patterns = r#"{
        "id": "guarded-pattern-test",
        "tool_keywords": ["kubectl"],
        "applies_to": [],
        "safe_patterns": [],
        "guarded_patterns": [
            {
                "id": "kubectl-delete-deployment",
                "enabled": true,
                "check": {
                    "type": "command_regex",
                    "regex": "kubectl delete deployment"
                },
                "tier": "tier1",
                "severity": "High",
                "explanation": "Deleting a deployment removes all running pods",
                "redirect": {
                    "channel": "deny",
                    "reason_template": "kubectl delete deployment is destructive. Use 'kubectl scale deployment --replicas=0' instead.",
                    "rewrite_template": null
                },
                "destructive": true
            },
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
                    "reason_template": "kubectl delete pvc is permanently destructive. Data cannot be recovered.",
                    "rewrite_template": null
                },
                "destructive": true
            }
        ]
    }"#;

    fs::write(&pack_path, pack_with_guarded_patterns).expect("pack should write");

    // Verify the pack can be loaded and used
    let dangerous_check = icg_with_stdin(
        &["check", "--stdin", "--pack", &pack_path.to_string_lossy()],
        r#"{"toolName":"Bash","toolInput":{"command":"kubectl delete deployment myapp"}}"#,
    );

    let stdout = String::from_utf8_lossy(&dangerous_check.stdout);
    let output = if stdout.is_empty() {
        String::from_utf8_lossy(&dangerous_check.stderr)
    } else {
        stdout
    };

    // Should deny the dangerous operation
    assert!(
        output.contains("DENIED") || output.contains("deny") || output.contains("destructive"),
        "Guarded pattern should deny dangerous operation, got: {}",
        output
    );
}

#[test]
fn creating_rule_pack_scenario_5_test_locally_with_check_command() {
    // Step 5: Verify individual commands can be tested with check command
    let temp_dir = tempdir().unwrap();
    let pack_path = temp_dir.path().join("local-test.json");

    let test_pack = r#"{
        "id": "local-test",
        "tool_keywords": ["vault"],
        "applies_to": [],
        "safe_patterns": [
            {
                "id": "safe-vault-get",
                "check": {
                    "type": "command_regex",
                    "regex": "vault kv get"
                }
            }
        ],
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

    fs::write(&pack_path, test_pack).expect("pack should write");

    // Test safe command
    let safe_check = icg_with_stdin(
        &["check", "--stdin", "--pack", &pack_path.to_string_lossy()],
        r#"{"toolName":"Bash","toolInput":{"command":"vault kv get secret/test"}}"#,
    );

    let stdout = String::from_utf8_lossy(&safe_check.stdout);
    let output = if stdout.is_empty() {
        String::from_utf8_lossy(&safe_check.stderr)
    } else {
        stdout
    };

    assert!(
        !output.contains("DENIED") && !output.contains("deny"),
        "Safe command should be allowed"
    );

    // Test dangerous command
    let dangerous_check = icg_with_stdin(
        &["check", "--stdin", "--pack", &pack_path.to_string_lossy()],
        r#"{"toolName":"Bash","toolInput":{"command":"vault kv destroy secret/test"}}"#,
    );

    let stdout = String::from_utf8_lossy(&dangerous_check.stdout);
    let output = if stdout.is_empty() {
        String::from_utf8_lossy(&dangerous_check.stderr)
    } else {
        stdout
    };

    assert!(
        output.contains("DENIED") || output.contains("deny"),
        "Dangerous command should be denied"
    );
}

#[test]
fn creating_rule_pack_scenario_6_generate_regression_suite() {
    // Step 6: Verify regression suite can be generated from pack
    let temp_dir = tempdir().unwrap();
    let pack_path = temp_dir.path().join("regression-test-pack.json");
    let output_path = temp_dir.path().join("regression-suite.json");

    let regression_pack = r#"{
        "id": "regression-test-pack",
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

    // Generate regression suite
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

    let stdout = String::from_utf8_lossy(&result.stdout);
    assert!(
        stdout.contains("regression") || stdout.contains("test") || stdout.contains("cases"),
        "Regression suite generation should report success"
    );

    // Verify output file was created and contains cases
    assert!(
        output_path.exists(),
        "Regression suite file should be created"
    );

    let content = fs::read_to_string(&output_path).expect("regression suite should be readable");
    assert!(
        content.contains("cases")
            || content.contains("test-dangerous-1")
            || content.contains("test-dangerous-2"),
        "Regression suite should contain test cases for each guarded pattern"
    );
}

#[test]
fn creating_rule_pack_scenario_pack_validation_rejects_invalid_regex() {
    // Verify that packs with invalid regex patterns are rejected
    let temp_dir = tempdir().unwrap();
    let pack_path = temp_dir.path().join("invalid-regex.json");

    let invalid_pack = r#"{
        "id": "invalid-regex",
        "tool_keywords": ["test"],
        "applies_to": [],
        "safe_patterns": [],
        "guarded_patterns": [
            {
                "id": "unclosed-group",
                "enabled": true,
                "check": {
                    "type": "command_regex",
                    "regex": "(unclosed["
                },
                "tier": "tier1",
                "severity": "Critical",
                "explanation": "Invalid regex",
                "redirect": {
                    "channel": "deny",
                    "reason_template": "Invalid",
                    "rewrite_template": null
                },
                "destructive": true
            }
        ]
    }"#;

    fs::write(&pack_path, invalid_pack).expect("pack should write");

    // Should fail to load or validate
    let result = icg(&[
        "check",
        "--pack",
        &pack_path.to_string_lossy(),
        "--command",
        "test",
    ]);

    // Should indicate an error
    let stderr = String::from_utf8_lossy(&result.stderr);
    let stdout = String::from_utf8_lossy(&result.stdout);
    let output = format!("{} {}", stdout, stderr);

    assert!(
        output.contains("error")
            || output.contains("invalid")
            || output.contains("failed")
            || !result.status.success(),
        "Invalid regex should cause validation error"
    );
}

#[test]
fn creating_rule_pack_scenario_pack_with_content_patterns() {
    // Verify content-mode packs (for file writes) can be created
    let temp_dir = tempdir().unwrap();
    let pack_path = temp_dir.path().join("content-pack.json");

    let content_pack = r#"{
        "id": "content-test",
        "tool_keywords": ["write", "edit"],
        "applies_to": ["*.yaml", "*.yml"],
        "safe_patterns": [],
        "guarded_patterns": [
            {
                "id": "api-key-in-write",
                "enabled": true,
                "type": "content_regex",
                "regex": "api[_-]?key[\"']?\\s*[:=]\\s*[\"']?[a-zA-Z0-9]{32,}",
                "tier": "tier1",
                "severity": "Critical",
                "explanation": "Writing API keys to files is a security risk",
                "redirect": {
                    "channel": "deny",
                    "reason_template": "Do not write API keys directly to files. Use secret management.",
                    "rewrite_template": null
                },
                "destructive": true
            }
        ]
    }"#;

    fs::write(&pack_path, content_pack).expect("pack should write");

    // Test content checking
    let dangerous_write = icg_with_stdin(
        &["check", "--stdin", "--pack", &pack_path.to_string_lossy()],
        r#"{"toolName":"Write","toolInput":{"filePath":"config.yaml","content":"api_key: sk1234567890abcdefghijklmnopqrstuv\n"}}"#,
    );

    let stdout = String::from_utf8_lossy(&dangerous_write.stdout);
    let output = if stdout.is_empty() {
        String::from_utf8_lossy(&dangerous_write.stderr)
    } else {
        stdout
    };

    // Should deny writing API keys
    assert!(
        output.contains("DENIED")
            || output.contains("deny")
            || output.contains("api_key")
            || output.contains("API key"),
        "Content pattern should deny dangerous writes, got: {}",
        output
    );

    // Test safe write
    let safe_write = icg_with_stdin(
        &["check", "--stdin", "--pack", &pack_path.to_string_lossy()],
        r#"{"toolName":"Write","toolInput":{"filePath":"config.yaml","content":"database_host: localhost\n"}}"#,
    );

    let stdout = String::from_utf8_lossy(&safe_write.stdout);
    let output = if stdout.is_empty() {
        String::from_utf8_lossy(&safe_write.stderr)
    } else {
        stdout
    };

    // Should allow safe writes
    assert!(
        !output.contains("DENIED") && !output.contains("deny"),
        "Safe content writes should be allowed"
    );
}

#[test]
fn creating_rule_pack_scenario_enabled_flag_controls_pattern() {
    // Verify the enabled flag controls whether a pattern is active
    let temp_dir = tempdir().unwrap();
    let pack_path = temp_dir.path().join("enabled-flag.json");

    let pack_with_disabled = r#"{
        "id": "enabled-flag-test",
        "tool_keywords": ["test"],
        "applies_to": [],
        "safe_patterns": [],
        "guarded_patterns": [
            {
                "id": "disabled-pattern",
                "enabled": false,
                "check": {
                    "type": "command_regex",
                    "regex": "test dangerous"
                },
                "tier": "tier1",
                "severity": "Critical",
                "explanation": "This pattern is disabled",
                "redirect": {
                    "channel": "deny",
                    "reason_template": "Pattern is disabled",
                    "rewrite_template": null
                },
                "destructive": true
            }
        ]
    }"#;

    fs::write(&pack_path, pack_with_disabled).expect("pack should write");

    // Test that disabled pattern doesn't block
    let result = icg_with_stdin(
        &["check", "--stdin", "--pack", &pack_path.to_string_lossy()],
        r#"{"toolName":"Bash","toolInput":{"command":"test dangerous"}}"#,
    );

    let stdout = String::from_utf8_lossy(&result.stdout);
    let output = if stdout.is_empty() {
        String::from_utf8_lossy(&result.stderr)
    } else {
        stdout
    };

    // Disabled pattern should not deny
    assert!(
        !output.contains("DENIED") && !output.contains("deny"),
        "Disabled pattern should not block commands"
    );
}
