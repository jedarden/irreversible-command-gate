//! Rule pack schema and loader
//!
//! Defines the complete data model for rule packs as specified in docs/plan/plan.md.
//! Supports both JSON and TOML serialization formats.

use anyhow::{Context, Result};
use regex::Regex;
use serde::{de::Error as DeError, Deserialize, Deserializer, Serialize};
use serde_json::Value;
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
#[derive(Debug, Clone, Serialize)]
pub struct Pattern {
    /// Unique identifier for this pattern
    pub id: String,

    /// The check that determines if this pattern matches
    #[serde(flatten)]
    pub check: Check,
}

impl<'de> Deserialize<'de> for Pattern {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let mut object = match Value::deserialize(deserializer)? {
            Value::Object(object) => object,
            _ => return Err(D::Error::custom("pattern must be a JSON object")),
        };

        let id = object
            .remove("id")
            .ok_or_else(|| D::Error::custom("pattern is missing id"))
            .and_then(|value| serde_json::from_value(value).map_err(D::Error::custom))?;
        let check = match object.remove("check") {
            Some(value) => serde_json::from_value(value).map_err(D::Error::custom)?,
            None => serde_json::from_value(Value::Object(object)).map_err(D::Error::custom)?,
        };

        Ok(Self { id, check })
    }
}

/// A guarded pattern requiring protection with detailed redirect specification
///
/// Defines a dangerous pattern, how dangerous it is (tier/severity), why it's
/// dangerous (explanation), and how to respond when it matches (redirect).
#[derive(Debug, Clone, Serialize)]
pub struct GuardedPattern {
    /// Unique identifier for this pattern
    pub id: String,

    /// Whether this rule participates in evaluation.
    ///
    /// This is part of the released rule-pack data.  It defaults to `true`
    /// so manifests written before the flag was introduced retain their
    /// existing behavior.
    #[serde(default = "default_enabled")]
    pub enabled: bool,

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

impl<'de> Deserialize<'de> for GuardedPattern {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let mut object = match Value::deserialize(deserializer)? {
            Value::Object(object) => object,
            _ => return Err(D::Error::custom("guarded pattern must be a JSON object")),
        };

        let id = object
            .remove("id")
            .ok_or_else(|| D::Error::custom("guarded pattern is missing id"))
            .and_then(|value| serde_json::from_value(value).map_err(D::Error::custom))?;
        let enabled = object
            .remove("enabled")
            .map(serde_json::from_value)
            .transpose()
            .map_err(D::Error::custom)?
            .unwrap_or_else(default_enabled);
        let check = match object.remove("check") {
            Some(value) => serde_json::from_value(value).map_err(D::Error::custom)?,
            None => {
                serde_json::from_value(Value::Object(object.clone())).map_err(D::Error::custom)?
            }
        };
        let tier = object
            .remove("tier")
            .ok_or_else(|| D::Error::custom("guarded pattern is missing tier"))
            .and_then(|value| serde_json::from_value(value).map_err(D::Error::custom))?;
        let severity = object
            .remove("severity")
            .ok_or_else(|| D::Error::custom("guarded pattern is missing severity"))
            .and_then(|value| serde_json::from_value(value).map_err(D::Error::custom))?;
        let explanation = object
            .remove("explanation")
            .ok_or_else(|| D::Error::custom("guarded pattern is missing explanation"))
            .and_then(|value| serde_json::from_value(value).map_err(D::Error::custom))?;
        let redirect = object
            .remove("redirect")
            .ok_or_else(|| D::Error::custom("guarded pattern is missing redirect"))
            .and_then(|value| serde_json::from_value(value).map_err(D::Error::custom))?;
        let destructive = object
            .remove("destructive")
            .map(serde_json::from_value)
            .transpose()
            .map_err(D::Error::custom)?
            .unwrap_or(false);

        Ok(Self {
            id,
            enabled,
            check,
            tier,
            severity,
            explanation,
            redirect,
            destructive,
        })
    }
}

