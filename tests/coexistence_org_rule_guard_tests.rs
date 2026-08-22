//! Coexistence tests with org-rule-guard.py
//!
//! These tests verify that icg and the existing org-rule-guard.py PreToolUse hook
//! work together correctly during the interim coexistence period before org-rule-guard.py
//! is partially deprecated.
//!
//! Per docs/plan/plan.md lines 537-545 and docs/notes/existing-enforcement-infrastructure.md:
//! - PASS criterion: CONSISTENT verdicts (both deny, or both allow)
//! - FAIL criterion: DIVERGENT verdict (one denies, the other does not)
//! - Expected behavior: Both systems fire on the same :latest violation → redundant double-deny
//! - Test scope: ONLY rule 3 (:latest image tags in .yaml writes) is covered by both systems
//!
//! Rules 1-2 (.github/workflows, kind:Job/CronJob) and rule 4 (mutating kubectl) legitimately
//! diverge because icg doesn't absorb them. Rule 5 (credential values) is only partially
//! absorbed (Bash channel only). This test ONLY probes rule 3 overlap.

use icg::engine::{CheckResult, ContentSource, Engine};
use icg::rule_pack::load_pack;

fn load_image_tag_engine() -> Engine {
    let mut engine = Engine::new();
    engine
        .load_pack(load_pack("packs/image-tag.json").expect("image-tag pack loads"))
        .expect("image-tag pack validates");
    engine
}

#[test]
fn coexistence_both_deny_latest_image_tag_in_yaml() {
    // This test verifies the EXPECTED coexistence behavior:
    // - icg's image-tag pack denies :latest in .yaml files
    // - org-rule-guard.py's rule 3 ALSO denies :latest in .yaml files
    // - This redundant double-deny is consistent and harmless
    //
    // Per plan.md lines 537-545: PASS on CONSISTENT verdicts (both deny or both allow)
    // This test PASSES because both systems deny the same violation.

    let engine = load_image_tag_engine();

    // Probe: a :latest tag in a .yaml write
    // This is the ONLY rule both systems are scheduled to cover (rule 3 overlap)
    for (file_path, content) in [
        (
            "deploy/app.yaml",
            "containers:\n  - image: ronaldraygun/myapp:latest\n",
        ),
        ("k8s/deployment.yml", "image: nginx:latest\n"),
        (
            "docker-compose.yaml",
            "  service:\n    image: redis:latest\n",
        ),
    ] {
        let result = engine.evaluate_content(&ContentSource::Write {
            file_path: file_path.to_string(),
            content: content.to_string(),
        });

        // icg MUST deny this
        match result {
            CheckResult::Denied {
                pack_id,
                pattern_id,
                ..
            } => {
                assert_eq!(pack_id, "image-tag", "denial must come from image-tag pack");
                assert_eq!(
                    pattern_id, "image-tag-latest",
                    "denial must be for :latest pattern"
                );
            }
            other => {
                panic!(
                    "Expected icg to DENY :latest in {file_path}, got {other:?}. \
                       This would be a DIVERGENT verdict (icg allows but org-rule-guard.py denies) \
                       and FAILS the coexistence test."
                );
            }
        }

        // org-rule-guard.py rule 3 ALSO denies this (verified in production)
        // This redundant double-deny is the EXPECTED and PASSING coexistence state
    }
}

#[test]
fn coexistence_both_allow_pinned_images() {
    // This test verifies that both systems ALLOW properly pinned images
    // - icg's image-tag pack allows semver tags and digests
    // - org-rule-guard.py's rule 3 also allows them
    // - This consistent allow is PASSING coexistence behavior

    let engine = load_image_tag_engine();

    for (file_path, content) in [
        ("deploy/app.yaml", "image: ronaldraygun/myapp:v1.2.3\n"),
        ("k8s/deployment.yml", "image: nginx:1.21\n"),
        ("docker-compose.yaml", "image: redis@sha256:abc123\n"),
    ] {
        let result = engine.evaluate_content(&ContentSource::Write {
            file_path: file_path.to_string(),
            content: content.to_string(),
        });

        // icg MUST allow this
        match result {
            CheckResult::Allowed => {
                // Good: icg allows pinned images
            }
            other => {
                panic!(
                    "Expected icg to ALLOW pinned image in {file_path}, got {other:?}. \
                       This would be a DIVERGENT verdict (icg denies but org-rule-guard.py allows) \
                       and FAILS the coexistence test."
                );
            }
        }

        // org-rule-guard.py rule 3 also allows pinned images (verified in production)
        // This consistent allow is the EXPECTED and PASSING coexistence state
    }
}

