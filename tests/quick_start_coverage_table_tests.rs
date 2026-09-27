//! quick-start.md's coverage table held to the enforced policy (coverage/v1).
//!
//! `documentation_consistency_tests.rs` walks `packs/*.json` and holds the
//! table to the shipped pattern ids and the per-pack counts, but those pins
//! never look at what the table documents *about* each rule: the
//! `(Critical)`/`(High)`/`(Medium)` severity annotations, and the redirect
//! behavior some rows promise ("rewritten to a plain push"). A pack edit
//! that re-severed a severity or flipped a rule's channel could ship while
//! every id-and-count pin stayed green, leaving the operator-facing table
//! describing a policy the engine no longer enforces (irrevers-e7bfa80c).
//!
//! These tests parse the table and diff it against `icg coverage --list
//! --format json` -- the machine surface the doc itself tells scripters to
//! read -- so every coverage/v1 rule must have a row, every documented
//! severity must equal the enforced one, and every non-deny rule must be
//! one the doc's channel prose accounts for. This complements, and does
//! not replace, `icg coverage-diff`: the diff gates a pack *change* against
//! a baseline at pack-edit time, these gate operator-facing doc *drift*.

use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;
use std::process::Command;

/// The severity words a row may annotate a rule with -- the exact spellings
/// coverage/v1 reports, which are the only three the `Severity` enum has.
const SEVERITIES: [&str; 3] = ["Critical", "High", "Medium"];

