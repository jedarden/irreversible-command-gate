//! End-to-end integration coverage for the `.github/workflows` write guard.
//!
//! `src/github_workflows.rs` unit-tests the path predicate in isolation, and
//! `src/engine.rs` unit-tests `evaluate_content` calling that predicate
//! directly. Neither exercises the actual hook front-end: stdin JSON ->
//! `Engine::input_source_from_pre_tool_use` -> `evaluate_content`, which is
//! the path `main.rs`'s `hook` subcommand uses for real Write/Edit
//! PreToolUse events. This drives the compiled `icg hook` binary the same
//! way Claude Code would, reusing the true/false-positive fixtures from
//! `github_workflows.rs`'s own tests.

use serde_json::{json, Value};
use std::io::Write;
use std::process::{Command, Stdio};
use tempfile::tempdir;

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

fn empty_pack_path(temp: &std::path::Path) -> std::path::PathBuf {
    let pack_path = temp.join("pack.json");
    let pack = json!({
        "id": "github-workflows-hook-integration-test",
        "tool_keywords": [],
        "applies_to": [],
        "safe_patterns": [],
        "guarded_patterns": []
    });
    std::fs::write(
        &pack_path,
        serde_json::to_vec_pretty(&pack).expect("pack should serialize"),
    )
    .expect("pack should be written");
    pack_path
}

#[test]
fn hook_denies_write_to_github_workflows_path() {
    let temp = tempdir().expect("temporary directory should be created");
    let pack_path = empty_pack_path(temp.path());

    let denied = run_hook_for_tool(
        &pack_path,
        "Write",
        json!({
            "filePath": ".github/workflows/ci.yml",
            "content": "name: ci\non: push\n",
        }),
    );

    assert_eq!(
        denied["hookSpecificOutput"]["permissionDecision"], "deny",
        "expected deny for .github/workflows/ci.yml, got {denied:?}"
    );
    let reason = denied["hookSpecificOutput"]["permissionDecisionReason"]
        .as_str()
        .expect("deny reason should be a string");
    assert!(
        reason.contains(".github/workflows"),
        "deny reason should mention .github/workflows, got: {reason}"
    );
}

#[test]
fn hook_denies_edit_to_github_workflows_path() {
    let temp = tempdir().expect("temporary directory should be created");
    let pack_path = empty_pack_path(temp.path());

    let denied = run_hook_for_tool(
        &pack_path,
        "Edit",
        json!({
            "file_path": ".github/workflows/ci.yml",
            "old_string": "push",
            "new_string": "pull_request",
        }),
    );

    assert_eq!(
        denied["hookSpecificOutput"]["permissionDecision"], "deny",
        "expected deny for .github/workflows/ci.yml, got {denied:?}"
    );
    assert!(denied["hookSpecificOutput"]
        .get("updatedInput")
        .is_none());
}

#[test]
fn hook_allows_write_to_lookalike_paths_outside_dot_github_workflows() {
    let temp = tempdir().expect("temporary directory should be created");
    let pack_path = empty_pack_path(temp.path());

    for file_path in ["src/workflows/foo.yml", ".github/workflows-extra/foo.yml"] {
        let allowed = run_hook_for_tool(
            &pack_path,
            "Write",
            json!({
                "filePath": file_path,
                "content": "name: ci\n",
            }),
        );

        assert_eq!(
            allowed["hookSpecificOutput"]["permissionDecision"], "allow",
            "expected allow for {file_path}, got {allowed:?}"
        );
    }
}