fn default_enabled() -> bool {
    true
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
    ///
    /// `data` is optional rule-pack data for predicates whose policy is
    /// configuration rather than executable logic.  For example, the misc
    /// pack's deprecated bead-CLI check stores its canonical and deprecated
    /// names here so a cutover only changes the manifest data.
    #[serde(rename = "predicate")]
    Predicate {
        predicate_name: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        data: Option<serde_json::Value>,
    },
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

/// The redirect-actionability schema gate
///
/// AGENTS.md rule 2: every guarded rule owes the caller an alternative — a
/// `redirect` whose reason only says "blocked" is an incomplete rule (see
/// `docs/notes/redirect-not-just-block.md`). This is the structural half of
/// that doctrine: a predicate deciding whether a `reason_template` names a
/// concrete sanctioned alternative, and a pack-level validator built on it.
///
/// Enforcement lives in `tests/redirect_actionability_tests.rs`, which runs
/// the validator over every shipped pack and carries the negative fixtures
/// (empty, whitespace-only, block-only and danger-without-alternative
/// reasons). It is deliberately NOT wired into `Engine::load_pack`: the
/// engine fails open, so a load-time rejection would silently drop the
/// offending pack at runtime — the worst outcome for a policy defect. A bad
/// redirect must fail loudly at authoring time, in CI.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RedirectActionabilityError {
    /// `reason_template` is missing or whitespace-only
    EmptyReason,

    /// `reason_template` only restates the block or the danger and names no
    /// concrete sanctioned alternative
    NonActionable,
}

impl std::fmt::Display for RedirectActionabilityError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::EmptyReason => write!(f, "redirect reason_template is empty"),
            Self::NonActionable => write!(
                f,
                "redirect reason_template merely restates the block and names \
                 no concrete sanctioned alternative"
            ),
        }
    }
}

/// Imperative verbs that signal a next step the agent can act on directly.
///
/// Deliberately a curated list, not "any verb": the gate must pass a reason
/// like "Run 'git pull' first" and reject one like "This command is blocked.
/// Do not run it." — negated verb phrases are stripped before this list is
/// consulted, so the verb inside a prohibition never counts.
const DIRECTIVE_VERBS: &[&str] = &[
    "use",
    "run",
    "pass",
    "try",
    "switch",
    "prefer",
    "pin",
    "capture",
    "record",
    "invoke",
    "install",
    "submit",
    "apply",
    "materialize",
    "inspect",
    "wait",
    "let",
    "add",
    "remove",
    "edit",
    "commit",
    "push",
    "write",
    "read",
    "set",
    "create",
    "call",
    "retry",
    "rotate",
    "consult",
    "follow",
    "choose",
    "pick",
    "migrate",
    "replace",
    "take",
    "keep",
    "include",
    "ask",
    "restore",
];

/// Negated verb phrases — "do not run", "never use", "cannot be undone" —
/// removed before the directive-verb scan. Only the negator and the word
/// immediately after it are stripped, so an alternative named later in the
/// sentence ("Never use ssd; use sata-large instead") still reads through.
const NEGATED_PHRASES: &str = r"(?i)\b(?:do\s+not|don't|don’t|cannot|can't|can’t|could\s+not|must\s+not|should\s+not|never|avoid|without|no\s+longer)\s+\w+";

fn negated_phrases() -> &'static Regex {
    use std::sync::OnceLock;
    static REGEX: OnceLock<Regex> = OnceLock::new();
    REGEX.get_or_init(|| Regex::new(NEGATED_PHRASES).expect("negated-phrase regex must compile"))
}

fn directive_verbs() -> &'static Regex {
    use std::sync::OnceLock;
    static REGEX: OnceLock<Regex> = OnceLock::new();
    REGEX.get_or_init(|| {
        Regex::new(&format!(r"\b(?:{})\b", DIRECTIVE_VERBS.join("|")))
            .expect("directive-verb regex must compile")
    })
}

fn derived_placeholders() -> &'static Regex {
    use std::sync::OnceLock;
    static REGEX: OnceLock<Regex> = OnceLock::new();
    REGEX.get_or_init(|| {
        Regex::new(r"\{[a-z_][a-z_0-9]*\}").expect("placeholder regex must compile")
    })
}

fn urls() -> &'static Regex {
    use std::sync::OnceLock;
    static REGEX: OnceLock<Regex> = OnceLock::new();
    REGEX.get_or_init(|| Regex::new(r"https?://\S+").expect("url regex must compile"))
}

fn code_spans() -> &'static Regex {
    use std::sync::OnceLock;
    static REGEX: OnceLock<Regex> = OnceLock::new();
    REGEX.get_or_init(|| Regex::new(r"`[^`\n]+`").expect("code-span regex must compile"))
}

