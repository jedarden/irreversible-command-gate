//! The `icg-ci` Codex compatibility matrix must cover the Codex CLI release
//! the adapter's runtime contract is pinned to.
//!
//! The 0.154 narrowing (`c762ed3`: Codex honors `deny` alone, rejects
//! `allow`/`ask`, and makes `updatedInput` unreachable) shipped while the
//! `codex-hook-compatibility` matrix was still frozen at 0.144-0.146 -- CI
//! kept gating releases against Codex versions older than the behavior the
//! adapter encodes, which is exactly the drift the matrix was adopted to
//! catch (irrevers-8b5faeb9's "strongest surviving objection": a pin set
//! nobody refreshes guards nothing). These tests pin the matrix to
//! [`icg::adapter::CODEX_RUNTIME_PIN`] so the next narrowing or pin bump
//! fails a build instead of shipping unnoticed (irrevers-048ce4f8).
//!
//! These tests read the DEPLOYED template. The copies this file used to gate
//! under containers/argo-guarded-builder/ were undeployed fossils (template
//! name `icg-ci-guarded`; iad-ci runs `icg-ci`, synced by ArgoCD from
//! jedarden/declarative-config), so the gate kept passing while the
//! deployed matrix lacked the pin -- the exact drift it was written to
//! catch (irrevers-ff4aad93). The fossils are deleted; the matrix is now
//! read from the declared upstream, skipping loudly where no
//! declarative-config checkout is reachable (the icg-ci build pod clones
//! only this repo) -- irrevers-fc96ecad. The build-pod-shaped network gate
//! on the deployed manifest remains irrevers-ff4aad93's job.

use std::fs;
use std::path::PathBuf;

/// The checkout this run audits.
///
/// `env!("CARGO_MANIFEST_DIR")` is baked in at compile time, and this box's
/// global cargo config points `target-dir` at a fleet-shared directory, so
/// cargo reuses a test binary built by a *different* checkout of this repo
/// whenever its fingerprint looks fresh; the baked path then names some
/// other tree and the reads below audit the wrong files. cargo runs test
/// binaries with the package root as the working directory, so prefer the
/// runtime cwd; fall back to the baked path only when it does not name a
/// checkout (the binary invoked by hand from an unrelated directory). Same
/// guard as `documentation_consistency_tests.rs`.
fn audited_checkout() -> PathBuf {
    if let Ok(cwd) = std::env::current_dir() {
        if cwd.join("Cargo.toml").exists() {
            return cwd;
        }
    }
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn read_repo_file(relative: &str) -> String {
    let path = audited_checkout().join(relative);
    fs::read_to_string(&path).unwrap_or_else(|e| {
        panic!(
            "should read {} from the audited checkout: {e}",
            path.display()
        )
    })
}

/// The live `icg-ci` WorkflowTemplate, relative to the declarative-config
/// checkout that is its single source of truth. This repo carries no
/// template copy: the ones under containers/argo-guarded-builder/ were
/// fossils (names `icg-ci-guarded` / `icg-guarded-ci`, deployed nowhere
/// while iad-ci runs `icg-ci`), deleted by irrevers-fc96ecad.
const LIVE_ICG_CI_TEMPLATE: &str = "k8s/iad-ci/argo-workflows/icg-ci-workflowtemplate.yml";

/// Read the live template from the declarative-config checkout, or `None`
/// when no checkout is reachable.
///
/// Discovery: `ICG_DECLARATIVE_CONFIG` (explicit override), else the
/// documented fleet layout -- declarative-config checked out beside this
/// one (`/home/coding/declarative-config` on codinghome). The icg-ci build
/// pod clones only this repo and no test may touch the network, so a run
/// without the sibling checkout has nothing honest to assert on: the
/// callers skip loudly rather than gate on a copy.
fn live_icg_ci_template() -> Option<String> {
    let root = match std::env::var("ICG_DECLARATIVE_CONFIG") {
        Ok(dir) if !dir.is_empty() => PathBuf::from(dir),
        _ => audited_checkout().parent()?.join("declarative-config"),
    };
    fs::read_to_string(root.join(LIVE_ICG_CI_TEMPLATE)).ok()
}

fn skip_message() -> String {
    format!(
        "SKIP: no declarative-config checkout found (set ICG_DECLARATIVE_CONFIG \
         or have jedarden/declarative-config checked out beside this repo); \
         cannot assert on the live {LIVE_ICG_CI_TEMPLATE} from here"
    )
}

/// The `withItems` codex versions of the `codex-hook-compatibility` step in
/// the guarded `icg-ci` WorkflowTemplate.
///
/// Hand-extracted rather than pulled in through a YAML dependency: the
/// step's shape is fixed and the block is bounded by YAML indentation --
/// only lines indented deeper than the `withItems:` key (skipping blanks
/// and comments) are its items, so a reformat fails these tests loudly
/// instead of parsing the wrong span.
fn codex_matrix_versions(template: &str) -> Vec<String> {
    let lines: Vec<&str> = template.lines().collect();

    let step = lines
        .iter()
        .position(|line| {
            line.trim_end()
                .ends_with("- name: codex-hook-compatibility")
        })
        .expect("workflow template must declare the codex-hook-compatibility step");

    let with_items = lines[step..]
        .iter()
        .position(|line| line.trim() == "withItems:")
        .map(|offset| step + offset)
        .expect("codex-hook-compatibility step must carry the version matrix in withItems");

    let indent = lines[with_items].len() - lines[with_items].trim_start().len();

    let mut versions = Vec::new();
    for line in &lines[with_items + 1..] {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue; // comments may sit between withItems: and its items
        }
        let line_indent = line.len() - line.trim_start().len();
        if line_indent <= indent {
            break; // dedent: the block (or the step) ended
        }
        let Some(item) = trimmed.strip_prefix("- ") else {
            break;
        };
        let version = item.trim().trim_matches('"');
        assert!(
            !version.is_empty(),
            "empty codex matrix item under withItems"
        );
        versions.push(version.to_string());
    }

    assert!(
        !versions.is_empty(),
        "codex-hook-compatibility withItems parsed to an empty matrix; \
         the extraction in this test needs updating alongside the template"
    );
    versions
}

