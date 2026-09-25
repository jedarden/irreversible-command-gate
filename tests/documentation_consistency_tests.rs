//! Documentation/CLI consistency assertions.
//!
//! The 2026-08-25 plan/artifact audit found systematic drift between the
//! docs and the shipped surface: phase checkboxes left open after their
//! beads closed, `bf` described as canonical after the bead-rs cutover,
//! troubleshooting claiming the PATH-wrapper was unimplemented, and the
//! `icg install` help naming a default directory the code no longer uses.
//! These tests pin the reconciled state so each class of drift fails a
//! build instead of silently recurring.
//!
//! The 2026-09 reconciliations extended that to plan.md's claims about
//! things outside the source tree -- shipped tags/releases and the open/
//! closed state of the beads the plan names. Those checks live at the end
//! of this file and read the committed ground truth those reconciliations
//! cite: `docs/notes/shipped-releases-inventory.md` and the git-tracked
//! `.beads/checkpoint/` snapshot.

use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Every document an operator or pack author is pointed at from the README,
/// the docs index, or the onboarding path.
///
/// The 2026-08-25 audit reconciled only `quick-start.md` and wrote its guards
/// to match, so the same fictions survived untouched in the docs a newcomer
/// actually reads first. The guards take the whole set now.
const OPERATOR_FACING_DOCS: [&str; 10] = [
    "docs/quick-start.md",
    "docs/onboarding-guide.md",
    "docs/examples/README.md",
    "docs/operators/README.md",
    "docs/operators/training-manual.md",
    "docs/operators/deny-messages.md",
    "docs/operators/deployment-guide.md",
    "docs/operators/troubleshooting.md",
    "docs/operators/migration-from-org-rule-guard.md",
    "docs/developers/README.md",
];

/// The checkout this run audits.
///
/// `env!("CARGO_MANIFEST_DIR")` is baked in at compile time, and this box's
/// global cargo config (`~/.cargo/config.toml`) points `target-dir` at the
/// shared `/build/target-workers`, so cargo reuses a test binary built by a
/// *different* checkout of this repo whenever its fingerprint looks fresh.
/// The baked path then names some other tree -- a deleted gate extraction or
/// a trial replay of an old commit -- and every doc/checkpoint read below
/// silently audits *that* tree. Seen live 2026-09-17: a reused binary read a
/// pre-reconciliation plan.md and failed its own guard. cargo runs test
/// binaries with the package root as the working directory, so prefer the
/// runtime cwd; fall back to the baked path only when it does not name a
/// checkout (the binary invoked by hand from an unrelated directory).
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

fn install_help() -> String {
    let output = Command::new(env!("CARGO_BIN_EXE_icg"))
        .args(["install", "--help"])
        .output()
        .expect("icg install --help should run");
    assert!(output.status.success(), "icg install --help should succeed");
    String::from_utf8_lossy(&output.stdout).into_owned()
}

#[test]
fn install_help_names_the_real_default_directory() {
    let help = install_help();

    assert!(
        help.contains(icg::documented_commands::DEFAULT_WRAPPER_INSTALL_DIR),
        "install help should name the actual default directory {}: {help}",
        icg::documented_commands::DEFAULT_WRAPPER_INSTALL_DIR
    );
    assert!(
        !help.contains("~/.local/bin"),
        "install help must not claim the pre-2026-08-22 user-writable \
         ~/.local/bin default (irrevers-6716fd2b moved it): {help}"
    );
}

#[test]
fn install_default_directory_is_root_owned_not_user_writable() {
    // The security-relevant half of the same claim: run_install's default
    // must stay a root-owned system path so the guarded agent cannot
    // replace the wrapper with its own binary.
    assert_eq!(
        icg::documented_commands::DEFAULT_WRAPPER_INSTALL_DIR,
        "/usr/local/bin"
    );
}

#[test]
fn troubleshooting_does_not_claim_the_wrapper_is_unimplemented() {
    let doc = repo_relative("docs/operators/troubleshooting.md");

    assert!(
        !doc.contains("subcommand is not implemented"),
        "troubleshooting.md must not claim the PATH-wrapper is unimplemented: \
         argv[0] dispatch shipped (irrevers-94eb1300, wrapper_deny_tests.rs)"
    );
    assert!(
        doc.contains("PATH wrapper and absolute paths"),
        "troubleshooting.md should keep its section describing the shipped \
         wrapper behavior"
    );
}

#[test]
fn troubleshooting_direct_tests_target_the_production_pack_directory() {
    let doc = repo_relative("docs/operators/troubleshooting.md");

    assert!(
        doc.contains("/etc/icg/packs"),
        "troubleshooting.md should reference the modular production pack \
         directory (the hook's directory-first default since 34bed52)"
    );
    assert!(
        !doc.contains("--rule-pack /etc/icg/rule-pack.json"),
        "troubleshooting.md direct-test examples should not point at the \
         legacy single-file artifact as the primary path"
    );
}

#[test]
fn troubleshooting_updater_sections_match_the_archive_deploying_updater() {
    let doc = repo_relative("docs/operators/troubleshooting.md");

    // `icg update` deploys the modular archive (409ca42): exact asset name,
    // --pack-dir activation directory, packs.previous rollback sibling.
    assert!(
        doc.contains("icg-packs.tar.gz"),
        "troubleshooting.md should name the updater's exact required asset"
    );
    assert!(
        doc.contains("packs.previous"),
        "troubleshooting.md rollback guidance should use the updater's \
         packs.previous sibling, not the legacy single-file restore"
    );
}

#[test]
fn plan_phase_checkboxes_reflect_closed_phase_beads() {
    let plan = repo_relative("docs/plan/plan.md");

    let open_phases: Vec<&str> = plan
        .lines()
        .filter(|line| line.starts_with("- [ ] **Phase"))
        .collect();
    assert!(
        open_phases.is_empty(),
        "plan.md still marks phases incomplete whose beads are closed: {open_phases:?}"
    );

    for phase in 0..=5 {
        assert!(
            plan.contains(&format!("- [x] **Phase {phase} ")),
            "plan.md should mark Phase {phase} complete with its \
             reconciliation note"
        );
    }
}

#[test]
fn plan_does_not_describe_bf_as_the_canonical_bead_cli() {
    let plan = repo_relative("docs/plan/plan.md");

    // The bf -> bead-rs cutover happened 2026-08-14; these exact stale
    // phrasings are what the audit found.
    for stale in [
        "`bf` (bead-forge) is currently canonical",
        "`bf` is currently canonical and `br` is deprecated",
        "`bf`'s `sync --flush-only` flag today",
        "Deny `bf sync --flush-only`",
        "Deny `bf doctor --repair`",
    ] {
        assert!(
            !plan.contains(stale),
            "plan.md still contains pre-cutover phrasing {stale:?}"
        );
    }

    assert!(
        plan.contains("Deny `bead sync flush-only`"),
        "plan.md Phase 2 should state the shipped rule in bead-rs syntax"
    );
}

#[test]
fn misc_pack_data_matches_the_documented_post_cutover_state() {
    let pack: serde_json::Value =
        serde_json::from_str(&repo_relative("packs/misc.json")).expect("misc pack parses");
    let rule = pack["guarded_patterns"]
        .as_array()
        .expect("guarded_patterns array")
        .iter()
        .find(|pattern| pattern["id"] == "deprecated-bead-cli")
        .expect("deprecated-bead-cli rule present");
    // The shipped manifest flattens check fields into the pattern object
    // (type/predicate_name/data as siblings of id), not under a "check" key.
    let data = &rule["data"];

    assert_eq!(data["currently_canonical"], "bead");
    assert_eq!(data["deprecated"], serde_json::json!(["bf", "br"]));
}

fn quick_start() -> String {
    repo_relative("docs/quick-start.md")
}

#[test]
fn quick_start_describes_the_kubectl_pack_as_it_ships() {
    let doc = quick_start();

    // The 2026-08-25 audit found these fictional kubectl claims in the wild.
    // The real pack (ADR-001, 2026-09-19) uses different ids, so they must
    // stay absent.
    for stale in ["kubectl delete pvc", "kubectl-delete-pvc", "**Kubernetes**"] {
        assert!(
            !doc.contains(stale),
            "quick-start.md still cites the fictional kubectl claim {stale:?}"
        );
    }

    // The pack is blanket; ArgoCD-aware narrowing is still not attempted,
    // and operators need to know the org hook denies the same commands
    // during coexistence.
    assert!(
        doc.contains("ArgoCD-aware kubectl scoping"),
        "quick-start.md should say the kubectl pack is blanket, not ArgoCD-aware"
    );
    assert!(
        doc.contains("org-rule-guard.py"),
        "quick-start.md should name org-rule-guard.py as the coexisting hook"
    );
}

#[test]
fn quick_start_pack_inventory_matches_the_shipped_packs() {
    let doc = quick_start();

    let packs_dir = audited_checkout().join("packs");
    let mut shipped_ids: Vec<String> = fs::read_dir(&packs_dir)
        .unwrap_or_else(|error| panic!("should read {}: {error}", packs_dir.display()))
        .filter_map(|entry| {
            let path = entry.expect("directory entry").path();
            if path.extension().and_then(|ext| ext.to_str()) == Some("json") {
                Some(path.file_stem()?.to_string_lossy().into_owned())
            } else {
                None
            }
        })
        .collect();
    assert!(
        !shipped_ids.is_empty(),
        "packs/ directory should not be empty"
    );
    shipped_ids.sort();

    for id in &shipped_ids {
        assert!(
            doc.contains(&format!("`{id}`")),
            "quick-start.md should list the shipped pack `{id}` in its \
             coverage inventory"
        );
    }

    // Fictional inventories the audit found: a "vault" pack id and pattern
    // counts that never existed. The OpenBao pack's id is `openbao`.
    for stale in [
        "vault (",
        "Pack: vault",
        "vault-kv-destroy",
        "(12 patterns)",
    ] {
        assert!(
            !doc.contains(stale),
            "quick-start.md still cites the fictional inventory {stale:?}"
        );
    }
}

