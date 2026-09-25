//! End-to-end hook coverage for the `sh -c` payload arm of the kubectl pack
//! (ADR-001's `irrevers-a4779a37` follow-up).
//!
//! `tests/kubectl_pack_tests.rs` pins the engine-level segmentation: a
//! shell's `-c` operand is lexed as the command line it is, so a guarded
//! verb inside the payload denies while quoted text that merely mentions
//! one stays allowed. None of that exercises the hook front-end an agent
//! actually talks to: stdin PreToolUse JSON -> Bash tool-input
//! normalization -> segmentation -> the production pack set -> response
//! rendering. This file drives the compiled `icg hook` binary over the
//! real shipped `packs/` directory -- the production configuration, so a
//! cross-pack keyword dispatch or pack ordering that shadows the kubectl
//! rules fails here too, not only in the single-pack engine suite -- and
//! pins both channels of the decision:
//!
//! - deny: a payload's mutating verb reaches the response as
//!   `permissionDecision: "deny"` carrying the pack's GitOps redirect
//!   verbatim and the `[pack=kubectl, pattern=…]` attribution;
//! - allow: quoted text that is not a command, verbs in data position,
//!   read-only verbs and the Argo Workflow carve-out come back as a bare
//!   allow with no reason payload at all.
//!
//! Supported shell boundary (ADR-001, Consequences): a `-c` command string
//! is static text, so the guard lexes and evaluates it. Shapes whose
//! commands are not text the guard can see -- a script fed on stdin or by
//! a heredoc, a script file, `eval`/indirect interpreters, anything a
//! command substitution produces at runtime -- stay unevaluated by
//! design; the `bash script.sh` / `bash --version` allow pins below hold
//! that deliberate boundary in place so it cannot quietly widen into
//! payload resolution or quietly narrow into denying the shell itself.

use icg::engine::{CheckResult, CommandSource, Engine};
use icg::rule_pack::load_pack;
use serde_json::{json, Value};
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use tempfile::tempdir;

/// Pack/pattern attribution pinned as literals, per the rule the
/// `job-cronjob-yaml` module states: an accidental rename must fail a test
/// instead of silently breaking the denial's downstream consumers.
const PACK_ID: &str = "kubectl";
const DELETE_PATTERN: &str = "kubectl-delete";
const MUTATING_PATTERN: &str = "kubectl-mutating-verb";

/// The shipped pack directory, i.e. the production configuration the hook
/// runs with in a checkout. Prefer the checkout the binary is running in
/// (cargo starts test binaries with the package root as cwd): this box's
/// shared target directory can hand back a binary built by a different
/// checkout of this repo, and the baked `CARGO_MANIFEST_DIR` then reads a
/// foreign tree -- the stale-path failure that reopened irrevers-f231c112.
/// Fall back to the baked path only when the cwd is not a checkout.
fn packs_dir() -> PathBuf {
    if let Ok(cwd) = std::env::current_dir() {
        if cwd.join("Cargo.toml").exists() {
            return cwd.join("packs");
        }
    }
    Path::new(env!("CARGO_MANIFEST_DIR")).join("packs")
}

