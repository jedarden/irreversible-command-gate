//! A test run must never reach the host's live denial log.
//!
//! `DenialStore` falls back to /var/cache/icg/denials.jsonl when
//! ICG_DENIAL_LOG is unset. On an instrumented host that also builds this
//! repository, that fallback is live traffic: on 2026-09-07 the practice
//! trial had 14 of its 72 denial records written by `cargo test` -- fixture
//! pattern ids sitting next to real ones, and no way to tell a fixture that
//! exercises a *real* pattern id from a real denial after the fact. The whole
//! trial window had to be re-baselined.
//!
//! Setting ICG_DENIAL_LOG in every deny-path test is per-test discipline, and
//! discipline is what leaked. The guard therefore lives in the library
//! (`denial_log::operational_log_path`), and these tests pin it from outside,
//! in both process shapes `cargo test` actually produces:
//!
//! 1. this test binary, which calls into the library in-process;
//! 2. the `icg` binary it spawns through `CARGO_BIN_EXE_icg`, which is an
//!    unmodified production binary and has no test marker of its own.
//!
//! The probe identifies its own record by pattern id -- the field that made
//! the original leak visible -- because command and content payloads are
//! redacted unless full-content logging is switched on. It reads the live
//! log's tail around each write rather than comparing file length: this host
//! appends real denials to the same file while the test runs.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::json;
use tempfile::{tempdir, TempDir};

/// A pattern id that cannot collide with a real denial, a fixture, or another
/// run of this suite.
fn probe_pattern_id() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock should be after the epoch")
        .as_nanos();
    format!("pollution-probe-dangerous-{}-{nanos}", std::process::id())
}

/// The live log the writer documents, not a second copy of the default.
fn live_log() -> PathBuf {
    icg::denial_log::DenialStore::default_path().expect("the writer documents a default path")
}

/// Everything appended to the live log after `offset` was taken.
fn live_log_tail(offset: u64) -> String {
    let bytes = std::fs::read(live_log()).unwrap_or_default();
    String::from_utf8_lossy(bytes.get(offset as usize..).unwrap_or(&[])).into_owned()
}

/// Byte length of the live log, or zero where it does not exist yet.
fn offset_of_live_log() -> u64 {
    std::fs::metadata(live_log())
        .map(|metadata| metadata.len())
        .unwrap_or(0)
}

/// The probe deliberately leaves ICG_DENIAL_LOG unset: that unset fallback is
/// the whole subject of this file. A shell that exports it would silently
/// turn these tests into no-ops.
fn guard_is_not_bypassed_by_an_exported_sink() {
    assert!(
        std::env::var_os("ICG_DENIAL_LOG").is_none(),
        "ICG_DENIAL_LOG is exported into the test environment; the pollution \
         guard tests would prove nothing. Run the suite without it."
    );
}

/// A minimal pack whose single guarded pattern denies the probe command and
/// names it with a unique pattern id.
fn guard_pack(directory: &TempDir, pattern_id: &str) -> PathBuf {
    let path = directory.path().join("pollution-guard-pack.json");
    let pack = json!({
        "id": "pollution-guard-test",
        "tool_keywords": ["pollution-probe"],
        "applies_to": [],
        "safe_patterns": [],
        "guarded_patterns": [{
            "id": pattern_id,
            "type": "command_regex",
            "regex": "^pollution-probe dangerous$",
            "tier": "tier1",
            "severity": "Critical",
            "explanation": "The pollution probe is guarded",
            "redirect": {
                "channel": "deny",
                "reason_template": "Do not run the pollution probe",
                "rewrite_template": null
            },
            "destructive": true
        }]
    });
    std::fs::write(
        &path,
        serde_json::to_vec_pretty(&pack).expect("pack should serialize"),
    )
    .expect("pack should be written");
    path
}

