//! Process-boundary suite for the Gemini CLI `BeforeTool` adapter
//! (`docs/notes/harness-adapter-contract.md` §6.4).
//!
//! The `gemini-cli-*` goldens in `adapter_contract_tests.rs` pin the exact
//! bytes of each recorded response. This suite pins what those goldens
//! cannot: the *shape* of the envelope under each verdict (the key sets
//! Gemini's hook reader is allowed to see), the inertness of the
//! Gemini-only context fields the payload carries around the translated
//! pair, and -- by executing harmless fake targets through a dispatcher
//! built strictly from Gemini's documented dispatch semantics -- that a
//! denied tool call never reaches execution while a failed adapter call
//! fails open and stays harmless. Everything drives the real
//! `icg hook --harness gemini-cli` binary with the official payload on
//! stdin, exactly as Gemini CLI delivers it.

use serde_json::{json, Value};
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

const ROOT: &str = env!("CARGO_MANIFEST_DIR");
const ADAPTER_FIXTURES: &str = "tests/fixtures/adapter";
const SHIPPED_PACKS: &str = "packs";
/// The shared warning pack, the same one the adapter goldens run on.
const WARNING_PACK: &str = "tests/fixtures/warning-verdict/warning-pack.json";
/// The fixture rewrite packs, the same ones the adapter goldens run on.
const COMMAND_REWRITE_PACK: &str = "tests/fixtures/adapter/command-rewrite-pack.json";
const EDIT_REWRITE_PACK: &str = "tests/fixtures/adapter/edit-rewrite-pack.json";

/// Marker string carried by every fixture's `mcp_context.server_name`. It
/// names the field as fixture material and doubles as a leak canary: the
/// value exists only in the request, so its appearance in a response would
/// mean the adapter echoes payload context.
const FIXTURE_ONLY_CONTEXT: &str = "fixture-only-unread-by-icg";

fn fixture_path(name: &str, suffix: &str) -> PathBuf {
    Path::new(ROOT)
        .join(ADAPTER_FIXTURES)
        .join(format!("{name}.{suffix}"))
}

fn pack_path(pack: &str) -> PathBuf {
    Path::new(ROOT).join(pack)
}

