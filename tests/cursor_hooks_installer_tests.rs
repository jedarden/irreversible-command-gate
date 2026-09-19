//! End-to-end tests for `icg install-cursor-hooks` (src/cursor_hooks.rs).
//!
//! The unit tests inside the module pin the merge on strings; these drive
//! the shipped binary so the acceptance criteria hold for the command an
//! operator actually types: idempotence, preservation of unrelated hooks,
//! creation of a missing file, a clear failure on malformed JSON, user-level
//! targeting via HOME, and uninstall.

use std::fs;
use std::path::Path;
use std::process::{Command, Output};

use tempfile::tempdir;

fn installer(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_icg"))
        .args(args)
        .output()
        .expect("icg should run")
}

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

/// A pre-existing hooks.json mixing one foreign hook, one stale ICG entry,
/// a second foreign event array, and an unknown top-level key.
const MIXED_EXISTING: &str = r#"{
  "version": 1,
  "hooks": {
    "preToolUse": [
      {"command": "echo mine", "matcher": "Read", "timeout": 5, "failClosed": true},
      {"command": "/usr/local/bin/icg hook --harness cursor", "matcher": "Shell", "timeout": 99}
    ],
    "afterFileEdit": [{"command": "./hooks/format.sh", "timeout": 3}]
  },
  "customTopLevel": {"keep": [1, 2, 3]}
}"#;

fn write_target(dir: &Path, text: &str) -> String {
    let cursor = dir.join(".cursor");
    fs::create_dir_all(&cursor).expect("cursor dir should create");
    let path = cursor.join("hooks.json");
    fs::write(&path, text).expect("hooks.json should write");
    path.to_string_lossy().into_owned()
}

fn read(path: &str) -> serde_json::Value {
    let text = fs::read_to_string(path).expect("hooks.json should read back");
    serde_json::from_str(&text).expect("hooks.json should stay valid JSON")
}

#[test]
fn install_creates_a_missing_file_and_rerunning_changes_nothing() {
    let dir = tempdir().expect("tempdir");

    let first = installer(&["install-cursor-hooks", "--project-dir", dir.path().to_str().unwrap()]);
    assert!(first.status.success(), "stderr: {}", stderr(&first));

    let path = dir.path().join(".cursor").join("hooks.json");
    let text = fs::read_to_string(&path).expect("installer should create the file");
    let root: serde_json::Value = serde_json::from_str(&text).expect("created file is valid JSON");
    assert_eq!(root["version"], 1);

    let pre = root["hooks"]["preToolUse"].as_array().expect("array");
    let bse = root["hooks"]["beforeShellExecution"]
        .as_array()
        .expect("array");
    assert_eq!(pre.len(), 1, "exactly one ICG entry, not a duplicate");
    assert_eq!(bse.len(), 1);
    assert_eq!(pre[0]["matcher"], "Shell|Write|Edit");
    assert_eq!(bse[0]["matcher"], "");
    assert!(pre[0]["command"]
        .as_str()
        .unwrap()
        .ends_with(" hook --harness cursor"));
    assert!(bse[0]["command"]
        .as_str()
        .unwrap()
        .contains(" --event before-shell-execution"));
    assert!(pre[0].get("failClosed").is_none(), "failClosed is opt-in");

    // Acceptance criterion 1: the second run is a byte-level no-op.
    let second = installer(&["install-cursor-hooks", "--project-dir", dir.path().to_str().unwrap()]);
    assert!(second.status.success(), "stderr: {}", stderr(&second));
    assert!(
        stdout(&second).contains("already up to date"),
        "second run should report no change, got: {}",
        stdout(&second)
    );
    assert_eq!(
        fs::read_to_string(&path).expect("file still readable"),
        text,
        "second run must leave the file byte-identical"
    );
}

#[test]
fn install_preserves_unrelated_hooks_matchers_and_keys() {
    let dir = tempdir().expect("tempdir");
    let path = write_target(dir.path(), MIXED_EXISTING);

    let output = installer(&["install-cursor-hooks", "--file", &path]);
    assert!(output.status.success(), "stderr: {}", stderr(&output));

    let root = read(&path);
    let pre = root["hooks"]["preToolUse"].as_array().unwrap();

    // Acceptance criterion 2: the foreign entry keeps every key and value.
    let mine = pre
        .iter()
        .find(|entry| entry["command"] == "echo mine")
        .expect("unrelated hook must survive");
    assert_eq!(mine["matcher"], "Read");
    assert_eq!(mine["timeout"], 5);
    assert_eq!(mine["failClosed"], true);

    // The stale ICG entry was replaced in place, not duplicated.
    assert_eq!(pre.len(), 2);
    assert_eq!(
        pre.iter()
            .filter(|entry| entry["command"]
                .as_str()
                .unwrap()
                .contains("icg hook --harness cursor"))
            .count(),
        1
    );

    // Foreign event arrays and unknown top-level keys survive verbatim.
    assert_eq!(
        root["hooks"]["afterFileEdit"],
        serde_json::json!([{"command": "./hooks/format.sh", "timeout": 3}])
    );
    assert_eq!(root["customTopLevel"], serde_json::json!({"keep": [1, 2, 3]}));

    // And the merged file is itself a fixed point of the installer.
    let before = fs::read_to_string(&path).unwrap();
    let again = installer(&["install-cursor-hooks", "--file", &path]);
    assert!(again.status.success());
    assert_eq!(fs::read_to_string(&path).unwrap(), before);
}

