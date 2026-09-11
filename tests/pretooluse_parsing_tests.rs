//! Tests for PreToolUse JSON parsing and validation
//!
//! These tests verify the core parsing functionality that powers the hook front-end.

use icg::engine::{
    CommandSource, ContentSource, Engine, InputSource, PreToolUseError, PreToolUseInput, ToolInput,
};
use serde_json::{json, Value};
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

#[test]
fn test_parse_bash_tool_from_json() {
    let json = r#"{
        "toolName": "Bash",
        "toolInput": {
            "command": "vault kv get secret/foo"
        },
        "id": "test-123",
        "timestamp": "2026-08-15T10:00:00Z",
        "sessionId": "session-abc"
    }"#;

    let result = Engine::parse_and_validate_pre_tool_use(json);
    assert!(result.is_ok());

    let parsed = result.unwrap();
    assert_eq!(parsed.tool_name, "Bash");
    assert_eq!(
        parsed.tool_input.command,
        Some("vault kv get secret/foo".to_string())
    );
    assert_eq!(parsed.id, Some("test-123".to_string()));
    assert_eq!(parsed.timestamp, Some("2026-08-15T10:00:00Z".to_string()));
    assert_eq!(parsed.session_id, Some("session-abc".to_string()));
}

#[test]
fn test_parse_write_tool_from_json() {
    let json = r#"{
        "toolName": "Write",
        "toolInput": {
            "filePath": "/path/to/config.yaml",
            "content": "storageClassName: sata"
        }
    }"#;

    let result = Engine::parse_and_validate_pre_tool_use(json);
    assert!(result.is_ok());

    let parsed = result.unwrap();
    assert_eq!(parsed.tool_name, "Write");
    assert_eq!(
        parsed.tool_input.file_path,
        Some("/path/to/config.yaml".to_string())
    );
    assert_eq!(
        parsed.tool_input.content,
        Some("storageClassName: sata".to_string())
    );
}

#[test]
fn test_parse_edit_tool_from_json() {
    let json = r#"{
        "toolName": "Edit",
        "toolInput": {
            "filePath": "/path/to/deployment.yaml",
            "oldString": "image: nginx:1.19",
            "newString": "image: nginx:latest"
        }
    }"#;

    let result = Engine::parse_and_validate_pre_tool_use(json);
    assert!(result.is_ok());

    let parsed = result.unwrap();
    assert_eq!(parsed.tool_name, "Edit");
    assert_eq!(
        parsed.tool_input.file_path,
        Some("/path/to/deployment.yaml".to_string())
    );
    assert_eq!(
        parsed.tool_input.old_string,
        Some("image: nginx:1.19".to_string())
    );
    assert_eq!(
        parsed.tool_input.new_string,
        Some("image: nginx:latest".to_string())
    );
}

#[test]
fn test_parse_with_optional_fields() {
    let json = r#"{
        "toolName": "Bash",
        "toolInput": {
            "command": "kubectl get pods"
        }
    }"#;

    let result = Engine::parse_and_validate_pre_tool_use(json);
    assert!(result.is_ok());

    let parsed = result.unwrap();
    assert_eq!(parsed.tool_name, "Bash");
    assert!(parsed.id.is_none());
    assert!(parsed.timestamp.is_none());
    assert!(parsed.session_id.is_none());
}

#[test]
fn test_parse_invalid_json() {
    let invalid_json = r#"{
        "toolName": "Bash",
        "toolInput": {
            "command": "git status"
    }"#; // Missing closing brace

    let result = Engine::parse_and_validate_pre_tool_use(invalid_json);
    assert!(result.is_err());

    let error = result.unwrap_err();
    let error_msg = format!("{}", error);
    assert!(error_msg.contains("Invalid JSON") || error_msg.contains("JSON parse error"));
}

#[test]
fn test_parse_missing_tool_name() {
    let json = r#"{
        "toolInput": {
            "command": "git status"
        }
    }"#;

    let result = Engine::parse_and_validate_pre_tool_use(json);
    assert!(result.is_err());

    let error = result.unwrap_err();
    let error_msg = format!("{}", error);
    assert!(error_msg.contains("Missing required field") || error_msg.contains("toolName"));
}

