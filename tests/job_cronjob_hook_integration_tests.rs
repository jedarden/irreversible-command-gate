//! End-to-end integration coverage for the `kind: Job` / `kind: CronJob`
//! write guard.
//!
//! `src/job_cronjob_yaml.rs` unit-tests the content predicate in isolation,
//! and `src/engine.rs` unit-tests `evaluate_content` calling that predicate
//! directly. Neither exercises the actual hook front-end: stdin JSON ->
//! `Engine::input_source_from_pre_tool_use` -> `evaluate_content`, which is
//! the path `main.rs`'s `hook` subcommand uses for real Write/Edit PreToolUse
//! events. This drives the compiled `icg hook` binary the same way Claude
//! Code or Codex would over the exact fixture tables the predicate's own
//! unit tests use -- `GUARDED_CONTENTS` and `UNGUARDED_CONTENTS`, shared from
//! `src/job_cronjob_yaml.rs` rather than redefined here -- and re-checks the
//! same tables through `Engine::evaluate_content` in-process.

use icg::engine::{CheckResult, ContentSource, Engine};
use icg::job_cronjob_yaml::{
    BLOCKED_REASON, GUARDED_CONTENTS, PACK_ID, PATTERN_ID, UNGUARDED_CONTENTS,
};
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
        "id": "job-cronjob-hook-integration-test",
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
fn write_input_for(file_path: &str, content: &str) -> Value {
    json!({
        "filePath": file_path,
        "content": content,
    })
}

/// The Edit tool_input a Claude Code PreToolUse event carries. The replaced
/// text is deliberately benign so any denial can only come from the new
/// content -- the guard judges what a write introduces, never what it
/// replaces.
fn edit_input_for(file_path: &str, new_content: &str) -> Value {
    json!({
        "file_path": file_path,
        "old_string": "metadata:\n  name: app\n",
        "new_string": new_content,
    })
}

/// Assert a hook response denies the write to `file_path` and pin the full
/// deny payload: `permissionDecisionReason` must be exactly the shared
/// `BLOCKED_REASON` wording followed by the pack/pattern attribution with
/// the targeted path rendered as the `file=` segment -- the rendered form of
/// `CheckResult::Denied { reason, matched_path: None, .. }` whose `file=`
/// fallback is documented on `denial_path_segment` in `src/main.rs`.
fn assert_hook_deny(response: &Value, file_path: &str) {
    assert_eq!(
        response["hookSpecificOutput"]["permissionDecision"], "deny",
        "expected deny for {file_path}, got {response:?}"
    );
    let reason = response["hookSpecificOutput"]["permissionDecisionReason"]
        .as_str()
        .expect("deny reason should be a string");
    assert_eq!(
        reason,
        format!("{BLOCKED_REASON} [pack={PACK_ID}, pattern={PATTERN_ID}, file={file_path}]"),
        "deny payload should quote the redirect reason and the targeted path with unchanged pack/pattern"
    );
    // The guard denies rather than rewrites, so no updatedInput channel.
    assert!(
        response["hookSpecificOutput"].get("updatedInput").is_none(),
        "deny for {file_path} must not carry updatedInput, got {response:?}"
    );
}

/// Assert a hook response allows the write to `file_path` and emits no deny
/// payload at all -- no reason line that could carry a file= segment.
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

/// Every guarded spelling -- plain Job and CronJob manifests, case variants
/// on key and value, tabs, doubled and extra spaces, quoted values, trailing
/// comments, CRLF line endings, later documents of a multi-document stream,
/// and an inline document-start -- must deny a Write at the hook boundary,
/// against an empty rule pack: the guard is built in, not pack-loaded.
#[test]
fn hook_denies_write_of_every_guarded_content() {
    let temp = tempdir().expect("temporary directory should be created");
    let pack_path = empty_pack_path(temp.path());

    for content in GUARDED_CONTENTS {
        let denied = run_hook_for_tool(
            &pack_path,
            "Write",
            write_input_for("k8s/job.yaml", content),
        );
        assert_hook_deny(&denied, "k8s/job.yaml");
    }
}

/// Same table through Edit: each spelling must deny at the hook boundary when
/// it is what the edit introduces.
#[test]
fn hook_denies_edit_introducing_every_guarded_content() {
    let temp = tempdir().expect("temporary directory should be created");
    let pack_path = empty_pack_path(temp.path());

    for content in GUARDED_CONTENTS {
        let denied = run_hook_for_tool(
            &pack_path,
            "Edit",
            edit_input_for("k8s/cronjob.yml", content),
        );
        assert_hook_deny(&denied, "k8s/cronjob.yml");
    }
}

