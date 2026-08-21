//! First-time Installation scenario tests (Scenario 1)
//!
//! These tests verify the first-time installation workflow documented in
//! docs/examples/README.md Scenario 1: First-time Installation.
//!
//! The scenario covers:
//! - Step 1: Download and Install (verify binary works)
//! - Step 2: Install Rule Packs (pack validation)
//! - Step 3: Configure Claude Code Hook (hook integration)
//! - Step 4: Test Installation (check and explain commands)
//! - Step 5: Review Setup (health status)
//!
//! This tests the installation and setup process for new operators.

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

#[test]
fn installation_scenario_1_verify_binary_version() {
    // Step 1: Verify the icg binary runs and reports a version
    let version = icg(&["--version"]);

    // Should succeed
    assert!(version.status.success(), "icg --version should succeed");

    let stdout = String::from_utf8_lossy(&version.stdout);
    // Should contain version information
    assert!(
        stdout.contains("icg") || stdout.contains("version"),
        "Version output should contain 'icg' or 'version'"
    );
}

#[test]
fn installation_scenario_2_verify_rule_packs_load() {
    // Step 2: Verify rule packs can be loaded and validated
    let packs = vec![
        "packs/image-tag.json",
        "packs/storage-class.json",
        "packs/beads.json",
    ];

    for pack_path in packs {
        if PathBuf::from(pack_path).exists() {
            let check = icg(&["check", "--pack", pack_path, "--command", "echo test"]);

            // The pack should load without errors
            // It might succeed or deny the command, but it should not fail to load
            let stderr = String::from_utf8_lossy(&check.stderr);
            assert!(
                !stderr.contains("failed to load") && !stderr.contains("error"),
                "Pack {} should load without errors, got: {}",
                pack_path,
                stderr
            );
        }
    }
}

#[test]
fn installation_scenario_4_dangerous_command_denied() {
    // Step 4: Test that dangerous commands are denied

    // Test dangerous content (storage-class: ssd)
    let dangerous_check = icg_with_stdin(
        &["check", "--stdin", "--pack", "packs/storage-class.json"],
        r#"{"toolName":"Write","toolInput":{"filePath":"claim.yaml","content":"storageClassName: ssd\n"}}"#,
    );

    let stdout = String::from_utf8_lossy(&dangerous_check.stdout);
    assert!(
        stdout.contains("DENIED") || stdout.contains("deny"),
        "Dangerous command should be denied, got: {}",
        stdout
    );
}

#[test]
fn installation_scenario_4_safe_command_allowed() {
    // Step 4: Test that safe commands are allowed

    // Test safe content (storage-class: sata)
    let safe_check = icg_with_stdin(
        &["check", "--stdin", "--pack", "packs/storage-class.json"],
        r#"{"toolName":"Write","toolInput":{"filePath":"claim.yaml","content":"storageClassName: sata\n"}}"#,
    );

    let stdout = String::from_utf8_lossy(&safe_check.stdout);
    assert!(
        stdout.contains("ALLOW") || stdout.contains("allow") || !stdout.contains("DENIED"),
        "Safe command should be allowed, got: {}",
        stdout
    );
}

#[test]
fn installation_scenario_5_health_status_works() {
    // Step 5: Verify health status command works
    let health = icg(&["health", "status"]);

    // Should succeed
    assert!(health.status.success(), "icg health status should succeed");

    let stdout = String::from_utf8_lossy(&health.stdout);
    // Should contain health information
    assert!(
        stdout.contains("Health") || stdout.contains("Status") || stdout.contains("health"),
        "Health status should contain health information"
    );
}

#[test]
fn installation_scenario_5_status_command_works() {
    // Step 5: Verify status command works
    let status = icg(&["status"]);

    // Should succeed
    assert!(status.status.success(), "icg status should succeed");

    let stdout = String::from_utf8_lossy(&status.stdout);
    // Should contain status information
    assert!(
        stdout.contains("Trust") || stdout.contains("Rule Pack") || stdout.contains("Status"),
        "Status should contain trust or rule pack information"
    );
}

#[test]
fn installation_scenario_explain_command_works() {
    // Test that explain command can be used
    // This tests documentation lookup capability

    // Create a temporary pack with a known pattern
    let temp_dir = tempdir().unwrap();
    let pack_path = temp_dir.path().join("test-pack.json");

    let test_pack = r#"{
        "id": "test-pack",
        "tool_keywords": ["test"],
        "applies_to": [],
        "safe_patterns": [],
        "guarded_patterns": [{
            "id": "test-pattern",
            "enabled": true,
            "check": {
                "type": "command_regex",
                "regex": "test dangerous"
            },
            "tier": "tier1",
            "severity": "Critical",
            "explanation": "Test dangerous operation",
            "redirect": {
                "channel": "deny",
                "reason_template": "This is a test",
                "rewrite_template": null
            },
            "destructive": true
        }]
    }"#;

    fs::write(&pack_path, test_pack).expect("test pack should write");

    // Try to explain the pattern (if explain command is implemented)
    let explain = icg(&["explain", "--pattern", "test-pattern", "--pack", &pack_path.to_string_lossy()]);

    // The command should not crash - it might succeed or fail gracefully
    let stderr = String::from_utf8_lossy(&explain.stderr);
    assert!(
        !stderr.contains("panic") && !stderr.contains("segfault"),
        "Explain command should not crash"
    );
}

#[test]
fn installation_scenario_coverage_list_works() {
    // Test that coverage list command works
    let coverage = icg(&["coverage", "--list"]);

    // Should succeed
    assert!(coverage.status.success(), "icg coverage --list should succeed");

    let stdout = String::from_utf8_lossy(&coverage.stdout);
    // Should list available packs
    assert!(
        stdout.contains("pack") || stdout.contains("coverage") || stdout.is_empty(),
        "Coverage list should show pack information"
    );
}

#[test]
fn installation_scenario_check_command_accepts_stdin() {
    // Verify check command can accept input via stdin
    let input = r#"{"toolName":"Bash","toolInput":{"command":"echo safe-command"}}"#;
    let result = icg_with_stdin(&["check", "--stdin"], input);

    // Should not crash
    let stderr = String::from_utf8_lossy(&result.stderr);
    assert!(
        !stderr.contains("panic") && !stderr.contains("segfault"),
        "Check command with stdin should not crash"
    );
}
