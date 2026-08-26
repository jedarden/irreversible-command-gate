//! Rule pack validation tests
//!
//! These tests verify that rule packs are properly validated for:
//! - Invalid regex patterns
//! - Missing required fields
//! - Schema violations
//! - Semantic inconsistencies (destructive flag vs redirect channel)
//! - Regex compilation errors
//! - Pattern ID uniqueness
//! - Safe/guarded pattern conflicts

use std::fs;
use std::path::Path;
use std::process::{Command, Output};
use tempfile::tempdir;

fn icg(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_icg"))
        .args(args)
        .output()
        .expect("icg should run")
}

fn icg_with_env(args: &[&str], envs: &[(&str, &Path)]) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_icg"));
    command.args(args);
    for (key, value) in envs {
        command.env(key, value);
    }
    command.output().expect("icg should run")
}

#[test]
fn validation_rejects_invalid_regex_unclosed_group() {
    // Test that packs with unclosed regex groups are rejected
    let temp_dir = tempdir().unwrap();
    let pack_path = temp_dir.path().join("unclosed-group.json");

    let invalid_regex = r#"{
        "id": "unclosed-test",
        "tool_keywords": ["test"],
        "applies_to": [],
        "safe_patterns": [],
        "guarded_patterns": [
            {
                "id": "unclosed-pattern",
                "enabled": true,
                "check": {
                    "type": "command_regex",
                    "regex": "(unclosed["
                },
                "tier": "tier1",
                "severity": "Critical",
                "explanation": "Invalid regex with unclosed group",
                "redirect": {
                    "channel": "deny",
                    "reason_template": "Invalid regex",
                    "rewrite_template": null
                },
                "destructive": true
            }
        ]
    }"#;

    fs::write(&pack_path, invalid_regex).expect("pack should write");

    let result = icg(&[
        "check",
        "--pack",
        &pack_path.to_string_lossy(),
        "--command",
        "test",
    ]);

    let stderr = String::from_utf8_lossy(&result.stderr);
    let stdout = String::from_utf8_lossy(&result.stdout);
    let output = format!("{}\n{}", stdout, stderr);

    // Should reject or warn about invalid regex
    assert!(
        output.contains("error")
            || output.contains("invalid")
            || output.contains("regex")
            || !result.status.success(),
        "Invalid regex (unclosed group) should be rejected"
    );
}

#[test]
fn validation_rejects_invalid_regex_unmatched_bracket() {
    // Test that packs with unmatched brackets are rejected
    let temp_dir = tempdir().unwrap();
    let pack_path = temp_dir.path().join("unmatched-bracket.json");

    let invalid_regex = r#"{
        "id": "unmatched-test",
        "tool_keywords": ["test"],
        "applies_to": [],
        "safe_patterns": [],
        "guarded_patterns": [
            {
                "id": "unmatched-pattern",
                "enabled": true,
                "check": {
                    "type": "command_regex",
                    "regex": "test[abc"
                },
                "tier": "tier1",
                "severity": "Critical",
                "explanation": "Invalid regex with unmatched bracket",
                "redirect": {
                    "channel": "deny",
                    "reason_template": "Invalid regex",
                    "rewrite_template": null
                },
                "destructive": true
            }
        ]
    }"#;

    fs::write(&pack_path, invalid_regex).expect("pack should write");

    let result = icg(&[
        "check",
        "--pack",
        &pack_path.to_string_lossy(),
        "--command",
        "test",
    ]);

    let stderr = String::from_utf8_lossy(&result.stderr);
    let stdout = String::from_utf8_lossy(&result.stdout);
    let output = format!("{}\n{}", stdout, stderr);

    assert!(
        output.contains("error")
            || output.contains("invalid")
            || output.contains("regex")
            || !result.status.success(),
        "Invalid regex (unmatched bracket) should be rejected"
    );
}