fn is_x_y_z(version: &str) -> bool {
    let parts: Vec<&str> = version.split('.').collect();
    parts.len() == 3
        && parts
            .iter()
            .all(|part| !part.is_empty() && part.bytes().all(|byte| byte.is_ascii_digit()))
}

fn numeric(version: &str) -> (u64, u64, u64) {
    let mut parts = version.split('.');
    let major = parts.next().unwrap_or_default();
    let minor = parts.next().unwrap_or_default();
    let patch = parts.next().unwrap_or_default();
    (
        major.parse().unwrap_or(0),
        minor.parse().unwrap_or(0),
        patch.parse().unwrap_or(0),
    )
}

/// The failure this file exists for: the runtime contract moved to 0.154
/// while the matrix kept gating 0.144-0.146 only.
#[test]
fn the_codex_compatibility_matrix_covers_the_runtime_pin() {
    let Some(template) = live_icg_ci_template() else {
        println!("{}", skip_message());
        return;
    };
    let versions = codex_matrix_versions(&template);

    assert!(
        versions.contains(&icg::adapter::CODEX_RUNTIME_PIN.to_string()),
        "the deployed icg-ci template's codex-hook-compatibility matrix \
         {versions:?} must include CODEX_RUNTIME_PIN ({}) -- the Codex \
         release whose runtime semantics the adapter encodes. Fix it in \
         jedarden/declarative-config ({LIVE_ICG_CI_TEMPLATE}); update the \
         pin's entry (and re-verify its rejections) whenever the adapter \
         contract moves.",
        icg::adapter::CODEX_RUNTIME_PIN
    );
}

/// Keep the matrix itself legible: every entry a real x.y.z release, listed
/// in ascending order so the next refresh appends instead of scattering.
#[test]
fn matrix_entries_are_well_formed_and_ascending() {
    let Some(template) = live_icg_ci_template() else {
        println!("{}", skip_message());
        return;
    };
    let versions = codex_matrix_versions(&template);

    for version in &versions {
        assert!(
            is_x_y_z(version),
            "codex matrix entry '{version}' is not an x.y.z release"
        );
    }

    let mut numeric_versions: Vec<(u64, u64, u64)> = versions.iter().map(|v| numeric(v)).collect();
    let sorted = numeric_versions.clone();
    numeric_versions.sort();
    assert_eq!(
        numeric_versions, sorted,
        "codex matrix {versions:?} must be listed in ascending version order"
    );
}

/// The pin is a contract, and the contract doc is where it is spelled out
/// (§6.2 quotes the pinned binary's rejections). If the pin moves, the doc
/// moves with it -- same policy as the rest of the docs-vs-reality guards.
#[test]
fn the_adapter_contract_doc_cites_the_same_pin() {
    let doc = read_repo_file("docs/notes/harness-adapter-contract.md");
    let citation = format!("Codex CLI {}", icg::adapter::CODEX_RUNTIME_PIN);
    assert!(
        doc.contains(&citation),
        "harness-adapter-contract.md must cite the runtime pin ('{citation}') \
         alongside the rejections that justify it"
    );
}
