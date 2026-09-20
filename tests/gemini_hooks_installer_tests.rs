//! End-to-end tests for `icg install-gemini-hooks` (src/gemini_hooks.rs).
//!
//! The unit tests inside the module pin the merge function on strings, and
//! the process-boundary suite in `gemini_hook_tests.rs` pins what the
//! installed hook does once Gemini CLI dispatches it. These drive the
//! shipped binary instead, so the acceptance criteria hold for the command
//! an operator actually types: a fresh install into a missing, empty, or
//! populated `settings.json`; idempotence (a re-run neither duplicates ICG
//! entries nor rewrites bytes); preservation of unrelated hooks, other
//! event arrays, and top-level settings; an uninstall that removes
//! ICG-owned entries and drops only the groups it empties; a clear failure
//! — never a rewrite — on malformed JSON or a wrong shape; the one-shot
//! `<target>.icg-backup`; and the `--user` / `--project-dir` / `--file` /
//! current-directory path derivation. The matcher's anchoring is pinned
//! here too, because the written config is where an unanchored regex would
//! start routing MCP and read-only tools into the gate.

use std::fs;
use std::path::Path;
use std::process::{Command, Output};

use serde_json::Value;
use tempfile::tempdir;

/// The anchored three-tool alternation the installer must write
/// (src/gemini_hooks.rs `BEFORE_TOOL_MATCHER`, contract §6.4). Pinned as a
/// literal so a widening of the matcher fails here before it ships.
const MATCHER: &str = "^(run_shell_command|write_file|replace)$";

/// A pre-existing settings.json mixing one foreign hook sharing a group
/// with a stale ICG entry, a foreign-only group, a second event array, and
/// unrelated top-level settings.
const MIXED_EXISTING: &str = r#"{
  "model": "gemini-2.5-pro",
  "hooks": {
    "BeforeTool": [
      {
        "matcher": "run_shell_command",
        "hooks": [
          {"type": "command", "command": "echo mine", "timeout": 500},
          {"type": "command", "command": "/usr/local/bin/icg hook --harness gemini-cli", "timeout": 99}
        ]
      },
      {
        "matcher": "write_file",
        "hooks": [{"type": "command", "command": "./hooks/audit.sh"}]
      }
    ],
    "BeforeAgent": [
      {"matcher": "", "hooks": [{"type": "command", "command": "./hooks/prompt.sh"}]}
    ]
  },
  "theme": "auto"
}"#;

/// The uninstall landscape: a mixed group (foreign + ICG), an ICG-only
/// group, a foreign-only group, and a group with no `hooks` array at all —
/// plus another event array and top-level settings that must survive.
const UNINSTALL_EXISTING: &str = r#"{
  "model": "gemini-2.5-pro",
  "hooks": {
    "BeforeTool": [
      {
        "matcher": "run_shell_command",
        "hooks": [
          {"type": "command", "command": "echo mine", "timeout": 500},
          {"type": "command", "command": "icg hook --harness gemini-cli"}
        ]
      },
      {
        "matcher": "write_file",
        "hooks": [{"type": "command", "command": "/usr/local/bin/icg hook --harness gemini-cli"}]
      },
      {
        "matcher": "replace",
        "hooks": [{"type": "command", "command": "./hooks/format.sh", "timeout": 3}]
      },
      {"matcher": "unparsed"}
    ],
    "AfterTool": [{"matcher": "", "hooks": [{"type": "command", "command": "./hooks/audit.sh"}]}]
  },
  "theme": "auto"
}"#;

fn installer(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_icg"))
        .args(args)
        .output()
        .expect("icg should run")
}

