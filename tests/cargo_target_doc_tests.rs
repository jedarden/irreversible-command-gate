//! The canonical cargo target-directory workflow, held against the docs.
//!
//! Host policy (the fleet cargo wrapper, `needle-d6b685b4`) pins **one
//! target directory per repo**: `/build/<origin-basename>`, matched by the
//! origin URL -- or, for a `git archive` extraction with no `.git`, by the
//! Cargo.toml identity. The wrapper overrides any other `CARGO_TARGET_DIR`
//! and refuses a `--target-dir` outside that directory; only its
//! subdirectories are kept.
//!
//! An earlier generation of this repository's guidance described the model
//! the wrapper has since replaced: a single shared `/build/target-workers`
//! that `CARGO_TARGET_DIR` "moves per invocation", with examples telling
//! the reader to export a per-run target dir to dodge the shared dir's
//! cargo lock. That guidance is now wrong twice over -- the shared dir is no
//! longer what a bare invocation gets, and a per-run `CARGO_TARGET_DIR` is
//! silently overridden (a `--target-dir` refused outright), so the
//! documented command would not do what the doc says: the binary lands
//! somewhere the doc's later steps do not look for it. These guards hold
//! every doc to the one canonical workflow: state the wrapper-pinned
//! `/build/<repo>` model, never name the superseded shared dir, and never
//! point a build outside the pinned dir -- not even in a comment, which
//! readers copy-paste as readily as commands.

use std::fs;
use std::path::{Path, PathBuf};

/// The wrapper-pinned target dir for this repo: `/build/` plus the origin
/// URL's basename, exactly as the wrapper computes it.
const PINNED_TARGET_DIR: &str = "/build/irreversible-command-gate";

/// The checkout this run audits.
///
/// Same rationale as `documentation_consistency_tests::audited_checkout`:
/// the pinned per-repo target dir serves test binaries built from *other*
/// checkouts of this repo whenever the fingerprint looks fresh, so a baked
/// `CARGO_MANIFEST_DIR` can name a different tree than the one being
/// audited. Cargo runs test binaries with the package root as the working
/// directory, so prefer the runtime cwd; fall back to the baked path only
/// when it does not name a checkout.
fn audited_checkout() -> PathBuf {
    if let Ok(cwd) = std::env::current_dir() {
        if cwd.join("Cargo.toml").exists() {
            return cwd;
        }
    }
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// Every markdown doc in the repository: the root README and AGENTS guide,
/// plus everything under `docs/` at any depth. Sorted so a failure lists
/// offenders deterministically.
fn markdown_docs(root: &Path) -> Vec<PathBuf> {
    let mut docs = vec![root.join("README.md"), root.join("AGENTS.md")];
    collect_markdown(&root.join("docs"), &mut docs);
    docs.sort();
    docs
}

fn collect_markdown(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_markdown(&path, out);
        } else if path.extension().and_then(|e| e.to_str()) == Some("md") {
            out.push(path);
        }
    }
}

/// The lines of a doc that carry instruction semantics: everything, for a
/// shell/tape asset; for markdown, only the lines inside ``` fences --
/// prose may discuss the policy (these guards exist because it now
/// legitimately does), but a fenced example is a command a reader runs.
fn instruction_lines(text: &str, is_markdown: bool) -> Vec<(usize, &str)> {
    let mut lines = Vec::new();
    let mut fenced = false;
    for (index, line) in text.lines().enumerate() {
        if is_markdown && line.trim_start().starts_with("```") {
            fenced = !fenced;
            continue;
        }
        if is_markdown && !fenced {
            continue;
        }
        lines.push((index + 1, line));
    }
    lines
}