/// Spawn `icg hook` against the real shipped packs directory, exactly as a
/// harness would for a Bash PreToolUse event, and return the response JSON.
/// Support-file paths are redirected into a temporary directory so the run
/// leaves no local state behind (the same discipline as
/// `hook_predicate_regression_tests`).
fn run_hook(command: &str) -> Value {
    let support = tempdir().expect("hook support directory should exist");
    let mut child = Command::new(env!("CARGO_BIN_EXE_icg"))
        .args(["hook", "--rule-pack"])
        .arg(packs_dir())
        .env("ICG_HEALTH_PATH", support.path().join("health.json"))
        .env("ICG_TELEMETRY_PATH", support.path().join("telemetry.json"))
        .env("ICG_DENIAL_LOG", support.path().join("denials.jsonl"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("hook process should start");

    let payload = json!({
        "tool_name": "Bash",
        "tool_input": { "command": command },
    });
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

/// The rendered deny reason for `pattern_id`: the pack's own reason
/// template verbatim (command-mode denials carry no path/file segment, so
/// `render_reason` passes the template through unchanged) followed by the
/// `[pack=, pattern=…]` attribution the shared envelope renderer appends.
/// Loaded from the shipped manifest rather than restated here, so a wording
/// edit that forgets the hook assertion fails with the diff in hand.
fn expected_deny_reason(pattern_id: &str) -> String {
    let pack = load_pack(packs_dir().join("kubectl.json"))
        .expect("shipped kubectl pack should load from packs/");
    let template = pack
        .guarded_patterns
        .iter()
        .find(|pattern| pattern.id == pattern_id)
        .unwrap_or_else(|| panic!("shipped kubectl pack should guard {pattern_id}"))
        .redirect
        .reason_template
        .trim()
        .to_string();
    format!("{template} [pack={PACK_ID}, pattern={pattern_id}]")
}

/// Assert the hook response denies `command` by `pattern_id` and pin the
/// full deny payload: the pack's GitOps redirect verbatim, the pack/pattern
/// attribution, and no input-replacement channel -- the guard denies rather
/// than rewrites.
fn assert_hook_deny(response: &Value, command: &str, pattern_id: &str) {
    let hook_output = &response["hookSpecificOutput"];
    assert_eq!(
        hook_output["permissionDecision"], "deny",
        "expected deny for {command:?}, got {response:?}"
    );
    assert_eq!(
        hook_output["permissionDecisionReason"],
        Value::String(expected_deny_reason(pattern_id)),
        "deny payload for {command:?} should quote the shipped {pattern_id} \
         redirect with unchanged attribution"
    );
    assert!(
        hook_output.get("updatedInput").is_none(),
        "deny for {command:?} must not carry updatedInput, got {response:?}"
    );
}

/// Assert the hook response allows `command` and emits no deny payload at
/// all -- no reason line that could be mistaken for a denial.
fn assert_hook_allow(response: &Value, command: &str) {
    let hook_output = &response["hookSpecificOutput"];
    assert_eq!(
        hook_output["permissionDecision"], "allow",
        "expected allow for {command:?}, got {response:?}"
    );
    assert!(
        hook_output.get("permissionDecisionReason").is_none(),
        "allow for {command:?} should carry no permissionDecisionReason, got {response:?}"
    );
}

/// A mutating verb inside a `-c` payload denies through the full hook
/// pipeline with its pattern attribution intact. The shapes mirror the
/// engine-level table in `tests/kubectl_pack_tests.rs` (flag clusters,
/// glued `-c'…'` quoting, wrapper composition, multi-segment payloads);
/// they are restated here on purpose, because the seam under test is the
/// front-end pipeline, not the segmentation the engine suite already owns.
#[test]
fn shell_payload_verbs_deny_through_the_hook_pipeline() {
    for (command, pattern_id) in [
        ("bash -c 'kubectl delete pod x'", DELETE_PATTERN),
        ("sh -c 'kubectl delete namespace scratch'", DELETE_PATTERN),
        (
            "sudo bash -c 'kubectl delete pvc data-postgres-0 -n commitgraph-production'",
            DELETE_PATTERN,
        ),
        ("timeout 30 bash -c 'kubectl delete pod x'", DELETE_PATTERN),
        (
            "bash -lc 'kubectl scale deploy/api --replicas=0 -n prod'",
            MUTATING_PATTERN,
        ),
        (
            "sh -c'kubectl apply -f deployment.yaml -n prod'",
            MUTATING_PATTERN,
        ),
        (
            "bash -c 'kubectl get pods && kubectl delete pod x'",
            DELETE_PATTERN,
        ),
    ] {
        assert_hook_deny(&run_hook(command), command, pattern_id);
    }
}

/// Nothing widens through the front-end: quoted text that is not a command
/// (`echo` prints it; it runs no kubectl), a shell name in data position,
/// read-only verbs, the Argo Workflow carve-out, and the unevaluated
/// script-file/version boundary all come back as bare allows.
#[test]
fn quoted_text_and_boundary_shapes_stay_allowed_through_the_hook_pipeline() {
    for command in [
        "bash -c 'kubectl get pods -n prod'",
        "bash -c 'kubectl rollout status deploy/api'",
        // The sanctioned Argo Workflow carve-out survives the payload.
        "bash -c 'kubectl create -f workflow.yaml -n argo-workflows'",
        // `echo` in a payload prints text; it does not run kubectl -- in
        // bare words or inside an inner quoting level.
        "bash -c 'echo kubectl delete pod x'",
        "bash -c 'echo \"kubectl apply -f deployment.yaml -n prod\"'",
        // A shell name in argument position is data, not a command boundary.
        "echo bash -c kubectl delete pod x",
        // No `-c` operand: nothing here names a payload to expand. A script
        // file's commands are not text the guard can see (ADR-001's
        // deliberate boundary), and `--version` runs no kubectl at all.
        "bash --version",
        "bash script.sh",
        "kubectl get pods -n prod",
    ] {
        assert_hook_allow(&run_hook(command), command);
    }
}

/// The same verdicts through the engine loaded with the whole shipped pack
/// set -- the in-process half of the production configuration. A pack
/// addition or dispatch reorder that shadows the kubectl rules for payload
/// shapes fails here with the structured result in hand, even if the hook
/// render above were to change shape.
#[test]
fn payload_verbs_deny_through_the_production_pack_set_in_process() {
    let mut engine = Engine::new();
    engine
        .load_packs_from_dir(packs_dir())
        .expect("shipped packs/ directory should load");

    for (command, pattern_id) in [
        ("bash -c 'kubectl delete pod x'", DELETE_PATTERN),
        ("sh -c 'kubectl delete namespace scratch'", DELETE_PATTERN),
        (
            "bash -lc 'kubectl scale deploy/api --replicas=0 -n prod'",
            MUTATING_PATTERN,
        ),
    ] {
        let result = engine.evaluate_command(&CommandSource::Hook(command.to_string()));
        assert!(
            matches!(
                &result,
                CheckResult::Denied { pack_id, pattern_id, .. }
                    if pack_id == PACK_ID && pattern_id == pattern_id
            ),
            "expected {command:?} to deny as {PACK_ID}/{pattern_id} through the \
             production pack set, got {result:?}"
        );
    }

    for command in [
        "bash -c 'kubectl get pods -n prod'",
        "bash -c 'echo kubectl delete pod x'",
        "bash -c 'echo \"kubectl apply -f deployment.yaml -n prod\"'",
        "bash script.sh",
    ] {
        assert_eq!(
            engine.evaluate_command(&CommandSource::Hook(command.to_string())),
            CheckResult::Allowed,
            "{command:?} should stay allowed through the production pack set"
        );
    }
}
