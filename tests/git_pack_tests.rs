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

#[test]
fn guarded_git_rules_fire_through_timeout_xargs_and_nice_wrappers() {
    let engine = load_git_engine();

    // Deny channel: credential fill must not be reachable through the
    // timeout/xargs/nice wrappers any more than through sudo.
    for command in [
        "timeout 10 git credential fill",
        "timeout -k 5 --signal=KILL 30 git credential fill",
        "xargs git credential fill",
        "xargs -0 -n 1 git credential fill",
        "nice -n 10 git credential fill",
    ] {
        assert_credential_fill_denied(
            engine.evaluate_command(&CommandSource::Hook(command.to_string())),
            command,
        );
    }

    // Rewrite channel: the force-push rule strips the flag from the unwrapped
    // command, so a wrapped push still retries as a safe plain push.
    let rewrite = engine.evaluate_command(&CommandSource::Hook(
        "timeout 60 git push --force origin main".to_string(),
    ));
    assert!(
        matches!(
            rewrite,
            CheckResult::Rewrite {
                ref pack_id,
                ref pattern_id,
                ..
            } if pack_id == "git" && pattern_id == "git-force-push"
        ),
        "expected git-force-push rewrite through a timeout wrapper, got {rewrite:?}"
    );

    // Allow channel: safe git verbs keep their safe-pattern coverage through
    // a wrapper -- unwrapping must not widen guarded matching.
    for command in [
        "timeout 30 git status",
        "xargs -0 git status",
        "nice -n 5 git log",
    ] {
        assert_eq!(
            engine.evaluate_command(&CommandSource::Hook(command.to_string())),
            CheckResult::Allowed,
            "safe git verb should stay allowed through a wrapper: {command}"
        );
    }
}

#[test]
fn guarded_git_rules_fire_through_shell_dash_c_payloads() {
    let engine = load_git_engine();

    // The `-c` operand of a shell is a live command line, so the guarded
    // rules reach through it. Deny channel:
    for command in [
        "bash -c 'git credential fill'",
        "sh -c 'git credential fill'",
        "sudo bash -c 'git credential fill'",
        "bash -lc 'git credential fill'",
    ] {
        assert_credential_fill_denied(
            engine.evaluate_command(&CommandSource::Hook(command.to_string())),
            command,
        );
    }

    // Rewrite channel: the force-push rule still strips the flag from the
    // command found inside the payload.
    let rewrite = engine.evaluate_command(&CommandSource::Hook(
        "bash -c 'git push --force origin main'".to_string(),
    ));
    assert!(
        matches!(
            rewrite,
            CheckResult::Rewrite {
                ref pack_id,
                ref pattern_id,
                ..
            } if pack_id == "git" && pattern_id == "git-force-push"
        ),
        "expected git-force-push rewrite through a shell payload, got {rewrite:?}"
    );

    // Allow channel: a shell name in argument position is data, and safe
    // git verbs stay allowed inside a payload. (The commit message quotes
    // force-push text rather than `git credential fill`: the credential
    // rule is deliberately unanchored and already fires on that text in a
    // message on an unexpanded tree, which is its own pre-existing story.)
    for command in [
        "bash -c 'git status'",
        "echo bash -c git push --force origin main",
        "git commit -m 'bash -c git push --force origin main' src/engine.rs",
    ] {
        assert_eq!(
            engine.evaluate_command(&CommandSource::Hook(command.to_string())),
            CheckResult::Allowed,
            "safe shape should stay allowed around a shell payload: {command}"
        );
    }
}
