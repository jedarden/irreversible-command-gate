//! End-to-end integration coverage for the `.github/workflows` write guard.
//!
//! `src/github_workflows.rs` unit-tests the path predicate in isolation, and
//! `src/engine.rs` unit-tests `evaluate_content` calling that predicate
//! directly. Neither exercises the actual hook front-end: stdin JSON ->
//! `Engine::input_source_from_pre_tool_use` -> `evaluate_content`, which is
//! the path `main.rs`'s `hook` subcommand uses for real Write/Edit
//! PreToolUse events. This drives the compiled `icg hook` binary the same
//! way Claude Code would over the exact fixture tables the predicate's own
//! unit tests use -- `GUARDED_PATHS` and `UNGUARDED_PATHS`, shared from
//! `src/github_workflows.rs` rather than redefined here -- and re-checks the
//! false positives through `Engine::evaluate_content` in-process.

use icg::engine::{CheckResult, ContentSource, Engine};
use icg::github_workflows::{GUARDED_PATHS, PROTECTED_REASON, UNGUARDED_PATHS};
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

/// The Write tool_input a Claude Code PreToolUse event carries. Uses the
/// camelCase spelling (`filePath`); the Edit fixture below uses the
/// snake_case alias (`file_path`) so both accepted spellings stay exercised.
fn write_input_for(file_path: &str) -> Value {
    json!({
        "filePath": file_path,
        "content": "name: ci\non: push\n",
    })
}

/// The Edit tool_input a Claude Code PreToolUse event carries.
fn edit_input_for(file_path: &str) -> Value {
    json!({
        "file_path": file_path,
        "old_string": "on: push",
        "new_string": "on: pull_request",
    })
}

/// Assert a hook response denies `file_path` and pin the full deny payload:
/// `permissionDecisionReason` must be exactly the shared `PROTECTED_REASON`
/// wording followed by the pack/pattern attribution with the targeted path
/// rendered as the `path=` segment -- the rendered form of
/// `CheckResult::Denied { reason, matched_path, .. }` the redirect-message
/// step's field contract is documented against.
fn assert_hook_deny(response: &Value, file_path: &str) {
    assert_eq!(
        response["hookSpecificOutput"]["permissionDecision"], "deny",
        "expected deny for {file_path}, got {response:?}"
    );
    let reason = response["hookSpecificOutput"]["permissionDecisionReason"]
        .as_str()
        .expect("deny reason should be a string");
    // The structured denial fields, rendered: Detection::Matched's `reason`
    // (the shared PROTECTED_REASON) verbatim, then pack/pattern unchanged,
    // then Detection::Matched's `matched_path` as the exact path the Write or
    // Edit targeted.
    assert_eq!(
        reason,
        format!("{PROTECTED_REASON} [pack=github-workflows, pattern=github-workflows-protected, path={file_path}]"),
        "deny payload should quote the protected reason and the targeted path with unchanged pack/pattern"
    );
    // The guard denies rather than rewrites, so no updatedInput channel.
    assert!(
        response["hookSpecificOutput"].get("updatedInput").is_none(),
        "deny for {file_path} must not carry updatedInput, got {response:?}"
    );
}

/// Assert a hook response allows `file_path` and emits no deny payload at
/// all -- no reason line that could carry a path= segment.
fn assert_hook_allow(response: &Value, file_path: &str) {
    assert_eq!(
        response["hookSpecificOutput"]["permissionDecision"], "allow",
        "expected allow for {file_path}, got {response:?}"
    );
    assert!(
        response["hookSpecificOutput"]
            .get("permissionDecisionReason")
            .is_none(),
        "allow for {file_path} should carry no permissionDecisionReason, got {response:?}"
    );
}

/// Every guarded fixture -- plain relative, checkout-prefixed relative,
/// `./`-prefixed, absolute, directory-only, `..`-normalizing, repeated-
/// separator, and deeply-nested spellings -- must deny a Write at the hook
/// boundary.
#[test]
fn hook_denies_write_to_every_guarded_path_form() {
    let temp = tempdir().expect("temporary directory should be created");
    let pack_path = empty_pack_path(temp.path());

    for file_path in GUARDED_PATHS {
        let denied = run_hook_for_tool(&pack_path, "Write", write_input_for(file_path));
        assert_hook_deny(&denied, file_path);
    }
}

/// Same table through Edit: each spelling must deny at the hook boundary for
/// both content tools.
#[test]
fn hook_denies_edit_to_every_guarded_path_form() {
    let temp = tempdir().expect("temporary directory should be created");
    let pack_path = empty_pack_path(temp.path());

    for file_path in GUARDED_PATHS {
        let denied = run_hook_for_tool(&pack_path, "Edit", edit_input_for(file_path));
        assert_hook_deny(&denied, file_path);
    }
}

