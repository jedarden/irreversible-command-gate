//! `icg coverage --format json` -- the machine-readable policy surface.
//!
//! Agents, bots and doc generators need to know what is enforced without
//! scraping `coverage --list`'s text. This pins the shape of that output and
//! keeps it in agreement with the packs on disk.

use serde_json::{json, Value};
use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use tempfile::TempDir;

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

/// The truncated-pack fixture every "this pack does not load" test shares.
fn corrupt_pack_contents() -> String {
    fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/corrupt-rule-pack.json"),
    )
    .expect("corrupt fixture should be readable")
}

/// The exact key set of a JSON object, for the key-set pins below. A
/// serde_json `Value` loses nothing here: its map is a set compare against
/// the documented field table, so an added, removed, or renamed field fails.
fn exact_keys(value: &Value) -> BTreeSet<&str> {
    value
        .as_object()
        .expect("expected a JSON object")
        .keys()
        .map(String::as_str)
        .collect()
}

fn coverage_json() -> Value {
    let packs = packs_dir();
    let output = icg(&[
        "coverage",
        "--list",
        "--format",
        "json",
        "--pack",
        packs.to_str().unwrap(),
    ]);
    assert!(
        output.status.success(),
        "coverage --format json should succeed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).expect("coverage --format json should emit valid JSON")
}

#[test]
fn coverage_json_reports_every_shipped_pack_and_rule() {
    let report = coverage_json();
    assert_eq!(report["format"], "coverage/v1");

    let mut on_disk_packs = BTreeSet::new();
    let mut on_disk_rules = BTreeSet::new();
    for entry in fs::read_dir(packs_dir()).expect("packs/ readable") {
        let path = entry.expect("entry").path();
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        let pack: Value = serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        on_disk_packs.insert(pack["id"].as_str().unwrap().to_owned());
        for rule in pack["guarded_patterns"].as_array().unwrap() {
            on_disk_rules.insert(rule["id"].as_str().unwrap().to_owned());
        }
    }

    let reported_packs: BTreeSet<String> = report["packs"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| p["id"].as_str().unwrap().to_owned())
        .collect();
    let reported_rules: BTreeSet<String> = report["packs"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|p| p["guarded_patterns"].as_array().unwrap())
        .map(|r| r["id"].as_str().unwrap().to_owned())
        .collect();

    assert_eq!(reported_packs, on_disk_packs);
    assert_eq!(reported_rules, on_disk_rules);
    assert_eq!(
        report["pack_count"].as_u64().unwrap() as usize,
        on_disk_packs.len()
    );
    assert_eq!(
        report["guarded_pattern_count"].as_u64().unwrap() as usize,
        on_disk_rules.len()
    );
    assert!(report["unreadable"].as_array().unwrap().is_empty());
}

#[test]
fn coverage_json_carries_the_fields_an_integrator_needs() {
    let report = coverage_json();
    let git = report["packs"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["id"] == "git")
        .expect("the git pack should be reported");

    assert!(git["tool_keywords"]
        .as_array()
        .unwrap()
        .contains(&Value::from("git")));
    assert!(!git["safe_patterns"].as_array().unwrap().is_empty());

    let force_push = git["guarded_patterns"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["id"] == "git-force-push")
        .expect("git-force-push should be reported");

    // A rewrite rule must be distinguishable from a deny rule without
    // running a command through the engine.
    assert_eq!(force_push["channel"], "UpdatedInput");
    assert_eq!(force_push["check"], "command_regex");
    assert_eq!(force_push["enabled"], true);
    for field in ["tier", "severity", "explanation", "redirect"] {
        assert!(
            force_push[field].as_str().is_some_and(|s| !s.is_empty()),
            "git-force-push should report a non-empty {field}"
        );
    }
}

#[test]
fn coverage_rejects_an_unknown_format() {
    let output = icg(&["coverage", "--format", "yaml"]);
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("unsupported --format"));
    assert!(
        output.stdout.is_empty(),
        "the rejection happens before anything is written to stdout: {}",
        String::from_utf8_lossy(&output.stdout)
    );
}