/// True when `text` carries a quoted snippet that reads like a command, flag
/// or path: two same-kind quote characters whose content contains whitespace
/// or one of the structural characters `/ - = $ < >`. The opening quote must
/// not continue a word, so possessives ("operator's") and contractions
/// ("doesn't") are prose, never quoted snippets.
fn contains_quoted_snippet(text: &str) -> bool {
    let bytes = text.as_bytes();
    for (i, &byte) in bytes.iter().enumerate() {
        if byte != b'\'' && byte != b'"' {
            continue;
        }
        if i > 0 && (bytes[i - 1].is_ascii_alphanumeric() || bytes[i - 1] == b'_') {
            continue;
        }
        let Some(close) = bytes[i + 1..].iter().position(|&c| c == byte) else {
            continue;
        };
        let content = &text[i + 1..i + 1 + close];
        if content.len() >= 2
            && content
                .chars()
                .any(|c| matches!(c, ' ' | '/' | '-' | '=' | '$' | '<' | '>'))
        {
            return true;
        }
    }
    false
}

/// Decide whether a guarded rule's redirect reason names a concrete
/// sanctioned alternative. A reason counts as actionable when ANY of these
/// markers is present:
///
/// 1. a non-empty `rewrite_template` — an `updated_input` redirect's rewrite
///    IS the alternative (the git pack's force-push rule);
/// 2. a derived-value placeholder (`{derived_value}`) — the value computed
///    at check time, exactly what the doctrine calls for (image-tag);
/// 3. a URL naming the sanctioned surface (argocd-topology);
/// 4. an inline code span quoting a command or path (openbao, kubectl);
/// 5. a quoted command/flag/path snippet (git, beads, docker);
/// 6. a positive directive verb, outside a negated phrase (most packs).
fn redirect_names_alternative(pattern: &GuardedPattern) -> bool {
    let reason = pattern.redirect.reason_template.trim();

    if pattern
        .redirect
        .rewrite_template
        .as_deref()
        .map(str::trim)
        .is_some_and(|template| !template.is_empty())
    {
        return true;
    }

    if derived_placeholders().is_match(reason) || urls().is_match(reason) {
        return true;
    }

    if code_spans().is_match(reason) || contains_quoted_snippet(reason) {
        return true;
    }

    let stripped = negated_phrases().replace_all(reason, " ");
    directive_verbs().is_match(&stripped.to_lowercase())
}

/// The actionability verdict for one guarded rule's redirect.
///
/// `Some(error)` means the rule fails the gate; the caller supplies the
/// pack/rule identity when reporting. Applies to every guarded rule whether
/// or not it is currently `enabled: false` — shipped policy data stays
/// complete regardless of whether it evaluates, because re-enabling a rule
/// must never resurrect an empty redirect with it.
pub fn redirect_actionability_violation(
    pattern: &GuardedPattern,
) -> Option<RedirectActionabilityError> {
    if pattern.redirect.reason_template.trim().is_empty() {
        return Some(RedirectActionabilityError::EmptyReason);
    }
    if redirect_names_alternative(pattern) {
        None
    } else {
        Some(RedirectActionabilityError::NonActionable)
    }
}

/// Validate every guarded rule in `pack` against the actionability gate.
///
/// The error names the pack, the rule and the doctrine, so a failing run
/// points straight at the redirect to rewrite.
pub fn validate_redirect_actionability(pack: &Pack) -> Result<()> {
    for pattern in &pack.guarded_patterns {
        if let Some(error) = redirect_actionability_violation(pattern) {
            anyhow::bail!(
                "guarded rule '{}.{}' has a non-actionable redirect: {}. Every \
                 guarded rule owes the caller a concrete sanctioned alternative \
                 -- see docs/notes/redirect-not-just-block.md",
                pack.id,
                pattern.id,
                error
            );
        }
    }
    Ok(())
}

/// Load a rule pack from a file (JSON or TOML)
pub fn load_pack<P: AsRef<Path>>(path: P) -> Result<Pack> {
    let path = path.as_ref();
    let content = std::fs::read_to_string(path)
        .with_context(|| format!("Failed to read rule pack from {}", path.display()))?;

    let extension = path.extension().and_then(|e| e.to_str());

    match extension {
        Some("json") => serde_json::from_str(&content)
            .with_context(|| format!("Failed to parse JSON from {}", path.display())),
        Some("toml") => {
            // TOML support via basic_str feature for inline tables
            // For proper TOML support, we'd need toml crate
            let _ = content;
            Err(anyhow::anyhow!(
                "TOML support not yet implemented - please use JSON manifests"
            ))
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
            serde_json::to_string_pretty(pack).context("Failed to serialize pack to JSON")?
        }
        Some("toml") => {
            return Err(anyhow::anyhow!(
                "TOML support not yet implemented - please use JSON manifests"
            ))
        }
        _ => serde_json::to_string_pretty(pack).context("Failed to serialize pack to JSON")?,
    };

    std::fs::write(path, content)
        .with_context(|| format!("Failed to write rule pack to {}", path.display()))?;

    Ok(())
}

