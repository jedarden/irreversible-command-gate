//! Integration tests proving that the release CI gates the actual pack bytes,
//! not static fixtures.
//!
//! These tests mutate real packs and verify that the release stage fails when
//! coverage regressions are introduced.

use std::fs;
use std::path::{Path, PathBuf};
use tempfile::TempDir;

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join(name)
}

fn run_icg(args: &[&str]) -> std::process::Output {
    std::process::Command::new(env!("CARGO_BIN_EXE_icg"))
        .args(args)
        .output()
        .expect("icg binary should start")
}

/// Test that mutating a real pack (removing a guarded pattern) causes the
/// coverage-diff gate to fail with exit code 2.
#[test]
fn mutating_real_pack_causes_coverage_gate_to_fail() {
    let temp = TempDir::new().expect("temporary workspace");
    let packs_dir = temp.path().join("packs");
    fs::create_dir(&packs_dir).expect("packs directory should be created");

    // Copy the real git.json pack to the temporary workspace (has tool_keywords)
    let real_git_pack = Path::new(env!("CARGO_MANIFEST_DIR")).join("packs/git.json");
    let mutated_pack = packs_dir.join("git.json");
    fs::copy(&real_git_pack, &mutated_pack).expect("pack should be copied");

    // Load and mutate the pack - remove git-force-push pattern to create a regression
    let pack_content = fs::read_to_string(&mutated_pack).expect("pack should be readable");
    let mut pack_value: serde_json::Value =
        serde_json::from_str(&pack_content).expect("pack should be valid JSON");

    // Remove the git-force-push guarded pattern
    if let Some(guarded_patterns) = pack_value
        .get_mut("guarded_patterns")
        .and_then(|v| v.as_array_mut())
    {
        guarded_patterns.retain(|pattern| {
            pattern
                .get("id")
                .and_then(|id| id.as_str())
                .unwrap_or("")
                != "git-force-push"
        });
    }

    fs::write(&mutated_pack, serde_json::to_string_pretty(&pack_value).expect("pack should serialize"))
        .expect("mutated pack should be written");

    // Build merged pack from the mutated packs (this is what CI does)
    let current_merged = temp.path().join("current-merged.json");
    let build_output = run_icg(&[
        "build-pack",
        "--pack-dir",
        packs_dir.to_str().unwrap(),
        "--output",
        current_merged.to_str().unwrap(),
    ]);
    assert!(
        build_output.status.success(),
        "build-pack should succeed: {}",
        String::from_utf8_lossy(&build_output.stderr)
    );

    // The coverage-diff gate should fail with exit code 2 when there's a regression
    let previous_release = fixture("previous-release.json");
    let diff_output = run_icg(&[
        "coverage-diff",
        previous_release.to_str().unwrap(),
        current_merged.to_str().unwrap(),
    ]);

    // Exit code 2 indicates regressions detected without justification
    assert_eq!(
        diff_output.status.code(),
        Some(2),
        "coverage-diff must fail with exit code 2 when regression is detected: {}",
        String::from_utf8_lossy(&diff_output.stderr)
    );

    let stdout = String::from_utf8(diff_output.stdout).expect("output should be UTF-8");
    assert!(
        stdout.contains("status: regressions_detected"),
        "report should indicate regressions were detected"
    );
    assert!(
        stdout.contains("justification: REQUIRED"),
        "report should require justification for regressions"
    );
}

/// Test that mutating a real pack (widening a safe pattern) causes the
/// coverage-diff gate to fail with exit code 2.
#[test]
fn widening_safe_pattern_in_real_pack_causes_coverage_gate_to_fail() {
    let temp = TempDir::new().expect("temporary workspace");
    let packs_dir = temp.path().join("packs");
    fs::create_dir(&packs_dir).expect("packs directory should be created");

    // Copy the real git.json pack to the temporary workspace
    let real_git_pack = Path::new(env!("CARGO_MANIFEST_DIR")).join("packs/git.json");
    let mutated_pack = packs_dir.join("git.json");
    fs::copy(&real_git_pack, &mutated_pack).expect("pack should be copied");

    // Load and mutate the pack - widen a safe pattern to create a regression
    let pack_content = fs::read_to_string(&mutated_pack).expect("pack should be readable");
    let mut pack_value: serde_json::Value =
        serde_json::from_str(&pack_content).expect("pack should be valid JSON");

    // Widen safe-git-status from "^git status" to ".*" (catch-all)
    if let Some(safe_patterns) = pack_value.get_mut("safe_patterns").and_then(|v| v.as_array_mut())
    {
        for pattern in safe_patterns.iter_mut() {
            if pattern
                .get("id")
                .and_then(|id| id.as_str())
                .unwrap_or("")
                == "safe-git-status"
            {
                if let Some(regex) = pattern.get_mut("regex") {
                    *regex = serde_json::json!(".*");
                }
            }
        }
    }

    fs::write(&mutated_pack, serde_json::to_string_pretty(&pack_value).expect("pack should serialize"))
        .expect("mutated pack should be written");

    // Build merged pack from the mutated packs (this is what CI does)
    let current_merged = temp.path().join("current-merged.json");
    let build_output = run_icg(&[
        "build-pack",
        "--pack-dir",
        packs_dir.to_str().unwrap(),
        "--output",
        current_merged.to_str().unwrap(),
    ]);
    assert!(
        build_output.status.success(),
        "build-pack should succeed: {}",
        String::from_utf8_lossy(&build_output.stderr)
    );

    // The coverage-diff gate should fail with exit code 2 when there's a regression
    let previous_release = fixture("previous-release.json");
    let diff_output = run_icg(&[
        "coverage-diff",
        previous_release.to_str().unwrap(),
        current_merged.to_str().unwrap(),
    ]);

    // Exit code 2 indicates regressions detected without justification
    assert_eq!(
        diff_output.status.code(),
        Some(2),
        "coverage-diff must fail with exit code 2 when regression is detected: {}",
        String::from_utf8_lossy(&diff_output.stderr)
    );

    let stdout = String::from_utf8(diff_output.stdout).expect("output should be UTF-8");
    assert!(
        stdout.contains("status: regressions_detected"),
        "report should indicate regressions were detected"
    );
    assert!(
        stdout.contains("justification: REQUIRED"),
        "report should require justification for regressions"
    );
}