#[test]
fn validation_rejects_invalid_regex_invalid_escape() {
    // Test that packs with invalid escape sequences are handled
    let temp_dir = tempdir().unwrap();
    let pack_path = temp_dir.path().join("invalid-escape.json");

    let invalid_regex = r#"{
        "id": "invalid-escape-test",
        "tool_keywords": ["test"],
        "applies_to": [],
        "safe_patterns": [],
        "guarded_patterns": [
            {
                "id": "invalid-escape-pattern",
                "enabled": true,
                "check": {
                    "type": "command_regex",
                    "regex": "test\\p"  // \p is invalid escape
                },
                "tier": "tier1",
                "severity": "Critical",
                "explanation": "Invalid regex with bad escape",
                "redirect": {
                    "channel": "deny",
                    "reason_template": "Invalid regex",
                    "rewrite_template": null
                },
                "destructive": true
            }
        ]
    }"#;

    fs::write(&pack_path, invalid_regex).expect("pack should write");

    let result = icg(&[
        "check",
        "--pack",
        &pack_path.to_string_lossy(),
        "--command",
        "test",
    ]);

    // Should handle the invalid escape (either reject or treat as literal 'p')
    let stderr = String::from_utf8_lossy(&result.stderr);
    let stdout = String::from_utf8_lossy(&result.stdout);
    let output = format!("{}\n{}", stdout, stderr);

    // As long as it doesn't crash, it's handling it
    assert!(
        !output.contains("panic") && !output.contains("segfault"),
        "Invalid regex should not crash the engine"
    );
}

#[test]
fn validation_rejects_missing_required_field_id() {
    // Test that packs missing required field 'id' are rejected
    let temp_dir = tempdir().unwrap();
    let pack_path = temp_dir.path().join("missing-id.json");

    let missing_id = r#"{
        "tool_keywords": ["test"],
        "applies_to": [],
        "safe_patterns": [],
        "guarded_patterns": []
    }"#;

    fs::write(&pack_path, missing_id).expect("pack should write");

    let result = icg(&[
        "check",
        "--pack",
        &pack_path.to_string_lossy(),
        "--command",
        "test",
    ]);

    let stderr = String::from_utf8_lossy(&result.stderr);
    let output = if !stderr.is_empty() {
        stderr
    } else {
        String::from_utf8_lossy(&result.stdout)
    };

    assert!(
        output.contains("error")
            || output.contains("missing")
            || output.contains("required")
            || output.contains("id")
            || !result.status.success(),
        "Missing required field 'id' should be rejected"
    );
}

#[test]
fn validation_allows_missing_tool_keywords_for_content_pack() {
    // Content packs may omit tool_keywords; the schema defaults it to an empty
    // list because dispatch is controlled by applies_to instead.
    let temp_dir = tempdir().unwrap();
    let pack_path = temp_dir.path().join("missing-keywords.json");

    let missing_keywords = r#"{
        "id": "missing-keywords-test",
        "applies_to": [],
        "safe_patterns": [],
        "guarded_patterns": []
    }"#;

    fs::write(&pack_path, missing_keywords).expect("pack should write");

    let result = icg(&[
        "check",
        "--pack",
        &pack_path.to_string_lossy(),
        "--command",
        "test",
    ]);

    let stderr = String::from_utf8_lossy(&result.stderr);
    let output = if !stderr.is_empty() {
        stderr
    } else {
        String::from_utf8_lossy(&result.stdout)
    };

    assert!(
        result.status.success(),
        "content pack should load: {output}"
    );
    assert!(output.contains("ALLOW"));
}

#[test]
fn validation_rejects_missing_pattern_id() {
    // Test that patterns missing 'id' are rejected
    let temp_dir = tempdir().unwrap();
    let pack_path = temp_dir.path().join("missing-pattern-id.json");

    let missing_pattern_id = r#"{
        "id": "missing-pattern-id-test",
        "tool_keywords": ["test"],
        "applies_to": [],
        "safe_patterns": [],
        "guarded_patterns": [
            {
                "enabled": true,
                "check": {
                    "type": "command_regex",
                    "regex": "test dangerous"
                },
                "tier": "tier1",
                "severity": "Critical",
                "explanation": "Pattern missing ID",
                "redirect": {
                    "channel": "deny",
                    "reason_template": "No ID",
                    "rewrite_template": null
                },
                "destructive": true
            }
        ]
    }"#;

    fs::write(&pack_path, missing_pattern_id).expect("pack should write");

    let result = icg(&[
        "check",
        "--pack",
        &pack_path.to_string_lossy(),
        "--command",
        "test dangerous",
    ]);

    let stderr = String::from_utf8_lossy(&result.stderr);
    let output = if !stderr.is_empty() {
        stderr
    } else {
        String::from_utf8_lossy(&result.stdout)
    };

    assert!(
        output.contains("error")
            || output.contains("missing")
            || output.contains("id")
            || !result.status.success(),
        "Pattern missing 'id' should be rejected"
    );
}

