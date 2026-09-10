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

/// Human-readable explanation for why a `.github/workflows/**` path is
/// protected, shared by [`detect`] and any caller that needs the same wording
/// without going through the detection call.
pub const PROTECTED_REASON: &str = "Writes to .github/workflows/ are blocked: workflow \
    definitions grant arbitrary CI privileges and must not be modified by an automated \
    write/edit. Ask a human maintainer to make this change via a reviewed pull request \
    instead.";

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
    fn matches_relative_path() {
        assert!(is_github_workflows_path(".github/workflows/ci.yml"));
    }

    #[test]
    fn matches_nested_relative_path() {
        assert!(is_github_workflows_path(
            "repo/.github/workflows/deploy.yml"
        ));
    }

    #[test]
    fn matches_absolute_path() {
        assert!(is_github_workflows_path(
            "/home/user/project/.github/workflows/ci.yml"
        ));
    }

    #[test]
    fn matches_dot_slash_prefixed_path() {
        assert!(is_github_workflows_path("./.github/workflows/ci.yml"));
    }

    #[test]
    fn matches_the_workflows_directory_itself() {
        assert!(is_github_workflows_path(".github/workflows"));
    }

    #[test]
    fn matches_path_with_parent_dir_segment_that_normalizes_into_it() {
        assert!(is_github_workflows_path("src/../.github/workflows/ci.yml"));
    }

    #[test]
    fn matches_deeply_nested_file_under_workflows() {
        assert!(is_github_workflows_path(
            ".github/workflows/actions/composite/action.yml"
        ));
    }

    #[test]
    fn does_not_match_substring_outside_dot_github() {
        assert!(!is_github_workflows_path("src/workflows/foo.yml"));
    }

    #[test]
    fn does_not_match_workflows_as_a_top_level_dir() {
        assert!(!is_github_workflows_path("workflows/foo.yml"));
    }

    #[test]
    fn does_not_match_sibling_directory_with_similar_name() {
        assert!(!is_github_workflows_path(".github/workflows-extra/foo.yml"));
        assert!(!is_github_workflows_path(
            ".github/workflows-archive/old.yml"
        ));
        assert!(!is_github_workflows_path(".github/workflows2/foo.yml"));
    }

    #[test]
    fn does_not_match_workflows_substring_in_unrelated_path() {
        assert!(!is_github_workflows_path("docs/my-workflows-notes.md"));
        assert!(!is_github_workflows_path("scripts/workflows_helper.py"));
    }

    #[test]
    fn does_not_match_other_dot_github_subdirectories() {
        assert!(!is_github_workflows_path(".github/ISSUE_TEMPLATE/bug.md"));
        assert!(!is_github_workflows_path(".github/dependabot.yml"));
    }

    #[test]
    fn does_not_match_dot_github_alone() {
        assert!(!is_github_workflows_path(".github"));
    }

    #[test]
    fn does_not_match_empty_string() {
        assert!(!is_github_workflows_path(""));
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
    fn matches_after_leading_parent_dirs_with_nothing_to_pop() {
        assert!(is_github_workflows_path("../../.github/workflows/ci.yml"));
    }

    #[test]
    fn does_not_match_repeated_separators_between_unrelated_components() {
        assert!(!is_github_workflows_path("a//b//c"));
    }

    #[test]
    fn matches_with_repeated_separators_around_the_pair() {
        assert!(is_github_workflows_path(".github//workflows//ci.yml"));
    }

    #[test]
    fn matches_trailing_slash_on_workflows_directory() {
        assert!(is_github_workflows_path(".github/workflows/"));
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
}