#[test]
fn test_parse_empty_tool_name() {
    let json = r#"{
        "toolName": "",
        "toolInput": {
            "command": "git status"
        }
    }"#;

    let result = Engine::parse_and_validate_pre_tool_use(json);
    assert!(result.is_err());

    let error = result.unwrap_err();
    let error_msg = format!("{}", error);
    assert!(error_msg.contains("Missing required field") || error_msg.contains("toolName"));
}

#[test]
fn test_validate_bash_missing_command() {
    let json = r#"{
        "toolName": "Bash",
        "toolInput": {}
    }"#;

    let result = Engine::parse_and_validate_pre_tool_use(json);
    assert!(result.is_err());

    let error = result.unwrap_err();
    let error_msg = format!("{}", error);
    assert!(error_msg.contains("missing 'command'") || error_msg.contains("Invalid input"));
}

#[test]
fn test_validate_bash_empty_command() {
    let json = r#"{
        "toolName": "Bash",
        "toolInput": {
            "command": "   "
        }
    }"#;

    let result = Engine::parse_and_validate_pre_tool_use(json);
    assert!(result.is_err());

    let error = result.unwrap_err();
    let error_msg = format!("{}", error);
    assert!(error_msg.contains("empty") || error_msg.contains("Invalid input"));
}

#[test]
fn test_validate_write_missing_file_path() {
    let json = r#"{
        "toolName": "Write",
        "toolInput": {
            "content": "some content"
        }
    }"#;

    let result = Engine::parse_and_validate_pre_tool_use(json);
    assert!(result.is_err());

    let error = result.unwrap_err();
    let error_msg = format!("{}", error);
    assert!(error_msg.contains("missing 'filePath'") || error_msg.contains("Invalid input"));
}

#[test]
fn test_validate_write_missing_content() {
    let json = r#"{
        "toolName": "Write",
        "toolInput": {
            "filePath": "/path/to/file.txt"
        }
    }"#;

    let result = Engine::parse_and_validate_pre_tool_use(json);
    assert!(result.is_err());

    let error = result.unwrap_err();
    let error_msg = format!("{}", error);
    assert!(error_msg.contains("missing 'content'") || error_msg.contains("Invalid input"));
}

#[test]
fn test_validate_edit_missing_old_string() {
    let json = r#"{
        "toolName": "Edit",
        "toolInput": {
            "filePath": "/path/to/file.txt",
            "newString": "new content"
        }
    }"#;

    let result = Engine::parse_and_validate_pre_tool_use(json);
    assert!(result.is_err());

    let error = result.unwrap_err();
    let error_msg = format!("{}", error);
    assert!(error_msg.contains("oldString") || error_msg.contains("Invalid input"));
}

#[test]
fn test_validate_edit_missing_new_string() {
    let json = r#"{
        "toolName": "Edit",
        "toolInput": {
            "filePath": "/path/to/file.txt",
            "oldString": "old content"
        }
    }"#;

    let result = Engine::parse_and_validate_pre_tool_use(json);
    assert!(result.is_err());

    let error = result.unwrap_err();
    let error_msg = format!("{}", error);
    assert!(error_msg.contains("newString") || error_msg.contains("Invalid input"));
}

#[test]
fn test_unknown_tool_allowed() {
    // Unknown tools should be allowed (fail-open)
    let json = r#"{
        "toolName": "UnknownTool",
        "toolInput": {
            "someField": "someValue"
        }
    }"#;

    let result = Engine::parse_and_validate_pre_tool_use(json);
    // Parsing should succeed for unknown tools (fail-open)
    assert!(result.is_ok());

    let parsed = result.unwrap();
    assert_eq!(parsed.tool_name, "UnknownTool");
}

