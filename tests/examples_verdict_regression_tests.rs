//! Pins docs/examples/README.md's inline transcripts to the real binary
//! against the repo's own packs/ (irrevers-08acc8ce). The scenario flow
//! tests (operator_scenarios, developer_scenarios, integration_scenarios)
//! own the fixture-backed steps and examples_coverage owns the audit
//! index, but neither walks the printed command-and-expected-output
//! pairs, so a pack change that invalidated a printed transcript stayed
//! invisible while every flow stayed green: Scenario 1 promised `ALLOW`
//! for a secret read the shipped openbao pack warns on, attributed the
//! denial to a `vault` pack that does not ship, Scenario 7 promised
//! `--force-with-lease` was allowed while the shipped git pack rewrites
//! it, and Scenario 10 still listed mutating kubectl verbs as uncovered
//! after the kubectl pack shipped. Like demo_verdict_regression_tests,
//! the pins go past the verdict prefix: each transcript is held to the
//! pack and pattern id printed next to it (the identifier `icg explain`
//! accepts) and to the alternative its channel promises, so example
//! drift fails CI instead of misleading a new operator. When a pinned
//! transcript moves, update docs/examples/README.md and the matrix here
//! together.

use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use tempfile::tempdir;

/// What the transcript's verdict promises the reader can do next, one
/// variant per channel: a denial redirects, a rewrite suggests the plain
/// input, the stdout-read warning names the no-reveal metadata read, and
/// an allow promises nothing beyond the absence of a verdict.
enum Alternative {
    Redirect,
    SuggestedPlainPush,
    MetadataGetHint,
    None,
}

