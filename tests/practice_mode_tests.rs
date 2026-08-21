use serde_json::{json, Value};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use tempfile::TempDir;

fn practice_pack(temp: &TempDir) -> PathBuf {
    let path = temp.path().join("practice-pack.json");
    let pack = json!({
        "id": "practice-mode-test",
        "tool_keywords": ["fake"],
        "applies_to": [],
        "safe_patterns": [],
        "guarded_patterns": [{
            "id": "fake-dangerous",
            "type": "command_regex",
            "regex": "^fake dangerous$",
            "tier": "tier1",
            "severity": "Critical",
            "explanation": "The fake dangerous command is guarded",
            "redirect": {
                "channel": "deny",
                "reason_template": "Do not run the fake dangerous command",
                "rewrite_template": null
            },
            "destructive": true
        }]
    });
    std::fs::write(
        &path,
        serde_json::to_vec_pretty(&pack).expect("practice pack should serialize"),
    )
    .expect("practice pack should be written");
    path
}

fn run_hook(pack: &Path, command: &str, use_flag: bool) -> (Value, String) {
    let temp = tempfile::tempdir().expect("hook support directory should be created");
    let mut child = Command::new(env!("CARGO_BIN_EXE_icg"))
        .arg("hook")
        .arg("--rule-pack")
        .arg(pack)
        .args(use_flag.then_some("--practice"))
        .env_remove("ICG_PRACTICE")
        .env("ICG_TELEMETRY_PATH", temp.path().join("telemetry.json"))
        .env("ICG_HEALTH_PATH", temp.path().join("health.json"))
        .env("ICG_DENIAL_LOG", temp.path().join("denials.jsonl"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("hook process should start");

    child
        .stdin
        .take()
        .expect("hook stdin should be available")
        .write_all(
            json!({
                "hook_event_name": "PreToolUse",
                "tool_name": "Bash",
                "tool_input": {"command": command}
            })
            .to_string()
            .as_bytes(),
        )
        .expect("hook input should be written");

    let output = child
        .wait_with_output()
        .expect("hook process should finish");
    assert!(
        output.status.success(),
        "hook failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    (
        serde_json::from_slice(&output.stdout).expect("hook stdout should be one JSON object"),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}

#[test]
fn practice_hook_reports_would_deny_in_codex_system_message() {
    let temp = tempfile::tempdir().expect("temporary directory should be created");
    let pack = practice_pack(&temp);
    let (response, stderr) = run_hook(&pack, "fake dangerous", true);

    assert_eq!(
        response["hookSpecificOutput"]["permissionDecision"],
        "allow"
    );
    assert!(response["hookSpecificOutput"]
        .get("permissionDecisionReason")
        .is_none());
    assert!(response["hookSpecificOutput"]
        .get("additionalContext")
        .is_none());
    let message = response["systemMessage"]
        .as_str()
        .expect("practice hook should emit Codex's systemMessage");
    assert!(message.contains("ICG PRACTICE MODE ACTIVE"));
    assert!(message.contains("WOULD DENY"));
    assert!(message.contains("fake-dangerous"));
    assert!(
        !stderr.contains("WOULD DENY"),
        "the would-be denial should use systemMessage, not hook stderr: {stderr}"
    );
}

#[test]
fn practice_hook_banner_is_present_for_an_allowed_check() {
    let temp = tempfile::tempdir().expect("temporary directory should be created");
    let pack = practice_pack(&temp);
    let (response, _) = run_hook(&pack, "fake status", true);

    assert_eq!(
        response["hookSpecificOutput"]["permissionDecision"],
        "allow"
    );
    let message = response["systemMessage"]
        .as_str()
        .expect("every practice hook check should carry the active banner");
    assert!(message.contains("ICG PRACTICE MODE ACTIVE"));
    assert!(!message.contains("WOULD DENY"));
}

#[test]
fn practice_env_var_enables_hook_mode_without_a_cli_flag() {
    let temp = tempfile::tempdir().expect("temporary directory should be created");
    let pack = practice_pack(&temp);
    let support = tempfile::tempdir().expect("hook support directory should be created");
    let mut child = Command::new(env!("CARGO_BIN_EXE_icg"))
        .args([
            "hook",
            "--rule-pack",
            pack.to_str().expect("pack path should be UTF-8"),
        ])
        .env("ICG_PRACTICE", "1")
        .env("ICG_TELEMETRY_PATH", support.path().join("telemetry.json"))
        .env("ICG_HEALTH_PATH", support.path().join("health.json"))
        .env("ICG_DENIAL_LOG", support.path().join("denials.jsonl"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("hook process should start");
    child
        .stdin
        .take()
        .expect("hook stdin should be available")
        .write_all(
            json!({
                "hook_event_name": "PreToolUse",
                "tool_name": "Bash",
                "tool_input": {"command": "fake dangerous"}
            })
            .to_string()
            .as_bytes(),
        )
        .expect("hook input should be written");
    let output = child
        .wait_with_output()
        .expect("hook process should finish");
    assert!(output.status.success());
    let response: Value = serde_json::from_slice(&output.stdout).expect("valid hook response");
    assert_eq!(
        response["hookSpecificOutput"]["permissionDecision"],
        "allow"
    );
    assert!(response["systemMessage"]
        .as_str()
        .expect("practice message should be present")
        .contains("WOULD DENY"));
}

#[cfg(unix)]
fn fake_tool(temp: &TempDir) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;

    let path = temp.path().join("fake");
    std::fs::write(&path, "#!/bin/sh\nprintf 'REAL_TOOL_RAN\\n'\n")
        .expect("fake tool should be written");
    let mut permissions = std::fs::metadata(&path)
        .expect("fake tool metadata should be readable")
        .permissions();
    permissions.set_mode(0o755);
    std::fs::set_permissions(&path, permissions).expect("fake tool should be executable");
    path
}

#[cfg(unix)]
fn run_wrapper(temp: &TempDir, pack: &Path, args: &[&str], practice: bool) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_icg"));
    command
        .arg("wrapper")
        .args(practice.then_some("--practice"))
        .arg("fake")
        .args(args)
        .env("PATH", temp.path())
        .env("ICG_RULE_PACK", pack)
        .env_remove("ICG_PRACTICE")
        .env("ICG_HEALTH_PATH", temp.path().join("health.json"))
        .env("ICG_DENIAL_LOG", temp.path().join("denials.jsonl"))
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    command.output().expect("wrapper process should finish")
}

#[cfg(unix)]
#[test]
fn practice_wrapper_reports_and_executes_a_would_be_denial() {
    let temp = tempfile::tempdir().expect("temporary directory should be created");
    fake_tool(&temp);
    let pack = practice_pack(&temp);
    let output = run_wrapper(&temp, &pack, &["dangerous"], true);

    assert!(output.status.success());
    assert_eq!(String::from_utf8_lossy(&output.stdout), "REAL_TOOL_RAN\n");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("ICG PRACTICE MODE ACTIVE"));
    assert!(stderr.contains("WOULD DENY"));
    assert!(stderr.contains("fake-dangerous"));
}

#[cfg(unix)]
#[test]
fn practice_wrapper_banner_is_present_for_an_allowed_check() {
    let temp = tempfile::tempdir().expect("temporary directory should be created");
    fake_tool(&temp);
    let pack = practice_pack(&temp);
    let output = run_wrapper(&temp, &pack, &["status"], true);

    assert!(output.status.success());
    assert_eq!(String::from_utf8_lossy(&output.stdout), "REAL_TOOL_RAN\n");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("ICG PRACTICE MODE ACTIVE"));
    assert!(!stderr.contains("WOULD DENY"));
}

#[cfg(unix)]
#[test]
fn enforcing_wrapper_still_blocks_the_same_command() {
    let temp = tempfile::tempdir().expect("temporary directory should be created");
    fake_tool(&temp);
    let pack = practice_pack(&temp);
    let output = run_wrapper(&temp, &pack, &["dangerous"], false);

    assert!(!output.status.success());
    assert!(!String::from_utf8_lossy(&output.stdout).contains("REAL_TOOL_RAN"));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("command denied"));
    assert!(!stderr.contains("PRACTICE MODE ACTIVE"));
}