#[test]
fn test_convert_bash_to_input_source() {
    let input = PreToolUseInput {
        tool_name: "Bash".to_string(),
        tool_input: ToolInput {
            command: Some("vault kv destroy secret/foo".to_string()),
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

    let result = Engine::input_source_from_pre_tool_use(input);
    assert!(result.is_ok());

    let input_source = result.unwrap();
    assert!(input_source.is_some());

    match input_source.as_ref().unwrap() {
        InputSource::Command(CommandSource::Hook(cmd)) => {
            assert_eq!(cmd, "vault kv destroy secret/foo");
        }
        _ => panic!("Expected Command source, got {:?}", input_source),
    }
}

#[test]
fn test_convert_write_to_input_source() {
    let input = PreToolUseInput {
        tool_name: "Write".to_string(),
        tool_input: ToolInput {
            command: None,
            file_path: Some("/path/to/config.yaml".to_string()),
            content: Some("storageClassName: sata".to_string()),
            old_string: None,
            new_string: None,
            encoding: None,
            mime_type: None,
        },
        id: None,
        timestamp: None,
        session_id: None,
    };

    let result = Engine::input_source_from_pre_tool_use(input);
    assert!(result.is_ok());

    let input_source = result.unwrap();
    assert!(input_source.is_some());

    match input_source.as_ref().unwrap() {
        InputSource::Content(ContentSource::Write { file_path, content }) => {
            assert_eq!(file_path, "/path/to/config.yaml");
            assert_eq!(content, "storageClassName: sata");
        }
        _ => panic!("Expected Content::Write source, got {:?}", input_source),
    }
}

#[test]
fn test_convert_edit_to_input_source() {
    let input = PreToolUseInput {
        tool_name: "Edit".to_string(),
        tool_input: ToolInput {
            command: None,
            file_path: Some("/path/to/deployment.yaml".to_string()),
            content: None,
            old_string: Some("image: nginx:1.19".to_string()),
            new_string: Some("image: nginx:latest".to_string()),
            encoding: None,
            mime_type: None,
        },
        id: None,
        timestamp: None,
        session_id: None,
    };

    let result = Engine::input_source_from_pre_tool_use(input);
    assert!(result.is_ok());

    let input_source = result.unwrap();
    assert!(input_source.is_some());

    match input_source.as_ref().unwrap() {
        InputSource::Content(ContentSource::Edit {
            file_path,
            old_content,
            new_content,
        }) => {
            assert_eq!(file_path, "/path/to/deployment.yaml");
            assert_eq!(old_content, "image: nginx:1.19");
            assert_eq!(new_content, "image: nginx:latest");
        }
        _ => panic!("Expected Content::Edit source, got {:?}", input_source),
    }
}

#[test]
fn test_convert_unknown_tool_returns_none() {
    let input = PreToolUseInput {
        tool_name: "UnknownTool".to_string(),
        tool_input: ToolInput {
            command: None,
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

    let result = Engine::input_source_from_pre_tool_use(input);
    assert!(result.is_ok());

    let input_source = result.unwrap();
    // Unknown tools should return None (fail-open)
    assert!(input_source.is_none());
}

#[test]
fn test_parse_with_encoding_and_mime_type() {
    let json = r#"{
        "toolName": "Write",
        "toolInput": {
            "filePath": "/path/to/image.png",
            "content": "base64encodedcontent",
            "encoding": "base64",
            "mimeType": "image/png"
        }
    }"#;

    let result = Engine::parse_and_validate_pre_tool_use(json);
    assert!(result.is_ok());

    let parsed = result.unwrap();
    assert_eq!(parsed.tool_input.encoding, Some("base64".to_string()));
    assert_eq!(parsed.tool_input.mime_type, Some("image/png".to_string()));
}

#[test]
fn test_read_from_stdin_bash_command() {
    // Verify that parsing works correctly for stdin-style JSON input
    let json = r#"{
        "toolName": "Bash",
        "toolInput": {
            "command": "kubectl get pods"
        }
    }"#;

    // The parsing logic is tested separately; this test documents
    // that stdin input follows the same PreToolUse JSON format
    let result = Engine::parse_and_validate_pre_tool_use(json);
    assert!(result.is_ok());
    assert_eq!(result.unwrap().tool_name, "Bash");
}

#[test]
fn test_parse_malformed_json_syntax_error() {
    let _malformed = r#"{
        "toolName": "Bash",
        "toolInput": {
            "command": "git status"
        },  <- extra comma here
    }"#;

    let result = Engine::parse_and_validate_pre_tool_use(_malformed);
    assert!(result.is_err());
}

#[test]
fn test_parse_extra_fields_allowed() {
    // JSON with extra fields should still parse (forward compatibility)
    let json = r#"{
        "toolName": "Bash",
        "toolInput": {
            "command": "git status"
        },
        "extraField": "someValue",
        "anotherField": 123
    }"#;

    let result = Engine::parse_and_validate_pre_tool_use(json);
    // Should succeed - extra fields are ignored
    assert!(result.is_ok());

    let parsed = result.unwrap();
    assert_eq!(parsed.tool_name, "Bash");
}

