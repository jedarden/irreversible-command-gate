use icg::engine::{CheckResult, CommandSource, Engine};
use icg::rule_pack::{load_pack, Channel, Check, GuardedPattern, Pack, Redirect, Severity, Tier};

fn load_misc_engine() -> Engine {
    let mut engine = Engine::new();
    engine
        .load_pack(load_pack("packs/misc.json").expect("misc pack should load"))
        .expect("misc pack should validate");
    engine
}

fn assert_deprecated_denied(
    result: CheckResult,
    pack_id_expected: &str,
    pattern_id_expected: &str,
    command: &str,
) {
    assert!(
        matches!(
            result,
            CheckResult::Denied {
                ref pack_id,
                ref pattern_id,
                ..
            } if pack_id == pack_id_expected && pattern_id == pattern_id_expected
        ),
        "expected {command:?} to be denied by the deprecated bead CLI rule"
    );
}

#[test]
fn manifest_keeps_cli_policy_in_rule_data_without_flipping_the_cutover() {
    let pack = load_pack("packs/misc.json").expect("misc pack should load");
    let rule = pack
        .guarded_patterns
        .iter()
        .find(|pattern| pattern.id == "deprecated-bead-cli")
        .expect("deprecated bead CLI rule should be present");

    let Check::Predicate {
        predicate_name,
        data: Some(data),
    } = &rule.check
    else {
        panic!("deprecated bead CLI rule should be a data-bearing predicate");
    };
    assert_eq!(predicate_name, "deprecated_command");
    assert_eq!(data["currently_canonical"], "bf");
    assert_eq!(data["deprecated"], serde_json::json!(["br"]));
}

#[test]
fn canonical_cli_is_allowed_and_deprecated_cli_is_denied_on_both_front_ends() {
    let engine = load_misc_engine();

    assert_eq!(
        engine.evaluate_command(&CommandSource::Hook("bf list".to_string())),
        CheckResult::Allowed
    );
    assert_deprecated_denied(
        engine.evaluate_command(&CommandSource::Hook("sudo br list".to_string())),
        "misc",
        "deprecated-bead-cli",
        "sudo br list",
    );

    let argv = ["br", "list"].into_iter().map(str::to_string).collect();
    assert_deprecated_denied(
        engine.evaluate_command(&engine.read_from_argv(argv)),
        "misc",
        "deprecated-bead-cli",
        "br list (wrapper argv)",
    );
}

#[test]
fn deprecated_rule_matches_invocations_not_mentions_and_reads_arbitrary_names_from_data() {
    let mut engine = Engine::new();
    engine
        .load_pack(Pack {
            id: "data-driven-test".to_string(),
            // The predicate contributes its deprecated executable to dispatch.
            tool_keywords: vec!["other-tool".to_string()],
            applies_to: vec![],
            safe_patterns: vec![],
            guarded_patterns: vec![GuardedPattern {
                id: "deprecated-cli".to_string(),
                enabled: true,
                check: Check::Predicate {
                    predicate_name: "deprecated_command".to_string(),
                    data: Some(serde_json::json!({
                        "currently_canonical": "modern-bead",
                        "deprecated": ["legacy-bead"],
                    })),
                },
                tier: Tier::Tier1,
                severity: Severity::Medium,
                explanation: "The configured legacy CLI is deprecated".to_string(),
                redirect: Redirect {
                    channel: Channel::Deny,
                    reason_template: "Use the canonical CLI".to_string(),
                    rewrite_template: None,
                },
                destructive: false,
            }],
        })
        .expect("data-driven test pack should validate");

    assert_eq!(
        engine.evaluate_command(&CommandSource::Hook("modern-bead list".to_string())),
        CheckResult::Allowed
    );
    assert_deprecated_denied(
        engine.evaluate_command(&CommandSource::Hook("legacy-bead list".to_string())),
        "data-driven-test",
        "deprecated-cli",
        "legacy-bead list",
    );
    assert_eq!(
        engine.evaluate_command(&CommandSource::Hook("echo legacy-bead".to_string())),
        CheckResult::Allowed
    );
}
