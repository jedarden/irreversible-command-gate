//! `docs/notes/coverage-json-api.md` — the note's field tables and the
//! emitted `coverage/v1` document agree.
//!
//! `tests/coverage_json_tests.rs` pins the behavior the note documents:
//! key sets, ordering, unreadable-pack reporting, the failure modes. This
//! file closes the remaining direction of the contract: the note's own
//! field tables must describe exactly what the command emits, so a field
//! renamed in code or a table row edited in prose fails here instead of
//! leaving integrators reading a contract the binary no longer speaks.
//! The catalog side of the same agreement is
//! `catalog_export_tests::catalog_note_field_tables_match_the_export`.

use serde_json::Value;
use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use tempfile::TempDir;

const COVERAGE_FORMAT: &str = "coverage/v1";

fn packs_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("packs")
}

fn icg(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_icg"))
        .args(args)
        .output()
        .expect("icg should run")
}

/// Run `coverage --list --format json` against explicit `--pack` paths.
fn coverage_json_against(packs: &[&Path]) -> Output {
    let mut args = vec!["coverage", "--list", "--format", "json"];
    for pack in packs {
        args.push("--pack");
        args.push(pack.to_str().expect("pack path is utf-8"));
    }
    icg(&args)
}

fn coverage_json() -> Value {
    let dir = packs_dir();
    let output = coverage_json_against(&[&dir]);
    assert!(
        output.status.success(),
        "coverage --format json should succeed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).expect("coverage --format json should emit valid JSON")
}

/// The note's field rows between two headings, as a set of field names.
/// The table header (`| Field | ...`) and separator (`| --- | ...`) don't
/// start with `` | ` `` and so never match.
fn documented_fields(note: &str, section_start: &str, section_end: &str) -> BTreeSet<String> {
    let start = note
        .find(section_start)
        .unwrap_or_else(|| panic!("the note should contain {section_start:?}"));
    let body = &note[start + section_start.len()..];
    let body = match section_end {
        "" => body,
        _ => body.split(section_end).next().expect("non-empty split"),
    };
    body.lines()
        .filter_map(|line| line.trim().strip_prefix("| `"))
        .filter_map(|cell| cell.split('`').next())
        .map(|field| field.to_string())
        .collect()
}

fn emitted_keys(value: &Value) -> BTreeSet<String> {
    value
        .as_object()
        .expect("expected a JSON object")
        .keys()
        .cloned()
        .collect()
}

#[test]
fn coverage_note_field_tables_match_the_export() {
    let note = fs::read_to_string("docs/notes/coverage-json-api.md")
        .expect("the contract note should exist");

    assert!(
        note.contains(COVERAGE_FORMAT),
        "the note must name the wire contract {COVERAGE_FORMAT} verbatim"
    );
    // Same version policy the catalog note pins: the Versioning section may
    // cite a hypothetical next version as policy; anywhere else, the only
    // contract the note may name is the shipped one.
    let versioning = note
        .find("## Versioning")
        .expect("the note should have a Versioning section");
    assert!(
        !note[..versioning].contains("coverage/v2"),
        "outside the Versioning policy the note must not name an unshipped format version"
    );
    assert!(
        !note.contains("coverage/v3"),
        "the note must not promise a format version that does not exist"
    );

    let report = coverage_json();

    // Every level of the document is compared against the note's table for
    // that level — exact equality, because coverage/v1 has no optional
    // fields at any level. (The catalog's sanctioned_alternative carries an
    // optional `rewrite`, so its note test has to settle for a subset
    // check there; nothing here does.)
    let sync = |what: &str, documented: BTreeSet<String>, emitted: &BTreeSet<String>| {
        assert_eq!(
            documented, *emitted,
            "{what}: the note's field table and the emitted document disagree"
        );
    };

    sync(
        "top level",
        documented_fields(&note, "One JSON object on stdout", "Each entry of `packs`:"),
        &emitted_keys(&report),
    );

    let packs = report["packs"].as_array().expect("packs array");
    let pack = packs.first().expect("the shipped packs are never empty");
    sync(
        "pack entries",
        documented_fields(
            &note,
            "Each entry of `packs`:",
            "Each entry of `guarded_patterns`:",
        ),
        &emitted_keys(pack),
    );

    let rules = pack["guarded_patterns"]
        .as_array()
        .expect("guarded_patterns array");
    let rule = rules.first().expect("the shipped packs guard something");
    sync(
        "guarded_patterns entries",
        documented_fields(
            &note,
            "Each entry of `guarded_patterns`:",
            "Each entry of `unreadable`:",
        ),
        &emitted_keys(rule),
    );

    // The unreadable table cannot be read off the all-ships document —
    // every shipped pack loads — so render one document where a corrupt
    // pack fails alongside a readable one and compare against its report.
    let dir = TempDir::new().expect("tempdir");
    fs::copy(
        packs_dir().join("tmux.json"),
        dir.path().join("readable.json"),
    )
    .expect("shipped tmux pack should copy");
    fs::copy(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/corrupt-rule-pack.json"),
        dir.path().join("corrupt.json"),
    )
    .expect("corrupt fixture should copy");

    let output = coverage_json_against(&[dir.path()]);
    assert!(
        output.status.success(),
        "a partially readable directory still succeeds: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let report: Value =
        serde_json::from_slice(&output.stdout).expect("partial readability still emits a document");
    let unreadable = report["unreadable"].as_array().expect("unreadable array");
    let entry = unreadable
        .first()
        .expect("the corrupt pack is reported in unreadable");
    sync(
        "unreadable entries",
        documented_fields(
            &note,
            "Each entry of `unreadable`:",
            "## Serialization contract",
        ),
        &emitted_keys(entry),
    );
}
