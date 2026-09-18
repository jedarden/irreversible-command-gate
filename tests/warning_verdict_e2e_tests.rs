//! End-to-end tests for the warning verdict semantics.
//!
//! `additional_context` is the one redirect channel that must never block:
//! the rule cannot decide reliably enough to stop the call, so the guard's
//! entire effect is the text it attaches to an allowed response. The fixture
//! pack beside this test (`warning-verdict/warning-pack.json`) carries that
//! channel in both input modes -- a command rule and a content rule -- and
//! every assertion below runs the real binary, `icg hook` for the harness
//! adapter and `icg check` for the operator CLI, so what is locked is the
//! redirect reason's survival into each front end's own rendering, not an
//! engine value a front end could drop.

use serde_json::{json, Value};
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use tempfile::tempdir;

const ROOT: &str = env!("CARGO_MANIFEST_DIR");

/// The command-mode warning case: (input command, expected redirect reason).
const COMMAND_WARNING: (&str, &str) = (
    "git worktree add /tmp/review-wt main",
    "Verify the worktree target is disposable before continuing",
);
const COMMAND_PATTERN: &str = "warn-worktree-add";

/// The content-mode warning case: (file content, expected redirect reason).
const CONTENT_WARNING: (&str, &str) = (
    "listen_address = \"0.0.0.0\"",
    "Confirm this service should be reachable from off-host",
);
const CONTENT_PATTERN: &str = "warn-public-bind-address";

const PACK_ID: &str = "warning-verdict-e2e";

fn fixture_pack() -> PathBuf {
    Path::new(ROOT).join("tests/fixtures/warning-verdict/warning-pack.json")
}

/// Spawn the real hook front end over a PreToolUse payload and parse the one
/// JSON object it prints. A warning must leave the hook process successful:
/// a non-zero exit would make the harness treat an advisory as a fault.
fn run_hook(tool_name: &str, tool_input: Value) -> Value {
    let mut child = Command::new(env!("CARGO_BIN_EXE_icg"))
        .args(["hook", "--rule-pack"])
        .arg(fixture_pack())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("hook process should start");

    let payload = json!({
        "tool_name": tool_name,
        "tool_input": tool_input,
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
        "the hook must stay successful on a warning: {:?}",
        output.status
    );
    serde_json::from_slice(&output.stdout).expect("hook stdout should be one JSON object")
}

fn run_check(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_icg"))
        .args(["check", "--pack"])
        .arg(fixture_pack())
        .args(args)
        .output()
        .expect("icg check should run")
}

fn stdout_of(output: &Output) -> String {
    String::from_utf8(output.stdout.clone()).expect("stdout should be UTF-8")
}

/// `additionalContext` for a command warning is the redirect reason with the
/// rule's attribution appended -- the same text the wrapper prints on stderr
/// and an operator reads from `icg check`, so every front end names the same
/// rule for the same input.
#[test]
fn hook_warning_stays_allowed_and_carries_the_redirect_reason() {
    let (command, reason) = COMMAND_WARNING;
    let response = run_hook("Bash", json!({ "command": command }));
    let output = &response["hookSpecificOutput"];

    assert_eq!(
        output["permissionDecision"], "allow",
        "a warning must never block the call"
    );
    assert_eq!(
        output["additionalContext"],
        format!("{reason} [pack={PACK_ID}, pattern={COMMAND_PATTERN}]"),
        "the redirect reason must survive into the hook's additionalContext"
    );

    // A warning is neither a rewrite nor a denial: neither of the fields
    // those verdicts carry may appear alongside the context.
    assert!(output.get("updatedInput").is_none());
    assert!(output.get("permissionDecisionReason").is_none());
    assert_eq!(output["hookEventName"], "PreToolUse");
}

#[test]
fn check_cli_preserves_the_warning_reason_with_its_attribution() {
    let (command, reason) = COMMAND_WARNING;
    let check = run_check(&["--command", command]);

    assert_eq!(
        check.status.code(),
        Some(0),
        "`icg check` exits 0 for a warning exactly as for every advisory \
         verdict: parse stdout, never the exit status"
    );
    let stdout = stdout_of(&check);
    assert_eq!(
        stdout.lines().next(),
        Some(format!("WARNING: {reason}").as_str()),
        "the decision line must open with the redirect reason, got: {stdout:?}"
    );
    for expected in [
        format!("Pack: {PACK_ID}"),
        format!("Pattern: {COMMAND_PATTERN}"),
    ] {
        assert!(
            stdout.contains(&expected),
            "warning stdout should carry {expected:?}, got: {stdout:?}"
        );
    }

    let stderr = String::from_utf8(check.stderr).expect("stderr should be UTF-8");
    assert!(
        stderr.is_empty(),
        "a plain check keeps stderr silent even on a warning, got: {stderr:?}"
    );
}

#[test]
fn safe_pattern_claims_the_command_before_the_warning_can_fire() {
    let response = run_hook("Bash", json!({ "command": "git status" }));
    let output = &response["hookSpecificOutput"];
    assert_eq!(output["permissionDecision"], "allow");
    assert!(
        output.get("additionalContext").is_none(),
        "a safe pattern short-circuits the pack, so no warning may attach: {:?}",
        output.get("additionalContext")
    );

    let check = run_check(&["--command", "git status"]);
    assert_eq!(check.status.code(), Some(0));
    assert!(
        stdout_of(&check).starts_with("ALLOW: no configured rule matched"),
        "a safe-pattern match is a plain allow on the CLI too"
    );
}

#[test]
fn content_warning_stays_allowed_and_keeps_the_reason_through_both_front_ends() {
    let (content, reason) = CONTENT_WARNING;

    let response = run_hook(
        "Write",
        json!({
            "filePath": "service.conf",
            "content": content,
            "encoding": "utf-8"
        }),
    );
    let output = &response["hookSpecificOutput"];
    assert_eq!(
        output["permissionDecision"], "allow",
        "a content-mode warning must never block the Write"
    );
    assert_eq!(
        output["additionalContext"],
        format!("{reason} [pack={PACK_ID}, pattern={CONTENT_PATTERN}, file=service.conf]"),
        "the content warning names its reason, rule, and the file it is about"
    );
    assert!(output.get("updatedInput").is_none());

    let temp = tempdir().expect("temporary directory should be created");
    let file_path = temp.path().join("service.conf");
    std::fs::write(&file_path, format!("{content}\n")).expect("warning fixture file should write");

    let check = run_check(&["--file", file_path.to_str().expect("path should be UTF-8")]);
    assert_eq!(check.status.code(), Some(0));
    let stdout = stdout_of(&check);
    assert_eq!(
        stdout.lines().next(),
        Some(format!("WARNING: {reason}").as_str()),
        "the CLI renders the content warning from the file's content, got: {stdout:?}"
    );
    assert!(
        stdout.contains(&format!("Pattern: {CONTENT_PATTERN}")),
        "content warning stdout should name the rule, got: {stdout:?}"
    );
}