#[test]
fn quick_start_documents_the_real_claude_code_hook_contract() {
    let doc = quick_start();

    // The documented contract (deployment-guide.md) is a matcher-array
    // PreToolUse hook in ~/.claude/settings.json invoking `icg hook` by
    // absolute path. The old `~/.config/claude-code` command/args object is
    // not a shape any harness reads.
    assert!(
        doc.contains("~/.claude/settings.json"),
        "quick-start.md should configure the hook in ~/.claude/settings.json"
    );
    assert!(
        doc.contains("\"matcher\": \"Bash|Write|Edit\""),
        "quick-start.md should use the matcher-array hook shape"
    );
    assert!(
        doc.contains("/usr/local/bin/icg hook"),
        "quick-start.md should invoke the hook by absolute path"
    );
    for stale in ["~/.config/claude-code", "\"args\": [\"hook\"]"] {
        assert!(
            !doc.contains(stale),
            "quick-start.md still shows the bogus {stale:?} hook shape"
        );
    }
}

#[test]
fn quick_start_is_a_single_coherent_guide() {
    let doc = quick_start();

    // The pre-rewrite file carried both a "Version 2.0" and a "Version 1.0"
    // half with duplicated Common Tasks / Quick Reference / Support
    // sections. One doc, one footer.
    assert_eq!(
        doc.matches("Quick Start Guide Version").count(),
        1,
        "quick-start.md should have exactly one version footer, not the \
         duplicated v1/v2 halves"
    );
    for heading in ["## Common Tasks", "## Quick Reference", "## Support"] {
        assert_eq!(
            doc.matches(heading).count(),
            1,
            "quick-start.md should contain {heading} exactly once"
        );
    }
}

/// Every guarded pattern that ships must appear by id in quick-start's
/// coverage table, and the table's per-pack count must match the pack file.
///
/// The weaker `quick_start_pack_inventory_matches_the_shipped_packs` check
/// only asserts pack *ids*, which is how `git-credential-fill-bare-stdout`
/// shipped (2026-08-27) as a Critical rule that no operator-facing doc
/// mentioned, while the table still claimed the git pack had three patterns.
#[test]
fn quick_start_coverage_table_matches_every_shipped_pattern() {
    let doc = quick_start();
    let packs_dir = audited_checkout().join("packs");

    let mut entries: Vec<_> = fs::read_dir(&packs_dir)
        .unwrap_or_else(|error| panic!("should read {}: {error}", packs_dir.display()))
        .map(|entry| entry.expect("directory entry").path())
        .filter(|path| path.extension().and_then(|ext| ext.to_str()) == Some("json"))
        .collect();
    entries.sort();
    assert!(!entries.is_empty(), "packs/ directory should not be empty");

    for path in entries {
        let pack: serde_json::Value = serde_json::from_str(&fs::read_to_string(&path).unwrap())
            .unwrap_or_else(|error| panic!("{} should be valid JSON: {error}", path.display()));
        let pack_id = pack["id"].as_str().expect("pack should carry an id");
        let patterns = pack["guarded_patterns"]
            .as_array()
            .expect("pack should carry guarded_patterns");

        for pattern in patterns {
            let pattern_id = pattern["id"].as_str().expect("pattern should carry an id");
            assert!(
                doc.contains(pattern_id),
                "quick-start.md's coverage table omits the shipped pattern \
                 `{pattern_id}` from pack `{pack_id}` -- a rule nobody can \
                 read about is a rule operators will report as a false positive"
            );
        }

        // The table's count column and the `icg coverage --list` transcript
        // both state a per-pack number; neither may drift from the pack file.
        let count = patterns.len();
        assert!(
            doc.contains(&format!("| `{pack_id}` | {count} |")),
            "quick-start.md's coverage table should say pack `{pack_id}` has \
             {count} patterns"
        );
        assert!(
            doc.contains(&format!("pack {pack_id} ({count} patterns)")),
            "quick-start.md's `icg coverage --list` transcript should show \
             `pack {pack_id} ({count} patterns)`"
        );
    }
}

/// Pack counts are spelled out in doc prose ("Eleven rule packs"); extend
/// this as the fleet grows. A count with no arm fails loudly instead of
/// letting a doc drift silently past it.
fn count_word(count: usize) -> &'static str {
    match count {
        10 => "Ten",
        11 => "Eleven",
        12 => "Twelve",
        13 => "Thirteen",
        _ => panic!("extend count_word() for {count} packs"),
    }
}

/// The README's "What ships today" inventory must agree with packs/*.json.
///
/// quick-start's coverage table has been pinned to the shipped packs since
/// the 2026-08-25 audit; the README had no equivalent guard, so the kubectl
/// pack (ADR-001, 2026-09-19) shipped while the README went on saying "Ten
/// rule packs, 26 guarded patterns, 18 safe patterns" with no kubectl row
/// -- found 2026-09-23, four days later (bead `irrevers-705a39ac`).
#[test]
fn readme_what_ships_today_matches_the_shipped_packs() {
    let doc = repo_relative("README.md");
    let start = doc
        .find("## What ships today")
        .expect("README should keep its 'What ships today' section");
    let section = doc[start..]
        .split("\n## ")
        .next()
        .expect("the section heading is non-empty");

    let packs_dir = audited_checkout().join("packs");
    let mut guarded = 0usize;
    let mut safe = 0usize;
    let mut ids: Vec<String> = Vec::new();
    for entry in fs::read_dir(&packs_dir).expect("packs/ should be readable") {
        let path = entry.expect("entry").path();
        if path.extension().and_then(|ext| ext.to_str()) != Some("json") {
            continue;
        }
        let pack: serde_json::Value = serde_json::from_str(&fs::read_to_string(&path).unwrap())
            .expect("pack should be valid JSON");
        guarded += pack["guarded_patterns"]
            .as_array()
            .expect("pack should carry guarded_patterns")
            .len();
        safe += pack["safe_patterns"]
            .as_array()
            .expect("pack should carry safe_patterns")
            .len();
        ids.push(
            pack["id"]
                .as_str()
                .expect("pack should carry an id")
                .to_owned(),
        );
    }
    ids.sort();
    assert!(!ids.is_empty(), "packs/ directory should not be empty");

    let claim = format!(
        "{} rule packs, {guarded} guarded patterns, {safe} safe patterns",
        count_word(ids.len())
    );
    assert!(
        section.contains(&claim),
        "README's 'What ships today' should say {claim:?}; it disagrees with \
         packs/*.json"
    );

    for id in &ids {
        assert!(
            section.contains(&format!("`{id}`")),
            "README's 'What ships today' table omits the shipped pack `{id}` \
             -- this is exactly how the kubectl pack shipped while the README \
             still said Ten rule packs"
        );
    }
}

/// The other total-count sentences a new pack invalidates, beside the
/// README's: quick-start's "What Gets Protected" opener and AGENTS.md's
/// coverage transcript. Pinning them means pack number twelve cannot land
/// without every count claim moving with it.
#[test]
fn doc_pack_count_claims_match_the_shipped_packs() {
    let packs_dir = audited_checkout().join("packs");
    let count = fs::read_dir(&packs_dir)
        .expect("packs/ should be readable")
        .filter(|entry| {
            entry
                .as_ref()
                .expect("entry")
                .path()
                .extension()
                .and_then(|ext| ext.to_str())
                == Some("json")
        })
        .count();
    assert!(count > 0, "packs/ directory should not be empty");

    let quick = quick_start();
    assert!(
        quick.contains(&format!("{} packs ship today", count_word(count))),
        "quick-start.md's 'What Gets Protected' opener should say {} packs \
         ship today",
        count
    );

    let agents = repo_relative("AGENTS.md");
    assert!(
        agents.contains(&format!("the {count} packs load")),
        "AGENTS.md's coverage --list transcript should say the {count} packs \
         load"
    );
}

