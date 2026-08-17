use serde_json::{json, Value};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use tempfile::TempDir;

fn write_pack(temp: &TempDir, pack: Value) -> PathBuf {
    let path = temp.path().join("pack.json");
    std::fs::write(
        &path,
        serde_json::to_vec_pretty(&pack).expect("rule pack should serialize"),
    )
    .expect("rule pack should be written");
    path
}

fn command_pack(temp: &TempDir, rewrite_template: Value, id: &str) -> PathBuf {
    write_pack(
        temp,
        json!({
            "id": id,
            "tool_keywords": ["git"],
            "applies_to": [],
            "safe_patterns": [],
            "guarded_patterns": [{
                "id": "rewrite-dangerous-command",
                "type": "command_regex",
                "regex": "git danger",
                "tier": "tier1",
                "severity": "High",
                "explanation": "The command needs a safe rewrite",
                "redirect": {
                    "channel": "updated_input",
                    "reason_template": "Use the safe command",
                    "rewrite_template": rewrite_template
                },
                "destructive": true
            }]
        }),
    )
}

fn run_hook(rule_pack: &Path, payload: Value) -> Value {
    let mut child = Command::new(env!("CARGO_BIN_EXE_icg"))
        .args([
            "hook",
            "--rule-pack",
            rule_pack.to_str().expect("temporary path should be UTF-8"),
        ])
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
        "hook failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    // Parsing the complete stdout buffer ensures the command produced exactly
    // one valid JSON response and did not mix diagnostics into stdout.
    serde_json::from_slice(&output.stdout).expect("hook stdout should be one JSON object")
}

fn expected_response(
    decision: &str,
    updated_input: Option<Value>,
    reason_field: Option<(&str, &str)>,
) -> Value {
    let mut hook_output = serde_json::Map::new();
    hook_output.insert(
        "hookEventName".to_string(),
        Value::String("PreToolUse".to_string()),
    );
    hook_output.insert(
        "permissionDecision".to_string(),
        Value::String(decision.to_string()),
    );
    if let Some(updated_input) = updated_input {
        hook_output.insert("updatedInput".to_string(), updated_input);
    }
    if let Some((field, value)) = reason_field {
        hook_output.insert(field.to_string(), Value::String(value.to_string()));
    }
    json!({"hookSpecificOutput": hook_output})
}

#[test]
fn command_rewrite_has_exact_shared_envelope_and_preserves_codex_fields() {
    let temp = tempfile::tempdir().expect("temporary directory should be created");
    let pack = command_pack(
        &temp,
        json!("git push --force-with-lease origin main"),
        "exact-command-rewrite",
    );
    let response = run_hook(
        &pack,
        json!({
            "hook_event_name": "PreToolUse",
            "tool_name": "Bash",
            "tool_input": {
                "command": "git danger",
                "description": "Push reviewed changes",
                "timeout": 120000,
                "run_in_background": false
            }
        }),
    );

    assert_eq!(
        response,
        expected_response(
            "allow",
            Some(json!({
                "command": "git push --force-with-lease origin main",
                "description": "Push reviewed changes",
                "timeout": 120000,
                "run_in_background": false
            })),
            Some((
                "additionalContext",
                "Use the safe command [pack=exact-command-rewrite, pattern=rewrite-dangerous-command]",
            )),
        )
    );
}

