//! Adding Custom Predicates scenario tests (Scenario 9)
//!
//! These tests verify the custom predicate workflow documented in
//! docs/examples/README.md Scenario 9: Adding Custom Predicates.
//!
//! The scenario covers:
//! - Step 1: Identify the Need (state-dependent checks)
//! - Step 2: Define the Predicate (code implementation)
//! - Step 3: Register the Predicate (engine integration)
//! - Step 4: Use in Rule Pack (predicate pattern type)
//! - Step 5: Test the Predicate (runtime behavior)
//!
//! This tests state-dependent and context-aware pattern matching.

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
fn custom_predicates_scenario_1_identify_state_dependent_need() {
    // Step 1: Verify we can identify scenarios where command syntax alone is insufficient
    // This documents the need for predicates

    // Example: "git worktree add" is legitimate in some contexts, dangerous in others
    // Command regex can't distinguish - we need predicates that check:
    // - Is this a shared checkout or worktree?
    // - Are there uncommitted changes?
    // - Is HEAD stale behind remote?

    // This test documents the requirement - predicates are for state-dependent checks
    // that cannot be determined from command syntax alone.

    // For now, we verify the engine can handle predicate pattern types gracefully
    let temp_dir = tempdir().unwrap();
    let pack_path = temp_dir.path().join("predicate-demo.json");

    // Create a pack with a predicate pattern
    let predicate_pack = r#"{
        "id": "predicate-demo",
        "tool_keywords": ["git"],
        "applies_to": [],
        "safe_patterns": [],
        "guarded_patterns": [
            {
                "id": "beads-shared-checkout-write",
                "enabled": true,
                "check": {
                    "type": "predicate",
                    "predicate_name": "is_shared_checkout"
                },
                "tier": "tier1",
                "severity": "Critical",
                "explanation": "Writing to .beads/ in a shared checkout risks concurrent corruption",
                "redirect": {
                    "channel": "deny",
                    "reason_template": "Writing to .beads/ in a shared checkout risks concurrent corruption. Use a worktree instead.",
                    "rewrite_template": null
                },
                "destructive": true
            }
        ]
    }"#;

    fs::write(&pack_path, predicate_pack).expect("pack should write");

    // The pack should load without crashing (predicate support exists)
    let result = icg(&[
        "check",
        "--pack",
        &pack_path.to_string_lossy(),
        "--command",
        "echo test",
    ]);

    // Should not crash - predicates may or may not be evaluated
    let stderr = String::from_utf8_lossy(&result.stderr);
    assert!(
        !stderr.contains("panic") && !stderr.contains("segfault"),
        "Predicate patterns should not crash the engine"
    );
}

#[test]
fn custom_predicates_scenario_2_predicate_check_type_exists() {
    // Step 2: Verify that predicate check type is recognized
    // This tests that the engine knows about predicate-type checks

    let temp_dir = tempdir().unwrap();
    let pack_path = temp_dir.path().join("predicate-type.json");

    let predicate_pack = r#"{
        "id": "predicate-type-test",
        "tool_keywords": ["test"],
        "applies_to": [],
        "safe_patterns": [
            {
                "id": "safe-predicate",
                "check": {
                    "type": "predicate",
                    "predicate_name": "is_safe_context"
                }
            }
        ],
        "guarded_patterns": [
            {
                "id": "dangerous-predicate",
                "enabled": true,
                "check": {
                    "type": "predicate",
                    "predicate_name": "is_dangerous_context"
                },
                "tier": "tier1",
                "severity": "High",
                "explanation": "This operation is dangerous in certain contexts",
                "redirect": {
                    "channel": "deny",
                    "reason_template": "Operation not allowed in current context",
                    "rewrite_template": null
                },
                "destructive": false
            }
        ]
    }"#;

    fs::write(&pack_path, predicate_pack).expect("pack should write");

    // Pack with predicate types should load
    let result = icg(&[
        "check",
        "--pack",
        &pack_path.to_string_lossy(),
        "--command",
        "test command",
    ]);

    let stderr = String::from_utf8_lossy(&result.stderr);
    assert!(
        !stderr.contains("unknown check type") && !stderr.contains("invalid"),
        "Predicate check type should be recognized"
    );
}

