//! Pure predicate for detecting paths under `.github/workflows/**`.
//!
//! This is the path-matching half of the `.github/workflows` write guard
//! (parent: writes to workflow definitions can grant a PR arbitrary CI
//! privileges, so the hook front-end wants to catch them before they land).
//! It does no filesystem or environment I/O -- it is a lexical predicate over
//! the path string a Write/Edit tool call or `apply_patch` hunk header
//! reports, so it is safe to call from a hook that must not add network or
//! disk I/O to the fast path.

use std::path::{Component, Path};

/// Does `path` refer to a location under `.github/workflows/`?
///
/// The path is normalized lexically before matching: `.` segments are
/// dropped, `..` segments pop the preceding component (bounded -- a `..`
/// with nothing to pop is simply ignored rather than erroring), and both
/// relative and absolute paths are accepted. A `.github/workflows` pair
/// matches at any depth, so a nested repository checkout or an absolute
/// path both match.
///
/// Matching is on whole path components, not substrings: `src/workflows/x`
/// and `.github/workflows-extra/x` do not match, because neither has a
/// component that is exactly `.github` immediately followed by a component
/// that is exactly `workflows`.
///
/// Component comparison is case-sensitive on case-sensitive filesystems and
/// case-insensitive on OSes whose native filesystem is normally
/// case-insensitive (Windows, macOS), matching how those OSes would actually
/// resolve the two directory names.
///
/// Malformed input (empty string, stray separators, unusual encodings) never
/// panics: it just normalizes to whatever components `Path::components()`
/// produces, and no match is found if that is not `.github/workflows`.
pub fn is_github_workflows_path(path: &str) -> bool {
    let components = normalized_components(path);
    components
        .windows(2)
        .any(|pair| component_eq(&pair[0], ".github") && component_eq(&pair[1], "workflows"))
}

/// The redirect shown when a `.github/workflows/**` write is denied.
///
/// Per the project's redirect policy (`docs/notes/redirect-not-just-block.md`)
/// a denial must be actionable, not just a block: the reason states *why* the
/// write is refused and *what to do instead* concretely enough that the next
/// step needs no research. The sanctioned alternative here is the reviewed
/// channel -- a human maintainer making the change in a reviewed pull request,
/// with CI-pipeline changes landing in the `declarative-config` repository
/// where the Argo Workflows templates live -- not a different spelling of the
/// same direct write.
pub const PROTECTED_REASON: &str = "Writes to .github/workflows/ are blocked: workflow \
    definitions grant arbitrary CI privileges, so an automated write/edit must not \
    modify them. Do not re-attempt this write. Route the change through the reviewed \
    channel instead: a human maintainer makes workflow changes in a reviewed pull \
    request, and CI pipelines run on Argo Workflows in the iad-ci cluster with their \
    templates in the declarative-config repository (k8s/iad-ci/argo-workflows/), so \
    propose the change there rather than writing the file directly.";

/// Path fixtures that must trip the guard, shared by this module's tests and
/// the hook-level integration tests
/// (`tests/github_workflows_hook_integration_tests.rs`), which drive the same
/// table through the compiled `icg hook` binary. Keeping one table means a
/// spelling accepted here is by construction also proven at the hook
/// boundary. Entries are commented with the spelling each one exercises.
pub const GUARDED_PATHS: &[&str] = &[
    // plain relative form
    ".github/workflows/ci.yml",
    // relative form under a checkout/repo prefix
    "repo/.github/workflows/deploy.yml",
    // `./`-prefixed relative form
    "./.github/workflows/ci.yml",
    // absolute form
    "/home/user/project/.github/workflows/ci.yml",
    // the workflows directory itself, no file part
    ".github/workflows",
    // trailing slash on the workflows directory
    ".github/workflows/",
    // `..` segments that normalize into the protected pair
    "src/../.github/workflows/ci.yml",
    // leading `..` with nothing to pop
    "../../.github/workflows/ci.yml",
    // repeated separators around the protected pair
    ".github//workflows//ci.yml",
    // deeply nested file under workflows
    ".github/workflows/actions/composite/action.yml",
];

