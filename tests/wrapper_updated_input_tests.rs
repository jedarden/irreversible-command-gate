//! PATH-wrapper updatedInput realization tests.
//!
//! A hook can return updatedInput as JSON, but the wrapper has to realize the
//! same decision by changing the argv passed to the real binary.  In
//! particular, a force-push rewrite must not merely print a replacement: the
//! real tool must receive argv without the dangerous flag.

#![cfg(unix)]

use serde_json::json;
use std::ffi::OsString;
use std::os::unix::fs::PermissionsExt;
use std::process::Command;

#[test]
fn updated_input_rewrites_wrapper_argv_before_exec() {
    let temp = tempfile::tempdir().expect("temporary directory should be created");
    let tool_dir = temp.path().join("bin");
    std::fs::create_dir(&tool_dir).expect("tool directory should be created");

    let tool = "icg-wrapper-rewrite-tool";
    let tool_path = tool_dir.join(tool);
    std::fs::write(
        &tool_path,
        "#!/bin/sh\nprintf 'argv:'\nfor arg do printf ' [%s]' \"$arg\"; done\nprintf '\\n'\n",
    )
    .expect("fake real tool should be written");
    let mut permissions = std::fs::metadata(&tool_path)
        .expect("fake real tool metadata should be available")
        .permissions();
    permissions.set_mode(0o755);
    std::fs::set_permissions(&tool_path, permissions).expect("fake real tool should be executable");

    let pack_path = temp.path().join("rewrite-pack.json");
    std::fs::write(
        &pack_path,
        serde_json::to_vec_pretty(&json!({
            "id": "wrapper-rewrite-pack",
            "tool_keywords": [tool],
            "applies_to": [],
            "safe_patterns": [],
            "guarded_patterns": [{
                "id": "strip-force-flag",
                "type": "command_regex",
                "regex": format!(r"{tool} push.*--force(?:-with-lease)?"),
                "tier": "tier1",
                "severity": "Critical",
                "explanation": "Force-pushing can overwrite remote history",
                "redirect": {
                    "channel": "updated_input",
                    "reason_template": "Stripped the force flag before execution",
                    "rewrite_template": format!("{tool} push origin main")
                },
                "destructive": true
            }]
        }))
        .expect("rewrite pack should serialize"),
    )
    .expect("rewrite pack should be written");

    let mut path_entries = vec![tool_dir];
    if let Some(path) = std::env::var_os("PATH") {
        path_entries.extend(std::env::split_paths(&path));
    }
    let path = std::env::join_paths(path_entries).expect("test PATH should be valid");

    let output = Command::new(env!("CARGO_BIN_EXE_icg"))
        .args(["wrapper", tool, "push", "--force", "origin", "main"])
        .env("ICG_RULE_PACK", &pack_path)
        .env("ICG_HEALTH_PATH", temp.path().join("health.json"))
        .env("ICG_TELEMETRY_PATH", temp.path().join("telemetry.json"))
        .env("HOME", temp.path())
        .env("XDG_CACHE_HOME", temp.path().join("cache"))
        .env("PATH", OsString::from(path))
        .output()
        .expect("wrapper should run");

    assert!(
        output.status.success(),
        "rewritten wrapper command should execute successfully: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        "argv: [push] [origin] [main]\n",
        "the real tool should receive rewritten argv without --force"
    );

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains(
            "icg updated command: Stripped the force flag before execution [pack=wrapper-rewrite-pack, pattern=strip-force-flag]"
        ),
        "wrapper should explain the argv rewrite on stderr, got: {stderr}"
    );
}
