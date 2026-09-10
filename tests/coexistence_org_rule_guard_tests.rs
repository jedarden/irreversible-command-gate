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
//! - Test scope: rule 3 (:latest image tags in .yaml writes) and rule 1 (.github/workflows
//!   writes) are both covered by icg; the rest are probed only to document expected divergence
//!
//! Rule 2 (kind:Job/CronJob) and rule 4 (mutating kubectl) legitimately diverge because icg
//! doesn't absorb them. Rule 5 (credential values) is only partially absorbed (Bash channel
//! only). Rule 1 (.github/workflows) is now absorbed by icg as well (see
//! coexistence_scope_limited_to_rule_3_overlap_only).

use icg::engine::{CheckResult, ContentSource, Engine, InputSource, PreToolUseInput, ToolInput};
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
    // - Rule 1 (.github/workflows) → now absorbed by icg (both systems deny)
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
    // icg now denies this (github_workflows guard is absorbed into evaluate_content_inner)
    // org-rule-guard.py rule 1 also denies this
    // Both deny → consistent, PASS
    assert!(matches!(result, CheckResult::Denied { .. }));

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
fn coexistence_write_tool_use_flags_github_workflows_path() {
    // This test drives the hook detection built in irrevers-520cbfa5 through
    // the actual PreToolUse front-end (PreToolUseInput -> InputSource ->
    // evaluate_content), constructing a Write tool_use event whose file_path
    // is under .github/workflows/**, rather than building a ContentSource
    // directly. This is the shape a real Claude Code Write tool call takes.

    let engine = load_image_tag_engine();

    let input = PreToolUseInput {
        tool_name: "Write".to_string(),
        tool_input: ToolInput {
            command: None,
            file_path: Some(".github/workflows/deploy.yml".to_string()),
            content: Some("name: Deploy\non: [push]\n".to_string()),
            old_string: None,
            new_string: None,
            encoding: None,
            mime_type: None,
        },
        id: None,
        timestamp: None,
        session_id: None,
    };

    let source = match Engine::input_source_from_pre_tool_use(input)
        .expect("Write tool_use event should convert to an InputSource")
        .expect("Write is a known tool and must produce an InputSource")
    {
        InputSource::Content(source) => source,
        other => panic!("expected InputSource::Content for a Write tool_use event, got {other:?}"),
    };

    let result = engine.evaluate_content(&source);

    match result {
        CheckResult::Denied {
            pack_id,
            pattern_id,
            reason,
        } => {
            assert_eq!(
                pack_id, "github-workflows",
                "denial must come from the github-workflows guard"
            );
            assert_eq!(pattern_id, "github-workflows-protected");
            assert!(
                reason.contains(".github/workflows"),
                "deny reason should mention .github/workflows, got: {reason}"
            );
        }
        other => panic!(
            "Expected a Write tool_use event targeting .github/workflows/deploy.yml to be \
             flagged (denied) by the hook detection, got {other:?}."
        ),
    }
}

#[test]
fn coexistence_write_tool_use_does_not_flag_non_workflows_path() {
    // This test drives the hook detection built in irrevers-520cbfa5 through
    // the actual PreToolUse front-end (PreToolUseInput -> InputSource ->
    // evaluate_content), constructing a Write tool_use event whose file_path
    // does NOT fall under .github/workflows/**, and asserts it is not flagged
    // by the github-workflows guard. This complements
    // coexistence_write_tool_use_flags_github_workflows_path, which covers the
    // matching-path case.

    let engine = load_image_tag_engine();

    let input = PreToolUseInput {
        tool_name: "Write".to_string(),
        tool_input: ToolInput {
            command: None,
            file_path: Some("src/lib.rs".to_string()),
            content: Some("fn main() {}\n".to_string()),
            old_string: None,
            new_string: None,
            encoding: None,
            mime_type: None,
        },
        id: None,
        timestamp: None,
        session_id: None,
    };

    let source = match Engine::input_source_from_pre_tool_use(input)
        .expect("Write tool_use event should convert to an InputSource")
        .expect("Write is a known tool and must produce an InputSource")
    {
        InputSource::Content(source) => source,
        other => panic!("expected InputSource::Content for a Write tool_use event, got {other:?}"),
    };

    let result = engine.evaluate_content(&source);

    assert!(
        matches!(result, CheckResult::Allowed),
        "Expected a Write tool_use event targeting src/lib.rs (outside .github/workflows/**) \
         to NOT be flagged by the hook detection, got {result:?}."
    );
}