fn installer_in(dir: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_icg"))
        .current_dir(dir)
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

fn write_target(dir: &Path, text: &str) -> String {
    let gemini = dir.join(".gemini");
    fs::create_dir_all(&gemini).expect("gemini dir should create");
    let path = gemini.join("settings.json");
    fs::write(&path, text).expect("settings.json should write");
    path.to_string_lossy().into_owned()
}

fn read(path: &str) -> Value {
    let text = fs::read_to_string(path).expect("settings.json should read back");
    serde_json::from_str(&text).expect("settings.json should stay valid JSON")
}

/// Every hook-definition command in the `BeforeTool` array that invokes the
/// icg hook front end for the gemini-cli harness — the same content
/// predicate the installer judges ownership by, applied here to count what
/// actually landed in the file.
fn icg_commands(root: &Value) -> Vec<String> {
    let mut commands = Vec::new();
    for group in root["hooks"]["BeforeTool"].as_array().into_iter().flatten() {
        for entry in group["hooks"].as_array().into_iter().flatten() {
            if let Some(command) = entry["command"].as_str() {
                if command.contains("hook --harness gemini-cli") {
                    commands.push(command.to_string());
                }
            }
        }
    }
    commands
}

#[test]
fn install_creates_a_missing_file_and_rerunning_changes_nothing() {
    let dir = tempdir().expect("tempdir");

    let first = installer(&[
        "install-gemini-hooks",
        "--project-dir",
        dir.path().to_str().unwrap(),
    ]);
    assert!(first.status.success(), "stderr: {}", stderr(&first));

    // --project-dir derives <dir>/.gemini/settings.json and creates it.
    let path = dir.path().join(".gemini").join("settings.json");
    let text = fs::read_to_string(&path).expect("installer should create the file");
    let root: Value = serde_json::from_str(&text).expect("created file is valid JSON");
    assert_eq!(
        root.as_object().unwrap().keys().collect::<Vec<_>>(),
        vec!["hooks"],
        "a fresh document carries exactly the hooks key"
    );

    let groups = root["hooks"]["BeforeTool"].as_array().expect("array");
    assert_eq!(groups.len(), 1, "exactly one ICG group, not a duplicate");
    assert_eq!(groups[0]["matcher"], MATCHER);
    let entries = groups[0]["hooks"].as_array().expect("array");
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0]["type"], "command");
    assert_eq!(entries[0]["timeout"], 10_000);
    assert!(entries[0]["command"]
        .as_str()
        .unwrap()
        .ends_with(" hook --harness gemini-cli"));
    assert_eq!(
        icg_commands(&root).len(),
        1,
        "one ICG entry in the whole file"
    );
    assert!(
        stdout(&first).contains("ICG entries: 0 replaced, 1 installed."),
        "a fresh install reports nothing replaced, got: {}",
        stdout(&first)
    );

    // A fresh install has no prior state worth a backup.
    assert!(
        !dir.path().join(".gemini/settings.json.icg-backup").exists(),
        "no backup may be written when there was nothing to back up"
    );

    // Acceptance criterion: the second run is a byte-level no-op.
    let second = installer(&[
        "install-gemini-hooks",
        "--project-dir",
        dir.path().to_str().unwrap(),
    ]);
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
    let after: Value =
        serde_json::from_str(&fs::read_to_string(&path).expect("file still readable"))
            .expect("file stays valid JSON");
    assert_eq!(icg_commands(&after).len(), 1, "still exactly one entry");
}

#[test]
fn an_empty_existing_file_is_filled_in_rather_than_rejected() {
    let dir = tempdir().expect("tempdir");
    let path = write_target(dir.path(), "   \n");

    let output = installer(&["install-gemini-hooks", "--file", &path]);
    assert!(output.status.success(), "stderr: {}", stderr(&output));

    let root = read(&path);
    assert!(root["hooks"]["BeforeTool"].is_array());
    assert_eq!(icg_commands(&root).len(), 1);
    // Whitespace is not a prior state: nothing to back up.
    assert!(
        !Path::new(&format!("{path}.icg-backup")).exists(),
        "an empty file must not produce a backup"
    );
}