/// How the block feeds the input: a `--command` string, or a stdin
/// PreToolUse payload with the `--harness` flag the block passes when it
/// passes one.
enum Invocation {
    Command(&'static str),
    Stdin {
        payload: &'static str,
        harness: Option<&'static str>,
    },
}

/// One printed transcript in docs/examples/README.md: the exact input the
/// block shows, the verdict prefix the block promises, and the rule the
/// block attributes the verdict to, in document order. `rule` is `None`
/// only for Scenario 3's allowed redirect target -- the block promises
/// the command executes, which means no rule fired, so rule attribution
/// there would be the safe-pattern ordering moving, not a detail.
struct PinnedTranscript {
    scenario: &'static str,
    invocation: Invocation,
    verdict: &'static str,
    rule: Option<(&'static str, &'static str)>,
    alternative: Alternative,
}

const PINNED_TRANSCRIPTS: &[PinnedTranscript] = &[
    PinnedTranscript {
        scenario: "Scenario 1, Step 4 (dangerous command)",
        invocation: Invocation::Stdin {
            payload: r#"{"toolName":"Bash","toolInput":{"command":"vault kv destroy secret/test"}}"#,
            harness: None,
        },
        verdict: "DENIED",
        rule: Some(("openbao", "openbao-destructive-verb")),
        alternative: Alternative::Redirect,
    },
    PinnedTranscript {
        scenario: "Scenario 1, Step 4 (stdout read)",
        invocation: Invocation::Stdin {
            payload: r#"{"toolName":"Bash","toolInput":{"command":"vault kv get secret/test"}}"#,
            harness: None,
        },
        verdict: "WARNING",
        rule: Some(("openbao", "openbao-kv-get-to-stdout")),
        alternative: Alternative::MetadataGetHint,
    },
    PinnedTranscript {
        scenario: "Scenario 3, Step 1 (denial envelope)",
        invocation: Invocation::Command("vault kv destroy secret/app/api-key"),
        verdict: "DENIED",
        rule: Some(("openbao", "openbao-destructive-verb")),
        alternative: Alternative::Redirect,
    },
    PinnedTranscript {
        scenario: "Scenario 3, Step 3 (redirect target)",
        invocation: Invocation::Command("vault kv patch secret/app/api-key -remove=expired_field"),
        verdict: "ALLOW",
        rule: None,
        alternative: Alternative::None,
    },
    PinnedTranscript {
        scenario: "Scenario 7, Step 4 (--force)",
        invocation: Invocation::Command("git push --force origin main"),
        verdict: "REWRITE",
        rule: Some(("git", "git-force-push")),
        alternative: Alternative::SuggestedPlainPush,
    },
    PinnedTranscript {
        scenario: "Scenario 7, Step 4 (-f)",
        invocation: Invocation::Command("git push -f origin main"),
        verdict: "REWRITE",
        rule: Some(("git", "git-force-push")),
        alternative: Alternative::SuggestedPlainPush,
    },
    PinnedTranscript {
        scenario: "Scenario 7, Step 4 (--force-with-lease)",
        invocation: Invocation::Command("git push --force-with-lease origin main"),
        verdict: "REWRITE",
        rule: Some(("git", "git-force-push")),
        alternative: Alternative::SuggestedPlainPush,
    },
    PinnedTranscript {
        scenario: "Scenario 11, Step 3 (apply_patch, storage class)",
        invocation: Invocation::Stdin {
            payload: r#"{"toolName":"apply_patch","toolInput":{"command":"*** Begin Patch\n*** Update File: deployment.yaml\n+storageClassName: ssd\n*** End Patch"}}"#,
            harness: Some("claude-code"),
        },
        verdict: "DENIED",
        rule: Some(("storage-class", "storage-class-ssd")),
        alternative: Alternative::Redirect,
    },
    PinnedTranscript {
        scenario: "Scenario 11, Step 3 (apply_patch, image tag)",
        invocation: Invocation::Stdin {
            payload: r#"{"toolName":"apply_patch","toolInput":{"command":"*** Begin Patch\n*** Update File: deployment.yaml\n+image: app:latest\n*** End Patch"}}"#,
            harness: Some("codex-cli"),
        },
        verdict: "DENIED",
        rule: Some(("image-tag", "image-tag-latest")),
        alternative: Alternative::Redirect,
    },
];

/// Reused test binaries bake `CARGO_MANIFEST_DIR` of a dead extraction (see
/// demo_verdict_regression_tests::repo_root); prefer the runtime cwd, which
/// cargo sets to the package root of the tree under test.
fn repo_root() -> PathBuf {
    if let Ok(cwd) = std::env::current_dir() {
        if cwd.join("Cargo.toml").exists() {
            return cwd;
        }
    }
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

/// Runs one pinned transcript through the real binary. Explicit `--pack`,
/// not the cwd default, so a host's installed packs never decide what the
/// README's own transcript shows; a private denial-log sink keeps the
/// deny-path inputs off any instrumented host log, exactly as
/// demo_verdict_regression_tests does.
fn run_transcript(root: &Path, transcript: &PinnedTranscript) -> Output {
    let sink = tempdir().expect("denial-log sink directory should be created");
    let packs = root.join("packs").to_string_lossy().into_owned();
    let mut command = Command::new(env!("CARGO_BIN_EXE_icg"));
    command
        .arg("check")
        .arg("--pack")
        .arg(&packs)
        .current_dir(root)
        .env("ICG_DENIAL_LOG", sink.path().join("denials.jsonl"));

    match &transcript.invocation {
        Invocation::Command(exact) => command.args(["--command", exact]).output().expect(
            "icg check should run -- cargo must build the icg binary for integration tests",
        ),
        Invocation::Stdin { payload, harness } => {
            if let Some(harness) = harness {
                command.args(["--harness", harness]);
            }
            command.arg("--stdin");
            use std::io::Write as _;
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
                .write_all(payload.as_bytes())
                .expect("stdin should accept the PreToolUse payload");
            child.wait_with_output().expect("icg check should finish")
        }
    }
}

fn check_success(transcript: &PinnedTranscript, output: &Output) -> String {
    format!(
        "{} exited non-zero (icg check is advisory and must always exit 0)\n\
         stdout: {}\nstderr: {}",
        transcript.scenario,
        stdout(output),
        stderr(output)
    )
}

#[test]
fn documented_example_transcripts_reproduce_their_documented_verdicts() {
    let root = repo_root();
    assert!(
        root.join("packs").is_dir(),
        "repo packs/ not found under {}",
        root.display()
    );

    for transcript in PINNED_TRANSCRIPTS {
        let output = run_transcript(&root, transcript);
        assert!(
            output.status.success(),
            "{}",
            check_success(transcript, &output)
        );
        let out = stdout(&output);
        let first_line = out.lines().next().unwrap_or_default().to_string();
        assert!(
            first_line.starts_with(transcript.verdict),
            "{} printed {:?} -- docs/examples/README.md's transcript claims this input \
             {}, so the walkthrough no longer matches the repo's behaviour; update \
             README.md and the matrix together\nstdout: {}\nstderr: {}",
            transcript.scenario,
            first_line,
            transcript.verdict,
            out,
            stderr(&output)
        );
    }
}

#[test]
fn documented_example_transcripts_carry_their_documented_rule_attribution() {
    let root = repo_root();
    for transcript in PINNED_TRANSCRIPTS {
        let output = run_transcript(&root, transcript);
        assert!(
            output.status.success(),
            "{}",
            check_success(transcript, &output)
        );
        let out = stdout(&output);

        match transcript.rule {
            Some((pack, pattern)) => {
                let pack_line = format!("Pack: {pack}");
                let pattern_line = format!("Pattern: {pattern}");
                assert!(
                    out.contains(&pack_line) && out.contains(&pattern_line),
                    "{} did not attribute its verdict to {pack_line:?} / \
                     {pattern_line:?} -- the README's transcript prints that \
                     attribution (the identifier `icg explain` accepts), so a \
                     reader following the scenario would land on a verdict they \
                     cannot look up; update README.md and the matrix together\n\
                     stdout: {out}\nstderr: {}",
                    transcript.scenario,
                    stderr(&output)
                );
            }
            None => assert!(
                !out.contains("Pattern:"),
                "{} carried rule attribution -- the README promises this input \
                 executes, so a rule firing means the safe-pattern ordering moved \
                 and the scenario's redirect target is no longer clean\n\
                 stdout: {out}",
                transcript.scenario
            ),
        }
    }
}

#[test]
fn documented_example_transcripts_keep_their_alternative_actionable() {
    let root = repo_root();
    for transcript in PINNED_TRANSCRIPTS {
        let output = run_transcript(&root, transcript);
        assert!(
            output.status.success(),
            "{}",
            check_success(transcript, &output)
        );
        let out = stdout(&output);

        match transcript.alternative {
            Alternative::None => {}
            Alternative::Redirect => assert!(
                out.lines()
                    .filter_map(|line| line.strip_prefix("Redirect: "))
                    .any(|text| !text.trim().is_empty()),
                "{} denied without a non-empty Redirect -- the README's transcript \
                 shows the reader the alternative, so the documented refusal must \
                 carry one\nstdout: {out}",
                transcript.scenario
            ),
            Alternative::SuggestedPlainPush => assert!(
                out.contains("Suggested input: git push origin main"),
                "{} rewrote without suggesting the plain push the README's \
                 transcript shows -- the scenario's reader could not act on it\n\
                 stdout: {out}",
                transcript.scenario
            ),
            Alternative::MetadataGetHint => assert!(
                out.contains("bao kv metadata get"),
                "{} warned without naming the no-reveal metadata read the README's \
                 transcript shows -- the scenario's reader could not act on it\n\
                 stdout: {out}",
                transcript.scenario
            ),
        }
    }
}

/// Scenario 3, Step 2 prints `icg explain --pattern openbao-destructive-verb`'s
/// output. The pack suites own the wording; this holds the transcript to the
/// facts a reader acts on: the rule is what the block says it is, it denies,
/// and the Alternative line carries the redirect the next step follows.
#[test]
fn documented_explain_output_reports_the_facts_scenario_3_documents() {
    let root = repo_root();
    let packs = root.join("packs").to_string_lossy().into_owned();
    let output = Command::new(env!("CARGO_BIN_EXE_icg"))
        .args([
            "explain",
            "--pack",
            &packs,
            "--pattern",
            "openbao-destructive-verb",
        ])
        .current_dir(&root)
        .output()
        .expect("icg explain should run -- cargo must build the icg binary for integration tests");
    assert!(
        output.status.success(),
        "icg explain exited non-zero\nstdout: {}\nstderr: {}",
        stdout(&output),
        stderr(&output)
    );

    let out = stdout(&output);
    for expected in [
        "Pattern: openbao-destructive-verb",
        "Pack: openbao",
        "Severity: Critical",
        "Redirect channel: Deny",
    ] {
        assert!(
            out.contains(expected),
            "Scenario 3's explain transcript is missing {expected:?} -- update \
             README.md and this pin together\nstdout: {out}\nstderr: {}",
            stderr(&output)
        );
    }
    assert!(
        out.contains("Alternative: This is an irreversible OpenBao operation"),
        "Scenario 3's explain transcript lost the Alternative line its next step \
         follows (the reader runs the patch command the rule recommends)\n\
         stdout: {out}"
    );
}

/// Scenarios 1 and 10 print `icg coverage --list`. The transcript's claim a
/// reader relies on is that every shipped pack is listed (Scenario 10's
/// overlap/gap prose is derived from it), not any one count -- a pack count
/// here would duplicate the per-pack suites and go stale on every new pack.
#[test]
fn coverage_list_enumerates_every_shipped_pack() {
    let root = repo_root();
    let packs_dir = root.join("packs");
    let shipped: Vec<String> = std::fs::read_dir(&packs_dir)
        .expect("repo packs/ should be readable")
        .filter_map(|entry| {
            let path = entry.expect("packs/ entry should stat").path();
            if path
                .extension()
                .is_some_and(|extension| extension == "json")
            {
                path.file_stem()
                    .map(|stem| stem.to_string_lossy().into_owned())
            } else {
                None
            }
        })
        .collect();
    assert!(
        !shipped.is_empty(),
        "no packs found under {} -- the packs/ layout moved",
        packs_dir.display()
    );

    let packs = packs_dir.to_string_lossy().into_owned();
    let output = Command::new(env!("CARGO_BIN_EXE_icg"))
        .args(["coverage", "--list", "--pack", &packs])
        .current_dir(&root)
        .output()
        .expect("icg coverage should run -- cargo must build the icg binary for integration tests");
    assert!(
        output.status.success(),
        "icg coverage --list exited non-zero\nstdout: {}\nstderr: {}",
        stdout(&output),
        stderr(&output)
    );

    let out = stdout(&output);
    for pack in &shipped {
        let listing = format!("✓ pack {pack} (");
        assert!(
            out.contains(&listing),
            "icg coverage --list does not list shipped pack {pack:?} -- Scenarios 1 \
             and 10 print this listing, so an unlisted pack would make the README's \
             overlap/gap prose lie\nstdout: {out}"
        );
    }
}

/// Tripwire on the README side of the pins: the corrected transcripts stay
/// in the document, and the stale claims this suite replaced never come
/// back without their verdicts moving too. Coarse README-wide markers, the
/// same altitude examples_coverage pins the audit index at.
#[test]
fn examples_readme_stays_pinned_to_the_transcripts() {
    let readme =
        std::fs::read_to_string(repo_root().join("docs").join("examples").join("README.md"))
            .expect("docs/examples/README.md should be readable");

    for marker in [
        // Scenario 1, Step 4: the deny block names the shipping pack, and
        // the stdout read is pinned as the warning it is.
        "Pattern: openbao-destructive-verb",
        "Pattern: openbao-kv-get-to-stdout",
        // Scenario 3, Steps 1-2: the envelope and explain blocks use the
        // real output labels and the deny channel.
        "Redirect channel: Deny",
        // Scenario 7, Step 4: the force variants rewrite.
        "REWRITE: Removed --force/-f/--force-with-lease from git push",
        // Scenario 10, Step 2: the coverage transcript shows the kubectl
        // pack that owns the mutating verbs.
        "✓ pack kubectl (3 patterns)",
        // Scenario 11, Step 3: both apply_patch envelopes carry their
        // pattern ids.
        "Pattern: storage-class-ssd",
        "Pattern: image-tag-latest",
        // Scenario 1, Step 1: the manifest Step 2 verifies is downloaded.
        "pack-manifest.json",
    ] {
        assert!(
            readme.contains(marker),
            "docs/examples/README.md lost the pinned transcript marker \
             {marker:?} -- the verdict matrix in \
             tests/examples_verdict_regression_tests.rs pins that transcript; \
             update the block and the matrix together"
        );
    }

    for stale in [
        // Scenario 7's old claim that lease pushes pass: the shipped git
        // pack rewrites them.
        "ALLOWED (different pattern)",
        "--force: BLOCKED",
        // Scenario 3's old fabricated envelope labels: real output uses
        // `Pack:` / `Pattern:`.
        "Rule Pack:",
        "Pattern ID:",
        // Scenario 1's stale version literal: the binary auto-bumps.
        "icg 0.1.3",
    ] {
        assert!(
            !readme.contains(stale),
            "docs/examples/README.md revived the stale claim {stale:?} -- it \
             contradicts the shipped packs the verdict matrix pins; run the \
             documented command and print what the binary actually says"
        );
    }
}