// ---------------------------------------------------------------------------
// Single-file Write/Edit extraction (irrevers-beebb069)
//
// The hook front-end runs stdin JSON -> parse_and_validate_pre_tool_use ->
// input_source_from_pre_tool_use -> evaluate_content. These tests pin two
// properties of that pipeline for single-file Write/Edit events:
//
// 1. The caller-supplied file_path reaches the ContentSource verbatim,
//    whatever path form the harness sent (relative, ./-prefixed, absolute,
//    snake_case wire spelling). Nothing on the extraction path may normalize
//    or canonicalize it.
// 2. A missing or malformed file_path, or a non-object tool_input, produces
//    no source at all and never panics; the front-end then allows the
//    operation (asserted end-to-end against the compiled hook below).
//
// github_workflows::detect stays the only matcher; nothing here re-implements
// detection -- the end-to-end probes only assert the allow/deny decision.

/// Extract the ContentSource a single-file Write/Edit event produces, going
/// through the same two-step pipeline the hook front-end uses.
fn extract_content_source(
    tool_name: &str,
    tool_input: Value,
) -> Result<ContentSource, PreToolUseError> {
    let payload = json!({"toolName": tool_name, "toolInput": tool_input});
    let parsed = Engine::parse_and_validate_pre_tool_use(&payload.to_string())?;
    match Engine::input_source_from_pre_tool_use(parsed)? {
        Some(InputSource::Content(source)) => Ok(source),
        other => panic!(
            "{tool_name} is a single-file tool and must produce a content source, got {other:?}"
        ),
    }
}

#[test]
fn test_write_extraction_preserves_file_path_forms_verbatim() {
    for file_path in [
        "packs/storage.yaml",              // relative
        "./packs/storage.yaml",            // ./-prefixed
        "/home/coding/packs/storage.yaml", // absolute
    ] {
        let source = extract_content_source(
            "Write",
            json!({"filePath": file_path, "content": "storageClassName: sata\n"}),
        )
        .unwrap_or_else(|error| panic!("valid Write payload should extract: {error}"));

        assert_eq!(
            source.file_path(),
            file_path,
            "the caller-supplied file_path must reach the ContentSource unmodified"
        );
    }
}

#[test]
fn test_edit_extraction_preserves_file_path_forms_verbatim() {
    for file_path in [
        "docs/notes/design.md",              // relative
        "./docs/notes/design.md",            // ./-prefixed
        "/home/coding/docs/notes/design.md", // absolute
    ] {
        let source = extract_content_source(
            "Edit",
            json!({
                "filePath": file_path,
                "oldString": "storageClassName: ssd",
                "newString": "storageClassName: sata",
            }),
        )
        .unwrap_or_else(|error| panic!("valid Edit payload should extract: {error}"));

        assert_eq!(
            source.file_path(),
            file_path,
            "the caller-supplied file_path must reach the ContentSource unmodified"
        );
    }
}

#[test]
fn test_claude_code_snake_case_wire_spelling_extracts_verbatim() {
    // Claude Code emits snake_case on the hook wire (tool_name/tool_input and
    // file_path/old_string/new_string); the parser accepts both spellings and
    // must hand through the path exactly as sent in either.
    for (tool_name, tool_input) in [
        (
            "Write",
            json!({"file_path": "./pods/storage.yaml", "content": "storageClassName: sata\n"}),
        ),
        (
            "Edit",
            json!({
                "file_path": "pods/storage.yaml",
                "old_string": "storageClassName: ssd",
                "new_string": "storageClassName: sata",
            }),
        ),
    ] {
        let payload = json!({"tool_name": tool_name, "tool_input": tool_input});
        let parsed = Engine::parse_and_validate_pre_tool_use(&payload.to_string())
            .unwrap_or_else(|error| panic!("snake_case {tool_name} payload should parse: {error}"));
        let source = match Engine::input_source_from_pre_tool_use(parsed).unwrap_or_else(|error| {
            panic!("snake_case {tool_name} payload should extract: {error}")
        }) {
            Some(InputSource::Content(source)) => source,
            other => panic!("expected a content source for {tool_name}, got {other:?}"),
        };

        let expected = if tool_name == "Write" {
            "./pods/storage.yaml"
        } else {
            "pods/storage.yaml"
        };
        assert_eq!(
            source.file_path(),
            expected,
            "snake_case file_path must reach the ContentSource unmodified"
        );
    }
}

