//! `icg catalog --json` — the versioned always/never event catalog.
//!
//! The policy was already machine-readable per pack (`coverage --format
//! json`), but a consumer that wants the *events* — what must never happen
//! and what is always allowed, each with its severity and sanctioned
//! alternative — had to parse packs it does not own. This pins the exported
//! catalog: its schema key sets, its determinism, its counts against the
//! coverage report, and above all its agreement with the engine, which is
//! what makes it "generated from the same source the engine dispatches on"
//! rather than a second list that can drift.
//! [`docs/notes/event-catalog-json-api.md`](docs/notes/event-catalog-json-api.md)
//! is the contract note these tests enforce.

use icg::catalog::{self, MatchExpression, CATALOG_FORMAT};
use icg::engine::{CheckResult, CommandSource, ContentSource, Engine};
use icg::github_workflows::{
    GUARDED_PATHS, PACK_ID as WORKFLOWS_PACK, PATTERN_ID as WORKFLOWS_PATTERN, PROTECTED_REASON,
};
use icg::job_cronjob_yaml::{
    BLOCKED_REASON, GUARDED_CONTENTS, PACK_ID as JOBCRON_PACK, PATTERN_ID as JOBCRON_PATTERN,
};
use icg::rule_pack::{load_pack, Channel, Check, Pack, Redirect, Severity, Tier};
use serde_json::{json, Value};
use std::collections::BTreeSet;
use std::fs;
use std::process::{Command, Output};

fn icg(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_icg"))
        .args(args)
        .output()
        .expect("icg should run")
}

/// Every shipped pack, loaded the way `icg catalog` loads them: the same
/// `rule_pack::load_pack` result the engine consumes.
fn shipped_packs() -> Vec<Pack> {
    let mut paths: Vec<std::path::PathBuf> = fs::read_dir("packs")
        .expect("packs/ readable from the package root")
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|path| path.extension().and_then(|ext| ext.to_str()) == Some("json"))
        .collect();
    paths.sort();
    paths
        .iter()
        .map(|path| load_pack(path).expect("shipped pack should load"))
        .collect()
}

fn catalog_from_cli() -> Value {
    let output = icg(&["catalog", "--json", "--pack", "packs"]);
    assert!(
        output.status.success(),
        "icg catalog --json should succeed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).expect("catalog should emit valid JSON")
}

/// The `(pack, id)` identity a denial record carries, from any verdict.
fn attribution(result: &CheckResult) -> (String, String) {
    match result {
        CheckResult::Denied {
            pack_id,
            pattern_id,
            ..
        }
        | CheckResult::Rewrite {
            pack_id,
            pattern_id,
            ..
        }
        | CheckResult::Warning {
            pack_id,
            pattern_id,
            ..
        } => (pack_id.clone(), pattern_id.clone()),
        CheckResult::Allowed => panic!("expected a verdict with attribution, got Allowed"),
    }
}

/// An engine loaded with the packs the sampled verdicts below dispatch on.
fn sampled_engine() -> Engine {
    let mut engine = Engine::new();
    for id in ["git", "openbao", "storage-class"] {
        let path = format!("packs/{id}.json");
        engine
            .load_pack(load_pack(&path).unwrap_or_else(|e| panic!("{path} should load: {e}")))
            .unwrap_or_else(|e| panic!("{path} should validate: {e}"));
    }
    engine
}

#[test]
fn catalog_exports_every_shipped_event() {
    let catalog = catalog_from_cli();
    assert_eq!(catalog["format"], CATALOG_FORMAT);

    // The ids on disk, plus the two built-in guards the engine carries in
    // code rather than in a pack file.
    let mut never_ids: BTreeSet<(String, String)> = BTreeSet::from([
        (WORKFLOWS_PACK.to_string(), WORKFLOWS_PATTERN.to_string()),
        (JOBCRON_PACK.to_string(), JOBCRON_PATTERN.to_string()),
    ]);
    let mut always_ids: BTreeSet<(String, String)> = BTreeSet::new();
    for pack in shipped_packs() {
        for rule in &pack.guarded_patterns {
            never_ids.insert((pack.id.clone(), rule.id.clone()));
        }
        for pattern in &pack.safe_patterns {
            always_ids.insert((pack.id.clone(), pattern.id.clone()));
        }
    }

    let exported_never: BTreeSet<(String, String)> = catalog["never"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| {
            (
                e["pack"].as_str().unwrap().to_owned(),
                e["id"].as_str().unwrap().to_owned(),
            )
        })
        .collect();
    let exported_always: BTreeSet<(String, String)> = catalog["always"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| {
            (
                e["pack"].as_str().unwrap().to_owned(),
                e["id"].as_str().unwrap().to_owned(),
            )
        })
        .collect();

    assert_eq!(
        exported_never, never_ids,
        "never events must match disk exactly"
    );
    assert_eq!(
        exported_always, always_ids,
        "always events must match disk exactly"
    );
    assert_eq!(catalog["never"].as_array().unwrap().len(), never_ids.len());
    assert_eq!(
        catalog["always"].as_array().unwrap().len(),
        always_ids.len()
    );
}

#[test]
fn catalog_digest_is_deterministic_and_not_installation_specific() {
    let first = icg(&["catalog", "--json", "--pack", "packs"]);
    let second = icg(&["catalog", "--json", "--pack", "packs"]);
    assert_eq!(
        first.stdout, second.stdout,
        "unchanged packs must render byte-identical catalogs"
    );

    let catalog = catalog_from_cli();
    let digest = catalog["catalog_digest"].as_str().unwrap();
    assert_eq!(digest.len(), 64, "digest is a SHA-256 hex string");
    assert!(
        digest
            .chars()
            .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()),
        "digest is lowercase hex: {digest}"
    );

    // The CLI document and the library render agree, and the digest covers
    // the event set rather than the binary: the same packs built in-process
    // (this checkout's tree) must produce the same digest the binary printed.
    let rebuilt = catalog::build(&shipped_packs()).expect("shipped packs should build a catalog");
    assert_eq!(rebuilt.catalog_digest, digest);
    assert_eq!(rebuilt.format, CATALOG_FORMAT);
}

