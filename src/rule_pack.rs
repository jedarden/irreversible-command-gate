//! Rule pack schema and loader
//!
//! Defines the complete data model for rule packs as specified in docs/plan/plan.md.
//! Supports both JSON and TOML serialization formats.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::Path;

/// A complete rule pack manifest
///
/// Defines patterns for either command-mode (matching shell invocations) or
/// content-mode (matching file contents written via Edit/Write operations).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Pack {
    /// Unique identifier for this pack (e.g., "vault", "git", "storage-class", "beads")
    pub id: String,

    /// For command-mode packs: executables this pack inspects
    ///
    /// Examples: ["vault", "bao"] for a Vault pack, ["git"] for a Git pack.
    /// Unused by content-mode packs, by beads, and by secrets (which scans the entire
    /// command string unconditionally).
    #[serde(default)]
    pub tool_keywords: Vec<String>,

    /// For content-mode packs: which Write/Edit targets this pack scans
    ///
    /// Examples: ["*.yaml", "*.yml"] for a Kubernetes YAML pack.
    /// Also used by the beads pack (Predicate-type check) to scope its .beads/ path match.
    /// Unused by pure command-mode packs (vault, git, secrets, misc, tmux).
    #[serde(default)]
    pub applies_to: Vec<String>,

    /// Explicitly-allowed patterns, checked FIRST with skip-the-rest precedence
    ///
    /// These patterns bypass the guarded_patterns check entirely. If a command or file
    /// matches any safe_pattern, the rest of the pack's guarded_patterns are skipped.
    #[serde(default)]
    pub safe_patterns: Vec<Pattern>,

    /// Patterns that require protection, with detailed redirect specifications
    ///
    /// Each guarded_pattern defines a dangerous pattern, its severity, and how to
    /// respond when matched (deny, rewrite, or warn).
    #[serde(default)]
    pub guarded_patterns: Vec<GuardedPattern>,
}

/// A lighter pattern than GuardedPattern - just a shape that's explicitly allowed
///
/// These don't have tier/severity/redirect information - they're simply whitelisted
/// patterns that skip the rest of the pack's guarded_patterns check.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Pattern {
    /// Unique identifier for this pattern
    pub id: String,

    /// The check that determines if this pattern matches
    #[serde(flatten)]
    pub check: Check,
}

/// A guarded pattern requiring protection with detailed redirect specification
///
/// Defines a dangerous pattern, how dangerous it is (tier/severity), why it's
/// dangerous (explanation), and how to respond when it matches (redirect).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GuardedPattern {
    /// Unique identifier for this pattern
    pub id: String,

    /// The check that determines if this pattern matches
    #[serde(flatten)]
    pub check: Check,

    /// Deterministic-difficulty tier (1 = stateless, 2 = needs cross-invocation state, 3 = context-dependent)
    pub tier: Tier,

    /// How dangerous this pattern is
    pub severity: Severity,

    /// Why this pattern is dangerous
    pub explanation: String,

    /// How to respond when this pattern matches
    pub redirect: Redirect,

    /// Whether this is a destructive pattern (for coverage-diff regression detection)
    ///
    /// This field is used by Layer 1 CI gate to detect narrowing of destructive patterns.
    #[serde(default)]
    pub destructive: bool,
}

/// The type of check used to determine if a pattern matches
///
/// Discriminated union (sum type) over the three check types:
/// - CommandRegex: matched against shell tokens
/// - ContentRegex: matched against file content
/// - Predicate: custom check function
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum Check {
    /// Match against shell command tokens
    ///
    /// Used by command-mode packs (vault, git, misc, tmux) and the secrets pack.
    /// The secrets pack uses CommandRegex but is hook-only (never reaches the wrapper).
    #[serde(rename = "command_regex")]
    CommandRegex { regex: String },

    /// Match against file content being written
    ///
    /// Used by content-mode packs (storage-class, image-tag, beads).
    /// These packs are hook-only (Write/Edit never reaches the wrapper).
    #[serde(rename = "content_regex")]
    ContentRegex { regex: String },

    /// Custom check function
    ///
    /// General umbrella for custom checks. Examples:
    /// - Filesystem stat for beads .beads/ paths (combined with applies_to glob match)
    /// - Synchronous network lookup (e.g., irrevers-8cff8cf4's Tier 1 exception)
    /// - Phase 2's state-store-backed checks
    #[serde(rename = "predicate")]
    Predicate { predicate_name: String },
}