#[test]
fn rewrite_template_variants_are_serialized_without_normalization() {
    let cases = [
        ("null-falls-back-to-original", Value::Null, "git danger"),
        ("empty-rewrite", json!(""), ""),
        (
            "special-characters",
            json!(r#"printf 'quoted "text"\n$HOME\\tmp; done'"#),
            r#"printf 'quoted "text"\n$HOME\\tmp; done'"#,
        ),
        (
            "multi-command-rewrite",
            json!("git status && printf 'safe; still one rewrite'"),
            "git status && printf 'safe; still one rewrite'",
        ),
    ];

    for (id, rewrite_template, expected_command) in cases {
        let temp = tempfile::tempdir().expect("temporary directory should be created");
        let pack = command_pack(&temp, rewrite_template, id);
        let response = run_hook(
            &pack,
            json!({
                "tool_name": "Bash",
                "tool_input": {
                    "command": "git danger"
                }
            }),
        );
        let expected_context =
            format!("Use the safe command [pack={id}, pattern=rewrite-dangerous-command]");

        assert_eq!(
            response,
            expected_response(
                "allow",
                Some(json!({"command": expected_command})),
                Some(("additionalContext", expected_context.as_str())),
            ),
            "unexpected response for rewrite template case {id}"
        );
    }
}

#[test]
fn deny_responses_never_contain_updated_input_for_claude_or_codex() {
    let temp = tempfile::tempdir().expect("temporary directory should be created");
    let pack = write_pack(
        &temp,
        json!({
            "id": "deny-without-rewrite",
            "tool_keywords": ["git"],
            "applies_to": [],
            "safe_patterns": [],
            "guarded_patterns": [{
                "id": "deny-dangerous-command",
                "type": "command_regex",
                "regex": "git reset --hard",
                "tier": "tier1",
                "severity": "Critical",
                "explanation": "The command discards work",
                "redirect": {
                    "channel": "deny",
                    "reason_template": "Do not discard work",
                    "rewrite_template": null
                },
                "destructive": true
            }]
        }),
    );

    let payloads = [
        json!({
            "toolName": "Bash",
            "toolInput": {"command": "git reset --hard HEAD"}
        }),
        json!({
            "tool_name": "Bash",
            "tool_input": {"command": "git reset --hard HEAD"}
        }),
    ];
    for payload in payloads {
        let response = run_hook(&pack, payload);
        assert_eq!(
            response,
            expected_response(
                "deny",
                None,
                Some((
                    "permissionDecisionReason",
                    "Do not discard work [pack=deny-without-rewrite, pattern=deny-dangerous-command]",
                )),
            )
        );
        assert!(response["hookSpecificOutput"].get("updatedInput").is_none());
    }
}

#[test]
fn missing_required_fields_fail_open_without_updated_input_for_both_schemas() {
    let temp = tempfile::tempdir().expect("temporary directory should be created");
    let pack = command_pack(&temp, json!("safe"), "missing-field-input");
    let payloads = [
        json!({"toolName": "Bash", "toolInput": {}}),
        json!({"tool_name": "Bash", "tool_input": {}}),
    ];

    for payload in payloads {
        let response = run_hook(&pack, payload);
        assert_eq!(
            response,
            expected_response("allow", None, None),
            "malformed input must not manufacture updatedInput"
        );
        assert!(response["hookSpecificOutput"].get("updatedInput").is_none());
    }
}

#[test]
fn claude_code_camel_case_write_rewrite_preserves_input_key_spelling() {
    let temp = tempfile::tempdir().expect("temporary directory should be created");
    let pack = write_pack(
        &temp,
        json!({
            "id": "claude-write-rewrite",
            "tool_keywords": [],
            "applies_to": ["*.yaml"],
            "safe_patterns": [],
            "guarded_patterns": [{
                "id": "rewrite-storage-class",
                "type": "content_regex",
                "regex": "storageClassName: ssd",
                "tier": "tier1",
                "severity": "High",
                "explanation": "Use the portable storage class",
                "redirect": {
                    "channel": "updated_input",
                    "reason_template": "Use sata",
                    "rewrite_template": "storageClassName: sata\n"
                },
                "destructive": true
            }]
        }),
    );
    let response = run_hook(
        &pack,
        json!({
            "toolName": "Write",
            "toolInput": {
                "filePath": "deploy/app.yaml",
                "content": "storageClassName: ssd\n",
                "encoding": "utf-8"
            }
        }),
    );

    assert_eq!(
        response,
        expected_response(
            "allow",
            Some(json!({
                "filePath": "deploy/app.yaml",
                "content": "storageClassName: sata\n",
                "encoding": "utf-8"
            })),
            Some((
                "additionalContext",
                "Use sata [pack=claude-write-rewrite, pattern=rewrite-storage-class, file=deploy/app.yaml]",
            )),
        )
    );
    assert!(response["hookSpecificOutput"]["updatedInput"]
        .get("file_path")
        .is_none());
}

#[test]
fn codex_apply_patch_rewrite_returns_a_complete_command_input() {
    let temp = tempfile::tempdir().expect("temporary directory should be created");
    let replacement_patch = "*** Begin Patch\n*** Update File: deploy/app.yaml\n@@\n-image: app:latest\n+image: app:1.2.3\n*** End Patch";
    let pack = write_pack(
        &temp,
        json!({
            "id": "codex-apply-patch-rewrite",
            "tool_keywords": [],
            "applies_to": ["*.yaml"],
            "safe_patterns": [],
            "guarded_patterns": [{
                "id": "rewrite-latest-image",
                "type": "content_regex",
                "regex": "image: app:latest",
                "tier": "tier1",
                "severity": "High",
                "explanation": "Pin the image",
                "redirect": {
                    "channel": "updated_input",
                    "reason_template": "Pin the image",
                    "rewrite_template": replacement_patch
                },
                "destructive": true
            }]
        }),
    );
    let original_patch = "*** Begin Patch\n*** Update File: deploy/app.yaml\n@@\n-image: app:1.2.3\n+image: app:latest\n*** End Patch";
    let response = run_hook(
        &pack,
        json!({
            "hook_event_name": "PreToolUse",
            "tool_name": "apply_patch",
            "tool_input": {
                "command": original_patch,
                "description": "Update the deployment"
            }
        }),
    );

    assert_eq!(
        response,
        expected_response(
            "allow",
            Some(json!({
                "command": replacement_patch,
                "description": "Update the deployment"
            })),
            Some((
                "additionalContext",
                "Pin the image [pack=codex-apply-patch-rewrite, pattern=rewrite-latest-image, file=deploy/app.yaml]",
            )),
        )
    );
}