#[test]
fn builtin_catalog_entries_agree_with_engine_denials() {
    // With no packs at all the catalog still names the built-ins, because
    // the engine enforces them in code before any pack is consulted.
    let builtin_only = catalog::build(&[]).expect("empty pack set should build");
    assert_eq!(
        builtin_only.never.len(),
        2,
        "exactly the two built-in guards"
    );
    assert!(
        builtin_only.always.is_empty(),
        "built-ins have no safe patterns"
    );
    let workflows_entry = builtin_only
        .never
        .iter()
        .find(|e| e.pack == WORKFLOWS_PACK && e.id == WORKFLOWS_PATTERN)
        .expect("workflows guard must be cataloged");
    let jobcron_entry = builtin_only
        .never
        .iter()
        .find(|e| e.pack == JOBCRON_PACK && e.id == JOBCRON_PATTERN)
        .expect("Job/CronJob guard must be cataloged");

    let engine = Engine::new();
    for path in GUARDED_PATHS {
        let result = engine.evaluate_content(&ContentSource::Write {
            file_path: (*path).to_string(),
            content: "name: ci\non: push\njobs: {}\n".to_string(),
        });
        assert_eq!(
            attribution(&result),
            (WORKFLOWS_PACK.to_string(), WORKFLOWS_PATTERN.to_string()),
            "engine denial for {path} must attribute to the catalog's workflows entry"
        );
        match &result {
            CheckResult::Denied { reason, .. } => assert_eq!(reason, PROTECTED_REASON),
            other => panic!("expected Denied for {path}, got {other:?}"),
        }
    }
    assert_eq!(
        workflows_entry.sanctioned_alternative.reason,
        PROTECTED_REASON
    );
    assert_eq!(workflows_entry.action, Channel::Deny);
    assert!(workflows_entry.enabled);

    for content in GUARDED_CONTENTS {
        let result = engine.evaluate_content(&ContentSource::Write {
            file_path: "k8s/job.yaml".to_string(),
            content: (*content).to_string(),
        });
        assert_eq!(
            attribution(&result),
            (JOBCRON_PACK.to_string(), JOBCRON_PATTERN.to_string()),
            "engine denial for guarded Job/CronJob content must attribute to the catalog's entry"
        );
        match &result {
            CheckResult::Denied { reason, .. } => assert_eq!(reason, BLOCKED_REASON),
            other => panic!("expected Denied for guarded content, got {other:?}"),
        }
    }
    assert_eq!(jobcron_entry.sanctioned_alternative.reason, BLOCKED_REASON);
    assert_eq!(jobcron_entry.action, Channel::Deny);
    assert!(jobcron_entry.enabled);
}

/// Every response channel the engine can emit, sampled through the real
/// dispatch path, must land on the catalog entry the catalog claims owns it.
/// This is the seam that makes the export trustworthy: an event the engine
/// stops emitting, or emits under a different identity, fails here.
#[test]
fn sampled_verdicts_attribute_to_their_catalog_entries() {
    let built = catalog::build(&shipped_packs()).expect("shipped packs should build");
    let entry = |pack: &str, id: &str| {
        built
            .never
            .iter()
            .find(|e| e.pack == pack && e.id == id)
            .unwrap_or_else(|| panic!("catalog must contain {pack}/{id}"))
    };

    let engine = sampled_engine();

    // Rewrite channel: force-push is rewritten, not denied.
    let result = engine.evaluate_command(&CommandSource::Hook(
        "git push --force origin main".to_string(),
    ));
    assert_eq!(
        attribution(&result),
        ("git".to_string(), "git-force-push".to_string())
    );
    let force_push = entry("git", "git-force-push");
    assert_eq!(force_push.action, Channel::UpdatedInput);
    assert!(force_push.sanctioned_alternative.rewrite.is_some());

    // Warning channel: an OpenBao read to stdout is allowed with context.
    let result =
        engine.evaluate_command(&CommandSource::Hook("bao kv get secret/app/db".to_string()));
    assert_eq!(
        attribution(&result),
        (
            "openbao".to_string(),
            "openbao-kv-get-to-stdout".to_string()
        )
    );
    let kv_get = entry("openbao", "openbao-kv-get-to-stdout");
    assert_eq!(kv_get.action, Channel::AdditionalContext);

    // Deny channel, content mode: an SSD storage class in a written manifest.
    let result = engine.evaluate_content(&ContentSource::Write {
        file_path: "claim.yaml".to_string(),
        content: "storageClassName: ssd-large\n".to_string(),
    });
    assert_eq!(
        attribution(&result),
        ("storage-class".to_string(), "storage-class-ssd".to_string())
    );
    assert_eq!(
        entry("storage-class", "storage-class-ssd").action,
        Channel::Deny
    );
}

#[test]
fn catalog_counts_agree_with_the_coverage_report() {
    let coverage = icg(&["coverage", "--list", "--format", "json", "--pack", "packs"]);
    assert!(coverage.status.success(), "coverage json should succeed");
    let coverage: Value =
        serde_json::from_slice(&coverage.stdout).expect("coverage should emit valid JSON");
    let catalog = catalog_from_cli();

    let guarded: usize = coverage["packs"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| p["guarded_patterns"].as_array().unwrap().len())
        .sum();
    let safe: usize = coverage["packs"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| p["safe_patterns"].as_array().unwrap().len())
        .sum();

    // The catalog adds exactly the two built-in guards to the pack rules.
    assert_eq!(
        catalog["never"].as_array().unwrap().len(),
        guarded + 2,
        "never = every guarded pack rule + the two built-in guards"
    );
    assert_eq!(
        catalog["always"].as_array().unwrap().len(),
        safe,
        "always = every safe pattern"
    );
}