/// Merge multiple packs into a single combined pack
///
/// This combines all tool_keywords, applies_to, safe_patterns, and guarded_patterns
/// from multiple packs into one unified pack. The merged pack gets a generic id
/// "rule-pack" and contains all patterns from all input packs.
pub fn merge_packs(packs: Vec<Pack>) -> Result<Pack> {
    if packs.is_empty() {
        return Err(anyhow::anyhow!("Cannot merge empty pack list"));
    }

    // An unconditional pack (empty tool_keywords -- the secrets pack's
    // whole-command scan) cannot survive a merge with keyword-dispatched
    // packs: the merged pack dispatches on the union of tool_keywords, so
    // the unconditional patterns would only be checked for commands whose
    // executable matches one of those keywords, silently losing exactly the
    // coverage the unconditional pack exists for. Refuse the mixed merge
    // loudly rather than ship a hollow artifact. A uniformly unconditional
    // set still merges correctly -- the union of empty keyword sets is
    // empty, so the merged pack keeps whole-command semantics.
    let mixed = packs.iter().any(|pack| pack.tool_keywords.is_empty())
        && packs.iter().any(|pack| !pack.tool_keywords.is_empty());
    if mixed {
        return Err(anyhow::anyhow!(
            "refusing to merge unconditional packs (empty tool_keywords, e.g. 'secrets') \
             with keyword-dispatched packs: the merged pack would dispatch on the union of \
             tool_keywords and the unconditional whole-command scan would be silently lost. \
             Distribute the packs directory as-is (ICG_RULE_PACK=/etc/icg/packs) instead of \
             a merged single-file artifact"
        ));
    }

    let mut merged_tool_keywords: Vec<String> = Vec::new();
    let mut merged_applies_to: Vec<String> = Vec::new();
    let mut merged_safe_patterns: Vec<Pattern> = Vec::new();
    let mut merged_guarded_patterns: Vec<GuardedPattern> = Vec::new();

    for pack in packs {
        // Merge tool_keywords, avoiding duplicates
        for keyword in pack.tool_keywords {
            if !merged_tool_keywords.contains(&keyword) {
                merged_tool_keywords.push(keyword);
            }
        }

        // Merge applies_to, avoiding duplicates
        for applies in pack.applies_to {
            if !merged_applies_to.contains(&applies) {
                merged_applies_to.push(applies);
            }
        }

        // Merge safe_patterns
        merged_safe_patterns.extend(pack.safe_patterns);

        // Merge guarded_patterns
        merged_guarded_patterns.extend(pack.guarded_patterns);
    }

    Ok(Pack {
        id: "rule-pack".to_string(),
        tool_keywords: merged_tool_keywords,
        applies_to: merged_applies_to,
        safe_patterns: merged_safe_patterns,
        guarded_patterns: merged_guarded_patterns,
    })
}