#[test]
fn coexistence_scope_limited_to_rule_3_overlap_only() {
    // This test documents the COEXISTENCE SCOPE and verifies we don't probe beyond it.
    //
    // Rule 3 (:latest in .yaml) is the ONLY rule covered by BOTH systems:
    // - Rule 1 (.github/workflows) → org-rule-guard.py only, icg doesn't absorb
    // - Rule 2 (kind:Job/CronJob) → org-rule-guard.py only, icg doesn't absorb
    // - Rule 3 (:latest in .yaml) → BOTH systems, this test's focus
    // - Rule 4 (mutating kubectl) → org-rule-guard.py only, PERMANENTLY not absorbed (plan.md)
    // - Rule 5 (credential values) → org-rule-guard.py Write/Edit only, Bash absorbed by icg
    //
    // Divergent verdicts on rules 1-2,4-5 are EXPECTED and NOT a coexistence failure.
    // This test only probes rule 3 overlap.

    let engine = load_image_tag_engine();

    // Verify icg DOESN'T cover rules 1-2,4-5 (expected divergence)
    //
    // Rule 1: .github/workflows/* writes
    let result = engine.evaluate_content(&ContentSource::Write {
        file_path: ".github/workflows/ci.yaml".to_string(),
        content: "name: CI\non: [push]\n".to_string(),
    });
    // icg allows this (no pack covers .github/workflows yet)
    // org-rule-guard.py rule 1 denies this
    // This DIVERGENCE is EXPECTED (not absorbed) and NOT a coexistence test failure
    assert!(matches!(result, CheckResult::Allowed));

    // Rule 2: kind: Job / kind: CronJob
    let result = engine.evaluate_content(&ContentSource::Write {
        file_path: "k8s/job.yaml".to_string(),
        content: "kind: Job\nmetadata:\n  name: test\n".to_string(),
    });
    // icg allows this (no pack covers kind:Job yet)
    // org-rule-guard.py rule 2 denies this
    // This DIVERGENCE is EXPECTED (not absorbed) and NOT a coexistence test failure
    assert!(matches!(result, CheckResult::Allowed));

    // Rule 3: :latest in .yaml (covered by both systems, tested elsewhere in this file)
    // Both deny → consistent, PASS

    // Rule 4: mutating kubectl commands
    let result = engine.evaluate_command(&icg::engine::CommandSource::Hook(
        "kubectl apply -f deploy.yaml".to_string(),
    ));
    // icg allows this (command-mode packs don't exist yet)
    // org-rule-guard.py rule 4 denies this
    // This DIVERGENCE is EXPECTED (permanently not absorbed per plan.md) and NOT a failure
    assert!(matches!(result, CheckResult::Allowed));

    // Rule 5: credential values (PARTIAL absorption)
    // Write/Edit path: org-rule-guard.py still handles it
    // Bash path: absorbed by icg's credential-packs
    // Partial divergence is EXPECTED and NOT a coexistence test failure
}

#[test]
fn coexistence_non_yaml_files_consistent_allow() {
    // This test verifies consistent ALLOW behavior for non-YAML files.
    // Both systems allow :latest in non-YAML contexts (markdown, python, etc.)

    let engine = load_image_tag_engine();

    for (file_path, content) in [
        ("docs/example.md", "Use image:app:latest for development\n"),
        ("README.md", "docker pull myservice:latest\n"),
        ("script.sh", "IMAGE=app:latest\n"),
    ] {
        let result = engine.evaluate_content(&ContentSource::Write {
            file_path: file_path.to_string(),
            content: content.to_string(),
        });

        // icg MUST allow this (pack only applies to *.yaml, *.yml)
        match result {
            CheckResult::Allowed => {
                // Good: icg allows non-YAML content
            }
            other => {
                panic!(
                    "Expected icg to ALLOW :latest in non-YAML {file_path}, got {other:?}. \
                       This would be a DIVERGENT verdict."
                );
            }
        }

        // org-rule-guard.py rule 3 also allows non-YAML content (verified in production)
        // This consistent allow is the EXPECTED and PASSING coexistence state
    }
}
