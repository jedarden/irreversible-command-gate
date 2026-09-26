//! Pins docs/assets/icg-flow.svg's engine-model claims to what the engine
//! and the shipped packs actually do. README embeds the figure as an
//! `<img>`, so it is prose's most-quoted surface while being invisible to
//! every markdown sweep -- the exact shape of the two figure drifts already
//! guarded in tests/documentation_consistency_tests.rs (the latency footer,
//! irrevers-f10cfae4, and the no-network exception, irrevers-a02dc12f).
//! Those guards leave the figure's verdict model unpinned: it renders a
//! four-verdict panel (ALLOW / WARNING / REWRITE / DENY), captions only
//! DENY as stopping the command, states the evaluation order and the
//! fail-open default, and sizes the shipped policy as "N packs, M rules" --
//! none of which anything held to the engine. A channel added to
//! `Channel` (src/rule_pack.rs), or a twelfth pack, would silently
//! falsify the diagram while every rule and pack test stayed green.
//!
//! Both sides of each comparison are derived: the channel universe and the
//! pack/rule counts from `icg coverage --list --format json` (the emitted
//! surface integrators read, not a count scraped off packs/*.json), and the
//! figure's claims from its rendered text. The blocking semantics the
//! panel captions are executed elsewhere -- deny blocks and warn never
//! blocks in src/adapter.rs's adapter tests, and the demo verdict matrix
//! (tests/demo_verdict_regression_tests.rs) runs a command per verdict --
//! so here the figure is held to the emitted policy, and an engine change
//! that grows a verdict fails until the figure moves in the same change.

use serde_json::Value;
use std::{fs, path::PathBuf, process::Command};

/// The checkout under audit: the demo claims are about what *ships* in the
/// repo, not whatever pack set is installed at /etc/icg/packs on the host
/// that happens to run the suite.
fn audited_checkout() -> PathBuf {
    if let Ok(cwd) = std::env::current_dir() {
        if cwd.join("Cargo.toml").exists() {
            return cwd;
        }
    }
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn repo_relative(relative: &str) -> String {
    let path = audited_checkout().join(relative);
    fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("should read {}: {error}", path.display()))
}

