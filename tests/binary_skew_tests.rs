//! The binary-versus-trusted-release skew report.
//!
//! Pins both halves of `icg status`'s **Binary Version** section: the
//! version comparison in [`icg::binary_skew`], and the section as the
//! real binary renders it against a staged trust pointer. The skew is the
//! only signal that a host's executable has drifted from the release its
//! own pointer names — `icg update` advances packs and the pointer, never
//! the root-owned binary — so every wording here is load-bearing for the
//! operator docs that quote it.

use std::path::{Path, PathBuf};
use std::process::Command;

use icg::binary_skew::{assess, skew_summary, status_lines, BinarySkew, ReleaseVersion};
use tempfile::tempdir;

// ---------------------------------------------------------------------------
// ReleaseVersion::parse
// ---------------------------------------------------------------------------

#[test]
fn release_version_parses_the_tag_shapes_the_pointer_carries() {
    let parsed = |r: &str| ReleaseVersion::parse(r).map(|v| v.parts());
    assert_eq!(parsed("v0.1.71"), Some((0, 1, 71)));
    assert_eq!(parsed("0.1.71"), Some((0, 1, 71)));
    assert_eq!(parsed("V1.2.3"), Some((1, 2, 3)));
    assert_eq!(parsed("v0.1"), Some((0, 1, 0)));
    assert_eq!(parsed("1"), Some((1, 0, 0)));
    assert_eq!(parsed("  v0.1.71 "), Some((0, 1, 71)));
}

#[test]
fn release_version_rejects_non_version_references() {
    // A commit SHA, a channel name, a prerelease tag, and other shapes the
    // trust pointer documentation allows are not versions: comparison must
    // report "unknown", never guess.
    let sha = "3f2a9c4b1d8e7f605a4b3c2d1e0f9a8b7c6d5e4f";
    for reference in [
        sha,
        "latest",
        "stable",
        "canary",
        "v0.1.71-rc1",
        "",
        "1.2.3.4",
        "v0.1.x",
        "1 .2",
    ] {
        assert!(
            ReleaseVersion::parse(reference).is_none(),
            "{reference:?} must not parse as a release version"
        );
    }
}

// ---------------------------------------------------------------------------
// assess
// ---------------------------------------------------------------------------

#[test]
fn comparison_is_numeric_not_lexicographic() {
    // 9 < 71 numerically; a string comparison would report the binary as
    // newer than the release it is actually behind.
    assert_eq!(
        assess("0.1.9", Some("v0.1.71")),
        BinarySkew::BinaryOlder {
            binary: ReleaseVersion::parse("0.1.9").unwrap(),
            trusted: ReleaseVersion::parse("v0.1.71").unwrap(),
        }
    );
}

#[test]
fn assess_reports_each_direction() {
    assert_eq!(assess("0.1.71", Some("v0.1.71")), BinarySkew::InSync);
    assert_eq!(assess("v0.1.71", Some("0.1.71")), BinarySkew::InSync);
    assert_eq!(
        assess("0.0.9", Some("v0.1.71")),
        BinarySkew::BinaryOlder {
            binary: ReleaseVersion::parse("0.0.9").unwrap(),
            trusted: ReleaseVersion::parse("v0.1.71").unwrap(),
        }
    );
    assert_eq!(
        assess("0.2.0", Some("v0.1.71")),
        BinarySkew::BinaryNewer {
            binary: ReleaseVersion::parse("0.2.0").unwrap(),
            trusted: ReleaseVersion::parse("v0.1.71").unwrap(),
        }
    );
    assert_eq!(assess("0.1.71", Some("stable")), BinarySkew::NotComparable);
    assert_eq!(assess("0.1.71", None), BinarySkew::NoTrustPointer);
}

// ---------------------------------------------------------------------------
// rendering
// ---------------------------------------------------------------------------

#[test]
fn status_lines_render_the_skew_field_and_the_sanctioned_remedy() {
    let older = status_lines("0.1.63", Some("v0.1.71"));
    let older_text = older.join("\n");
    assert!(older_text.contains("**Running Binary:** v0.1.63"));
    assert!(older_text.contains("**Trusted Release:** `v0.1.71`"));
    assert!(older_text.contains(
        "**Binary Skew:** SKEWED — running binary v0.1.63 is older than \
         trusted release v0.1.71"
    ));
    // The remedy names the documented procedure, and the line makes clear
    // that the updater will never clear the skew on its own.
    assert!(older_text.contains("docs/operators/deployment-guide.md"));
    assert!(older_text.contains("Upgrade the executable from source"));
    assert!(older_text.contains("never upgrades it"));

    let newer = status_lines("0.2.0", Some("v0.1.71")).join("\n");
    assert!(newer.contains("is newer than trusted release v0.1.71"));
    assert!(newer.contains("docs/runbooks/release-cutting.md"));

    let sync = status_lines("0.1.71", Some("v0.1.71")).join("\n");
    assert!(sync.contains(
        "**Binary Skew:** in sync — running binary v0.1.71 matches trusted release v0.1.71"
    ));
    // An in-sync section must not tell the operator to do anything.
    assert!(!sync.contains("deployment-guide.md"));
    assert!(!sync.contains("release-cutting.md"));
}