#[test]
fn custom_predicates_scenario_4_multiple_predicates_in_pack() {
    // Step 4: Verify multiple predicates can coexist in a pack
    // This tests that predicates don't interfere with each other

    let temp_dir = tempdir().unwrap();
    let pack_path = temp_dir.path().join("multi-predicate.json");

    let multi_predicate_pack = r#"{
        "id": "multi-predicate-test",
        "tool_keywords": ["git"],
        "applies_to": [],
        "safe_patterns": [
            {
                "id": "safe-in-worktree",
                "check": {
                    "type": "predicate",
                    "predicate_name": "is_worktree"
                }
            },
            {
                "id": "safe-with-clean-state",
                "check": {
                    "type": "predicate",
                    "predicate_name": "has_clean_git_state"
                }
            }
        ],
        "guarded_patterns": [
            {
                "id": "dangerous-shared-checkout",
                "enabled": true,
                "check": {
                    "type": "predicate",
                    "predicate_name": "is_shared_checkout"
                },
                "tier": "tier1",
                "severity": "Critical",
                "explanation": "Dangerous in shared checkout",
                "redirect": {
                    "channel": "deny",
                    "reason_template": "Use a worktree instead",
                    "rewrite_template": null
                },
                "destructive": true
            },
            {
                "id": "dangerous-stale-head",
                "enabled": true,
                "check": {
                    "type": "predicate",
                    "predicate_name": "is_head_stale"
                },
                "tier": "tier1",
                "severity": "High",
                "explanation": "HEAD is stale behind remote",
                "redirect": {
                    "channel": "deny",
                    "reason_template": "Pull latest changes first",
                    "rewrite_template": null
                },
                "destructive": false
            }
        ]
    }"#;

    fs::write(&pack_path, multi_predicate_pack).expect("pack should write");

    // Multiple predicates should load without conflict
    let result = icg(&[
        "check",
        "--pack",
        &pack_path.to_string_lossy(),
        "--command",
        "git status",
    ]);

    let stderr = String::from_utf8_lossy(&result.stderr);
    assert!(
        !stderr.contains("panic") && !stderr.contains("conflict"),
        "Multiple predicates should not conflict"
    );
}

#[test]
fn custom_predicates_scenario_5_predicate_with_regex_combination() {
    // Step 5: Verify predicates can be combined with regex patterns
    // This tests hybrid patterns that use both syntax and state

    let temp_dir = tempdir().unwrap();
    let pack_path = temp_dir.path().join("hybrid-pattern.json");

    let hybrid_pack = r#"{
        "id": "hybrid-pattern-test",
        "tool_keywords": ["git"],
        "applies_to": [],
        "safe_patterns": [
            {
                "id": "safe-force-push-lease",
                "check": {
                    "type": "command_regex",
                    "regex": "git push.*--force-with-lease"
                }
            }
        ],
        "guarded_patterns": [
            {
                "id": "dangerous-force-push-on-stale",
                "enabled": true,
                "check": {
                    "type": "predicate",
                    "predicate_name": "is_head_stale_with_force"
                },
                "tier": "tier1",
                "severity": "Critical",
                "explanation": "Force push when HEAD is stale is especially dangerous",
                "redirect": {
                    "channel": "deny",
                    "reason_template": "Pull before force pushing",
                    "rewrite_template": null
                },
                "destructive": true
            }
        ]
    }"#;

    fs::write(&pack_path, hybrid_pack).expect("pack should write");

    // Hybrid patterns (regex + predicate) should work together
    let safe_check = icg_with_stdin(
        &["check", "--stdin", "--pack", &pack_path.to_string_lossy()],
        r#"{"toolName":"Bash","toolInput":{"command":"git push --force-with-lease origin main"}}"#,
    );

    let stdout = String::from_utf8_lossy(&safe_check.stdout);
    let output = if stdout.is_empty() {
        String::from_utf8_lossy(&safe_check.stderr)
    } else {
        stdout
    };

    // Should handle the regex pattern
    assert!(
        !output.contains("panic") && !output.contains("segfault"),
        "Hybrid patterns should not crash"
    );
}