// --- the coverage/v1 contract ------------------------------------------
//
// docs/notes/coverage-json-api.md specifies this API. The tests below pin
// it: a shape change must fail here until the format version moves with it.

/// The exact top-level key set. Adding a field for a new feature without
/// bumping the format version would silently pass every lenient consumer
/// and break the strict ones; this makes it a build failure instead.
#[test]
fn coverage_json_pins_its_top_level_key_set() {
    let report = coverage_json();
    let keys: BTreeSet<&str> = report
        .as_object()
        .expect("coverage/v1 should be an object")
        .keys()
        .map(String::as_str)
        .collect();
    let expected: BTreeSet<&str> = [
        "format",
        "packs",
        "unreadable",
        "pack_count",
        "guarded_pattern_count",
    ]
    .into_iter()
    .collect();
    assert_eq!(keys, expected, "coverage/v1 top-level keys moved");
}

/// The same promise one level down: the note's pack and guarded-pattern
/// field tables are exact sets too, and "consumers key on the full key sets
/// above, and those sets are pinned by test" is only true if every level is
/// pinned — a new field on a pack or rule object would otherwise sail past
/// the top-level pin and still break every strict consumer.
#[test]
fn coverage_json_pins_the_key_sets_at_every_level() {
    let report = coverage_json();
    for pack in report["packs"].as_array().unwrap() {
        assert_eq!(
            exact_keys(pack),
            BTreeSet::from([
                "id",
                "path",
                "tool_keywords",
                "applies_to",
                "safe_patterns",
                "guarded_patterns",
            ]),
            "coverage/v1 pack keys moved"
        );
        for rule in pack["guarded_patterns"].as_array().unwrap() {
            assert_eq!(
                exact_keys(rule),
                BTreeSet::from([
                    "id",
                    "enabled",
                    "tier",
                    "severity",
                    "channel",
                    "destructive",
                    "check",
                    "explanation",
                    "redirect",
                ]),
                "coverage/v1 guarded-pattern keys moved"
            );
        }
    }
}

/// Every rule, in every shipped pack, must be fully described: the
/// integrator reading coverage/v1 gets the same metadata `icg explain`
/// prints, and every guarded rule still owes its caller an alternative
/// (repo rule 2).
#[test]
fn every_reported_rule_carries_complete_metadata() {
    let report = coverage_json();
    let channels = ["Deny", "UpdatedInput", "AdditionalContext"];
    let checks = ["command_regex", "content_regex", "predicate"];

    let packs = report["packs"].as_array().unwrap();
    assert!(!packs.is_empty(), "the shipped packs should all report");
    for pack in packs {
        let pack_id = pack["id"].as_str().unwrap();
        assert!(!pack_id.is_empty());
        assert!(!pack["path"].as_str().unwrap().is_empty());
        for field in [
            "tool_keywords",
            "applies_to",
            "safe_patterns",
            "guarded_patterns",
        ] {
            assert!(
                pack[field].is_array(),
                "pack {pack_id} should report {field} as an array"
            );
        }
        for rule in pack["guarded_patterns"].as_array().unwrap() {
            let rule_id = rule["id"].as_str().unwrap();
            assert!(!rule_id.is_empty(), "rule ids are never blank");
            assert!(rule["enabled"].is_boolean(), "{rule_id}: enabled");
            assert!(rule["destructive"].is_boolean(), "{rule_id}: destructive");
            for field in ["tier", "severity", "explanation", "redirect"] {
                assert!(
                    rule[field]
                        .as_str()
                        .is_some_and(|value| !value.trim().is_empty()),
                    "{rule_id} should report a non-empty {field}"
                );
            }
            assert!(
                checks.contains(&rule["check"].as_str().unwrap()),
                "{rule_id}: unknown check kind {:?}",
                rule["check"]
            );
            assert!(
                channels.contains(&rule["channel"].as_str().unwrap()),
                "{rule_id}: unknown channel {:?}",
                rule["channel"]
            );
        }
    }
}