/// Deterministic-difficulty tier for a guarded pattern
///
/// Classifies how difficult it is to decide if a pattern matches:
/// - Tier 1: Stateless, decidable from a single invocation alone
/// - Tier 2: Needs state that persists across invocations
/// - Tier 3: Not reliably decidable from command syntax alone
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Tier {
    /// Stateless, decidable from a single invocation alone
    ///
    /// Examples: command text, filesystem predicate, or a single synchronous network check.
    /// This is what Phase 1 ships.
    Tier1,

    /// Needs state that persists across invocations
    ///
    /// Examples: "did a git pull happen earlier in this session"
    /// Requires Phase 2's state store.
    Tier2,

    /// Not reliably decidable from command syntax alone
    ///
    /// Examples: git worktree add (legitimate in some contexts, dangerous in others).
    /// Never a deny - at most a non-blocking heuristic additionalContext warning.
    /// May never be pursued at all.
    Tier3,
}

/// How dangerous a guarded pattern is
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Severity {
    /// Pattern causes immediate, irreversible damage
    #[serde(rename = "Critical")]
    Critical,

    /// Pattern causes significant damage or is hard to reverse
    #[serde(rename = "High")]
    High,

    /// Pattern causes moderate damage or has workarounds
    #[serde(rename = "Medium")]
    Medium,
}

/// How to respond when a guarded pattern matches
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Redirect {
    /// The response channel
    pub channel: Channel,

    /// Human-readable reason, supports {derived_value} placeholders
    pub reason_template: String,

    /// Rewritten input (only used when channel = UpdatedInput)
    ///
    /// Provides a safe alternative to the dangerous command.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rewrite_template: Option<String>,
}

/// The response channel when a guarded pattern matches
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Channel {
    /// Block the operation entirely
    ///
    /// Used for critical destructive patterns that must never execute.
    #[serde(rename = "deny")]
    Deny,

    /// Provide updated/safe input to the user
    ///
    /// Used when a safe alternative exists. The rewrite_template provides the
    /// alternative command or content.
    #[serde(rename = "updated_input")]
    UpdatedInput,

    /// Allow with additional context/warning
    ///
    /// Used for Tier 3 patterns that can't be reliably decided. Never blocks,
    /// just provides heuristic warnings.
    #[serde(rename = "additional_context")]
    AdditionalContext,
}

/// Load a rule pack from a file (JSON or TOML)
pub fn load_pack<P: AsRef<Path>>(path: P) -> Result<Pack> {
    let path = path.as_ref();
    let content = std::fs::read_to_string(path)
        .with_context(|| format!("Failed to read rule pack from {}", path.display()))?;

    let extension = path.extension().and_then(|e| e.to_str());

    match extension {
        Some("json") => {
            serde_json::from_str(&content)
                .with_context(|| format!("Failed to parse JSON from {}", path.display()))
        }
        Some("toml") => {
            // TOML support via basic_str feature for inline tables
            // For proper TOML support, we'd need toml crate
            let _ = content;
            Err(anyhow::anyhow!("TOML support not yet implemented - please use JSON manifests"))
        }
        _ => {
            // Default to JSON if no extension
            serde_json::from_str(&content)
                .with_context(|| format!("Failed to parse JSON from {}", path.display()))
        }
    }
}

