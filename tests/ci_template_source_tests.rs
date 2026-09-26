//! The Argo WorkflowTemplates that run this repo's CI have exactly one
//! source of truth: jedarden/declarative-config
//! (`k8s/iad-ci/argo-workflows/`). For a long stretch this repo also carried
//! template copies under containers/argo-guarded-builder/ (template names
//! `icg-ci-guarded` and `icg-guarded-ci`) that were deployed nowhere -- the
//! cluster runs `icg-ci` and `icg-guarded-builder`, both synced by ArgoCD
//! from declarative-config -- while docs/operators/argo-integration-guide.md
//! told operators to apply the in-repo copy. A contributor following that
//! guide edited a fossil and expected CI behavior to change: a documented
//! workflow whose stated artifact was not the real one. The copies are
//! deleted and the docs now name declarative-config (irrevers-fc96ecad);
//! these tests keep it that way.
//!
//! Three guards:
//!
//! 1. no undeclared in-repo WorkflowTemplate YAML may reappear;
//! 2. a copy that *is* declared (in [`DECLARED_MIRRORS`]) must carry the
//!    mirror header and, whenever a declarative-config checkout is
//!    reachable, must match its upstream byte-for-byte -- the in-repo shape
//!    of the ArgoCD selfHeal rule, since nothing here reverts a drifting
//!    copy automatically;
//! 3. the operator-facing docs must keep pointing at declarative-config and
//!    must not resurrect the deleted fossil filenames or dead template names.

use std::fs;
use std::path::{Path, PathBuf};

/// A deliberate in-repo mirror of a declarative-config template.
///
/// Empty by contract: template work happens in declarative-config and
/// ArgoCD ships it. If a mirror is ever genuinely warranted (a
/// release-pinned snapshot, say), declare it here -- this repo's path
/// first, the declarative-config path second -- and keep it honest: it must
/// open with [`MIRROR_HEADER`] and must equal its upstream whenever a
/// declarative-config checkout is reachable. The containers/
/// argo-guarded-builder/ fossils are why undeclared copies fail
/// [`no_in_repo_workflowtemplate_copy_exists`].
const DECLARED_MIRRORS: &[(&str, &str)] = &[];

/// The first line every declared mirror must carry, so a reader can tell a
/// mirror from an authoritative template at a glance.
const MIRROR_HEADER: &str =
    "# READ-ONLY MIRROR of jedarden/declarative-config -- edit the upstream file, not this copy:";

/// Where the live templates live inside the declarative-config checkout.
const LIVE_TEMPLATES_DIR: &str = "k8s/iad-ci/argo-workflows";

/// The operator-facing docs a contributor following the deleted fossils
/// would have been misled by. All three must keep naming the source of
/// truth and must never name the fossil files or dead template names again.
const OPERATOR_DOCS: &[&str] = &[
    "docs/operators/argo-integration-guide.md",
    "docs/monitoring-deployment-guide.md",
    "containers/argo-guarded-builder/README.md",
];

/// The deleted filenames and the dead template names they carried. Searched
/// as substrings, so a doc mentioning `icg-guarded-ci` in any spelling of
/// the old fossil fails.
const FOSSIL_NAMES: &[&str] = &[
    "icg-ci-guarded-workflowtemplate.yml",
    "icg-guarded-ci-workflowtemplate.yml",
    "icg-ci-guarded",
    "icg-guarded-ci",
];

/// The checkout this run audits.
///
/// `env!("CARGO_MANIFEST_DIR")` is baked in at compile time, and this box's
/// global cargo config points `target-dir` at a fleet-shared directory, so
/// cargo reuses a test binary built by a *different* checkout of this repo
/// whenever its fingerprint looks fresh; the baked path then names some
/// other tree and the reads below audit the wrong files. cargo runs test
/// binaries with the package root as the working directory, so prefer the
/// runtime cwd; fall back to the baked path only when it does not name a
/// checkout. Same guard as `documentation_consistency_tests.rs`.
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

/// Locate the declarative-config checkout, or `None` when none is
/// reachable. Discovery: `ICG_DECLARATIVE_CONFIG` (explicit override), else
/// the documented fleet layout -- declarative-config checked out beside
/// this one (`/home/coding/declarative-config` on codinghome). The icg-ci
/// build pod clones only this repo, so the drift and upstream checks skip
/// loudly there rather than fail on an artifact they cannot see.
fn declarative_config_root() -> Option<PathBuf> {
    let root = match std::env::var("ICG_DECLARATIVE_CONFIG") {
        Ok(dir) if !dir.is_empty() => PathBuf::from(dir),
        _ => audited_checkout().parent()?.join("declarative-config"),
    };
    root.is_dir().then_some(root)
}

/// Every in-repo `*.yml`/`*.yaml` that declares `kind: WorkflowTemplate` --
/// the shape a template copy takes if one ever reappears.
///
/// Operator docs may quote `kind: WorkflowTemplate` inside fenced examples,
/// but those sit in `.md` files; only YAML files count, so an example can
/// never trip this scan and a real copy can never hide behind it.
fn in_repo_template_yamls(dir: &Path) -> Vec<PathBuf> {
    let mut found = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let name = path
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or_default();
            if path.is_dir() {
                // Skip the checkout's own plumbing; none of it carries
                // template YAML, and `.beads` objects can quote whole
                // historical files.
                if !matches!(name, ".git" | "target" | ".beads" | "node_modules") {
                    stack.push(path);
                }
            } else if matches!(
                path.extension().and_then(|ext| ext.to_str()),
                Some("yml") | Some("yaml")
            ) && fs::read_to_string(&path)
                .is_ok_and(|text| text.contains("kind: WorkflowTemplate"))
            {
                found.push(path);
            }
        }
    }
    found.sort();
    found
}

