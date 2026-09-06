//! Every shipped pack must generate a deny suite, and every rule must be
//! either a case or an explicitly-reasoned exclusion.
//!
//! `icg regression-suite packs/<id>.json` -- the form the README documents --
//! used to abort on six of the ten shipped packs. Three were the command
//! synthesizer mangling a regex it could not reverse (`\b` became a literal
//! `b`); the rest were rules a *deny* suite cannot represent at all: predicate
//! checks needing live state, non-deny channels, and the `secrets` pack, which
//! matches unconditionally and so has no keyword to build a command from.
//! Aborting on the second group hid the first.

use icg::regression::generate_regression_suite_from_manifest;
use icg::rule_pack::{load_pack, Channel, Check};
use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

fn pack_paths() -> Vec<PathBuf> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("packs");
    let mut paths: Vec<PathBuf> = fs::read_dir(&dir)
        .expect("packs/ should be readable")
        .map(|entry| entry.expect("directory entry").path())
        .filter(|path| path.extension().and_then(|e| e.to_str()) == Some("json"))
        .collect();
    paths.sort();
    assert!(!paths.is_empty(), "packs/ should not be empty");
    paths
}

#[test]
fn every_shipped_pack_generates_a_suite() {
    for path in pack_paths() {
        generate_regression_suite_from_manifest(&path).unwrap_or_else(|error| {
            panic!(
                "{} should generate a regression suite: {error:#}",
                path.display()
            )
        });
    }
}

#[test]
fn every_enabled_rule_is_either_a_case_or_a_reasoned_skip() {
    for path in pack_paths() {
        let pack = load_pack(&path).expect("pack loads");
        let suite = generate_regression_suite_from_manifest(&path).expect("suite generates");

        let enabled: BTreeSet<&str> = pack
            .guarded_patterns
            .iter()
            .filter(|p| p.enabled)
            .map(|p| p.id.as_str())
            .collect();
        let covered: BTreeSet<&str> = suite
            .cases
            .iter()
            .map(|c| c.pattern_id.as_str())
            .chain(suite.skipped.iter().map(|s| s.pattern_id.as_str()))
            .collect();

        assert_eq!(
            enabled,
            covered,
            "{} leaves an enabled rule unaccounted for",
            path.display()
        );

        for skipped in &suite.skipped {
            assert!(
                !skipped.reason.trim().is_empty(),
                "{}/{} was skipped without a reason",
                skipped.pack_id,
                skipped.pattern_id
            );
        }
    }
}

/// A skip is only legitimate for a structural impossibility. A deny rule with
/// a regex check is always representable -- if one is ever skipped, the
/// generator has started hiding the coverage hole it exists to surface.
#[test]
fn no_deny_regex_rule_is_ever_skipped() {
    for path in pack_paths() {
        let pack = load_pack(&path).expect("pack loads");
        let suite = generate_regression_suite_from_manifest(&path).expect("suite generates");

        for skipped in &suite.skipped {
            let pattern = pack
                .guarded_patterns
                .iter()
                .find(|p| p.id == skipped.pattern_id)
                .expect("skipped pattern belongs to the pack");

            let is_deny = pattern.redirect.channel == Channel::Deny;
            let is_regex = matches!(
                pattern.check,
                Check::CommandRegex { .. } | Check::ContentRegex { .. }
            );
            let unconditional_pack = pack.tool_keywords.is_empty()
                && !matches!(pattern.check, Check::ContentRegex { .. });

            assert!(
                !(is_deny && is_regex && !unconditional_pack),
                "{}/{} is a deny rule with a regex check and should carry a fixed \
                 case, not a skip -- add an `example_command` to the pack. Reason \
                 given: {}",
                skipped.pack_id,
                skipped.pattern_id,
                skipped.reason
            );
        }
    }
}

/// The three rules whose regexes defeat the command synthesizer now carry a
/// hand-authored `example_command`, and it must keep working.
#[test]
fn hand_authored_examples_still_produce_their_cases() {
    for (pack_file, pattern_id) in [
        ("git.json", "git-credential-fill-bare-stdout"),
        ("openbao.json", "openbao-inline-secret-literal"),
        ("tmux.json", "bare-nato-session"),
    ] {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("packs")
            .join(pack_file);
        let suite = generate_regression_suite_from_manifest(&path).expect("suite generates");
        assert!(
            suite.cases.iter().any(|c| c.pattern_id == pattern_id),
            "{pack_file} should carry a fixed deny case for {pattern_id}"
        );
    }
}

/// The release gate's corpus is what actually blocks a bad release. Making
/// per-pack generation permissive must not have widened or narrowed it.
#[test]
fn the_release_gate_corpus_is_unchanged() {
    let packs = Path::new(env!("CARGO_MANIFEST_DIR")).join("packs");
    let suite = icg::regression::generate_release_regression_suite(&packs)
        .expect("release gate suite generates");

    let ids: BTreeSet<&str> = suite.cases.iter().map(|c| c.pattern_id.as_str()).collect();
    let expected: BTreeSet<&str> = [
        "docker-image-rm-force",
        "docker-system-prune-all",
        "docker-volume-rm",
        "duplicate-ardenone-cluster-root",
        "git-commit-without-pathspec",
        "image-tag-bare-sha",
        "image-tag-latest",
        "needle-cleanup",
        "openbao-destructive-verb",
        "storage-class-ssd",
    ]
    .into_iter()
    .collect();

    assert_eq!(
        ids, expected,
        "the release gate corpus changed; that is a coverage change requiring review"
    );
}
