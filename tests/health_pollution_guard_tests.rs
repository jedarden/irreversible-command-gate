//! A test run must never reach the host's live health state or telemetry.
//!
//! `HealthStore::from_environment_or_default` falls back to
//! /var/cache/icg/health-state.json when ICG_HEALTH_PATH is unset, and every
//! lifecycle boundary syncs a snapshot into /var/cache/icg/telemetry.json
//! when ICG_TELEMETRY_PATH is unset. On an instrumented host that also builds
//! this repository, those fallbacks are live traffic: on 2026-09-13 the
//! production crash history was 382 records and *every retained record*
//! carried a test context -- `could not find the real \`fake_tool\` binary
//! after skipping icg wrapper entries in PATH` from wrapper tests,
//! `guard availability failure during evaluation` from hook tests -- inflating
//! exactly the totals that crash-rate analyses and denial-count style
//! investigations read.
//!
//! Setting ICG_HEALTH_PATH/ICG_TELEMETRY_PATH in every guard-evaluating test
//! is per-test discipline, and discipline is what leaked (26 test files spawn
//! `icg` without either variable). The guard therefore lives in the library
//! (`health::HealthStore::from_environment_or_default`,
//! `telemetry::operational_store_path`), and these tests pin it from outside,
//! in both process shapes `cargo test` actually produces:
//!
//! 1. this test binary, which calls into the library in-process;
//! 2. the `icg` binary it spawns through `CARGO_BIN_EXE_icg`, which is an
//!    unmodified production binary and has no test marker of its own.
//!
//! Each probe identifies its own record by a unique context string, and reads
//! the live files after the write rather than comparing them to a snapshot:
//! this host rewrites both files while the test runs, for real guarded tool
//! calls from real sessions. Relocation, not refusal: the relocated records
//! are asserted to exist, because a guard that silently dropped health
//! tracking under test would be passing these tests while proving nothing.

use std::path::Path;
use std::process::{Command, Output, Stdio};
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::json;
use tempfile::tempdir;

/// A context string that cannot collide with a real crash, a fixture, or
/// another run of this suite.
fn probe_context() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock should be after the epoch")
        .as_nanos();
    format!(
        "pollution-probe health crash {}-{nanos}",
        std::process::id()
    )
}

/// The live files the writer documents, not second copies of the defaults.
fn live_health_path() -> std::path::PathBuf {
    icg::health::HealthStore::default_path().expect("the writer documents a default path")
}

fn live_telemetry_path() -> std::path::PathBuf {
    icg::telemetry::default_path()
}

fn file_contains(path: &Path, marker: &str) -> bool {
    let bytes = std::fs::read(path).unwrap_or_default();
    String::from_utf8_lossy(&bytes).contains(marker)
}

/// The probes deliberately leave ICG_HEALTH_PATH and ICG_TELEMETRY_PATH
/// unset: that unset fallback is the whole subject of this file. A shell that
/// exports either would silently turn these tests into no-ops.
fn guard_is_not_bypassed_by_an_exported_sink() {
    for variable in ["ICG_HEALTH_PATH", "ICG_TELEMETRY_PATH"] {
        assert!(
            std::env::var_os(variable).is_none(),
            "{variable} is exported into the test environment; the pollution \
             guard tests would prove nothing. Run the suite without it."
        );
    }
}

/// A minimal pack that allows everything: the probe invocation must reach the
/// real-binary lookup and fail there, which is the exact shape that produced
/// the production noise.
fn allow_all_pack(directory: &tempfile::TempDir) -> std::path::PathBuf {
    let path = directory.path().join("pollution-guard-pack.json");
    std::fs::write(
        &path,
        serde_json::to_vec_pretty(&json!({
            "id": "pollution-guard-test",
            "tool_keywords": ["pollution-probe"],
            "applies_to": [],
            "safe_patterns": [],
            "guarded_patterns": []
        }))
        .expect("pack should serialize"),
    )
    .expect("pack should be written");
    path
}

