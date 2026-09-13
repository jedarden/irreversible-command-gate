//! `beads-shared-checkout-write` guards the bead STORE, not everything that
//! happens to live under `.beads/`.
//!
//! The rule matched `.beads/**` and fired 23 times in 21 hours on lab, every
//! one of them a scratch file: `.beads/state/<bead-id>/render_markdown.py`,
//! `.beads/logs/<id>-classification.json`, `sampler.sh`. Those directories
//! are gitignored, carry no tracked files, and are not bead-rs directories at
//! all -- bead-rs creates beads.db, checkpoint/, config.json, diagnostics/,
//! receipts/ and traces/. Under enforcement all 23 would have been blocked,
//! and the redirect would have told the agent to make a git worktree for a
//! throwaway script.
//!
//! The hazard the rule exists for is concurrent corruption of the store.
//! These tests pin the boundary.

use icg::engine::{CheckResult, ContentSource, Engine};
use icg::rule_pack::load_pack;
use std::fs;
use std::path::Path;
use tempfile::TempDir;

/// A shared primary checkout: `.git` is a directory.
fn shared_checkout() -> TempDir {
    let dir = TempDir::new().expect("tempdir");
    fs::create_dir_all(dir.path().join(".git")).expect(".git dir");
    fs::create_dir_all(dir.path().join(".beads/checkpoint")).expect("checkpoint");
    fs::create_dir_all(dir.path().join(".beads/state/domchk-884dd8ae")).expect("state");
    fs::create_dir_all(dir.path().join(".beads/logs")).expect("logs");
    fs::create_dir_all(dir.path().join(".beads/diagnostics")).expect("diagnostics");
    dir
}

/// A linked worktree: `.git` is a FILE. Never guarded -- that is the escape
/// hatch the rule's own message recommends.
fn linked_worktree() -> TempDir {
    let dir = TempDir::new().expect("tempdir");
    fs::write(
        dir.path().join(".git"),
        "gitdir: /elsewhere/.git/worktrees/w",
    )
    .expect(".git file");
    fs::create_dir_all(dir.path().join(".beads")).expect(".beads");
    dir
}

fn engine() -> Engine {
    let mut engine = Engine::new();
    let packs = Path::new(env!("CARGO_MANIFEST_DIR")).join("packs");
    let mut entries: Vec<_> = fs::read_dir(&packs)
        .expect("packs/")
        .map(|e| e.expect("entry").path())
        .filter(|p| p.extension().and_then(|e| e.to_str()) == Some("json"))
        .collect();
    entries.sort();
    for path in entries {
        engine
            .load_pack(load_pack(&path).expect("pack loads"))
            .expect("pack loads");
    }
    engine
}

fn denied(dir: &TempDir, relative: &str) -> bool {
    let path = dir.path().join(relative);
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    matches!(
        engine().evaluate_content(&ContentSource::Write {
            file_path: path.to_string_lossy().into_owned(),
            content: "scratch\n".to_string(),
        }),
        CheckResult::Denied { .. }
    )
}

/// The store itself. Concurrent writes here are the hazard.
#[test]
fn writing_the_bead_store_in_a_shared_checkout_is_denied() {
    let dir = shared_checkout();
    for target in [
        ".beads/beads.db",
        ".beads/checkpoint/current.json",
        ".beads/checkpoint/forensic.jsonl",
        ".beads/events.jsonl",
        ".beads/heartbeats.jsonl",
        ".beads/config.json",
    ] {
        assert!(
            denied(&dir, target),
            "{target} is bead-store state and must deny"
        );
    }
}

/// The 23 real paths lab recorded. Every one must allow.
#[test]
fn the_scratch_paths_lab_recorded_are_allowed() {
    let dir = shared_checkout();
    for target in [
        ".beads/state/domchk-884dd8ae/render_markdown.py",
        ".beads/state/domchk-884dd8ae/compile_report.py",
        ".beads/state/domchk-1835a393/sampler.sh",
        ".beads/state/domchk-ca6412a0/compute_unique_commits_authors.py",
        ".beads/state/domchk-990ef135/extract_traces.py",
        ".beads/state/domchk-53a64cb1/extract_commits.py",
        ".beads/state/domchk-82a54a7c/compute_divergence_metrics.py",
        ".beads/logs/bf-6d3d6-classification.json",
        ".beads/logs/bf-6d3d6-root-cause.json",
    ] {
        assert!(
            !denied(&dir, target),
            "{target} is scratch, not the store -- blocking it stops legitimate work"
        );
    }
}

