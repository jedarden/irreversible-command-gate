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

use icg::catalog::{self, CATALOG_FORMAT, MatchExpression};
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
                MatchExpression::Regex { regex: regex.clone() }
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
    // changes what the policy says.
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
