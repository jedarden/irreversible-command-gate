//! The versioned always/never event catalog (`icg catalog`, `icg-catalog/v1`).
//!
//! ICG is the component that owns the authoritative list of events that must
//! never happen and the always-allowed counterpart, and the component that
//! enforces them at the PreToolUse boundary. External consumers — TWILL's
//! D-10 gap detector first among them — must not parse rule packs they do
//! not own, and must not keep a second copy of the list that can drift. This
//! module renders the catalog from the same two sources the engine dispatches
//! on: the loaded rule packs ([`crate::rule_pack`]) and the built-in guards
//! the engine consults before any pack ([`crate::github_workflows`],
//! [`crate::job_cronjob_yaml`]).
//!
//! Nothing here is hand-maintained per event: every id, severity, tier,
//! action, explanation and sanctioned alternative is read out of the pack
//! file or the guard module that owns the enforcement decision. The
//! enumeration of built-in guards below is the one intentional hand-written
//! list, because the engine's dispatch of built-ins is code
//! (`Engine::evaluate_content_inner`), not data. That seam is kept honest by
//! `tests/catalog_export_tests.rs`, which drives the real engine with a
//! triggering input for every built-in entry and asserts the emitted denial
//! attribution equals the catalog's — a guard the catalog forgets, or one
//! the catalog invents, fails a build.
//!
//! The document is versioned twice over, so a consumer can detect drift
//! without understanding its contents: `format` names the wire contract
//! (bump it for a shape change) and `catalog_digest` is the SHA-256 of the
//! event set (identical inputs render byte-identical documents, so any
//! policy change moves the digest).

use crate::rule_pack::{Check, Pack};
use anyhow::{bail, Context, Result};
use serde::Serialize;
use sha2::{Digest, Sha256};

/// The wire contract this module emits. A consumer must refuse a document
/// whose `format` it does not recognize rather than guess at fields.
pub const CATALOG_FORMAT: &str = "icg-catalog/v1";

/// What an event matches, rendered from the same `Check` the engine
/// compiles. Predicate entries name the predicate the engine's registry
/// dispatches on; the predicate's logic is code in this repository, not
/// data a consumer can re-implement from the catalog.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(untagged)]
pub enum MatchExpression {
    /// A regular expression, verbatim from the pack.
    Regex { regex: String },
    /// The name of the engine-side predicate this event dispatches on.
    Predicate { predicate: String },
}

impl MatchExpression {
    fn of(check: &Check) -> Self {
        match check {
            Check::CommandRegex { regex } | Check::ContentRegex { regex } => Self::Regex {
                regex: regex.clone(),
            },
            Check::Predicate { predicate_name, .. } => Self::Predicate {
                predicate: predicate_name.clone(),
            },
        }
    }
}

/// The check kind, spelled the way `icg explain` and the coverage API do.
fn check_kind(check: &Check) -> &'static str {
    match check {
        Check::CommandRegex { .. } => "command_regex",
        Check::ContentRegex { .. } => "content_regex",
        Check::Predicate { .. } => "predicate",
    }
}

/// The alternative the caller is owed instead of the never event.
///
/// `reason` is the engine's reason text. For pack rules it is the raw
/// `reason_template` and may carry `{placeholder}` fields the engine fills
/// from the matching invocation (see `render_reason` in the engine); for
/// built-in guards it is the guard's literal denial reason. `rewrite` is
/// present only for `updated_input` events, where it is the rewrite
/// template the engine substitutes.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct SanctionedAlternative {
    pub reason: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rewrite: Option<String>,
}

/// One event that must never happen.
///
/// `id` and `pack` are the denial record's `pattern_id` and `pack_id` —
/// together they are the event's stable identity, so a consumer can match a
/// recorded denial against the catalog without a translation table.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct CatalogEvent {
    /// Stable event id, as emitted in denial records and accepted by
    /// `icg explain --pattern`.
    pub id: String,
    /// Owning pack id, as emitted in denial records. Built-in guards use
    /// their synthetic pack id (`github-workflows`, `job-cronjob-yaml`).
    pub pack: String,
    /// How dangerous the event is: `Critical`, `High`, or `Medium`.
    pub severity: crate::rule_pack::Severity,
    /// Deterministic-difficulty tier of the check: `tier1`, `tier2`, or
    /// `tier3`.
    pub tier: crate::rule_pack::Tier,
    /// The engine's response channel when the event is caught: `deny`,
    /// `updated_input`, or `additional_context`.
    pub action: crate::rule_pack::Channel,
    /// Whether the event guards an irreversible operation.
    pub destructive: bool,
    /// False when the rule ships disabled; a disabled event is cataloged
    /// but the engine does not enforce it. Built-in guards are code and are
    /// always enabled.
    pub enabled: bool,
    /// How the event is recognized: `command_regex`, `content_regex`, or
    /// `predicate`.
    pub check: String,
    /// What the check matches, from the same source the engine compiles.
    #[serde(rename = "match")]
    pub matching: MatchExpression,
    /// Why this event must never happen.
    pub explanation: String,
    /// The alternative the caller is owed instead of the event.
    pub sanctioned_alternative: SanctionedAlternative,
}