/// The kubectl pack's shipped rule ids, and the retired exclusion claim.
///
/// ADR-001 (2026-09-19) absorbed org-rule-guard.py's rule 4 as
/// `packs/kubectl.json`, and the same change removed the pre-ADR statements
/// -- the README's "It does not cover `kubectl` mutations", quick-start's
/// "there is deliberately no kubectl pack and there will not be one", the
/// coexistence suite's "PERMANENTLY not absorbed". This keeps each of them
/// from returning on a surface an operator reads first. docs/adr/001 quotes
/// the old wording as superseded history, and plan.md and the
/// infrastructure note carry quote-and-retract residuals; both sit outside
/// this scan set on purpose.
#[test]
fn kubectl_pack_is_documented_as_shipped_and_the_exclusion_claim_stays_dead() {
    let pack: serde_json::Value = serde_json::from_str(&repo_relative("packs/kubectl.json"))
        .expect("kubectl pack should be valid JSON");
    let rule_ids: Vec<&str> = pack["guarded_patterns"]
        .as_array()
        .expect("kubectl pack should carry guarded_patterns")
        .iter()
        .map(|rule| rule["id"].as_str().expect("rule should carry an id"))
        .collect();
    assert_eq!(
        rule_ids,
        [
            "kubectl-delete",
            "kubectl-mutating-verb",
            "kubectl-create-outside-argo"
        ],
        "the kubectl pack's rule ids changed; update quick-start's coverage \
         table, the README row and this guard together"
    );

    let quick = quick_start();
    for id in &rule_ids {
        assert!(
            quick.contains(id),
            "quick-start.md's coverage table should name kubectl rule `{id}`"
        );
    }
    assert!(
        quick.contains(&format!("| `kubectl` | {} |", rule_ids.len())),
        "quick-start.md's coverage table should carry a kubectl row with \
         {} rules",
        rule_ids.len()
    );

    let readme = repo_relative("README.md");
    assert!(
        readme.contains("Its `kubectl` rules are blanket, not ArgoCD-aware"),
        "README's non-goals should describe the kubectl pack as it ships -- \
         blanket, not ArgoCD-aware (ADR-001)"
    );

    // The coexistence documentation records the absorption, not the
    // exclusion: the migration guide's rule table says double-deny, and the
    // infrastructure note says rule 4 moved to the pack.
    let migration = repo_relative("docs/operators/migration-from-org-rule-guard.md");
    assert!(
        migration.contains("**Covered by both**"),
        "the org-rule-guard migration guide should keep recording mutating \
         kubectl as denied by both guards during coexistence"
    );
    let infrastructure = repo_relative("docs/notes/existing-enforcement-infrastructure.md");
    assert!(
        infrastructure.contains("absorbed 2026-09-19 as the `kubectl`"),
        "existing-enforcement-infrastructure.md should keep recording rule 4 \
         as absorbed by the kubectl pack"
    );

    let mut scanned: Vec<(String, String)> = OPERATOR_FACING_DOCS
        .iter()
        .map(|doc| (doc.to_string(), repo_relative(doc)))
        .collect();
    scanned.push(("README.md".to_owned(), readme));
    scanned.push(("AGENTS.md".to_owned(), repo_relative("AGENTS.md")));

    // (needle, why it is wrong) -- the same shape
    // `operator_docs_do_not_cite_a_fictional_surface` uses.
    let banned = [
        (
            "no kubectl pack",
            "ADR-001 shipped the kubectl pack; a doc may describe its scope \
             but not claim it does not exist",
        ),
        (
            "deliberately no kubectl",
            "the pre-ADR-001 exclusion wording quick-start carried until \
             2026-09-19",
        ),
        (
            "It does not cover `kubectl`",
            "the pre-ADR-001 README non-goal; the kubectl pack covers these, \
             blanket rather than ArgoCD-aware",
        ),
        (
            "PERMANENTLY not absorbed",
            "the pre-ADR-001 coexistence-suite wording; rule 4 is absorbed",
        ),
    ];
    for (doc, text) in &scanned {
        for (needle, why) in banned {
            assert!(!text.contains(needle), "{doc} cites {needle:?} -- {why}");
        }
    }
}

/// Operator-facing docs must not resurrect the fictional inventory the
/// 2026-08-25 audit removed from quick-start.md.
///
/// The audit fixed one file. `training-manual.md`, `examples/README.md` and
/// `onboarding-guide.md` kept citing a `vault` pack (the shipped pack is
/// `openbao`), a `vault-kv-destroy` pattern id `icg explain` rejects, and a
/// `~/.config/claude-code/settings.json` hook shape no harness reads --
/// which is what a newcomer following the onboarding path actually typed.
#[test]
fn operator_docs_do_not_cite_a_fictional_surface() {
    let docs = OPERATOR_FACING_DOCS;

    // (needle, why it is wrong)
    let banned = [
        ("packs/vault.json", "the shipped OpenBao pack is `openbao`"),
        (
            "vault-kv-destroy",
            "the shipped pattern id is `openbao-destructive-verb`",
        ),
        (
            "~/.config/claude-code",
            "no harness reads this path; hooks live in ~/.claude/settings.json",
        ),
        (
            "~/.config/codex-cli",
            "the Codex CLI reads ~/.codex/hooks.json",
        ),
        (
            "releases/download/v0.1.0",
            "v0.1.0 is an orphaned tag that never carried artifacts",
        ),
        (
            "irreversible-command-gate/v0.1.0/packs/",
            "there is no v0.1.0 tag to fetch packs from",
        ),
        (
            "\"verdict\": \"deny\"",
            "the hook emits a PreToolUse envelope, not an icg-specific verdict object",
        ),
        (
            "ICG_PACK_PATH",
            "the environment variables are ICG_PACK_DIR and ICG_RULE_PACK",
        ),
    ];

    for doc in docs {
        let text = repo_relative(doc);
        for (needle, why) in banned {
            assert!(!text.contains(needle), "{doc} cites {needle:?} -- {why}");
        }
    }
}

/// Any pattern id an operator doc tells the reader to pass to `icg explain`
/// must actually exist in a shipped pack.
#[test]
fn documented_pattern_ids_exist_in_a_shipped_pack() {
    let packs_dir = audited_checkout().join("packs");
    let mut shipped: Vec<String> = Vec::new();
    for entry in fs::read_dir(&packs_dir).expect("packs/ should be readable") {
        let path = entry.expect("directory entry").path();
        if path.extension().and_then(|ext| ext.to_str()) != Some("json") {
            continue;
        }
        let pack: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(&path).unwrap()).expect("pack parses");
        for pattern in pack["guarded_patterns"].as_array().unwrap() {
            shipped.push(pattern["id"].as_str().unwrap().to_owned());
        }
    }

    for doc in OPERATOR_FACING_DOCS {
        let text = repo_relative(doc);
        for line in text.lines() {
            let Some(rest) = line.split("icg explain --pattern ").nth(1) else {
                continue;
            };
            let raw = rest.split_whitespace().next().unwrap_or_default();
            // `<id>`, `<pattern-id>`, `$PATTERN` and friends are placeholders.
            if raw.starts_with('<') || raw.starts_with('$') || raw.starts_with('{') {
                continue;
            }
            let cited = raw.trim_matches(|c: char| !c.is_ascii_alphanumeric() && c != '-');
            if cited.is_empty() {
                continue;
            }
            assert!(
                shipped.iter().any(|id| id == cited),
                "{doc} tells the reader to run `icg explain --pattern {cited}`, \
                 but no shipped pack defines that pattern"
            );
        }
    }
}

/// Install instructions must name the release this tree would cut.
///
/// Docs said "no GitHub release has been cut yet" for weeks after the tag
/// existed, and before that pointed at a `v0.1.0` that was orphaned. Pin the
/// version they cite to Cargo.toml so the two move together.
#[test]
fn install_docs_cite_the_current_release_version() {
    let cargo = repo_relative("Cargo.toml");
    let version = cargo
        .lines()
        .find_map(|line| line.strip_prefix("version = \""))
        .and_then(|rest| rest.split('"').next())
        .expect("Cargo.toml should declare a version");
    let tag = format!("v{version}");

    for doc in [
        "README.md",
        "docs/quick-start.md",
        "docs/onboarding-guide.md",
        "docs/examples/README.md",
        "docs/operators/training-manual.md",
    ] {
        let text = repo_relative(doc);
        assert!(
            text.contains("releases/download/"),
            "{doc} should tell the reader where to get the release binary"
        );
        assert!(
            text.contains(&format!("releases/download/{tag}")),
            "{doc} cites a release other than the current {tag}"
        );
        for stale in [
            "No GitHub release has been cut",
            "no release has been cut",
            "No end-to-end release has been cut",
        ] {
            assert!(
                !text.contains(stale),
                "{doc} still claims no release exists, but {tag} is tagged"
            );
        }
    }
}

/// Rule packs are release data installed to /etc/icg on arbitrary hosts.
/// Their text may not point at a file in one operator's home directory.
#[test]
fn packs_do_not_reference_paths_outside_this_repository() {
    let packs_dir = audited_checkout().join("packs");
    for entry in fs::read_dir(&packs_dir).expect("packs/ readable") {
        let path = entry.expect("entry").path();
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        let text = fs::read_to_string(&path).unwrap();
        let name = path.file_name().unwrap().to_string_lossy().into_owned();

        for offender in ["~/CLAUDE.md", "~/scratch/", "/home/coding"] {
            assert!(
                !text.contains(offender),
                "{name} points at {offender}, which does not exist for anyone \
                 who installs this pack from a release"
            );
        }

        // `~/.config/<app>/creds` is a generic destination the redirect tells
        // the caller to create, not a reference to an existing document --
        // so tildes are allowed only in that placeholder shape.
        for capture in text.split("~/").skip(1) {
            let referenced: String = capture
                .chars()
                .take_while(|c| !c.is_whitespace() && *c != '`' && *c != '"')
                .collect();
            assert!(
                referenced.starts_with(".config/"),
                "{name} references the home-directory path ~/{referenced}; \
                 pack text may only name a generic ~/.config destination"
            );
        }
    }
}

