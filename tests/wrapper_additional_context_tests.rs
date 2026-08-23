//! Wrapper front-end additionalContext realization tests.
//!
//! The wrapper has no hook response envelope to carry `additionalContext`.
//! It must surface the warning on stderr while still executing the real tool
//! with the original argv.

#![cfg(unix)]

use serde_json::json;
use std::os::unix::fs::PermissionsExt;
use std::process::Command;

#[test]
fn additional_context_warns_on_stderr_and_executes_original_command() {
    let temp = tempfile::tempdir().expect("temporary directory should be created");
    let tool_dir = temp.path().join("bin");
    std::fs::create_dir(&tool_dir).expect("tool directory should be created");

    let tool = "icg-wrapper-warning-tool";
    let tool_path = tool_dir.join(tool);
    std::fs::write(&tool_path, "#!/bin/sh\nprintf 'real-tool:%s\\n' \"$*\"\n")
        .expect("fake real tool should be written");
    let mut permissions = std::fs::metadata(&tool_path)
        .expect("fake real tool metadata should be available")
        .permissions();
    permissions.set_mode(0o755);
    std::fs::set_permissions(&tool_path, permissions).expect("fake real tool should be executable");

    let pack_path = temp.path().join("warning-pack.json");
    std::fs::write(
        &pack_path,
        serde_json::to_vec_pretty(&json!({
            "id": "wrapper-warning-pack",
            "tool_keywords": [tool],
            "applies_to": [],
            "safe_patterns": [],
            "guarded_patterns": [{
                "id": "warn-wrapper-command",
                "type": "command_regex",
                "regex": format!(r"{tool} warn"),
                "tier": "tier3",
                "severity": "Medium",
                "explanation": "The command deserves operator attention",
                "redirect": {
                    "channel": "additional_context",
                    "reason_template": "Verify the wrapper command before continuing",
                    "rewrite_template": null
                },
                "destructive": false
            }]
        }))
        .expect("warning pack should serialize"),
    )
    .expect("warning pack should be written");

    let mut path_entries = vec![tool_dir];
    if let Some(path) = std::env::var_os("PATH") {
        path_entries.extend(std::env::split_paths(&path));
    }
    let path = std::env::join_paths(path_entries).expect("test PATH should be valid");

    let mut command = Command::new(env!("CARGO_BIN_EXE_icg"));
    command
        .args(["wrapper", tool, "warn", "--preserve-this-argument"])
        .env("ICG_RULE_PACK", &pack_path)
        .env("ICG_HEALTH_PATH", temp.path().join("health.json"))
        .env("ICG_TELEMETRY_PATH", temp.path().join("telemetry.json"))
        .env("PATH", &path);
    let output = command.output().expect("wrapper should run");

    assert!(
        output.status.success(),
        "additionalContext must not block execution: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        "real-tool:warn --preserve-this-argument\n",
        "wrapper should exec the real tool with its original arguments"
    );

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains(
            "icg warning: Verify the wrapper command before continuing [pack=wrapper-warning-pack, pattern=warn-wrapper-command]"
        ),
        "wrapper warning should be printed to stderr, got: {stderr}"
    );
}
