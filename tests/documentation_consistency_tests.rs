//! Documentation/CLI consistency assertions.
//!
//! The 2026-08-25 plan/artifact audit found systematic drift between the
//! docs and the shipped surface: phase checkboxes left open after their
//! beads closed, `bf` described as canonical after the bead-rs cutover,
//! troubleshooting claiming the PATH-wrapper was unimplemented, and the
//! `icg install` help naming a default directory the code no longer uses.
//! These tests pin the reconciled state so each class of drift fails a
//! build instead of silently recurring.

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

    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
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
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
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

    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
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
    let packs_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("packs");

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
