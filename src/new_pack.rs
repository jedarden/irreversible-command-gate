//! Pack scaffolding tool
//!
//! Generates new rule pack files with pre-filled Pack/GuardedPattern fields,
//! plus paired regression-test stubs. Enforces the "every rule needs a test"
//! discipline structurally rather than relying on author memory.

use anyhow::{Context, Result};
use std::fs;
use std::path::{Path, PathBuf};

use crate::rule_pack::{Channel, Check, GuardedPattern, Pack, Pattern, Redirect, Severity, Tier};

/// Generate a new pack scaffolding with paired test stub
///
/// # Arguments
/// * `pack_name` - Name/ID for the pack (e.g., "vault", "storage-class")
/// * `pack_type` - Type of pack: "command" (shell commands) or "content" (file contents)
/// * `output_dir` - Target directory for generated files
///
/// # Returns
/// Paths to the generated pack file and test file
pub fn generate_pack_scaffolding(
    pack_name: &str,
    pack_type: &str,
    output_dir: &Path,
) -> Result<(PathBuf, PathBuf)> {
    validate_pack_name(pack_name)?;

    // Validate pack_type
    if pack_type != "command" && pack_type != "content" {
        return Err(anyhow::anyhow!(
            "Pack type must be either 'command' or 'content', got: '{}'",
            pack_type
        ));
    }

    // Create output directory if it doesn't exist
    fs::create_dir_all(output_dir).with_context(|| {
        format!(
            "Failed to create output directory: {}",
            output_dir.display()
        )
    })?;

    // Generate the pack
    let pack = generate_pack_template(pack_name, pack_type)?;

    // Write pack file
    let pack_file_path = output_dir.join(format!("{}.json", pack_name));
    let pack_json =
        serde_json::to_string_pretty(&pack).context("Failed to serialize pack to JSON")?;

    // Generate and write test stub
    let test_file_path = output_dir.join(format!("{}_pack_tests.rs", pack_name));
    let test_content = generate_test_stub(pack_name, pack_type);

    // A scaffold should never silently replace an author's existing pack or
    // tests. Check both destinations before creating either file so a retry is
    // explicit and a failed run cannot destroy existing work.
    if pack_file_path.exists() {
        return Err(anyhow::anyhow!(
            "Refusing to overwrite existing pack file: {}",
            pack_file_path.display()
        ));
    }
    if test_file_path.exists() {
        return Err(anyhow::anyhow!(
            "Refusing to overwrite existing test file: {}",
            test_file_path.display()
        ));
    }

    write_new_file(&pack_file_path, &pack_json, "pack")?;
    if let Err(error) = write_new_file(&test_file_path, &test_content, "test") {
        // Avoid leaving a misleading half-scaffold behind if the second file
        // cannot be created. The pack did not exist before this invocation.
        let _ = fs::remove_file(&pack_file_path);
        return Err(error);
    }

    Ok((pack_file_path, test_file_path))
}

fn validate_pack_name(pack_name: &str) -> Result<()> {
    let valid_characters = pack_name.chars().all(|character| {
        character.is_ascii_lowercase() || character.is_ascii_digit() || character == '-'
    });

    if pack_name.is_empty()
        || !valid_characters
        || pack_name.starts_with('-')
        || pack_name.ends_with('-')
        || pack_name.contains("--")
    {
        return Err(anyhow::anyhow!(
            "Pack name must be kebab-case (lowercase letters, hyphens, digits only): '{}'",
            pack_name
        ));
    }

    Ok(())
}

fn write_new_file(path: &Path, contents: &str, kind: &str) -> Result<()> {
    use std::io::Write;

    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .with_context(|| format!("Failed to create {kind} file: {}", path.display()))?;
    file.write_all(contents.as_bytes())
        .with_context(|| format!("Failed to write {kind} file: {}", path.display()))?;
    file.write_all(b"\n")
        .with_context(|| format!("Failed to finish {kind} file: {}", path.display()))?;
    Ok(())
}

/// Generate a Pack template with example patterns
fn generate_pack_template(pack_name: &str, pack_type: &str) -> Result<Pack> {
    let pack = match pack_type {
        "command" => generate_command_pack_template(pack_name),
        "content" => generate_content_pack_template(pack_name),
        _ => return Err(anyhow::anyhow!("Unknown pack type: {}", pack_type)),
    };

    Ok(pack)
}