/// The mirror contract's body extraction: everything after the header line.
/// Shared by the drift check and pinned by
/// [`mirror_body_extraction_strips_only_the_header`] so the check cannot
/// rot into comparing the wrong span.
fn mirror_body(mirror: &str) -> &str {
    mirror
        .strip_prefix(MIRROR_HEADER)
        .unwrap_or(mirror)
        .trim_start_matches('\n')
}

#[test]
fn no_in_repo_workflowtemplate_copy_exists() {
    let checkout = audited_checkout();
    let undeclared: Vec<PathBuf> = in_repo_template_yamls(&checkout)
        .into_iter()
        .filter(|path| {
            let relative = path.strip_prefix(&checkout).unwrap_or(path);
            !DECLARED_MIRRORS
                .iter()
                .any(|(repo_path, _)| Path::new(repo_path) == relative)
        })
        .collect();

    assert!(
        undeclared.is_empty(),
        "undeclared in-repo WorkflowTemplate YAML found at {undeclared:?}.\n\
         The templates that run this repo's CI live in jedarden/declarative-config \
         ({LIVE_TEMPLATES_DIR}/) and reach iad-ci through ArgoCD; an in-repo copy is \
         a fossil a contributor will edit expecting CI behavior to change -- the \
         containers/argo-guarded-builder/ copies were deleted for exactly that \
         (irrevers-fc96ecad). Delete the copy, or declare it in DECLARED_MIRRORS \
         with the mirror header and keep it byte-identical to its upstream."
    );
}

#[test]
fn declared_mirrors_carry_the_header_and_match_upstream() {
    if DECLARED_MIRRORS.is_empty() {
        // Nothing declared: the contract has nothing to bite on until a
        // mirror exists. [`mirror_body_extraction_strips_only_the_header`]
        // keeps the extraction honest in the meantime.
        return;
    }
    let root = declarative_config_root();
    for (repo_path, upstream_path) in DECLARED_MIRRORS {
        let mirror = read_repo_file(repo_path);
        assert!(
            mirror.starts_with(MIRROR_HEADER),
            "{repo_path} is declared as a mirror but does not open with the \
             mirror header -- a reader cannot tell it is not the \
             authoritative template"
        );

        let Some(root) = &root else {
            println!("SKIP: no declarative-config checkout found; cannot drift-check {repo_path}");
            continue;
        };
        let upstream = fs::read_to_string(root.join(upstream_path)).unwrap_or_else(|e| {
            panic!("should read declared upstream {upstream_path} from the declarative-config checkout: {e}")
        });
        assert_eq!(
            mirror_body(&mirror),
            upstream,
            "{repo_path} has drifted from its declared upstream \
             {upstream_path} in jedarden/declarative-config. A mirror that \
             silently diverges is the fossil problem again with extra steps; \
             re-copy the upstream (header stays on top) or delete the mirror."
        );
    }
}

/// The upstream files the operator docs point at must actually be there, so
/// a rename in declarative-config fails here instead of leaving every doc
/// pointer aimed at a missing file.
#[test]
fn the_declared_upstream_templates_exist_where_the_docs_point() {
    let Some(root) = declarative_config_root() else {
        println!(
            "SKIP: no declarative-config checkout found (set ICG_DECLARATIVE_CONFIG \
             or have jedarden/declarative-config checked out beside this repo); \
             cannot verify the upstream template inventory from here"
        );
        return;
    };
    for filename in [
        "icg-ci-workflowtemplate.yml",
        "icg-guarded-builder-workflowtemplate.yml",
    ] {
        let content = fs::read_to_string(root.join(LIVE_TEMPLATES_DIR).join(filename))
            .unwrap_or_else(|e| {
                panic!("should read {filename} from the declarative-config checkout: {e}")
            });
        assert!(
            content.contains("kind: WorkflowTemplate"),
            "{filename} should declare a WorkflowTemplate; the operator docs \
             point at it as the live template"
        );
    }
}

#[test]
fn operator_docs_name_declarative_config_as_the_single_template_source() {
    for doc in OPERATOR_DOCS {
        let text = read_repo_file(doc);
        assert!(
            text.contains("declarative-config"),
            "{doc} must name jedarden/declarative-config as the single source \
             of truth for the CI WorkflowTemplates"
        );
        for fossil in FOSSIL_NAMES {
            assert!(
                !text.contains(fossil),
                "{doc} still references '{fossil}' -- a filename (or dead \
                 template name) deleted with the fossil copies \
                 (irrevers-fc96ecad). Point the reader at jedarden/\
                 declarative-config ({LIVE_TEMPLATES_DIR}/) instead."
            );
        }
    }

    // The guide is where the documented workflow lived; it must say where
    // the templates actually live, not just avoid the old names.
    let guide = read_repo_file(OPERATOR_DOCS[0]);
    assert!(
        guide.contains(LIVE_TEMPLATES_DIR),
        "{} must carry the upstream path ({LIVE_TEMPLATES_DIR}/), not just \
         the repository name",
        OPERATOR_DOCS[0]
    );
}

#[test]
fn mirror_body_extraction_strips_only_the_header() {
    let upstream = "apiVersion: argoproj.io/v1alpha1\nkind: WorkflowTemplate\n";
    let mirror = format!("{MIRROR_HEADER}\n{upstream}");
    assert!(mirror.starts_with(MIRROR_HEADER));
    assert_eq!(
        mirror_body(&mirror),
        upstream,
        "mirror body extraction must leave exactly the upstream bytes"
    );
    // A file without the header is compared as-is rather than silently
    // mis-sliced (the drift check asserts the header first; this is the
    // extraction's own contract).
    assert_eq!(mirror_body(upstream), upstream);
}
