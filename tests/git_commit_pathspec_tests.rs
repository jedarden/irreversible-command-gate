//! Regression tests for git commits that do not name an explicit pathspec.

use icg::engine::{CheckResult, CommandSource, Engine};
use icg::rule_pack::{load_pack, Channel, Check, Severity, Tier};

fn load_git_engine() -> Engine {
    let mut engine = Engine::new();
    engine
        .load_pack(load_pack("packs/git.json").expect("git pack should load"))
        .expect("git pack should validate");
    engine
}

fn assert_denied(result: CheckResult, pattern_id: &str, command: &str) {
    assert!(
        matches!(
            result,
            CheckResult::Denied {
                ref pack_id,
                pattern_id: ref actual_pattern_id,
                ..
            } if pack_id == "git" && actual_pattern_id == pattern_id
        ),
        "expected {command:?} to be denied by {pattern_id}, got {result:?}"
    );
}

#[test]
fn manifest_declares_tier_one_deny_only_commit_rules() {
    let pack = load_pack("packs/git.json").expect("git pack should load");

    let pattern = pack
        .guarded_patterns
        .iter()
        .find(|pattern| pattern.id == "git-commit-without-pathspec")
        .expect("missing git-commit-without-pathspec");
    assert!(pattern.enabled);
    assert_eq!(pattern.tier, Tier::Tier1);
    assert_eq!(pattern.severity, Severity::High);
    assert!(pattern.destructive);
    assert_eq!(pattern.redirect.channel, Channel::Deny);
    assert!(pattern.redirect.rewrite_template.is_none());
    assert!(matches!(pattern.check, Check::CommandRegex { .. }));
}

#[test]
fn commit_without_pathspec_is_denied_by_hook_and_wrapper_front_ends() {
    let engine = load_git_engine();

    for (command, pattern_id) in [
        ("git commit -a", "git-commit-without-pathspec"),
        (
            "git commit --all -m \"everything\"",
            "git-commit-without-pathspec",
        ),
        ("git commit -m \"fix bug\"", "git-commit-without-pathspec"),
        (
            "git commit --message \"fix bug\" --no-verify",
            "git-commit-without-pathspec",
        ),
    ] {
        assert_denied(
            engine.evaluate_command(&CommandSource::Hook(command.to_string())),
            pattern_id,
            command,
        );

        let argv = icg::engine::Engine::new().read_from_argv(
            match command {
                "git commit -a" => vec!["git", "commit", "-a"],
                "git commit --all -m \"everything\"" => {
                    vec!["git", "commit", "--all", "-m", "everything"]
                }
                "git commit -m \"fix bug\"" => {
                    vec!["git", "commit", "-m", "fix bug"]
                }
                _ => vec!["git", "commit", "--message", "fix bug", "--no-verify"],
            }
            .into_iter()
            .map(str::to_string)
            .collect(),
        );
        assert_denied(engine.evaluate_command(&argv), pattern_id, command);
    }
}

#[test]
fn explicit_pathspecs_and_unrelated_git_commands_remain_allowed() {
    let engine = load_git_engine();

    for command in [
        "git commit -m \"fix bug\" src/main.rs",
        "git commit --message \"fix bug\" -- src/main.rs",
        "git commit src/main.rs -m \"fix bug\"",
        "git commit -m \"mention --all in the message\" src/main.rs",
        "git commit --dry-run",
        "git status",
        "git push origin main",
    ] {
        assert_eq!(
            engine.evaluate_command(&CommandSource::Hook(command.to_string())),
            CheckResult::Allowed,
            "command should remain allowed: {command}"
        );
    }
}

#[test]
fn denial_reason_gives_the_explicit_commit_form() {
    let engine = load_git_engine();
    let result = engine.evaluate_command(&CommandSource::Hook(
        "git commit -m \"fix bug\"".to_string(),
    ));

    match result {
        CheckResult::Denied { reason, .. } => {
            assert!(reason.contains("same file paths to git commit"));
            assert!(reason.contains("git commit <paths> -m"));
        }
        other => panic!("expected denial, got {other:?}"),
    }
}
