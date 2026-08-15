use std::path::PathBuf;

#[test]
fn test_no_regressions_clean_release() {
    let previous = PathBuf::from("tests/fixtures/previous-release.json");
    let current = PathBuf::from("tests/fixtures/current-release-clean.json");

    let diff = icg::coverage::run_coverage_diff(previous, current).unwrap();

    assert!(!diff.has_regressions(), "Clean release should not have regressions");
    assert!(diff.removed_guarded_patterns.is_empty());
    assert!(diff.widened_safe_patterns.is_empty());
    assert!(diff.narrowed_destructive_patterns.is_empty());
}

#[test]
fn test_detects_removed_guarded_patterns() {
    let previous = PathBuf::from("tests/fixtures/previous-release.json");
    let current = PathBuf::from("tests/fixtures/current-release-regression.json");

    let diff = icg::coverage::run_coverage_diff(previous, current).unwrap();

    assert!(diff.has_regressions(), "Should detect regressions");

    // vault-policy-delete was removed
    assert!(diff.removed_guarded_patterns.contains(&"vault-policy-delete".to_string()));
    // vault-secrets-disable was removed
    assert!(diff.removed_guarded_patterns.contains(&"vault-secrets-disable".to_string()));
    assert_eq!(diff.removed_guarded_patterns.len(), 2);
}

#[test]
fn test_detects_widened_safe_patterns() {
    let previous = PathBuf::from("tests/fixtures/previous-release.json");
    let current = PathBuf::from("tests/fixtures/current-release-regression.json");

    let diff = icg::coverage::run_coverage_diff(previous, current).unwrap();

    assert!(diff.has_regressions());

    // safe-list was widened from "vault.*list" to ".*" (catch-all)
    let widened_list: Vec<&str> = diff.widened_safe_patterns
        .iter()
        .map(|p| p.pattern_id.as_str())
        .collect();
    assert!(widened_list.contains(&"safe-list"), "Should detect widened safe-list pattern");
}

#[test]
fn test_detects_narrowed_destructive_patterns() {
    let previous = PathBuf::from("tests/fixtures/previous-release.json");
    let current = PathBuf::from("tests/fixtures/current-release-regression.json");

    let diff = icg::coverage::run_coverage_diff(previous, current).unwrap();

    assert!(diff.has_regressions());

    // git-force-push was narrowed from "git push.*--force" to "git push --force"
    // (removed the wildcard .* between push and --force)
    let narrowed_list: Vec<&str> = diff.narrowed_destructive_patterns
        .iter()
        .map(|p| p.pattern_id.as_str())
        .collect();
    assert!(narrowed_list.contains(&"git-force-push"), "Should detect narrowed git-force-push pattern");
}

#[test]
fn test_pattern_widened_detection() {
    // More specific -> Less specific (widened)
    assert!(icg::coverage::is_pattern_widened("specific-thing", ".*"));
    assert!(icg::coverage::is_pattern_widened("vault.*list", ".*"));
    assert!(icg::coverage::is_pattern_widened("^git status", "git.*"));
}

#[test]
fn test_pattern_not_widened() {
    // Same or more specific (not widened)
    assert!(!icg::coverage::is_pattern_widened(".*", ".*"));
    assert!(!icg::coverage::is_pattern_widened(".*", "specific-thing"));
}

#[test]
fn test_pattern_narrowed_detection() {
    // Less specific -> More specific (narrowed)
    assert!(icg::coverage::is_pattern_narrowed(".*", "specific-thing"));
    assert!(icg::coverage::is_pattern_narrowed("git push.*--force", "git push --force"));
    assert!(icg::coverage::is_pattern_narrowed("dangerous-.*", "dangerous-thing"));
}

#[test]
fn test_pattern_not_narrowed() {
    // Same or less specific (not narrowed)
    assert!(!icg::coverage::is_pattern_narrowed(".*", ".*"));
    assert!(!icg::coverage::is_pattern_narrowed("specific-thing", ".*"));
}

#[test]
fn test_load_rule_pack() {
    let path = PathBuf::from("tests/fixtures/previous-release.json");
    let pack = icg::coverage::load_rule_pack(path).unwrap();

    assert_eq!(pack.id, "test-pack-previous");
    assert_eq!(pack.safe_patterns.len(), 3);
    assert_eq!(pack.guarded_patterns.len(), 4);

    // Verify a specific pattern
    let vault_destroy = pack.guarded_patterns.iter()
        .find(|p| p.id == "vault-kv-destroy")
        .expect("Should find vault-kv-destroy pattern");
    assert_eq!(vault_destroy.check_value, "vault kv destroy");
    assert!(vault_destroy.destructive);
}