#[test]
fn status_lines_render_unknown_without_a_comparable_pointer() {
    let sha = "3f2a9c4b1d8e7f605a4b3c2d1e0f9a8b7c6d5e4f";
    let not_comparable = status_lines("0.1.71", Some(sha)).join("\n");
    assert!(not_comparable.contains("**Binary Skew:** unknown — trusted reference"));
    assert!(not_comparable.contains("is not a release version"));

    let none = status_lines("0.1.71", None).join("\n");
    assert!(none.contains("**Trusted Release:** (not configured)"));
    assert!(none.contains("no trust pointer is configured"));
}

#[test]
fn skew_summary_agrees_with_assess_for_every_direction() {
    for (binary, trusted) in [
        ("0.1.71", Some("v0.1.71")),
        ("0.1.9", Some("v0.1.71")),
        ("0.2.0", Some("v0.1.71")),
        ("0.1.71", Some("stable")),
        ("0.1.71", None),
    ] {
        let skew = assess(binary, trusted);
        let summary = skew_summary(&skew, binary, trusted);
        match skew {
            BinarySkew::InSync => assert!(summary.starts_with("in sync")),
            BinarySkew::BinaryOlder { .. } | BinarySkew::BinaryNewer { .. } => {
                assert!(summary.starts_with("SKEWED"))
            }
            BinarySkew::NotComparable | BinarySkew::NoTrustPointer => {
                assert!(summary.starts_with("unknown"))
            }
        }
    }
}

// ---------------------------------------------------------------------------
// the section as `icg status` prints it
// ---------------------------------------------------------------------------

fn write_pointer(dir: &Path, trusted_ref: &str) -> PathBuf {
    let path = dir.join("trust-pointer.json");
    std::fs::write(&path, format!("{{\"trusted_ref\":\"{trusted_ref}\"}}\n"))
        .expect("staging trust pointer should write");
    path
}

fn icg_status(pointer: Option<&Path>) -> (bool, String) {
    let mut command = Command::new(env!("CARGO_BIN_EXE_icg"));
    command.arg("status");
    if let Some(path) = pointer {
        command.arg("--trust-pointer-path").arg(path);
    }
    let output = command.output().expect("icg binary should run");
    (
        output.status.success(),
        String::from_utf8_lossy(&output.stdout).into_owned(),
    )
}

#[test]
fn status_reports_in_sync_against_the_built_version() {
    let dir = tempdir().expect("tempdir");
    let pointer = write_pointer(dir.path(), &format!("v{}", env!("CARGO_PKG_VERSION")));
    let (success, text) = icg_status(Some(&pointer));
    assert!(success, "icg status should succeed");
    assert!(text.contains("## Binary Version"));
    assert!(text.contains(&format!(
        "**Running Binary:** v{}",
        env!("CARGO_PKG_VERSION")
    )));
    assert!(text.contains("**Binary Skew:** in sync — running binary"));
}

#[test]
fn status_reports_skew_when_packs_advance_past_the_binary() {
    let dir = tempdir().expect("tempdir");
    let pointer = write_pointer(dir.path(), "v9.9.9");
    let (success, text) = icg_status(Some(&pointer));
    assert!(success, "icg status should succeed");
    assert!(text.contains(&format!(
        "**Binary Skew:** SKEWED — running binary v{} is older than \
         trusted release v9.9.9",
        env!("CARGO_PKG_VERSION")
    )));
    assert!(text.contains("docs/operators/deployment-guide.md"));
}

#[test]
fn status_reports_the_pointer_lagging_the_binary() {
    let dir = tempdir().expect("tempdir");
    let pointer = write_pointer(dir.path(), "v0.0.1");
    let (success, text) = icg_status(Some(&pointer));
    assert!(success, "icg status should succeed");
    assert!(text.contains("is newer than trusted release v0.0.1"));
    assert!(text.contains("docs/runbooks/release-cutting.md"));
}

#[test]
fn status_reports_a_non_version_reference_as_unknown() {
    let dir = tempdir().expect("tempdir");
    let sha = "3f2a9c4b1d8e7f605a4b3c2d1e0f9a8b7c6d5e4f";
    let pointer = write_pointer(dir.path(), sha);
    let (success, text) = icg_status(Some(&pointer));
    assert!(success, "icg status should succeed");
    assert!(text.contains("is not a release version"));
}

#[test]
fn status_without_a_pointer_reports_unknown() {
    let dir = tempdir().expect("tempdir");
    let missing = dir.path().join("no-pointer-here.json");
    let (success, text) = icg_status(Some(&missing));
    assert!(success, "icg status should succeed");
    assert!(text.contains("**Trusted Release:** (not configured)"));
    assert!(text.contains("no trust pointer is configured"));
}

#[test]
fn binary_version_section_sits_between_the_pointer_and_the_packs() {
    let dir = tempdir().expect("tempdir");
    let pointer = write_pointer(dir.path(), &format!("v{}", env!("CARGO_PKG_VERSION")));
    let (_, text) = icg_status(Some(&pointer));
    let trust = text
        .find("## Trust Pointer")
        .expect("Trust Pointer section");
    let binary = text
        .find("## Binary Version")
        .expect("Binary Version section");
    let packs = text
        .find("## Rule Pack Version")
        .expect("Rule Pack Version section");
    assert!(trust < binary && binary < packs);
}