/// `--list` is the documented spelling but is optional under
/// `--format json`; both spellings are part of the contract.
#[test]
fn coverage_json_works_without_the_optional_list_flag() {
    let output = icg(&[
        "coverage",
        "--format",
        "json",
        "--pack",
        packs_dir().to_str().unwrap(),
    ]);
    assert!(output.status.success());
    let report: Value =
        serde_json::from_slice(&output.stdout).expect("json without --list is still coverage/v1");
    assert_eq!(report["format"], "coverage/v1");
}

/// `ICG_PACK_DIR` replaces the default pack search path when no `--pack`
/// is given — part of the documented resolution order. A single-pack
/// directory must yield exactly one pack: if the variable were ignored or
/// merely additive, the repository's own `packs/` (found from the working
/// directory) would report too.
#[test]
fn icg_pack_dir_replaces_the_default_search_path() {
    let dir = TempDir::new().expect("tempdir");
    fs::copy(packs_dir().join("tmux.json"), dir.path().join("only.json"))
        .expect("shipped tmux pack should copy");
    let output = Command::new(env!("CARGO_BIN_EXE_icg"))
        .args(["coverage", "--format", "json"])
        .env("ICG_PACK_DIR", dir.path())
        .output()
        .expect("icg should run");
    assert!(
        output.status.success(),
        "ICG_PACK_DIR at a readable pack should load: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let report: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["format"], "coverage/v1");
    assert_eq!(report["pack_count"].as_u64(), Some(1));
    assert_eq!(report["packs"][0]["id"], "tmux");

    let empty = TempDir::new().expect("tempdir");
    let output = Command::new(env!("CARGO_BIN_EXE_icg"))
        .args(["coverage", "--format", "json"])
        .env("ICG_PACK_DIR", empty.path())
        .output()
        .expect("icg should run");
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("no rule packs found"),
        "an empty ICG_PACK_DIR is refused like an empty --pack dir: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

/// The last link of the documented resolution order: with no `--pack`, no
/// `ICG_PACK_DIR`, and no `/etc/icg` installation, the `packs/` directory
/// relative to the working directory is what loads. On a host that does have
/// `/etc/icg/packs` the report legitimately carries those too, so the
/// assertion is that every shipped pack reports — never that nothing else
/// does.
#[test]
fn the_working_directory_packs_dir_is_a_default_search_path() {
    let output = Command::new(env!("CARGO_BIN_EXE_icg"))
        .args(["coverage", "--format", "json"])
        .env_remove("ICG_PACK_DIR")
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .output()
        .expect("icg should run");
    assert!(
        output.status.success(),
        "the working directory's packs/ should load with no --pack: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let report: Value =
        serde_json::from_slice(&output.stdout).expect("the default path should emit a document");
    assert_eq!(report["format"], "coverage/v1");
    assert!(
        report["unreadable"].as_array().unwrap().is_empty(),
        "every shipped pack is readable; nothing here is an unreadable entry"
    );

    let shipped: BTreeSet<String> = fs::read_dir(packs_dir())
        .expect("packs/ readable")
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|path| path.extension().and_then(|ext| ext.to_str()) == Some("json"))
        .map(|path| {
            serde_json::from_str::<Value>(&fs::read_to_string(&path).expect("pack file readable"))
                .expect("shipped pack should parse")["id"]
                .as_str()
                .expect("pack id")
                .to_owned()
        })
        .collect();
    let reported: BTreeSet<String> = report["packs"]
        .as_array()
        .unwrap()
        .iter()
        .map(|pack| pack["id"].as_str().unwrap().to_owned())
        .collect();
    assert!(
        shipped.is_subset(&reported),
        "every shipped pack must report from the working-directory default: missing {:?}",
        shipped.difference(&reported).collect::<Vec<_>>()
    );
}

/// An unreadable pack is reported in `unreadable` — path plus reason —
/// while every readable pack in the same directory still reports, the
/// command exits 0, and the counts describe only what loaded.
#[test]
fn an_unreadable_pack_is_reported_alongside_the_readable_ones() {
    let dir = TempDir::new().expect("tempdir");
    fs::copy(
        packs_dir().join("tmux.json"),
        dir.path().join("readable.json"),
    )
    .expect("shipped tmux pack should copy");
    let corrupt_path = dir.path().join("corrupt.json");
    fs::write(&corrupt_path, corrupt_pack_contents()).expect("write corrupt pack");

    let output = coverage_json_against(&[dir.path()]);
    assert!(
        output.status.success(),
        "a partially readable directory still succeeds: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let report: Value =
        serde_json::from_slice(&output.stdout).expect("partial readability still emits a document");

    let packs = report["packs"].as_array().unwrap();
    assert_eq!(packs.len(), 1, "only the readable pack reports");
    assert_eq!(packs[0]["id"], "tmux");
    assert_eq!(report["pack_count"].as_u64(), Some(1));
    assert_eq!(
        report["guarded_pattern_count"].as_u64(),
        Some(packs[0]["guarded_patterns"].as_array().unwrap().len() as u64),
        "counts describe only what loaded"
    );

    let unreadable = report["unreadable"].as_array().unwrap();
    assert_eq!(unreadable.len(), 1, "the corrupt pack is named");
    assert!(
        unreadable[0]["path"]
            .as_str()
            .unwrap()
            .ends_with("corrupt.json"),
        "unreadable entries carry the path: {:?}",
        unreadable[0]
    );
    assert!(
        !unreadable[0]["error"].as_str().unwrap().trim().is_empty(),
        "unreadable entries carry the reason"
    );
    assert_eq!(
        exact_keys(&unreadable[0]),
        BTreeSet::from(["path", "error"]),
        "unreadable entries carry exactly the documented pair"
    );
}

/// A directory where nothing loads must not produce an empty coverage/v1
/// document — an empty report reads as "nothing is enforced".
#[test]
fn a_directory_where_every_pack_is_unreadable_fails_without_a_document() {
    let dir = TempDir::new().expect("tempdir");
    fs::write(dir.path().join("corrupt.json"), corrupt_pack_contents())
        .expect("write corrupt pack");

    let output = coverage_json_against(&[dir.path()]);
    assert!(!output.status.success());
    assert!(
        output.stdout.is_empty(),
        "no document is emitted when nothing loaded: {}",
        String::from_utf8_lossy(&output.stdout)
    );
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("no readable rule packs"),
        "stderr names the failure: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

/// Consumers cache and byte-diff this document; two runs over an unchanged
/// pack tree must agree byte for byte, and packs must be path-ordered.
#[test]
fn coverage_json_is_byte_stable_across_runs() {
    let packs = packs_dir();
    let first = coverage_json_against(&[&packs]);
    let second = coverage_json_against(&[&packs]);
    assert!(first.status.success() && second.status.success());
    assert_eq!(
        first.stdout, second.stdout,
        "identical invocations must produce identical bytes"
    );

    let report: Value = serde_json::from_slice(&first.stdout).unwrap();
    let paths: Vec<PathBuf> = report["packs"]
        .as_array()
        .unwrap()
        .iter()
        .map(|pack| PathBuf::from(pack["path"].as_str().unwrap()))
        .collect();
    let mut sorted = paths.clone();
    sorted.sort();
    assert_eq!(paths, sorted, "packs are ordered by resolved path");
}

/// An empty directory is refused, not reported as zero coverage.
#[test]
fn an_empty_pack_directory_is_an_error_not_an_empty_report() {
    let dir = TempDir::new().expect("tempdir");
    let output = coverage_json_against(&[dir.path()]);
    assert!(!output.status.success());
    assert!(
        output.stdout.is_empty(),
        "no document for an empty directory"
    );
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("no rule packs found"),
        "stderr names the failure: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

/// A typo'd --pack path must fail loudly rather than quietly reporting
/// only the packs that did resolve.
#[test]
fn a_missing_pack_path_is_an_error() {
    let dir = TempDir::new().expect("tempdir");
    let missing = dir.path().join("does-not-exist");
    let output = coverage_json_against(&[&missing]);
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("rule-pack path does not exist"),
        "stderr names the failure: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

// --- ordering, dedup, and drift detection -------------------------------
//
// The serialization contract's remaining promises: the order fields and
// rules are emitted in, what a repeated --pack does, and what a consumer
// sees when the policy moves under it.

/// A minimal pack whose every string this file authors, so the raw-byte
/// scan in `fields_are_emitted_in_the_documented_declaration_order` can
/// never trip over a shipped regex or explanation that happens to contain
/// a quoted key name.
fn write_order_fixture_pack(dir: &Path) -> PathBuf {
    let path = dir.join("order-fixture.json");
    fs::write(
        &path,
        serde_json::to_string_pretty(&json!({
            "id": "order-fixture",
            "tool_keywords": ["ordercmd"],
            "applies_to": ["the order fixture pack"],
            "safe_patterns": [{
                "id": "ordercmd-dry-run",
                "type": "command_regex",
                "regex": "^ordercmd --dry-run"
            }],
            "guarded_patterns": [{
                "id": "ordercmd-destroy",
                "enabled": true,
                "type": "command_regex",
                "regex": "^ordercmd destroy",
                "tier": "tier1",
                "severity": "High",
                "explanation": "ordercmd destroy cannot be undone",
                "destructive": true,
                "redirect": {
                    "channel": "deny",
                    "reason_template": "run ordercmd destroy --dry-run first"
                }
            }]
        }))
        .expect("order fixture pack should serialize"),
    )
    .expect("order fixture pack should write");
    path
}

/// Byte offset of `key` in `doc`, searched from `cursor` and moving the
/// cursor past the match. A key that only appears before the cursor fails
/// the test, which is the point: declaration order is "each key after the
/// previous one".
fn find_after(doc: &str, cursor: &mut usize, key: &str) {
    let found = doc[*cursor..]
        .find(key)
        .unwrap_or_else(|| panic!("expected {key} after byte {}", *cursor));
    *cursor += found + key.len();
}

/// The serialization contract pins field order to the declaration order in
/// `src/documented_commands.rs` — "and so on down the levels" included.
/// Parsing into a `Value` loses order (serde_json's map sorts keys), so
/// this walks the raw bytes with a moving cursor over the quoted key names.
/// A reordered struct is a wire-format change and must fail here until the
/// format version moves with it.
#[test]
fn fields_are_emitted_in_the_documented_declaration_order() {
    let dir = TempDir::new().expect("tempdir");
    let pack = write_order_fixture_pack(dir.path());
    let output = coverage_json_against(&[&pack]);
    assert!(
        output.status.success(),
        "the order fixture pack should load: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let doc = String::from_utf8(output.stdout).expect("stdout is utf-8");

    // Top level: format, packs, unreadable, pack_count, guarded_pattern_count.
    let mut cursor = 0;
    for key in [
        "\"format\"",
        "\"packs\"",
        "\"unreadable\"",
        "\"pack_count\"",
        "\"guarded_pattern_count\"",
    ] {
        find_after(&doc, &mut cursor, key);
    }

    // Pack level. Restarting at "packs" is safe: the fixture has one pack,
    // so every pack key appears exactly once before the rule object begins.
    cursor = doc.find("\"packs\"").expect("packs key should be present");
    for key in [
        "\"id\"",
        "\"path\"",
        "\"tool_keywords\"",
        "\"applies_to\"",
        "\"safe_patterns\"",
        "\"guarded_patterns\"",
    ] {
        find_after(&doc, &mut cursor, key);
    }

    // Rule level: the cursor sits just past the guarded_patterns key, and
    // the one rule's keys follow in declaration order.
    for key in [
        "\"id\"",
        "\"enabled\"",
        "\"tier\"",
        "\"severity\"",
        "\"channel\"",
        "\"destructive\"",
        "\"check\"",
        "\"explanation\"",
        "\"redirect\"",
    ] {
        find_after(&doc, &mut cursor, key);
    }
}

/// `tool_keywords` and `applies_to` are promised verbatim, `safe_patterns`
/// as the allow-list members' ids, and the rules "in the pack file's own
/// declaration order". Order is not cosmetic: first-match-wins among
/// guarded patterns makes pack order part of the policy, so a re-ordered
/// report would misdescribe the policy even with the identical id set.
#[test]
fn packs_are_reported_verbatim_from_their_files_in_declaration_order() {
    let report = coverage_json();
    for pack in report["packs"].as_array().unwrap() {
        let what = pack["id"].as_str().unwrap();
        let path = PathBuf::from(pack["path"].as_str().unwrap());
        let file: Value =
            serde_json::from_str(&fs::read_to_string(&path).expect("pack file readable")).unwrap();

        assert_eq!(
            pack["tool_keywords"], file["tool_keywords"],
            "{what}: tool_keywords verbatim"
        );
        assert_eq!(
            pack["applies_to"], file["applies_to"],
            "{what}: applies_to verbatim"
        );

        let ids = |value: &Value, field: &str| -> Vec<String> {
            value[field]
                .as_array()
                .unwrap_or_else(|| panic!("{what}: {field} should be an array"))
                .iter()
                .map(|entry| {
                    entry["id"]
                        .as_str()
                        .map(str::to_owned)
                        .unwrap_or_else(|| entry.as_str().expect("string").to_owned())
                })
                .collect()
        };
        assert_eq!(
            ids(pack, "safe_patterns"),
            ids(&file, "safe_patterns"),
            "{what}: safe_patterns are the file's ids in file order"
        );
        assert_eq!(
            ids(pack, "guarded_patterns"),
            ids(&file, "guarded_patterns"),
            "{what}: guarded_patterns in the pack file's own declaration order"
        );
    }
}

/// "Repeated values naming the same path are deduplicated": a consumer
/// assembling --pack values from layered config must not see doubled packs
/// or doubled counts. Dedup plus the same sort makes the document
/// byte-identical to the single-path invocation.
#[test]
fn repeated_pack_paths_are_deduplicated() {
    let packs = packs_dir();
    let once = coverage_json_against(&[&packs]);
    let twice = coverage_json_against(&[&packs, &packs]);
    assert!(once.status.success() && twice.status.success());
    assert_eq!(
        once.stdout, twice.stdout,
        "a repeated --pack value must not change the document"
    );
}

/// "explicit `--pack` values are deduplicated and sorted": the document is
/// a function of the pack *set*, not the order the caller listed them in. A
/// consumer assembling --pack values from layered config must not be able
/// to reorder the report by reordering its config.
#[test]
fn explicit_pack_values_are_sorted_not_taken_in_caller_order() {
    let git = packs_dir().join("git.json");
    let tmux = packs_dir().join("tmux.json");
    let forward = coverage_json_against(&[&git, &tmux]);
    let reverse = coverage_json_against(&[&tmux, &git]);
    assert!(forward.status.success() && reverse.status.success());
    assert_eq!(
        forward.stdout, reverse.stdout,
        "the caller's --pack order must not change the document"
    );

    let report: Value = serde_json::from_slice(&forward.stdout).unwrap();
    let paths: Vec<&str> = report["packs"]
        .as_array()
        .unwrap()
        .iter()
        .map(|pack| pack["path"].as_str().unwrap())
        .collect();
    assert_eq!(
        paths,
        vec![
            git.to_str().expect("pack path is utf-8"),
            tmux.to_str().expect("pack path is utf-8")
        ],
        "packs report in resolved-path order"
    );
}

/// "A file whose name does not end in `.json` is not treated as a pack,
/// even when it exists." The same valid pack under a `.txt` name is
/// refused when named explicitly — the gate must not silently render an
/// empty report for a path the caller clearly meant — and a `.txt` entry in
/// a pack directory contributes nothing while its `.json` neighbours still
/// report, without surfacing as an `unreadable` entry: the gate skips the
/// file, it does not fail to load it.
#[test]
fn a_non_json_file_is_not_treated_as_a_pack_even_when_it_exists() {
    let dir = TempDir::new().expect("tempdir");
    let wrong_extension = dir.path().join("pack.txt");
    fs::copy(packs_dir().join("tmux.json"), &wrong_extension)
        .expect("shipped tmux pack should copy");

    // Named explicitly: the file exists, is readable, holds a valid pack —
    // and the extension gate still refuses it.
    let output = coverage_json_against(&[&wrong_extension]);
    assert!(
        !output.status.success(),
        "an existing non-.json file must not be treated as a pack"
    );
    assert!(output.stdout.is_empty());
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("no rule packs found"),
        "the refusal is the empty-pack-set error, got {:?}",
        String::from_utf8_lossy(&output.stderr)
    );

    // Offered as a directory: the .txt entry contributes nothing, the .json
    // neighbour still reports, and nothing lands in `unreadable`.
    fs::copy(
        packs_dir().join("git.json"),
        dir.path().join("readable.json"),
    )
    .expect("shipped git pack should copy");
    let output = coverage_json_against(&[dir.path()]);
    assert!(
        output.status.success(),
        "the .json neighbour should load: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let report: Value = serde_json::from_slice(&output.stdout).expect("the document should parse");
    let packs = report["packs"].as_array().unwrap();
    assert_eq!(packs.len(), 1, "the .txt entry contributed no pack");
    assert_eq!(packs[0]["id"], "git");
    assert_eq!(
        report["unreadable"].as_array().unwrap().len(),
        0,
        "the extension gate skips the file; it is not a load failure"
    );
}

/// The closing promise of both contract notes: a consumer detects policy
/// drift from the documents alone, without parsing rule-pack files. The
/// consumer's whole view here is two coverage/v1 documents taken before
/// and after an edit to the pack file; the diff must localize the change.
#[test]
fn a_consumer_diffing_two_documents_sees_the_policy_edit() {
    let dir = TempDir::new().expect("tempdir");
    let pack = write_order_fixture_pack(dir.path());

    let before = coverage_json_against(&[&pack]);
    assert!(before.status.success());
    let before: Value =
        serde_json::from_slice(&before.stdout).expect("the before document should parse");

    // The policy author appends a rule to the pack file.
    let mut file: Value =
        serde_json::from_str(&fs::read_to_string(&pack).expect("fixture readable")).unwrap();
    file["guarded_patterns"]
        .as_array_mut()
        .expect("guarded_patterns array")
        .push(json!({
            "id": "ordercmd-wipe",
            "enabled": true,
            "type": "command_regex",
            "regex": "^ordercmd wipe",
            "tier": "tier1",
            "severity": "High",
            "explanation": "ordercmd wipe cannot be undone",
            "destructive": true,
            "redirect": {
                "channel": "deny",
                "reason_template": "run ordercmd wipe --dry-run first"
            }
        }));
    fs::write(&pack, serde_json::to_string_pretty(&file).unwrap()).expect("fixture rewritten");

    let after = coverage_json_against(&[&pack]);
    assert!(after.status.success());
    let after: Value =
        serde_json::from_slice(&after.stdout).expect("the after document should parse");

    assert_eq!(
        after["pack_count"], before["pack_count"],
        "the edit adds a rule, not a pack"
    );
    assert_eq!(
        after["guarded_pattern_count"].as_u64().unwrap(),
        before["guarded_pattern_count"].as_u64().unwrap() + 1,
        "the count moves by exactly the one added rule"
    );
    assert!(after["unreadable"].as_array().unwrap().is_empty());

    let ids = |report: &Value| -> Vec<String> {
        report["packs"][0]["guarded_patterns"]
            .as_array()
            .unwrap()
            .iter()
            .map(|rule| rule["id"].as_str().unwrap().to_owned())
            .collect()
    };
    let (before_ids, after_ids) = (ids(&before), ids(&after));
    let added: Vec<&String> = after_ids
        .iter()
        .filter(|id| !before_ids.contains(id))
        .collect();
    assert_eq!(
        added,
        vec!["ordercmd-wipe"],
        "the diff names exactly the added rule"
    );
    assert_eq!(
        after_ids.last().map(String::as_str),
        Some("ordercmd-wipe"),
        "the appended rule reports where the file declared it: last"
    );
}