/// Other tool-written directories under .beads/ are equally not the store.
#[test]
fn tool_written_side_directories_are_allowed() {
    let dir = shared_checkout();
    for target in [
        ".beads/diagnostics/starvation-report.jsonl",
        ".beads/traces/run-1.json",
        ".beads/receipts/r1.json",
        ".beads/crash-reports/c1.md",
    ] {
        assert!(
            !denied(&dir, target),
            "{target} is tool output, not the store"
        );
    }
}

/// The full week trial (2026-09-06..09-13, bead irrevers-19835ba1 →
/// irrevers-05855de6) on lab's v0.1.3 binary, which predates the narrowing.
///
/// 30 would-denies, 15 distinct paths, every one a domain-check Write under
/// `.beads/state/` or `.beads/logs/`: per-bead analysis scripts, step
/// summaries, and classification records. The predicate sees only the target
/// path, never the content, so replaying the 15 distinct paths covers all 30
/// recorded denials. `state/work-completion/` is included alongside them:
/// domain-check's own CLAUDE.md directs verify-work-completion.sh to write
/// there on every pre-close check, so blocking it breaks that repo's defined
/// workflow. All must allow, and the store must still deny -- both directions
/// in one place, as the verification record for that bead.
#[test]
fn the_week_trial_paths_replay_as_allow_and_the_store_still_denies() {
    let dir = shared_checkout();
    for target in [
        // The 15 distinct paths lab recorded, verbatim.
        ".beads/logs/bf-6d3d6-classification.json",
        ".beads/logs/bf-6d3d6-root-cause.json",
        ".beads/state/crash-prevention-testing/gc-bounds.md",
        ".beads/state/crash-prevention-testing/safeguards.md",
        ".beads/state/domchk-0bc507d3/domchk-da433598-verification-2026-09-08.md",
        ".beads/state/domchk-1835a393/sampler.sh",
        ".beads/state/domchk-53a64cb1/extract_commits.py",
        ".beads/state/domchk-82a54a7c/compute_divergence_metrics.py",
        ".beads/state/domchk-884dd8ae/compile_report.py",
        ".beads/state/domchk-884dd8ae/render_markdown.py",
        ".beads/state/domchk-990ef135/extract_traces.py",
        ".beads/state/domchk-ca6412a0/compute_unique_commits_authors.py",
        ".beads/state/safeguard-verification-2026-09-06/step1-summary.md",
        ".beads/state/safeguard-verification-2026-09-06/step2-summary.md",
        ".beads/state/safeguard-verification-2026-09-06/step3-summary.md",
        // Sanctioned by domain-check's CLAUDE.md (verify-work-completion.sh).
        ".beads/state/work-completion/domchk-0bc507d3.json",
    ] {
        assert!(
            !denied(&dir, target),
            "{target} was denied; the week trial proved these are legitimate writes"
        );
    }

    // The other direction: a direct write at the store must still deny.
    for target in [
        ".beads/beads.db",
        ".beads/beads.db-wal",
        ".beads/checkpoint/current.json",
        ".beads/config.json",
        ".beads/events.jsonl",
        ".beads/heartbeats.jsonl",
    ] {
        assert!(
            denied(&dir, target),
            "{target} is the store; narrowing for the trial paths must not open it"
        );
    }
}

/// A linked worktree is the sanctioned way to work on the store, so it must
/// never be guarded -- otherwise the redirect sends the caller somewhere that
/// is also blocked.
#[test]
fn a_linked_worktree_is_never_guarded() {
    let dir = linked_worktree();
    for target in [".beads/beads.db", ".beads/state/x/y.py"] {
        assert!(
            !denied(&dir, target),
            "{target} in a linked worktree must allow; .git is a file there"
        );
    }
}
