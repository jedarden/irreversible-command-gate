use serde_json::{json, Value};
use std::io::Write;
use std::process::{Command, Stdio};
use tempfile::tempdir;

fn run_hook(rule_pack: &std::path::Path, tool_input: Value) -> Value {
    run_hook_for_tool(rule_pack, "Bash", tool_input)
}

fn run_hook_for_tool(rule_pack: &std::path::Path, tool_name: &str, tool_input: Value) -> Value {
    let mut child = Command::new(env!("CARGO_BIN_EXE_icg"))
        .args([
            "hook",
            "--rule-pack",
            rule_pack.to_str().expect("temporary path should be UTF-8"),
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("hook process should start");

    let input = json!({
        "tool_name": tool_name,
        "tool_input": tool_input,
    });
    child
        .stdin
        .take()
        .expect("hook stdin should be available")
        .write_all(input.to_string().as_bytes())
        .expect("hook input should be written");

    let output = child
        .wait_with_output()
        .expect("hook process should finish");
    assert!(output.status.success(), "hook failed: {:?}", output.status);
    serde_json::from_slice(&output.stdout).expect("hook stdout should be one JSON object")
}

fn test_pack() -> Value {
    json!({
        "id": "hook-response-output-test",
        "tool_keywords": ["git"],
        "applies_to": [],
        "safe_patterns": [],
        "guarded_patterns": [
            {
                "id": "deny-reset",
                "type": "command_regex",
                "regex": "git reset --hard",
                "tier": "tier1",
                "severity": "Critical",
                "explanation": "Reset discards work",
                "redirect": {
                    "channel": "deny",
                    "reason_template": "Do not discard work",
                    "rewrite_template": null
                },
                "destructive": true
            },
            {
                "id": "rewrite-force-push",
                "type": "command_regex",
                "regex": "git push.*--force",
                "tier": "tier1",
                "severity": "High",
                "explanation": "Use the lease form",
                "redirect": {
                    "channel": "updated_input",
                    "reason_template": "Use --force-with-lease",
                    "rewrite_template": "git push --force-with-lease"
                },
                "destructive": true
            },
            {
                "id": "warn-worktree",
                "type": "command_regex",
                "regex": "git worktree add",
                "tier": "tier3",
                "severity": "Medium",
                "explanation": "Check the target",
                "redirect": {
                    "channel": "additional_context",
                    "reason_template": "Verify the worktree is disposable",
                    "rewrite_template": null
                },
                "destructive": false
            }
        ]
    })
}

#[test]
fn emits_allow_deny_rewrite_and_warning_responses() {
    let temp = tempdir().expect("temporary directory should be created");
    let pack_path = temp.path().join("pack.json");
    std::fs::write(
        &pack_path,
        serde_json::to_vec_pretty(&test_pack()).expect("pack should serialize"),
    )
    .expect("pack should be written");

    let allowed = run_hook(&pack_path, json!({"command": "git status"}));
    assert_eq!(allowed["hookSpecificOutput"]["permissionDecision"], "allow");

    let denied = run_hook(&pack_path, json!({"command": "git reset --hard HEAD"}));
    assert_eq!(denied["hookSpecificOutput"]["permissionDecision"], "deny");
    assert!(denied["hookSpecificOutput"]["permissionDecisionReason"]
        .as_str()
        .expect("deny reason should be a string")
        .contains("Do not discard work"));
    assert!(denied["hookSpecificOutput"].get("updatedInput").is_none());

    let rewritten = run_hook(
        &pack_path,
        json!({
            "command": "git push origin main --force",
            "description": "preserve this field"
        }),
    );
    assert_eq!(
        rewritten["hookSpecificOutput"]["permissionDecision"],
        "allow"
    );
    assert_eq!(
        rewritten["hookSpecificOutput"]["updatedInput"]["command"],
        "git push --force-with-lease"
    );
    assert_eq!(
        rewritten["hookSpecificOutput"]["updatedInput"]["description"],
        "preserve this field"
    );

    let warning = run_hook(
        &pack_path,
        json!({"command": "git worktree add path branch"}),
    );
    assert_eq!(warning["hookSpecificOutput"]["permissionDecision"], "allow");
    assert!(warning["hookSpecificOutput"]["additionalContext"]
        .as_str()
        .expect("warning context should be a string")
        .contains("Verify the worktree is disposable"));
}

#[test]
fn serializes_content_rewrite_template_and_preserves_tool_input() {
    let temp = tempdir().expect("temporary directory should be created");
    let pack_path = temp.path().join("pack.json");
    let pack = json!({
        "id": "content-rewrite-test",
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
                "reason_template": "Rewrite the storage class",
                "rewrite_template": "storageClassName: sata"
            },
            "destructive": true
        }]
    });
    std::fs::write(
        &pack_path,
        serde_json::to_vec_pretty(&pack).expect("pack should serialize"),
    )
    .expect("pack should be written");

    let rewritten = run_hook_for_tool(
        &pack_path,
        "Write",
        json!({
            "filePath": "deployment.yaml",
            "content": "storageClassName: ssd",
            "encoding": "utf-8"
        }),
    );
    let output = &rewritten["hookSpecificOutput"];
    assert_eq!(output["permissionDecision"], "allow");
    assert_eq!(output["updatedInput"]["content"], "storageClassName: sata");
    assert_eq!(output["updatedInput"]["filePath"], "deployment.yaml");
    assert_eq!(output["updatedInput"]["encoding"], "utf-8");
}

#[test]
fn preserves_snake_case_edit_input_when_serializing_rewrite() {
    let temp = tempdir().expect("temporary directory should be created");
    let pack_path = temp.path().join("pack.json");
    let pack = json!({
        "id": "edit-rewrite-test",
        "tool_keywords": [],
        "applies_to": ["*.txt"],
        "safe_patterns": [],
        "guarded_patterns": [{
            "id": "rewrite-text",
            "type": "content_regex",
            "regex": "unsafe",
            "tier": "tier1",
            "severity": "High",
            "explanation": "Use the safe text",
            "redirect": {
                "channel": "updated_input",
                "reason_template": "Rewrite the unsafe text",
                "rewrite_template": "safe"
            },
            "destructive": true
        }]
    });
    std::fs::write(
        &pack_path,
        serde_json::to_vec_pretty(&pack).expect("pack should serialize"),
    )
    .expect("pack should be written");

    let rewritten = run_hook_for_tool(
        &pack_path,
        "Edit",
        json!({
            "file_path": "notes.txt",
            "old_string": "unsafe",
            "new_string": "unsafe",
            "description": "preserve this field"
        }),
    );
    let output = &rewritten["hookSpecificOutput"];
    assert_eq!(output["permissionDecision"], "allow");
    assert_eq!(output["updatedInput"]["new_string"], "safe");
    assert!(output["updatedInput"].get("newString").is_none());
    assert_eq!(output["updatedInput"]["description"], "preserve this field");
}