/// Lookalike path fixtures that must never trip the guard, shared by this
/// module's tests and the hook-level integration tests. From
/// irrevers-61a08562: matching is on whole path components, so a
/// "workflows" substring outside `.github`, a `.github/workflows*` sibling
/// directory, and other `.github` content are all ordinary writable paths.
pub const UNGUARDED_PATHS: &[&str] = &[
    // `workflows` component outside .github
    "src/workflows/foo.yml",
    // `workflows` as a top-level directory
    "workflows/foo.yml",
    // `workflows` substring inside unrelated components
    "docs/my-workflows-notes.md",
    "scripts/workflows_helper.py",
    // `.github` siblings that merely start with `workflows`
    ".github/workflows-extra/foo.yml",
    ".github/workflows-archive/old.yml",
    ".github/workflows2/foo.yml",
    ".github/workflows-internal/provision.yml",
    // a sibling of `.github` that merely ends in `.github`; its `workflows`
    // child is ordinary content
    "my.github/workflows/ci.yml",
    // other `.github` content that is not workflows
    ".github/ISSUE_TEMPLATE/bug.md",
    ".github/dependabot.yml",
    // the `.github` directory itself
    ".github",
];

/// Structured outcome of checking a Write/Edit target path against the
/// `.github/workflows/**` guard.
///
/// This is the shape a downstream redirect-message step is expected to
/// consume, so it is a named enum rather than `Option<String>` or a bare
/// `bool`: a non-match is always the explicit [`Detection::NoMatch`] variant,
/// never `None`/`null` with no further shape, and a match always carries both
/// the exact path string that triggered it (`matched_path`) and a
/// human-readable `reason` -- callers never need to re-derive either from the
/// input path or re-run [`is_github_workflows_path`] to explain the denial.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Detection {
    /// `path` is under `.github/workflows/`.
    Matched {
        /// The exact path string that was checked and matched. This is the
        /// caller-supplied path, not a canonicalized or re-normalized form,
        /// so a redirect message can quote back exactly what the tool call
        /// targeted.
        matched_path: String,
        /// Human-readable explanation of why this path is protected.
        reason: String,
    },
    /// `path` is not under `.github/workflows/`.
    NoMatch,
}

impl Detection {
    /// Does this detection represent a match?
    pub fn is_match(&self) -> bool {
        matches!(self, Detection::Matched { .. })
    }
}

/// Check `path` against the `.github/workflows/**` guard and return a
/// structured [`Detection`] describing the outcome.
///
/// This wraps [`is_github_workflows_path`] -- the underlying predicate is
/// still the single source of truth for the matching logic -- and adds the
/// matched path and reason a redirect-message step needs, so callers that
/// want the structured result don't reimplement it around the bare
/// predicate.
pub fn detect(path: &str) -> Detection {
    if is_github_workflows_path(path) {
        Detection::Matched {
            matched_path: path.to_string(),
            reason: PROTECTED_REASON.to_string(),
        }
    } else {
        Detection::NoMatch
    }
}

/// Lexically normalize `path` into a flat list of its `Normal` components.
///
/// `RootDir` and path prefixes (drive letters, UNC prefixes) only anchor the
/// path; they carry no name to compare against `.github` or `workflows`, so
/// they are dropped rather than represented as empty components.
fn normalized_components(path: &str) -> Vec<String> {
    let mut components: Vec<String> = Vec::new();

    for component in Path::new(path).components() {
        match component {
            Component::Normal(part) => components.push(part.to_string_lossy().into_owned()),
            Component::ParentDir => {
                components.pop();
            }
            Component::CurDir | Component::RootDir | Component::Prefix(_) => {}
        }
    }

    components
}

