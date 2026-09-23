//! Output and exit-status contract for the `icg check` diagnostic front end.
//!
//! `icg check` is advisory: every evaluation outcome -- allow, warning,
//! rewrite, and deny -- exits `0`, and the decision line on stdout is the
//! authoritative result. This is the contract AGENTS.md documents ("parse
//! stdout, never the exit status") and the wrapper and hook front ends rely
//! on: a caller that read a non-zero exit as "the guard blocked this" would
//! misread every deny as a crash, and one that scraped stderr for the
//! decision would find nothing. `--debug` adds the full evaluation trace on
//! stderr while stdout stays byte-for-byte the same, so piping stdout into a
//! report is unchanged by debugging.
//!
//! The same contract holds in content mode: `icg check --file -` reads the
//! file body from stdin (`-` is the documented content-mode spelling) and
//! must reach every decision through the identical exit-status and stream
//! split, so a script piping a manifest through the tester sees exactly
//! what a `--command` caller sees.
//!
//! These tests run the real binary and lock both halves of the contract at
//! the process boundary, where the exit status and the two streams actually
//! exist.

use serde_json::json;
use std::io::Write as _;
use std::process::{Command, Output, Stdio};
use tempfile::tempdir;

/// One pattern per redirect channel, so a single pack reaches every
/// `CheckResult` variant from a plain `--command` invocation.
fn contract_pack() -> serde_json::Value {
    json!({
        "id": "check-output-contract",
        "tool_keywords": ["git"],
        "applies_to": [],
        "safe_patterns": [],
        "guarded_patterns": [
            {
                "id": "deny-reset",
                "type": "command_regex",
                "regex": "git reset --hard",
                "tier": "tier1",
                "severity": "Critical",
                "explanation": "Reset discards work",
                "redirect": {
                    "channel": "deny",
                    "reason_template": "Do not discard work",
                    "rewrite_template": null
                },
                "destructive": true
            },
            {
                "id": "rewrite-force-push",
                "type": "command_regex",
                "regex": "git push.*--force",
                "tier": "tier1",
                "severity": "High",
                "explanation": "Use the lease form",
                "redirect": {
                    "channel": "updated_input",
                    "reason_template": "Use --force-with-lease",
                    "rewrite_template": "git push --force-with-lease"
                },
                "destructive": true
            },
            {
                "id": "warn-worktree",
                "type": "command_regex",
                "regex": "git worktree add",
                "tier": "tier3",
                "severity": "Medium",
                "explanation": "Check the target",
                "redirect": {
                    "channel": "additional_context",
                    "reason_template": "Verify the worktree is disposable",
                    "rewrite_template": null
                },
                "destructive": false
            }
        ]
    })
}

/// Commands that reach each decision, with the stdout line that announces it.
const DECISION_CASES: &[(&str, &str)] = &[
    ("git status", "ALLOW: no configured rule matched"),
    (
        "git worktree add /tmp/wt main",
        "WARNING: Verify the worktree is disposable",
    ),
    (
        "git push origin main --force",
        "REWRITE: Use --force-with-lease",
    ),
    ("git reset --hard HEAD~1", "DENIED by icg"),
];

fn write_pack(dir: &std::path::Path) -> std::path::PathBuf {
    let pack_path = dir.join("contract-pack.json");
    std::fs::write(
        &pack_path,
        serde_json::to_vec_pretty(&contract_pack()).expect("pack should serialize"),
    )
    .expect("pack should be written");
    pack_path
}

fn run_check(pack_path: &std::path::Path, extra_args: &[&str]) -> Output {
    // A private denial-log sink keeps the deny-path invocations below off any
    // instrumented host log, no matter how the test-driven-caller guard
    // resolves inside the spawned binary. The decision output and exit status
    // the contract tests assert on are independent of where a denial record
    // lands.
    let sink = tempdir().expect("denial-log sink directory should be created");
    let output = Command::new(env!("CARGO_BIN_EXE_icg"))
        .args([
            "check",
            "--pack",
            pack_path.to_str().expect("temporary path should be UTF-8"),
        ])
        .args(extra_args)
        .env("ICG_DENIAL_LOG", sink.path().join("denials.jsonl"))
        .output()
        .expect("icg check should run");
    drop(sink);
    output
}

