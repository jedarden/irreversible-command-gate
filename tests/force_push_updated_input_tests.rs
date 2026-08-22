use serde_json::{json, Value};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use tempfile::TempDir;

const REWRITE_REASON: &str = "Stripped --force/-f/--force-with-lease flags because force-pushing can overwrite remote history and lose commits; a normal push is safer.";
const SANITIZED_COMMAND: &str = "git push origin main";

fn force_push_pack(temp: &TempDir) -> PathBuf {
    let path = temp.path().join("git-force-push-pack.json");
    let pack = json!({
        "id": "git-force-push-updated-input",
        "tool_keywords": ["git"],
        "applies_to": [],
        "safe_patterns": [],
        "guarded_patterns": [{
            "id": "strip-force-push-flags",
            "type": "command_regex",
            "regex": "^git\\s+push(?:\\s+\\S+)*\\s+(?:--force-with-lease(?:=\\S+)?|--force|-f)(?:\\s|$)",
            "tier": "tier1",
            "severity": "Critical",
            "explanation": "Force-pushing can overwrite remote history",
            "redirect": {
                "channel": "updated_input",
                "reason_template": REWRITE_REASON,
                "rewrite_template": "{command_without_force}"
            },
            "destructive": true
        }]
    });
    std::fs::write(
        &path,
        serde_json::to_vec_pretty(&pack).expect("rule pack should serialize"),
    )
    .expect("rule pack should be written");
    path
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
    serde_json::from_slice(&output.stdout).expect("hook stdout should be one JSON object")
}

fn assert_force_push_rewrite(response: &Value, expected_input: &Value) {
    assert_eq!(
        response,
        &json!({
            "hookSpecificOutput": {
                "hookEventName": "PreToolUse",
                "permissionDecision": "allow",
                "updatedInput": {
                    "command": SANITIZED_COMMAND,
                    "description": "Push reviewed changes",
                    "timeout": 120000,
                    "run_in_background": false
                },
                "additionalContext": format!(
                    "{REWRITE_REASON} [pack=git-force-push-updated-input, pattern=strip-force-push-flags]"
                )
            }
        })
    );

    let hook_output = &response["hookSpecificOutput"];
    assert_eq!(hook_output["updatedInput"]["command"], SANITIZED_COMMAND);
    assert!(!hook_output["updatedInput"]["command"]
        .as_str()
        .expect("rewritten command should be a string")
        .contains("--force"));
    assert!(hook_output.get("permissionDecisionReason").is_none());
    assert_eq!(&hook_output["updatedInput"], expected_input);
    let explanation = hook_output["additionalContext"]
        .as_str()
        .expect("rewrite explanation should be a string");
    assert!(explanation.contains("Stripped --force/-f/--force-with-lease flags"));
    assert!(explanation.contains("overwrite remote history and lose commits"));
}

#[test]
fn force_push_is_sanitized_in_updated_input_for_both_harness_payloads() {
    let temp = tempfile::tempdir().expect("temporary directory should be created");
    let pack = force_push_pack(&temp);

    let cases = [
        (
            "Claude Code camelCase payload",
            json!({
                "hookEventName": "PreToolUse",
                "toolName": "Bash",
                "toolInput": {
                    "command": "git push --force origin main",
                    "description": "Push reviewed changes",
                    "timeout": 120000,
                    "run_in_background": false
                }
            }),
        ),
        (
            "Codex CLI snake_case short flag payload",
            json!({
                "hook_event_name": "PreToolUse",
                "tool_name": "Bash",
                "tool_input": {
                    "command": "git push -f origin main",
                    "description": "Push reviewed changes",
                    "timeout": 120000,
                    "run_in_background": false
                }
            }),
        ),
        (
            "Codex CLI snake_case lease flag payload",
            json!({
                "hook_event_name": "PreToolUse",
                "tool_name": "Bash",
                "tool_input": {
                    "command": "git push --force-with-lease origin main",
                    "description": "Push reviewed changes",
                    "timeout": 120000,
                    "run_in_background": false
                }
            }),
        ),
    ];

    let expected_input = json!({
        "command": SANITIZED_COMMAND,
        "description": "Push reviewed changes",
        "timeout": 120000,
        "run_in_background": false
    });

    for (harness, payload) in cases {
        let response = run_hook(&pack, payload);
        assert_force_push_rewrite(&response, &expected_input);
        assert!(
            response["hookSpecificOutput"]["additionalContext"]
                .as_str()
                .expect("rewrite explanation should be present")
                .contains("force-pushing"),
            "{harness} response should explain why the flag was stripped"
        );
    }
}