#[test]
fn catalog_schema_pins_its_key_sets() {
    let catalog = catalog_from_cli();

    let top_level: BTreeSet<&str> = catalog
        .as_object()
        .unwrap()
        .keys()
        .map(|k| k.as_str())
        .collect();
    assert_eq!(
        top_level,
        BTreeSet::from(["format", "catalog_digest", "icg_version", "never", "always"]),
        "top-level key set is the wire contract; bump the format version to change it"
    );

    fn event_keys(event: &Value) -> BTreeSet<&str> {
        event
            .as_object()
            .unwrap()
            .keys()
            .map(|k| k.as_str())
            .collect()
    }
    for event in catalog["never"].as_array().unwrap() {
        assert_eq!(
            event_keys(event),
            BTreeSet::from([
                "id",
                "pack",
                "severity",
                "tier",
                "action",
                "destructive",
                "enabled",
                "check",
                "match",
                "explanation",
                "sanctioned_alternative",
            ])
        );
        assert!(!event["id"].as_str().unwrap().is_empty());
        assert!(!event["pack"].as_str().unwrap().is_empty());
        assert!(
            !event["explanation"].as_str().unwrap().is_empty(),
            "{} must say why it must never happen",
            event["id"]
        );
        assert!(
            ["Critical", "High", "Medium"].contains(&event["severity"].as_str().unwrap()),
            "severity is one of the documented spellings: {}",
            event["severity"]
        );
        assert!(
            ["tier1", "tier2", "tier3"].contains(&event["tier"].as_str().unwrap()),
            "tier is one of the documented spellings: {}",
            event["tier"]
        );
        assert!(
            ["deny", "updated_input", "additional_context"]
                .contains(&event["action"].as_str().unwrap()),
            "action uses the hook wire spelling: {}",
            event["action"]
        );

        // The match object is a tagged union: exactly one key, agreeing with
        // `check`.
        let keys: BTreeSet<&str> = event["match"]
            .as_object()
            .unwrap()
            .keys()
            .map(|k| k.as_str())
            .collect();
        match event["check"].as_str().unwrap() {
            "command_regex" | "content_regex" => {
                assert_eq!(keys, BTreeSet::from(["regex"]));
                assert!(!event["match"]["regex"].as_str().unwrap().is_empty());
            }
            "predicate" => {
                assert_eq!(keys, BTreeSet::from(["predicate"]));
                assert!(!event["match"]["predicate"].as_str().unwrap().is_empty());
            }
            other => panic!("unknown check kind {other:?}"),
        }

        // The sanctioned alternative always carries a reason (repo rule 2),
        // and carries a rewrite exactly when the channel is updated_input —
        // a rewrite on a deny (or a rewrite channel with nothing to rewrite
        // to) would be a lying catalog entry.
        let alternative = &event["sanctioned_alternative"];
        let alternative_keys: BTreeSet<&str> = alternative
            .as_object()
            .unwrap()
            .keys()
            .map(|k| k.as_str())
            .collect();
        assert!(alternative_keys.contains("reason"));
        assert!(
            alternative_keys.len() <= 2,
            "sanctioned_alternative carries at most reason + rewrite: {alternative_keys:?}"
        );
        assert!(!alternative["reason"].as_str().unwrap().is_empty());
        assert_eq!(
            alternative_keys.contains("rewrite"),
            event["action"] == "updated_input",
            "{}: rewrite presence must match the updated_input channel",
            event["id"]
        );
    }

    for event in catalog["always"].as_array().unwrap() {
        assert_eq!(
            event_keys(event),
            BTreeSet::from(["id", "pack", "check", "match"]),
            "always events carry recognition, not enforcement fields"
        );
    }

    // Both event arrays are sorted by (pack, id) so a plain diff between
    // two catalogs reads as a policy change, not a re-ordering.
    let ordered = |events: &Vec<Value>| -> Vec<(String, String)> {
        events
            .iter()
            .map(|e| {
                (
                    e["pack"].as_str().unwrap().to_owned(),
                    e["id"].as_str().unwrap().to_owned(),
                )
            })
            .collect()
    };
    let never = ordered(catalog["never"].as_array().unwrap());
    let always = ordered(catalog["always"].as_array().unwrap());
    let mut sorted = never.clone();
    sorted.sort();
    assert_eq!(never, sorted, "never must be sorted by (pack, id)");
    let mut sorted = always.clone();
    sorted.sort();
    assert_eq!(always, sorted, "always must be sorted by (pack, id)");
}

/// Byte offset of `key` in `doc`, searched from `cursor` and moving the
/// cursor past the match — the same moving-cursor walk
/// `coverage_json_tests.rs` uses, because parsing into a `Value` re-sorts
/// keys and would silently forgive a reordering.
fn find_after(doc: &str, cursor: &mut usize, key: &str) {
    let found = doc[*cursor..]
        .find(key)
        .unwrap_or_else(|| panic!("expected {key} after byte {}", *cursor));
    *cursor += found + key.len();
}