/// Real production Write/Edit events carry absolute paths into a checkout.
/// GUARDED_PATHS proves the absolute spelling lexically; this proves an
/// absolute path that actually resolves under a real directory denies too.
#[test]
fn hook_denies_absolute_path_into_a_real_directory_for_both_tools() {
    let temp = tempdir().expect("temporary directory should be created");
    let pack_path = empty_pack_path(temp.path());
    let absolute = temp
        .path()
        .join(".github/workflows/deploy.yml")
        .to_string_lossy()
        .into_owned();

    for (tool_name, tool_input) in [
        ("Write", write_input_for(&absolute)),
        ("Edit", edit_input_for(&absolute)),
    ] {
        let denied = run_hook_for_tool(&pack_path, tool_name, tool_input);
        assert_hook_deny(&denied, &absolute);
    }
}

/// The irrevers-61a08562 false-positive table -- "workflows" substrings
/// outside `.github`, `.github/workflows*` sibling directories, and other
/// `.github` content -- must never deny a Write, and must stay silent (no
/// permissionDecisionReason for a non-match).
#[test]
fn hook_allows_write_to_every_lookalike_path() {
    let temp = tempdir().expect("temporary directory should be created");
    let pack_path = empty_pack_path(temp.path());

    for file_path in UNGUARDED_PATHS {
        let allowed = run_hook_for_tool(&pack_path, "Write", write_input_for(file_path));
        assert_hook_allow(&allowed, file_path);
    }
}

/// Same false-positive table through Edit: every lookalike must allow
/// silently for both content tools.
#[test]
fn hook_allows_edit_to_every_lookalike_path() {
    let temp = tempdir().expect("temporary directory should be created");
    let pack_path = empty_pack_path(temp.path());

    for file_path in UNGUARDED_PATHS {
        let allowed = run_hook_for_tool(&pack_path, "Edit", edit_input_for(file_path));
        assert_hook_allow(&allowed, file_path);
    }
}

/// Mid-layer re-check of the same shared false-positive table through
/// `Engine::evaluate_content` in-process (the call the hook front-end itself
/// makes after parsing stdin): every lookalike must come back Allowed for
/// both Write and Edit shapes. With the empty pack loaded, any
/// `.github/workflows` match would deny before pack dispatch, so Allowed
/// here is proof the lookalike did not trip the guard.
#[test]
fn evaluate_content_allows_every_lookalike_for_both_tools() {
    let engine = Engine::new();

    for file_path in UNGUARDED_PATHS {
        for source in [
            ContentSource::Write {
                file_path: (*file_path).to_string(),
                content: "name: ci\n".to_string(),
            },
            ContentSource::Edit {
                file_path: (*file_path).to_string(),
                old_content: "on: push".to_string(),
                new_content: "on: pull_request".to_string(),
            },
        ] {
            assert_eq!(
                engine.evaluate_content(&source),
                CheckResult::Allowed,
                "expected Allowed for {file_path}"
            );
        }
    }
}

/// The guarded counterpart at the same mid-layer: every shared path form
/// must deny through `evaluate_content` for both Write and Edit, and the
/// denial must carry the full structured payload the redirect-message step
/// consumes -- the shared `PROTECTED_REASON` wording, the exact input path
/// back as `matched_path`, and the guard's pack/pattern attribution.
#[test]
fn evaluate_content_denies_every_guarded_path_form_for_both_tools() {
    let engine = Engine::new();

    for file_path in GUARDED_PATHS {
        for source in [
            ContentSource::Write {
                file_path: (*file_path).to_string(),
                content: "name: ci\n".to_string(),
            },
            ContentSource::Edit {
                file_path: (*file_path).to_string(),
                old_content: "on: push".to_string(),
                new_content: "on: pull_request".to_string(),
            },
        ] {
            match engine.evaluate_content(&source) {
                CheckResult::Denied {
                    reason,
                    pack_id,
                    pattern_id,
                    matched_path,
                } => {
                    assert_eq!(reason, PROTECTED_REASON);
                    assert_eq!(pack_id, "github-workflows");
                    assert_eq!(pattern_id, "github-workflows-protected");
                    assert_eq!(
                        matched_path.as_deref(),
                        Some(*file_path),
                        "denial should carry matched_path {file_path:?}"
                    );
                }
                other => panic!("expected Denied for {file_path}, got {other:?}"),
            }
        }
    }
}

/// The tool_input a Codex `apply_patch` PreToolUse event carries: the patch
/// text itself, in `command`.
fn apply_patch_input_for(patch: &str) -> Value {
    json!({ "command": patch })
}