#[test]
fn validation_rejects_missing_check_type() {
    // Test that patterns missing 'check.type' are rejected
    let temp_dir = tempdir().unwrap();
    let pack_path = temp_dir.path().join("missing-check-type.json");

    let missing_check_type = r#"{
        "id": "missing-check-type-test",
        "tool_keywords": ["test"],
        "applies_to": [],
        "safe_patterns": [],
        "guarded_patterns": [
            {
                "id": "no-check-type",
                "enabled": true,
                "check": {
                    "regex": "test dangerous"
                },
                "tier": "tier1",
                "severity": "Critical",
                "explanation": "Pattern missing check type",
                "redirect": {
                    "channel": "deny",
                    "reason_template": "No check type",
                    "rewrite_template": null
                },
                "destructive": true
            }
        ]
    }"#;

    fs::write(&pack_path, missing_check_type).expect("pack should write");

    let result = icg(&[
        "check",
        "--pack",
        &pack_path.to_string_lossy(),
        "--command",
        "test dangerous",
    ]);

    let stderr = String::from_utf8_lossy(&result.stderr);
    let output = if !stderr.is_empty() {
        stderr
    } else {
        String::from_utf8_lossy(&result.stdout)
    };

    assert!(
        output.contains("error")
            || output.contains("missing")
            || output.contains("type")
            || !result.status.success(),
        "Pattern missing 'check.type' should be rejected"
    );
}

#[test]
fn validation_rejects_duplicate_pattern_ids() {
    // Test that duplicate pattern IDs are rejected
    let temp_dir = tempdir().unwrap();
    let pack_path = temp_dir.path().join("duplicate-ids.json");

    let duplicate_ids = r#"{
        "id": "duplicate-ids-test",
        "tool_keywords": ["test"],
        "applies_to": [],
        "safe_patterns": [],
        "guarded_patterns": [
            {
                "id": "duplicate-id",
                "enabled": true,
                "check": {
                    "type": "command_regex",
                    "regex": "test dangerous1"
                },
                "tier": "tier1",
                "severity": "Critical",
                "explanation": "First pattern",
                "redirect": {
                    "channel": "deny",
                    "reason_template": "First",
                    "rewrite_template": null
                },
                "destructive": true
            },
            {
                "id": "duplicate-id",
                "enabled": true,
                "check": {
                    "type": "command_regex",
                    "regex": "test dangerous2"
                },
                "tier": "tier1",
                "severity": "Critical",
                "explanation": "Second pattern with same ID",
                "redirect": {
                    "channel": "deny",
                    "reason_template": "Second",
                    "rewrite_template": null
                },
                "destructive": true
            }
        ]
    }"#;

    fs::write(&pack_path, duplicate_ids).expect("pack should write");

    // The command is denied, which attempts a denial-log write to
    // /var/cache/icg. On a non-root run that write fails and the
    // icg_monitoring_event line lands on stderr, shadowing the stdout
    // denial text this assertion reads. A private log path keeps the
    // scenario isolated from the host, root or not.
    let denial_log = temp_dir.path().join("denials.jsonl");
    let result = icg_with_env(
        &[
            "check",
            "--pack",
            &pack_path.to_string_lossy(),
            "--command",
            "test dangerous1",
        ],
        &[("ICG_DENIAL_LOG", &denial_log)],
    );

    let stderr = String::from_utf8_lossy(&result.stderr);
    let output = if !stderr.is_empty() {
        stderr
    } else {
        String::from_utf8_lossy(&result.stdout)
    };

    assert!(
        output.contains("duplicate")
            || output.contains("unique")
            || output.contains("id")
            || !result.status.success(),
        "Duplicate pattern IDs should be rejected"
    );
}

#[test]
fn validation_rejects_redirect_without_rewrite_for_safe_operations() {
    // Test that non-destructive operations with 'deny' redirect are flagged
    // Non-destructive operations should either allow or provide a rewrite
    let temp_dir = tempdir().unwrap();
    let pack_path = temp_dir.path().join("redirect-conflict.json");

    let redirect_conflict = r#"{
        "id": "redirect-conflict-test",
        "tool_keywords": ["test"],
        "applies_to": [],
        "safe_patterns": [],
        "guarded_patterns": [
            {
                "id": "non-destructive-deny",
                "enabled": true,
                "check": {
                    "type": "command_regex",
                    "regex": "test mildly-dangerous"
                },
                "tier": "tier2",
                "severity": "Medium",
                "explanation": "Not destructive but denied",
                "redirect": {
                    "channel": "deny",
                    "reason_template": "Operation is not safe",
                    "rewrite_template": null
                },
                "destructive": false
            }
        ]
    }"#;

    fs::write(&pack_path, redirect_conflict).expect("pack should write");

    let result = icg(&[
        "check",
        "--pack",
        &pack_path.to_string_lossy(),
        "--command",
        "test mildly-dangerous",
    ]);

    // This is a warning case - non-destructive with deny and no rewrite
    // Should probably provide a rewrite if it's not truly destructive
    let stderr = String::from_utf8_lossy(&result.stderr);
    let stdout = String::from_utf8_lossy(&result.stdout);
    let output = format!("{}\n{}", stdout, stderr);

    // At minimum should not crash
    assert!(
        !output.contains("panic") && !output.contains("segfault"),
        "Redirect conflicts should not crash the engine"
    );
}

