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
//!
//! Two narration surfaces the first sweep left unpinned are held here too:
//! the engine box's worked dispatch example ("bao|vault → openbao pack") is
//! held to the shipped pack's `tool_keywords`, and README's verdict table
//! -- the "Hook response" column an integrator quotes -- is held to the
//! wire `icg hook` really emits per verdict and to the panel's chips.

use serde_json::Value;
use std::{
    fs,
    io::Write as _,
    path::PathBuf,
    process::{Command, Stdio},
};

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
    assert_eq!(report["format"], "coverage/v2");
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

/// The engine box teaches dispatch with a worked example -- it renders
/// "bao|vault → openbao pack" under "dispatch by tool keyword" -- and both
/// alt surfaces state the mechanism ("dispatches to a rule pack by tool
/// keyword"). That example is a factual claim about packs/: the engine
/// builds its dispatch index from each pack's `tool_keywords`
/// (`pack_dispatch_keywords` / `keyword_index` in src/engine.rs), so the
/// named pack must ship and its keywords must render exactly as drawn. A
/// keyword added, removed or reordered in the pack would silently falsify
/// the worked example while every pack and coverage test stayed green --
/// the drift class the evaluation figure's trace pin
/// (tests/documentation_consistency_tests.rs, irrevers-48f6d7c2) closed
/// for the walkthrough; that test also executes the mechanism itself (a
/// keyword command reaches exactly the pack claiming it). The same box and
/// README's alt text locate the policy -- root-owned packs under
/// /etc/icg/packs -- and that claim is pinned as needles here; the truth
/// side (the installer really does create that directory root-owned) is
/// install_script_tests::install_script_installs_root_owned.
#[test]
fn flow_figure_dispatch_example_matches_the_shipped_packs() {
    let figure = repo_relative("docs/assets/icg-flow.svg");
    let text = svg_text(&figure);

    // Every "<keywords> → <id> pack" example the figure renders must name
    // a shipped pack whose tool_keywords render exactly that list.
    // Scanning instead of hardcoding the one example keeps a second worked
    // example honest for free; the fail-open line ("a crash → allow") has
    // no "pack" after its arrow and is skipped.
    let tokens: Vec<&str> = text.split(' ').collect();
    let mut examples = 0usize;
    for i in 1..tokens.len().saturating_sub(2) {
        if tokens[i] != "→" || !tokens[i - 1].contains('|') || tokens[i + 2] != "pack" {
            continue;
        }
        examples += 1;
        let rendered_keywords: Vec<String> = tokens[i - 1].split('|').map(str::to_owned).collect();
        let pack_id = tokens[i + 1];
        let pack: Value = serde_json::from_str(&repo_relative(&format!("packs/{pack_id}.json")))
            .unwrap_or_else(|error| {
                panic!(
                    "the figure's dispatch example names {pack_id}, which must \
                 ship as packs/{pack_id}.json: {error}"
                )
            });
        let keywords: Vec<String> = pack["tool_keywords"]
            .as_array()
            .unwrap_or_else(|| {
                panic!(
                    "packs/{pack_id}.json should carry tool_keywords -- the \
                        dispatch index is built from them"
                )
            })
            .iter()
            .map(|value| {
                value
                    .as_str()
                    .expect("keyword should be a string")
                    .to_owned()
            })
            .collect();
        assert!(
            !keywords.is_empty(),
            "packs/{pack_id}.json declares no tool_keywords, so nothing \
             dispatches to it -- the figure's example is about a mechanism \
             that no longer reaches it"
        );
        assert_eq!(
            keywords, rendered_keywords,
            "icg-flow.svg's dispatch example disagrees with \
             packs/{pack_id}.json's tool_keywords -- the figure's worked \
             example and the pack move together"
        );
    }
    assert!(
        examples > 0,
        "the dispatch-example scan found no \"keywords → id pack\" example \
         in icg-flow.svg; the parser has probably rotted"
    );

    // The mechanism sentence, on both alt surfaces a non-visual reader
    // gets: the figure's own <desc> and README's alt attribute for the
    // same <img>.
    for (surface, body) in [
        ("icg-flow.svg's <desc>", svg_desc(&figure)),
        ("README's alt text", readme_flow_alt()),
    ] {
        assert!(
            body.contains("dispatches to a rule pack by tool keyword"),
            "{surface} should keep stating the dispatch mechanism -- it is \
             the sentence the engine box's worked example illustrates"
        );
    }

    // The title names what the figure actually shows: one evaluation, end
    // to end.
    assert!(
        figure.contains("evaluates a tool call"),
        "icg-flow.svg's <title> should keep naming the evaluation it draws"
    );

    // Where the policy lives, per the same box and README's alt text: the
    // deployed pack directory, root-owned. (The installer's root-owned
    // creation of it is pinned in install_script_tests.)
    assert!(
        text.contains("/etc/icg/packs/")
            && text.contains("root-owned policy the guarded agent cannot rewrite"),
        "icg-flow.svg's packs box should keep stating the deployed policy \
         location and its root ownership"
    );
    assert!(
        readme_flow_alt().contains("root-owned in /etc/icg/packs"),
        "README's alt text for icg-flow.svg should keep stating that rule \
         packs live root-owned in /etc/icg/packs"
    );
}

