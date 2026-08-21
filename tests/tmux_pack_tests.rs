use icg::engine::{CheckResult, CommandSource, Engine};
use icg::rule_pack::{load_pack, Channel, Check, Severity, Tier};

fn load_tmux_engine() -> Engine {
    let mut engine = Engine::new();
    engine
        .load_pack(load_pack("packs/tmux.json").expect("tmux pack should load"))
        .expect("tmux pack should validate");
    engine
}

fn assert_denied(engine: &Engine, command: &str) {
    let result = engine.evaluate_command(&CommandSource::Hook(command.to_string()));
    assert!(
        matches!(
            result,
            CheckResult::Denied {
                ref pack_id,
                ref pattern_id,
                ..
            } if pack_id == "tmux" && pattern_id == "bare-nato-session"
        ),
        "expected {command:?} to be denied by the bare NATO session rule, got {result:?}"
    );
}

#[test]
fn manifest_declares_a_tier_one_command_regex_deny_rule() {
    let pack = load_pack("packs/tmux.json").expect("tmux pack should load");

    assert_eq!(pack.id, "tmux");
    assert_eq!(pack.tool_keywords, ["tmux"]);
    assert!(pack.applies_to.is_empty());
    assert!(pack.safe_patterns.is_empty());
    assert_eq!(pack.guarded_patterns.len(), 1);

    let rule = &pack.guarded_patterns[0];
    assert_eq!(rule.id, "bare-nato-session");
    assert!(rule.enabled);
    assert_eq!(rule.tier, Tier::Tier1);
    assert_eq!(rule.severity, Severity::Medium);
    assert!(!rule.destructive);
    assert_eq!(rule.redirect.channel, Channel::Deny);
    assert!(rule.redirect.rewrite_template.is_none());
    assert!(matches!(rule.check, Check::CommandRegex { .. }));
}

#[test]
fn bare_nato_targets_are_denied_by_hook_and_wrapper_front_ends() {
    let engine = load_tmux_engine();
    let nato_names = [
        "alpha", "bravo", "charlie", "delta", "echo", "foxtrot", "golf", "hotel", "india",
        "juliet", "kilo", "lima", "mike", "november", "oscar", "papa", "quebec", "romeo", "sierra",
        "tango", "uniform", "victor", "whiskey", "xray", "yankee", "zulu",
    ];

    for name in nato_names {
        let command = format!("tmux send-keys -t {name} C-c");
        assert_denied(&engine, &command);

        let argv = command
            .split_whitespace()
            .map(str::to_string)
            .collect::<Vec<_>>();
        let result = engine.evaluate_command(&engine.read_from_argv(argv));
        assert!(
            matches!(
                result,
                CheckResult::Denied {
                    ref pack_id,
                    ref pattern_id,
                    ..
                } if pack_id == "tmux" && pattern_id == "bare-nato-session"
            ),
            "expected wrapper argv for {command:?} to be denied, got {result:?}"
        );
    }
}

#[test]
fn target_syntax_and_session_boundaries_are_checked_without_blocking_workers() {
    let engine = load_tmux_engine();

    for command in [
        "tmux attach-session --target=alpha",
        "tmux capture-pane -t bravo:0",
        "tmux list-panes -t charlie.1",
        "tmux switch-client -t=delta",
        "tmux send-keys -t 'echo' C-c",
    ] {
        assert_denied(&engine, command);
    }

    for command in [
        "tmux send-keys -t alpha-worker C-c",
        "tmux send-keys -t needle-claude-codex-alpha C-c",
        "tmux send-keys -t @1234 C-c",
        "tmux list-sessions",
        "tmux display-message alpha",
    ] {
        assert_eq!(
            engine.evaluate_command(&CommandSource::Hook(command.to_string())),
            CheckResult::Allowed,
            "worker or non-target tmux command should remain allowed: {command}"
        );
    }
}