/// The irrevers-b559d088 false-positive table -- longer scalars sharing the
/// prefix, other workload kinds, comments, unrelated fields whose value
/// merely contains the phrase, block-scalar string literals, and malformed
/// or partial YAML -- must never deny a Write of a `.yaml` target, and must
/// stay silent (no permissionDecisionReason for a non-match).
#[test]
fn hook_allows_write_of_every_false_positive_content() {
    let temp = tempdir().expect("temporary directory should be created");
    let pack_path = empty_pack_path(temp.path());

    for content in UNGUARDED_CONTENTS {
        let allowed = run_hook_for_tool(
            &pack_path,
            "Write",
            write_input_for("k8s/app.yaml", content),
        );
        assert_hook_allow(&allowed, "k8s/app.yaml");
    }
}

/// Same false-positive table through Edit: every lookalike must allow
/// silently for both content tools.
#[test]
fn hook_allows_edit_to_every_false_positive_content() {
    let temp = tempdir().expect("temporary directory should be created");
    let pack_path = empty_pack_path(temp.path());

    for content in UNGUARDED_CONTENTS {
        let allowed =
            run_hook_for_tool(&pack_path, "Edit", edit_input_for("k8s/app.yaml", content));
        assert_hook_allow(&allowed, "k8s/app.yaml");
    }
}

/// The guard is scoped to `.yaml`/`.yml` targets at the hook boundary too:
/// Job content aimed at any other path stays writable.
#[test]
fn hook_allows_job_content_on_non_yaml_paths() {
    let temp = tempdir().expect("temporary directory should be created");
    let pack_path = empty_pack_path(temp.path());

    for file_path in ["docs/jobs.md", "src/job.rs", "README", "job.yaml.j2"] {
        let allowed = run_hook_for_tool(
            &pack_path,
            "Write",
            write_input_for(file_path, "kind: Job\n"),
        );
        assert_hook_allow(&allowed, file_path);
    }
}

/// An Edit that *removes* a Job must stay allowed at the hook boundary: only
/// the replacement text is judged, so fixing an existing Job (old text
/// matching, new text a Deployment) goes through.
#[test]
fn hook_allows_edit_that_removes_a_job() {
    let temp = tempdir().expect("temporary directory should be created");
    let pack_path = empty_pack_path(temp.path());

    let allowed = run_hook_for_tool(
        &pack_path,
        "Edit",
        json!({
            "file_path": "k8s/job.yaml",
            "old_string": "apiVersion: batch/v1\nkind: Job\n",
            "new_string": "apiVersion: apps/v1\nkind: Deployment\n",
        }),
    );
    assert_hook_allow(&allowed, "k8s/job.yaml");
}

/// The tool_input a Codex `apply_patch` PreToolUse event carries: the patch
/// text itself, in `command`.
fn apply_patch_input_for(patch: &str) -> Value {
    json!({ "command": patch })
}

/// A multi-file Codex patch that introduces a Job manifest among ordinary
/// files must deny at the hook boundary -- patch breadth cannot hide a
/// guarded document, because every normalized file runs through the same
/// detection call.
#[test]
fn hook_denies_multi_file_patch_introducing_a_job_among_benign_files() {
    let temp = tempdir().expect("temporary directory should be created");
    let pack_path = empty_pack_path(temp.path());

    let patch = "*** Begin Patch\n\
                 *** Add File: README.md\n\
                 +hello\n\
                 *** Add File: k8s/migrate.yaml\n\
                 +apiVersion: batch/v1\n\
                 +kind: Job\n\
                 +metadata:\n\
                 +  name: migrate\n\
                 *** Update File: deploy/app.yaml\n\
                 @@\n\
                 -image: app:1.2.3\n\
                 +image: app:1.2.4\n\
                 *** End Patch";
    let denied = run_hook_for_tool(&pack_path, "apply_patch", apply_patch_input_for(patch));
    // The rendered `file=` segment carries the whole patch's file list (the
    // batch context `render_hook_response` is handed), not just the guarded
    // file -- the denial itself names the guard, not one path.
    assert_hook_deny(&denied, "README.md,k8s/migrate.yaml,deploy/app.yaml");
}

/// A multi-file patch whose YAML files carry only false-positive shapes -- a
/// ConfigMap embedding a script that mentions a Job, a Deployment manifest --
/// must allow silently at the hook boundary.
#[test]
fn hook_allows_multi_file_patch_with_only_false_positive_yaml() {
    let temp = tempdir().expect("temporary directory should be created");
    let pack_path = empty_pack_path(temp.path());

    let patch = "*** Begin Patch\n\
                 *** Add File: README.md\n\
                 +hello\n\
                 *** Add File: k8s/config.yaml\n\
                 +data:\n\
                 +  seed.sh: |\n\
                 +    kind: Job\n\
                 *** Add File: deploy/app.yaml\n\
                 +kind: Deployment\n\
                 *** End Patch";
    let allowed = run_hook_for_tool(&pack_path, "apply_patch", apply_patch_input_for(patch));
    assert_hook_allow(&allowed, "a multi-file false-positive patch");
}

