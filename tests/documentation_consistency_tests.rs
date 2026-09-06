//! Documentation/CLI consistency assertions.
//!
//! The 2026-08-25 plan/artifact audit found systematic drift between the
//! docs and the shipped surface: phase checkboxes left open after their
//! beads closed, `bf` described as canonical after the bead-rs cutover,
//! troubleshooting claiming the PATH-wrapper was unimplemented, and the
//! `icg install` help naming a default directory the code no longer uses.
//! These tests pin the reconciled state so each class of drift fails a
//! build instead of silently recurring.

use std::fs;
use std::path::Path;
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

fn repo_relative(relative: &str) -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join(relative);
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
fn quick_start_makes_no_kubectl_coverage_claim() {
    let doc = quick_start();

    // kubectl is deliberately not a pack (plan.md "Explicitly not
    // attempted"): mutating-verb blocking stays org-rule-guard.py's job.
    // These exact claims are what the 2026-08-25 audit found in the wild.
    for stale in ["kubectl delete pvc", "kubectl-delete-pvc", "**Kubernetes**"] {
        assert!(
            !doc.contains(stale),
            "quick-start.md must not claim kubectl coverage ({stale:?}): \
             kubectl is explicitly not a pack"
        );
    }

    // The doc should say the quiet part out loud: who owns kubectl instead.
    assert!(
        doc.contains("org-rule-guard.py"),
        "quick-start.md should state that kubectl mutation blocking stays \
         with org-rule-guard.py"
    );
}

#[test]
fn quick_start_pack_inventory_matches_the_shipped_packs() {
    let doc = quick_start();

    let packs_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("packs");
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
    let packs_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("packs");

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
            "v0.1.0 is an orphaned tag; the first real release is v0.1.1",
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
    let packs_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("packs");
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
    let packs_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("packs");
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