#[test]
fn install_preserves_unrelated_hooks_events_and_settings() {
    let dir = tempdir().expect("tempdir");
    let path = write_target(dir.path(), MIXED_EXISTING);

    let output = installer(&["install-gemini-hooks", "--file", &path]);
    assert!(output.status.success(), "stderr: {}", stderr(&output));

    let root = read(&path);
    let groups = root["hooks"]["BeforeTool"].as_array().unwrap();

    // The stale ICG entry was replaced, not duplicated: one fresh group
    // joins the two pre-existing ones, and only one ICG command remains.
    assert_eq!(groups.len(), 3);
    assert_eq!(icg_commands(&root).len(), 1);

    // The foreign entry sharing the stale ICG entry's group survives
    // verbatim, keeping every schema field.
    let shared = groups
        .iter()
        .find(|group| group["matcher"] == "run_shell_command")
        .expect("the shared group must survive");
    assert_eq!(
        shared["hooks"],
        serde_json::json!([{"type": "command", "command": "echo mine", "timeout": 500}])
    );

    // The untouched foreign group, the other event array, and the
    // top-level settings keep their values.
    assert_eq!(
        root["hooks"]["BeforeTool"]
            .as_array()
            .unwrap()
            .iter()
            .find(|group| group["matcher"] == "write_file")
            .expect("the foreign-only group must survive")["hooks"],
        serde_json::json!([{"type": "command", "command": "./hooks/audit.sh"}])
    );
    assert_eq!(
        root["hooks"]["BeforeAgent"],
        serde_json::json!([{"matcher": "", "hooks": [{"type": "command", "command": "./hooks/prompt.sh"}]}])
    );
    assert_eq!(root["model"], "gemini-2.5-pro");
    assert_eq!(root["theme"], "auto");

    // And the merged file is itself a fixed point of the installer.
    let before = fs::read_to_string(&path).unwrap();
    let again = installer(&["install-gemini-hooks", "--file", &path]);
    assert!(again.status.success(), "stderr: {}", stderr(&again));
    assert!(stdout(&again).contains("already up to date"));
    assert_eq!(fs::read_to_string(&path).unwrap(), before);
}

#[test]
fn malformed_json_fails_with_a_clear_error_and_is_not_clobbered() {
    let dir = tempdir().expect("tempdir");
    let broken = "{\"model\": \"gemini-2.5-pro\", \"hooks\": {";
    let path = write_target(dir.path(), broken);

    let output = installer(&["install-gemini-hooks", "--file", &path]);
    assert!(
        !output.status.success(),
        "malformed JSON must fail, got: {}",
        stdout(&output)
    );
    let message = stderr(&output);
    assert!(message.contains("not valid JSON"), "stderr: {message}");
    assert!(
        message.contains(&path),
        "stderr should name the file: {message}"
    );
    assert!(
        message.contains("left unchanged"),
        "stderr should promise the file is untouched: {message}"
    );
    assert_eq!(
        fs::read_to_string(&path).unwrap(),
        broken,
        "the failed merge must not rewrite the file"
    );
    assert!(
        !Path::new(&format!("{path}.icg-backup")).exists(),
        "a failed merge must not leave a backup behind either"
    );
}

