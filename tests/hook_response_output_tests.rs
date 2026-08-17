use serde_json::{json, Value};
use std::io::Write;
use std::process::{Command, Stdio};
use tempfile::tempdir;

fn run_hook(rule_pack: &std::path::Path, tool_input: Value) -> Value {
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
        "tool_name": "Bash",
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
    assert_eq!(
        allowed["hookSpecificOutput"]["permissionDecision"],
        "allow"
    );

    let denied = run_hook(&pack_path, json!({"command": "git reset --hard HEAD"}));
    assert_eq!(
        denied["hookSpecificOutput"]["permissionDecision"],
        "deny"
    );
    assert!(denied["hookSpecificOutput"]["permissionDecisionReason"]
        .as_str()
        .expect("deny reason should be a string")
        .contains("Do not discard work"));

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

    let warning = run_hook(&pack_path, json!({"command": "git worktree add path branch"}));
    assert_eq!(
        warning["hookSpecificOutput"]["permissionDecision"],
        "allow"
    );
    assert!(warning["hookSpecificOutput"]["additionalContext"]
        .as_str()
        .expect("warning context should be a string")
        .contains("Verify the worktree is disposable"));
}