/// Every `icg <subcommand>` a doc shows in a runnable position must exist.
///
/// The audits so far checked pack ids, pattern ids, hook shapes and release
/// URLs. They did not check the CLI itself, so six invented commands survived
/// in the deeper sections a reader reaches after the install pages --
/// `icg alert create`, `icg audit`, `icg benchmark`, `icg export`,
/// `icg validate-pack`, `icg verify-coverage`, `icg config`. Each is
/// presented as something to type.
#[test]
fn documented_subcommands_all_exist() {
    let output = Command::new(env!("CARGO_BIN_EXE_icg"))
        .arg("--help")
        .output()
        .expect("icg --help should run");
    let help = String::from_utf8_lossy(&output.stdout).into_owned();

    let mut real: BTreeSet<String> = BTreeSet::new();
    let mut in_commands = false;
    for line in help.lines() {
        if line.starts_with("Commands:") {
            in_commands = true;
            continue;
        }
        if line.starts_with("Options:") {
            break;
        }
        if !in_commands {
            continue;
        }
        if let Some(word) = line.split_whitespace().next() {
            if word.chars().all(|c| c.is_ascii_lowercase() || c == '-') && !word.is_empty() {
                real.insert(word.to_owned());
            }
        }
    }
    assert!(
        real.contains("check") && real.contains("hook"),
        "failed to parse the subcommand list out of `icg --help`"
    );
    // `wrapper` is real but `#[command(hide = true)]`, so it never appears in
    // --help. Docs are right to name it.
    real.insert("wrapper".to_owned());

    let root = &audited_checkout();
    let mut offenders: Vec<String> = Vec::new();

    for doc in markdown_files(&root.join("docs"))
        .into_iter()
        .chain([root.join("README.md"), root.join("AGENTS.md")])
    {
        let Ok(text) = fs::read_to_string(&doc) else {
            continue;
        };
        let relative = doc.strip_prefix(root).unwrap_or(&doc).display().to_string();

        // The ideas ledger names commands that were *proposed* and in most
        // cases killed; archived audits describe a tree that no longer
        // exists. Neither tells a reader to run anything.
        if relative.contains("ideas-ledger") || relative.contains("notes/archive/") {
            continue;
        }

        let mut fenced = false;
        for (number, line) in text.lines().enumerate() {
            if line.trim_start().starts_with("```") {
                fenced = !fenced;
                continue;
            }
            // Only a fenced line, or an inline `icg …` span, is a runnable
            // position. Prose like "icg intercepts the call" is not.
            // A line that names a command in order to say it does not exist
            // is a correction, not an instruction. "There is no `icg alert`"
            // must be allowed to say so.
            if disclaims_a_command(line) {
                continue;
            }
            let candidates = if fenced {
                subcommands_after_icg(line)
            } else {
                inline_code_subcommands_after_icg(line)
            };
            for candidate in candidates {
                if !real.contains(&candidate) {
                    offenders.push(format!("{relative}:{}: icg {candidate}", number + 1));
                }
            }
        }
    }

    assert!(
        offenders.is_empty(),
        "docs show {} `icg <subcommand>` invocation(s) that do not exist:\n  {}",
        offenders.len(),
        offenders.join("\n  ")
    );
}

fn markdown_files(dir: &Path) -> Vec<PathBuf> {
    let mut found = Vec::new();
    let Ok(entries) = fs::read_dir(dir) else {
        return found;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            found.extend(markdown_files(&path));
        } else if path.extension().and_then(|e| e.to_str()) == Some("md") {
            found.push(path);
        }
    }
    found
}

/// Subcommand words following an `icg` in *command position*.
///
/// Command position means the line's first token, or the first token after a
/// shell separator. Prose mentioning icg mid-sentence -- including a `#`
/// comment inside a fence, or markdown heredoc'd into a file -- is not a
/// runnable invocation and must not be flagged.
fn subcommands_after_icg(line: &str) -> Vec<String> {
    let trimmed = line.trim_start();
    if trimmed.starts_with('#') || trimmed.starts_with("//") {
        return Vec::new();
    }

    let mut found = Vec::new();
    let bytes = line.as_bytes();
    let mut search = 0usize;
    while let Some(offset) = line[search..].find("icg ") {
        let at = search + offset;
        // Everything before `icg` on this line, with any path prefix removed.
        let before = line[..at].trim_end_matches(|c: char| c != ' ' && c != '\t');
        let prefix = before.trim();
        let in_command_position = prefix.is_empty()
            || prefix.ends_with('$')
            || prefix.ends_with('|')
            || prefix.ends_with("&&")
            || prefix.ends_with("||")
            || prefix.ends_with(';')
            || prefix.ends_with("sudo")
            || prefix.ends_with("run");
        // `icg` must also start a word.
        let starts_word =
            at == 0 || matches!(bytes[at - 1], b' ' | b'$' | b'|' | b'(' | b'/' | b'\t');
        search = at + 4;
        if !starts_word || !in_command_position {
            continue;
        }
        // Skip a path prefix like ./target/release/icg -- still a real call.
        if let Some(word) = line[search..].split_whitespace().next() {
            if word.starts_with('-') || word.starts_with('<') || word.starts_with('$') {
                continue;
            }
            let cleaned: String = word
                .chars()
                .take_while(|c| c.is_ascii_lowercase() || *c == '-')
                .collect();
            if !cleaned.is_empty() {
                found.push(cleaned);
            }
        }
    }
    found
}

/// The same, but only inside inline code spans, so prose cannot trip it.
fn inline_code_subcommands_after_icg(line: &str) -> Vec<String> {
    let mut found = Vec::new();
    for span in line.split('`').skip(1).step_by(2) {
        found.extend(subcommands_after_icg(span));
    }
    found
}

/// Does this line name a command in order to deny that it exists?
fn disclaims_a_command(line: &str) -> bool {
    let lowered = line.to_ascii_lowercase();
    [
        "there is no",
        "does not exist",
        "never existed",
        "no such command",
        "is not a subcommand",
        "not a real subcommand",
    ]
    .iter()
    .any(|phrase| lowered.contains(phrase))
}

/// Every long flag a doc passes to an `icg` subcommand must exist on it.
///
/// `documented_subcommands_all_exist` catches an invented *command*; it does
/// not catch an invented *flag* on a real one, which is how
/// `icg status --severity`, `--by-severity`, `--summary`, `--rate`,
/// `--compare-weeks`, `--report`, `--tag`, `--by-session`,
/// `icg new-pack --id/--mode`, `icg update --dry-run/--rollback` and
/// `icg export-denial --id/--output` all survived. Each reads as a working
/// invocation and exits 2.
#[test]
fn documented_flags_exist_on_their_subcommand() {
    let root = &audited_checkout();
    let mut offenders: Vec<String> = Vec::new();

    for doc in markdown_files(&root.join("docs"))
        .into_iter()
        .chain([root.join("README.md"), root.join("AGENTS.md")])
    {
        let Ok(text) = fs::read_to_string(&doc) else {
            continue;
        };
        let relative = doc.strip_prefix(root).unwrap_or(&doc).display().to_string();
        if relative.contains("ideas-ledger") || relative.contains("notes/archive/") {
            continue;
        }

        let mut fenced = false;
        for (number, line) in text.lines().enumerate() {
            if line.trim_start().starts_with("```") {
                fenced = !fenced;
                continue;
            }
            if !fenced || disclaims_a_command(line) {
                continue;
            }
            let trimmed = line.trim_start();
            if trimmed.starts_with('#') || trimmed.starts_with("//") {
                continue;
            }
            let Some(rest) = invocation_after_icg(line) else {
                continue;
            };
            let Some((path, flags)) = split_invocation(&rest) else {
                continue;
            };
            let Some(accepted) = flags_for_subcommand(&path) else {
                continue; // not a real subcommand: the other test owns that
            };
            for flag in flags {
                if !accepted.contains(&flag) {
                    offenders.push(format!(
                        "{relative}:{}: icg {} {flag}",
                        number + 1,
                        path.join(" ")
                    ));
                }
            }
        }
    }

    assert!(
        offenders.is_empty(),
        "docs pass {} flag(s) that the subcommand does not accept:\n  {}",
        offenders.len(),
        offenders.join("\n  ")
    );
}

/// The text following an `icg` in command position, if any.
fn invocation_after_icg(line: &str) -> Option<String> {
    // Stop at a shell pipe or redirect: what follows belongs to another program.
    let head = line
        .split(" | ")
        .next()
        .unwrap_or(line)
        .split('>')
        .next()
        .unwrap_or(line);
    let bytes = head.as_bytes();
    let mut search = 0usize;
    while let Some(offset) = head[search..].find("icg ") {
        let at = search + offset;
        let before = head[..at].trim_end_matches(|c: char| c != ' ' && c != '\t');
        let prefix = before.trim();
        let command_position = prefix.is_empty()
            || prefix.ends_with('$')
            || prefix.ends_with('|')
            || prefix.ends_with("&&")
            || prefix.ends_with(';')
            || prefix.ends_with("sudo");
        let starts_word =
            at == 0 || matches!(bytes[at - 1], b' ' | b'$' | b'|' | b'(' | b'/' | b'\t');
        search = at + 4;
        if command_position && starts_word {
            return Some(head[search..].to_string());
        }
    }
    None
}

