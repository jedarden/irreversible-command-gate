use icg::engine::{CheckResult, CommandSource, ContentSource, Engine};
use icg::rule_pack::{load_pack, Channel, Check, Severity, Tier};
use std::fs;

fn load_image_tag_engine() -> Engine {
    let mut engine = Engine::new();
    engine
        .load_pack(load_pack("packs/image-tag.json").expect("image-tag pack loads"))
        .expect("image-tag pack validates");
    engine
}

#[test]
fn manifest_declares_yaml_content_mode_and_deny_rule() {
    let pack = load_pack("packs/image-tag.json").expect("image-tag pack loads");

    assert_eq!(pack.id, "image-tag");
    assert!(pack.tool_keywords.is_empty());
    assert_eq!(pack.applies_to, vec!["*.yaml", "*.yml"]);
    assert!(pack.safe_patterns.is_empty());
    assert_eq!(pack.guarded_patterns.len(), 2);

    for (rule, id) in pack
        .guarded_patterns
        .iter()
        .zip(["image-tag-latest", "image-tag-bare-sha"])
    {
        assert_eq!(rule.id, id);
        assert!(rule.enabled);
        assert_eq!(rule.tier, Tier::Tier1);
        assert_eq!(rule.severity, Severity::High);
        assert!(rule.destructive);
        assert_eq!(rule.redirect.channel, Channel::Deny);
        assert!(matches!(rule.check, Check::ContentRegex { .. }));
    }
}

#[test]
fn denies_latest_image_tags_in_yaml_and_yml_content() {
    let engine = load_image_tag_engine();

    for (file_path, content) in [
        (
            "deploy/app.yaml",
            "containers:\n  - image: ronaldraygun/myapp:latest\n",
        ),
        ("deploy/app.yml", "image: ronaldraygun/myapp:latest\n"),
    ] {
        let result = engine.evaluate_content(&ContentSource::Write {
            file_path: file_path.to_string(),
            content: content.to_string(),
        });

        assert!(
            matches!(
                result,
                CheckResult::Denied {
                    ref pack_id,
                    ref pattern_id,
                    ..
                } if pack_id == "image-tag" && pattern_id == "image-tag-latest"
            ),
            "expected :latest in {file_path} to be denied, got {result:?}"
        );
    }
}

#[test]
fn denies_bare_sha_image_tags_in_yaml_and_yml_content() {
    let engine = load_image_tag_engine();

    for (file_path, content) in [
        (
            "deploy/app.yaml",
            "containers:\n  - image: ronaldraygun/myapp:0123456789abcdef0123456789abcdef01234567\n",
        ),
        ("deploy/app.yml", "image: ronaldraygun/myapp:abcdef12\n"),
    ] {
        let result = engine.evaluate_content(&ContentSource::Write {
            file_path: file_path.to_string(),
            content: content.to_string(),
        });

        assert!(
            matches!(
                result,
                CheckResult::Denied {
                    ref pack_id,
                    ref pattern_id,
                    ref reason,
                } if pack_id == "image-tag"
                    && pattern_id == "image-tag-bare-sha"
                    && reason.contains("containers/<name>/VERSION")
            ),
            "expected bare SHA in {file_path} to be denied, got {result:?}"
        );
    }
}

#[test]
fn embeds_the_container_version_in_unpinned_image_denials() {
    let root = tempfile::tempdir().expect("temporary repository should exist");
    fs::create_dir_all(root.path().join("containers/myapp")).expect("container directory");
    fs::write(root.path().join("containers/myapp/VERSION"), "1.2.3\n")
        .expect("version file");

    let engine = load_image_tag_engine();
    let file_path = root.path().join("deploy/app.yaml");

    for content in [
        "image: ronaldraygun/myapp:latest\n",
        "image: ronaldraygun/myapp:abcdef12\n",
    ] {
        let result = engine.evaluate_content(&ContentSource::Write {
            file_path: file_path.to_string_lossy().into_owned(),
            content: content.to_string(),
        });

        let CheckResult::Denied { reason, .. } = result else {
            panic!("expected unpinned image to be denied");
        };
        assert!(reason.contains("1.2.3"), "denial should include the version: {reason}");
        assert!(!reason.contains("{derived_value}"), "placeholder must be rendered: {reason}");
        assert!(!reason.contains("containers/<name>/VERSION"), "path pointer must be replaced: {reason}");
    }
}

#[test]
fn allows_pinned_images_non_yaml_content_and_commands() {
    let engine = load_image_tag_engine();

    assert_eq!(
        engine.evaluate_content(&ContentSource::Write {
            file_path: "deploy/app.yaml".to_string(),
            content: "image: ronaldraygun/myapp:v1.2.3\n".to_string(),
        }),
        CheckResult::Allowed
    );

    assert_eq!(
        engine.evaluate_content(&ContentSource::Write {
            file_path: "deploy/app.yaml".to_string(),
            content: "image: ronaldraygun/myapp@sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef\n".to_string(),
        }),
        CheckResult::Allowed
    );

    assert_eq!(
        engine.evaluate_content(&ContentSource::Write {
            file_path: "deploy/app.yaml".to_string(),
            content: "image: otherorg/myapp:0123456789abcdef0123456789abcdef01234567\n".to_string(),
        }),
        CheckResult::Allowed
    );

    assert_eq!(
        engine.evaluate_content(&ContentSource::Write {
            file_path: "docs/example.md".to_string(),
            content: "image: ronaldraygun/myapp:latest\n".to_string(),
        }),
        CheckResult::Allowed
    );

    assert_eq!(
        engine.evaluate_command(&CommandSource::Hook(
            "echo 'image: ronaldraygun/myapp:latest' > deploy/app.yaml".to_string(),
        )),
        CheckResult::Allowed
    );
}
