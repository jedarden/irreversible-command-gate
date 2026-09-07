//! The reader and the writer must agree on where the denial log lives.
//!
//! They did not. `DenialLog::default_path()` is /var/cache/icg/denials.jsonl
//! -- chosen because /var/cache is writable by the hook identity -- but
//! `icg status --denials` looked in /var/log/icg/ and then fell through to
//! /var/cache/icg/session-state.json, a STATE file that merely exists. It
//! selected that, failed to parse it, and reported "invalid denial record",
//! which reads like log corruption rather than looking in the wrong place.
//!
//! Both hosts instrumented on 2026-09-06 were collecting data the documented
//! readout could not read.

use std::fs;
use std::process::Command;
use tempfile::tempdir;

fn icg(args: &[&str], env: &[(&str, &str)]) -> std::process::Output {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_icg"));
    cmd.args(args);
    for (k, v) in env {
        cmd.env(k, v);
    }
    cmd.output().expect("icg should run")
}

const RECORD: &str = r#"{"id":"denial-1","timestamp":"2026-09-07T04:10:49.885197245Z","pack_id":"git","pattern_id":"git-commit-without-pathspec","category":"guarded_operation","severity":"high","reason":"git commit without a pathspec can sweep unrelated staged changes into the commit.","denied_input":{"type":"Command","command":"<redacted>","segments":[]},"context":{"user":"coding","tool":"command"},"system_state":{"uptime_seconds":0.0,"release_ref":""}}"#;

/// The writer's default is the reader's default. One source of truth.
#[test]
fn reader_default_matches_the_writer_default() {
    let writer = icg::denial_log::DenialStore::default_path().expect("writer has a default");
    assert_eq!(
        writer.to_string_lossy(),
        "/var/cache/icg/denials.jsonl",
        "the writer's documented default moved; the reader must follow it"
    );
    let candidates = icg::documented_commands::denial_log_search_paths();
    assert!(
        candidates.first().map(|p| p == &writer).unwrap_or(false),
        "the reader must try the writer's own default first, got {candidates:?}"
    );
}

/// A state file is not a denial log and must never be selected as one.
#[test]
fn the_state_store_path_is_not_offered_as_a_denial_log() {
    let candidates = icg::documented_commands::denial_log_search_paths();
    let state = icg::state_store::StateStore::default_path().expect("state store has a default");
    assert!(
        !candidates.contains(&state),
        "session-state.json is state, not a denial log; offering it produced \
         'invalid denial record in /var/cache/icg/session-state.json'"
    );
}

/// A readable log at the writer's default is found with no environment set.
#[test]
fn status_reads_the_log_the_hook_writes() {
    let dir = tempdir().expect("tempdir");
    let log = dir.path().join("denials.jsonl");
    fs::write(&log, format!("{RECORD}\n")).expect("write log");

    let output = icg(
        &["status", "--denials", "--since", "30d"],
        &[("ICG_DENIAL_LOG", log.to_str().unwrap())],
    );
    let text = String::from_utf8_lossy(&output.stdout);
    assert!(
        text.contains("git-commit-without-pathspec"),
        "status should list the recorded denial, got:\n{text}\nstderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

/// And the summary the docs tell operators to run must work on the same data.
#[test]
fn pattern_summary_works_on_the_hooks_own_log() {
    let dir = tempdir().expect("tempdir");
    let log = dir.path().join("denials.jsonl");
    fs::write(&log, format!("{RECORD}\n{RECORD}\n")).expect("write log");

    let output = icg(
        &["status", "--denials", "--pattern-summary", "--since", "30d"],
        &[("ICG_DENIAL_LOG", log.to_str().unwrap())],
    );
    let text = String::from_utf8_lossy(&output.stdout);
    assert!(
        text.contains("git-commit-without-pathspec"),
        "pattern-summary should group the recorded denials, got:\n{text}\nstderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

/// When nothing is found, say what was looked for.
#[test]
fn a_missing_log_names_the_paths_tried() {
    let dir = tempdir().expect("tempdir");
    let output = icg(
        &["status", "--denials", "--since", "30d"],
        &[(
            "ICG_DENIAL_LOG",
            dir.path().join("absent.jsonl").to_str().unwrap(),
        )],
    );
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        text.contains("absent.jsonl"),
        "the error should name the path it could not read, got:\n{text}"
    );
}