/// A truncated patch (Begin marker present, End marker lost to a cut-off
/// generation) is parsed as far as it goes: the guarded content already seen
/// still denies. This pins the defensive-parse contract at the hook boundary
/// -- partial input is salvaged, never a silent pass for the guarded content.
#[test]
fn hook_denies_truncated_patch_whose_parsed_content_introduces_a_job() {
    let temp = tempdir().expect("temporary directory should be created");
    let pack_path = empty_pack_path(temp.path());

    let truncated = "*** Begin Patch\n\
                     *** Add File: k8s/nightly.yaml\n\
                     +kind: CronJob\n";
    let denied = run_hook_for_tool(&pack_path, "apply_patch", apply_patch_input_for(truncated));
    assert_hook_deny(&denied, "k8s/nightly.yaml");
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

// --- in-process (mid-layer) coverage ---

/// Pin the full structured Job/CronJob denial: the shared redirect wording,
/// the guard's pack/pattern attribution, and no matched path -- that slot
/// names a path, and this guard denied on content, so the rendered reason
/// falls back to `file=<target>`.
fn assert_job_cronjob_denial(result: CheckResult) {
    match result {
        CheckResult::Denied {
            reason,
            pack_id,
            pattern_id,
            matched_path,
        } => {
            assert_eq!(reason, BLOCKED_REASON);
            assert_eq!(pack_id, PACK_ID);
            assert_eq!(pattern_id, PATTERN_ID);
            assert_eq!(
                matched_path, None,
                "a content denial carries no matched_path"
            );
        }
        other => panic!("expected a Job/CronJob denial, got {other:?}"),
    }
}

/// Mid-layer re-check of both shared tables through `Engine::evaluate_content`
/// in-process (the call the hook front-end itself makes after parsing stdin):
/// every guarded spelling denies and every false positive stays allowed, for
/// both Write and Edit shapes, with the empty pack loaded -- proving the
/// guard is built in rather than pack-dispatched.
#[test]
fn evaluate_content_splits_both_shared_tables_for_both_tools_with_an_empty_pack() {
    let engine = Engine::new();

    for content in GUARDED_CONTENTS {
        for source in [
            ContentSource::Write {
                file_path: "k8s/job.yaml".to_string(),
                content: (*content).to_string(),
            },
            ContentSource::Edit {
                file_path: "k8s/job.yaml".to_string(),
                old_content: "metadata:\n  name: app\n".to_string(),
                new_content: (*content).to_string(),
            },
        ] {
            assert_job_cronjob_denial(engine.evaluate_content(&source));
        }
    }

    for content in UNGUARDED_CONTENTS {
        for source in [
            ContentSource::Write {
                file_path: "k8s/app.yaml".to_string(),
                content: (*content).to_string(),
            },
            ContentSource::Edit {
                file_path: "k8s/app.yaml".to_string(),
                old_content: "kind: Job".to_string(),
                new_content: (*content).to_string(),
            },
        ] {
            assert_eq!(
                engine.evaluate_content(&source),
                CheckResult::Allowed,
                "expected Allowed for {content:?}"
            );
        }
    }
}

/// The multi-file batch shape at the mid-layer: `evaluate_content_batch`
/// runs every normalized patch file through the same detection, so a Job
/// manifest added in any position of the patch denies.
#[test]
fn evaluate_content_batch_denies_a_job_in_any_patch_position() {
    let engine = Engine::new();

    // The guarded file first, in the middle, and last across the batch --
    // position must not matter.
    for patch in [
        "*** Begin Patch\n\
         *** Add File: k8s/first.yaml\n\
         +kind: Job\n\
         *** Add File: README.md\n\
         +hello\n\
         *** End Patch",
        "*** Begin Patch\n\
         *** Add File: README.md\n\
         +hello\n\
         *** Add File: k8s/middle.yaml\n\
         +kind: CronJob\n\
         *** Add File: deploy/app.yaml\n\
         +kind: Deployment\n\
         *** End Patch",
        "*** Begin Patch\n\
         *** Add File: README.md\n\
         +hello\n\
         *** Add File: k8s/last.yaml\n\
         +kind: Job\n\
         *** End Patch",
    ] {
        let sources = icg::engine::normalize_apply_patch(patch).expect("patch should normalize");
        assert!(
            sources.len() > 1,
            "each fixture should be a multi-file patch"
        );
        assert_job_cronjob_denial(engine.evaluate_content_batch(&sources));
    }
}
