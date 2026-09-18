//! Golden fixtures and process-boundary tests for the harness-adapter
//! contract (`src/adapter.rs`, `docs/notes/harness-adapter-contract.md`).
//!
//! Everything here drives the real `icg hook` binary: a fixture request is
//! written to its stdin exactly as the harness would, and the one JSON object
//! on stdout is compared against the fixture's recorded response. That locks
//! the whole boundary -- adapter parse, engine evaluation, adapter render --
//! rather than any function the front end could silently stop calling.
//!
//! The fixtures live in `tests/fixtures/adapter/`. Each `<harness>-<scenario>
//! .request.json` pairs with `.response.json`; the malformed-input fixture is
//! `.request.txt` because it is deliberately not valid JSON. Two fixture
//! packs sit beside them (`command-rewrite-pack.json`,
//! `edit-rewrite-pack.json`) so the rewrite goldens stay deterministic even
//! if the shipped packs' reasons evolve; the deny, allow, and warning
//! goldens deliberately run against the *shipped* packs and the shared
//! `warning-verdict` pack to prove the adapters evaluate the same policy as
//! everything else.

use serde_json::{json, Value};
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

const ROOT: &str = env!("CARGO_MANIFEST_DIR");
const ADAPTER_FIXTURES: &str = "tests/fixtures/adapter";
const SHIPPED_PACKS: &str = "packs";
/// The shared warning pack, which lives outside the adapter fixtures because
/// the engine's own warning-verdict tests use the same file.
const WARNING_PACK: &str = "tests/fixtures/warning-verdict/warning-pack.json";

/// The contract version the process boundary currently speaks. Locked here
/// so a bump in `src/adapter.rs` has to be acknowledged here too.
const ADAPTER_CONTRACT_VERSION: u32 = 1;

/// One golden case: (request fixture, harness to declare, pack to load,
/// response fixture). Declaring the harness exercises the `--harness` flag
/// on every golden; the undecorated hook is separately proven equivalent
/// below.
fn golden_cases() -> Vec<(&'static str, &'static str, &'static str, &'static str)> {
    vec![
        (
            "claude-code-allow",
            "claude-code",
            SHIPPED_PACKS,
            "claude-code-allow",
        ),
        (
            "claude-code-rewrite",
            "claude-code",
            "command-rewrite-pack",
            "claude-code-rewrite",
        ),
        (
            "claude-code-deny-write",
            "claude-code",
            SHIPPED_PACKS,
            "claude-code-deny-write",
        ),
        (
            "claude-code-warning",
            "claude-code",
            WARNING_PACK,
            "claude-code-warning",
        ),
        (
            "claude-code-unsupported-tool",
            "claude-code",
            SHIPPED_PACKS,
            "claude-code-unsupported-tool",
        ),
        (
            "claude-code-edit-rewrite-preserved-fields",
            "claude-code",
            "edit-rewrite-pack",
            "claude-code-edit-rewrite-preserved-fields",
        ),
        (
            "codex-cli-deny-patch",
            "codex-cli",
            SHIPPED_PACKS,
            "codex-cli-deny-patch",
        ),
        (
            "codex-cli-deny-command",
            "codex-cli",
            SHIPPED_PACKS,
            "codex-cli-deny-command",
        ),
    ]
}

fn fixture_path(name: &str, suffix: &str) -> PathBuf {
    Path::new(ROOT)
        .join(ADAPTER_FIXTURES)
        .join(format!("{name}.{suffix}"))
}

fn pack_path(name: &str) -> PathBuf {
    if name == SHIPPED_PACKS {
        Path::new(ROOT).join(name)
    } else if name == WARNING_PACK {
        Path::new(ROOT).join(WARNING_PACK)
    } else {
        Path::new(ROOT).join(ADAPTER_FIXTURES).join(format!("{name}.json"))
    }
}

/// Spawn the real hook front end over a request fixture and return its
/// process output. The command line mirrors what a harness's hook config
/// runs: `icg hook --rule-pack <packs> --harness <harness>`.
fn run_hook(request: &Path, harness: Option<&str>, pack: &str) -> std::process::Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_icg"));
    command
        .args(["hook", "--rule-pack"])
        .arg(pack_path(pack));
    if let Some(harness) = harness {
        command.args(["--harness", harness]);
    }
    let request_json =
        std::fs::read_to_string(request).unwrap_or_else(|error| panic!("read {request:?}: {error}"));

    let mut child = command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("hook process should start");
    child
        .stdin
        .take()
        .expect("hook stdin should be available")
        .write_all(request_json.as_bytes())
        .expect("hook input should be written");
    child.wait_with_output().expect("hook process should finish")
}

fn stdout_json(output: &std::process::Output) -> Value {
    assert!(
        output.status.success(),
        "the hook must stay successful for every fixture verdict: {:?}",
        output.status
    );
    serde_json::from_slice(&output.stdout)
        .expect("hook stdout should be exactly one JSON object")
}