/// One `icg hook` run against the shipped packs, via the same stdin JSON a
/// harness sends. The denial-log sink keeps the deny probe off any
/// instrumented host log, the convention check_demo_command uses.
fn hook_response(command: &str) -> Value {
    let sink = tempfile::tempdir().expect("denial-log sink directory should create");
    let mut child = Command::new(env!("CARGO_BIN_EXE_icg"))
        .args(["hook", "--rule-pack"])
        .arg(audited_checkout().join("packs"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .env("ICG_DENIAL_LOG", sink.path().join("denials.jsonl"))
        .spawn()
        .expect("icg hook should start");
    child
        .stdin
        .take()
        .expect("hook stdin should be available")
        .write_all(
            serde_json::json!({
                "tool_name": "Bash",
                "tool_input": { "command": command },
            })
            .to_string()
            .as_bytes(),
        )
        .expect("hook input should be written");
    let output = child
        .wait_with_output()
        .expect("hook process should finish");
    assert!(
        output.status.success(),
        "icg hook should exit 0 for {command:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).expect("hook stdout should be one JSON object")
}

/// README's verdict table -- the "Hook response" column an integrator
/// quotes to wire the gate into a harness -- held to the wire `icg hook`
/// actually emits for each verdict and to the panel icg-flow.svg renders.
/// The emissions are pinned per scenario with private packs in
/// hook_response_output_tests, and the panel's channels are pinned to
/// coverage/v1 above, but nothing held the README's four rows to any real
/// emission or to the figure: a channel rename or an output-shape change
/// could falsify the front-page table while both of those suites stayed
/// green. Each row's probe is one of the demo matrix's command-mode inputs
/// (demo_verdict_regression_tests pins their verdicts against the same
/// packs), so a pack edit that flips a verdict fails both suites in the
/// same change.
#[test]
fn readme_verdict_table_matches_the_hook_wire_and_the_figure() {
    // (verdict, hook-response cell with its backticks stripped, probe
    // command) -- the cells are pinned exactly, so a rewording moves this
    // table in the same change as the README.
    const ROWS: [(&str, &str, &str); 4] = [
        ("ALLOW", "permissionDecision: allow", "git status"),
        (
            "WARNING",
            "allow + additionalContext",
            "bao kv get -field=token secret/app/db",
        ),
        (
            "REWRITE",
            "allow + updatedInput",
            "git push --force origin main",
        ),
        (
            "DENY",
            "permissionDecision: deny",
            "bao kv destroy secret/app/db",
        ),
    ];

    // 1. Parse the table: the header, then the four data rows in order.
    let readme = repo_relative("README.md");
    let header = readme
        .find("| Verdict | Hook response | When |")
        .expect("README should keep its verdict table header");
    let mut parsed: Vec<(String, String)> = Vec::new();
    for line in readme[header..].lines().skip(1) {
        let line = line.trim();
        if !line.starts_with('|') {
            break;
        }
        if line.starts_with("| ---") {
            continue;
        }
        let cells: Vec<&str> = line.split('|').map(str::trim).collect();
        assert!(
            cells.len() >= 4,
            "malformed verdict-table row {line:?}: expected the \
             |verdict|hook response|when| shape"
        );
        let unbacktick = |cell: &str| cell.replace('`', "");
        parsed.push((unbacktick(cells[1]), unbacktick(cells[2])));
    }
    let expected: Vec<(String, String)> = ROWS
        .iter()
        .map(|(verdict, cell, _)| ((*verdict).to_owned(), (*cell).to_owned()))
        .collect();
    assert_eq!(
        parsed, expected,
        "README's verdict table drifted from the four documented rows -- \
         the table, the engine's channels and this test move together"
    );

    // 2. Each row's cell held to what `icg hook` really emits for that
    //    verdict. The wire is the same hookSpecificOutput shape
    //    hook_response_output_tests pins with private packs; here the
    //    shipped packs produce it.
    for (verdict, cell, command) in ROWS {
        let output = hook_response(command);
        let wire = &output["hookSpecificOutput"];
        match verdict {
            "ALLOW" => {
                assert_eq!(
                    wire["permissionDecision"], "allow",
                    "the {verdict} probe {command:?} must grant the command"
                );
                assert!(
                    wire.get("additionalContext").is_none(),
                    "ALLOW is captioned as running silently, and its table \
                     row promises a bare permissionDecision -- a caution \
                     smuggled into the allow probe {command:?} falsifies \
                     both: {wire}"
                );
                assert!(
                    wire.get("updatedInput").is_none(),
                    "the ALLOW probe {command:?} must not rewrite: {wire}"
                );
            }
            "WARNING" => {
                assert_eq!(
                    wire["permissionDecision"], "allow",
                    "a warning never blocks -- the {verdict} probe \
                     {command:?} must still grant the command"
                );
                assert!(
                    wire["additionalContext"]
                        .as_str()
                        .is_some_and(|context| !context.is_empty()),
                    "the {verdict} row promises `allow` + `additionalContext` \
                     but the probe {command:?} carried no context: {wire}"
                );
                assert!(
                    wire.get("updatedInput").is_none(),
                    "the {verdict} row promises a caution, not a \
                     substitution -- the probe {command:?} must not rewrite: \
                     {wire}"
                );
            }
            "REWRITE" => {
                assert_eq!(
                    wire["permissionDecision"], "allow",
                    "a rewrite grants the command via updatedInput -- the \
                     {verdict} probe {command:?} must not deny"
                );
                assert!(
                    wire["updatedInput"].as_object().is_some(),
                    "the {verdict} row promises `allow` + `updatedInput` but \
                     the probe {command:?} carried no rewrite: {wire}"
                );
            }
            "DENY" => {
                assert_eq!(
                    wire["permissionDecision"], "deny",
                    "the {verdict} probe {command:?} must block"
                );
                assert!(
                    wire["permissionDecisionReason"]
                        .as_str()
                        .is_some_and(|reason| !reason.is_empty()),
                    "the {verdict} row's When column promises the reason \
                     carries the alternative -- the probe {command:?} must \
                     deny with a reason: {wire}"
                );
            }
            other => panic!("unexpected verdict row {other:?}"),
        }
        // Whatever the cell names must exist on the wire.
        for key in ["permissionDecision", "additionalContext", "updatedInput"] {
            if cell.contains(key) {
                assert!(
                    wire.get(key).is_some(),
                    "the {verdict} row names {key} but the wire carries none: \
                     {wire}"
                );
            }
        }
    }

    // 3. The figure side: every verdict the table rows spell renders as its
    //    own chip in the panel (ALLOW included -- it is the no-match and
    //    safe-pattern default, so no Channel names it and the coverage-derived
    //    sweep above never checks its chip), and README's framing sentence
    //    keeps saying what the panel's caption says: only deny stops.
    let figure = repo_relative("docs/assets/icg-flow.svg");
    for (verdict, _, _) in ROWS {
        assert!(
            figure.contains(&format!(">{verdict}<")),
            "README's table documents the {verdict} verdict but icg-flow.svg \
             no longer renders it as a chip -- the table and the panel are \
             the same four-verdict model"
        );
    }
    let collapsed = svg_text(&readme);
    assert!(
        collapsed.contains("only `deny` stops the command"),
        "README should keep framing the table with the panel's central \
         claim -- only deny stops the command"
    );
}