/// One `icg hook` invocation, hermetic everywhere except the denial log:
/// telemetry, health, and the fail-closed policy are pointed at `directory`,
/// so the only file a deny can reach outside it is the live log. An explicit
/// `sink` is honoured; without one the invocation relies on the fallback.
fn run_hook(pack: &Path, directory: &TempDir, command: &str, sink: Option<&Path>) -> Output {
    // Environment is configured on the Command: a Child's environment is
    // inherited at spawn time and cannot be changed afterwards.
    let mut hook = Command::new(env!("CARGO_BIN_EXE_icg"));
    hook.args(["hook", "--rule-pack"])
        .arg(pack)
        .env(
            "ICG_TELEMETRY_PATH",
            directory.path().join("telemetry.json"),
        )
        .env("ICG_HEALTH_PATH", directory.path().join("health.json"))
        .env(
            "ICG_FAIL_CLOSED_POLICY",
            directory.path().join("fail-closed-policy.json"),
        );
    match sink {
        Some(sink) => {
            hook.env("ICG_DENIAL_LOG", sink);
        }
        // The fallback under test is the *unset* variable. `env_remove` keeps
        // the probe hermetic even if a wrapper exported one.
        None => {
            hook.env_remove("ICG_DENIAL_LOG");
        }
    }

    let mut child = hook
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("hook process should start");

    child
        .stdin
        .take()
        .expect("hook stdin should be available")
        .write_all(
            json!({
                "hook_event_name": "PreToolUse",
                "tool_name": "Bash",
                "tool_input": {"command": command}
            })
            .to_string()
            .as_bytes(),
        )
        .expect("hook input should be written");

    child
        .wait_with_output()
        .expect("hook process should finish")
}

fn assert_denied(output: &Output, context: &str) {
    assert!(
        output.status.success(),
        "{context}: hook should not fail: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let response = String::from_utf8_lossy(&output.stdout);
    assert!(
        response.contains("\"permissionDecision\":\"deny\""),
        "{context}: expected a deny, got: {response}"
    );
}

fn assert_marker_absent(tail: &str, marker: &str, context: &str) {
    assert!(
        !tail.contains(marker),
        "{context}: the probe record reached the host's live denial log -- \
         exactly the pollution that re-baselined the practice trial. Tail: {tail}"
    );
}

/// The library write path, exercised from a test binary's own process.
#[test]
fn an_in_process_denial_never_reaches_the_live_log() {
    guard_is_not_bypassed_by_an_exported_sink();
    let marker = probe_pattern_id();
    let offset = offset_of_live_log();

    let source = icg::engine::InputSource::Command(icg::engine::CommandSource::Hook(
        "pollution-probe dangerous".to_string(),
    ));
    let denied = icg::engine::CheckResult::Denied {
        reason: "pollution guard probe".to_string(),
        pack_id: "pollution-guard-test".to_string(),
        pattern_id: marker.clone(),
        matched_path: None,
    };
    icg::denial_log::record_operational_denial(&source, &denied);

    let tail = live_log_tail(offset);
    assert_marker_absent(&tail, &marker, "a denial recorded from a test binary");
}

/// The spawned-binary path: an unmodified `icg` whose parent is a test
/// harness has no test marker in its own argv, only in its inherited
/// environment.
#[test]
fn a_denied_hook_invocation_never_reaches_the_live_log() {
    guard_is_not_bypassed_by_an_exported_sink();
    let directory = tempdir().expect("support directory");
    let marker = probe_pattern_id();
    let pack = guard_pack(&directory, &marker);
    let offset = offset_of_live_log();

    let output = run_hook(&pack, &directory, "pollution-probe dangerous", None);
    assert_denied(&output, "spawned hook without ICG_DENIAL_LOG");

    let tail = live_log_tail(offset);
    assert_marker_absent(&tail, &marker, "a denial from a test-spawned icg");
}

/// The guard must not have broken logging: the same invocation that is
/// refused the default still records to a sink it was named explicitly.
#[test]
fn an_explicit_denial_log_still_records() {
    guard_is_not_bypassed_by_an_exported_sink();
    let directory = tempdir().expect("support directory");
    let sink = directory.path().join("denials.jsonl");
    let marker = probe_pattern_id();
    let pack = guard_pack(&directory, &marker);

    let output = run_hook(&pack, &directory, "pollution-probe dangerous", Some(&sink));
    assert_denied(&output, "spawned hook with ICG_DENIAL_LOG");

    let recorded = std::fs::read_to_string(&sink)
        .expect("an explicitly named denial log should have been created");
    assert!(
        recorded.contains(&marker),
        "an explicit ICG_DENIAL_LOG must still record denials, got: {recorded}"
    );
}
