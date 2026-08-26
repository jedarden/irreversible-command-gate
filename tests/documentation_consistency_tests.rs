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
    for stale in [
        "kubectl delete pvc",
        "kubectl-delete-pvc",
        "**Kubernetes**",
    ] {
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
    assert!(!shipped_ids.is_empty(), "packs/ directory should not be empty");
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
    for stale in ["vault (", "Pack: vault", "vault-kv-destroy", "(12 patterns)"] {
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