/// Test that pack manifest provides cryptographic verification of release integrity.
#[test]
fn pack_manifest_provides_cryptographic_verification() {
    let temp = TempDir::new().expect("temporary workspace");
    let packs_dir = temp.path().join("packs");
    fs::create_dir(&packs_dir).expect("packs directory should be created");

    // Copy a real pack to the temporary workspace
    let real_storage_class_pack = Path::new(env!("CARGO_MANIFEST_DIR")).join("packs/storage-class.json");
    fs::copy(&real_storage_class_pack, packs_dir.join("storage-class.json"))
        .expect("pack should be copied");

    // Generate manifest
    let manifest_path = temp.path().join("manifest.json");
    let manifest_output = run_icg(&[
        "pack-manifest",
        "--pack-dir",
        packs_dir.to_str().unwrap(),
        "--output",
        manifest_path.to_str().unwrap(),
    ]);
    assert!(
        manifest_output.status.success(),
        "manifest generation should succeed"
    );

    // Verify the manifest contains SHA-256 hashes
    let manifest_content = fs::read_to_string(&manifest_path).expect("manifest should be readable");
    let manifest: serde_json::Value = serde_json::from_str(&manifest_content).expect("manifest should be valid JSON");

    assert_eq!(manifest["version"], "v1");
    assert!(manifest["generated_at"].is_string());

    // Check that packs have SHA-256 hashes
    if let Some(packs) = manifest.get("packs").and_then(|p| p.as_object()) {
        for (_pack_id, entry) in packs {
            let sha256 = entry.get("sha256").and_then(|s| s.as_str());
            assert!(
                sha256.is_some(),
                "pack entry should have SHA-256 hash"
            );
            let hash = sha256.unwrap();
            assert_eq!(hash.len(), 64, "SHA-256 hash should be 64 hex characters");
            assert!(
                hash.chars().all(|c| c.is_ascii_hexdigit()),
                "SHA-256 hash should be hexadecimal"
            );
        }
    }
}

/// Test that mutating a pack after manifest generation causes verification to fail.
#[test]
fn mutating_pack_after_manifest_causes_verification_failure() {
    let temp = TempDir::new().expect("temporary workspace");
    let packs_dir = temp.path().join("packs");
    fs::create_dir(&packs_dir).expect("packs directory should be created");

    // Copy a real pack to the temporary workspace
    let real_openbao_pack = Path::new(env!("CARGO_MANIFEST_DIR")).join("packs/openbao.json");
    let pack_path = packs_dir.join("openbao.json");
    fs::copy(&real_openbao_pack, &pack_path).expect("pack should be copied");

    // Generate manifest
    let manifest_path = temp.path().join("manifest.json");
    let manifest_output = run_icg(&[
        "pack-manifest",
        "--pack-dir",
        packs_dir.to_str().unwrap(),
        "--output",
        manifest_path.to_str().unwrap(),
    ]);
    assert!(
        manifest_output.status.success(),
        "manifest generation should succeed"
    );

    // Mutate the pack after manifest generation
    let mut pack_content = fs::read_to_string(&pack_path).expect("pack should be readable");
    pack_content.push_str("\n  // MUTATED AFTER MANIFEST");
    fs::write(&pack_path, pack_content).expect("mutated pack should be written");

    // Verification should fail
    let verify_output = run_icg(&[
        "pack-manifest",
        "--pack-dir",
        packs_dir.to_str().unwrap(),
        "--verify",
        manifest_path.to_str().unwrap(),
    ]);

    assert!(
        !verify_output.status.success(),
        "verification must fail when pack is mutated after manifest generation: {}",
        String::from_utf8_lossy(&verify_output.stderr)
    );

    let stderr = String::from_utf8(verify_output.stderr).expect("stderr should be UTF-8");
    assert!(
        stderr.contains("mismatched SHA-256") || stderr.contains("does not match"),
        "verification should report hash mismatch"
    );
}
