//! Pins docs/assets/demo.sh's verdicts to the claims README.md makes about
//! docs/assets/icg-demo.gif ("real `icg check` output", reproducible with
//! demo.sh). Each demo input implies one verdict -- `git status` is allowed,
//! a force-push is rewritten, an OpenBao read to stdout warns, and an
//! OpenBao destroy, a bare `git credential fill`, and a `:latest` image tag
//! in file content are each denied -- but the per-rule suites never walk the
//! script itself: a widened safe pattern or a renumbered redirect channel
//! could invalidate the front-page artifact while every one of them stayed
//! green (irrevers-36603bb6). These tests run every demo input through the
//! real binary against the repo's own packs/ -- the same `icg check`
//! surface demo.sh drives -- and hold demo.sh and the pinned matrix in
//! lockstep, so a new demo line cannot ship without a pinned verdict either.
//! When a pinned verdict moves, update demo.sh, this matrix, and the GIF
//! (recipe in demo.sh's header) together.

use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use tempfile::tempdir;

/// One demo.sh input and the verdict prefix README.md promises for it, in
/// demo order -- the order the GIF scrolls, and the order the README's
/// caption narrates.
enum DemoInput {
    Command {
        command: &'static str,
        verdict: &'static str,
    },
    Content {
        content: &'static str,
        verdict: &'static str,
    },
}

const PINNED_DEMO_INPUTS: &[DemoInput] = &[
    DemoInput::Command {
        command: "git status",
        verdict: "ALLOW",
    },
    DemoInput::Command {
        command: "git push --force origin main",
        verdict: "REWRITE",
    },
    DemoInput::Command {
        command: "bao kv get -field=token secret/app/db",
        verdict: "WARNING",
    },
    DemoInput::Command {
        command: "bao kv destroy secret/app/db",
        verdict: "DENIED",
    },
    DemoInput::Command {
        command: "git credential fill",
        verdict: "DENIED",
    },
    DemoInput::Content {
        content: "image: ronaldraygun/armor:latest\n",
        verdict: "DENIED",
    },
];

impl DemoInput {
    /// Mode-and-input pair compared against what the demo.sh parser finds.
    fn descriptor(&self) -> (String, String) {
        match self {
            DemoInput::Command { command, .. } => ("command".to_string(), (*command).to_string()),
            DemoInput::Content { content, .. } => ("content".to_string(), (*content).to_string()),
        }
    }

    /// The demo.sh spelling of this input, for failure messages that read
    /// like the script the reader can open.
    fn label(&self) -> String {
        match self {
            DemoInput::Command { command, .. } => format!("demo '{command}'"),
            DemoInput::Content { content, .. } => {
                format!("printf '%s' {content:?} | icg check --file -")
            }
        }
    }
}