#[test]
fn malformed_json_fails_with_a_clear_error_and_is_not_clobbered() {
    let dir = tempdir().expect("tempdir");
    let broken = "{\"version\": 1, \"hooks\": {";
    let path = write_target(dir.path(), broken);

    let output = installer(&["install-cursor-hooks", "--file", &path]);
    assert!(
        !output.status.success(),
        "malformed JSON must fail, got: {}",
        stdout(&output)
    );
    let message = stderr(&output);
    assert!(message.contains("not valid JSON"), "stderr: {message}");
    assert!(message.contains(&path), "stderr should name the file: {message}");
    assert!(message.contains("unchanged"), "stderr: {message}");
    assert_eq!(fs::read_to_string(&path).unwrap(), broken);
}

#[test]
fn unsupported_schema_versions_are_rejected_without_touching_the_file() {
    let dir = tempdir().expect("tempdir");
    let existing = r#"{"version": 2, "hooks": {}}"#;
    let path = write_target(dir.path(), existing);

    let output = installer(&["install-cursor-hooks", "--file", &path]);
    assert!(!output.status.success(), "version 2 must be refused");
    let message = stderr(&output);
    assert!(message.contains("version"), "stderr: {message}");
    assert_eq!(fs::read_to_string(&path).unwrap(), existing);
}

#[test]
fn user_flag_targets_the_home_cursor_directory() {
    let home = tempdir().expect("tempdir");

    let output = Command::new(env!("CARGO_BIN_EXE_icg"))
        .args(["install-cursor-hooks", "--user"])
        .env("HOME", home.path())
        .output()
        .expect("icg should run");
    assert!(output.status.success(), "stderr: {}", stderr(&output));

    let path = home.path().join(".cursor").join("hooks.json");
    let root = read(path.to_str().expect("temp path is utf-8"));
    assert_eq!(root["version"], 1);
    assert!(root["hooks"]["preToolUse"].is_array());
}

#[test]
fn uninstall_removes_icg_entries_and_leaves_everything_else() {
    let dir = tempdir().expect("tempdir");
    let path = write_target(dir.path(), MIXED_EXISTING);

    let output = installer(&["install-cursor-hooks", "--file", &path, "--uninstall"]);
    assert!(output.status.success(), "stderr: {}", stderr(&output));

    let root = read(&path);
    let pre = root["hooks"]["preToolUse"].as_array().unwrap();
    assert_eq!(pre.len(), 1);
    assert_eq!(pre[0]["command"], "echo mine");
    assert_eq!(
        root["hooks"]["afterFileEdit"],
        serde_json::json!([{"command": "./hooks/format.sh", "timeout": 3}])
    );
    assert_eq!(root["customTopLevel"], serde_json::json!({"keep": [1, 2, 3]}));

    // Uninstalling again is honest about finding nothing.
    let again = installer(&["install-cursor-hooks", "--file", &path, "--uninstall"]);
    assert!(again.status.success());
    assert!(stdout(&again).contains("No ICG entries found"));
}

#[test]
fn an_empty_existing_file_is_filled_in_rather_than_rejected() {
    let dir = tempdir().expect("tempdir");
    let path = write_target(dir.path(), "   \n");

    let output = installer(&["install-cursor-hooks", "--file", &path]);
    assert!(output.status.success(), "stderr: {}", stderr(&output));
    let root = read(&path);
    assert_eq!(root["version"], 1);
}

#[test]
fn fail_closed_reaches_the_written_entries() {
    let dir = tempdir().expect("tempdir");
    let path = write_target(dir.path(), "{\"version\": 1, \"hooks\": {}}");

    let output = installer(&["install-cursor-hooks", "--file", &path, "--fail-closed"]);
    assert!(output.status.success(), "stderr: {}", stderr(&output));
    let root = read(&path);
    assert_eq!(root["hooks"]["preToolUse"][0]["failClosed"], true);
    assert_eq!(root["hooks"]["beforeShellExecution"][0]["failClosed"], true);
}

#[test]
fn rule_pack_is_recorded_in_the_installed_commands() {
    let dir = tempdir().expect("tempdir");
    let path = write_target(dir.path(), "{\"version\": 1, \"hooks\": {}}");

    let packs = dir.path().join("packs");
    fs::create_dir_all(&packs).expect("packs dir");
    let output = installer(&[
        "install-cursor-hooks",
        "--file",
        &path,
        "--rule-pack",
        packs.to_str().unwrap(),
    ]);
    assert!(output.status.success(), "stderr: {}", stderr(&output));

    let root = read(&path);
    let command = root["hooks"]["preToolUse"][0]["command"]
        .as_str()
        .unwrap()
        .to_string();
    assert!(command.contains("--rule-pack"), "command: {command}");
    assert!(command.contains(&packs.to_str().unwrap().to_string()));
}