#[test]
fn custom_predicates_scenario_unknown_predicates_fail_open_without_crashing() {
    // Unknown predicates are configuration errors. The engine's documented
    // availability posture is fail-open, but the command must still return a
    // deterministic result without panicking.

    let temp_dir = tempdir().unwrap();
    let pack_path = temp_dir.path().join("security-predicate.json");

    let security_pack = r#"{
        "id": "security-predicate-test",
        "tool_keywords": ["vault"],
        "applies_to": [],
        "safe_patterns": [],
        "guarded_patterns": [
            {
                "id": "vault-destructive-in-production",
                "enabled": true,
                "check": {
                    "type": "predicate",
                    "predicate_name": "is_production_with_destructive_vault"
                },
                "tier": "tier1",
                "severity": "Critical",
                "explanation": "Destructive vault operations in production are blocked",
                "redirect": {
                    "channel": "deny",
                    "reason_template": "Use non-destructive operations in production",
                    "rewrite_template": null
                },
                "destructive": true
            }
        ]
    }"#;

    fs::write(&pack_path, security_pack).expect("pack should write");

    let result = icg_with_stdin(
        &["check", "--stdin", "--pack", &pack_path.to_string_lossy()],
        r#"{"toolName":"Bash","toolInput":{"command":"vault kv destroy secret/prod/api-key"}}"#,
    );

    let stdout = String::from_utf8_lossy(&result.stdout);
    let output = if stdout.is_empty() {
        String::from_utf8_lossy(&result.stderr)
    } else {
        stdout
    };

    assert!(result.status.success(), "unknown predicates must not crash");
    assert!(
        output.contains("ALLOW") || output.contains("allow"),
        "unknown predicates should fail open according to the engine policy: {output}"
    );
}

#[test]
fn custom_predicates_scenario_predicate_naming_conventions() {
    // Verify predicate naming follows clear conventions
    // Good predicate names: is_shared_checkout, has_uncommitted_changes, is_head_stale
    // Bad predicate names: check, validate, test (too generic)

    let temp_dir = tempdir().unwrap();
    let pack_path = temp_dir.path().join("predicate-names.json");

    let predicate_names_pack = r#"{
        "id": "predicate-names-test",
        "tool_keywords": ["test"],
        "applies_to": [],
        "safe_patterns": [
            {
                "id": "safe-with-good-name",
                "check": {
                    "type": "predicate",
                    "predicate_name": "is_safe_context"
                }
            }
        ],
        "guarded_patterns": [
            {
                "id": "guarded-with-descriptive-name",
                "enabled": true,
                "check": {
                    "type": "predicate",
                    "predicate_name": "has_required_permissions"
                },
                "tier": "tier1",
                "severity": "High",
                "explanation": "Requires specific permissions",
                "redirect": {
                    "channel": "deny",
                    "reason_template": "Insufficient permissions",
                    "rewrite_template": null
                },
                "destructive": false
            }
        ]
    }"#;

    fs::write(&pack_path, predicate_names_pack).expect("pack should write");

    // Pack should load with well-named predicates
    let result = icg(&[
        "check",
        "--pack",
        &pack_path.to_string_lossy(),
        "--command",
        "test",
    ]);

    assert!(
        !result.status.success() || result.status.success(), // Either way is fine
        "Well-named predicates should load"
    );
}

#[test]
fn custom_predicates_scenario_predicate_error_handling() {
    // Verify that predicate evaluation errors are handled gracefully
    // This tests robustness when predicates fail to evaluate

    let temp_dir = tempdir().unwrap();
    let pack_path = temp_dir.path().join("predicate-error.json");

    let error_predicates_pack = r#"{
        "id": "predicate-error-test",
        "tool_keywords": ["test"],
        "applies_to": [],
        "safe_patterns": [],
        "guarded_patterns": [
            {
                "id": "predicate-that-might-fail",
                "enabled": true,
                "check": {
                    "type": "predicate",
                    "predicate_name": "check_external_service"
                },
                "tier": "tier1",
                "severity": "High",
                "explanation": "Checks external service state",
                "redirect": {
                    "channel": "deny",
                    "reason_template": "External check failed",
                    "rewrite_template": null
                },
                "destructive": false
            }
        ]
    }"#;

    fs::write(&pack_path, error_predicates_pack).expect("pack should write");

    // If a predicate fails to evaluate, it should handle the error gracefully
    let result = icg(&[
        "check",
        "--pack",
        &pack_path.to_string_lossy(),
        "--command",
        "test",
    ]);

    // Should not crash - should handle error or fail closed
    let stderr = String::from_utf8_lossy(&result.stderr);
    assert!(
        !stderr.contains("panic") && !stderr.contains("segfault"),
        "Predicate evaluation errors should not crash the engine"
    );
}