/// Split `check --command "git push --force"` into (["check"], ["--command"]).
///
/// Flags inside a quoted argument belong to the guarded command, not to icg --
/// `icg check --command "git push --force origin main"` is the single most
/// common line in these docs and must not be read as `icg check --force`.
fn split_invocation(rest: &str) -> Option<(Vec<String>, Vec<String>)> {
    let mut path = Vec::new();
    let mut flags = Vec::new();
    let mut quote: Option<char> = None;
    let mut current = String::new();
    let mut words: Vec<String> = Vec::new();

    for character in rest.chars() {
        match quote {
            Some(q) if character == q => quote = None,
            Some(_) => {}
            None if character == '"' || character == '\'' => quote = Some(character),
            None if character.is_whitespace() => {
                if !current.is_empty() {
                    words.push(std::mem::take(&mut current));
                }
                continue;
            }
            None => {}
        }
        if quote.is_none() && (character == '"' || character == '\'') {
            continue;
        }
        if quote.is_some() && (character == '"' || character == '\'') {
            continue;
        }
        current.push(character);
    }
    if !current.is_empty() && quote.is_none() {
        words.push(current);
    }

    let mut seen_flag = false;
    for word in words {
        if word.starts_with("--") {
            seen_flag = true;
            let name = word.split('=').next().unwrap_or(&word);
            // `--command|--stdin|--file` is alternation notation, not a call.
            if name.contains('|') || name.ends_with(')') {
                return None;
            }
            flags.push(name.to_string());
        } else if !seen_flag
            && word.chars().all(|c| c.is_ascii_lowercase() || c == '-')
            && !word.is_empty()
        {
            path.push(word);
        }
    }
    (!path.is_empty()).then_some((path, flags))
}

/// Long flags `icg <path…> --help` accepts, or None if the path is not real.
fn flags_for_subcommand(path: &[String]) -> Option<BTreeSet<String>> {
    // Try the deepest path first, then fall back to the top-level subcommand:
    // `backup create` has its own flags, but `status` has no nested commands.
    for depth in (1..=path.len().min(2)).rev() {
        let mut args: Vec<&str> = path[..depth].iter().map(String::as_str).collect();
        args.push("--help");
        let output = Command::new(env!("CARGO_BIN_EXE_icg")).args(&args).output();
        let Ok(output) = output else { continue };
        if !output.status.success() {
            continue;
        }
        let text = String::from_utf8_lossy(&output.stdout).into_owned();
        let mut flags = BTreeSet::new();
        for line in text.lines() {
            for token in line.split_whitespace() {
                if let Some(flag) = token.strip_suffix(',') {
                    if flag.starts_with("--") {
                        flags.insert(flag.to_string());
                    }
                }
                if token.starts_with("--") {
                    let name = token
                        .split(['=', '<', '[', ','])
                        .next()
                        .unwrap_or(token)
                        .to_string();
                    if name.len() > 2 {
                        flags.insert(name);
                    }
                }
            }
        }
        return Some(flags);
    }
    None
}

/// No operator doc may describe the PATH wrapper as unimplemented.
///
/// `troubleshooting_does_not_claim_the_wrapper_is_unimplemented` guards one
/// file by one phrase. `deployment-guide.md` said the same thing in two other
/// wordings -- "not a production wrapper yet", "a parser scaffold [that] does
/// not execute a real binary" -- and told the reader not to create the
/// symlinks at all. argv[0] dispatch ships: the wrapper denies with a
/// non-zero exit and otherwise execs the real binary (wrapper_deny_tests.rs).
#[test]
fn no_doc_claims_the_wrapper_is_unimplemented() {
    let stale = [
        "subcommand is not implemented",
        "not a production wrapper",
        "parser scaffold",
        "does not execute a real binary",
        "Do not create PATH-shadowing symlinks",
        "wrapper is not yet implemented",
    ];

    let root = &audited_checkout();
    let mut offenders = Vec::new();
    for doc in markdown_files(&root.join("docs"))
        .into_iter()
        .chain([root.join("README.md"), root.join("AGENTS.md")])
    {
        let Ok(text) = fs::read_to_string(&doc) else {
            continue;
        };
        let relative = doc.strip_prefix(root).unwrap_or(&doc).display().to_string();
        if relative.contains("ideas-ledger") || relative.contains("notes/archive/") {
            continue;
        }
        for phrase in stale {
            if text.contains(phrase) {
                offenders.push(format!("{relative}: {phrase:?}"));
            }
        }
    }

    assert!(
        offenders.is_empty(),
        "docs still describe the shipped PATH wrapper as unimplemented:\n  {}",
        offenders.join("\n  ")
    );
}

/// The guard performs no identity check, and the docs must say so.
///
/// Four rules tell the caller that "a human" should run the operation
/// instead. Nothing in the engine distinguishes a human from an agent --
/// there is no isatty, getuid or SUDO_USER anywhere in `src/` -- so that
/// phrasing describes a procedure, and a reader who assumes otherwise has
/// assumed a control that does not exist.
#[test]
fn docs_state_that_the_guard_does_not_check_caller_identity() {
    let readme = repo_relative("README.md");
    assert!(
        readme.contains("It does not know who is calling."),
        "the README's non-goals must state that no identity check exists"
    );

    let quick_start = quick_start();
    assert!(
        quick_start.contains("**audited** escape hatch, not a restricted one"),
        "quick-start must describe ICG_DISABLED as audited rather than restricted"
    );
    assert!(
        !quick_start.contains("operator-controlled escape hatch"),
        "quick-start must not imply ICG_DISABLED is restricted to an operator; \
         it is an environment variable the guarded agent can set"
    );

    let deployment = repo_relative("docs/operators/deployment-guide.md");
    assert!(
        deployment.contains("### Scoping the wrapper to the agent"),
        "the deployment guide must explain how to scope the wrapper's symlinks \
         so they reach the agent without shadowing the operator's shell"
    );
}

/// Build instructions must not hard-code where cargo puts its output.
///
/// The hosts this repository is built on share one cargo target directory:
/// the global `~/.cargo/config.toml` points `target-dir` at
/// `/build/target-workers`, and `CARGO_TARGET_DIR` moves it per invocation.
/// A doc that runs a freshly built binary from an assumed in-checkout
/// `target/release` path is therefore wrong on those hosts (the path does
/// not exist -- `install.sh` grew its `cargo metadata` fallback after a
/// "cannot stat" failure on exactly this), and a reader who "fixes" it by
/// redirecting the build into the checkout or under `/home` recreates the
/// 2026-09-07 incident that policy exists to prevent: a relative target dir
/// resolved against `~/.cargo` filled `/home` to 99%. The sanctioned forms
/// are the ones the repo itself uses -- `cargo run --release`, and
/// `install.sh --from-checkout` / `cargo metadata`'s `target_directory`
/// for locating the artifact.
#[test]
fn docs_do_not_hardcode_the_cargo_build_output_location() {
    let root = audited_checkout();
    let docs = OPERATOR_FACING_DOCS
        .iter()
        .map(|relative| root.join(relative))
        .chain([
            root.join("README.md"),
            root.join("AGENTS.md"),
            // Shell/tape assets are all instruction -- no fences to stay
            // inside.
            root.join("docs/assets/demo.sh"),
            root.join("docs/assets/demo.tape"),
        ]);

    let mut offenders = Vec::new();
    for doc in docs {
        let text = fs::read_to_string(&doc)
            .unwrap_or_else(|error| panic!("should read {}: {error}", doc.display()));
        let relative = doc
            .strip_prefix(&root)
            .unwrap_or(&doc)
            .display()
            .to_string();
        // In markdown only fenced blocks are instructions; prose may
        // discuss the policy (and this file's own doc comments do).
        let is_markdown = relative.ends_with(".md");
        let mut fenced = false;
        for (number, line) in text.lines().enumerate() {
            if is_markdown && line.trim_start().starts_with("```") {
                fenced = !fenced;
                continue;
            }
            if is_markdown && !fenced {
                continue;
            }
            if line.contains("target/release") {
                offenders.push(format!(
                    "{relative}:{}: hard-coded build-output path -- resolve it \
                     via cargo metadata / install.sh --from-checkout, or use \
                     cargo run --release",
                    number + 1
                ));
            }
            if directs_build_output_into_home(line) {
                offenders.push(format!(
                    "{relative}:{}: directs cargo build output into /home",
                    number + 1
                ));
            }
        }
    }

    assert!(
        offenders.is_empty(),
        "docs carry build instructions that assume or recreate an in-/home \
         target directory:\n  {}",
        offenders.join("\n  ")
    );
}

/// A line that points cargo's build output under `/home` -- the filesystem
/// the shared-target policy keeps build output off.
fn directs_build_output_into_home(line: &str) -> bool {
    const HOME_ROOTS: [&str; 3] = ["/home", "$HOME", "~/"];
    let home_rooted = |value: &str| {
        let value = value.trim_matches(|c| c == '"' || c == '\'');
        HOME_ROOTS.iter().any(|root| value.starts_with(root))
    };
    let words: Vec<&str> = line.split_whitespace().collect();
    words.iter().any(|word| {
        word.strip_prefix("CARGO_TARGET_DIR=")
            .or_else(|| word.strip_prefix("--target-dir="))
            .is_some_and(home_rooted)
    }) || words
        .windows(2)
        .any(|pair| pair[0] == "--target-dir" && home_rooted(pair[1]))
}