/// The catalog document's top-level fields are emitted in `Catalog`'s
/// declaration order — the same promise `coverage/v1` pins for its own
/// document. The event-level wire order is already pinned, indirectly but
/// exactly, by `catalog_digest_matches_the_documented_recipe`; the document
/// level has no such accidental pin, and a reordering is a wire-format
/// change a byte-diffing consumer would see as noise on every policy edit.
#[test]
fn catalog_document_fields_are_emitted_in_declaration_order() {
    let output = icg(&["catalog", "--json", "--pack", "packs"]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let doc = String::from_utf8(output.stdout).expect("stdout is utf-8");

    let mut cursor = 0;
    for key in [
        "\"format\"",
        "\"catalog_digest\"",
        "\"icg_version\"",
        "\"never\"",
        "\"always\"",
    ] {
        find_after(&doc, &mut cursor, key);
    }
}

/// The document occupies stdout alone: byte 0 is `{`, the first emitted key
/// is the format discriminator, and stderr stays silent on success — a
/// consumer piping stdout into a streaming parser can dispatch on
/// `icg-catalog/v1` from the first bytes and treat stderr as fault-only.
/// (`--debug` traces are `icg check`'s stream contract, pinned in
/// `check_output_contract_tests.rs`; the catalog does not accept the flag,
/// so no trace can ever interleave with the document — the full parse below
/// also proves the stream is exactly one object against trailing noise.)
#[test]
fn the_catalog_occupies_stdout_alone_with_stderr_fault_only() {
    let output = icg(&["catalog", "--json", "--pack", "packs"]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        output.stderr.is_empty(),
        "a successful render keeps stderr fault-only: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let doc = String::from_utf8(output.stdout).expect("stdout is utf-8");
    assert!(
        doc.starts_with('{'),
        "byte 0 opens the object — no banner or prefix may precede it: {doc:?}"
    );
    let first_key = doc[1..].trim_start();
    assert!(
        first_key.starts_with("\"format\""),
        "format is the first emitted key, got: {first_key:?}"
    );
    serde_json::from_str::<Value>(&doc)
        .expect("the whole stdout stream is exactly one JSON document");
}

/// A rule that ships disabled is cataloged — a consumer must be able to see
/// the whole policy surface, including the part not currently enforced —
/// but carries `enabled: false` so a gap detector does not treat it as a
/// hole in the gate.
#[test]
fn disabled_rules_are_cataloged_but_marked_not_enforced() {
    let mut pack = Pack {
        id: "catalog-test-pack".to_string(),
        tool_keywords: vec!["off-command".to_string()],
        applies_to: vec![],
        safe_patterns: vec![],
        guarded_patterns: vec![],
    };
    pack.guarded_patterns.push(icg::rule_pack::GuardedPattern {
        id: "off-rule".to_string(),
        enabled: false,
        check: Check::CommandRegex {
            regex: "^off-command".to_string(),
        },
        tier: Tier::Tier1,
        severity: Severity::Medium,
        explanation: "off-rule exists to prove disabled rules stay visible".to_string(),
        redirect: Redirect {
            channel: Channel::Deny,
            reason_template: "do the off-command the sanctioned way instead".to_string(),
            rewrite_template: None,
        },
        destructive: false,
    });

    let built = catalog::build(&[pack]).expect("pack should build");
    let event = built
        .never
        .iter()
        .find(|e| e.id == "off-rule")
        .expect("a disabled rule is still cataloged");
    assert!(
        !event.enabled,
        "the catalog must mark the rule not enforced"
    );

    // And the engine agrees it is not enforced: the event executing is not a
    // gate gap, because the gate never claimed to catch it.
    let mut engine = Engine::new();
    engine
        .load_pack(shipped_pack_with_disabled_rule())
        .expect("pack should validate");
    assert!(matches!(
        engine.evaluate_command(&CommandSource::Hook("off-command --force".to_string())),
        CheckResult::Allowed
    ));
}

/// Rebuild the same single-pack fixture through the JSON wire format, so
/// the disabled-rule assertion above runs against what a pack author would
/// actually ship rather than an in-memory struct.
fn shipped_pack_with_disabled_rule() -> Pack {
    let temp = tempfile::tempdir().expect("temp dir");
    let path = temp.path().join("off.json");
    fs::write(
        &path,
        serde_json::to_vec_pretty(&json!({
            "id": "catalog-test-pack-wire",
            "tool_keywords": ["off-command"],
            "applies_to": [],
            "safe_patterns": [],
            "guarded_patterns": [{
                "id": "off-rule-wire",
                "enabled": false,
                "type": "command_regex",
                "regex": "^off-command",
                "tier": "tier1",
                "severity": "Medium",
                "explanation": "off-rule exists to prove disabled rules stay visible",
                "redirect": {
                    "channel": "deny",
                    "reason_template": "do the off-command the sanctioned way instead"
                },
                "destructive": false
            }]
        }))
        .unwrap(),
    )
    .expect("pack fixture should write");
    load_pack(&path).expect("wire fixture should load")
}

#[test]
fn icg_catalog_without_json_flag_is_the_same_command() {
    let bare = icg(&["catalog", "--pack", "packs"]);
    assert!(bare.status.success(), "bare icg catalog should succeed");
    let flagged = icg(&["catalog", "--json", "--pack", "packs"]);
    assert_eq!(
        bare.stdout, flagged.stdout,
        "--json is the documented spelling of the only output the catalog has"
    );
}

#[test]
fn catalog_refuses_to_render_when_a_pack_is_unreadable() {
    // A catalog describing only a *subset* of the packs on disk would be
    // indistinguishable from a legitimate policy change, so a broken pack
    // fails the export instead of quietly shrinking it.
    let temp = tempfile::tempdir().expect("temp dir");
    fs::write(temp.path().join("broken.json"), "{not json").expect("broken fixture writes");
    let output = icg(&["catalog", "--json", "--pack", temp.path().to_str().unwrap()]);
    assert!(
        !output.status.success(),
        "a broken pack must fail the export"
    );
    assert!(
        output.stdout.is_empty(),
        "no catalog document is printed for a broken pack tree"
    );
}

/// The exported event is the pack rule, field for field — severity, tier,
/// action, destructive, enabled, check, match, explanation, and the
/// sanctioned alternative. `catalog_exports_every_shipped_event` proves the
/// catalog is *complete* (the id sets match disk); this proves it is
/// *faithful*: what a consumer reads is the same data the engine enforces,
/// because both are rendered from the same loaded pack. (The CLI document
/// and this library render are one document — the digest-agreement test
/// above pins that, since the binary hashes its own rendering.)
#[test]
fn catalog_event_fields_match_their_pack_rules() {
    fn expected_expression(check: &Check) -> MatchExpression {
        match check {
            Check::CommandRegex { regex } | Check::ContentRegex { regex } => {
                MatchExpression::Regex {
                    regex: regex.clone(),
                }
            }
            Check::Predicate { predicate_name, .. } => MatchExpression::Predicate {
                predicate: predicate_name.clone(),
            },
        }
    }
    fn expected_check_kind(check: &Check) -> &'static str {
        match check {
            Check::CommandRegex { .. } => "command_regex",
            Check::ContentRegex { .. } => "content_regex",
            Check::Predicate { .. } => "predicate",
        }
    }

    let built = catalog::build(&shipped_packs()).expect("shipped packs should build");
    let never_by_key: std::collections::HashMap<(&str, &str), &catalog::CatalogEvent> = built
        .never
        .iter()
        .map(|event| ((event.pack.as_str(), event.id.as_str()), event))
        .collect();
    let always_by_key: std::collections::HashMap<(&str, &str), &catalog::AlwaysEvent> = built
        .always
        .iter()
        .map(|event| ((event.pack.as_str(), event.id.as_str()), event))
        .collect();

    for pack in shipped_packs() {
        for rule in &pack.guarded_patterns {
            let what = format!("{}/{}", pack.id, rule.id);
            let event = never_by_key
                .get(&(pack.id.as_str(), rule.id.as_str()))
                .unwrap_or_else(|| panic!("catalog must contain {what}"));
            assert_eq!(event.severity, rule.severity, "{what}: severity");
            assert_eq!(event.tier, rule.tier, "{what}: tier");
            assert_eq!(event.action, rule.redirect.channel, "{what}: action");
            assert_eq!(event.destructive, rule.destructive, "{what}: destructive");
            assert_eq!(event.enabled, rule.enabled, "{what}: enabled");
            assert_eq!(
                event.check,
                expected_check_kind(&rule.check),
                "{what}: check"
            );
            assert_eq!(
                event.matching,
                expected_expression(&rule.check),
                "{what}: match"
            );
            assert_eq!(event.explanation, rule.explanation, "{what}: explanation");
            assert_eq!(
                event.sanctioned_alternative.reason, rule.redirect.reason_template,
                "{what}: sanctioned_alternative.reason"
            );
            assert_eq!(
                event.sanctioned_alternative.rewrite, rule.redirect.rewrite_template,
                "{what}: sanctioned_alternative.rewrite"
            );
        }
        for pattern in &pack.safe_patterns {
            let what = format!("{}/{}", pack.id, pattern.id);
            let event = always_by_key
                .get(&(pack.id.as_str(), pattern.id.as_str()))
                .unwrap_or_else(|| panic!("catalog must contain {what}"));
            assert_eq!(
                event.check,
                expected_check_kind(&pattern.check),
                "{what}: check"
            );
            assert_eq!(
                event.matching,
                expected_expression(&pattern.check),
                "{what}: match"
            );
        }
    }
}

/// One guarded rule, rebuilt fresh per call so each digest vector below
/// starts from the identical policy.
fn digest_guarded_rule(id: &str) -> icg::rule_pack::GuardedPattern {
    icg::rule_pack::GuardedPattern {
        id: id.to_string(),
        enabled: true,
        check: Check::CommandRegex {
            regex: format!("^{id} "),
        },
        tier: Tier::Tier1,
        severity: Severity::High,
        explanation: format!("{id} is irreversible"),
        redirect: Redirect {
            channel: Channel::Deny,
            reason_template: format!("run {id} --dry-run first"),
            rewrite_template: None,
        },
        destructive: true,
    }
}

fn digest_safe_pattern(id: &str) -> icg::rule_pack::Pattern {
    icg::rule_pack::Pattern {
        id: id.to_string(),
        check: Check::CommandRegex {
            regex: format!("^{id} --dry-run"),
        },
    }
}

fn digest_fixture_pack() -> Pack {
    Pack {
        id: "digest-fixture".to_string(),
        tool_keywords: vec!["digestcmd".to_string()],
        applies_to: vec![],
        safe_patterns: vec![digest_safe_pattern("digestcmd-dry-run")],
        guarded_patterns: vec![digest_guarded_rule("digestcmd-destroy")],
    }
}

/// The digest is the consumer's drift signal, so every policy edit the
/// contract note promises must move it: "a rule added, disabled, reworded,
/// or removed". A digest that survived any of those would let a consumer
/// keep reasoning about a policy that no longer exists. This pins each
/// promised vector plus the quieter edits (a severity retune, a channel
/// change, an always-allowed pattern change), with unchanged policy as the
/// control.
#[test]
fn digest_moves_when_the_policy_changes() {
    let digest = |pack: Pack| {
        catalog::build(&[pack])
            .expect("fixture pack should build")
            .catalog_digest
    };
    let base = digest(digest_fixture_pack());
    assert_eq!(
        base,
        digest(digest_fixture_pack()),
        "unchanged policy must keep the digest stable"
    );
    let moved = |label: &str, pack: Pack| {
        assert_ne!(base, digest(pack), "{label} must move the digest");
    };

    // A guarded rule added / removed.
    let mut added = digest_fixture_pack();
    added
        .guarded_patterns
        .push(digest_guarded_rule("digestcmd-wipe"));
    moved("adding a guarded rule", added);
    let mut removed = digest_fixture_pack();
    removed.guarded_patterns.clear();
    moved("removing a guarded rule", removed);

    // The quieter edits — none change the event set's shape, but every one
    // changes what the policy says. The id rename is the quietest of all:
    // the rule is field-for-field identical, but (pack, id) is the identity
    // a denial record carries, so a rename must move the digest or a
    // consumer keeps matching denials against an event that no longer
    // exists under the id it records.
    let mut renamed = digest_fixture_pack();
    renamed.guarded_patterns[0].id = "digestcmd-destroy-renamed".to_string();
    moved("a rule id rename", renamed);

    let mut reworded = digest_fixture_pack();
    reworded.guarded_patterns[0].explanation = "reworded: digestcmd destroy loses data".to_string();
    moved("a reworded explanation", reworded);

    let mut alternative = digest_fixture_pack();
    alternative.guarded_patterns[0].redirect.reason_template =
        "reworded: run digestcmd destroy --dry-run first".to_string();
    moved("a reworded sanctioned alternative", alternative);

    let mut disabled = digest_fixture_pack();
    disabled.guarded_patterns[0].enabled = false;
    moved("a rule disabled", disabled);

    let mut retuned = digest_fixture_pack();
    retuned.guarded_patterns[0].severity = Severity::Critical;
    moved("a severity retune", retuned);

    let mut channel = digest_fixture_pack();
    channel.guarded_patterns[0].redirect.channel = Channel::UpdatedInput;
    channel.guarded_patterns[0].redirect.rewrite_template =
        Some("digestcmd destroy --dry-run".to_string());
    moved("a channel change", channel);

    // The always side is part of the event set too.
    let mut safe_added = digest_fixture_pack();
    safe_added
        .safe_patterns
        .push(digest_safe_pattern("digestcmd-inspect"));
    moved("an always-allowed pattern added", safe_added);
    let mut safe_removed = digest_fixture_pack();
    safe_removed.safe_patterns.clear();
    moved("an always-allowed pattern removed", safe_removed);
}

/// "Repeated values naming the same path are deduplicated": a consumer
/// assembling --pack values from layered config must not see doubled
/// events. Dedup plus the (pack, id) sort makes the catalog byte-identical
/// to the single-path invocation.
#[test]
fn repeated_pack_paths_are_deduplicated() {
    let once = icg(&["catalog", "--json", "--pack", "packs"]);
    let twice = icg(&["catalog", "--json", "--pack", "packs", "--pack", "packs"]);
    assert!(once.status.success() && twice.status.success());
    assert_eq!(
        once.stdout, twice.stdout,
        "a repeated --pack value must not change the catalog"
    );
}

/// The note defines the digest as "the SHA-256 of the canonical JSON
/// serialization of `{"format": "icg-catalog/v1", "never": [...], "always":
/// [...]}` — the entire event set and nothing else". Every other digest
/// assertion here compares one `catalog::build` against another, so they
/// share the recipe and cannot see it change: fold `icg_version` (or
/// anything else) into the hash and they all still pass while the note
/// starts lying. This recomputes the digest from the emitted document alone,
/// exactly as a consumer following the note would, so the recipe itself is
/// pinned and `icg_version` is provably not an input.
///
/// The event mirrors repeat the wire field order field for field. The
/// canonical serialization's key order is the struct declaration order, and
/// a round-trip through `serde_json::Value` would silently re-sort it — so
/// the arrays are parsed into these mirrors to preserve it. A field the
/// implementation adds but the mirror drops therefore breaks this test:
/// that is the point. The digest's bytes move with any shape change, the
/// note says so, and the mirror + note + format version move together in
/// the same commit or nothing builds. (`match` and
/// `sanctioned_alternative` need no mirror — a tagged union is always one
/// key, and `reason`/`rewrite` are already in sort order.)
#[derive(serde::Deserialize, serde::Serialize)]
struct DigestInputEvent {
    id: String,
    pack: String,
    severity: String,
    tier: String,
    action: String,
    destructive: bool,
    enabled: bool,
    check: String,
    #[serde(rename = "match")]
    matching: Value,
    explanation: String,
    sanctioned_alternative: Value,
}

#[derive(serde::Deserialize, serde::Serialize)]
struct DigestInputAlwaysEvent {
    id: String,
    pack: String,
    check: String,
    #[serde(rename = "match")]
    matching: Value,
}

#[derive(serde::Serialize)]
struct DocumentedDigestInput<'a> {
    format: &'a str,
    never: Vec<DigestInputEvent>,
    always: Vec<DigestInputAlwaysEvent>,
}

