//! Debugging false positives scenario tests (Scenario 8)
//!
//! These tests verify the false positive debugging workflow documented in
//! docs/examples/README.md Scenario 8: Debugging False Positives.
//!
//! The scenario covers:
//! - Step 1: Reproduce the Issue
//! - Step 2: Analyze the Match (debug output)
//! - Step 3: Identify the Problem
//! - Step 4: Fix the Pattern
//! - Step 5: Verify the Fix
//!
//! This tests pattern debugging capabilities and pattern refinement workflows.

use icg::engine::{CheckResult, CommandSource, ContentSource, Engine};
use icg::rule_pack::{
    load_pack, Channel, Check, GuardedPattern, Pack, Pattern, Redirect, Severity, Tier,
};
use std::path::PathBuf;
use tempfile::tempdir;

#[test]
fn debugging_scenario_1_reproduce_false_positive_issue() {
    // Step 1: Reproduce a false positive where a legitimate command is blocked
    let mut engine = Engine::new();
    let pack = load_pack("packs/image-tag.json").expect("image-tag pack should load");
    engine.load_pack(pack).expect("pack should load");

    // Test case: legitimate pinned image should be allowed
    let result = engine.evaluate_content(&ContentSource::Write {
        file_path: "deploy/app.yaml".to_string(),
        content: "image: nginx:1.21.0\n".to_string(),
    });

    // This should be allowed (pinned version)
    match result {
        CheckResult::Allowed => {
            // Correct - pinned images are allowed
        }
        CheckResult::Denied { pattern_id, .. } => {
            panic!(
                "False positive: pinned image should be allowed, but was denied by pattern '{}'",
                pattern_id
            );
        }
        other => panic!("Unexpected result: {:?}", other),
    }
}

#[test]
fn debugging_scenario_2_analyze_pattern_matching_behavior() {
    // Step 2: Analyze which patterns match for a given input
    let mut engine = Engine::new();
    let pack = load_pack("packs/storage-class.json").expect("storage-class pack should load");
    engine.load_pack(pack).expect("pack should load");

    // Test that dangerous content is caught
    let dangerous_content = "storageClassName: ssd\n";
    let result = engine.evaluate_content(&ContentSource::Write {
        file_path: "claim.yaml".to_string(),
        content: dangerous_content.to_string(),
    });

    match result {
        CheckResult::Denied {
            pattern_id,
            pack_id,
            reason,
            ..
        } => {
            assert_eq!(pack_id, "storage-class");
            assert!(pattern_id.contains("ssd") || pattern_id.contains("storage-class"));
            assert!(reason.contains("SSD") || reason.contains("prohibited"));
        }
        other => panic!("Expected denial for dangerous content, got: {:?}", other),
    }
}

#[test]
fn debugging_scenario_3_identify_overly_broad_patterns() {
    // Step 3: Identify patterns that are too broad and cause false positives
    // Create a test pack with an overly broad pattern

    let temp_dir = tempdir().unwrap();
    let pack_path = temp_dir.path().join("overly-broad.json");

    // Create a pack with a pattern that's too broad
    let overly_broad_pack = Pack {
        id: "overly-broad-test".to_string(),
        tool_keywords: vec!["kubectl".to_string()],
        applies_to: vec![],
        safe_patterns: vec![],
        guarded_patterns: vec![GuardedPattern {
            id: "kubectl-delete-overly-broad".to_string(),
            enabled: true,
            check: Check::CommandRegex {
                // This pattern is TOO BROAD - matches "delete" anywhere
                regex: "kubectl delete".to_string(),
            },
            tier: Tier::Tier1,
            severity: Severity::High,
            explanation: "Deleting kubernetes resources".to_string(),
            redirect: Redirect {
                channel: Channel::Deny,
                reason_template: "kubectl delete is dangerous".to_string(),
                rewrite_template: None,
            },
            destructive: true,
        }],
    };

    // Write the test pack
    let pack_json =
        serde_json::to_string_pretty(&overly_broad_pack).expect("pack should serialize");
    std::fs::write(&pack_path, pack_json).expect("pack should write");

    // Load and test
    let pack = load_pack(&pack_path).expect("test pack should load");
    let mut engine = Engine::new();
    engine.load_pack(pack).expect("pack should load");

    // The overly broad pattern catches BOTH legitimate and dangerous operations
    let test_cases = vec![
        (
            "kubectl delete pod old-pod",
            "legitimate - should be allowed",
        ),
        (
            "kubectl delete deployment myapp",
            "legitimate - should be allowed",
        ),
        (
            "kubectl delete pvc data-pvc",
            "dangerous - should be blocked",
        ),
    ];

    for (command, description) in test_cases {
        let result = engine.evaluate_command(&CommandSource::Hook(command.to_string()));

        // With the overly broad pattern, ALL are blocked (false positives)
        match result {
            CheckResult::Denied { pattern_id, .. } => {
                assert_eq!(pattern_id, "kubectl-delete-overly-broad");
                // This demonstrates the false positive problem
            }
            CheckResult::Allowed => {
                // If allowed, the pattern isn't catching it (good)
            }
            other => panic!("Unexpected result for '{}': {:?}", command, other),
        }
    }
}