/// Every stanza in the coverage-justification record must name a real rule.
///
/// `coverage-diff` cannot distinguish a widening from a narrowing, so it
/// flags any regex change on a destructive pattern and CI requires a
/// `## <pattern-id>` stanza in packs/coverage-justifications.md. That file is
/// a standing waiver list, so it needs its own guard: a stanza naming a rule
/// that no longer exists is dead weight that quietly grows, and a typo in an
/// id is a waiver that silently covers nothing.
#[test]
fn coverage_justifications_name_real_patterns() {
    let record = repo_relative("packs/coverage-justifications.md");
    let packs_dir = audited_checkout().join("packs");

    let mut shipped: BTreeSet<String> = BTreeSet::new();
    for entry in fs::read_dir(&packs_dir).expect("packs/ readable") {
        let path = entry.expect("entry").path();
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        let pack: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(&path).unwrap()).expect("pack parses");
        for pattern in pack["guarded_patterns"].as_array().unwrap() {
            shipped.insert(pattern["id"].as_str().unwrap().to_owned());
        }
    }

    for line in record.lines() {
        let Some(id) = line.strip_prefix("## ") else {
            continue;
        };
        let id = id.trim();
        assert!(
            shipped.contains(id),
            "packs/coverage-justifications.md has a stanza for `{id}`, which no \
             shipped pack defines. Either the id is a typo -- in which case the \
             waiver covers nothing and CI will still stop -- or the rule is gone \
             and the stanza should be removed."
        );
    }
}

/// The engine's no-network guarantee and its one exception
/// (bead `irrevers-d8469484`).
///
/// `tests/no_network_boundary_tests.rs` enforces the guarantee against the
/// engine; this guard holds the prose to it. README.md, AGENTS.md rule 3,
/// deployment-guide.md, training-manual.md and onboarding-guide.md each
/// stated the guarantee absolutely -- "does no network I/O", "evaluates
/// commands without network access", "doesn't make network calls" --
/// while the shipped `git-stale-remote-head-push` rule runs a live
/// `git ls-remote` before every non-force `git push`
/// (`push_requires_current_remote_head` -> `check_remote_head_stale` in
/// src/engine.rs), the exception documented in
/// docs/notes/no-network-boundary.md. The reconciled pages state the
/// exception wherever they state the claim, so a reader sizing what an
/// `icg check` may touch no longer rules out a network round trip the
/// shipped packs actually make. The needles below are the absolute
/// phrasings; a page may carry one only with the exception named in the
/// same document.
#[test]
fn no_network_claims_name_the_stale_remote_head_exception() {
    let claim_needles = [
        ("no network i/o", "README's absolute sentence"),
        (
            "without network access",
            "deployment-guide's absolute sentence",
        ),
        ("make network calls", "onboarding-guide's FAQ answer"),
        ("zero network", "training-manual's design-principle list"),
        (
            "doesn't require external calls",
            "training-manual's design-principle list",
        ),
    ];
    let names_the_exception = |text: &str| {
        let lowered = text.to_ascii_lowercase();
        lowered.contains("stale-remote-head") || lowered.contains("ls-remote")
    };

    let mut scanned: Vec<(String, String)> = OPERATOR_FACING_DOCS
        .iter()
        .map(|doc| (doc.to_string(), repo_relative(doc)))
        .collect();
    scanned.push(("README.md".to_owned(), repo_relative("README.md")));
    scanned.push(("AGENTS.md".to_owned(), repo_relative("AGENTS.md")));

    for (doc, text) in &scanned {
        let lowered = text.to_ascii_lowercase();
        for (needle, origin) in claim_needles {
            if lowered.contains(needle) && !names_the_exception(text) {
                panic!(
                    "{doc} states the no-network guarantee ({origin}) without \
                     naming its one exception -- the stale-remote-head lookup \
                     before a non-force `git push`, documented in \
                     docs/notes/no-network-boundary.md"
                );
            }
        }
    }

    // The reconciled surfaces must keep stating the exception explicitly,
    // not merely avoid the needles above: activation (the rule and its
    // ls-remote lookup), failure behavior, and fail-open semantics.
    let readme = repo_relative("README.md");
    for marker in [
        "git-stale-remote-head-push",
        "ls-remote",
        "fails open",
        "docs/notes/no-network-boundary.md",
    ] {
        assert!(
            readme.contains(marker),
            "README.md's no-network sentence must keep naming {marker}"
        );
    }
    let deployment = repo_relative("docs/operators/deployment-guide.md");
    for marker in [
        "git-stale-remote-head-push",
        "ls-remote",
        "fails open",
        "../notes/no-network-boundary.md",
    ] {
        assert!(
            deployment.contains(marker),
            "deployment-guide.md's network paragraph must keep naming {marker}"
        );
    }
    for (doc, text) in [
        (
            "docs/operators/training-manual.md",
            repo_relative("docs/operators/training-manual.md"),
        ),
        (
            "docs/onboarding-guide.md",
            repo_relative("docs/onboarding-guide.md"),
        ),
        (
            "docs/developers/README.md",
            repo_relative("docs/developers/README.md"),
        ),
    ] {
        assert!(
            names_the_exception(&text),
            "{doc} must keep naming the stale-remote-head exception to the \
             no-network guarantee"
        );
    }

    // The exception the docs describe is the one that ships: the pack
    // defines the rule with the push-gated predicate and a deny redirect.
    let pack: serde_json::Value =
        serde_json::from_str(&repo_relative("packs/git.json")).expect("git pack parses");
    let rule = pack["guarded_patterns"]
        .as_array()
        .expect("git pack should carry guarded_patterns")
        .iter()
        .find(|pattern| pattern["id"] == "git-stale-remote-head-push")
        .expect(
            "packs/git.json should define git-stale-remote-head-push; \
                 the docs' exception names it",
        );
    assert_eq!(
        rule["predicate_name"], "push_requires_current_remote_head",
        "the documented exception must stay the push-gated predicate"
    );
    assert_eq!(
        rule["redirect"]["channel"], "deny",
        "the stale-remote-head rule must keep denying a stale push"
    );

    // ...and the canonical note must keep describing it.
    let note = repo_relative("docs/notes/no-network-boundary.md");
    for marker in [
        "git-stale-remote-head-push",
        "push_requires_current_remote_head",
        "ls-remote",
        "fails open",
    ] {
        assert!(
            note.contains(marker),
            "docs/notes/no-network-boundary.md must keep naming {marker}"
        );
    }
}

// ---------------------------------------------------------------------------
// plan.md release/status claims vs committed ground truth.
//
// The 2026-09-14 Phase 0 reconciliation and the 2026-09-17 "verified
// fail-closed" reconciliation (beads `irrevers-eff8909f`, `irrevers-4e649dbf`,
// `irrevers-92e6e55c`) each fixed the same drift class by hand: plan.md said
// "no release has ever been cut" for weeks after tags shipped, and its
// still-open lists described beads whose status had since moved. Both were
// found only by a human re-reading the plan against `git tag` and the bead
// store. These checks pin the reconciled claims to the committed evidence
// those reconciliations cite -- `docs/notes/shipped-releases-inventory.md`
// for tags/releases, `.beads/checkpoint/` (git-tracked, auto-published on
// every bead mutation) for bead status -- so the next drift fails `cargo
// test` in a normal run instead of waiting for another manual audit.
// ---------------------------------------------------------------------------

/// Whitespace-flattened text, so a claim split across markdown line breaks
/// ("`irrevers-f59f9313`,\n  `irrevers-c87a3c50`\n  closed)") reads as one
/// window.
fn flattened(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// `v0.1.N` -> N; the only tag shape this repo cuts.
fn patch_version(tag: &str) -> u64 {
    tag.strip_prefix("v0.1.")
        .and_then(|rest| rest.parse().ok())
        .unwrap_or_else(|| panic!("unexpected tag shape {tag:?}"))
}

struct InventoryRow {
    tag: String,
    released: bool,
    published_date: String,
    latest: bool,
}

/// Parse the per-version table out of the shipped-releases inventory.
///
/// That doc was generated from `git for-each-ref` and the GitHub Releases
/// API (collection method recorded in the doc itself); it is the committed
/// stand-in for those live sources, which a test cannot query -- CI checks
/// this repo out with `git clone --depth 1`, so no tags and no `gh` exist at
/// test time there.
fn shipped_inventory_rows() -> Vec<InventoryRow> {
    let text = repo_relative("docs/notes/shipped-releases-inventory.md");
    let mut rows = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if !line.starts_with("| v0.1.") {
            continue;
        }
        let cells: Vec<&str> = line.split('|').collect();
        assert!(
            cells.len() >= 6,
            "inventory table row should have five cells: {line}"
        );
        let published = cells[4].trim();
        rows.push(InventoryRow {
            tag: cells[1].trim().to_owned(),
            released: !published.is_empty() && published != "\u{2014}",
            published_date: published.to_owned(),
            latest: line.contains("**Latest**"),
        });
    }
    assert!(
        rows.len() >= 60,
        "the inventory should enumerate the shipped tags, got {}",
        rows.len()
    );
    for (index, row) in rows.iter().enumerate() {
        assert_eq!(
            patch_version(&row.tag),
            index as u64,
            "inventory tags should be contiguous from v0.1.0"
        );
    }
    rows
}

/// "`vA`--`vB`" (en-dash separated, backticked) -> ("vA", "vB").
fn backticked_range(range: &str) -> (String, String) {
    let (first, rest) = range
        .split_once("\u{2013}`")
        .expect("range should be two backticked tags joined by an en-dash");
    let last = rest
        .split('`')
        .next()
        .expect("range should close the second tag");
    let first = first.strip_suffix('`').unwrap_or(first);
    (first.to_owned(), last.to_owned())
}