/// Every `CARGO_TARGET_DIR`/`--target-dir` setting on a line, as
/// (kind, unquoted value) pairs. Handles both `VAR=value` /
/// `--flag=value` words and the two-word `--flag value` form, including
/// after `export`.
fn target_dir_settings(line: &str) -> Vec<(&'static str, String)> {
    let words: Vec<&str> = line.split_whitespace().collect();
    let mut found = Vec::new();
    let quoted = |value: &str| value.trim_matches(|c| c == '"' || c == '\'').to_string();
    for word in &words {
        if let Some(value) = word.strip_prefix("CARGO_TARGET_DIR=") {
            found.push(("CARGO_TARGET_DIR", quoted(value)));
        }
        if let Some(value) = word.strip_prefix("--target-dir=") {
            found.push(("--target-dir", quoted(value)));
        }
    }
    for pair in words.windows(2) {
        if pair[0] == "--target-dir" {
            found.push(("--target-dir", quoted(pair[1])));
        }
    }
    found
}

fn is_inside_pinned_target_dir(value: &str) -> bool {
    value == PINNED_TARGET_DIR || value.starts_with(&format!("{PINNED_TARGET_DIR}/"))
}

/// AGENTS.md is the canonical statement of the workflow; if its description
/// of the pinned target dir erodes, every other doc loses its reference.
#[test]
fn agents_md_states_the_wrapper_pinned_target_workflow() {
    let text =
        fs::read_to_string(audited_checkout().join("AGENTS.md")).expect("AGENTS.md readable");
    let normalized: String = text.split_whitespace().collect::<Vec<_>>().join(" ");
    for marker in [
        "one target directory per repo",
        PINNED_TARGET_DIR,
        "overrides any other `CARGO_TARGET_DIR`",
        "refuses a `--target-dir`",
        "subdirectory",
    ] {
        assert!(
            normalized.contains(marker),
            "AGENTS.md must keep stating the canonical cargo target workflow; \
             lost marker: {marker:?}"
        );
    }
}

/// `/build/target-workers` was the pre-wrapper shared target dir. Any doc
/// still naming it describes a workflow the wrapper now overrides -- the
/// exact drift this file exists to keep from coming back.
#[test]
fn no_doc_names_the_superseded_shared_target_dir() {
    let root = audited_checkout();
    let mut offenders = Vec::new();
    for doc in markdown_docs(&root) {
        let text = fs::read_to_string(&doc)
            .unwrap_or_else(|error| panic!("should read {}: {error}", doc.display()));
        if text.contains("target-workers") {
            let relative = doc.strip_prefix(&root).unwrap_or(&doc).display();
            offenders.push(relative.to_string());
        }
    }
    assert!(
        offenders.is_empty(),
        "docs still name the superseded shared /build/target-workers target \
         dir; state the wrapper-pinned {PINNED_TARGET_DIR} model instead:\n  {}",
        offenders.join("\n  ")
    );
}

/// A fenced example that sets `CARGO_TARGET_DIR` or `--target-dir` must
/// point at the pinned dir or one of its subdirs -- the only values the
/// wrapper honors. Anything else is silently overridden (env var) or
/// refused outright (flag), so the documented command builds somewhere its
/// own next step does not look.
#[test]
fn fenced_examples_never_point_a_build_outside_the_pinned_target_dir() {
    let root = audited_checkout();
    let mut docs = markdown_docs(&root);
    // Shell/tape assets are all instruction -- no fences to stay inside.
    docs.push(root.join("docs/assets/demo.sh"));
    docs.push(root.join("docs/assets/demo.tape"));

    let mut offenders = Vec::new();
    for doc in &docs {
        let text = fs::read_to_string(doc)
            .unwrap_or_else(|error| panic!("should read {}: {error}", doc.display()));
        let is_markdown = doc.extension().and_then(|e| e.to_str()) == Some("md");
        let relative = doc.strip_prefix(&root).unwrap_or(doc).display();
        for (number, line) in instruction_lines(&text, is_markdown) {
            for (kind, value) in target_dir_settings(line) {
                if !is_inside_pinned_target_dir(&value) {
                    offenders.push(format!(
                        "{relative}:{number}: sets {kind} to {value:?} -- outside \
                         {PINNED_TARGET_DIR}, so the wrapper overrides it \
                         (CARGO_TARGET_DIR) or refuses it (--target-dir); point it \
                         at the pinned dir or a subdir of it"
                    ));
                }
            }
        }
    }
    assert!(
        offenders.is_empty(),
        "docs carry build instructions the wrapper will not honor:\n  {}",
        offenders.join("\n  ")
    );
}