/// Generate a command-mode pack template
fn generate_command_pack_template(pack_name: &str) -> Pack {
    Pack {
        id: pack_name.to_string(),
        tool_keywords: vec![pack_name.to_string()],
        applies_to: vec![],
        safe_patterns: vec![
            Pattern {
                id: format!("safe-{}-read", pack_name),
                check: Check::CommandRegex {
                    regex: format!("{}.*get", pack_name),
                },
            },
            Pattern {
                id: format!("safe-{}-list", pack_name),
                check: Check::CommandRegex {
                    regex: format!("{}.*list", pack_name),
                },
            },
        ],
        guarded_patterns: vec![GuardedPattern {
            id: format!("{}-dangerous-operation", pack_name),
            enabled: true,
            check: Check::CommandRegex {
                regex: format!("{}.*dangerous-command", pack_name),
            },
            tier: Tier::Tier1,
            severity: Severity::Critical,
            explanation: format!(
                "{} dangerous operation that causes irreversible damage",
                pack_name
            ),
            redirect: Redirect {
                channel: Channel::Deny,
                reason_template: format!(
                    "This {} operation is permanently destructive and cannot be undone",
                    pack_name
                ),
                rewrite_template: Some(format!("{} safe-alternative", pack_name)),
            },
            destructive: true,
        }],
    }
}

/// Generate a content-mode pack template
fn generate_content_pack_template(pack_name: &str) -> Pack {
    Pack {
        id: pack_name.to_string(),
        tool_keywords: vec![],
        applies_to: vec!["*.yaml".to_string(), "*.yml".to_string()],
        safe_patterns: vec![Pattern {
            id: format!("safe-{}-pattern", pack_name),
            check: Check::ContentRegex {
                regex: "safe-pattern".to_string(),
            },
        }],
        guarded_patterns: vec![GuardedPattern {
            id: format!("{}-dangerous-content", pack_name),
            enabled: true,
            check: Check::ContentRegex {
                regex: "dangerous-pattern".to_string(),
            },
            tier: Tier::Tier1,
            severity: Severity::High,
            explanation: format!("{} content that violates safety constraints", pack_name),
            redirect: Redirect {
                channel: Channel::Deny,
                reason_template: format!(
                    "This {} pattern is prohibited and must be replaced",
                    pack_name
                ),
                rewrite_template: Some("safe-alternative-pattern".to_string()),
            },
            destructive: true,
        }],
    }
}