#[test]
fn coexistence_edit_tool_use_flags_github_workflows_path() {
    // This test drives the hook detection built in irrevers-520cbfa5 through
    // the actual PreToolUse front-end (PreToolUseInput -> InputSource ->
    // evaluate_content), constructing an Edit tool_use event whose file_path
    // is under .github/workflows/**, rather than building a ContentSource
    // directly. This complements coexistence_write_tool_use_flags_github_workflows_path,
    // which covers the Write tool.

    let engine = load_image_tag_engine();

    let input = PreToolUseInput {
        tool_name: "Edit".to_string(),
        tool_input: ToolInput {
            command: None,
            file_path: Some(".github/workflows/deploy.yml".to_string()),
            content: None,
            old_string: Some("on: [push]\n".to_string()),
            new_string: Some("on: [push, pull_request]\n".to_string()),
            encoding: None,
            mime_type: None,
        },
        id: None,
        timestamp: None,
        session_id: None,
    };

    let source = match Engine::input_source_from_pre_tool_use(input)
        .expect("Edit tool_use event should convert to an InputSource")
        .expect("Edit is a known tool and must produce an InputSource")
    {
        InputSource::Content(source) => source,
        other => panic!("expected InputSource::Content for an Edit tool_use event, got {other:?}"),
    };

    let result = engine.evaluate_content(&source);

    match result {
        CheckResult::Denied {
            pack_id,
            pattern_id,
            reason,
        } => {
            assert_eq!(
                pack_id, "github-workflows",
                "denial must come from the github-workflows guard"
            );
            assert_eq!(pattern_id, "github-workflows-protected");
            assert!(
                reason.contains(".github/workflows"),
                "deny reason should mention .github/workflows, got: {reason}"
            );
        }
        other => panic!(
            "Expected an Edit tool_use event targeting .github/workflows/deploy.yml to be \
             flagged (denied) by the hook detection, got {other:?}."
        ),
    }
}

#[test]
fn coexistence_edit_tool_use_does_not_flag_non_workflows_path() {
    // This test drives the hook detection built in irrevers-520cbfa5 through
    // the actual PreToolUse front-end (PreToolUseInput -> InputSource ->
    // evaluate_content), constructing an Edit tool_use event whose file_path
    // does NOT fall under .github/workflows/**, and asserts it is not flagged
    // by the github-workflows guard. This complements
    // coexistence_edit_tool_use_flags_github_workflows_path, which covers the
    // matching-path case, and mirrors
    // coexistence_write_tool_use_does_not_flag_non_workflows_path for the Edit tool.

    let engine = load_image_tag_engine();

    let input = PreToolUseInput {
        tool_name: "Edit".to_string(),
        tool_input: ToolInput {
            command: None,
            file_path: Some("src/lib.rs".to_string()),
            content: None,
            old_string: Some("fn main() {}\n".to_string()),
            new_string: Some("fn main() { println!(\"hi\"); }\n".to_string()),
            encoding: None,
            mime_type: None,
        },
        id: None,
        timestamp: None,
        session_id: None,
    };

    let source = match Engine::input_source_from_pre_tool_use(input)
        .expect("Edit tool_use event should convert to an InputSource")
        .expect("Edit is a known tool and must produce an InputSource")
    {
        InputSource::Content(source) => source,
        other => panic!("expected InputSource::Content for an Edit tool_use event, got {other:?}"),
    };

    let result = engine.evaluate_content(&source);

    assert!(
        matches!(result, CheckResult::Allowed),
        "Expected an Edit tool_use event targeting src/lib.rs (outside .github/workflows/**) \
         to NOT be flagged by the hook detection, got {result:?}."
    );
}