/// The "there are 62 tags (`v0.1.0`\u{2013}`v0.1.61`)" claim in plan.md.
fn plan_tag_count_claim(plan: &str) -> (u64, String, String) {
    let mut from = 0;
    while let Some(rel) = plan[from..].find("there are ") {
        let rest = &plan[from + rel + "there are ".len()..];
        let digits: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
        if let Ok(count) = digits.parse::<u64>() {
            if let Some(range) = rest[digits.len()..].strip_prefix(" tags (`") {
                let (first, last) = backticked_range(range);
                return (count, first, last);
            }
        }
        from += rel + "there are ".len();
    }
    panic!(
        "plan.md should state the shipped tag count as \
         \"there are N tags (`vA`\u{2013}`vB`)\""
    );
}

/// The "61 GitHub Releases (`vA`\u{2013}`vB`; ...)" claim, with the full
/// parenthetical so callers can check its qualifiers.
fn plan_release_count_claim(plan: &str) -> (u64, String, String, String) {
    let mut from = 0;
    while let Some(rel) = plan[from..].find("GitHub Releases (`v") {
        let at = from + rel;
        let before = plan[..at].trim_end();
        let digits: String = before
            .chars()
            .rev()
            .take_while(|c| c.is_ascii_digit())
            .collect::<String>()
            .chars()
            .rev()
            .collect();
        if let Ok(count) = digits.parse::<u64>() {
            let rest = &plan[at..];
            let open = rest.find('(').expect("release claim opens a parenthetical");
            let close = rest[open..]
                .find(')')
                .expect("release parenthetical closes")
                + open;
            // skip "(`" -- the marker above guarantees that shape
            let paren = &rest[open + 2..close];
            let (range_part, _) = paren.split_once("; ").unwrap_or((paren, ""));
            let (first, last) = backticked_range(range_part);
            return (count, first, last, paren.to_owned());
        }
        from = at + 1;
    }
    panic!("plan.md should state the GitHub Releases count and range");
}

/// The "latest `v0.1.61` on 2026-09-13" claim in plan.md.
fn plan_latest_release_claim(plan: &str) -> (String, String) {
    let mut from = 0;
    while let Some(rel) = plan[from..].find("latest `v0.") {
        let at = from + rel;
        let rest = &plan[at + "latest `".len()..];
        let tag = rest.split('`').next().expect("latest tag is backticked");
        let after = &rest[rest.find('`').expect("latest tag closes") + 1..];
        if let Some(date) = after.strip_prefix(" on ") {
            let day: String = date
                .chars()
                .take_while(|c| c.is_ascii_digit() || *c == '-')
                .collect();
            if day.len() == 10 {
                return (tag.to_owned(), day);
            }
        }
        from = at + 1;
    }
    panic!(
        "plan.md should state the latest release as \
         \"latest `vX.Y.Z` on YYYY-MM-DD\""
    );
}

/// plan.md's Phase 0 release block must agree with the shipped-releases
/// inventory, and the old "no release has ever been cut" fiction may appear
/// only as a quoted residual that the same sentence marks stale.
#[test]
fn plan_release_claims_match_the_shipped_releases_inventory() {
    let plan = flattened(&repo_relative("docs/plan/plan.md"));
    let rows = shipped_inventory_rows();

    let unreleased: Vec<&str> = rows
        .iter()
        .filter(|row| !row.released)
        .map(|row| row.tag.as_str())
        .collect();
    assert_eq!(
        unreleased,
        ["v0.1.0"],
        "the inventory should show exactly one tagged-but-never-released version"
    );

    let (tag_count, tag_first, tag_last) = plan_tag_count_claim(&plan);
    assert_eq!(
        tag_count as usize,
        rows.len(),
        "plan.md's shipped tag count disagrees with the inventory"
    );
    assert_eq!(tag_first, rows[0].tag, "plan.md's first tag disagrees");
    assert_eq!(
        tag_last,
        rows[rows.len() - 1].tag,
        "plan.md's newest tag disagrees with the inventory"
    );

    let released: Vec<&InventoryRow> = rows.iter().filter(|row| row.released).collect();
    let (release_count, release_first, release_last, paren) = plan_release_count_claim(&plan);
    assert_eq!(
        release_count as usize,
        released.len(),
        "plan.md's GitHub Releases count disagrees with the inventory"
    );
    assert_eq!(
        release_first, released[0].tag,
        "plan.md's first release disagrees"
    );
    assert_eq!(
        release_last,
        released[released.len() - 1].tag,
        "plan.md's newest release disagrees with the inventory"
    );
    assert!(
        paren.contains("never published to GitHub"),
        "plan.md should keep stating that the first tag ({}) has no GitHub \
         release -- that is the set difference the inventory records",
        unreleased[0]
    );

    let latest_row = rows
        .iter()
        .find(|row| row.latest)
        .expect("the inventory should mark one row **Latest**");
    let (latest_tag, latest_date) = plan_latest_release_claim(&plan);
    assert_eq!(
        latest_tag, latest_row.tag,
        "plan.md's latest release disagrees"
    );
    assert_eq!(
        latest_date,
        &latest_row.published_date[..10],
        "plan.md's latest-release date disagrees with the inventory"
    );

    assert!(
        plan.contains(
            "all four assets (`icg`, `icg-packs.tar.gz`, \
                       `pack-manifest.json`, `rule-pack.json`)"
        ),
        "plan.md should keep naming the four release assets every release ships"
    );
    let inventory = repo_relative("docs/notes/shipped-releases-inventory.md");
    for asset in ["icg-packs.tar.gz", "pack-manifest.json", "rule-pack.json"] {
        assert!(
            inventory.contains(asset),
            "the inventory's artifact evidence should name {asset}"
        );
    }

    // The reconciled block quotes the old fiction only to retract it. Any
    // occurrence of the no-release claim must carry its own retraction
    // nearby, so the stale phrasing cannot quietly become affirmative again.
    let mut residuals = 0;
    for phrase in [
        "no release has ever been cut",
        "no release has been cut",
        "No GitHub release has been cut",
    ] {
        let mut from = 0;
        while let Some(rel) = plan[from..].find(phrase) {
            let start = from + rel;
            let end = (start + phrase.len() + 90).min(plan.len());
            let tail = &plan[start..end];
            assert!(
                tail.contains("stale") || tail.contains("no longer holds"),
                "plan.md states {phrase:?} without marking it stale; releases \
                 exist -- see docs/notes/shipped-releases-inventory.md"
            );
            residuals += 1;
            from = start + phrase.len();
        }
    }
    assert!(
        residuals >= 1,
        "the reconciled release block (quoting and retracting the stale \
         no-release residual) should stay in plan.md"
    );
}

/// Where tags are actually available (a working tree or full clone), the
/// inventory must not have fallen behind them. CI checks out with
/// `git clone --depth 1` -- no tags -- and NEEDLE verification extracts a
/// tarball with no `.git` at all, so an empty or missing tag list skips
/// rather than fails; the plan-vs-inventory assertions above carry those
/// environments.
#[test]
fn local_git_tags_agree_with_the_shipped_releases_inventory() {
    let rows = shipped_inventory_rows();
    let checkout = audited_checkout();
    let Ok(output) = Command::new("git")
        .args([
            "-C",
            checkout.to_str().expect("checkout path is valid utf-8"),
            "tag",
            "-l",
            "v0.1.*",
        ])
        .output()
    else {
        return; // no git or no checkout: nothing live to compare against
    };
    if !output.status.success() {
        return;
    }
    let tags: Vec<String> = String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(str::trim)
        .filter(|tag| !tag.is_empty())
        .map(str::to_owned)
        .collect();
    if tags.is_empty() {
        return; // shallow CI checkout: no refs to compare
    }

    let live_max = tags.iter().map(|tag| patch_version(tag)).max().unwrap();
    let inventory_max = patch_version(&rows[rows.len() - 1].tag);
    for index in 0..rows.len() {
        let tag = format!("v0.1.{index}");
        assert!(
            tags.iter().any(|live| live == &tag),
            "the inventory lists {tag} but this checkout does not have it; \
             either the inventory is stale or tags were not fetched \
             (git fetch --tags origin)"
        );
    }
    assert_eq!(
        live_max, inventory_max,
        "this checkout has tags the inventory does not list (v0.1.{live_max} \
         vs v0.1.{inventory_max}): re-collect the inventory and reconcile \
         plan.md's release block"
    );
}

/// A release/status claim plan.md's prose attaches to a bead id.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ClaimedStatus {
    Closed,
    Open,
    InProgress,
}

impl ClaimedStatus {
    /// What the bead store's `base_status` must say for the claim to hold.
    /// "Open" is the loose claim and tolerates `in_progress` (still not
    /// closed); "in progress" is specific and requires exactly that.
    fn holds_against(self, base_status: &str) -> bool {
        match self {
            ClaimedStatus::Closed => base_status == "closed",
            ClaimedStatus::Open => matches!(base_status, "open" | "in_progress"),
            ClaimedStatus::InProgress => base_status == "in_progress",
        }
    }
}