/// Load all pack files from a directory and merge them into a single pack
///
/// This reads all .json files from the specified directory, loads them as packs,
/// and merges them into a single unified pack suitable for distribution as a
/// rule-pack.json artifact.
///
/// Unconditional packs (empty tool_keywords, like 'secrets') are excluded from
/// the merged artifact since they cannot be mixed with keyword-dispatched packs
/// without losing coverage. These packs remain available in the packs/ directory
/// for hook mode, which uses the directory directly rather than the merged artifact.
pub fn load_and_merge_packs_from_dir<P: AsRef<Path>>(dir: P) -> Result<Pack> {
    let dir = dir.as_ref();
    let entries = std::fs::read_dir(dir)
        .with_context(|| format!("Failed to read pack directory: {}", dir.display()))?;

    let mut packs = Vec::new();
    let mut skipped_packs = Vec::new();

    for entry in entries {
        let entry = entry.context("Failed to read directory entry")?;
        let path = entry.path();

        // Only load .json files
        if path.extension().and_then(|e| e.to_str()) == Some("json") {
            let pack = load_pack(&path)
                .with_context(|| format!("Failed to load pack from: {}", path.display()))?;

            // Skip unconditional packs (empty tool_keywords) in merged artifact
            // They remain available in the packs/ directory for hook mode
            if pack.tool_keywords.is_empty() {
                skipped_packs.push(pack.id.clone());
                eprintln!("ℹ️  Skipping unconditional pack '{}' from merged artifact (will remain in packs/ directory for hook mode)", pack.id);
            } else {
                packs.push(pack);
            }
        }
    }

    if packs.is_empty() {
        return Err(anyhow::anyhow!(
            "No pack files found in directory: {}",
            dir.display()
        ));
    }

    if !skipped_packs.is_empty() {
        eprintln!(
            "ℹ️  Excluded {} unconditional pack(s) from merged artifact: {}",
            skipped_packs.len(),
            skipped_packs.join(", ")
        );
    }

    merge_packs(packs)
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
            enabled: true,
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
                enabled: true,
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
        assert_eq!(
            pack.tool_keywords,
            vec!["vault".to_string(), "git".to_string()]
        );
        assert_eq!(pack.safe_patterns.len(), 5);
        assert_eq!(pack.guarded_patterns.len(), 8);

        // Verify first safe pattern structure
        let safe = &pack.safe_patterns[0];
        assert_eq!(safe.id, "safe-vault-read");
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
        assert!(guarded
            .redirect
            .reason_template
            .contains("permanently destructive"));
        assert!(guarded.redirect.rewrite_template.is_none());

        // Verify git force-push pattern has rewrite_template
        let git_force = &pack.guarded_patterns[3];
        assert_eq!(git_force.id, "git-force-push");
        assert_eq!(git_force.tier, Tier::Tier1);
        assert_eq!(git_force.severity, Severity::Critical);
        assert_eq!(git_force.redirect.channel, Channel::Deny);
        assert!(git_force.redirect.rewrite_template.is_some());
        assert!(git_force
            .redirect
            .rewrite_template
            .as_ref()
            .unwrap()
            .contains("--force-with-lease"));

        // Verify content-mode patterns exist (storage-class, image-tag)
        let storage_class = &pack.guarded_patterns[4];
        assert_eq!(storage_class.id, "storage-class-ssd");
        match &storage_class.check {
            Check::ContentRegex { regex } => {
                assert!(regex.contains("storageClassName"));
            }
            _ => panic!("Expected ContentRegex check"),
        }

        let image_tag = &pack.guarded_patterns[6];
        assert!(image_tag.id.contains("image-tag"));
        match &image_tag.check {
            Check::ContentRegex { regex } => {
                assert!(regex.contains("image"));
            }
            _ => panic!("Expected ContentRegex check"),
        }
    }

    #[test]
    fn test_mixed_unconditional_merge_is_refused() {
        // The secrets pack is unconditional (empty tool_keywords). Merging it
        // with keyword-dispatched packs would fold its whole-command scan
        // behind the merged pack's keyword dispatch and silently drop the
        // coverage it exists for, so the merge must fail loudly.
        let unconditional = Pack {
            id: "secrets".to_string(),
            tool_keywords: vec![],
            applies_to: vec![],
            safe_patterns: vec![],
            guarded_patterns: vec![],
        };
        let keyword_dispatched = Pack {
            id: "git".to_string(),
            tool_keywords: vec!["git".to_string()],
            applies_to: vec![],
            safe_patterns: vec![],
            guarded_patterns: vec![],
        };

        let err = merge_packs(vec![unconditional.clone(), keyword_dispatched])
            .expect_err("mixed merge must be refused");
        assert!(
            err.to_string().contains("unconditional"),
            "error should explain the unconditional-pack loss, got: {err}"
        );
    }

    #[test]
    fn test_uniformly_unconditional_packs_still_merge() {
        let a = Pack {
            id: "secrets".to_string(),
            tool_keywords: vec![],
            applies_to: vec![],
            safe_patterns: vec![],
            guarded_patterns: vec![],
        };
        let b = Pack {
            id: "other-unconditional".to_string(),
            tool_keywords: vec![],
            applies_to: vec![],
            safe_patterns: vec![],
            guarded_patterns: vec![],
        };

        let merged = merge_packs(vec![a, b]).expect("uniform merge should succeed");
        assert!(merged.tool_keywords.is_empty());
    }

    #[test]
    fn test_load_previous_release_fixture() {
        let pack = load_pack("tests/fixtures/previous-release.json")
            .expect("Failed to load previous-release.json");

        assert_eq!(pack.id, "test-pack-previous");
        assert_eq!(
            pack.tool_keywords,
            vec!["vault".to_string(), "git".to_string()]
        );
        assert_eq!(pack.safe_patterns.len(), 5);
        assert_eq!(pack.guarded_patterns.len(), 8);
    }

    /// A minimal guarded rule carrying just the redirect under test.
    fn actionability_rule(reason: &str, rewrite: Option<&str>) -> GuardedPattern {
        GuardedPattern {
            id: "fixture-rule".to_string(),
            enabled: true,
            check: Check::CommandRegex {
                regex: "^fixture-tool destroy".to_string(),
            },
            tier: Tier::Tier1,
            severity: Severity::Critical,
            explanation: "fixture rule for the actionability gate".to_string(),
            redirect: Redirect {
                channel: Channel::Deny,
                reason_template: reason.to_string(),
                rewrite_template: rewrite.map(str::to_string),
            },
            destructive: true,
        }
    }

    #[test]
    fn actionability_rejects_empty_reason() {
        for reason in ["", "   ", " \n\t "] {
            assert_eq!(
                redirect_actionability_violation(&actionability_rule(reason, None)),
                Some(RedirectActionabilityError::EmptyReason),
                "whitespace-only reason {reason:?} must be rejected"
            );
        }
    }

    #[test]
    fn actionability_rejects_block_only_reason() {
        assert_eq!(
            redirect_actionability_violation(&actionability_rule("This command is blocked.", None)),
            Some(RedirectActionabilityError::NonActionable)
        );
    }

    #[test]
    fn actionability_verb_inside_a_prohibition_does_not_count() {
        // The distinguishing negative case: the reason names a directive
        // verb, but only inside its own prohibition. The negation stripper
        // must remove "Do not run" so the reason cannot ride to a pass on
        // the verb it forbids.
        assert_eq!(
            redirect_actionability_violation(&actionability_rule(
                "This operation is dangerous and is not allowed. Do not run it. \
                 There is no alternative.",
                None
            )),
            Some(RedirectActionabilityError::NonActionable)
        );
    }

    #[test]
    fn actionability_directive_verb_passes() {
        assert_eq!(
            redirect_actionability_violation(&actionability_rule(
                "Remote HEAD has moved forward since your last fetch/pull. \
                 Run 'git pull' first to integrate remote changes before pushing.",
                None
            )),
            None
        );
    }

    #[test]
    fn actionability_rewrite_template_counts_as_the_alternative() {
        // An updated_input redirect's rewrite IS the sanctioned alternative,
        // even when the reason prose is only an explanation.
        assert_eq!(
            redirect_actionability_violation(&actionability_rule(
                "Force-push flags can rewrite remote history and lose commits.",
                Some("{command_without_force}")
            )),
            None
        );
    }

    #[test]
    fn actionability_quoted_command_counts_but_possessive_does_not() {
        // 'kv delete' is a quoted command snippet -> actionable.
        assert_eq!(
            redirect_actionability_violation(&actionability_rule(
                "This is an irreversible OpenBao operation. 'kv delete' \
                 soft-deletes and is recoverable.",
                None
            )),
            None
        );
        // A possessive apostrophe is prose, not a quoted snippet.
        assert_eq!(
            redirect_actionability_violation(&actionability_rule(
                "This is the operator's session and it is blocked.",
                None
            )),
            Some(RedirectActionabilityError::NonActionable)
        );
    }

    #[test]
    fn actionability_derived_placeholder_passes() {
        assert_eq!(
            redirect_actionability_violation(&actionability_rule(
                "The :latest image tag is banned. Pin this image to {derived_value}.",
                None
            )),
            None
        );
    }

    #[test]
    fn validate_redirect_actionability_names_pack_rule_and_doctrine() {
        let mut pack = Pack {
            id: "fixture-pack".to_string(),
            tool_keywords: vec!["fixture-tool".to_string()],
            applies_to: vec![],
            safe_patterns: vec![],
            guarded_patterns: vec![actionability_rule("Blocked.", None)],
        };
        let error = validate_redirect_actionability(&pack)
            .expect_err("block-only redirect must fail the gate");
        let message = error.to_string();
        assert!(
            message.contains("fixture-pack.fixture-rule"),
            "error should name pack and rule, got: {message}"
        );
        assert!(
            message.contains("docs/notes/redirect-not-just-block.md"),
            "error should point at the doctrine, got: {message}"
        );

        // An actionable rule validates clean, and an empty one reports
        // EmptyReason through the same entry point.
        pack.guarded_patterns[0].redirect.reason_template =
            "Use 'fixture-tool list' to inspect, then remove one by one.".to_string();
        validate_redirect_actionability(&pack).expect("directive-verb redirect must pass the gate");
    }
}