/// One event that is always allowed.
///
/// Safe patterns are the always-allowed counterpart of the never events:
/// matching one skips the rest of its pack's guarded patterns, so a consumer
/// reasoning about "why was this allowed" needs them in the same catalog.
/// They carry no severity, action or alternative — being allowed is the
/// alternative.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct AlwaysEvent {
    /// Stable event id, as accepted by `icg explain --pattern`.
    pub id: String,
    /// Owning pack id.
    pub pack: String,
    /// How the event is recognized.
    pub check: String,
    /// What the check matches.
    #[serde(rename = "match")]
    pub matching: MatchExpression,
}

/// The complete exported catalog.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct Catalog {
    /// The wire contract, [`CATALOG_FORMAT`].
    pub format: &'static str,
    /// SHA-256 over the canonical serialization of `format`, `never` and
    /// `always` — the entire event set, and nothing installation-specific.
    /// Identical inputs render byte-identical documents, so this is the
    /// drift signal: a consumer stores the digest it last consumed and any
    /// change to the event set moves it. The `icg` release version is
    /// deliberately *not* an input, so a binary bump alone does not
    /// masquerade as a policy change.
    pub catalog_digest: String,
    /// The `icg` version that rendered the document, for provenance.
    pub icg_version: String,
    /// Events that must never happen, ordered by (`pack`, `id`).
    pub never: Vec<CatalogEvent>,
    /// Events that are always allowed, ordered by (`pack`, `id`).
    pub always: Vec<AlwaysEvent>,
}

/// The digest input: everything the digest covers, and nothing else.
#[derive(Serialize)]
struct CatalogContent<'a> {
    format: &'static str,
    never: &'a [CatalogEvent],
    always: &'a [AlwaysEvent],
}

fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    let digest = hasher.finalize();
    let mut hex = String::with_capacity(digest.len() * 2);
    for byte in digest {
        hex.push_str(&format!("{byte:02x}"));
    }
    hex
}

/// Render one pack guarded pattern as a never event.
fn never_event(pack_id: &str, rule: &crate::rule_pack::GuardedPattern) -> CatalogEvent {
    CatalogEvent {
        id: rule.id.clone(),
        pack: pack_id.to_string(),
        severity: rule.severity,
        tier: rule.tier,
        action: rule.redirect.channel,
        destructive: rule.destructive,
        enabled: rule.enabled,
        check: check_kind(&rule.check).to_string(),
        matching: MatchExpression::of(&rule.check),
        explanation: rule.explanation.clone(),
        sanctioned_alternative: SanctionedAlternative {
            reason: rule.redirect.reason_template.clone(),
            rewrite: rule.redirect.rewrite_template.clone(),
        },
    }
}

/// Render one pack safe pattern as an always event.
fn always_event(pack_id: &str, pattern: &crate::rule_pack::Pattern) -> AlwaysEvent {
    AlwaysEvent {
        id: pattern.id.clone(),
        pack: pack_id.to_string(),
        check: check_kind(&pattern.check).to_string(),
        matching: MatchExpression::of(&pattern.check),
    }
}

/// The built-in guards the engine consults before any pack. One entry per
/// arm of `Engine::evaluate_content_inner`'s built-in section; the test
/// suite proves the two lists agree by driving the engine itself.
fn builtin_events() -> Vec<CatalogEvent> {
    vec![
        crate::github_workflows::catalog_event(),
        crate::job_cronjob_yaml::catalog_event(),
    ]
}