/// Compare two path components using this OS's native filesystem case
/// sensitivity.
fn component_eq(a: &str, b: &str) -> bool {
    if cfg!(any(target_os = "windows", target_os = "macos")) {
        a.eq_ignore_ascii_case(b)
    } else {
        a == b
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_shared_guarded_fixture_matches() {
        for path in GUARDED_PATHS {
            assert!(
                is_github_workflows_path(path),
                "expected {path:?} to be guarded"
            );
        }
    }

    #[test]
    fn every_shared_unguarded_fixture_is_ignored() {
        for path in UNGUARDED_PATHS {
            assert!(
                !is_github_workflows_path(path),
                "expected {path:?} to stay writable"
            );
        }
    }

    #[test]
    fn does_not_match_empty_string() {
        assert!(!is_github_workflows_path(""));
    }

    /// Case handling must follow the native filesystem, not a fixed rule:
    /// a mixed-case spelling matches exactly where the OS would resolve it
    /// to `.github/workflows` (macOS, Windows) and nowhere else. Asserting
    /// the `cfg!` expectation keeps this test portable across the OSes CI
    /// and dev boxes actually run.
    #[test]
    fn case_handling_follows_the_native_filesystem() {
        let mixed_case = ".GitHub/Workflows/ci.yml";
        let expected = cfg!(any(target_os = "windows", target_os = "macos"));
        assert_eq!(
            is_github_workflows_path(mixed_case),
            expected,
            "mixed-case .GitHub/Workflows/ should match iff the native \
             filesystem is case-insensitive"
        );
    }

    #[test]
    fn does_not_panic_on_unusual_encodings() {
        // Interior NUL bytes, control characters, and non-ASCII bytes must
        // not panic -- they just fail to look like `.github/workflows`.
        assert!(!is_github_workflows_path(
            "\u{0}.github/workflows\u{0}/x\u{7}.yml"
        ));
        assert!(!is_github_workflows_path("日本語/.github/workflowsX"));
        assert!(is_github_workflows_path("日本語/.github/workflows/x.yml"));
    }

    #[test]
    fn does_not_match_only_a_leading_run_of_parent_dirs() {
        assert!(!is_github_workflows_path("../../.."));
    }

    #[test]
    fn does_not_match_repeated_separators_between_unrelated_components() {
        assert!(!is_github_workflows_path("a//b//c"));
    }

    #[test]
    fn detect_returns_matched_with_path_and_reason() {
        match detect(".github/workflows/ci.yml") {
            Detection::Matched {
                matched_path,
                reason,
            } => {
                assert_eq!(matched_path, ".github/workflows/ci.yml");
                assert_eq!(reason, PROTECTED_REASON);
                assert!(!reason.is_empty());
            }
            Detection::NoMatch => panic!("expected Matched for a .github/workflows/ path"),
        }
    }

    #[test]
    fn detect_returns_no_match_for_unrelated_path() {
        assert_eq!(detect("src/workflows/foo.yml"), Detection::NoMatch);
        assert_eq!(
            detect(".github/workflows-extra/foo.yml"),
            Detection::NoMatch
        );
    }

    #[test]
    fn detect_is_match_reflects_variant() {
        assert!(detect(".github/workflows/ci.yml").is_match());
        assert!(!detect("README.md").is_match());
    }

    /// The denial reason is a redirect, not just a block: it must state why
    /// the write is refused and name the sanctioned alternative concretely
    /// enough that the next step needs no research -- the reviewed pull
    /// request channel and the repository where CI templates actually live.
    #[test]
    fn protected_reason_is_an_actionable_redirect_not_just_a_block() {
        // Why the write is blocked.
        assert!(
            PROTECTED_REASON.contains("blocked"),
            "reason should state that the write is blocked: {PROTECTED_REASON}"
        );
        assert!(
            PROTECTED_REASON.contains("CI privileges"),
            "reason should state why the path is protected: {PROTECTED_REASON}"
        );
        // What to do instead.
        assert!(
            PROTECTED_REASON.contains("instead"),
            "reason should mark the alternative with \"instead\": {PROTECTED_REASON}"
        );
        assert!(
            PROTECTED_REASON.contains("reviewed pull request"),
            "reason should name the reviewed channel: {PROTECTED_REASON}"
        );
        assert!(
            PROTECTED_REASON.contains("declarative-config"),
            "reason should point at where CI templates actually live: {PROTECTED_REASON}"
        );
    }
}