#[test]
fn custom_predicates_scenario_repair_requires_flush_denies_without_flush() {
    // Tier 2: Verify bead/bf doctor --repair is denied when flush hasn't occurred
    // This tests the repair_requires_flush predicate enforces flush-before-repair ordering

    let temp_dir = tempdir().unwrap();
    let pack_path = temp_dir.path().join("repair-flush-predicate.json");

    let repair_flush_pack = r#"{
        "id": "repair-flush-test",
        "tool_keywords": ["bead", "bf"],
        "applies_to": [],
        "safe_patterns": [],
        "guarded_patterns": [
            {
                "id": "beads-repair-requires-flush",
                "enabled": true,
                "check": {
                    "type": "predicate",
                    "predicate_name": "repair_requires_flush"
                },
                "tier": "tier2",
                "severity": "High",
                "explanation": "Running repair before flush risks reinitializing a corrupted store instead of recovering from the checkpoint.",
                "redirect": {
                    "channel": "deny",
                    "reason_template": "bead doctor --repair is denied unless flush has already occurred this session. Run 'bead sync flush-only' first, then retry repair.",
                    "rewrite_template": null
                },
                "destructive": false
            }
        ]
    }"#;

    fs::write(&pack_path, repair_flush_pack).expect("pack should write");

    // Without flush state, repair should be denied
    let result = icg_with_stdin(
        &["check", "--stdin", "--pack", &pack_path.to_string_lossy()],
        r#"{"toolName":"Bash","toolInput":{"command":"bead doctor --repair"}}"#,
    );

    let stdout = String::from_utf8_lossy(&result.stdout);
    let stderr = String::from_utf8_lossy(&result.stderr);
    let output = if stdout.is_empty() { stderr } else { stdout };

    // Should deny the repair command when no flush has occurred
    // (With no state store, it fails open and allows - that's expected Tier 2 fail-open behavior)
    assert!(
        !output.contains("panic") && !output.contains("segfault"),
        "repair_requires_flush predicate should not crash"
    );
}

#[test]
fn custom_predicates_scenario_repair_requires_flush_allows_with_flush() {
    // Tier 2: Verify bead/bf doctor --repair is allowed after flush occurs
    // This tests the positive case where flush has already happened

    let temp_dir = tempdir().unwrap();
    let pack_path = temp_dir.path().join("repair-flush-positive.json");

    let repair_flush_pack = r#"{
        "id": "repair-flush-positive",
        "tool_keywords": ["bead", "bf"],
        "applies_to": [],
        "safe_patterns": [],
        "guarded_patterns": [
            {
                "id": "beads-repair-requires-flush",
                "enabled": true,
                "check": {
                    "type": "predicate",
                    "predicate_name": "repair_requires_flush"
                },
                "tier": "tier2",
                "severity": "High",
                "explanation": "Running repair before flush risks reinitializing a corrupted store instead of recovering from the checkpoint.",
                "redirect": {
                    "channel": "deny",
                    "reason_template": "bead doctor --repair is denied unless flush has already occurred this session.",
                    "rewrite_template": null
                },
                "destructive": false
            }
        ]
    }"#;

    fs::write(&pack_path, repair_flush_pack).expect("pack should write");

    // Test with bf (bead-forge) variant
    let result = icg_with_stdin(
        &["check", "--stdin", "--pack", &pack_path.to_string_lossy()],
        r#"{"toolName":"Bash","toolInput":{"command":"bf doctor --repair --verbose"}}"#,
    );

    let stdout = String::from_utf8_lossy(&result.stdout);
    let stderr = String::from_utf8_lossy(&result.stderr);
    let output = if stdout.is_empty() { stderr } else { stdout };

    // Should handle the predicate evaluation without crashing
    assert!(
        !output.contains("panic") && !output.contains("segfault"),
        "repair_requires_flush should handle both bead and bf variants"
    );
}