#[test]
fn test_write_missing_file_path_yields_no_source_without_panicking() {
    let input = PreToolUseInput {
        tool_name: "Write".to_string(),
        tool_input: ToolInput {
            command: None,
            file_path: None,
            content: Some("storageClassName: ssd\n".to_string()),
            old_string: None,
            new_string: None,
            encoding: None,
            mime_type: None,
        },
        id: None,
        timestamp: None,
        session_id: None,
    };

    // No panic; an error (no source) is the fail-open shape -- the hook
    // front-end converts it to an allow.
    let error = Engine::input_source_from_pre_tool_use(input)
        .expect_err("Write without file_path must not yield a source");
    assert!(
        matches!(&error, PreToolUseError::InvalidInput { tool, .. } if tool == "Write"),
        "expected InvalidInput for Write, got {error}"
    );
}

#[test]
fn test_edit_missing_file_path_yields_no_source_without_panicking() {
    let input = PreToolUseInput {
        tool_name: "Edit".to_string(),
        tool_input: ToolInput {
            command: None,
            file_path: None,
            content: None,
            old_string: Some("image: nginx:1.19".to_string()),
            new_string: Some("image: nginx:latest".to_string()),
            encoding: None,
            mime_type: None,
        },
        id: None,
        timestamp: None,
        session_id: None,
    };

    let error = Engine::input_source_from_pre_tool_use(input)
        .expect_err("Edit without file_path must not yield a source");
    assert!(
        matches!(&error, PreToolUseError::InvalidInput { tool, .. } if tool == "Edit"),
        "expected InvalidInput for Edit, got {error}"
    );
}

#[test]
fn test_malformed_file_path_and_tool_input_fail_without_panicking() {
    // Each of these is a malformed single-file Write/Edit event: a non-string
    // file_path, an explicitly null one, or a tool_input that is not an
    // object at all. Parsing must reject them as data errors -- never panic,
    // never fabricate a source.
    let malformed_payloads = [
        (
            "filePath as number",
            json!({"toolName": "Write", "toolInput": {"filePath": 123, "content": "x"}}),
        ),
        (
            "filePath as null",
            json!({"toolName": "Write", "toolInput": {"filePath": null, "content": "x"}}),
        ),
        (
            "toolInput as string",
            json!({"toolName": "Write", "toolInput": "rotate-credentials"}),
        ),
        (
            "toolInput as sequence",
            json!({"toolName": "Edit", "toolInput": ["./docs/plan/plan.md"]}),
        ),
        (
            "toolInput as null",
            json!({"toolName": "Edit", "toolInput": null}),
        ),
    ];

    for (label, payload) in malformed_payloads {
        let result = Engine::parse_and_validate_pre_tool_use(&payload.to_string());
        let error = result.err().unwrap_or_else(|| {
            panic!("malformed payload ({label}) must be rejected, not accepted")
        });
        assert!(
            matches!(
                error,
                PreToolUseError::InvalidJson(_) | PreToolUseError::InvalidInput { .. }
            ),
            "malformed payload ({label}) must be a data error, got {error}"
        );
        // The Display impl is what the hook front-end logs; it must stay
        // total for these shapes too.
        let _rendered = format!("{error}");
    }
}

// --- End-to-end fail-open probes -------------------------------------------
//
// The allow decision itself only exists at the hook front-end boundary
// (extraction errors are converted to None -> Allowed there), so these drive
// the compiled binary the way Claude Code would. A probe pack denies a
// well-formed Write carrying deny-worthy content, which keeps the allow
// assertions below non-vacuous: the malformed events are allowed because the
// front-end fails open, not because nothing would have matched.

