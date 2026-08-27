use icg::engine::{CheckResult, CommandSource, Engine};
use icg::rule_pack::{load_pack, Channel, Check, Severity, Tier};

fn load_git_engine() -> Engine {
    let mut engine = Engine::new();
    engine
        .load_pack(load_pack("packs/git.json").expect("git pack should load"))
        .expect("git pack should validate");
    engine
}

fn assert_credential_fill_denied(result: CheckResult, input: &str) {
    assert!(
        matches!(
            result,
            CheckResult::Denied {
                ref pack_id,
                ref pattern_id,
                ..
            } if pack_id == "git" && pattern_id == "git-credential-fill-bare-stdout"
        ),
        "expected {input:?} to be denied by git-credential-fill-bare-stdout, got a different result"
    );
}

#[test]
fn manifest_declares_the_credential_fill_guard_rule() {
    let pack = load_pack("packs/git.json").expect("git pack should load");

    let rule = pack
        .guarded_patterns
        .iter()
        .find(|p| p.id == "git-credential-fill-bare-stdout")
        .expect("git-credential-fill-bare-stdout rule should exist");

    assert!(rule.enabled);
    assert_eq!(rule.tier, Tier::Tier1);
    assert_eq!(rule.severity, Severity::Critical);
    // Leaking a credential value is not itself destructive (nothing is
    // deleted or rewritten) -- matches the openbao pack's
    // openbao-inline-secret-literal precedent (Critical, destructive: false).
    assert!(!rule.destructive);
    assert_eq!(rule.redirect.channel, Channel::Deny);
    assert!(rule.redirect.rewrite_template.is_none());
    assert!(matches!(rule.check, Check::CommandRegex { .. }));
}

#[test]
fn bare_git_credential_fill_is_denied() {
    let engine = load_git_engine();

    for command in [
        "git credential fill",
        "git credential fill <<< $'protocol=https\\nhost=git.ardenone.com\\n'",
        "git status && git credential fill",
        "git credential fill | cat",
        // NOTE: "timeout 10 git credential fill ..." is NOT covered -- `timeout`
        // is absent from Engine::new()'s ignored_prefixes (unlike sudo/nohup/
        // time/command/exec), so pack dispatch never reaches "git" through it.
        // That's a separate, pre-existing engine gap (see engine.rs ~L802),
        // not something a pack-level regex can fix. Filed for the maintainers
        // rather than worked around here.
    ] {
        assert_credential_fill_denied(
            engine.evaluate_command(&CommandSource::Hook(command.to_string())),
            command,
        );
    }
}

#[test]
fn captured_or_redirected_credential_fill_remains_allowed() {
    let engine = load_git_engine();

    for command in [
        "TOKEN=$(git credential fill <<< $'protocol=https\\nhost=git.ardenone.com\\n' | sed -n 's/^password=//p')",
        "FORGEJO_TOKEN=$(git credential fill <<< $'protocol=https\\nhost=x\\n' | grep password | cut -d= -f2)",
        "git credential fill <<< $'protocol=https\\nhost=x\\n' > /tmp/creds.txt",
    ] {
        assert_eq!(
            engine.evaluate_command(&CommandSource::Hook(command.to_string())),
            CheckResult::Allowed,
            "captured/redirected credential fill should remain allowed: {command}"
        );
    }
}

#[test]
fn unrelated_git_credential_commands_remain_allowed() {
    let engine = load_git_engine();

    for command in [
        "git credential approve",
        "git credential reject",
        "git credential-cache exit",
    ] {
        assert_eq!(
            engine.evaluate_command(&CommandSource::Hook(command.to_string())),
            CheckResult::Allowed,
            "unrelated command should remain allowed: {command}"
        );
    }
}
