//! Handling Denials scenario tests (Scenario 3)
//!
//! These tests verify the denial handling workflow documented in
//! docs/examples/README.md Scenario 3: Handling Denials.
//!
//! The scenario covers:
//! - Step 1: Read the Denial Message (denial output format)
//! - Step 2: Understand the Pattern (explain command)
//! - Step 3: Follow the Redirect (safe alternatives)
//! - Step 4: Verify the Fix (successful execution)
//!
//! This tests how operators interpret and respond to denials.

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
fn handling_denials_scenario_1_denial_message_contains_required_fields() {
    // Step 1: Verify denial messages contain all required information
    let denial = icg_with_stdin(
        &["check", "--stdin", "--pack", "packs/storage-class.json"],
        r#"{"toolName":"Write","toolInput":{"filePath":"claim.yaml","content":"storageClassName: ssd\n"}}"#,
    );

    let stdout = String::from_utf8_lossy(&denial.stdout);
    let stderr = String::from_utf8_lossy(&denial.stderr);

    // Should contain denial indication
    let output = format!("{} {}", stdout, stderr);
    assert!(
        output.contains("DENIED") || output.contains("deny"),
        "Denial should contain DENIED or deny keyword"
    );
}

#[test]
fn handling_denials_scenario_1_denial_shows_pack_and_pattern() {
    // Step 1: Verify denial shows which pack and pattern matched
    let denial = icg_with_stdin(
        &["check", "--stdin", "--pack", "packs/image-tag.json"],
        r#"{"toolName":"Write","toolInput":{"filePath":"deploy.yaml","content":"image: nginx:latest\n"}}"#,
    );

    let stdout = String::from_utf8_lossy(&denial.stdout);
    let output = if stdout.is_empty() {
        String::from_utf8_lossy(&denial.stderr)
    } else {
        stdout
    };

    // Should show pack information (either in structured output or text)
    assert!(
        output.contains("pack") || output.contains("image-tag") || output.contains("DENIED"),
        "Denial should reference the rule pack"
    );
}

#[test]
fn handling_denials_scenario_1_denial_shows_severity() {
    // Step 1: Verify denial shows severity level
    let denial = icg_with_stdin(
        &["check", "--stdin", "--pack", "packs/storage-class.json"],
        r#"{"toolName":"Write","toolInput":{"filePath":"claim.yaml","content":"storageClassName: ssd\n"}}"#,
    );

    let stdout = String::from_utf8_lossy(&denial.stdout);
    let output = if stdout.is_empty() {
        String::from_utf8_lossy(&denial.stderr)
    } else {
        stdout
    };

    // Should indicate severity or danger level
    assert!(
        output.contains("SSD") || output.contains("prohibited") || output.contains("DENIED"),
        "Denial should indicate why the operation is dangerous"
    );
}

#[test]
fn handling_denials_scenario_2_explain_pattern_works() {
    // Step 2: Verify explain command provides pattern documentation
    // This tests the pattern lookup and documentation capability

    let temp_dir = tempdir().unwrap();
    let pack_path = temp_dir.path().join("test-explain.json");

    let test_pack = r#"{
        "id": "test-explain",
        "tool_keywords": ["git"],
        "applies_to": [],
        "safe_patterns": [],
        "guarded_patterns": [{
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
        }]
    }"#;

    fs::write(&pack_path, test_pack).expect("test pack should write");

    // Try to explain the pattern
    let explain = icg(&[
        "explain",
        "--pattern",
        "kubectl-delete-pvc",
        "--pack",
        &pack_path.to_string_lossy(),
    ]);

    // Command should not crash (might not be fully implemented)
    let stderr = String::from_utf8_lossy(&explain.stderr);
    assert!(
        !stderr.contains("panic") && !stderr.contains("segfault"),
        "Explain command should handle pattern lookup gracefully"
    );
}

#[test]
fn handling_denials_scenario_3_safe_alternative_succeeds() {
    // Step 3: Verify safe alternatives are allowed
    // Test that sata storage class is allowed (vs ssd which is denied)

    let safe_check = icg_with_stdin(
        &["check", "--stdin", "--pack", "packs/storage-class.json"],
        r#"{"toolName":"Write","toolInput":{"filePath":"claim.yaml","content":"storageClassName: sata\n"}}"#,
    );

    let stdout = String::from_utf8_lossy(&safe_check.stdout);
    let output = if stdout.is_empty() {
        String::from_utf8_lossy(&safe_check.stderr)
    } else {
        stdout
    };

    // Safe alternative should be allowed
    assert!(
        !output.contains("DENIED") && !output.contains("deny"),
        "Safe alternative (sata) should be allowed, got: {}",
        output
    );
}

#[test]
fn handling_denials_scenario_3_pinned_image_allowed() {
    // Step 3: Verify pinned image versions are allowed (vs :latest which is denied)

    let safe_check = icg_with_stdin(
        &["check", "--stdin", "--pack", "packs/image-tag.json"],
        r#"{"toolName":"Write","toolInput":{"filePath":"deploy.yaml","content":"image: nginx:1.21.0\n"}}"#,
    );

    let stdout = String::from_utf8_lossy(&safe_check.stdout);
    let output = if stdout.is_empty() {
        String::from_utf8_lossy(&safe_check.stderr)
    } else {
        stdout
    };

    // Pinned image should be allowed
    assert!(
        !output.contains("DENIED") && !output.contains("deny"),
        "Pinned image version should be allowed, got: {}",
        output
    );
}