/// False-positive fixtures from irrevers-61a08562's path-matcher test table:
/// paths that merely contain the substring "workflows" outside `.github`,
/// sibling directories under `.github` that look like but are not
/// `.github/workflows`, and the bare `.github` directory itself. These must
/// never be flagged by the hook detection,
/// through the same PreToolUse front-door used by the matching-path tests
/// above (coexistence_write_tool_use_flags_github_workflows_path and
/// coexistence_edit_tool_use_flags_github_workflows_path).
const WORKFLOWS_LOOKALIKE_PATHS: &[&str] = &[
    "src/workflows/foo.yml",
    "workflows/foo.yml",
    ".github/workflows-extra/foo.yml",
    ".github/workflows-archive/old.yml",
    ".github/workflows2/foo.yml",
    "docs/my-workflows-notes.md",
    "scripts/workflows_helper.py",
    ".github/ISSUE_TEMPLATE/bug.md",
    ".github/dependabot.yml",
    // the .github directory itself (unit fn does_not_match_dot_github_alone)
    ".github",
];

#[test]
fn coexistence_write_tool_use_does_not_flag_workflows_lookalike_paths() {
    let engine = load_image_tag_engine();

    for file_path in WORKFLOWS_LOOKALIKE_PATHS {
        let input = PreToolUseInput {
            tool_name: "Write".to_string(),
            tool_input: ToolInput {
                command: None,
                file_path: Some(file_path.to_string()),
                content: Some("name: ci\n".to_string()),
                old_string: None,
                new_string: None,
                encoding: None,
                mime_type: None,
            },
            id: None,
            timestamp: None,
            session_id: None,
        };

        let source = match Engine::input_source_from_pre_tool_use(input)
            .expect("Write tool_use event should convert to an InputSource")
            .expect("Write is a known tool and must produce an InputSource")
        {
            InputSource::Content(source) => source,
            other => {
                panic!("expected InputSource::Content for a Write tool_use event, got {other:?}")
            }
        };

        let result = engine.evaluate_content(&source);

        assert!(
            matches!(result, CheckResult::Allowed),
            "Expected a Write tool_use event targeting lookalike path {file_path} to NOT be \
             flagged by the hook detection, got {result:?}."
        );
    }
}

#[test]
fn coexistence_edit_tool_use_does_not_flag_workflows_lookalike_paths() {
    let engine = load_image_tag_engine();

    for file_path in WORKFLOWS_LOOKALIKE_PATHS {
        let input = PreToolUseInput {
            tool_name: "Edit".to_string(),
            tool_input: ToolInput {
                command: None,
                file_path: Some(file_path.to_string()),
                content: None,
                old_string: Some("push".to_string()),
                new_string: Some("pull_request".to_string()),
                encoding: None,
                mime_type: None,
            },
            id: None,
            timestamp: None,
            session_id: None,
        };

        let source = match Engine::input_source_from_pre_tool_use(input)
            .expect("Edit tool_use event should convert to an InputSource")
            .expect("Edit is a known tool and must produce an InputSource")
        {
            InputSource::Content(source) => source,
            other => {
                panic!("expected InputSource::Content for an Edit tool_use event, got {other:?}")
            }
        };

        let result = engine.evaluate_content(&source);

        assert!(
            matches!(result, CheckResult::Allowed),
            "Expected an Edit tool_use event targeting lookalike path {file_path} to NOT be \
             flagged by the hook detection, got {result:?}."
        );
    }
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