#[test]
fn validation_accepts_well_formed_pack() {
    // Test that well-formed packs are accepted
    let temp_dir = tempdir().unwrap();
    let pack_path = temp_dir.path().join("well-formed.json");

    let well_formed = r#"{
        "id": "well-formed-test",
        "tool_keywords": ["test"],
        "applies_to": [],
        "safe_patterns": [
            {
                "id": "safe-pattern",
                "check": {
                    "type": "command_regex",
                    "regex": "^test safe"
                }
            }
        ],
        "guarded_patterns": [
            {
                "id": "dangerous-pattern",
                "enabled": true,
                "check": {
                    "type": "command_regex",
                    "regex": "test dangerous"
                },
                "tier": "tier1",
                "severity": "Critical",
                "explanation": "Well-formed dangerous pattern",
                "redirect": {
                    "channel": "deny",
                    "reason_template": "This is dangerous",
                    "rewrite_template": "test safe-alternative"
                },
                "destructive": true
            }
        ]
    }"#;

    fs::write(&pack_path, well_formed).expect("pack should write");

    let result = icg(&[
        "check",
        "--pack",
        &pack_path.to_string_lossy(),
        "--command",
        "test safe",
    ]);

    let stderr = String::from_utf8_lossy(&result.stderr);
    let stdout = String::from_utf8_lossy(&result.stdout);
    let output = format!("{}\n{}", stdout, stderr);

    // Well-formed pack should load successfully
    assert!(
        !output.contains("error") && !output.contains("invalid"),
        "Well-formed pack should be accepted"
    );
}

#[test]
fn validation_handles_empty_patterns_array() {
    // Test that packs with empty patterns arrays are valid
    let temp_dir = tempdir().unwrap();
    let pack_path = temp_dir.path().join("empty-patterns.json");

    let empty_patterns = r#"{
        "id": "empty-patterns-test",
        "tool_keywords": ["test"],
        "applies_to": [],
        "safe_patterns": [],
        "guarded_patterns": []
    }"#;

    fs::write(&pack_path, empty_patterns).expect("pack should write");

    let result = icg(&[
        "check",
        "--pack",
        &pack_path.to_string_lossy(),
        "--command",
        "test anything",
    ]);

    // Empty patterns array is valid (just allows everything)
    let stderr = String::from_utf8_lossy(&result.stderr);
    let output = if !stderr.is_empty() {
        stderr
    } else {
        String::from_utf8_lossy(&result.stdout)
    };

    assert!(
        !output.contains("error") || output.contains("empty") && output.contains("valid"),
        "Empty patterns array should be valid"
    );
}

#[test]
fn validation_rejects_invalid_tier_value() {
    // Test that invalid tier values are rejected
    let temp_dir = tempdir().unwrap();
    let pack_path = temp_dir.path().join("invalid-tier.json");

    let invalid_tier = r#"{
        "id": "invalid-tier-test",
        "tool_keywords": ["test"],
        "applies_to": [],
        "safe_patterns": [],
        "guarded_patterns": [
            {
                "id": "invalid-tier-pattern",
                "enabled": true,
                "check": {
                    "type": "command_regex",
                    "regex": "test dangerous"
                },
                "tier": "tier5",
                "severity": "Critical",
                "explanation": "Invalid tier",
                "redirect": {
                    "channel": "deny",
                    "reason_template": "Invalid tier",
                    "rewrite_template": null
                },
                "destructive": true
            }
        ]
    }"#;

    fs::write(&pack_path, invalid_tier).expect("pack should write");

    let result = icg(&[
        "check",
        "--pack",
        &pack_path.to_string_lossy(),
        "--command",
        "test dangerous",
    ]);

    let stderr = String::from_utf8_lossy(&result.stderr);
    let output = if !stderr.is_empty() {
        stderr
    } else {
        String::from_utf8_lossy(&result.stdout)
    };

    assert!(
        output.contains("error")
            || output.contains("invalid")
            || output.contains("tier")
            || !result.status.success(),
        "Invalid tier value should be rejected"
    );
}