/// Every golden request must render exactly its recorded response: allow,
/// additional-context warning, rewrite, deny, and the preserved
/// harness-specific fields on a rewrite all locked byte-for-byte.
#[test]
fn golden_fixtures_render_exactly_their_recorded_response() {
    for (request_name, harness, pack, response_name) in golden_cases() {
        let output = run_hook(
            &fixture_path(request_name, "request.json"),
            Some(harness),
            pack,
        );
        let actual = stdout_json(&output);
        let expected_text = std::fs::read_to_string(fixture_path(response_name, "response.json"))
            .expect("response fixture should exist");
        let expected: Value =
            serde_json::from_str(&expected_text).expect("response fixture should parse");

        assert_eq!(
            actual, expected,
            "golden mismatch for {request_name}: the contract's response changed; \
             if the change is deliberate, regenerate the fixture and say so in \
             the commit"
        );
    }
}

/// A truncated payload is malformed input: the hook fails open with a
/// successful process and a plain allow, the contract's malformed-input
/// behavior.
#[test]
fn malformed_input_fails_open_with_a_successful_process() {
    let output = run_hook(
        &fixture_path("codex-cli-malformed-input", "request.txt"),
        Some("codex-cli"),
        SHIPPED_PACKS,
    );

    assert!(
        output.status.success(),
        "fail-open must leave the hook process successful: {:?}",
        output.status
    );
    let response: Value =
        serde_json::from_slice(&output.stdout).expect("stdout should be one JSON object");
    assert_eq!(
        response,
        json!({
            "hookSpecificOutput": {
                "hookEventName": "PreToolUse",
                "permissionDecision": "allow"
            }
        })
    );
    assert!(
        !output.stderr.is_empty(),
        "the failure is diagnosed on stderr so an operator can see why nothing was checked"
    );
}

/// The undecorated hook (the wiring that predates the adapter contract)
/// must keep serving exactly the envelope the declared Claude Code adapter
/// serves: adding the contract changed no existing invocation.
#[test]
fn undecorated_hook_matches_the_declared_claude_code_envelope() {
    let request = fixture_path("claude-code-allow", "request.json");
    let undecorated = stdout_json(&run_hook(&request, None, SHIPPED_PACKS));
    let declared = stdout_json(&run_hook(&request, Some("claude-code"), SHIPPED_PACKS));

    assert_eq!(
        undecorated, declared,
        "the default adapter and the declared claude-code adapter must agree"
    );
}

/// A declared harness is recorded in evaluation telemetry by its fixed
/// slug -- and nothing from the payload ever is. The record's key set is
/// asserted in full so a future field that smuggles request data into
/// telemetry fails here first.
#[test]
fn declared_harness_reaches_telemetry_as_a_slug_and_no_payload_does() {
    let temp = tempfile::tempdir().expect("temporary directory should be created");
    let telemetry_path = temp.path().join("telemetry.json");
    const MARKER: &str = "TELEMETRY-HYGIENE-MARKER-7f3a";

    let mut child = Command::new(env!("CARGO_BIN_EXE_icg"))
        .args([
            "hook",
            "--rule-pack",
            &pack_path(SHIPPED_PACKS).to_string_lossy(),
            "--harness",
            "codex-cli",
        ])
        .env("ICG_TELEMETRY_PATH", &telemetry_path)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("hook process should start");
    // A denied command carrying a payload-only marker. The rule id the
    // denial names may legitimately appear in telemetry's rule counters;
    // the marker may not, because it exists only in the request.
    let payload = json!({
        "hook_event_name": "PreToolUse",
        "tool_name": "Bash",
        "tool_input": {
            "command": format!("git credential fill --marker {MARKER}")
        }
    });
    child
        .stdin
        .take()
        .expect("hook stdin should be available")
        .write_all(payload.to_string().as_bytes())
        .expect("hook input should be written");
    let output = child.wait_with_output().expect("hook process should finish");
    assert!(output.status.success(), "hook process should succeed");

    let store: Value = serde_json::from_str(
        &std::fs::read_to_string(&telemetry_path).expect("telemetry file should exist"),
    )
    .expect("telemetry file should parse");
    let record = &store["window"]["records"][0];
    assert_eq!(record["harness"], "codex-cli");
    assert_eq!(record["verdict"], "denied");

    // The whole record is verdict-shaped: identity metadata only, no echo
    // of what was evaluated.
    let mut keys: Vec<&str> = record
        .as_object()
        .expect("record should be an object")
        .keys()
        .map(String::as_str)
        .collect();
    keys.sort_unstable();
    assert_eq!(
        keys,
        vec!["harness", "release_ref", "session_id", "timestamp", "verdict"],
        "evaluation records stay verdict-shaped; a new key must be justified \
         against the no-payload-data rule before it lands"
    );

    let raw = std::fs::read_to_string(&telemetry_path).expect("telemetry file should be readable");
    assert!(
        !raw.contains(MARKER),
        "request data leaked into telemetry: the marker from the denied \
         command must never appear in the store"
    );
}