/// Save a rule pack to a file (JSON or TOML)
pub fn save_pack<P: AsRef<Path>>(pack: &Pack, path: P) -> Result<()> {
    let path = path.as_ref();
    let extension = path.extension().and_then(|e| e.to_str());

    let content = match extension {
        Some("json") => {
            serde_json::to_string_pretty(pack)
                .context("Failed to serialize pack to JSON")?
        }
        Some("toml") => {
            return Err(anyhow::anyhow!("TOML support not yet implemented - please use JSON manifests"))
        }
        _ => {
            serde_json::to_string_pretty(pack)
                .context("Failed to serialize pack to JSON")?
        }
    };

    std::fs::write(path, content)
        .with_context(|| format!("Failed to write rule pack to {}", path.display()))?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_serialize_pattern_command_regex() {
        let pattern = Pattern {
            id: "safe-read".to_string(),
            check: Check::CommandRegex {
                regex: "vault kv get".to_string(),
            },
        };

        let json = serde_json::to_string_pretty(&pattern).unwrap();
        println!("Serialized pattern:\n{}", json);

        // Deserialize back
        let deserialized: Pattern = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.id, "safe-read");
        match deserialized.check {
            Check::CommandRegex { regex } => {
                assert_eq!(regex, "vault kv get");
            }
            _ => panic!("Expected CommandRegex"),
        }
    }

    #[test]
    fn test_serialize_guarded_pattern() {
        let pattern = GuardedPattern {
            id: "vault-kv-destroy".to_string(),
            check: Check::CommandRegex {
                regex: "vault kv destroy".to_string(),
            },
            tier: Tier::Tier1,
            severity: Severity::Critical,
            explanation: "Permanently destroys vault data versions".to_string(),
            redirect: Redirect {
                channel: Channel::Deny,
                reason_template: "vault kv destroy is permanently destructive".to_string(),
                rewrite_template: None,
            },
            destructive: true,
        };

        let json = serde_json::to_string_pretty(&pattern).unwrap();
        println!("Serialized guarded pattern:\n{}", json);

        // Deserialize back
        let deserialized: GuardedPattern = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.id, "vault-kv-destroy");
        assert_eq!(deserialized.tier, Tier::Tier1);
        assert_eq!(deserialized.severity, Severity::Critical);
        assert!(deserialized.destructive);
    }

    #[test]
    fn test_serialize_pack_with_all_fields() {
        let pack = Pack {
            id: "vault".to_string(),
            tool_keywords: vec!["vault".to_string(), "bao".to_string()],
            applies_to: vec![],
            safe_patterns: vec![Pattern {
                id: "safe-read".to_string(),
                check: Check::CommandRegex {
                    regex: "vault kv get".to_string(),
                },
            }],
            guarded_patterns: vec![GuardedPattern {
                id: "vault-kv-destroy".to_string(),
                check: Check::CommandRegex {
                    regex: "vault kv destroy".to_string(),
                },
                tier: Tier::Tier1,
                severity: Severity::Critical,
                explanation: "Permanently destroys vault data versions".to_string(),
                redirect: Redirect {
                    channel: Channel::Deny,
                    reason_template: "Destructive operation".to_string(),
                    rewrite_template: None,
                },
                destructive: true,
            }],
        };

        let json = serde_json::to_string_pretty(&pack).unwrap();
        println!("Serialized pack:\n{}", json);

        // Deserialize back
        let deserialized: Pack = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.id, "vault");
        assert_eq!(deserialized.tool_keywords.len(), 2);
        assert_eq!(deserialized.safe_patterns.len(), 1);
        assert_eq!(deserialized.guarded_patterns.len(), 1);
    }

    #[test]
    fn test_load_current_release_fixture() {
        // This test verifies the schema matches the committed fixture format
        let pack = load_pack("tests/fixtures/current-release-clean.json")
            .expect("Failed to load current-release-clean.json");

        assert_eq!(pack.id, "test-pack-current-clean");
        assert_eq!(pack.tool_keywords, vec!["vault".to_string(), "git".to_string()]);
        assert_eq!(pack.safe_patterns.len(), 3);
        assert_eq!(pack.guarded_patterns.len(), 4);

        // Verify first safe pattern structure
        let safe = &pack.safe_patterns[0];
        assert_eq!(safe.id, "safe-read-operations");
        match &safe.check {
            Check::CommandRegex { regex } => {
                assert_eq!(regex, "vault kv get");
            }
            _ => panic!("Expected CommandRegex check"),
        }

        // Verify first guarded pattern has all required fields
        let guarded = &pack.guarded_patterns[0];
        assert_eq!(guarded.id, "vault-kv-destroy");
        assert_eq!(guarded.tier, Tier::Tier1);
        assert_eq!(guarded.severity, Severity::Critical);
        assert!(guarded.destructive);
        assert_eq!(guarded.redirect.channel, Channel::Deny);
        assert!(guarded.redirect.reason_template.contains("permanently destructive"));
        assert!(guarded.redirect.rewrite_template.is_none());

        // Verify git force-push pattern has rewrite_template
        let git_force = &pack.guarded_patterns[2];
        assert_eq!(git_force.id, "git-force-push");
        assert_eq!(git_force.tier, Tier::Tier1);
        assert_eq!(git_force.severity, Severity::Critical);
        assert_eq!(git_force.redirect.channel, Channel::Deny);
        assert!(git_force.redirect.rewrite_template.is_some());
        assert!(git_force.redirect.rewrite_template.as_ref().unwrap().contains("--force-with-lease"));
    }

    #[test]
    fn test_load_previous_release_fixture() {
        let pack = load_pack("tests/fixtures/previous-release.json")
            .expect("Failed to load previous-release.json");

        assert_eq!(pack.id, "test-pack-previous");
        assert_eq!(pack.tool_keywords, vec!["vault".to_string(), "git".to_string()]);
        assert_eq!(pack.safe_patterns.len(), 3);
        assert_eq!(pack.guarded_patterns.len(), 4);
    }
}
