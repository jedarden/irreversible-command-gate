use icg::coverage::{render_coverage_diff_report, run_coverage_diff};
use icg::engine::{CheckResult, CommandSource, ContentSource, Engine};
use icg::regression::{generate_regression_suite, verify_regression_suite};
use icg::rule_pack::{save_pack, Channel, Check, GuardedPattern, Pack, Redirect, Severity, Tier};
use tempfile::tempdir;

fn command_pattern(id: &str, enabled: bool) -> GuardedPattern {
    GuardedPattern {
        id: id.to_string(),
        enabled,
        check: Check::CommandRegex {
            regex: "dangerous command".to_string(),
        },
        tier: Tier::Tier1,
        severity: Severity::Critical,
        explanation: "test destructive operation".to_string(),
        redirect: Redirect {
            channel: Channel::Deny,
            reason_template: "denied".to_string(),
            rewrite_template: None,
        },
        destructive: true,
    }
}

fn content_pattern(id: &str, enabled: bool) -> GuardedPattern {
    GuardedPattern {
        id: id.to_string(),
        enabled,
        check: Check::ContentRegex {
            regex: "dangerous content".to_string(),
        },
        tier: Tier::Tier1,
        severity: Severity::High,
        explanation: "test dangerous content".to_string(),
        redirect: Redirect {
            channel: Channel::Deny,
            reason_template: "denied".to_string(),
            rewrite_template: None,
        },
        destructive: true,
    }
}

#[test]
fn omitted_enabled_flag_preserves_legacy_default() {
    let pattern: GuardedPattern = serde_json::from_str(
        r#"{
            "id": "legacy-rule",
            "type": "command_regex",
            "regex": "dangerous command",
            "tier": "tier1",
            "severity": "Critical",
            "explanation": "legacy rule",
            "redirect": {
                "channel": "deny",
                "reason_template": "denied",
                "rewrite_template": null
            }
        }"#,
    )
    .unwrap();

    assert!(pattern.enabled);
}

#[test]
fn disabled_command_and_content_rules_are_not_evaluated() {
    let mut command_engine = Engine::new();
    command_engine
        .load_pack(Pack {
            id: "command-pack".to_string(),
            tool_keywords: vec!["tool".to_string()],
            applies_to: Vec::new(),
            safe_patterns: Vec::new(),
            guarded_patterns: vec![command_pattern("disabled-command", false)],
        })
        .unwrap();
    assert_eq!(
        command_engine.evaluate_command(&CommandSource::Hook("tool dangerous command".to_string())),
        CheckResult::Allowed
    );

    let mut content_engine = Engine::new();
    content_engine
        .load_pack(Pack {
            id: "content-pack".to_string(),
            tool_keywords: Vec::new(),
            applies_to: vec!["*.yaml".to_string()],
            safe_patterns: Vec::new(),
            guarded_patterns: vec![content_pattern("disabled-content", false)],
        })
        .unwrap();
    assert_eq!(
        content_engine.evaluate_content(&ContentSource::Write {
            file_path: "deployment.yaml".to_string(),
            content: "dangerous content".to_string(),
        }),
        CheckResult::Allowed
    );
}

#[test]
fn fixed_suite_covers_enabled_rules_only() {
    let pack = Pack {
        id: "mixed-pack".to_string(),
        tool_keywords: vec!["tool".to_string()],
        applies_to: Vec::new(),
        safe_patterns: Vec::new(),
        guarded_patterns: vec![
            command_pattern("enabled-rule", true),
            command_pattern("disabled-rule", false),
        ],
    };

    let suite = generate_regression_suite(&pack).unwrap();
    assert_eq!(suite.cases.len(), 1);
    assert_eq!(suite.cases[0].pattern_id, "enabled-rule");
    verify_regression_suite(&pack, &suite).unwrap();
}

#[test]
fn disabling_a_rule_is_reported_as_a_release_regression() {
    let directory = tempdir().unwrap();
    let previous_path = directory.path().join("previous.json");
    let current_path = directory.path().join("current.json");
    let previous = Pack {
        id: "release-pack".to_string(),
        tool_keywords: vec!["tool".to_string()],
        applies_to: Vec::new(),
        safe_patterns: Vec::new(),
        guarded_patterns: vec![command_pattern("rule-to-disable", true)],
    };
    let current = Pack {
        guarded_patterns: vec![command_pattern("rule-to-disable", false)],
        ..previous.clone()
    };
    save_pack(&previous, &previous_path).unwrap();
    save_pack(&current, &current_path).unwrap();

    let diff = run_coverage_diff(previous_path.clone(), current_path.clone()).unwrap();
    assert_eq!(diff.disabled_guarded_patterns, vec!["rule-to-disable"]);
    assert_eq!(diff.disabled_guarded_pattern_changes[0].previous, "true");
    assert_eq!(diff.disabled_guarded_pattern_changes[0].current, "false");
    assert!(diff.has_regressions());

    let report = render_coverage_diff_report(&previous_path, &current_path, &diff, None);
    assert!(report.contains("## Disabled guarded_patterns"));
    assert!(report.contains("status: regressions_detected"));
    assert!(report.contains("justification: REQUIRED"));
}