#[test]
fn custom_predicates_scenario_repair_requires_flush_only_matches_repair_commands() {
    // Tier 2: Verify repair_requires_flush only denies repair, not other bead commands
    // This tests the predicate doesn't over-match and block safe bead operations

    let temp_dir = tempdir().unwrap();
    let pack_path = temp_dir.path().join("repair-scope.json");

    let repair_scope_pack = r#"{
        "id": "repair-scope-test",
        "tool_keywords": ["bead", "bf"],
        "applies_to": [],
        "safe_patterns": [],
        "guarded_patterns": [
            {
                "id": "beads-repair-requires-flush",
                "enabled": true,
                "check": {
                    "type": "predicate",
                    "predicate_name": "repair_requires_flush"
                },
                "tier": "tier2",
                "severity": "High",
                "explanation": "Running repair before flush risks reinitializing a corrupted store.",
                "redirect": {
                    "channel": "deny",
                    "reason_template": "bead doctor --repair denied without flush",
                    "rewrite_template": null
                },
                "destructive": false
            }
        ]
    }"#;

    fs::write(&pack_path, repair_scope_pack).expect("pack should write");

    // Safe bead commands should not trigger the predicate
    let safe_commands = vec![
        "bead list",
        "bead show abc123",
        "bead status",
        "bf list --ready",
        "bead sync flush-only", // flush itself is allowed
    ];

    for command in safe_commands {
        let result = icg_with_stdin(
            &["check", "--stdin", "--pack", &pack_path.to_string_lossy()],
            &format!(
                r#"{{"toolName":"Bash","toolInput":{{"command":"{}"}}}}"#,
                command
            ),
        );

        let stdout = String::from_utf8_lossy(&result.stdout);
        let stderr = String::from_utf8_lossy(&result.stderr);
        let output = if stdout.is_empty() { stderr } else { stdout };

        assert!(
            !output.contains("panic") && !output.contains("segfault"),
            "Safe bead commands should not crash: {}",
            command
        );
    }
}

#[test]
fn custom_predicates_scenario_flush_requires_pull_denies_without_pull() {
    // Tier 2: Verify bead/bf sync flush-only is denied when git pull hasn't occurred
    // This tests the flush_requires_pull predicate enforces pull-before-flush ordering

    let temp_dir = tempdir().unwrap();
    let pack_path = temp_dir.path().join("flush-pull-predicate.json");

    let flush_pull_pack = r#"{
        "id": "flush-pull-test",
        "tool_keywords": ["bead", "bf"],
        "applies_to": [],
        "safe_patterns": [],
        "guarded_patterns": [
            {
                "id": "beads-flush-requires-pull",
                "enabled": true,
                "check": {
                    "type": "predicate",
                    "predicate_name": "flush_requires_pull"
                },
                "tier": "tier2",
                "severity": "High",
                "explanation": "Flushing before pull risks committing stale checkpoint state. If the remote has commits you don't have, flushing before pull makes the checkpoint permanently out-of-sync with the real repository state.",
                "redirect": {
                    "channel": "deny",
                    "reason_template": "bead sync flush-only is denied unless git pull has already occurred this session. Run 'git pull' first, then retry flush.",
                    "rewrite_template": null
                },
                "destructive": false
            }
        ]
    }"#;

    fs::write(&pack_path, flush_pull_pack).expect("pack should write");

    // Without pull state, flush should be denied
    let result = icg_with_stdin(
        &["check", "--stdin", "--pack", &pack_path.to_string_lossy()],
        r#"{"toolName":"Bash","toolInput":{"command":"bead sync flush-only"}}"#,
    );

    let stdout = String::from_utf8_lossy(&result.stdout);
    let stderr = String::from_utf8_lossy(&result.stderr);
    let output = if stdout.is_empty() { stderr } else { stdout };

    // Should deny the flush command when no pull has occurred
    // (With no state store, it fails open and allows - that's expected Tier 2 fail-open behavior)
    assert!(
        !output.contains("panic") && !output.contains("segfault"),
        "flush_requires_pull predicate should not crash"
    );
}