#[test]
fn catalog_digest_matches_the_documented_recipe() {
    let catalog = catalog_from_cli();
    let input = DocumentedDigestInput {
        format: catalog["format"].as_str().unwrap(),
        never: serde_json::from_value(catalog["never"].clone()).expect("never array"),
        always: serde_json::from_value(catalog["always"].clone()).expect("always array"),
    };
    let canonical = serde_json::to_vec(&input).expect("digest input should serialize");
    use sha2::Digest as _;
    let mut hasher = sha2::Sha256::new();
    hasher.update(&canonical);
    let recomputed: String = hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    assert_eq!(
        catalog["catalog_digest"].as_str().unwrap(),
        recomputed,
        "catalog_digest must be SHA-256 of {{format, never, always}} exactly as the \
         note defines it — icg_version and nothing else may enter the hash"
    );
}

/// The note's error contract: a pack that fails to load — malformed JSON,
/// failed validation, an unreadable file — exits non-zero, prints nothing on
/// stdout, and puts an `Error:` line on stderr. This pins the full shape,
/// including the validation arm (well-formed JSON that is not a valid pack),
/// a `--pack` path that does not exist, and the genuinely unreadable file.
#[test]
fn broken_packs_fail_the_export_with_the_documented_error_shape() {
    let temp = tempfile::tempdir().expect("temp dir");
    let dir = temp.path();

    // Malformed JSON.
    fs::write(dir.join("broken.json"), "{not json").expect("broken fixture writes");
    // Well-formed JSON whose guarded rule is missing a required field —
    // readable, parseable, and still not a pack.
    fs::write(
        dir.join("invalid.json"),
        r#"{"id":"shape-pack","tool_keywords":["shapecmd"],"applies_to":[],"safe_patterns":[],
            "guarded_patterns":[{"id":"shape-rule","type":"command_regex","regex":"^shapecmd",
            "tier":"tier1","explanation":"no severity here",
            "redirect":{"channel":"deny","reason_template":"r"},"destructive":false}]}"#,
    )
    .expect("invalid fixture writes");

    for name in ["broken.json", "invalid.json"] {
        let path = dir.join(name);
        let output = icg(&["catalog", "--json", "--pack", path.to_str().unwrap()]);
        assert!(
            !output.status.success(),
            "{name}: a pack that fails to load must fail the export"
        );
        assert!(
            output.stdout.is_empty(),
            "{name}: no catalog document is printed"
        );
        assert!(
            String::from_utf8_lossy(&output.stderr).starts_with("Error:"),
            "{name}: stderr must carry an Error: line, got {:?}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    // An explicit --pack path that does not exist is a caller error, not an
    // empty catalog.
    let missing = dir.join("does-not-exist.json");
    let output = icg(&["catalog", "--json", "--pack", missing.to_str().unwrap()]);
    assert!(
        !output.status.success(),
        "a nonexistent explicit --pack path must fail the export"
    );
    assert!(output.stdout.is_empty());
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("does not exist"),
        "the error should say the path does not exist, got {:?}",
        String::from_utf8_lossy(&output.stderr)
    );

    // The note's third arm: an actually *unreadable* file — well-formed JSON
    // sitting on disk, but the loader cannot open it. A catalog describing
    // only the readable packs would be indistinguishable from a legitimate
    // policy change, so a permission failure must fail the export exactly
    // like a parse failure. Root reads through 0o000 (CAP_DAC_OVERRIDE), so
    // the arm is skipped loudly where it cannot be produced.
    if unsafe { libc::geteuid() } == 0 {
        eprintln!("skipping the unreadable-pack arm: root reads through 0o000");
        return;
    }
    let unreadable = dir.join("unreadable.json");
    fs::write(
        &unreadable,
        r#"{"id":"unreadable-pack","tool_keywords":["unreadable-cmd"],"applies_to":[],
            "safe_patterns":[],"guarded_patterns":[]}"#,
    )
    .expect("unreadable fixture writes");
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&unreadable, fs::Permissions::from_mode(0o000))
            .expect("unreadable fixture permissions");
    }
    let output = icg(&["catalog", "--json", "--pack", unreadable.to_str().unwrap()]);
    assert!(
        !output.status.success(),
        "an unreadable pack file must fail the export"
    );
    assert!(
        output.stdout.is_empty(),
        "no catalog document is printed for an unreadable pack"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.starts_with("Error:"),
        "stderr must carry an Error: line, got {stderr:?}"
    );
    assert!(
        stderr.contains("Failed to read rule pack"),
        "the error must be the read arm, not a parse arm, got {stderr:?}"
    );
}