/// One pattern per redirect channel, over content instead of a command, so
/// `--file -` reaches every `CheckResult` variant against a single pack. The
/// applies_to glob is what makes the synthetic `stdin.yaml` path dispatch:
/// content-mode rules only fire when the target's file name matches, which
/// is exactly the dispatch a real `--file <manifest>` call exercises.
fn contract_content_pack() -> serde_json::Value {
    json!({
        "id": "file-dash-contract",
        "tool_keywords": [],
        "applies_to": ["*.yaml"],
        "safe_patterns": [],
        "guarded_patterns": [
            {
                "id": "content-deny-marker",
                "type": "content_regex",
                "regex": "probe_deny_marker",
                "tier": "tier1",
                "severity": "Critical",
                "explanation": "The deny marker is banned",
                "redirect": {
                    "channel": "deny",
                    "reason_template": "Do not write the deny marker",
                    "rewrite_template": null
                },
                "destructive": true
            },
            {
                "id": "content-rewrite-marker",
                "type": "content_regex",
                "regex": "probe_rewrite_marker",
                "tier": "tier2",
                "severity": "High",
                "explanation": "The rewrite marker has a pinned form",
                "redirect": {
                    "channel": "updated_input",
                    "reason_template": "Use the pinned form",
                    "rewrite_template": "probe_rewrite_marker: on"
                },
                "destructive": true
            },
            {
                "id": "content-warn-marker",
                "type": "content_regex",
                "regex": "probe_warn_marker",
                "tier": "tier3",
                "severity": "Medium",
                "explanation": "The warn marker needs a look",
                "redirect": {
                    "channel": "additional_context",
                    "reason_template": "Check the warn marker",
                    "rewrite_template": null
                },
                "destructive": false
            }
        ]
    })
}

/// Pipable file bodies that reach each decision, with the stdout line that
/// announces it. Only the piped bytes differ between cases -- the invocation
/// shape and the synthetic `stdin.yaml` target are identical -- so a flipped
/// decision is proof that stdin content, not a file on disk, was evaluated.
const CONTENT_CASES: &[(&str, &str)] = &[
    (
        "probe_clean_marker: fine",
        "ALLOW: no configured rule matched",
    ),
    ("probe_warn_marker: maybe", "WARNING: Check the warn marker"),
    ("probe_rewrite_marker: off", "REWRITE: Use the pinned form"),
    ("probe_deny_marker: on", "DENIED by icg"),
];

fn write_content_pack(dir: &std::path::Path) -> std::path::PathBuf {
    let pack_path = dir.join("contract-content-pack.json");
    std::fs::write(
        &pack_path,
        serde_json::to_vec_pretty(&contract_content_pack()).expect("pack should serialize"),
    )
    .expect("pack should be written");
    pack_path
}