/// One `icg wrapper <tool>` invocation whose real binary does not exist.
/// Environment is hermetic everywhere except the two sinks under test: an
/// explicit `sink` is honoured; without one the invocation relies on the
/// fallbacks, so both are removed rather than merely unset.
fn run_wrapper_without_real_tool(tool: &str, pack: &Path, sink: Option<&Path>) -> Output {
    let mut hook = Command::new(env!("CARGO_BIN_EXE_icg"));
    hook.args(["wrapper", tool, "some", "args"])
        .env("ICG_RULE_PACK", pack)
        .env_remove("ICG_HEALTH_PATH")
        .env_remove("ICG_TELEMETRY_PATH");
    if let Some(sink) = sink {
        hook.env("ICG_HEALTH_PATH", sink);
    }

    hook.stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .expect("wrapper process should run")
}

/// The library write path, exercised from a test binary's own process: the
/// relocated store must receive the record, and neither live file may.
#[test]
fn an_in_process_crash_never_reaches_the_live_health_state() {
    guard_is_not_bypassed_by_an_exported_sink();
    let marker = probe_context();

    let store = icg::health::HealthStore::from_environment_or_default()
        .expect("the relocated store should resolve");
    assert_ne!(
        store.path(),
        live_health_path(),
        "a test-driven store must not resolve to the host's live health state"
    );

    let crash = icg::health::CrashRecord::new(icg::health::CrashType::ExitCodeError)
        .with_context(marker.clone());
    store
        .record_crash(crash)
        .expect("the relocated store should accept the record");

    assert!(
        file_contains(store.path(), &marker),
        "relocation must still record: the probe crash should be durable in {:?}",
        store.path()
    );
    assert!(
        !file_contains(&live_health_path(), &marker),
        "the probe crash reached the host's live health state -- exactly the \
         pollution that filled the production crash history with test noise"
    );
    assert!(
        !file_contains(&live_telemetry_path(), &marker),
        "the probe crash reached the host's live telemetry cache"
    );
}

/// The spawned-binary path: the exact production-noise shape, an `icg
/// wrapper` whose real binary is missing, so the lifecycle records an
/// `exit_code_error` crash on the way out.
#[test]
fn a_crashing_wrapper_invocation_never_reaches_the_live_health_state() {
    guard_is_not_bypassed_by_an_exported_sink();
    let directory = tempdir().expect("support directory");
    let tool = format!(
        "pollution-probe-tool-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock should be after the epoch")
            .as_nanos()
    );
    let pack = allow_all_pack(&directory);

    let output = run_wrapper_without_real_tool(&tool, &pack, None);
    assert!(
        !output.status.success(),
        "a wrapper whose real binary is missing should fail: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("event=crash_recorded"),
        "the wrapper should have recorded its crash somewhere (relocated, not \
         dropped), stderr: {stderr}"
    );

    assert!(
        !file_contains(&live_health_path(), &tool),
        "the wrapper's crash record reached the host's live health state -- \
         this is the `fake_tool` pollution, reprised"
    );
    assert!(
        !file_contains(&live_telemetry_path(), &tool),
        "the wrapper's crash record reached the host's live telemetry cache"
    );
}

/// The guard must not have broken tracking: the same failing invocation that
/// is refused the default still records to a health path it was named
/// explicitly.
#[test]
fn an_explicit_health_path_still_records() {
    guard_is_not_bypassed_by_an_exported_sink();
    let directory = tempdir().expect("support directory");
    let sink = directory.path().join("health.json");
    let tool = format!(
        "pollution-probe-tool-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock should be after the epoch")
            .as_nanos()
    );
    let pack = allow_all_pack(&directory);

    let output = run_wrapper_without_real_tool(&tool, &pack, Some(&sink));
    assert!(
        !output.status.success(),
        "the probe wrapper should still fail with an explicit sink: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let recorded = std::fs::read_to_string(&sink)
        .expect("an explicitly named health path should have been created");
    assert!(
        recorded.contains(&tool),
        "an explicit ICG_HEALTH_PATH must still record crashes, got: {recorded}"
    );
    assert!(
        recorded.contains("exit_code_error"),
        "the crash should be classified as an exit-code error, got: {recorded}"
    );
}
