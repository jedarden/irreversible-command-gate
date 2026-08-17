//! Tests for PreToolUse JSON parsing and validation
//!
//! These tests verify the core parsing functionality that powers the hook front-end.

use icg::engine::{CommandSource, ContentSource, Engine, InputSource, PreToolUseInput, ToolInput};

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
