use icg::engine::{CheckResult, CommandSource, Engine};
use icg::rule_pack::{load_pack, Channel, Check, Severity, Tier};

fn load_misc_engine() -> Engine {
    let mut engine = Engine::new();
    engine
        .load_pack(load_pack("packs/misc.json").expect("misc pack should load"))
        .expect("misc pack should validate");
    engine
}

fn assert_needle_cleanup_denied(result: CheckResult, input: &str) {
    assert!(
        matches!(
            result,
            CheckResult::Denied {
                ref pack_id,
                ref pattern_id,
                ..
            } if pack_id == "misc" && pattern_id == "needle-cleanup"
        ),
        "expected {input:?} to be denied by the needle cleanup rule"
    );
}

#[test]
fn manifest_declares_a_tier_one_command_regex_deny_rule() {
    let pack = load_pack("packs/misc.json").expect("misc pack should load");

    assert_eq!(pack.id, "misc");
    assert_eq!(pack.tool_keywords, ["needle"]);
    assert!(pack.applies_to.is_empty());
    assert!(pack.safe_patterns.is_empty());
    assert_eq!(pack.guarded_patterns.len(), 1);

    let rule = &pack.guarded_patterns[0];
    assert_eq!(rule.id, "needle-cleanup");
    assert!(rule.enabled);
    assert_eq!(rule.tier, Tier::Tier1);
    assert_eq!(rule.severity, Severity::Critical);
    assert!(rule.destructive);
    assert_eq!(rule.redirect.channel, Channel::Deny);
    assert!(rule.redirect.rewrite_template.is_none());
    assert!(matches!(rule.check, Check::CommandRegex { .. }));
}

#[test]
fn needle_cleanup_is_denied_by_hook_and_wrapper_front_ends() {
    let engine = load_misc_engine();

    for command in [
        "needle cleanup",
        "needle cleanup --all",
        "sudo needle cleanup",
    ] {
        assert_needle_cleanup_denied(
            engine.evaluate_command(&CommandSource::Hook(command.to_string())),
            command,
        );
    }

    let argv = ["needle", "cleanup"]
        .into_iter()
        .map(str::to_string)
        .collect();
    assert_needle_cleanup_denied(
        engine.evaluate_command(&engine.read_from_argv(argv)),
        "needle cleanup (wrapper argv)",
    );
}

#[test]
fn unrelated_needle_commands_remain_allowed() {
    let engine = load_misc_engine();

    for command in [
        "needle status",
        "needle worker list",
        "needle cleanup-worktree",
        "echo needle cleanup",
    ] {
        assert_eq!(
            engine.evaluate_command(&CommandSource::Hook(command.to_string())),
            CheckResult::Allowed,
            "unrelated command should remain allowed: {command}"
        );
    }
}