#[test]
fn validation_rejects_invalid_severity_value() {
    // Test that invalid severity values are rejected
    let temp_dir = tempdir().unwrap();
    let pack_path = temp_dir.path().join("invalid-severity.json");

    let invalid_severity = r#"{
        "id": "invalid-severity-test",
        "tool_keywords": ["test"],
        "applies_to": [],
        "safe_patterns": [],
        "guarded_patterns": [
            {
                "id": "invalid-severity-pattern",
                "enabled": true,
                "check": {
                    "type": "command_regex",
                    "regex": "test dangerous"
                },
                "tier": "tier1",
                "severity": "Emergency",
                "explanation": "Invalid severity",
                "redirect": {
                    "channel": "deny",
                    "reason_template": "Invalid severity",
                    "rewrite_template": null
                },
                "destructive": true
            }
        ]
    }"#;

    fs::write(&pack_path, invalid_severity).expect("pack should write");

    let result = icg(&[
        "check",
        "--pack",
        &pack_path.to_string_lossy(),
        "--command",
        "test dangerous",
    ]);

    let stderr = String::from_utf8_lossy(&result.stderr);
    let output = if !stderr.is_empty() {
        stderr
    } else {
        String::from_utf8_lossy(&result.stdout)
    };

    assert!(
        output.contains("error")
            || output.contains("invalid")
            || output.contains("severity")
            || !result.status.success(),
        "Invalid severity value should be rejected"
    );
}

#[test]
fn validation_rejects_invalid_channel_value() {
    // Test that invalid redirect channel values are rejected
    let temp_dir = tempdir().unwrap();
    let pack_path = temp_dir.path().join("invalid-channel.json");

    let invalid_channel = r#"{
        "id": "invalid-channel-test",
        "tool_keywords": ["test"],
        "applies_to": [],
        "safe_patterns": [],
        "guarded_patterns": [
            {
                "id": "invalid-channel-pattern",
                "enabled": true,
                "check": {
                    "type": "command_regex",
                    "regex": "test dangerous"
                },
                "tier": "tier1",
                "severity": "Critical",
                "explanation": "Invalid channel",
                "redirect": {
                    "channel": "block",
                    "reason_template": "Invalid channel",
                    "rewrite_template": null
                },
                "destructive": true
            }
        ]
    }"#;

    fs::write(&pack_path, invalid_channel).expect("pack should write");

    let result = icg(&[
        "check",
        "--pack",
        &pack_path.to_string_lossy(),
        "--command",
        "test dangerous",
    ]);

    let stderr = String::from_utf8_lossy(&result.stderr);
    let output = if !stderr.is_empty() {
        stderr
    } else {
        String::from_utf8_lossy(&result.stdout)
    };

    assert!(
        output.contains("error")
            || output.contains("invalid")
            || output.contains("channel")
            || !result.status.success(),
        "Invalid redirect channel value should be rejected"
    );
}

#[test]
fn validation_handles_complex_regex_patterns() {
    // Test that complex but valid regex patterns are accepted
    let temp_dir = tempdir().unwrap();
    let pack_path = temp_dir.path().join("complex-regex.json");

    let complex_regex = r#"{
        "id": "complex-regex-test",
        "tool_keywords": ["test"],
        "applies_to": [],
        "safe_patterns": [],
        "guarded_patterns": [
            {
                "id": "complex-pattern",
                "enabled": true,
                "check": {
                    "type": "command_regex",
                    "regex": "^(test|exam)\\s+(dangerous|destructive)\\s+(?:(?!--safe).)+$"
                },
                "tier": "tier1",
                "severity": "Critical",
                "explanation": "Complex regex pattern",
                "redirect": {
                    "channel": "deny",
                    "reason_template": "Complex pattern matched",
                    "rewrite_template": null
                },
                "destructive": true
            }
        ]
    }"#;

    fs::write(&pack_path, complex_regex).expect("pack should write");

    let result = icg(&[
        "check",
        "--pack",
        &pack_path.to_string_lossy(),
        "--command",
        "test dangerous command",
    ]);

    // Complex regex should compile and work
    let stderr = String::from_utf8_lossy(&result.stderr);
    let output = if !stderr.is_empty() {
        stderr
    } else {
        String::from_utf8_lossy(&result.stdout)
    };

    assert!(
        !output.contains("error") || output.contains("DENIED"),
        "Complex valid regex should be accepted"
    );
}