/// Generate a test stub for the pack
fn generate_test_stub(pack_name: &str, pack_type: &str) -> String {
    let _pack_safe_name = pack_name.replace("-", "_");
    let fixture_path = format!("{}.json", pack_name);

    match pack_type {
        "command" => format!(
            r##"//! Tests for {} pack
//!
//! This file tests the {} rule pack.
//! Add regression tests here as you implement guarded patterns.

use std::path::PathBuf;

use icg::engine::{{Engine, CommandSource, CheckResult}};
use icg::rule_pack::load_pack;

#[test]
fn fixture_loads_successfully() {{
    let manifest = PathBuf::from("{}");
    let pack = load_pack(&manifest).unwrap();

    assert_eq!(pack.id, "{}");
    assert!(!pack.guarded_patterns.is_empty(), "Pack must have at least one guarded pattern");
}}

#[test]
fn safe_patterns_bypass_check() {{
    let manifest = PathBuf::from("{}");
    let pack = load_pack(&manifest).unwrap();

    let mut engine = Engine::new();
    engine.load_pack(pack).unwrap();

    // Test safe pattern - should not trigger deny
    let safe_command = format!("{} get", "{}");
    let result = engine.evaluate_command(&CommandSource::Hook(safe_command));

    // Safe patterns should not be denied
    if matches!(result, CheckResult::Denied {{ .. }}) {{
        panic!("Safe pattern should not trigger deny, got: {{:?}}", result);
    }}
}}

#[test]
fn guarded_pattern_detects_dangerous_operations() {{
    let manifest = PathBuf::from("{}");
    let pack = load_pack(&manifest).unwrap();

    let mut engine = Engine::new();
    engine.load_pack(pack).unwrap();

    // Test guarded pattern - should trigger deny
    let dangerous_command = format!("{} dangerous-command", "{}");
    let result = engine.evaluate_command(&CommandSource::Hook(dangerous_command));

    // Dangerous patterns should be denied
    match result {{
        CheckResult::Denied {{ pattern_id, .. }} => {{
            assert_eq!(
                pattern_id,
                "{}-dangerous-operation",
                "The generated regression test must identify the guarded rule"
            );
        }}
        _ => {{
            panic!("Dangerous operation should be denied, got: {{:?}}", result);
        }}
    }}
}}

// Add more tests below as you implement additional guarded patterns
// Each guarded pattern should have a corresponding test case
"##,
            pack_name,
            pack_name,
            fixture_path,
            pack_name,
            fixture_path,
            pack_name,
            pack_name,
            fixture_path,
            pack_name,
            pack_name,
            pack_name
        ),

        "content" => format!(
            r##"//! Tests for {} pack
//!
//! This file tests the {} rule pack.
//! Add regression tests here as you implement guarded patterns.

use std::path::PathBuf;

use icg::engine::{{Engine, ContentSource, CheckResult}};
use icg::rule_pack::load_pack;

#[test]
fn fixture_loads_successfully() {{
    let manifest = PathBuf::from("{}");
    let pack = load_pack(&manifest).unwrap();

    assert_eq!(pack.id, "{}");
    assert!(!pack.guarded_patterns.is_empty(), "Pack must have at least one guarded pattern");
}}

#[test]
fn safe_patterns_bypass_check() {{
    let manifest = PathBuf::from("{}");
    let pack = load_pack(&manifest).unwrap();

    let mut engine = Engine::new();
    engine.load_pack(pack).unwrap();

    // Test safe pattern - should not trigger deny
    let safe_content = r#"safe-pattern"#;
    let result = engine.evaluate_content(&ContentSource::Write {{
        file_path: "test.yaml".into(),
        content: safe_content.to_string(),
    }});

    // Safe patterns should not be denied
    if matches!(result, CheckResult::Denied {{ .. }}) {{
        panic!("Safe pattern should not trigger deny, got: {{:?}}", result);
    }}
}}

#[test]
fn guarded_pattern_detects_dangerous_content() {{
    let manifest = PathBuf::from("{}");
    let pack = load_pack(&manifest).unwrap();

    let mut engine = Engine::new();
    engine.load_pack(pack).unwrap();

    // Test guarded pattern - should trigger deny
    let dangerous_content = r#"dangerous-pattern"#;
    let result = engine.evaluate_content(&ContentSource::Write {{
        file_path: "test.yaml".into(),
        content: dangerous_content.to_string(),
    }});

    // Dangerous patterns should be denied
    match result {{
        CheckResult::Denied {{ pattern_id, .. }} => {{
            assert_eq!(
                pattern_id,
                "{}-dangerous-content",
                "The generated regression test must identify the guarded rule"
            );
        }}
        _ => {{
            panic!("Dangerous content should be denied, got: {{:?}}", result);
        }}
    }}
}}

// Add more tests below as you implement additional guarded patterns
// Each guarded pattern should have a corresponding test case
"##,
            pack_name, pack_name, fixture_path, pack_name, fixture_path, pack_name, fixture_path
        ),

        _ => format!("// Unknown pack type: {}\n", pack_type).to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    use tempfile::tempdir;

    #[test]
    fn test_generate_command_pack() {
        let pack = generate_command_pack_template("test-tool");

        assert_eq!(pack.id, "test-tool");
        assert_eq!(pack.tool_keywords, vec!["test-tool"]);
        assert!(pack.applies_to.is_empty());
        assert_eq!(pack.safe_patterns.len(), 2);
        assert_eq!(pack.guarded_patterns.len(), 1);

        let safe = &pack.safe_patterns[0];
        assert_eq!(safe.id, "safe-test-tool-read");

        let guarded = &pack.guarded_patterns[0];
        assert_eq!(guarded.id, "test-tool-dangerous-operation");
        assert_eq!(guarded.tier, Tier::Tier1);
        assert_eq!(guarded.severity, Severity::Critical);
    }

    #[test]
    fn test_generate_content_pack() {
        let pack = generate_content_pack_template("test-pattern");

        assert_eq!(pack.id, "test-pattern");
        assert!(pack.tool_keywords.is_empty());
        assert_eq!(pack.applies_to, vec!["*.yaml", "*.yml"]);
        assert_eq!(pack.safe_patterns.len(), 1);
        assert_eq!(pack.guarded_patterns.len(), 1);

        let guarded = &pack.guarded_patterns[0];
        assert_eq!(guarded.id, "test-pattern-dangerous-content");
        assert_eq!(guarded.tier, Tier::Tier1);
        assert_eq!(guarded.severity, Severity::High);
    }

    #[test]
    fn test_validate_pack_name() {
        let valid_names = vec!["vault", "storage-class", "test123", "my-pack"];
        for name in valid_names {
            assert!(validate_pack_name(name).is_ok());
        }

        let invalid_names = vec![
            "",
            "Vault",
            "test_pack",
            "test.pack",
            "test pack",
            "-test",
            "test-",
            "test--pack",
        ];
        for name in invalid_names {
            assert!(
                validate_pack_name(name).is_err(),
                "{name} should be invalid"
            );
        }
    }

    #[test]
    fn scaffold_writes_a_loadable_pack_and_paired_test_stub() {
        let directory = tempdir().unwrap();

        let (pack_path, test_path) =
            generate_pack_scaffolding("example-tool", "command", directory.path()).unwrap();

        let pack = crate::rule_pack::load_pack(&pack_path).unwrap();
        assert_eq!(pack.id, "example-tool");
        assert_eq!(pack.tool_keywords, vec!["example-tool"]);
        assert_eq!(pack.safe_patterns.len(), 2);
        assert_eq!(pack.guarded_patterns.len(), 1);
        assert!(pack.guarded_patterns[0].enabled);
        assert!(pack.guarded_patterns[0].destructive);

        let test_stub = fs::read_to_string(test_path).unwrap();
        assert!(test_stub.contains("fixture_loads_successfully"));
        assert!(test_stub.contains("guarded_pattern_detects_dangerous_operations"));
        assert!(test_stub.contains("example-tool-dangerous-operation"));
    }

    #[test]
    fn scaffold_supports_content_packs() {
        let directory = tempdir().unwrap();

        let (pack_path, _) =
            generate_pack_scaffolding("manifest-rules", "content", directory.path()).unwrap();
        let pack = crate::rule_pack::load_pack(&pack_path).unwrap();

        assert!(pack.tool_keywords.is_empty());
        assert_eq!(pack.applies_to, vec!["*.yaml", "*.yml"]);
        assert!(matches!(
            pack.guarded_patterns[0].check,
            Check::ContentRegex { .. }
        ));
    }

    #[test]
    fn scaffold_refuses_to_overwrite_existing_files() {
        let directory = tempdir().unwrap();
        generate_pack_scaffolding("example-tool", "command", directory.path()).unwrap();
        let pack_path = directory.path().join("example-tool.json");
        let original = fs::read_to_string(&pack_path).unwrap();

        let error = generate_pack_scaffolding("example-tool", "command", directory.path())
            .expect_err("a second scaffold should not overwrite files");

        assert!(error.to_string().contains("Refusing to overwrite"));
        assert_eq!(fs::read_to_string(pack_path).unwrap(), original);
    }

    #[test]
    fn scaffold_rejects_unknown_pack_type() {
        let directory = tempdir().unwrap();

        let error = generate_pack_scaffolding("example-tool", "unknown", directory.path())
            .expect_err("unknown pack types should be rejected");

        assert!(error.to_string().contains("command") && error.to_string().contains("content"));
        assert!(directory.path().read_dir().unwrap().next().is_none());
    }

    #[test]
    fn scaffold_rejects_invalid_pack_name_before_creating_directory() {
        let directory = tempdir().unwrap().path().join("output");

        let error = generate_pack_scaffolding("../escape", "command", &directory)
            .expect_err("path traversal is not a pack name");

        assert!(error.to_string().contains("kebab-case"));
        assert!(!directory.exists());
    }
}