/// "A file whose name does not end in `.json` is not treated as a pack,
/// even when it exists." A consumer assembling --pack values from layered
/// config relies on that gate: a stray `pack.txt` next to the real packs
/// must neither load nor shrink the catalog. Both spellings fail loudly
/// rather than rendering a silent subset.
#[test]
fn a_non_json_file_is_not_treated_as_a_pack_even_when_it_exists() {
    let temp = tempfile::tempdir().expect("temp dir");
    let valid_pack = json!({
        "id": "extension-gate-pack",
        "tool_keywords": ["extcmd"],
        "applies_to": [],
        "safe_patterns": [],
        "guarded_patterns": [{
            "id": "ext-rule",
            "enabled": true,
            "type": "command_regex",
            "regex": "^extcmd",
            "tier": "tier1",
            "severity": "High",
            "explanation": "ext-rule is irreversible",
            "redirect": {"channel": "deny", "reason_template": "run extcmd --dry-run first"},
            "destructive": false
        }]
    });
    let wrong_extension = temp.path().join("pack.txt");
    fs::write(
        &wrong_extension,
        serde_json::to_string_pretty(&valid_pack).expect("fixture serializes"),
    )
    .expect("fixture writes");

    // Named explicitly: the file exists, is readable, holds a valid pack —
    // and the extension gate still refuses it.
    let output = icg(&[
        "catalog",
        "--json",
        "--pack",
        wrong_extension.to_str().unwrap(),
    ]);
    assert!(
        !output.status.success(),
        "an existing non-.json file must not be treated as a pack"
    );
    assert!(output.stdout.is_empty());

    // Offered as a directory: the .txt entry contributes nothing, so the
    // export fails rather than rendering a catalog without its pack.
    let output = icg(&["catalog", "--json", "--pack", temp.path().to_str().unwrap()]);
    assert!(
        !output.status.success(),
        "a pack directory with no .json entries must fail the export, not render the built-ins alone"
    );
    assert!(output.stdout.is_empty());
}