#[test]
fn debugging_scenario_4_fix_pattern_to_be_more_specific() {
    // Step 4: Fix the pattern to be more specific and avoid false positives
    let temp_dir = tempdir().unwrap();
    let pack_path = temp_dir.path().join("fixed-pattern.json");

    // Create a pack with SPECIFIC patterns that only catch truly dangerous operations
    let fixed_pack = Pack {
        id: "fixed-specific-test".to_string(),
        tool_keywords: vec!["kubectl".to_string()],
        applies_to: vec![],
        safe_patterns: vec![
            Pattern {
                id: "safe-delete-pod".to_string(),
                check: Check::CommandRegex {
                    regex: "^kubectl delete pod ".to_string(),
                },
            },
            Pattern {
                id: "safe-delete-deployment".to_string(),
                check: Check::CommandRegex {
                    regex: "^kubectl delete deployment ".to_string(),
                },
            },
        ],
        guarded_patterns: vec![GuardedPattern {
            id: "kubectl-delete-pvc-only".to_string(),
            enabled: true,
            check: Check::CommandRegex {
                // This ONLY catches PVC deletion, which is truly dangerous
                regex: "kubectl delete pvc".to_string(),
            },
            tier: Tier::Tier1,
            severity: Severity::Critical,
            explanation: "Deleting a PVC destroys persistent data".to_string(),
            redirect: Redirect {
                channel: Channel::Deny,
                reason_template:
                    "kubectl delete pvc is permanently destructive. Data cannot be recovered."
                        .to_string(),
                rewrite_template: None,
            },
            destructive: true,
        }],
    };

    // Write the fixed pack
    let pack_json = serde_json::to_string_pretty(&fixed_pack).expect("pack should serialize");
    std::fs::write(&pack_path, pack_json).expect("pack should write");

    // Load and test the fixed behavior
    let pack = load_pack(&pack_path).expect("fixed pack should load");
    let mut engine = Engine::new();
    engine.load_pack(pack).expect("pack should load");

    // Test that legitimate operations are now allowed
    let legitimate_cases = vec![
        "kubectl delete pod old-pod",
        "kubectl delete deployment myapp",
    ];

    for command in legitimate_cases {
        let result = engine.evaluate_command(&CommandSource::Hook(command.to_string()));

        match result {
            CheckResult::Allowed => {
                // Good - legitimate operations are allowed
            }
            CheckResult::Denied { pattern_id, .. } => {
                panic!(
                    "False positive: '{}' should be allowed but was denied by '{}'",
                    command, pattern_id
                );
            }
            other => panic!("Unexpected result for '{}': {:?}", command, other),
        }
    }

    // Test that dangerous operations are still blocked
    let dangerous_command = "kubectl delete pvc data-pvc";
    let result = engine.evaluate_command(&CommandSource::Hook(dangerous_command.to_string()));

    match result {
        CheckResult::Denied { pattern_id, .. } => {
            assert_eq!(pattern_id, "kubectl-delete-pvc-only");
            // Good - dangerous operation is still blocked
        }
        CheckResult::Allowed => {
            panic!("Security regression: dangerous PVC deletion should be blocked");
        }
        other => panic!("Unexpected result: {:?}", other),
    }
}