/// A multi-file Codex patch that touches `.github/workflows/**` among
/// ordinary files must deny at the hook boundary, and the `path=` segment
/// must name the guarded file -- not one of the benign files the same patch
/// also touches (here including the shared `src/workflows/` lookalike).
#[test]
fn hook_denies_multi_file_patch_touching_a_guarded_path() {
    let temp = tempdir().expect("temporary directory should be created");
    let pack_path = empty_pack_path(temp.path());

    let patch = "*** Begin Patch\n\
                 *** Add File: README.md\n\
                 +hello\n\
                 *** Update File: src/workflows/foo.yml\n\
                 @@\n\
                 -key: old\n\
                 +key: new\n\
                 *** Add File: .github/workflows/deploy.yml\n\
                 +on: push\n\
                 *** End Patch";
    let denied = run_hook_for_tool(&pack_path, "apply_patch", apply_patch_input_for(patch));
    assert_hook_deny(&denied, ".github/workflows/deploy.yml");
}

/// A multi-file patch of only unguarded paths -- benign files plus the
/// `src/workflows/` lookalike -- must allow silently at the hook boundary.
#[test]
fn hook_allows_multi_file_patch_with_only_unguarded_paths() {
    let temp = tempdir().expect("temporary directory should be created");
    let pack_path = empty_pack_path(temp.path());

    let patch = "*** Begin Patch\n\
                 *** Add File: README.md\n\
                 +hello\n\
                 *** Add File: src/workflows/foo.yml\n\
                 +steps: []\n\
                 *** Update File: .github/workflows-extra/other.yml\n\
                 @@\n\
                 -key: old\n\
                 +key: new\n\
                 *** End Patch";
    let allowed = run_hook_for_tool(&pack_path, "apply_patch", apply_patch_input_for(patch));
    assert_hook_allow(&allowed, "a multi-file unguarded patch");
}

/// A truncated patch (Begin marker present, End marker lost to a cut-off
/// generation) is parsed as far as it goes: the guarded header already seen
/// names a file the patch was about to touch, so it still denies. This pins
/// the defensive-parse contract at the hook boundary -- partial input is
/// salvaged, never a silent pass for the guarded path.
#[test]
fn hook_denies_truncated_patch_whose_parsed_header_is_guarded() {
    let temp = tempdir().expect("temporary directory should be created");
    let pack_path = empty_pack_path(temp.path());

    let truncated = "*** Begin Patch\n*** Update File: .github/workflows/ci.yml\n@@\n-on: push\n";
    let denied = run_hook_for_tool(&pack_path, "apply_patch", apply_patch_input_for(truncated));
    assert_hook_deny(&denied, ".github/workflows/ci.yml");
}

/// Input that cannot be parsed as a patch at all must neither panic nor
/// block: the hook exits successfully with a plain allow, leaving the
/// unparseable case to the guard's fail-open handling. (`run_hook_for_tool`
/// asserts the process itself exited with status 0 -- a panic would fail
/// here before the allow is even checked.)
#[test]
fn hook_fails_open_on_unparseable_patch_input() {
    let temp = tempdir().expect("temporary directory should be created");
    let pack_path = empty_pack_path(temp.path());

    let allowed = run_hook_for_tool(
        &pack_path,
        "apply_patch",
        apply_patch_input_for("totally not a patch"),
    );
    assert_hook_allow(&allowed, "an unparseable patch");
}

/// Deleting a workflow definition is a touch like editing one: a
/// `*** Delete File:` header with no hunks still denies.
#[test]
fn hook_denies_delete_only_patch_of_a_guarded_file() {
    let temp = tempdir().expect("temporary directory should be created");
    let pack_path = empty_pack_path(temp.path());

    let patch = "*** Begin Patch\n*** Delete File: .github/workflows/ci.yml\n*** End Patch";
    let denied = run_hook_for_tool(&pack_path, "apply_patch", apply_patch_input_for(patch));
    assert_hook_deny(&denied, ".github/workflows/ci.yml");
}

/// Moving a workflow file to an unguarded path must not smuggle it past the
/// guard: the move's source path is checked alongside the destination, so
/// the deny names the guarded path it abandons.
#[test]
fn hook_denies_patch_moving_a_guarded_file_out_of_workflows() {
    let temp = tempdir().expect("temporary directory should be created");
    let pack_path = empty_pack_path(temp.path());

    let patch = "*** Begin Patch\n\
                 *** Update File: .github/workflows/ci.yml\n\
                 *** Move to: ci-backup.yml\n\
                 @@\n\
                 -on: push\n\
                 +on: pull_request\n\
                 *** End Patch";
    let denied = run_hook_for_tool(&pack_path, "apply_patch", apply_patch_input_for(patch));
    assert_hook_deny(&denied, ".github/workflows/ci.yml");
}