/// Spawn the real hook front end over a raw request string and return its
/// process output, exactly as Gemini CLI receives it: the command line
/// mirrors the installed hook config
/// (`icg hook --rule-pack <packs> --harness gemini-cli`).
fn dispatch_str(request: &str, pack: &str) -> std::process::Output {
    let mut child = Command::new(env!("CARGO_BIN_EXE_icg"))
        .args([
            "hook",
            "--rule-pack",
            &pack_path(pack).to_string_lossy(),
            "--harness",
            "gemini-cli",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("hook process should start");
    child
        .stdin
        .take()
        .expect("hook stdin should be available")
        .write_all(request.as_bytes())
        .expect("hook input should be written");
    child
        .wait_with_output()
        .expect("hook process should finish")
}

/// Same, over a request fixture file -- the official payload shape.
fn dispatch(request: &Path, pack: &str) -> std::process::Output {
    let request_json = std::fs::read_to_string(request)
        .unwrap_or_else(|error| panic!("read {request:?}: {error}"));
    dispatch_str(&request_json, pack)
}

fn stdout_json(output: &std::process::Output) -> Value {
    assert!(
        output.status.success(),
        "every Gemini verdict ships as exit 0: {:?}",
        output.status
    );
    serde_json::from_slice(&output.stdout).expect("stdout should be exactly one JSON object")
}

/// The response's sorted top-level key set -- the shape Gemini's hook
/// reader sees. Asserted in full everywhere so an extra field fails here
/// before it can surprise the harness.
fn key_set(value: &Value) -> Vec<&str> {
    let mut keys: Vec<&str> = value
        .as_object()
        .expect("response should be a JSON object")
        .keys()
        .map(String::as_str)
        .collect();
    keys.sort_unstable();
    keys
}

/// Gemini CLI's documented `BeforeTool` dispatch (contract §6.4, from
/// `docs/hooks/reference.md` in google-gemini/gemini-cli): exit 0 parses
/// stdout, where a `decision: "deny"` blocks the tool call (the reason
/// becomes the tool error) and anything else -- including the permissive
/// empty object -- proceeds; exit 2 would be a system block; **any other
/// exit is a non-fatal warning and the CLI continues**. This is the
/// harness-side interpretation the fake-target canaries below execute
/// against.
enum Dispatch {
    Blocked,
    Proceeds,
}

fn gemini_dispatch(output: &std::process::Output) -> Dispatch {
    if !output.status.success() {
        // "Any other exit = non-fatal warning, the CLI continues."
        return Dispatch::Proceeds;
    }
    let response: Value =
        serde_json::from_slice(&output.stdout).expect("exit 0 stdout should be one JSON object");
    match response["decision"].as_str() {
        Some("deny") => Dispatch::Blocked,
        _ => Dispatch::Proceeds,
    }
}

/// Execute a fake target the way Gemini CLI would run an admitted tool
/// call. Every target is a marker touch under the test's scratch tree --
/// harmless, and observable after the fact.
fn run_fake_target(command: &str) {
    let status = Command::new("sh")
        .arg("-c")
        .arg(command)
        .status()
        .expect("fake target should start");
    assert!(status.success(), "fake target should succeed: {command:?}");
}

/// An allow renders the permissive empty object: no `decision` key at all
/// -- `decision: "allow"` is never emitted to Gemini, whose confirmation
/// flow it could stand in for -- and nothing else on stdout either.
#[test]
fn allow_is_the_permissive_empty_object() {
    let output = dispatch(
        &fixture_path("gemini-cli-allow", "request.json"),
        SHIPPED_PACKS,
    );
    let response = stdout_json(&output);

    assert_eq!(
        response,
        json!({}),
        "an allow is the empty object: Gemini proceeds unless told otherwise"
    );
}

/// A warning has no advisory-context channel on `BeforeTool`, so it
/// degrades to a bare allow carrying the attributed reason on the common
/// `systemMessage` field -- the only key, with no decision attached.
#[test]
fn warning_degrades_to_a_bare_system_message() {
    let output = dispatch(
        &fixture_path("gemini-cli-warning-shell", "request.json"),
        WARNING_PACK,
    );
    let response = stdout_json(&output);

    assert_eq!(
        key_set(&response),
        vec!["systemMessage"],
        "a warning is exactly one systemMessage: no decision, no \
         hookSpecificOutput"
    );
    let message = response["systemMessage"].as_str().expect("string message");
    assert!(
        !message.is_empty(),
        "the attributed reason rides systemMessage, got an empty string"
    );
}

/// A denial is the common top-level `decision`/`reason` pair -- `reason`
/// is required when denied (it becomes the tool error) -- and carries
/// nothing else: no systemMessage, no hookSpecificOutput.
#[test]
fn deny_is_the_top_level_decision_reason_pair() {
    let output = dispatch(
        &fixture_path("gemini-cli-deny-shell", "request.json"),
        SHIPPED_PACKS,
    );
    let response = stdout_json(&output);

    assert_eq!(
        key_set(&response),
        vec!["decision", "reason"],
        "a deny is exactly the decision/reason pair"
    );
    assert_eq!(response["decision"], json!("deny"));
    let reason = response["reason"]
        .as_str()
        .expect("a deny carries a reason");
    assert!(
        !reason.is_empty(),
        "reason is required when denied; got an empty string"
    );
}

/// A rewrite rides `hookSpecificOutput.tool_input`, which Gemini merges
/// with and overrides the model's arguments with field-by-field -- so the
/// object is the COMPLETE replacement input: every field the harness sent,
/// only the rewrite key substituted.
#[test]
fn rewrite_rides_hook_specific_output_as_a_complete_replacement() {
    let output = dispatch(
        &fixture_path("gemini-cli-rewrite-shell", "request.json"),
        COMMAND_REWRITE_PACK,
    );
    let response = stdout_json(&output);

    assert_eq!(
        key_set(&response),
        vec!["hookSpecificOutput"],
        "a rewrite carries only the hookSpecificOutput channel"
    );
    assert_eq!(
        key_set(&response["hookSpecificOutput"]),
        vec!["tool_input"],
        "the channel carries exactly the replacement tool_input"
    );
    let tool_input = &response["hookSpecificOutput"]["tool_input"];
    assert_eq!(
        key_set(tool_input),
        vec!["command", "description", "directory"],
        "the replacement is complete: every request field, not just the \
         rewritten one -- a partial object would leave the matched field \
         intact underneath Gemini's merge"
    );
    assert_eq!(
        tool_input["command"],
        json!("git push origin main"),
        "the rewrite key is substituted"
    );
    assert_eq!(tool_input["description"], json!("Push reviewed changes"));
    assert_eq!(tool_input["directory"], json!("/project"));
}

/// The same completeness on a `replace`: the unmodeled `reviewer_note` the
/// harness sent survives into the replacement, and only `new_string` is
/// substituted.
#[test]
fn replace_rewrite_preserves_unmodeled_fields() {
    let output = dispatch(
        &fixture_path(
            "gemini-cli-replace-rewrite-preserved-fields",
            "request.json",
        ),
        EDIT_REWRITE_PACK,
    );
    let response = stdout_json(&output);

    let tool_input = &response["hookSpecificOutput"]["tool_input"];
    assert_eq!(
        key_set(tool_input),
        vec!["file_path", "new_string", "old_string", "reviewer_note"],
        "the replacement preserves every field the harness sent"
    );
    assert_eq!(tool_input["new_string"], json!("storageClassName: sata"));
    assert_eq!(tool_input["old_string"], json!("storageClassName: ssd"));
    assert_eq!(
        tool_input["reviewer_note"],
        json!("pin for the spot cluster"),
        "unmodeled fields ride the merge-override untouched"
    );
}

/// The Gemini-only context fields (`session_id`, `transcript_path`, `cwd`,
/// `timestamp`, `mcp_context`, `original_request_name`) are the lenient
/// parse's inert cargo: every verdict renders byte-identically without
/// them, and no context value ever echoes into the response.
#[test]
fn the_gemini_only_context_fields_are_inert() {
    const CONTEXT_FIELDS: [&str; 6] = [
        "session_id",
        "transcript_path",
        "cwd",
        "timestamp",
        "mcp_context",
        "original_request_name",
    ];
    // (fixture, pack) pairs covering every verdict on `run_shell_command`.
    let cases = [
        ("gemini-cli-allow", SHIPPED_PACKS),
        ("gemini-cli-deny-shell", SHIPPED_PACKS),
        ("gemini-cli-rewrite-shell", COMMAND_REWRITE_PACK),
        ("gemini-cli-warning-shell", WARNING_PACK),
    ];

    for (name, pack) in cases {
        let official: Value = serde_json::from_str(
            &std::fs::read_to_string(fixture_path(name, "request.json"))
                .unwrap_or_else(|error| panic!("read {name}: {error}")),
        )
        .expect("official fixture should parse");
        let stripped = match &official {
            Value::Object(map) => Value::Object(
                map.iter()
                    .filter(|(key, _)| !CONTEXT_FIELDS.contains(&key.as_str()))
                    .map(|(key, value)| (key.clone(), value.clone()))
                    .collect(),
            ),
            other => panic!("fixture should be an object, got {other:?}"),
        };

        let with_context = dispatch_str(&official.to_string(), pack);
        let raw = String::from_utf8_lossy(&with_context.stdout).into_owned();
        let without_context = stdout_json(&dispatch_str(&stripped.to_string(), pack));

        assert_eq!(
            stdout_json(&with_context),
            without_context,
            "{name}: the context fields must not move the verdict"
        );
        assert!(
            !raw.contains(FIXTURE_ONLY_CONTEXT) && !raw.contains("/home/dev/app"),
            "{name}: payload context leaked into the response: {raw:?}"
        );
    }
}

/// Malformed stdin keeps the shared fail-open boundary: exit 0, the
/// permissive empty object on stdout -- and the diagnostic on stderr ONLY.
/// stdout stays exactly the one JSON object Gemini parses; no prose ever
/// shares the channel.
#[test]
fn malformed_input_fails_open_with_diagnostics_on_stderr_only() {
    let output = dispatch(
        &fixture_path("malformed-input", "request.txt"),
        SHIPPED_PACKS,
    );

    assert!(
        output.status.success(),
        "fail-open leaves the process successful: {:?}",
        output.status
    );
    let stdout = String::from_utf8(output.stdout).expect("stdout should be UTF-8");
    assert_eq!(
        stdout.trim(),
        "{}",
        "stdout is exactly the permissive object -- no diagnostic prose \
         shares the channel Gemini parses"
    );
    let response: Value = serde_json::from_str(stdout.trim()).expect("the object should parse");
    assert_eq!(response, json!({}));
    let stderr = String::from_utf8(output.stderr).expect("stderr should be UTF-8");
    assert!(
        !stderr.is_empty(),
        "the failure is diagnosed on stderr so an operator can see why \
         nothing was checked"
    );
}

/// Canary, denied side: a tool call that trips a shipped rule -- a
/// credential fill sequenced with a marker touch, the shape a bypass
/// attempt would take -- blocks the whole call, so the fake target is
/// never executed. Gemini dispatches a `decision: "deny"` to a blocked
/// tool with the reason as the tool error; the marker's absence after the
/// dispatch is that semantics executed, not asserted.
#[test]
fn a_denied_invocation_never_reaches_the_fake_target() {
    let scratch = tempfile::tempdir().expect("scratch tree should be created");
    let marker = scratch.path().join("denied.marker");
    assert!(!marker.exists(), "scratch starts clean");

    let payload = json!({
        "hook_event_name": "BeforeTool",
        "tool_name": "run_shell_command",
        "tool_input": {
            "command": format!("git credential fill; touch {}", marker.display())
        }
    });
    let output = dispatch_str(&payload.to_string(), SHIPPED_PACKS);

    assert!(
        matches!(gemini_dispatch(&output), Dispatch::Blocked),
        "the sequenced credential fill must deny the whole call, got: {}",
        String::from_utf8_lossy(&output.stdout)
    );
    assert!(
        !marker.exists(),
        "a denied tool call is blocked before execution: the fake target \
         must never run"
    );
}

/// Canary, allowed control: the same dispatcher admits an unremarkable
/// command, and the fake target really runs. This is the proof that the
/// denied canary's absent marker was the gate and not a dead executor --
/// the exact pairing the Cursor dispatch proof uses.
#[test]
fn an_allowed_invocation_reaches_the_fake_target() {
    let scratch = tempfile::tempdir().expect("scratch tree should be created");
    let marker = scratch.path().join("allowed.marker");

    let payload = json!({
        "hook_event_name": "BeforeTool",
        "tool_name": "run_shell_command",
        "tool_input": {
            "command": format!("touch {}", marker.display())
        }
    });
    let output = dispatch_str(&payload.to_string(), SHIPPED_PACKS);

    match gemini_dispatch(&output) {
        Dispatch::Proceeds => run_fake_target(&format!("touch {}", marker.display())),
        Dispatch::Blocked => panic!(
            "an unremarkable touch must proceed, got: {}",
            String::from_utf8_lossy(&output.stdout)
        ),
    }
    assert!(marker.exists(), "the admitted fake target really executed");
}

/// Canary, failed side (fail-open): an adapter that cannot load its policy
/// still exits 0 with the permissive object and a stderr diagnostic, so
/// Gemini's dispatch proceeds and the fake target runs -- a failed gate is
/// a warning to the operator, never a wedge or a corruption of the flow.
#[test]
fn a_failed_pack_load_fails_open_and_stays_harmless() {
    let scratch = tempfile::tempdir().expect("scratch tree should be created");
    let marker = scratch.path().join("pack-failure.marker");

    let payload = json!({
        "hook_event_name": "BeforeTool",
        "tool_name": "run_shell_command",
        "tool_input": {
            "command": format!("touch {}", marker.display())
        }
    });
    let mut child = Command::new(env!("CARGO_BIN_EXE_icg"))
        .args([
            "hook",
            "--rule-pack",
            "/nonexistent/interlock/packs",
            "--harness",
            "gemini-cli",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("hook process should start");
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
        "a policy-load failure fails open, not with a crash: {:?}",
        output.status
    );
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    assert_eq!(
        stdout.trim(),
        "{}",
        "the permissive object is the fail-open shape on stdout"
    );
    assert!(
        !output.stderr.is_empty(),
        "the failure is diagnosed on stderr for the operator"
    );

    match gemini_dispatch(&output) {
        Dispatch::Proceeds => run_fake_target(&format!("touch {}", marker.display())),
        Dispatch::Blocked => panic!("a fail-open result must never block"),
    }
    assert!(
        marker.exists(),
        "the harness flow continued after the adapter failure: the fake \
         target ran"
    );
}

/// Canary, failed side (dead process): an adapter that dies mid-flight --
/// here pinned deterministically by killing the process while it blocks on
/// stdin, standing in for any crash or timeout -- is, per Gemini's
/// documented dispatch, a non-fatal warning and the CLI continues. The
/// fake target runs, the turn moves on, and stdout carried no partial
/// decision a harness could misread.
#[test]
fn a_crashed_adapter_leaves_the_flow_harmless() {
    let scratch = tempfile::tempdir().expect("scratch tree should be created");
    let marker = scratch.path().join("crash.marker");

    let mut child = Command::new(env!("CARGO_BIN_EXE_icg"))
        .args([
            "hook",
            "--rule-pack",
            &pack_path(SHIPPED_PACKS).to_string_lossy(),
            "--harness",
            "gemini-cli",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("hook process should start");
    // Nothing is written to stdin: the hook blocks there for the payload,
    // so the kill below lands on a live adapter -- deterministically, with
    // no verdict ever rendered.
    child.kill().expect("adapter process should be killed");
    let output = child
        .wait_with_output()
        .expect("killed process should be reaped");

    assert!(
        !output.status.success(),
        "the killed adapter must present as a failed invocation"
    );
    assert!(
        output.stdout.is_empty(),
        "a killed adapter emitted no partial decision: {:?}",
        String::from_utf8_lossy(&output.stdout)
    );

    // Gemini's "any other exit = non-fatal warning, the CLI continues".
    match gemini_dispatch(&output) {
        Dispatch::Proceeds => run_fake_target(&format!("touch {}", marker.display())),
        Dispatch::Blocked => panic!("a dead adapter must not block"),
    }
    assert!(
        marker.exists(),
        "the flow continued past the dead adapter: the fake target ran"
    );
}