/// Run `icg check --file -` with `content` piped to stdin. `--file -` is the
/// documented content-mode spelling for stdin ("'-' reads stdin"), so the
/// piped bytes are the file body the guard evaluates.
fn run_check_with_stdin(pack_path: &std::path::Path, content: &str, extra_args: &[&str]) -> Output {
    let sink = tempdir().expect("denial-log sink directory should be created");
    let mut child = Command::new(env!("CARGO_BIN_EXE_icg"))
        .args([
            "check",
            "--pack",
            pack_path.to_str().expect("temporary path should be UTF-8"),
            "--file",
            "-",
        ])
        .args(extra_args)
        .env("ICG_DENIAL_LOG", sink.path().join("denials.jsonl"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("icg check should run");
    child
        .stdin
        .take()
        .expect("stdin should be piped")
        .write_all(content.as_bytes())
        .expect("stdin content should be written");
    let output = child.wait_with_output().expect("icg check should finish");
    drop(sink);
    output
}

#[test]
fn every_decision_exits_successfully() {
    let temp = tempdir().expect("temporary directory should be created");
    let pack_path = write_pack(temp.path());

    for (command, expected_line) in DECISION_CASES {
        let output = run_check(&pack_path, &["--command", command]);
        assert_eq!(
            output.status.code(),
            Some(0),
            "`icg check --command {command:?}` must exit 0: the guard reports its \
             decision in stdout text, and a failing status would make every \
             advisory outcome look like a guard crash"
        );
        let stdout = String::from_utf8(output.stdout).expect("stdout should be UTF-8");
        assert!(
            stdout.starts_with(expected_line),
            "decision for {command:?} should open stdout with {expected_line:?}, got: {stdout:?}"
        );
    }
}

#[test]
fn decisions_are_emitted_on_stdout_and_keep_stderr_silent() {
    let temp = tempdir().expect("temporary directory should be created");
    let pack_path = write_pack(temp.path());

    for (command, expected_line) in DECISION_CASES {
        let output = run_check(&pack_path, &["--command", command]);
        let stdout = String::from_utf8(output.stdout).expect("stdout should be UTF-8");
        assert!(
            stdout.contains(expected_line),
            "decision for {command:?} should appear on stdout, got: {stdout:?}"
        );

        let stderr = String::from_utf8(output.stderr).expect("stderr should be UTF-8");
        assert!(
            stderr.is_empty(),
            "a plain check must keep stderr silent so callers can treat it as \
             fault-only; got: {stderr:?}"
        );
    }

    // The advisory decisions carry their attribution with them: a caller
    // parsing stdout can name the pack and pattern that fired without a
    // second invocation.
    let denied = run_check(&pack_path, &["--command", "git reset --hard HEAD~1"]);
    let stdout = String::from_utf8(denied.stdout).expect("stdout should be UTF-8");
    for expected in [
        "Reason: Do not discard work",
        "Pack: check-output-contract",
        "Pattern: deny-reset",
    ] {
        assert!(
            stdout.contains(expected),
            "deny stdout should carry {expected:?}, got: {stdout:?}"
        );
    }
    let rewritten = run_check(&pack_path, &["--command", "git push origin main --force"]);
    let stdout = String::from_utf8(rewritten.stdout).expect("stdout should be UTF-8");
    assert!(
        stdout.contains("Suggested input: git push --force-with-lease"),
        "rewrite stdout should carry the suggested input, got: {stdout:?}"
    );
}

#[test]
fn debug_flag_writes_traces_to_stderr_without_touching_stdout() {
    let temp = tempdir().expect("temporary directory should be created");
    let pack_path = write_pack(temp.path());

    for (command, _) in DECISION_CASES {
        let plain = run_check(&pack_path, &["--command", command]);
        let debugged = run_check(&pack_path, &["--command", command, "--debug"]);

        assert_eq!(
            debugged.status.code(),
            Some(0),
            "--debug must not change the advisory exit status for {command:?}"
        );
        assert_eq!(
            debugged.stdout, plain.stdout,
            "--debug must leave stdout byte-identical for {command:?}; the \
             decision stream is parsed by callers that never asked for traces"
        );

        let stderr = String::from_utf8(debugged.stderr).expect("stderr should be UTF-8");
        assert!(
            stderr.contains("Loaded 1 rule pack(s)"),
            "--debug should announce the loaded packs on stderr, got: {stderr:?}"
        );
        assert!(
            stderr.contains("DEBUG: Pattern matching trace"),
            "--debug should write the evaluation trace to stderr, got: {stderr:?}"
        );
        assert!(
            stderr.contains(&format!("Command: {command}")),
            "the trace should name the evaluated command, got: {stderr:?}"
        );
        assert!(
            stderr.contains("Final verdict:"),
            "the trace should end in a verdict, got: {stderr:?}"
        );

        let plain_stderr = String::from_utf8(plain.stderr).expect("stderr should be UTF-8");
        assert!(
            plain_stderr.is_empty(),
            "without --debug stderr must stay silent for {command:?}; got: {plain_stderr:?}"
        );
    }

    // The trace shows its work, not just the outcome: the deny case names the
    // pack that dispatched, the pattern that matched, and the verdict.
    let denied = run_check(
        &pack_path,
        &["--command", "git reset --hard HEAD~1", "--debug"],
    );
    let stderr = String::from_utf8(denied.stderr).expect("stderr should be UTF-8");
    for expected in [
        "Pack dispatched: check-output-contract",
        "deny-reset: MATCH",
        "Final verdict: DENY",
    ] {
        assert!(
            stderr.contains(expected),
            "deny trace should contain {expected:?}, got: {stderr:?}"
        );
    }
}

#[test]
fn file_dash_stdin_content_reaches_every_decision_and_exits_successfully() {
    let temp = tempdir().expect("temporary directory should be created");
    let pack_path = write_content_pack(temp.path());

    for (content, expected_line) in CONTENT_CASES {
        let output = run_check_with_stdin(&pack_path, content, &[]);
        assert_eq!(
            output.status.code(),
            Some(0),
            "`icg check --file -` must exit 0 for {expected_line:?}: the advisory \
             contract covers content mode too, and a failing status would make \
             every piped-manifest outcome look like a guard crash"
        );
        let stdout = String::from_utf8(output.stdout).expect("stdout should be UTF-8");
        assert!(
            stdout.starts_with(expected_line),
            "piped content {content:?} should open stdout with {expected_line:?}, got: {stdout:?}"
        );

        let stderr = String::from_utf8(output.stderr).expect("stderr should be UTF-8");
        assert!(
            stderr.is_empty(),
            "a plain `--file -` check must keep stderr silent so callers can \
             treat it as fault-only; got: {stderr:?}"
        );
    }
}

#[test]
fn file_dash_content_decisions_carry_parseable_attribution_on_stdout() {
    let temp = tempdir().expect("temporary directory should be created");
    let pack_path = write_content_pack(temp.path());

    // The advisory decisions name the pack and pattern that fired, so a
    // caller parsing stdout can attribute a piped-manifest verdict without
    // a second invocation -- the same attribution `--command` prints.
    let denied = run_check_with_stdin(&pack_path, "probe_deny_marker: on\n", &[]);
    let stdout = String::from_utf8(denied.stdout).expect("stdout should be UTF-8");
    for expected in [
        "Reason: Do not write the deny marker",
        "Pack: file-dash-contract",
        "Pattern: content-deny-marker",
    ] {
        assert!(
            stdout.contains(expected),
            "content deny stdout should carry {expected:?}, got: {stdout:?}"
        );
    }

    let rewritten = run_check_with_stdin(&pack_path, "probe_rewrite_marker: off\n", &[]);
    let stdout = String::from_utf8(rewritten.stdout).expect("stdout should be UTF-8");
    for expected in [
        "REWRITE: Use the pinned form",
        "Suggested input: probe_rewrite_marker: on",
        "Pack: file-dash-contract",
        "Pattern: content-rewrite-marker",
    ] {
        assert!(
            stdout.contains(expected),
            "content rewrite stdout should carry {expected:?}, got: {stdout:?}"
        );
    }

    let warned = run_check_with_stdin(&pack_path, "probe_warn_marker: maybe\n", &[]);
    let stdout = String::from_utf8(warned.stdout).expect("stdout should be UTF-8");
    for expected in [
        "WARNING: Check the warn marker",
        "Pack: file-dash-contract",
        "Pattern: content-warn-marker",
    ] {
        assert!(
            stdout.contains(expected),
            "content warning stdout should carry {expected:?}, got: {stdout:?}"
        );
    }
}

#[test]
fn file_dash_debug_writes_diagnostics_to_stderr_and_leaves_stdout_parseable() {
    let temp = tempdir().expect("temporary directory should be created");
    let pack_path = write_content_pack(temp.path());

    for (content, _) in CONTENT_CASES {
        let plain = run_check_with_stdin(&pack_path, content, &[]);
        let debugged = run_check_with_stdin(&pack_path, content, &["--debug"]);

        assert_eq!(
            debugged.status.code(),
            Some(0),
            "--debug must not change the advisory exit status for piped content {content:?}"
        );
        assert_eq!(
            debugged.stdout, plain.stdout,
            "--debug must leave `--file -` stdout byte-identical for {content:?}; the \
             decision stream is parsed by callers that never asked for traces"
        );

        let stderr = String::from_utf8(debugged.stderr).expect("stderr should be UTF-8");
        assert!(
            stderr.contains("Loaded 1 rule pack(s)"),
            "--debug should announce the loaded packs on stderr, got: {stderr:?}"
        );
        assert!(
            stderr.contains("DEBUG: Pattern matching trace is available for command checks"),
            "content mode has no command trace to print, so --debug should say so on \
             stderr rather than polluting stdout, got: {stderr:?}"
        );

        let plain_stderr = String::from_utf8(plain.stderr).expect("stderr should be UTF-8");
        assert!(
            plain_stderr.is_empty(),
            "without --debug stderr must stay silent for piped content {content:?}; got: {plain_stderr:?}"
        );
    }
}