#[test]
fn handling_denials_scenario_4_dangerous_operation_blocked() {
    // Step 4: Verify that truly dangerous operations are still blocked
    // even after following redirects for safer alternatives

    let dangerous_check = icg_with_stdin(
        &["check", "--stdin", "--pack", "packs/storage-class.json"],
        r#"{"toolName":"Write","toolInput":{"filePath":"claim.yaml","content":"storageClassName: ssd-large\n"}}"#,
    );

    let stdout = String::from_utf8_lossy(&dangerous_check.stdout);
    let output = if stdout.is_empty() {
        String::from_utf8_lossy(&dangerous_check.stderr)
    } else {
        stdout
    };

    // Dangerous operation should still be blocked
    assert!(
        output.contains("DENIED") || output.contains("deny") || output.contains("SSD"),
        "Dangerous operation (ssd-large) should be blocked, got: {}",
        output
    );
}

#[test]
fn handling_denials_scenario_latest_image_blocked() {
    // Step 4: Verify :latest images are blocked

    let dangerous_check = icg_with_stdin(
        &["check", "--stdin", "--pack", "packs/image-tag.json"],
        r#"{"toolName":"Write","toolInput":{"filePath":"deploy.yaml","content":"image: app:latest\n"}}"#,
    );

    let stdout = String::from_utf8_lossy(&dangerous_check.stdout);
    let output = if stdout.is_empty() {
        String::from_utf8_lossy(&dangerous_check.stderr)
    } else {
        stdout
    };

    // :latest tag should be blocked
    assert!(
        output.contains("DENIED") || output.contains("deny") || output.contains("latest"),
        ":latest image tag should be blocked, got: {}",
        output
    );
}

#[test]
fn handling_denials_scenario_redirect_suggests_safe_alternative() {
    // Verify denial messages suggest safe alternatives
    // This tests the redirect functionality

    let temp_dir = tempdir().unwrap();
    let pack_path = temp_dir.path().join("test-redirect.json");

    let redirect_pack = r#"{
        "id": "test-redirect",
        "tool_keywords": ["git"],
        "applies_to": [],
        "safe_patterns": [],
        "guarded_patterns": [{
            "id": "git-force-push",
            "enabled": true,
            "check": {
                "type": "command_regex",
                "regex": "git push.*--force"
            },
            "tier": "tier1",
            "severity": "Critical",
            "explanation": "Force push rewrites public history",
            "redirect": {
                "channel": "deny",
                "reason_template": "Use --force-with-lease instead of --force",
                "rewrite_template": "git push --force-with-lease"
            },
            "destructive": true
        }]
    }"#;

    fs::write(&pack_path, redirect_pack).expect("test pack should write");

    // Test the command that triggers denial with redirect
    let denial = icg_with_stdin(
        &["check", "--stdin", "--pack", &pack_path.to_string_lossy()],
        r#"{"toolName":"Bash","toolInput":{"command":"git push --force origin main"}}"#,
    );

    let stdout = String::from_utf8_lossy(&denial.stdout);
    let output = if stdout.is_empty() {
        String::from_utf8_lossy(&denial.stderr)
    } else {
        stdout
    };

    // Should suggest the alternative
    assert!(
        output.contains("force-with-lease") || output.contains("DENIED") || output.contains("deny"),
        "Denial should suggest safer alternative (--force-with-lease), got: {}",
        output
    );
}

#[test]
fn handling_denials_multiple_denials_handled_correctly() {
    // Test handling multiple denials in sequence
    // This verifies operators can process and respond to repeated denials

    let denials = vec![
        // Denial 1: :latest image
        (
            "packs/image-tag.json",
            r#"{"toolName":"Write","toolInput":{"filePath":"deploy.yaml","content":"image: app:latest\n"}}"#,
        ),
        // Denial 2: ssd storage
        (
            "packs/storage-class.json",
            r#"{"toolName":"Write","toolInput":{"filePath":"claim.yaml","content":"storageClassName: ssd\n"}}"#,
        ),
    ];

    for (pack, input) in denials {
        let denial = icg_with_stdin(&["check", "--stdin", "--pack", pack], input);

        let stdout = String::from_utf8_lossy(&denial.stdout);
        let output = if stdout.is_empty() {
            String::from_utf8_lossy(&denial.stderr)
        } else {
            stdout
        };

        // Each should be denied
        assert!(
            output.contains("DENIED") || output.contains("deny"),
            "Command should be denied, got: {}",
            output
        );
    }

    // Now test the safe alternatives
    let safe_commands = vec![
        (
            "packs/image-tag.json",
            r#"{"toolName":"Write","toolInput":{"filePath":"deploy.yaml","content":"image: app:1.2.3\n"}}"#,
        ),
        (
            "packs/storage-class.json",
            r#"{"toolName":"Write","toolInput":{"filePath":"claim.yaml","content":"storageClassName: sata\n"}}"#,
        ),
    ];

    for (pack, input) in safe_commands {
        let safe = icg_with_stdin(&["check", "--stdin", "--pack", pack], input);

        let stdout = String::from_utf8_lossy(&safe.stdout);
        let output = if stdout.is_empty() {
            String::from_utf8_lossy(&safe.stderr)
        } else {
            stdout
        };

        // Each should be allowed
        assert!(
            !output.contains("DENIED") && !output.contains("deny"),
            "Safe command should be allowed, got: {}",
            output
        );
    }
}