/// id -> base_status for every bead in the git-tracked checkpoint snapshot.
///
/// `beads.db` is local-only, but bead-rs republishes `.beads/checkpoint/`
/// after every committed mutation, so the checkpoint is the committed ground
/// truth a test can read -- including in a clean extraction, where no
/// database exists.
fn checkpoint_bead_statuses() -> BTreeMap<String, String> {
    let checkpoint = audited_checkout().join(".beads/checkpoint");
    let current: serde_json::Value = serde_json::from_str(
        &fs::read_to_string(checkpoint.join("current.json"))
            .expect(".beads/checkpoint/current.json should exist (it is git-tracked)"),
    )
    .expect("checkpoint current.json should parse");
    let active_root = current["active_root"]["path"]
        .as_str()
        .expect("current.json should name its active snapshot object")
        .to_owned();
    let object = fs::read_to_string(checkpoint.join(active_root))
        .expect("the active checkpoint object should exist beside current.json");
    let mut statuses = BTreeMap::new();
    for line in object.lines() {
        if line.trim().is_empty() {
            continue;
        }
        let record: serde_json::Value =
            serde_json::from_str(line).expect("checkpoint record should parse");
        if let Some(issue) = record.get("issue") {
            statuses.insert(
                issue["id"]
                    .as_str()
                    .expect("checkpoint issue has an id")
                    .to_owned(),
                issue["base_status"]
                    .as_str()
                    .expect("checkpoint issue has a base_status")
                    .to_owned(),
            );
        }
    }
    assert!(
        statuses.len() > 100,
        "the checkpoint snapshot should carry the full bead store, got {} issues",
        statuses.len()
    );
    statuses
}

/// A char that keeps a word together; the hyphen matters, so "fail-closed"
/// never reads as the standalone word "closed".
fn is_word_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_' || c == '-'
}

/// Offsets of `word` in `text` where it stands alone -- not inside
/// "fail-closed", "openbao", "opens", or similar.
fn boundary_matches(text: &str, word: &str) -> Vec<usize> {
    let mut hits = Vec::new();
    let mut from = 0;
    while let Some(rel) = text[from..].find(word) {
        let start = from + rel;
        let end = start + word.len();
        let before = text[..start].chars().next_back();
        let after = text[end..].chars().next();
        let standalone =
            before.is_none_or(|c| !is_word_char(c)) && after.is_none_or(|c| !is_word_char(c));
        if standalone {
            hits.push(start);
        }
        from = end;
    }
    hits
}

/// The word ending just before `at`, skipping punctuation -- used to tell
/// behavioral prose ("fails open while the guard's reliability is unproven")
/// from a status claim.
fn word_before(text: &str, at: usize) -> &str {
    let trimmed = text[..at].trim_end_matches(|c: char| !c.is_alphanumeric());
    let start = trimmed
        .char_indices()
        .rev()
        .take_while(|(_, c)| c.is_alphanumeric())
        .last()
        .map(|(i, _)| i)
        .unwrap_or(trimmed.len());
    &trimmed[start..]
}

/// Strip a historical parenthetical -- "(open when this paragraph was
/// reconciled 2026-09-14)" -- so a narrative reference to an older status
/// does not shadow the claim that follows it ("closed the same day").
fn strip_historical_qualifiers(window: &str) -> &str {
    let mut rest = window;
    loop {
        let after_ticks = rest.trim_start_matches(['`', ' ', ',']);
        let Some(without_paren) = after_ticks.strip_prefix('(') else {
            return rest;
        };
        let historical =
            without_paren.starts_with("open ") || without_paren.starts_with("in progress");
        match (historical, without_paren.find(')')) {
            (true, Some(close)) => rest = &without_paren[close + 1..],
            _ => return rest,
        }
    }
}

/// The status claim plan.md attaches to a bead id, read from the text right
/// after it.
///
/// Only tight, adjacent phrasings count -- "(`irrevers-x`, open)",
/// "`irrevers-x` closed", "(`irrevers-x`, in progress)" -- because the plan's
/// prose also says "fails open" and "fail-closed" about *behavior*, and a
/// loose scan would pin fiction as fact. Guards, in order:
/// - a possessive ("`irrevers-x`'s discussion") is a reference, not a claim;
/// - a historical parenthetical is skipped (see
///   [`strip_historical_qualifiers`]);
/// - an "open" preceded by "fail"/"fails" is behavioral prose, not a status;
/// - "open as `irrevers-x`" is the one claim-before-the-id form the plan
///   uses ("still open as `irrevers-6b4ded56`").
fn claimed_status(window: &str) -> Option<ClaimedStatus> {
    let rest = strip_historical_qualifiers(window);
    let mut candidates: Vec<(usize, ClaimedStatus)> = Vec::new();
    for (word, claim) in [
        ("closed", ClaimedStatus::Closed),
        ("in progress", ClaimedStatus::InProgress),
        ("open", ClaimedStatus::Open),
    ] {
        for at in boundary_matches(rest, word) {
            candidates.push((at, claim));
        }
    }
    candidates.sort_by_key(|(at, _)| *at);
    for (at, claim) in candidates {
        if claim == ClaimedStatus::Open && matches!(word_before(rest, at), "fail" | "fails") {
            continue;
        }
        return Some(claim);
    }
    None
}

/// The beads whose status the 2026-09 reconciliations pinned, with the
/// claim those reconciled paragraphs assert. Each must stay present in
/// plan.md with exactly this claim, and the claim must hold against the
/// checkpoint.
///
/// - `irrevers-6b4ded56` -- first host installation (closed 2026-09-18;
///   codinghome enforcement cutover deliberately retained Generation 0
///   FailOpen, so the paragraph's fail-closed point still stands; the
///   bead title's `ex44` naming is stale — codinghome replaced that
///   decommissioned host)
/// - `irrevers-beee1069` -- builder image shipped /etc/icg world-writable
///   (closed 2026-09-18; reconciled 2026-09-19)
/// - `irrevers-c36bba27` -- fixed builder image publication (closed
///   2026-09-18; reconciled 2026-09-19)
/// - `irrevers-92e6e55c` -- lock/policy lineage umbrella (closed 2026-09-14)
/// - `irrevers-84b36e47` -- end-to-end release verification (closed
///   2026-09-06)
const GUARDED_BEAD_CLAIMS: [(&str, ClaimedStatus); 5] = [
    ("irrevers-6b4ded56", ClaimedStatus::Closed),
    ("irrevers-beee1069", ClaimedStatus::Closed),
    ("irrevers-c36bba27", ClaimedStatus::Closed),
    ("irrevers-92e6e55c", ClaimedStatus::Closed),
    ("irrevers-84b36e47", ClaimedStatus::Closed),
];

/// Every status claim plan.md makes about any bead must hold against the
/// checkpoint, and the claims this reconciliation pinned must survive both
/// in the checkpoint and as explicit text in the plan.
#[test]
fn plan_bead_status_claims_match_the_bead_checkpoint() {
    let plan = flattened(&repo_relative("docs/plan/plan.md"));
    let statuses = checkpoint_bead_statuses();

    // id -> the claim (if any) each mention carries.
    let mut claims: BTreeMap<String, Vec<Option<ClaimedStatus>>> = BTreeMap::new();
    let mut from = 0;
    while let Some(rel) = plan[from..].find("irrevers-") {
        let id_start = from + rel;
        let id_end = id_start + "irrevers-".len() + 8;
        from = id_end;
        let Some(id) = plan.get(id_start..id_end) else {
            continue;
        };
        if !id["irrevers-".len()..]
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit())
        {
            continue;
        }
        // a ninth hex digit means this is some longer id, not a bead id
        if plan[id_end..]
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_hexdigit())
        {
            continue;
        }
        let after = &plan[id_end..];
        let claim = if after.starts_with("'s") || after.starts_with("`'s") {
            None
        } else {
            // .get() so a window edge landing inside a multibyte char
            // truncates to the remainder instead of panicking
            let window = plan
                .get(id_end..(id_end + 110).min(plan.len()))
                .unwrap_or(after);
            claimed_status(window).or_else(|| {
                let before = &plan[..id_start];
                let cut = before
                    .char_indices()
                    .find(|(at, _)| before.len() - at <= 30)
                    .map(|(at, _)| at)
                    .unwrap_or(0);
                let tail = &before[cut..];
                (tail.ends_with("open as `") || tail.ends_with("open as "))
                    .then_some(ClaimedStatus::Open)
            })
        };
        claims.entry(id.to_owned()).or_default().push(claim);
    }
    assert!(
        claims.len() > 40,
        "plan.md should still name its beads, found {}",
        claims.len()
    );

    for (id, mentions) in &claims {
        let base = statuses.get(id).unwrap_or_else(|| {
            panic!(
                "plan.md names {id}, which the bead checkpoint does not know; \
                 the plan must not reference beads outside the store"
            )
        });
        for claim in mentions.iter().flatten() {
            assert!(
                claim.holds_against(base),
                "plan.md claims {id} is {claim:?}, but the bead checkpoint says \
                 {base:?}; reconcile the plan paragraph or the bead -- one of \
                 the two has drifted"
            );
        }
    }

    for (id, expected) in GUARDED_BEAD_CLAIMS {
        let mentions = claims.get(id).unwrap_or_else(|| {
            panic!(
                "plan.md must keep stating the status of {id}; the reconciled \
                 paragraph this guard pins has been removed"
            )
        });
        let stated: Vec<ClaimedStatus> = mentions.iter().flatten().copied().collect();
        assert!(
            !stated.is_empty(),
            "plan.md mentions {id} but no longer states its status; the \
             reconciled paragraph must keep the explicit claim"
        );
        for claim in &stated {
            assert_eq!(
                claim, &expected,
                "plan.md must keep claiming {id} is {expected:?} (as \
                 reconciled); it now says {claim:?}"
            );
        }
        let base = statuses.get(id).expect("guarded bead is in the checkpoint");
        assert!(
            expected.holds_against(base),
            "the guard expects {id} to be {expected:?} but the checkpoint now \
             says {base:?}; the reconciliation itself has drifted -- update \
             the guard together with plan.md"
        );
    }
}
