use icg::engine::{CheckResult, CommandSource, Engine};
use icg::rule_pack::{load_pack, Channel, Check, Severity, Tier};

fn load_docker_engine() -> Engine {
    let mut engine = Engine::new();
    engine
        .load_pack(load_pack("packs/docker.json").expect("Docker pack should load"))
        .expect("Docker pack should validate");
    engine
}

fn assert_denied(engine: &Engine, command: &str, pattern_id: &str) {
    let result = engine.evaluate_command(&CommandSource::Hook(command.to_string()));
    assert!(
        matches!(
            result,
            CheckResult::Denied {
                ref pack_id,
                pattern_id: ref actual_pattern_id,
                ..
            } if pack_id == "docker" && actual_pattern_id == pattern_id
        ),
        "expected {command:?} to be denied by {pattern_id}, got {result:?}"
    );
}

#[test]
fn manifest_declares_three_critical_deny_rules() {
    let pack = load_pack("packs/docker.json").expect("Docker pack should load");

    assert_eq!(pack.id, "docker");
    assert_eq!(pack.tool_keywords, ["docker"]);
    assert!(pack.applies_to.is_empty());
    assert!(pack.safe_patterns.is_empty());
    assert_eq!(pack.guarded_patterns.len(), 3);

    for pattern in &pack.guarded_patterns {
        assert!(pattern.enabled);
        assert_eq!(pattern.tier, Tier::Tier1);
        assert_eq!(pattern.severity, Severity::Critical);
        assert!(pattern.destructive);
        assert_eq!(pattern.redirect.channel, Channel::Deny);
        assert!(pattern.redirect.rewrite_template.is_none());
        assert!(matches!(pattern.check, Check::CommandRegex { .. }));
    }
}

#[test]
fn denies_broad_prune_and_volume_removal() {
    let engine = load_docker_engine();

    for (command, pattern_id) in [
        ("docker system prune -a", "docker-system-prune-all"),
        ("docker system prune --all", "docker-system-prune-all"),
        (
            "sudo docker system prune --force --all",
            "docker-system-prune-all",
        ),
        ("docker volume rm build-cache", "docker-volume-rm"),
        ("docker volume remove old-data", "docker-volume-rm"),
    ] {
        assert_denied(&engine, command, pattern_id);
    }
}

#[test]
fn denies_forced_removal_of_tagged_or_in_use_images() {
    let engine = load_docker_engine();

    for (command, pattern_id) in [
        ("docker rmi -f example/app:old", "docker-image-rm-force"),
        (
            "docker rmi example/app:old --force",
            "docker-image-rm-force",
        ),
        (
            "docker image rm --force example/app:old",
            "docker-image-rm-force",
        ),
        (
            "docker image remove -f example/app:old",
            "docker-image-rm-force",
        ),
    ] {
        assert_denied(&engine, command, pattern_id);
    }
}

#[test]
fn wrapper_argv_is_guarded_and_safe_docker_commands_remain_allowed() {
    let engine = load_docker_engine();

    let argv = ["docker", "volume", "rm", "persistent-data"]
        .into_iter()
        .map(str::to_string)
        .collect();
    assert!(matches!(
        engine.evaluate_command(&engine.read_from_argv(argv)),
        CheckResult::Denied {
            ref pack_id,
            ref pattern_id,
            ..
        } if pack_id == "docker" && pattern_id == "docker-volume-rm"
    ));

    for command in [
        "docker system df",
        "docker system prune",
        "docker volume ls",
        "docker image ls",
        "docker image rm example/app:old",
        "docker rm -f app-container",
    ] {
        assert_eq!(
            engine.evaluate_command(&CommandSource::Hook(command.to_string())),
            CheckResult::Allowed,
            "safe Docker command should remain allowed: {command}"
        );
    }
}