/// Build the catalog from loaded packs plus the built-in guards.
///
/// `packs` must already be loaded — this is the same `rule_pack::load_pack`
/// result the engine consumes. Event identity is (`pack`, `id`); a duplicate
/// within one pack is a pack-authoring bug and fails the build loudly rather
/// than silently collapsing two events into one.
pub fn build(packs: &[Pack]) -> Result<Catalog> {
    let mut never: Vec<CatalogEvent> = builtin_events();
    let mut always: Vec<AlwaysEvent> = Vec::new();

    for pack in packs {
        for rule in &pack.guarded_patterns {
            never.push(never_event(&pack.id, rule));
        }
        for pattern in &pack.safe_patterns {
            always.push(always_event(&pack.id, pattern));
        }
    }

    // Deterministic order regardless of pack directory iteration order, so
    // the digest is a property of the policy, not of the filesystem.
    never.sort_by(|a, b| (&a.pack, &a.id).cmp(&(&b.pack, &b.id)));
    always.sort_by(|a, b| (&a.pack, &a.id).cmp(&(&b.pack, &b.id)));

    for window in never.windows(2) {
        if window[0].pack == window[1].pack && window[0].id == window[1].id {
            bail!(
                "duplicate event id '{}' in pack '{}': event ids must be unique within a pack",
                window[0].id,
                window[0].pack
            );
        }
    }
    for window in always.windows(2) {
        if window[0].pack == window[1].pack && window[0].id == window[1].id {
            bail!(
                "duplicate safe-pattern id '{}' in pack '{}': event ids must be unique within a pack",
                window[0].id,
                window[0].pack
            );
        }
    }

    let content = CatalogContent {
        format: CATALOG_FORMAT,
        never: &never,
        always: &always,
    };
    let canonical = serde_json::to_vec(&content).context("canonical catalog serialization")?;

    Ok(Catalog {
        format: CATALOG_FORMAT,
        catalog_digest: sha256_hex(&canonical),
        icg_version: env!("CARGO_PKG_VERSION").to_string(),
        never,
        always,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn empty_pack(id: &str) -> Pack {
        Pack {
            id: id.to_string(),
            tool_keywords: vec![],
            applies_to: vec![],
            safe_patterns: vec![],
            guarded_patterns: vec![],
        }
    }

    fn guarded(id: &str, severity: crate::rule_pack::Severity) -> crate::rule_pack::GuardedPattern {
        crate::rule_pack::GuardedPattern {
            id: id.to_string(),
            enabled: true,
            check: Check::CommandRegex {
                regex: format!("^{id}"),
            },
            tier: crate::rule_pack::Tier::Tier1,
            severity,
            explanation: format!("{id} must never happen"),
            redirect: crate::rule_pack::Redirect {
                channel: crate::rule_pack::Channel::Deny,
                reason_template: format!("do {id} the sanctioned way instead"),
                rewrite_template: None,
            },
            destructive: true,
        }
    }

    #[test]
    fn both_builtin_guards_are_cataloged() {
        let events = builtin_events();
        let ids: Vec<(&str, &str)> = events
            .iter()
            .map(|e| (e.pack.as_str(), e.id.as_str()))
            .collect();
        assert_eq!(
            ids,
            vec![
                ("github-workflows", "github-workflows-protected"),
                ("job-cronjob-yaml", "kind-job-cronjob")
            ]
        );
        for event in &events {
            assert!(event.enabled, "built-ins are code and always enabled");
            assert_eq!(event.action, crate::rule_pack::Channel::Deny);
            assert!(!event.sanctioned_alternative.reason.is_empty());
            assert!(event.sanctioned_alternative.rewrite.is_none());
        }
    }

    #[test]
    fn digest_moves_with_the_event_set_and_not_with_the_binary_version() {
        let base = build(&[empty_pack("p")]).unwrap();
        let same = build(&[empty_pack("p")]).unwrap();
        assert_eq!(base.catalog_digest, same.catalog_digest);

        let mut pack = empty_pack("p");
        pack.guarded_patterns
            .push(guarded("p-never", crate::rule_pack::Severity::Critical));
        let with_rule = build(&[pack]).unwrap();
        assert_ne!(base.catalog_digest, with_rule.catalog_digest);
    }

    #[test]
    fn duplicate_ids_within_a_pack_fail_loudly() {
        let mut pack = empty_pack("p");
        pack.guarded_patterns = vec![
            guarded("same-id", crate::rule_pack::Severity::Critical),
            guarded("same-id", crate::rule_pack::Severity::High),
        ];
        let err = build(&[pack]).expect_err("duplicate ids must fail");
        assert!(err.to_string().contains("duplicate event id"));
    }

    #[test]
    fn events_sort_by_pack_then_id_for_a_stable_digest() {
        let mut first = empty_pack("a");
        first.guarded_patterns = vec![guarded("z-late", crate::rule_pack::Severity::High)];
        let mut second = empty_pack("b");
        second.guarded_patterns = vec![guarded("a-early", crate::rule_pack::Severity::High)];

        let catalog = build(&[first, second]).unwrap();
        let never_packs: Vec<&str> = catalog
            .never
            .iter()
            .filter(|e| e.pack != "github-workflows" && e.pack != "job-cronjob-yaml")
            .map(|e| e.pack.as_str())
            .collect();
        assert_eq!(never_packs, vec!["a", "b"]);
    }
}