/// The checkout this run audits.
///
/// Same rationale as documentation_consistency_tests' helper: the cargo
/// wrapper pins one shared target directory per repo, so a reused test
/// binary's baked `CARGO_MANIFEST_DIR` can name some other tree. cargo runs
/// test binaries with the package root as the working directory; prefer the
/// runtime cwd and fall back to the baked path only when it does not name
/// a checkout.
fn audited_checkout() -> PathBuf {
    if let Ok(cwd) = std::env::current_dir() {
        if cwd.join("Cargo.toml").exists() {
            return cwd;
        }
    }
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// `icg coverage --list --format json` against the shipped packs, with the
/// stamp and the unreadable list asserted so a failing pack is a loud
/// failure here rather than a silently smaller rule set to diff against.
fn coverage_v1() -> serde_json::Value {
    let packs = audited_checkout().join("packs");
    let output = Command::new(env!("CARGO_BIN_EXE_icg"))
        .args([
            "coverage",
            "--list",
            "--format",
            "json",
            "--pack",
            packs.to_str().expect("packs path should be UTF-8"),
        ])
        .output()
        .expect("icg coverage --list should run");
    assert!(
        output.status.success(),
        "coverage --list --format json should succeed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let report: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("coverage should emit valid JSON");
    assert_eq!(report["format"], "coverage/v1");
    let unreadable = report["unreadable"]
        .as_array()
        .expect("coverage/v1 should carry an unreadable array");
    assert!(
        unreadable.is_empty(),
        "every shipped pack should load; coverage reported unreadable: \
         {unreadable:?}"
    );
    report
}

/// The coverage table itself: pack id -> the row's "What it blocks" cell.
///
/// The section is extracted first so a second pipe-table elsewhere in the
/// doc cannot satisfy (or pollute) the parse. The block cell is the *last*
/// column: markdown rows end in `|`, so splitting on `|` and taking the
/// cell before the trailing empty piece keeps working even if a column is
/// inserted ahead of it.
fn coverage_table_cells() -> BTreeMap<String, String> {
    let doc = fs::read_to_string(audited_checkout().join("docs/quick-start.md"))
        .expect("docs/quick-start.md should be readable");
    let start = doc
        .find("## What Gets Protected")
        .expect("quick-start.md should keep its 'What Gets Protected' section");
    let section = doc[start..]
        .split("\n## ")
        .next()
        .expect("the section heading is non-empty");

    let mut cells = BTreeMap::new();
    for line in section.lines().filter(|line| line.starts_with("| `")) {
        let fields: Vec<&str> = line.split('|').collect();
        // "| pack | count | scope | blocks |" splits into an empty leading
        // piece, four cells, and an empty trailing piece.
        assert!(
            fields.len() >= 6,
            "coverage-table row should keep the four documented columns \
             (Pack, Patterns, Scope, What it blocks): {line}"
        );
        let pack = fields[1].trim().trim_matches('`').to_owned();
        let block_cell = fields[fields.len() - 2].trim().to_owned();
        let existing = cells.insert(pack.clone(), block_cell);
        assert!(
            existing.is_none(),
            "quick-start.md's coverage table should carry pack `{pack}` \
             exactly once"
        );
    }
    assert!(
        !cells.is_empty(),
        "quick-start.md's coverage table should have at least one pack row"
    );
    cells
}

/// The row-wide ``(all Severity)`` annotation, if the cell carries one --
/// the "all of the above" form rows use when every listed rule shares a
/// severity. At most one per row: two groups make "which rule carries
/// which severity" ambiguous, so a second one is a documentation bug.
fn group_severity(cell: &str) -> Option<String> {
    let start = cell.find("(all ")?;
    let inner = cell[start + 5..].split(')').next().unwrap_or("").trim();
    assert!(
        SEVERITIES.contains(&inner),
        "the row's (all {inner}) annotation does not name a severity \
         coverage/v1 can report ({SEVERITIES:?})"
    );
    assert!(
        !cell[start + 5..].contains("(all "),
        "a coverage-table row may carry at most one (all Severity) group; \
         annotate the rules individually instead"
    );
    Some(inner.to_owned())
}

/// The severity an annotation right after a backticked id declares, if any.
fn annotation_severity(after_token: &str) -> Option<String> {
    let after = after_token.trim_start();
    if !after.starts_with('(') {
        return None;
    }
    let inner = after[1..].split(')').next().unwrap_or("").trim();
    if SEVERITIES.contains(&inner) {
        Some(inner.to_owned())
    } else {
        // "(all Critical)" is the row-wide form handled by `group_severity`;
        // any other parenthetical is prose, not an annotation.
        None
    }
}

/// The severity the table documents for `rule_id` within one row cell.
///
/// Two spellings ship: a per-rule ``(Critical)`` immediately after the
/// backticked id, and one row-wide ``(all Critical)`` covering the ids
/// that lack their own. A rule mentioned more than once makes the parse
/// ambiguous, so that is rejected outright.
fn documented_severity(cell: &str, rule_id: &str) -> Option<String> {
    let needle = format!("`{rule_id}`");
    let occurrences = cell.matches(&needle).count();
    assert!(
        occurrences <= 1,
        "quick-start.md's coverage table mentions `{rule_id}` \
         {occurrences} times in one row; mention it once so its severity \
         annotation is unambiguous"
    );
    let start = cell.find(&needle)?;
    let after = &cell[start + needle.len()..];
    match annotation_severity(after) {
        Some(severity) => Some(severity),
        None => group_severity(cell),
    }
}

/// True when `token` is shaped like a rule id (lowercase, digits, hyphens).
///
/// Used only to tell rule ids from prose inside a row that carries a
/// row-wide ``(all Severity)``: prose tokens in those rows either hold
/// spaces (`kv destroy`) or other punctuation (`.beads`), while every
/// shipped id is hyphenated lowercase.
fn shaped_like_a_rule_id(token: &str) -> bool {
    !token.is_empty()
        && token
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
        && token.contains('-')
        && !token.starts_with('-')
        && !token.ends_with('-')
        && !token.contains("--")
}

/// Every coverage/v1 rule must appear in its pack's coverage-table row --
/// the direction the id pins in documentation_consistency_tests only check
/// doc-wide (`doc.contains(pattern_id)`), which a mention in unrelated
/// prose would satisfy while the table itself stayed incomplete.
#[test]
fn coverage_v1_rules_all_have_a_coverage_table_row() {
    let report = coverage_v1();
    let cells = coverage_table_cells();
    for pack in report["packs"].as_array().expect("packs array") {
        let pack_id = pack["id"].as_str().expect("pack id");
        let rules = pack["guarded_patterns"].as_array().expect("rules array");
        if rules.is_empty() {
            continue;
        }
        let cell = cells.get(pack_id).unwrap_or_else(|| {
            panic!(
                "coverage/v1 reports pack `{pack_id}` with guarded rules, \
                 but quick-start.md's coverage table has no row for it"
            )
        });
        for rule in rules {
            let rule_id = rule["id"].as_str().expect("rule id");
            assert!(
                cell.contains(&format!("`{rule_id}`")),
                "quick-start.md's coverage-table row for pack `{pack_id}` \
                 never mentions `{rule_id}` -- the table promises its ids \
                 are the ones `icg explain` accepts, so a shipped rule no \
                 operator can read about is how silent coverage holes get \
                 reported as false positives"
            );
        }
    }
}

/// Every severity the table documents must equal the severity coverage/v1
/// enforces, in both directions: an annotation that drifted after a pack
/// edit, and a rule documented with no annotation at all, each fail.
#[test]
fn coverage_table_rule_severities_match_coverage_v1() {
    let report = coverage_v1();
    let cells = coverage_table_cells();
    for pack in report["packs"].as_array().expect("packs array") {
        let pack_id = pack["id"].as_str().expect("pack id");
        let rules = pack["guarded_patterns"].as_array().expect("rules array");
        if rules.is_empty() {
            continue;
        }
        let cell = cells.get(pack_id).unwrap_or_else(|| {
            panic!(
                "coverage/v1 reports pack `{pack_id}` with guarded rules, \
                 but quick-start.md's coverage table has no row for it"
            )
        });
        for rule in rules {
            let rule_id = rule["id"].as_str().expect("rule id");
            let enforced = rule["severity"].as_str().expect("severity");
            let documented = documented_severity(cell, rule_id).unwrap_or_else(|| {
                panic!(
                    "quick-start.md's coverage table documents `{rule_id}` \
                     (pack `{pack_id}`) without a severity annotation -- \
                     add `(Severity)` after the id, or one row-wide \
                     `(all Severity)` if the whole row shares it"
                )
            });
            assert_eq!(
                documented, enforced,
                "quick-start.md's coverage table documents `{rule_id}` as \
                 {documented} but coverage/v1 enforces {enforced} -- the \
                 operator-facing table and the enforced policy drifted"
            );
        }
    }
}

/// The table promises "Pattern IDs below are the IDs `icg explain`
/// accepts" -- so an id presented as a rule must resolve in coverage/v1
/// under its own pack. A rule renamed or removed from a pack otherwise
/// leaves a stale id behind that no other pin catches: the doc-wide id
/// check asserts shipped ids appear in the doc, not that doc ids ship.
///
/// A token is *presented* as a rule id when it carries its own severity
/// annotation, or when it is shaped like an id inside a row carrying a
/// row-wide ``(all Severity)`` (which claims the severity for every id in
/// the row).
#[test]
fn coverage_table_ids_are_the_ids_icg_explain_accepts() {
    let report = coverage_v1();
    let cells = coverage_table_cells();
    for pack in report["packs"].as_array().expect("packs array") {
        let pack_id = pack["id"].as_str().expect("pack id");
        let shipped: Vec<&str> = pack["guarded_patterns"]
            .as_array()
            .expect("rules array")
            .iter()
            .map(|rule| rule["id"].as_str().expect("rule id"))
            .collect();
        let Some(cell) = cells.get(pack_id) else {
            continue; // a missing row is the other tests' failure to report
        };
        let group = group_severity(cell);
        let segments: Vec<&str> = cell.split('`').collect();
        for index in (1..segments.len()).step_by(2) {
            let token = segments[index];
            let after = segments.get(index + 1).copied().unwrap_or("");
            let presented = annotation_severity(after).is_some()
                || (group.is_some() && shaped_like_a_rule_id(token));
            assert!(
                !presented || shipped.contains(&token),
                "quick-start.md's coverage-table row for pack `{pack_id}` \
                 presents `{token}` as a rule id, but coverage/v1 reports \
                 no such rule for that pack -- the table promises its ids \
                 are the ones `icg explain` accepts"
            );
        }
    }
}

/// The doc's channel prose must account for every rule that does not deny.
///
/// quick-start's verdict list promises one outcome per redirect channel and
/// names the shipped exceptions: the git row's force-push is "rewritten to
/// a plain push" (updated_input) and `openbao-kv-get-to-stdout` is the
/// named additional_context example. A rule that denies is fully described
/// by its severity row; a rule that warns or rewrites changes what the
/// operator sees happen, so it must move this list (and this arm) when it
/// lands -- the same convention as `count_word()` in
/// documentation_consistency_tests: a channel with no arm fails loudly
/// instead of letting the doc drift silently past it.
#[test]
fn coverage_table_accounts_for_every_non_deny_redirect_channel() {
    const DOCUMENTED_NON_DENY: [(&str, &str); 2] = [
        ("git-force-push", "UpdatedInput"),
        ("openbao-kv-get-to-stdout", "AdditionalContext"),
    ];

    let report = coverage_v1();
    let mut enforced: BTreeMap<&str, &str> = BTreeMap::new();
    for pack in report["packs"].as_array().expect("packs array") {
        for rule in pack["guarded_patterns"].as_array().expect("rules array") {
            let rule_id = rule["id"].as_str().expect("rule id");
            let channel = rule["channel"].as_str().expect("channel");
            if channel != "Deny" {
                enforced.insert(rule_id, channel);
            }
        }
    }

    for (rule_id, documented) in DOCUMENTED_NON_DENY {
        let actual = enforced.get(rule_id).copied();
        assert_eq!(
            actual,
            Some(documented),
            "quick-start.md documents `{rule_id}` as a {documented} rule, \
             but coverage/v1 reports {:?} -- the verdict list and the \
             coverage table promise a channel the engine no longer uses",
            actual.unwrap_or("Deny")
        );
    }
    for (rule_id, channel) in &enforced {
        let documented = DOCUMENTED_NON_DENY
            .iter()
            .find(|(candidate, _)| candidate == rule_id)
            .map(|(_, documented)| *documented);
        assert_eq!(
            documented,
            Some(*channel),
            "coverage/v1 reports `{rule_id}` as a {channel} rule, but \
             quick-start.md's channel prose does not account for it -- add \
             it to DOCUMENTED_NON_DENY here and document its verdict in \
             the doc's verdict list and coverage table"
        );
    }
}
