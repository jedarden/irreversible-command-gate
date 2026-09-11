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
use icg::github_workflows::{GUARDED_PATHS, UNGUARDED_PATHS};
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

/// Assert a hook response denies `file_path` and that the deny reason quotes
/// the exact path the tool call targeted, with the guard's pack/pattern
/// attribution unchanged.
fn assert_hook_deny(response: &Value, file_path: &str) {
    assert_eq!(
        response["hookSpecificOutput"]["permissionDecision"], "deny",
        "expected deny for {file_path}, got {response:?}"
    );
    let reason = response["hookSpecificOutput"]["permissionDecisionReason"]
        .as_str()
        .expect("deny reason should be a string");
    // The protected reason (Detection::Matched's `reason`) is carried verbatim.
    assert!(
        reason.contains("must not be modified by an automated write/edit"),
        "deny reason should carry the protected reason, got: {reason}"
    );
    // ...and so is the exact path the Write or Edit targeted
    // (Detection::Matched's `matched_path`), rendered as the `path=` segment
    // with pack/pattern unchanged.
    assert!(
        reason.contains(&format!(
            "[pack=github-workflows, pattern=github-workflows-protected, path={file_path}]"
        )),
        "deny reason should quote the targeted path with unchanged pack/pattern, got: {reason}"
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
/// must deny through `evaluate_content` for both Write and Edit, carrying
/// the exact input path back as `matched_path` -- the contract the hook
/// boundary's `path=` reason segment above is rendered from.
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
                    pack_id,
                    pattern_id,
                    matched_path,
                    ..
                } => {
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
