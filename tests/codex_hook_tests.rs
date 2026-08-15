use icg::engine::{
    normalize_apply_patch, ContentSource, Engine, InputSource, PreToolUseError, PreToolUseInput,
    ToolInput,
};

#[test]
fn parses_codex_snake_case_bash_input() {
    let input = Engine::parse_and_validate_pre_tool_use(
        r#"{
            "hook_event_name": "PreToolUse",
            "tool_name": "Bash",
            "tool_input": {"command": "git status"}
        }"#,
    )
    .expect("Codex hook input should parse");

    assert_eq!(input.tool_name, "Bash");
    assert_eq!(input.tool_input.command.as_deref(), Some("git status"));
}

#[test]
fn normalizes_codex_apply_patch_into_content_input() {
    let patch = "*** Begin Patch\n*** Update File: deploy/app.yaml\n@@\n-old: safe\n+storageClassName: ssd\n+image: nginx:latest\n*** End Patch\n";
    let input = PreToolUseInput {
        tool_name: "apply_patch".to_string(),
        tool_input: ToolInput {
            command: Some(patch.to_string()),
            file_path: None,
            content: None,
            old_string: None,
            new_string: None,
            encoding: None,
            mime_type: None,
        },
        id: None,
        timestamp: None,
        session_id: None,
    };

    let source = Engine::input_source_from_pre_tool_use(input)
        .expect("apply_patch should normalize")
        .expect("apply_patch should produce content input");

    match source {
        InputSource::Content(ContentSource::Write { file_path, content }) => {
            assert_eq!(file_path, "deploy/app.yaml");
            assert!(content.contains("storageClassName: ssd"));
            assert!(content.contains("image: nginx:latest"));
            assert!(!content.contains("old: safe"));
        }
        other => panic!("expected one normalized content source, got {other:?}"),
    }
}

#[test]
fn normalizes_every_file_in_a_multi_file_codex_patch() {
    let patch = "*** Begin Patch\n*** Add File: a.yaml\n+storageClassName: ssd\n*** Add File: b.yaml\n+image: nginx:latest\n*** End Patch";
    let sources = normalize_apply_patch(patch).expect("multi-file patch should normalize");

    assert_eq!(sources.len(), 2);
    assert_eq!(sources[0].file_path(), "a.yaml");
    assert_eq!(sources[0].new_content(), "storageClassName: ssd\n");
    assert_eq!(sources[1].file_path(), "b.yaml");
    assert_eq!(sources[1].new_content(), "image: nginx:latest\n");
}

#[test]
fn apply_patch_input_uses_batch_for_multiple_files() {
    let input = PreToolUseInput {
        tool_name: "apply_patch".to_string(),
        tool_input: ToolInput {
            command: Some(
                "*** Begin Patch\n*** Add File: a.yaml\n+x\n*** Add File: b.yaml\n+y\n*** End Patch"
                    .to_string(),
            ),
            file_path: None,
            content: None,
            old_string: None,
            new_string: None,
            encoding: None,
            mime_type: None,
        },
        id: None,
        timestamp: None,
        session_id: None,
    };

    assert!(matches!(
        Engine::input_source_from_pre_tool_use(input).unwrap(),
        Some(InputSource::ContentBatch(sources)) if sources.len() == 2
    ));
}

#[test]
fn rejects_malformed_apply_patch_payloads() {
    let error = normalize_apply_patch("*** Begin Patch\n*** End Patch").unwrap_err();
    assert!(matches!(error, PreToolUseError::InvalidInput { tool, .. } if tool == "apply_patch"));
}