/// Without a declared harness the record carries none: undeclared means
/// unnamed, never guessed from the payload.
#[test]
fn undeclared_hook_records_no_harness_identity() {
    let temp = tempfile::tempdir().expect("temporary directory should be created");
    let telemetry_path = temp.path().join("telemetry.json");

    let mut child = Command::new(env!("CARGO_BIN_EXE_icg"))
        .args([
            "hook",
            "--rule-pack",
            &pack_path(SHIPPED_PACKS).to_string_lossy(),
        ])
        .env("ICG_TELEMETRY_PATH", &telemetry_path)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("hook process should start");
    let payload = json!({
        "hook_event_name": "PreToolUse",
        "tool_name": "Bash",
        "tool_input": { "command": "git status" }
    });
    child
        .stdin
        .take()
        .expect("hook stdin should be available")
        .write_all(payload.to_string().as_bytes())
        .expect("hook input should be written");
    let output = child.wait_with_output().expect("hook process should finish");
    assert!(output.status.success());

    let store: Value = serde_json::from_str(
        &std::fs::read_to_string(&telemetry_path).expect("telemetry file should exist"),
    )
    .expect("telemetry file should parse");
    let record = &store["window"]["records"][0];
    assert!(
        record["harness"].is_null(),
        "an undeclared invocation records no harness, got: {record}"
    );
}

/// A harness whose adapter is specified but not implemented is refused
/// before any evaluation: a response the harness cannot read must never be
/// emitted under its name.
#[test]
fn an_unimplemented_harness_is_refused_before_any_evaluation() {
    let temp = tempfile::tempdir().expect("temporary directory should be created");
    let telemetry_path = temp.path().join("telemetry.json");

    let output = Command::new(env!("CARGO_BIN_EXE_icg"))
        .args([
            "hook",
            "--rule-pack",
            &pack_path(SHIPPED_PACKS).to_string_lossy(),
            "--harness",
            "cursor",
        ])
        .env("ICG_TELEMETRY_PATH", &telemetry_path)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("hook process should start")
        .wait_with_output()
        .expect("hook process should finish");

    assert!(
        !output.status.success(),
        "an unimplemented adapter must refuse to run"
    );
    let stderr = String::from_utf8(output.stderr).expect("stderr should be UTF-8");
    assert!(
        stderr.contains("cursor"),
        "the refusal names the harness so wiring is fixable, got: {stderr:?}"
    );
    assert!(
        output.stdout.is_empty(),
        "no decision envelope is emitted for an unimplemented adapter"
    );

    // The refusal happens before any evaluation, so the telemetry store
    // carries no evaluation record and no trace of the harness slug. (The
    // store file itself is created by the health lifecycle's startup sync,
    // which every hook invocation performs before the adapter contract is
    // consulted; what the refusal guarantees is that nothing about *this
    // call* lands in it.)
    let raw = std::fs::read_to_string(&telemetry_path).expect("telemetry store should be readable");
    let store: Value =
        serde_json::from_str(&raw).expect("telemetry store should parse");
    assert!(
        store["window"]["records"]
            .as_array()
            .expect("records should be an array")
            .is_empty(),
        "a refused invocation evaluates nothing, got records: {}",
        store["window"]["records"]
    );
    assert!(
        !raw.contains("cursor"),
        "a refused invocation records no harness identity"
    );
}

/// The canonical request the adapters build carries the contract version.
/// Verified through the library (the binary's wire format does not expose
/// the version) so a bump without migration is caught next to the code.
#[test]
fn the_canonical_request_carries_the_current_contract_version() {
    use icg::adapter;

    assert_eq!(adapter::ADAPTER_CONTRACT_VERSION, ADAPTER_CONTRACT_VERSION);

    let engine = icg::engine::Engine::new();
    let input: icg::engine::PreToolUseInput = serde_json::from_value(json!({
        "tool_name": "Bash",
        "tool_input": { "command": "git status" }
    }))
    .expect("input parses");
    for harness in [adapter::HarnessId::ClaudeCode, adapter::HarnessId::CodexCli] {
        let request = adapter::adapter_for(harness)
            .expect("shipped adapter")
            .build_request(&engine, clone_input(&input), None);
        assert_eq!(
            request.contract_version, ADAPTER_CONTRACT_VERSION,
            "{harness:?} requests carry the current contract version"
        );
        assert_eq!(request.harness, harness);
    }
}

/// `PreToolUseInput` is not `Clone`; rebuild an equal input per adapter.
fn clone_input(input: &icg::engine::PreToolUseInput) -> icg::engine::PreToolUseInput {
    icg::engine::PreToolUseInput {
        tool_name: input.tool_name.clone(),
        tool_input: icg::engine::ToolInput {
            command: input.tool_input.command.clone(),
            file_path: input.tool_input.file_path.clone(),
            content: input.tool_input.content.clone(),
            old_string: input.tool_input.old_string.clone(),
            new_string: input.tool_input.new_string.clone(),
            encoding: input.tool_input.encoding.clone(),
            mime_type: input.tool_input.mime_type.clone(),
        },
        id: input.id.clone(),
        timestamp: input.timestamp.clone(),
        session_id: input.session_id.clone(),
    }
}
