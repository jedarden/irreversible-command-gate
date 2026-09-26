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
//! The contract has a fault half too. When evaluation cannot happen at all
//! -- a pack path that does not exist, a pack that does not parse, no packs
//! installed, an unreadable `--file`, a missing input mode, an unparseable
//! `--stdin` payload -- `icg check` exits `1` with the error on stderr and
//! stdout left empty (quick-start.md documents the no-packs spelling
//! verbatim). stdout non-empty therefore means a verdict was actually
//! reached, so an adapter scraping stdout can never mistake broken
//! infrastructure for an ALLOW. The engine's own fail-open-on-parse-failure
//! posture is hook-facing: in check mode it is announced on stderr and
//! never becomes a stdout decision. The one fault-adjacent path that does
//! answer on stdout with exit 0 is the guard-disabled bypass, and
//! emergency_response_tests.rs pins that it announces itself there.
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

/// Run `icg check` with explicit arguments and extra environment, capturing
/// both streams. The happy-path helpers above fix the invocation shape; the
/// fault contract varies it (missing pack, missing mode flag, redirected
/// pack directory), so it drives the argument list after the `check`
/// subcommand instead.
fn run_check_raw(args: &[&str], envs: &[(&str, &std::path::Path)]) -> Output {
    // Same private denial-log sink as the happy-path helpers: a fault run
    // never evaluates, so nothing is recorded, but keeping the environment
    // identical means a fault test cannot pass because of where a denial
    // record would have landed.
    let sink = tempdir().expect("denial-log sink directory should be created");
    let mut command = Command::new(env!("CARGO_BIN_EXE_icg"));
    command
        .arg("check")
        .args(args)
        .env("ICG_DENIAL_LOG", sink.path().join("denials.jsonl"));
    for (key, value) in envs {
        command.env(key, value);
    }
    let output = command.output().expect("icg check should run");
    drop(sink);
    output
}

/// Run `icg check --stdin` with `payload` piped in, for the fault contract
/// of PreToolUse mode: an unparseable payload must fail the same way the
/// other input faults do, whatever the engine's internal posture was.
fn run_check_stdin_mode(pack_path: &std::path::Path, payload: &str) -> Output {
    let sink = tempdir().expect("denial-log sink directory should be created");
    let mut child = Command::new(env!("CARGO_BIN_EXE_icg"))
        .args([
            "check",
            "--pack",
            pack_path.to_str().expect("temporary path should be UTF-8"),
            "--stdin",
        ])
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
        .write_all(payload.as_bytes())
        .expect("stdin payload should be written");
    let output = child.wait_with_output().expect("icg check should finish");
    drop(sink);
    output
}

#[test]
fn every_fault_exits_one_with_an_empty_stdout_and_the_error_on_stderr() {
    let temp = tempdir().expect("temporary directory should be created");
    let pack_path = write_pack(temp.path());
    let pack_str = pack_path.to_str().expect("temporary path should be UTF-8");

    let broken_pack = temp.path().join("broken-pack.json");
    std::fs::write(&broken_pack, "{ not json").expect("broken pack should be written");
    let broken_str = broken_pack
        .to_str()
        .expect("temporary path should be UTF-8");

    let missing_pack = temp.path().join("does-not-exist.json");
    let missing_str = missing_pack
        .to_str()
        .expect("temporary path should be UTF-8");

    let missing_file = temp.path().join("no-such-target.yaml");
    let missing_file_str = missing_file
        .to_str()
        .expect("temporary path should be UTF-8");

    let empty_pack_dir = temp.path().join("no-packs-here");
    std::fs::create_dir(&empty_pack_dir).expect("empty pack directory should be created");

    // (arguments, extra environment, a fragment of the fault it must report)
    let faults: Vec<(Vec<&str>, Vec<(&str, &std::path::Path)>, &str)> = vec![
        (
            vec!["--pack", broken_str, "--command", "git status"],
            vec![],
            "Error: failed to load rule pack",
        ),
        (
            vec!["--pack", missing_str, "--command", "git status"],
            vec![],
            "Error: rule-pack path does not exist",
        ),
        (
            // No explicit pack and an ICG_PACK_DIR with no packs in it: the
            // documented no-policy spelling, quoted verbatim in
            // docs/quick-start.md.
            vec!["--command", "git status"],
            vec![("ICG_PACK_DIR", empty_pack_dir.as_path())],
            "Error: no rule packs found; pass --pack <path>",
        ),
        (
            vec!["--pack", pack_str, "--file", missing_file_str],
            vec![],
            "Error: failed to read file",
        ),
        (
            vec!["--pack", pack_str],
            vec![],
            "Error: one of --command, --stdin, or --file is required",
        ),
    ];

    for (args, envs, expected_fragment) in faults {
        let output = run_check_raw(&args, &envs);
        assert_eq!(
            output.status.code(),
            Some(1),
            "a fault that prevented evaluation must exit 1, not 0: the exit \
             status is the only fault signal a stdout-parsing caller has left, \
             and zero would read as the documented advisory success \
             (args: {args:?})"
        );
        let stdout = String::from_utf8(output.stdout).expect("stdout should be UTF-8");
        assert!(
            stdout.is_empty(),
            "a fault must keep stdout empty so no decision line can be \
             fabricated from broken infrastructure (args: {args:?}, got: \
             {stdout:?})"
        );
        let stderr = String::from_utf8(output.stderr).expect("stderr should be UTF-8");
        assert!(
            stderr.contains("Error:"),
            "the fault must be reported on stderr as an error (args: {args:?}, \
             got: {stderr:?})"
        );
        assert!(
            stderr.contains(expected_fragment),
            "the fault should name what broke (args: {args:?}, expected \
             {expected_fragment:?}, got: {stderr:?})"
        );
    }
}

#[test]
fn stdin_parse_failure_announces_fail_open_on_stderr_and_never_a_verdict_on_stdout() {
    let temp = tempdir().expect("temporary directory should be created");
    let pack_path = write_pack(temp.path());

    // The engine fails open on unparseable hook input -- that posture is
    // what keeps the hook front end permissive when a harness sends garbage.
    // The check front end must surface that posture as the stderr notice it
    // is, and still refuse to render a decision: nothing was evaluated, so
    // stdout carries no ALLOW and the exit is the documented fault exit.
    let output = run_check_stdin_mode(&pack_path, "not a PreToolUse payload");

    assert_eq!(
        output.status.code(),
        Some(1),
        "an unparseable --stdin payload must exit 1: the engine's fail-open \
         posture belongs to the hook front end, and a zero exit here would \
         tell adapters the payload was evaluated"
    );
    let stdout = String::from_utf8(output.stdout).expect("stdout should be UTF-8");
    assert!(
        stdout.is_empty(),
        "a failed stdin parse must not announce any verdict on stdout, got: \
         {stdout:?}"
    );
    let stderr = String::from_utf8(output.stderr).expect("stderr should be UTF-8");
    assert!(
        stderr.contains("fail-open"),
        "the engine's fail-open notice should stay on stderr, got: {stderr:?}"
    );
    assert!(
        stderr.contains("stdin did not contain a valid PreToolUse request"),
        "the fault explanation should name the parse failure, got: {stderr:?}"
    );
}