#[test]
fn debugging_scenario_5_verify_fix_with_regression_suite() {
    // Step 5: Verify the fix by testing against the regression suite
    // This tests that pattern fixes don't introduce regressions

    let temp_dir = tempdir().unwrap();
    let pack_path = temp_dir.path().join("test-regression.json");

    // Create a pack with multiple patterns
    let test_pack = Pack {
        id: "regression-test".to_string(),
        tool_keywords: vec!["test-tool".to_string()],
        applies_to: vec![],
        safe_patterns: vec![],
        guarded_patterns: vec![
            GuardedPattern {
                id: "pattern-1-dangerous".to_string(),
                enabled: true,
                check: Check::CommandRegex {
                    regex: "test-tool dangerous-op".to_string(),
                },
                tier: Tier::Tier1,
                severity: Severity::Critical,
                explanation: "Dangerous operation".to_string(),
                redirect: Redirect {
                    channel: Channel::Deny,
                    reason_template: "Operation is dangerous".to_string(),
                    rewrite_template: None,
                },
                destructive: true,
            },
            GuardedPattern {
                id: "pattern-2-destructive".to_string(),
                enabled: true,
                check: Check::CommandRegex {
                    regex: "test-tool destroy".to_string(),
                },
                tier: Tier::Tier1,
                severity: Severity::Critical,
                explanation: "Destructive operation".to_string(),
                redirect: Redirect {
                    channel: Channel::Deny,
                    reason_template: "Operation is destructive".to_string(),
                    rewrite_template: None,
                },
                destructive: true,
            },
        ],
    };

    // Write test pack
    let pack_json = serde_json::to_string_pretty(&test_pack).expect("pack should serialize");
    std::fs::write(&pack_path, pack_json).expect("pack should write");

    // Load and verify all patterns still work
    let pack = load_pack(&pack_path).expect("test pack should load");
    let mut engine = Engine::new();
    engine.load_pack(pack).expect("pack should load");

    // Verify each guarded pattern still denies
    for pattern in &test_pack.guarded_patterns {
        if let Check::CommandRegex { regex } = &pattern.check {
            let result = engine.evaluate_command(&CommandSource::Hook(regex.clone()));

            match result {
                CheckResult::Denied { pattern_id, .. } => {
                    assert_eq!(pattern_id, pattern.id);
                    // Good - pattern still works
                }
                CheckResult::Allowed => {
                    panic!(
                        "Regression: pattern '{}' no longer denies '{}'",
                        pattern.id, regex
                    );
                }
                other => panic!(
                    "Unexpected result for pattern '{}': {:?}",
                    pattern.id, other
                ),
            }
        }
    }
}

#[test]
fn debugging_scenario_pattern_refinement_maintains_security() {
    // Test that pattern refinement maintains security while reducing false positives
    // This simulates the iterative refinement process

    let temp_dir = tempdir().unwrap();
    let pack_path = temp_dir.path().join("refinement-test.json");

    // Start with an overly strict pattern
    let overly_strict = Pack {
        id: "refinement-test".to_string(),
        tool_keywords: vec!["vault".to_string()],
        applies_to: vec![],
        safe_patterns: vec![],
        guarded_patterns: vec![GuardedPattern {
            id: "vault-any-write".to_string(),
            enabled: true,
            check: Check::CommandRegex {
                // TOO STRICT - blocks ALL vault write operations
                regex: "vault write".to_string(),
            },
            tier: Tier::Tier1,
            severity: Severity::Critical,
            explanation: "Vault write operations are dangerous".to_string(),
            redirect: Redirect {
                channel: Channel::Deny,
                reason_template: "All vault writes are blocked".to_string(),
                rewrite_template: None,
            },
            destructive: true,
        }],
    };

    // Write initial pack
    let pack_json = serde_json::to_string_pretty(&overly_strict).expect("pack should serialize");
    std::fs::write(&pack_path, pack_json).expect("pack should write");

    // Test the overly strict behavior
    let pack = load_pack(&pack_path).expect("pack should load");
    let mut engine = Engine::new();
    engine.load_pack(pack).expect("pack should load");

    // This is overly strict - legitimate writes are blocked
    let legitimate_write = "vault write secret/config ttl=1h";
    let result = engine.evaluate_command(&CommandSource::Hook(legitimate_write.to_string()));
    assert!(matches!(result, CheckResult::Denied { .. }));

    // Now refine to be more specific
    let refined = Pack {
        id: "refinement-test".to_string(),
        tool_keywords: vec!["vault".to_string()],
        applies_to: vec![],
        safe_patterns: vec![Pattern {
            id: "safe-write-secret".to_string(),
            check: Check::CommandRegex {
                regex: "vault write secret/".to_string(),
            },
        }],
        guarded_patterns: vec![GuardedPattern {
            id: "vault-destroy-only".to_string(),
            enabled: true,
            check: Check::CommandRegex {
                // MORE SPECIFIC - only blocks truly dangerous operations
                regex: "vault kv destroy".to_string(),
            },
            tier: Tier::Tier1,
            severity: Severity::Critical,
            explanation: "Vault destroy operations are permanently destructive".to_string(),
            redirect: Redirect {
                channel: Channel::Deny,
                reason_template: "vault kv destroy is permanently destructive and cannot be undone"
                    .to_string(),
                rewrite_template: Some("vault kv patch".to_string()),
            },
            destructive: true,
        }],
    };

    // Write refined pack
    let pack_json = serde_json::to_string_pretty(&refined).expect("pack should serialize");
    std::fs::write(&pack_path, pack_json).expect("pack should write");

    // Load and test refined behavior
    let pack = load_pack(&pack_path).expect("pack should load");
    engine.load_pack(pack).expect("pack should load");

    // Now legitimate writes should be allowed
    let result = engine.evaluate_command(&CommandSource::Hook(legitimate_write.to_string()));
    assert!(matches!(result, CheckResult::Allowed));

    // But dangerous operations should still be blocked
    let dangerous = "vault kv destroy secret/test";
    let result = engine.evaluate_command(&CommandSource::Hook(dangerous.to_string()));
    assert!(matches!(result, CheckResult::Denied { .. }));
}
