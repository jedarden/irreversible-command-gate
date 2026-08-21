use icg::engine::{CheckResult, CommandSource, Engine};
use icg::rule_pack::{load_pack, Channel, Check, Severity, Tier};

fn load_openbao_engine() -> Engine {
    let mut engine = Engine::new();
    engine
        .load_pack(load_pack("packs/openbao.json").expect("OpenBao pack should load"))
        .expect("OpenBao pack should validate");
    engine
}

fn assert_denied(result: CheckResult, command: &str) {
    assert!(
        matches!(
            result,
            CheckResult::Denied {
                ref pack_id,
                ref pattern_id,
                ..
            } if pack_id == "openbao" && pattern_id == "openbao-destructive-verb"
        ),
        "expected {command:?} to be denied by the OpenBao destructive-verb rule, got {result:?}"
    );
}

#[test]
fn destructive_verbs_are_command_regex_deny_only_rules() {
    let pack = load_pack("packs/openbao.json").expect("OpenBao pack should load");

    assert_eq!(pack.tool_keywords, ["bao", "vault"]);
    let rule = pack
        .guarded_patterns
        .iter()
        .find(|pattern| pattern.id == "openbao-destructive-verb")
        .expect("destructive-verb rule should be present");

    assert_eq!(rule.tier, Tier::Tier1);
    assert_eq!(rule.severity, Severity::Critical);
    assert!(rule.destructive);
    assert_eq!(rule.redirect.channel, Channel::Deny);
    assert!(rule.redirect.rewrite_template.is_none());
    assert!(matches!(rule.check, Check::CommandRegex { .. }));
}

#[test]
fn destructive_verbs_are_denied_for_hook_commands_and_wrapper_argv() {
    let engine = load_openbao_engine();
    let commands = [
        "vault kv destroy secret/app",
        "bao kv destroy secret/app",
        "vault secrets disable kv/",
        "bao policy delete app-policy",
        "vault token revoke hvs.example",
        "bao lease revoke database/creds/app",
    ];

    for command in commands {
        assert_denied(
            engine.evaluate_command(&CommandSource::Hook(command.to_string())),
            command,
        );

        let argv = command
            .split_whitespace()
            .map(str::to_string)
            .collect::<Vec<_>>();
        assert_denied(
            engine.evaluate_command(&engine.read_from_argv(argv)),
            command,
        );
    }
}

#[test]
fn self_token_revoke_and_reversible_operations_remain_allowed() {
    let engine = load_openbao_engine();

    for command in [
        "vault token revoke -self",
        "bao kv delete secret/app",
        "vault policy read app-policy",
        "bao lease lookup database/creds/app",
    ] {
        assert_eq!(
            engine.evaluate_command(&CommandSource::Hook(command.to_string())),
            CheckResult::Allowed,
            "safe OpenBao command should remain allowed: {command}"
        );
    }
}