/// The figure's human-visible text: every tag replaced by a space, then
/// whitespace collapsed. Plain `split_whitespace` is not enough here --
/// sentences are split across `<text>` elements, so the tags must come out
/// before the needles can span them.
fn svg_text(svg: &str) -> String {
    let mut visible = String::with_capacity(svg.len());
    let mut in_tag = false;
    for character in svg.chars() {
        match character {
            '<' => {
                in_tag = true;
                visible.push(' ');
            }
            '>' => in_tag = false,
            _ if !in_tag => visible.push(character),
            _ => {}
        }
    }
    visible.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// The contents of an SVG's `<desc>` element -- the alt text the file
/// itself carries for non-visual readers.
fn svg_desc(svg: &str) -> String {
    let (_, rest) = svg
        .split_once("<desc")
        .expect("icg-flow.svg should carry a <desc> element as its alt text");
    let (_, body) = rest.split_once('>').expect("the <desc> should open");
    let (body, _) = body.split_once("</desc>").expect("the <desc> should close");
    svg_text(body)
}

/// README's alt attribute for the flow figure -- the other half of the alt
/// text a non-visual reader gets.
fn readme_flow_alt() -> String {
    let readme = repo_relative("README.md");
    let Some((_, after_src)) = readme.split_once("icg-flow.svg") else {
        panic!("README.md should keep embedding docs/assets/icg-flow.svg");
    };
    let Some((_, alt)) = after_src.split_once("alt=\"") else {
        panic!("README's icg-flow.svg <img> should carry alt text");
    };
    alt.split('"')
        .next()
        .expect("alt text should close its quote")
        .to_owned()
}

/// coverage/v1's channel spelling -> the verdict chip the figure renders
/// for it. `Allow` is deliberately absent: it is the no-match and
/// safe-pattern default, not a `Channel` variant a guarded pattern can
/// name -- the figure's fourth chip.
const CHANNEL_CHIPS: [(&str, &str); 3] = [
    ("Deny", "DENY"),
    ("UpdatedInput", "REWRITE"),
    ("AdditionalContext", "WARNING"),
];

/// `icg coverage --list --format json` over the shipped packs: the emitted
/// policy surface the figure's claims are held to.
fn coverage_report() -> Value {
    let packs_dir = audited_checkout().join("packs");
    let output = Command::new(env!("CARGO_BIN_EXE_icg"))
        .args([
            "coverage",
            "--list",
            "--format",
            "json",
            "--pack",
            packs_dir.to_str().expect("packs path should be UTF-8"),
        ])
        .output()
        .expect("icg coverage --list should run");
    assert!(
        output.status.success(),
        "coverage --list --format json should succeed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let report: Value = serde_json::from_slice(&output.stdout)
        .expect("coverage --format json should emit valid JSON");
    assert_eq!(report["format"], "coverage/v1");
    let unreadable = report["unreadable"]
        .as_array()
        .expect("coverage/v1 should carry an unreadable array");
    assert!(
        unreadable.is_empty(),
        "every shipped pack should load; a pack that fails to load makes \
         the figure's policy chip wrong for a different reason than drift \
         -- fix the pack first: {unreadable:?}"
    );
    report
}

/// The policy size the figure states as "<N> packs, <M> rules".
fn stated_pack_and_rule_counts(text: &str) -> (u64, u64) {
    let (pack_count, rule_count) = text
        .split_once(" packs, ")
        .and_then(|(before, rest)| {
            let digits: String = before
                .chars()
                .rev()
                .take_while(|character| character.is_ascii_digit())
                .collect();
            let digits: String = digits.chars().rev().collect();
            let pack_count: u64 = digits.parse().ok()?;
            let digits_end = rest.find(|character: char| !character.is_ascii_digit())?;
            let rule_count: u64 = rest[..digits_end].parse().ok()?;
            rest[digits_end..]
                .starts_with(" rules")
                .then_some((pack_count, rule_count))
        })
        .unwrap_or_else(|| {
            panic!(
                "icg-flow.svg should keep stating the shipped policy's size \
                 as '<N> packs, <M> rules'; the chip is the reader's summary \
                 of what ships, and this test derives both numbers from \
                 coverage/v1"
            )
        });
    (pack_count, rule_count)
}

/// The figure's verdict-model claims must hold against the emitted policy:
/// every channel a shipped guarded pattern can name is one of the three
/// verdict chips the panel renders, each chip still carries the caption
/// that tells the reader whether it stops the command, and the panel's
/// ordering, fail-open and attribution sentences stay.
#[test]
fn flow_figure_verdict_claims_match_the_engine() {
    let figure = repo_relative("docs/assets/icg-flow.svg");
    let text = svg_text(&figure);

    // The panel's caption pairings are the "only deny stops the command"
    // claim made per verdict: three chips describe the command going ahead,
    // one describes it being blocked. Pin them exactly -- a rewording moves
    // these needles in the same change, so a caption cannot quietly drift
    // into promising (or threatening) the wrong outcome.
    for (needle, why) in [
        (
            "ALLOW runs, silently",
            "the allow verdict must stay captioned as the command going \
             ahead unmodified",
        ),
        (
            "WARNING runs, with a caution",
            "the warning verdict must stay captioned as the command going \
             ahead -- never as a block (src/adapter.rs: a warning never \
             blocks)",
        ),
        (
            "REWRITE safe form substituted",
            "the rewrite verdict must stay captioned as substitution, the \
             one verdict that changes the command",
        ),
        (
            "DENY blocked + what to do",
            "the deny verdict must stay the only chip captioned as \
             blocking, and it must keep promising the alternative",
        ),
    ] {
        assert!(
            text.contains(needle),
            "icg-flow.svg's verdict panel dropped {needle:?} -- {why}; the \
             panel and this test move together"
        );
    }

    // Each chip rendered as a chip, not folded into prose.
    for (_, chip) in CHANNEL_CHIPS {
        assert!(
            figure.contains(&format!(">{chip}<")),
            "icg-flow.svg should keep rendering {chip} as its own verdict \
             chip; the panel is the figure's summary of the four-verdict \
             model"
        );
    }

    // The panel's summary sentences, and the claim each makes about the
    // engine: deny alone stops, and every verdict attributes to a pack and
    // rule (the executed truth-side of that attribution is
    // sampled_verdicts_attribute_to_their_catalog_entries in
    // tests/catalog_export_tests.rs).
    for needle in [
        "Only DENY stops the command.",
        "Every verdict names the pack and rule.",
    ] {
        assert!(
            text.contains(needle),
            "icg-flow.svg's verdict panel should keep stating {needle:?}"
        );
    }

    // The evaluation order and the fail-open default, as the figure draws
    // them: safe patterns first (a match ends it), then guarded patterns
    // (first match wins and picks the channel), and allow when nothing
    // decides otherwise. The truth-side of the ordering is pinned on the
    // evaluation figure and its --debug trace in
    // tests/documentation_consistency_tests.rs.
    for needle in [
        "safe patterns first",
        "a match ends it — allow",
        "first match wins; its rule picks the channel below",
        "no pack loaded, no match, or a crash → allow",
    ] {
        assert!(
            text.contains(needle),
            "icg-flow.svg should keep stating the evaluation claim \
             {needle:?} -- the figure is where a reader learns the engine's \
             order of operations and its fail-open default"
        );
    }

    // The verdict model the panel shows must cover every channel the
    // shipped policy actually uses: derive the channel universe from
    // coverage/v1 and hold it inside the figure's model, so an engine
    // change that grows a verdict cannot ship beside a figure that still
    // shows four.
    let mut rendered_chips = std::collections::BTreeSet::new();
    for pack in coverage_report()["packs"]
        .as_array()
        .expect("coverage/v1 should carry a packs array")
    {
        let pack_id = pack["id"].as_str().expect("pack should carry an id");
        for rule in pack["guarded_patterns"]
            .as_array()
            .expect("pack should carry guarded_patterns")
        {
            let rule_id = rule["id"].as_str().expect("rule should carry an id");
            let channel = rule["channel"]
                .as_str()
                .expect("rule should carry a channel");
            let Some((_, chip)) = CHANNEL_CHIPS.iter().find(|(name, _)| *name == channel) else {
                panic!(
                    "coverage/v1 reports channel {channel:?} on \
                     {pack_id}/{rule_id}, which is not one of the three \
                     verdict channels icg-flow.svg's panel renders -- the \
                     engine grew a verdict the architecture figure does not \
                     show. Render it (and map it in CHANNEL_CHIPS) in the \
                     same change as the engine work"
                );
            };
            rendered_chips.insert(*chip);
        }
    }
    for chip in rendered_chips {
        assert!(
            figure.contains(&format!(">{chip}<")),
            "the shipped policy uses the {chip} verdict but icg-flow.svg no \
             longer renders its chip"
        );
    }

    // The figure's own alt text and README's alt attribute for the same
    // <img> are what a non-visual reader gets; both must carry the
    // four-verdict model and the only-deny-stops claim, not just the
    // rendered shapes. (The network exception those texts also state is
    // held by flow_diagram_states_the_network_exception_with_its_claim in
    // tests/documentation_consistency_tests.rs.)
    for (surface, body) in [
        ("icg-flow.svg's <desc>", svg_desc(&figure)),
        ("README's alt text", readme_flow_alt()),
    ] {
        for needle in [
            "allow, warning, rewrite, or deny",
            "Only deny stops the command.",
        ] {
            assert!(
                body.contains(needle),
                "{surface} should keep stating the verdict model \
                 {needle:?} -- the alt text is the claim a screen reader \
                 gets, and it must match the panel the figure renders"
            );
        }
    }
}

/// The figure's policy chip ("<N> packs, <M> rules") must state what
/// coverage/v1 actually loads: the pack count and the guarded-rule count
/// derived from the emitted report, not from a directory scan. A twelfth
/// pack or a thirtieth guarded rule must move the figure in the same
/// change -- the same update-together rule the demo GIF's verdict matrix
/// imposes, and the same drift class as the kubectl pack landing while the
/// README still said Ten rule packs.
#[test]
fn flow_figure_pack_chip_matches_the_shipped_policy() {
    let report = coverage_report();
    let packs = report["packs"]
        .as_array()
        .expect("coverage/v1 should carry a packs array");
    let loaded_packs = packs.len() as u64;
    let guarded_rules: u64 = packs
        .iter()
        .map(|pack| {
            pack["guarded_patterns"]
                .as_array()
                .expect("pack should carry guarded_patterns")
                .len() as u64
        })
        .sum();

    let text = svg_text(&repo_relative("docs/assets/icg-flow.svg"));
    let (stated_packs, stated_rules) = stated_pack_and_rule_counts(&text);
    assert_eq!(
        (stated_packs, stated_rules),
        (loaded_packs, guarded_rules),
        "icg-flow.svg states {stated_packs} packs / {stated_rules} rules \
         but coverage/v1 loads {loaded_packs} packs / {guarded_rules} \
         guarded rules -- the figure's policy chip and packs/ move \
         together: add the pack or rule and update the figure in the same \
         change"
    );
}