fn write_probe_pack(temp: &Path) -> PathBuf {
    let pack_path = temp.join("probe-pack.json");
    let pack = json!({
        "id": "pretooluse-extraction-probe",
        "tool_keywords": [],
        "applies_to": ["*.yaml"],
        "safe_patterns": [],
        "guarded_patterns": [{
            "id": "probe-ssd-storage-class",
            "type": "content_regex",
            "regex": "storageClassName:\\s*ssd",
            "tier": "tier1",
            "severity": "High",
            "explanation": "test probe: ssd storage classes are denied so fail-open probes are non-vacuous",
            "destructive": true,
            "redirect": {
                "channel": "deny",
                "reason_template": "probe: ssd storage classes are forbidden, use sata",
                "rewrite_template": null
            }
        }]
    });
    std::fs::write(
        &pack_path,
        serde_json::to_vec_pretty(&pack).expect("pack should serialize"),
    )
    .expect("pack should be written");
    pack_path
}

fn run_hook_with_tool_input(rule_pack: &Path, tool_input: Value) -> Value {
    let payload = json!({"toolName": "Write", "toolInput": tool_input});
    let telemetry_dir = tempfile::tempdir().expect("telemetry tempdir should be created");

    let mut child = Command::new(env!("CARGO_BIN_EXE_icg"))
        .args([
            "hook",
            "--rule-pack",
            rule_pack.to_str().expect("pack path should be UTF-8"),
        ])
        .env(
            "ICG_TELEMETRY_PATH",
            telemetry_dir.path().join("telemetry.json"),
        )
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("hook process should start");
    child
        .stdin
        .take()
        .expect("hook stdin should be available")
        .write_all(payload.to_string().as_bytes())
        .expect("hook input should be written");
    let output = child
        .wait_with_output()
        .expect("hook process should finish");

    assert!(
        output.status.success(),
        "hook must not crash on malformed input; stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).expect("hook stdout should be one JSON object")
}

fn permission_decision(response: &Value) -> &str {
    response["hookSpecificOutput"]["permissionDecision"]
        .as_str()
        .unwrap_or_else(|| panic!("hook response should carry a permission decision: {response:?}"))
}

#[test]
fn test_hook_denies_well_formed_write_control() {
    // Control for the fail-open probes: the probe pack does deny a
    // well-formed Write whose content matches, dispatched off the same
    // caller-supplied relative file_path.
    let temp = tempfile::tempdir().expect("temporary directory should be created");
    let pack_path = write_probe_pack(temp.path());

    let response = run_hook_with_tool_input(
        &pack_path,
        json!({"filePath": "pods/storage.yaml", "content": "storageClassName: ssd\n"}),
    );

    assert_eq!(
        permission_decision(&response),
        "deny",
        "probe pack should deny a well-formed deny-worthy Write, got {response:?}"
    );
}

#[test]
fn test_hook_fails_open_on_write_missing_file_path() {
    let temp = tempfile::tempdir().expect("temporary directory should be created");
    let pack_path = write_probe_pack(temp.path());

    // The content alone would be denied had a source been extracted; the
    // missing file_path must fail open instead.
    let response =
        run_hook_with_tool_input(&pack_path, json!({"content": "storageClassName: ssd\n"}));

    assert_eq!(
        permission_decision(&response),
        "allow",
        "missing file_path must fail open, got {response:?}"
    );
}

#[test]
fn test_hook_fails_open_on_write_malformed_file_path_type() {
    let temp = tempfile::tempdir().expect("temporary directory should be created");
    let pack_path = write_probe_pack(temp.path());

    let response = run_hook_with_tool_input(
        &pack_path,
        json!({"filePath": 123, "content": "storageClassName: ssd\n"}),
    );

    assert_eq!(
        permission_decision(&response),
        "allow",
        "malformed file_path type must fail open, got {response:?}"
    );
}

#[test]
fn test_hook_fails_open_on_non_object_tool_input() {
    let temp = tempfile::tempdir().expect("temporary directory should be created");
    let pack_path = write_probe_pack(temp.path());

    let response = run_hook_with_tool_input(&pack_path, json!("rotate-credentials"));

    assert_eq!(
        permission_decision(&response),
        "allow",
        "non-object tool_input must fail open, got {response:?}"
    );
}