#[test]
fn wrong_shapes_are_rejected_without_touching_the_file() {
    // (document, the fragment the error must name) — each is JSON that
    // parses but is not the settings shape the schema documents.
    let cases = [
        (
            r#"{"hooks": {"BeforeTool": "not-an-array"}}"#,
            "must be a JSON array",
        ),
        (r#"{"hooks": "not-an-object"}"#, "must be a JSON object"),
        ("[1, 2, 3]", "must be a JSON object"),
    ];

    for (document, fragment) in cases {
        let dir = tempdir().expect("tempdir");
        let path = write_target(dir.path(), document);

        let output = installer(&["install-gemini-hooks", "--file", &path]);
        assert!(
            !output.status.success(),
            "{document} must be refused, got: {}",
            stdout(&output)
        );
        let message = stderr(&output);
        assert!(
            message.contains(fragment),
            "stderr should say {fragment:?}, got: {message}"
        );
        assert!(
            message.contains("left unchanged"),
            "stderr should promise the file is untouched: {message}"
        );
        assert_eq!(fs::read_to_string(&path).unwrap(), document);
    }
}

#[test]
fn uninstall_removes_icg_entries_and_drops_only_emptied_groups() {
    let dir = tempdir().expect("tempdir");
    let path = write_target(dir.path(), UNINSTALL_EXISTING);

    let output = installer(&["install-gemini-hooks", "--file", &path, "--uninstall"]);
    assert!(output.status.success(), "stderr: {}", stderr(&output));

    let root = read(&path);
    let groups = root["hooks"]["BeforeTool"].as_array().unwrap();

    // The mixed group keeps its foreign entry and loses only ours; the
    // ICG-only group goes with its entry; the foreign-only group and the
    // group without a `hooks` array pass through untouched.
    assert_eq!(groups.len(), 3, "only the emptied group is dropped");
    assert_eq!(icg_commands(&root).len(), 0, "no ICG entry remains");
    assert_eq!(
        groups[0],
        serde_json::json!({
            "matcher": "run_shell_command",
            "hooks": [{"type": "command", "command": "echo mine", "timeout": 500}]
        }),
        "the foreign entry sharing a group with ours survives"
    );
    assert_eq!(groups[1]["matcher"], "replace");
    assert_eq!(groups[2], serde_json::json!({"matcher": "unparsed"}));

    // Other events and top-level settings are untouched, and no
    // BeforeTool key was added or invented.
    assert_eq!(
        root["hooks"]["AfterTool"],
        serde_json::json!([{"matcher": "", "hooks": [{"type": "command", "command": "./hooks/audit.sh"}]}])
    );
    assert_eq!(root["model"], "gemini-2.5-pro");
    assert_eq!(root["theme"], "auto");

    // Uninstalling again is honest about finding nothing — and, being a
    // no-op, does not rewrite the file.
    let settled = fs::read_to_string(&path).unwrap();
    let again = installer(&["install-gemini-hooks", "--file", &path, "--uninstall"]);
    assert!(again.status.success(), "stderr: {}", stderr(&again));
    assert!(
        stdout(&again).contains("No ICG entries found"),
        "second uninstall should report nothing removed, got: {}",
        stdout(&again)
    );
    assert_eq!(fs::read_to_string(&path).unwrap(), settled);
}

#[test]
fn uninstall_from_a_file_without_before_tool_is_a_byte_level_no_op() {
    let dir = tempdir().expect("tempdir");
    let existing = r#"{"model": "gemini-2.5-pro", "hooks": {"AfterTool": []}}"#;
    let path = write_target(dir.path(), existing);

    let output = installer(&["install-gemini-hooks", "--file", &path, "--uninstall"]);
    assert!(output.status.success(), "stderr: {}", stderr(&output));
    assert!(stdout(&output).contains("No ICG entries found"));
    assert_eq!(
        fs::read_to_string(&path).unwrap(),
        existing,
        "an uninstall never adds keys or normalizes a file it has nothing \
         to remove from"
    );
}

#[test]
fn the_backup_holds_the_pre_install_state_and_survives_reruns() {
    let dir = tempdir().expect("tempdir");
    let path = write_target(dir.path(), MIXED_EXISTING);
    let backup = format!("{path}.icg-backup");

    let first = installer(&["install-gemini-hooks", "--file", &path]);
    assert!(first.status.success(), "stderr: {}", stderr(&first));
    assert!(
        fs::read_to_string(&backup).expect("backup should exist") == MIXED_EXISTING,
        "the backup holds the pre-install state"
    );

    // The immediate no-op re-run leaves the backup alone.
    let second = installer(&["install-gemini-hooks", "--file", &path]);
    assert!(second.status.success(), "stderr: {}", stderr(&second));
    assert!(stdout(&second).contains("already up to date"));
    assert_eq!(fs::read_to_string(&backup).unwrap(), MIXED_EXISTING);

    // A later hand edit makes the next run a real write again; the backup
    // must still hold the PRE-INSTALLER state, not the intermediate one —
    // it is written only when absent, so it is never clobbered.
    let mut hand_edited = read(&path);
    hand_edited
        .as_object_mut()
        .unwrap()
        .insert("handEdited".to_string(), serde_json::json!(true));
    fs::write(&path, serde_json::to_string_pretty(&hand_edited).unwrap())
        .expect("hand edit should write");

    let third = installer(&["install-gemini-hooks", "--file", &path]);
    assert!(third.status.success(), "stderr: {}", stderr(&third));
    assert!(
        stdout(&third).contains("(updated)"),
        "the third run really rewrote the file, got: {}",
        stdout(&third)
    );
    assert_eq!(
        fs::read_to_string(&backup).unwrap(),
        MIXED_EXISTING,
        "a rerun must not clobber the one-time backup"
    );
    let root = read(&path);
    assert_eq!(root["handEdited"], true, "the hand edit survives the merge");
    assert_eq!(icg_commands(&root).len(), 1);
}

#[test]
fn user_flag_targets_the_home_gemini_directory() {
    let home = tempdir().expect("tempdir");

    let output = Command::new(env!("CARGO_BIN_EXE_icg"))
        .args(["install-gemini-hooks", "--user"])
        .env("HOME", home.path())
        .output()
        .expect("icg should run");
    assert!(output.status.success(), "stderr: {}", stderr(&output));

    let path = home.path().join(".gemini").join("settings.json");
    let root = read(path.to_str().expect("temp path is utf-8"));
    assert_eq!(icg_commands(&root).len(), 1);
}

#[test]
fn the_default_target_is_the_current_directorys_gemini_file() {
    let dir = tempdir().expect("tempdir");

    let output = installer_in(dir.path(), &["install-gemini-hooks"]);
    assert!(output.status.success(), "stderr: {}", stderr(&output));

    let path = dir.path().join(".gemini").join("settings.json");
    assert!(
        path.exists(),
        "cwd derivation must reach .gemini/settings.json"
    );
    let root = read(path.to_str().expect("temp path is utf-8"));
    assert_eq!(icg_commands(&root).len(), 1);
}

/// The matcher is pinned twice: as the exact string in the written file —
/// so any edit to `BEFORE_TOOL_MATCHER` fails here — and as behavior, by
/// compiling the written text and proving the anchors are what keep MCP
/// tools (`mcp_<server>_<tool>`), read-only tools, and near-miss names
/// from ever routing to ICG through the config itself.
#[test]
fn the_installed_matcher_is_the_anchored_three_tool_alternation() {
    let dir = tempdir().expect("tempdir");
    let path = write_target(dir.path(), MIXED_EXISTING);

    let output = installer(&["install-gemini-hooks", "--file", &path]);
    assert!(output.status.success(), "stderr: {}", stderr(&output));

    let root = read(&path);
    let matcher = root["hooks"]["BeforeTool"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|group| group["matcher"].as_str())
        .find(|matcher| matcher.starts_with('^'))
        .expect("the anchored matcher is written");
    assert_eq!(matcher, MATCHER);

    let pattern = regex::Regex::new(matcher).expect("the written matcher compiles");
    for name in ["run_shell_command", "write_file", "replace"] {
        assert!(pattern.is_match(name), "{name} must match");
    }
    for name in [
        "mcp_fs_write_file",
        "mcp__fs__write_file",
        "read_file",
        "read_many_files",
        "glob",
        "grep",
        "search_file_content",
        "ls",
        "google_web_search",
        "safe_write_file",
        "replace_all",
        "run_shell_command_extra",
        "",
    ] {
        assert!(
            !pattern.is_match(name),
            "{name:?} must never route to ICG through the installed matcher"
        );
    }
}

#[test]
fn rule_pack_is_recorded_in_the_installed_command() {
    let dir = tempdir().expect("tempdir");
    let path = write_target(dir.path(), "{}");

    let packs = dir.path().join("packs");
    fs::create_dir_all(&packs).expect("packs dir");
    let output = installer(&[
        "install-gemini-hooks",
        "--file",
        &path,
        "--rule-pack",
        packs.to_str().unwrap(),
    ]);
    assert!(output.status.success(), "stderr: {}", stderr(&output));

    let root = read(&path);
    let commands = icg_commands(&root);
    assert_eq!(commands.len(), 1);
    assert!(
        commands[0].contains("--rule-pack"),
        "command: {}",
        commands[0]
    );
    assert!(
        commands[0].contains(&packs.to_str().unwrap().to_string()),
        "command: {}",
        commands[0]
    );
}