/// "Duplicate ids within one pack are a pack-authoring bug and fail the
/// export loudly rather than silently collapsing two events into one."
/// `duplicate_ids_within_a_pack_fail_loudly` (in `src/catalog.rs`) proves
/// `build` refuses; this proves the *export command* refuses, over the wire
/// format a pack author actually ships. Event identity is (pack, id) — a
/// silent collapse would make a denial record ambiguous, on the never side
/// and (the quieter path) the always side alike.
#[test]
fn duplicate_ids_fail_the_export_command_loudly() {
    let temp = tempfile::tempdir().expect("temp dir");
    let guarded_rule = |regex: &str| {
        json!({
            "id": "dup-rule",
            "enabled": true,
            "type": "command_regex",
            "regex": regex,
            "tier": "tier1",
            "severity": "High",
            "explanation": "dup-rule cannot be undone",
            "redirect": {"channel": "deny", "reason_template": "run dupcmd --dry-run first"},
            "destructive": true
        })
    };
    let pack = |safe_patterns: Value, guarded_patterns: Value| {
        json!({
            "id": "duplicate-ids-pack",
            "tool_keywords": ["dupcmd"],
            "applies_to": [],
            "safe_patterns": safe_patterns,
            "guarded_patterns": guarded_patterns
        })
    };

    // Two distinct events, one id — the pack-authoring bug the note names.
    let cases = [
        (
            "dup-never.json",
            pack(
                json!([]),
                json!([guarded_rule("^dupcmd a "), guarded_rule("^dupcmd b ")]),
            ),
            "event",
        ),
        (
            "dup-always.json",
            pack(
                json!([
                    {"id": "dup-safe", "type": "command_regex", "regex": "^dupcmd --dry-run-a"},
                    {"id": "dup-safe", "type": "command_regex", "regex": "^dupcmd --dry-run-b"}
                ]),
                json!([]),
            ),
            "safe-pattern",
        ),
    ];

    for (name, pack, what) in cases {
        let path = temp.path().join(name);
        fs::write(
            &path,
            serde_json::to_string_pretty(&pack).expect("fixture serializes"),
        )
        .expect("fixture writes");
        let output = icg(&["catalog", "--json", "--pack", path.to_str().unwrap()]);
        assert!(
            !output.status.success(),
            "duplicate {what} ids must fail the export"
        );
        assert!(output.stdout.is_empty(), "no document is printed");
        assert!(
            String::from_utf8_lossy(&output.stderr).contains("duplicate"),
            "the error should name the duplicate, got {:?}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

/// "The two built-in guards are always included; they are not packs and
/// cannot be excluded." Rendering from a fixture pack that names neither
/// built-in must still emit both entries under their synthetic pack ids —
/// a consumer's gap detection counts on the built-ins being in every
/// document, because the engine enforces them in code before any pack.
#[test]
fn builtin_guards_are_included_even_when_no_pack_names_them() {
    let temp = tempfile::tempdir().expect("temp dir");
    fs::write(
        temp.path().join("only.json"),
        serde_json::to_string_pretty(&json!({
            "id": "builtin-exclusion-pack",
            "tool_keywords": ["bexcmd"],
            "applies_to": [],
            "safe_patterns": [],
            "guarded_patterns": [{
                "id": "bex-rule",
                "enabled": true,
                "type": "command_regex",
                "regex": "^bexcmd",
                "tier": "tier1",
                "severity": "Medium",
                "explanation": "bex-rule cannot be undone",
                "redirect": {"channel": "deny", "reason_template": "run bexcmd --dry-run first"},
                "destructive": false
            }]
        }))
        .expect("fixture serializes"),
    )
    .expect("fixture writes");

    let output = icg(&["catalog", "--json", "--pack", temp.path().to_str().unwrap()]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let catalog: Value =
        serde_json::from_slice(&output.stdout).expect("catalog should emit valid JSON");
    let exported: BTreeSet<(String, String)> = catalog["never"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| {
            (
                e["pack"].as_str().unwrap().to_owned(),
                e["id"].as_str().unwrap().to_owned(),
            )
        })
        .collect();
    for builtin in [
        (WORKFLOWS_PACK, WORKFLOWS_PATTERN),
        (JOBCRON_PACK, JOBCRON_PATTERN),
    ] {
        assert!(
            exported.contains(&(builtin.0.to_string(), builtin.1.to_string())),
            "{}/{} must be in every catalog, whatever packs were named",
            builtin.0,
            builtin.1
        );
    }
}

/// "With no `--pack`, the loader uses `ICG_PACK_DIR` when that environment
/// variable is set" — the same default chain as `coverage` and the hook,
/// spelled for the catalog. The environment is set for the child process
/// only; no other test sees it.
#[test]
fn no_pack_argument_uses_icg_pack_dir() {
    let temp = tempfile::tempdir().expect("temp dir");
    fs::write(
        temp.path().join("env-dir.json"),
        serde_json::to_string_pretty(&json!({
            "id": "icg-pack-dir-pack",
            "tool_keywords": ["envcmd"],
            "applies_to": [],
            "safe_patterns": [],
            "guarded_patterns": [{
                "id": "env-dir-rule",
                "enabled": true,
                "type": "command_regex",
                "regex": "^envcmd",
                "tier": "tier1",
                "severity": "High",
                "explanation": "env-dir-rule cannot be undone",
                "redirect": {"channel": "deny", "reason_template": "run envcmd --dry-run first"},
                "destructive": false
            }]
        }))
        .expect("fixture serializes"),
    )
    .expect("fixture writes");

    let output = Command::new(env!("CARGO_BIN_EXE_icg"))
        .args(["catalog", "--json"])
        .env("ICG_PACK_DIR", temp.path())
        .output()
        .expect("icg should run");
    assert!(
        output.status.success(),
        "ICG_PACK_DIR should feed the no-argument catalog: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let catalog: Value =
        serde_json::from_slice(&output.stdout).expect("catalog should emit valid JSON");
    let exported: BTreeSet<(String, String)> = catalog["never"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| {
            (
                e["pack"].as_str().unwrap().to_owned(),
                e["id"].as_str().unwrap().to_owned(),
            )
        })
        .collect();
    assert!(
        exported.contains(&("icg-pack-dir-pack".to_string(), "env-dir-rule".to_string())),
        "the pack in ICG_PACK_DIR must be cataloged"
    );
    assert!(
        exported.contains(&(WORKFLOWS_PACK.to_string(), WORKFLOWS_PATTERN.to_string())),
        "the built-ins ride along on the environment-resolved catalog too"
    );
}

/// "`icg explain --pattern <id>` renders one event's full caller-facing
/// redirect. The catalog's `id` is the same key" — for every *pack* event,
/// never and always alike. The two built-in guards are code, not packs, and
/// `explain` reads packs only, so their ids resolve in denial records and in
/// the catalog but deliberately not in `explain`; the note says exactly
/// that, and this pins both halves so neither the contract note nor
/// `explain` can drift from the other.
#[test]
fn icg_explain_accepts_every_pack_event_id() {
    let catalog = catalog_from_cli();
    let ids = |array: &str| -> Vec<(String, String)> {
        catalog[array]
            .as_array()
            .unwrap()
            .iter()
            .map(|e| {
                (
                    e["pack"].as_str().unwrap().to_owned(),
                    e["id"].as_str().unwrap().to_owned(),
                )
            })
            .collect()
    };

    for (pack, id) in ids("never").into_iter().chain(ids("always")) {
        let output = icg(&["explain", "--pattern", &id, "--pack", "packs"]);
        if pack == WORKFLOWS_PACK || pack == JOBCRON_PACK {
            assert!(
                !output.status.success(),
                "{id}: a built-in guard is code, not a pack, and explain must not pretend to render it"
            );
        } else {
            assert!(
                output.status.success(),
                "{pack}/{id}: the catalog's id must be the key icg explain --pattern accepts: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        }
    }
}

/// The note is the contract; this keeps its field tables synchronized with
/// the document the CLI actually emits, in both directions. The wire shape
/// is already pinned by `catalog_schema_pins_its_key_sets` — this pins the
/// *prose* to the same reality, so a field renamed in code fails here until
/// the note's tables move with it (and a table edit that invents a field
/// fails too). The table sections are named by their exact lead-ins; a
/// reworded lead-in fails loudly and is fixed alongside the doc it renamed.
#[test]
fn catalog_note_field_tables_match_the_export() {
    let note = fs::read_to_string("docs/notes/event-catalog-json-api.md")
        .expect("the contract note should exist");

    assert!(
        note.contains(CATALOG_FORMAT),
        "the note must name the wire contract {CATALOG_FORMAT} verbatim"
    );
    // The Versioning section may cite a hypothetical next version as policy;
    // anywhere else, the only contract the note may name is the shipped one.
    let versioning = note
        .find("## Versioning")
        .expect("the note should have a Versioning section");
    let before_versioning = &note[..versioning];
    assert!(
        !before_versioning.contains("icg-catalog/v2"),
        "outside the Versioning policy the note must not name an unshipped format version"
    );
    assert!(
        !note.contains("icg-catalog/v3"),
        "the note must not promise a format version that does not exist"
    );

    let documented_fields = |section_start: &str, section_end: &str| -> BTreeSet<String> {
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
    };

    let catalog = catalog_from_cli();
    let top_level: BTreeSet<String> = catalog.as_object().unwrap().keys().cloned().collect();
    let emitted_keys = |array: &str| -> BTreeSet<String> {
        catalog[array]
            .as_array()
            .unwrap()
            .first()
            .expect("the shipped catalog is never empty")
            .as_object()
            .unwrap()
            .keys()
            .cloned()
            .collect()
    };

    let sync = |what: &str, documented: BTreeSet<String>, emitted: &BTreeSet<String>| {
        assert_eq!(
            documented, *emitted,
            "{what}: the note's field table and the emitted document disagree"
        );
    };
    sync(
        "top level",
        documented_fields("One JSON object on stdout", "Each entry of"),
        &top_level,
    );
    sync(
        "never events",
        documented_fields("Each entry of `never`:", "Each `sanctioned_alternative`:"),
        &emitted_keys("never"),
    );
    sync(
        "always events",
        documented_fields("Each entry of `always`:", "Always events carry no severity"),
        &emitted_keys("always"),
    );
    // The sanctioned_alternative table is a subset check: `rewrite` is
    // optional and absent from every shipped event, so no emitted document
    // carries both keys to compare against.
    let alternative = documented_fields("Each `sanctioned_alternative`:", "Each entry of");
    assert_eq!(
        alternative,
        BTreeSet::from(["reason".to_string(), "rewrite".to_string()]),
        "the note's sanctioned_alternative table should document reason and optional rewrite"
    );
}

/// The contract note's reason for existing: a consumer stores the digest it
/// last consumed and compares — equal means the policy it reasoned about is
/// unchanged, different means re-read. This runs that workflow end to end
/// over the wire format. The consumer's whole view is three stdout
/// documents; between renders the policy author edits the pack file, and
/// each edit must move the digest while the document diff localizes the
/// change — without the consumer ever parsing a rule pack.
#[test]
fn a_consumer_detects_policy_drift_without_parsing_rule_packs() {
    let temp = tempfile::tempdir().expect("temp dir");
    let pack_path = temp.path().join("drift-watch.json");
    let write_pack = |guarded: Value| {
        fs::write(
            &pack_path,
            serde_json::to_string_pretty(&json!({
                "id": "drift-watch",
                "tool_keywords": ["driftcmd"],
                "applies_to": [],
                "safe_patterns": [{
                    "id": "driftcmd-dry-run",
                    "type": "command_regex",
                    "regex": "^driftcmd --dry-run"
                }],
                "guarded_patterns": guarded
            }))
            .expect("drift fixture should serialize"),
        )
        .expect("drift fixture pack should write");
    };
    let guarded = |id: &str, enabled: bool| {
        json!({
            "id": id,
            "enabled": enabled,
            "type": "command_regex",
            "regex": format!("^{} ", id),
            "tier": "tier1",
            "severity": "High",
            "explanation": format!("{id} cannot be undone"),
            "destructive": true,
            "redirect": {
                "channel": "deny",
                "reason_template": format!("run {id} --dry-run first")
            }
        })
    };
    let render = |label: &str| -> Value {
        let output = icg(&["catalog", "--json", "--pack", pack_path.to_str().unwrap()]);
        assert!(
            output.status.success(),
            "{label}: the drift fixture pack should export: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        serde_json::from_slice(&output.stdout).expect("the catalog should parse")
    };
    let never_keys = |catalog: &Value| -> BTreeSet<(String, String)> {
        catalog["never"]
            .as_array()
            .unwrap()
            .iter()
            .map(|e| {
                (
                    e["pack"].as_str().unwrap().to_owned(),
                    e["id"].as_str().unwrap().to_owned(),
                )
            })
            .collect()
    };
    let always_keys = |catalog: &Value| -> BTreeSet<(String, String)> {
        catalog["always"]
            .as_array()
            .unwrap()
            .iter()
            .map(|e| {
                (
                    e["pack"].as_str().unwrap().to_owned(),
                    e["id"].as_str().unwrap().to_owned(),
                )
            })
            .collect()
    };

    write_pack(json!([guarded("driftcmd-destroy", true)]));
    let first = render("initial");
    let first_digest = first["catalog_digest"].as_str().unwrap().to_owned();

    // Policy edit 1: a rule added alongside the existing one. The stored
    // digest no longer matches, and diffing the two catalogs localizes the
    // change to exactly that event — one added, none removed.
    write_pack(json!([
        guarded("driftcmd-destroy", true),
        guarded("driftcmd-wipe", true)
    ]));
    let second = render("after adding a rule");
    let second_digest = second["catalog_digest"].as_str().unwrap().to_owned();
    assert_ne!(
        second_digest, first_digest,
        "adding a rule must move the digest a consumer last stored"
    );
    let first_never = never_keys(&first);
    let added: Vec<_> = never_keys(&second)
        .difference(&first_never)
        .cloned()
        .collect();
    assert_eq!(
        added,
        vec![("drift-watch".to_string(), "driftcmd-wipe".to_string())],
        "the diff names exactly the added event"
    );
    let second_never = never_keys(&second);
    let removed: Vec<_> = first_never.difference(&second_never).cloned().collect();
    assert!(removed.is_empty(), "the edit removed nothing: {removed:?}");
    assert_eq!(
        always_keys(&second),
        always_keys(&first),
        "the edit touched no always-allowed event"
    );

    // Policy edit 2: the rule disabled. A gap detector reading only ids
    // sees nothing move — the event stays cataloged — but the digest still
    // moves, because what the gate enforces changed.
    write_pack(json!([
        guarded("driftcmd-destroy", true),
        guarded("driftcmd-wipe", false)
    ]));
    let third = render("after disabling a rule");
    assert_ne!(
        third["catalog_digest"].as_str().unwrap(),
        second_digest,
        "disabling an enforced rule must move the digest"
    );
    assert_eq!(
        never_keys(&third),
        never_keys(&second),
        "a disabled rule is cataloged, not removed"
    );
    let wipe = third["never"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["id"] == "driftcmd-wipe")
        .expect("the disabled event is still cataloged");
    assert_eq!(
        wipe["enabled"], false,
        "the document marks the event not enforced"
    );
}
