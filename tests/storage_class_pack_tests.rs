use icg::engine::{CheckResult, CommandSource, ContentSource, Engine};
use icg::rule_pack::{load_pack, Channel, Check, Severity, Tier};

fn load_storage_class_engine() -> Engine {
    let mut engine = Engine::new();
    engine
        .load_pack(load_pack("packs/storage-class.json").expect("storage-class pack loads"))
        .expect("storage-class pack validates");
    engine
}

#[test]
fn manifest_declares_yaml_content_mode_and_deny_rule() {
    let pack = load_pack("packs/storage-class.json").expect("storage-class pack loads");

    assert_eq!(pack.id, "storage-class");
    assert!(pack.tool_keywords.is_empty());
    assert_eq!(pack.applies_to, vec!["*.yaml", "*.yml"]);
    assert!(pack.safe_patterns.is_empty());
    assert_eq!(pack.guarded_patterns.len(), 1);

    let rule = &pack.guarded_patterns[0];
    assert_eq!(rule.id, "storage-class-ssd");
    assert!(rule.enabled);
    assert_eq!(rule.tier, Tier::Tier1);
    assert_eq!(rule.severity, Severity::High);
    assert!(rule.destructive);
    assert_eq!(rule.redirect.channel, Channel::Deny);
    assert!(matches!(rule.check, Check::ContentRegex { .. }));
}

#[test]
fn denies_ssd_and_ssd_large_in_yaml_and_yml_content() {
    let engine = load_storage_class_engine();

    for (file_path, storage_class) in [
        ("deploy/storage.yaml", "ssd"),
        ("deploy/storage.yml", "ssd-large"),
    ] {
        let result = engine.evaluate_content(&ContentSource::Write {
            file_path: file_path.to_string(),
            content: format!("  storageClassName: {storage_class}\n"),
        });

        assert!(
            matches!(
                result,
                CheckResult::Denied {
                    ref pack_id,
                    ref pattern_id,
                    ..
                } if pack_id == "storage-class" && pattern_id == "storage-class-ssd"
            ),
            "expected {storage_class} in {file_path} to be denied, got {result:?}"
        );
    }
}

#[test]
fn allows_other_storage_classes_and_non_yaml_content() {
    let engine = load_storage_class_engine();

    for storage_class in ["sata", "sata-large", "standard"] {
        assert_eq!(
            engine.evaluate_content(&ContentSource::Write {
                file_path: "deploy/storage.yaml".to_string(),
                content: format!("storageClassName: {storage_class}\n"),
            }),
            CheckResult::Allowed,
            "{storage_class} should be allowed"
        );
    }

    assert_eq!(
        engine.evaluate_content(&ContentSource::Write {
            file_path: "deploy/storage.json".to_string(),
            content: "{\"storageClassName\":\"ssd\"}".to_string(),
        }),
        CheckResult::Allowed
    );
}

#[test]
fn storage_class_pack_is_not_used_for_command_mode() {
    let engine = load_storage_class_engine();

    assert_eq!(
        engine.evaluate_command(&CommandSource::Hook(
            "echo 'storageClassName: ssd' > deploy/storage.yaml".to_string(),
        )),
        CheckResult::Allowed
    );
}