/// Reused test binaries bake `CARGO_MANIFEST_DIR` of a dead extraction (see
/// documentation_consistency_tests::audited_checkout); prefer the runtime
/// cwd, which cargo sets to the package root of the tree under test.
fn repo_root() -> PathBuf {
    if let Ok(cwd) = std::env::current_dir() {
        if cwd.join("Cargo.toml").exists() {
            return cwd;
        }
    }
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// The single-quoted arguments starting at `cursor` (bash single quotes
/// cannot escape a quote, so `'...'` is unambiguous, including across the
/// newlines demo_file bodies contain).
fn read_single_quoted_args(text: &str, mut cursor: usize, count: usize) -> Option<Vec<String>> {
    let bytes = text.as_bytes();
    let mut args = Vec::with_capacity(count);
    for _ in 0..count {
        while cursor < bytes.len() && bytes[cursor].is_ascii_whitespace() {
            cursor += 1;
        }
        if bytes.get(cursor) != Some(&b'\'') {
            return None;
        }
        cursor += 1;
        let start = cursor;
        loop {
            if cursor >= bytes.len() {
                return None;
            }
            if bytes[cursor] == b'\'' {
                break;
            }
            cursor += 1;
        }
        args.push(text[start..cursor].to_string());
        cursor += 1;
    }
    Some(args)
}

/// Every input demo.sh feeds `icg check`, as (mode, input) pairs: the
/// command of each `demo '...'` call and the file body of each
/// `demo_file '<body>' ...` call, in script order. Byte offsets index the
/// whole text so a demo_file body's embedded newline does not end the
/// argument.
fn demo_script_inputs(script: &str) -> Vec<(String, String)> {
    let mut line_starts = vec![0usize];
    for (offset, _) in script.match_indices('\n') {
        line_starts.push(offset + 1);
    }

    let mut inputs = Vec::new();
    for start in line_starts {
        let rest = &script[start..];
        if rest.strip_prefix("demo ").is_some() {
            let args =
                read_single_quoted_args(script, start + "demo ".len(), 1).unwrap_or_else(|| {
                    panic!("demo call at byte {start} has no single-quoted command")
                });
            inputs.push(("command".to_string(), args[0].clone()));
        } else if rest.strip_prefix("demo_file ").is_some() {
            // Only the body: the label argument is double-quoted, and only
            // the body reaches `icg check --file -`.
            let args = read_single_quoted_args(script, start + "demo_file ".len(), 1)
                .unwrap_or_else(|| {
                    panic!("demo_file call at byte {start} has no single-quoted body")
                });
            inputs.push(("content".to_string(), args[0].clone()));
        }
    }
    inputs
}

fn run_check(root: &Path, input: &DemoInput) -> Output {
    // A private denial-log sink keeps the deny-path demo inputs off any
    // instrumented host log, exactly as check_output_contract_tests does;
    // the verdicts asserted below are read from stdout and are independent
    // of where a denial record lands.
    let sink = tempdir().expect("denial-log sink directory should be created");
    let packs = root.join("packs").to_string_lossy().into_owned();
    let mut command = Command::new(env!("CARGO_BIN_EXE_icg"));
    command
        .arg("check")
        // Explicit --pack, not the cwd default: the default also loads
        // /etc/icg/rule-pack.json and /etc/icg/packs when they exist, and a
        // host's installed packs must never decide what the README's own
        // artifact shows.
        .arg("--pack")
        .arg(&packs)
        .current_dir(root)
        .env("ICG_DENIAL_LOG", sink.path().join("denials.jsonl"));

    match input {
        DemoInput::Command {
            command: demo_command,
            ..
        } => command.args(["--command", demo_command]).output().expect(
            "icg check should run -- cargo must build the icg binary for integration tests",
        ),
        DemoInput::Content { content, .. } => {
            use std::io::Write as _;
            command.args(["--file", "-"]);
            let mut child = command
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .expect(
                    "icg check should run -- cargo must build the icg binary for integration tests",
                );
            child
                .stdin
                .take()
                .expect("stdin should be available")
                .write_all(content.as_bytes())
                .expect("stdin should accept the file body");
            child.wait_with_output().expect("icg check should finish")
        }
    }
}

#[test]
fn demo_script_and_pinned_matrix_stay_in_lockstep() {
    let script = std::fs::read_to_string(repo_root().join("docs").join("assets").join("demo.sh"))
        .expect("docs/assets/demo.sh should be readable");

    let parsed = demo_script_inputs(&script);
    assert!(
        !parsed.is_empty(),
        "demo.sh yielded no inputs -- the demo_script_inputs parser rotted into matching nothing"
    );

    let pinned: Vec<(String, String)> = PINNED_DEMO_INPUTS
        .iter()
        .map(DemoInput::descriptor)
        .collect();
    assert_eq!(
        parsed, pinned,
        "demo.sh's inputs and PINNED_DEMO_INPUTS diverged -- a demo line changed without \
         its verdict being re-pinned; update demo.sh, the matrix, and the GIF (see \
         demo.sh's header) together"
    );
}

#[test]
fn every_demo_input_reproduces_the_verdict_readme_promises() {
    let root = repo_root();
    assert!(
        root.join("packs").is_dir(),
        "repo packs/ not found under {}",
        root.display()
    );

    for input in PINNED_DEMO_INPUTS {
        let output = run_check(&root, input);
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        let verdict = match input {
            DemoInput::Command { verdict, .. } | DemoInput::Content { verdict, .. } => *verdict,
        };
        let label = input.label();

        assert!(
            output.status.success(),
            "{label} exited non-zero (icg check is advisory and must always exit 0)\n\
             stdout: {stdout}\nstderr: {stderr}"
        );
        let first_line = stdout.lines().next().unwrap_or_default();
        assert!(
            first_line.starts_with(verdict),
            "{label} printed {first_line:?} -- README.md's demo claims this input {verdict}, \
             so the GIF no longer matches the repo's behaviour\nstdout: {stdout}\nstderr: {stderr}"
        );
    }
}
