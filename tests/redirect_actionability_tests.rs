//! The redirect-actionability CI gate
//!
//! AGENTS.md rule 2: every guarded rule owes the caller an alternative — a
//! `redirect` whose reason only says "blocked" is an incomplete rule. This
//! suite is the enforcement half of that doctrine:
//!
//! - every shipped pack's guarded rules must pass
//!   [`icg::rule_pack::validate_redirect_actionability`], with a floor on the
//!   guarded-rule count so the sweep cannot silently degenerate to zero;
//! - the negative fixtures under `tests/fixtures/redirect-actionability/`
//!   (empty, whitespace-only, block-only, and danger-without-alternative
//!   reasons) must each be REJECTED, with the error naming the offending
//!   pack and rule;
//! - the positive control fixtures must pass.
//!
//! The gate is deliberately enforced here, in CI, rather than inside
//! `Engine::load_pack`: the engine fails open, so a load-time rejection
//! would silently drop the offending pack at runtime — the worst outcome
//! for a policy defect. A bad redirect must fail loudly at authoring time.

use icg::rule_pack::{
    load_pack, redirect_actionability_violation, validate_redirect_actionability,
    RedirectActionabilityError,
};
use std::path::Path;

const FIXTURES: &str = "tests/fixtures/redirect-actionability";
/// Below this many guarded rules the sweep is not really sweeping; the
/// shipped packs carry 30 today.
const SHIPPED_GUARDED_RULE_FLOOR: usize = 25;

/// Every shipped pack, loaded the way the engine loads them.
fn shipped_packs() -> Vec<icg::rule_pack::Pack> {
    let mut paths: Vec<std::path::PathBuf> = std::fs::read_dir("packs")
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

fn fixture_pack(name: &str) -> icg::rule_pack::Pack {
    let path = Path::new(FIXTURES).join(name);
    load_pack(&path)
        .unwrap_or_else(|error| panic!("fixture {} should load: {error}", path.display()))
}

#[test]
fn every_shipped_guarded_rule_redirects_to_a_concrete_alternative() {
    let mut guarded_rules = 0;
    for pack in shipped_packs() {
        for pattern in &pack.guarded_patterns {
            guarded_rules += 1;
            assert_eq!(
                redirect_actionability_violation(pattern),
                None,
                "shipped rule '{}.{}' fails the actionability gate: {:?} -- {}",
                pack.id,
                pattern.id,
                pattern.redirect.reason_template,
                "every guarded rule owes the caller a concrete sanctioned \
                 alternative (docs/notes/redirect-not-just-block.md)"
            );
        }
        validate_redirect_actionability(&pack)
            .unwrap_or_else(|error| panic!("shipped pack '{}' must validate: {error}", pack.id));
    }
    assert!(
        guarded_rules >= SHIPPED_GUARDED_RULE_FLOOR,
        "the sweep covered only {guarded_rules} guarded rules -- it must never \
         degenerate below {SHIPPED_GUARDED_RULE_FLOOR}"
    );
}

#[test]
fn negative_fixture_empty_reason_is_rejected() {
    let pack = fixture_pack("empty-reason.json");
    assert_eq!(
        redirect_actionability_violation(&pack.guarded_patterns[0]),
        Some(RedirectActionabilityError::EmptyReason)
    );
    let error = validate_redirect_actionability(&pack)
        .expect_err("empty redirect reason must fail the gate");
    assert!(
        error
            .to_string()
            .contains("redirect-actionability-empty-reason.fixture-empty-reason"),
        "error should name pack and rule, got: {error}"
    );
}

#[test]
fn negative_fixture_whitespace_reason_is_rejected() {
    let pack = fixture_pack("whitespace-reason.json");
    assert_eq!(
        redirect_actionability_violation(&pack.guarded_patterns[0]),
        Some(RedirectActionabilityError::EmptyReason),
        "a whitespace-only reason is empty, not actionable"
    );
    assert!(validate_redirect_actionability(&pack).is_err());
}

#[test]
fn negative_fixture_block_only_reason_is_rejected() {
    let pack = fixture_pack("blocked-only.json");
    assert_eq!(
        redirect_actionability_violation(&pack.guarded_patterns[0]),
        Some(RedirectActionabilityError::NonActionable)
    );
    let error =
        validate_redirect_actionability(&pack).expect_err("block-only redirect must fail the gate");
    let message = error.to_string();
    assert!(
        message.contains("redirect-actionability-blocked-only.fixture-blocked-only"),
        "error should name pack and rule, got: {message}"
    );
    assert!(
        message.contains("docs/notes/redirect-not-just-block.md"),
        "error should point the author at the doctrine, got: {message}"
    );
}

#[test]
fn negative_fixture_danger_explanation_without_alternative_is_rejected() {
    // The load-bearing negative fixture: the reason carries a directive verb,
    // but only inside its own prohibition ("Do not run it"). A gate that
    // matched any verb would pass this and be worthless.
    let pack = fixture_pack("danger-no-alternative.json");
    assert_eq!(
        redirect_actionability_violation(&pack.guarded_patterns[0]),
        Some(RedirectActionabilityError::NonActionable)
    );
    assert!(validate_redirect_actionability(&pack).is_err());
}

#[test]
fn positive_control_directive_verb_passes_the_gate() {
    let pack = fixture_pack("actionable-directive-verb.json");
    assert_eq!(
        redirect_actionability_violation(&pack.guarded_patterns[0]),
        None
    );
    validate_redirect_actionability(&pack).expect("positive control must validate");
}

#[test]
fn positive_control_rewrite_template_passes_the_gate() {
    // Mirrors the git pack's force-push rule: an updated_input redirect
    // whose rewrite_template IS the sanctioned alternative.
    let pack = fixture_pack("actionable-via-rewrite.json");
    assert_eq!(
        redirect_actionability_violation(&pack.guarded_patterns[0]),
        None
    );
    validate_redirect_actionability(&pack).expect("positive control must validate");
}