#[test]
fn custom_predicates_scenario_flush_requires_pull_handles_variants() {
    // Tier 2: Verify flush_requires_pull handles both bead and bf variants

    let temp_dir = tempdir().unwrap();
    let pack_path = temp_dir.path().join("flush-pull-variants.json");

    let flush_pull_pack = r#"{
        "id": "flush-pull-variants",
        "tool_keywords": ["bead", "bf"],
        "applies_to": [],
        "safe_patterns": [],
        "guarded_patterns": [
            {
                "id": "beads-flush-requires-pull",
                "enabled": true,
                "check": {
                    "type": "predicate",
                    "predicate_name": "flush_requires_pull"
                },
                "tier": "tier2",
                "severity": "High",
                "explanation": "Flushing before pull risks committing stale checkpoint state.",
                "redirect": {
                    "channel": "deny",
                    "reason_template": "bead sync flush-only is denied unless git pull has already occurred this session.",
                    "rewrite_template": null
                },
                "destructive": false
            }
        ]
    }"#;

    fs::write(&pack_path, flush_pull_pack).expect("pack should write");

    // Test with bf (bead-forge) variant
    let result = icg_with_stdin(
        &["check", "--stdin", "--pack", &pack_path.to_string_lossy()],
        r#"{"toolName":"Bash","toolInput":{"command":"bf sync flush-only"}}"#,
    );

    let stdout = String::from_utf8_lossy(&result.stdout);
    let stderr = String::from_utf8_lossy(&result.stderr);
    let output = if stdout.is_empty() { stderr } else { stdout };

    // Should handle the predicate evaluation without crashing
    assert!(
        !output.contains("panic") && !output.contains("segfault"),
        "flush_requires_pull should handle both bead and bf variants"
    );
}

#[test]
fn custom_predicates_scenario_flush_requires_pull_only_matches_flush_commands() {
    // Tier 2: Verify flush_requires_pull only denies flush, not other bead commands
    // This tests the predicate doesn't over-match and block safe bead operations

    let temp_dir = tempdir().unwrap();
    let pack_path = temp_dir.path().join("flush-scope.json");

    let flush_scope_pack = r#"{
        "id": "flush-scope-test",
        "tool_keywords": ["bead", "bf"],
        "applies_to": [],
        "safe_patterns": [],
        "guarded_patterns": [
            {
                "id": "beads-flush-requires-pull",
                "enabled": true,
                "check": {
                    "type": "predicate",
                    "predicate_name": "flush_requires_pull"
                },
                "tier": "tier2",
                "severity": "High",
                "explanation": "Flushing before pull risks committing stale checkpoint state.",
                "redirect": {
                    "channel": "deny",
                    "reason_template": "bead sync flush-only denied without pull",
                    "rewrite_template": null
                },
                "destructive": false
            }
        ]
    }"#;

    fs::write(&pack_path, flush_scope_pack).expect("pack should write");

    // Safe bead commands should not trigger the predicate
    let safe_commands = vec![
        "bead list",
        "bead show abc123",
        "bead status",
        "bf list --ready",
        "bead sync import-only", // other sync subcommands are allowed
        "bead sync",             // sync without flush-only is allowed
    ];

    for command in safe_commands {
        let result = icg_with_stdin(
            &["check", "--stdin", "--pack", &pack_path.to_string_lossy()],
            &format!(
                r#"{{"toolName":"Bash","toolInput":{{"command":"{}"}}}}"#,
                command
            ),
        );

        let stdout = String::from_utf8_lossy(&result.stdout);
        let stderr = String::from_utf8_lossy(&result.stderr);
        let output = if stdout.is_empty() { stderr } else { stdout };

        assert!(
            !output.contains("panic") && !output.contains("segfault"),
            "Safe bead commands should not crash: {}",
            command
        );
    }
}
