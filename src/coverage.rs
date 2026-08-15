use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

/// Rule pack manifest structure
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RulePack {
    pub id: String,
    #[serde(default)]
    pub safe_patterns: Vec<Pattern>,
    #[serde(default)]
    pub guarded_patterns: Vec<GuardedPattern>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Pattern {
    pub id: String,
    #[serde(rename = "check")]
    pub check_value: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GuardedPattern {
    pub id: String,
    #[serde(rename = "check")]
    pub check_value: String,
    pub tier: u32,
    pub severity: String,
    pub explanation: String,
    #[serde(default)]
    pub destructive: bool, // If true, this is a destructive_pattern
}

/// Coverage diff result
#[derive(Debug)]
pub struct CoverageDiff {
    pub removed_guarded_patterns: Vec<String>,
    pub widened_safe_patterns: Vec<PatternChange>,
    pub narrowed_destructive_patterns: Vec<PatternChange>,
}

#[derive(Debug)]
pub struct PatternChange {
    pub pattern_id: String,
    pub previous: String,
    pub current: String,
}

impl CoverageDiff {
    pub fn has_regressions(&self) -> bool {
        !self.removed_guarded_patterns.is_empty()
            || !self.widened_safe_patterns.is_empty()
            || !self.narrowed_destructive_patterns.is_empty()
    }
}

/// Load a rule pack manifest from a file
pub fn load_rule_pack(path: std::path::PathBuf) -> Result<RulePack> {
    let content = std::fs::read_to_string(&path)
        .with_context(|| format!("Failed to read rule pack from {}", path.display()))?;

    // Try JSON first, then TOML if that fails
    let pack: RulePack = if path.extension().map_or(false, |e| e == "json") {
        serde_json::from_str(&content)
            .with_context(|| format!("Failed to parse JSON from {}", path.display()))?
    } else {
        // Assume TOML for other extensions
        let _content = content.replace("check =", "check_value =");
        // Basic TOML parsing would go here - for now, assume JSON
        return Err(anyhow::anyhow!("TOML support not yet implemented - please use JSON manifests"));
    };

    Ok(pack)
}

/// Run coverage diff between two rule pack manifests
pub fn run_coverage_diff(
    previous_path: std::path::PathBuf,
    current_path: std::path::PathBuf,
) -> Result<CoverageDiff> {
    let previous = load_rule_pack(previous_path)?;
    let current = load_rule_pack(current_path)?;

    // Build maps by pattern ID for easy lookup
    let previous_guarded: HashMap<&str, &GuardedPattern> = previous
        .guarded_patterns
        .iter()
        .map(|p| (p.id.as_str(), p))
        .collect();

    let current_guarded: HashMap<&str, &GuardedPattern> = current
        .guarded_patterns
        .iter()
        .map(|p| (p.id.as_str(), p))
        .collect();

    let previous_safe: HashMap<&str, &Pattern> = previous
        .safe_patterns
        .iter()
        .map(|p| (p.id.as_str(), p))
        .collect();

    let current_safe: HashMap<&str, &Pattern> = current
        .safe_patterns
        .iter()
        .map(|p| (p.id.as_str(), p))
        .collect();

    // Detect removed guarded patterns
    let previous_ids: HashSet<&str> = previous_guarded.keys().cloned().collect();
    let current_ids: HashSet<&str> = current_guarded.keys().cloned().collect();
    let removed: Vec<String> = previous_ids
        .difference(&current_ids)
        .map(|s| s.to_string())
        .collect();

    // Detect widened safe patterns
    let mut widened_safe = Vec::new();
    for (id, prev_pattern) in &previous_safe {
        if let Some(curr_pattern) = current_safe.get(id) {
            if is_pattern_widened(&prev_pattern.check_value, &curr_pattern.check_value) {
                widened_safe.push(PatternChange {
                    pattern_id: id.to_string(),
                    previous: prev_pattern.check_value.clone(),
                    current: curr_pattern.check_value.clone(),
                });
            }
        }
    }

    // Detect narrowed destructive patterns
    let mut narrowed_destructive = Vec::new();
    for (id, prev_pattern) in &previous_guarded {
        if let Some(curr_pattern) = current_guarded.get(id) {
            // Only check patterns marked as destructive
            if prev_pattern.destructive && curr_pattern.destructive {
                if is_pattern_narrowed(&prev_pattern.check_value, &curr_pattern.check_value) {
                    narrowed_destructive.push(PatternChange {
                        pattern_id: id.to_string(),
                        previous: prev_pattern.check_value.clone(),
                        current: curr_pattern.check_value.clone(),
                    });
                }
            }
        }
    }

    Ok(CoverageDiff {
        removed_guarded_patterns: removed,
        widened_safe_patterns: widened_safe,
        narrowed_destructive_patterns: narrowed_destructive,
    })
}

/// Check if a pattern has been widened (allows more than before)
///
/// For regex patterns:
/// - A pattern is widened if it becomes less specific
/// - Examples: `.*` is wider than `specific-thing-.*`
/// - We detect this by checking if the new pattern is a superset of the old
pub fn is_pattern_widened(previous: &str, current: &str) -> bool {
    // Simple heuristic: if current is more permissive than previous
    // This is a conservative check - we flag suspicious changes

    // If current contains ".*" and previous doesn't, it's likely widened
    let current_has_wildcard = current.contains(".*") || current.contains(".+");
    let previous_has_wildcard = previous.contains(".*") || previous.contains(".+");

    // If previous didn't have wildcards but current does, likely widened
    if !previous_has_wildcard && current_has_wildcard {
        return true;
    }

    // If current is literally ".*" or similar catch-all, it's widened
    if current == ".*" || current == ".+" || current == ".*?" {
        return previous != ".*" && previous != ".+";
    }

    // Character class relaxation: [abc] -> . or [a-z]
    // This is a simplified check - real implementation would need regex parsing
    let prev_char_class = previous.matches('[').count();
    let curr_char_class = current.matches('[').count();
    if curr_char_class < prev_char_class {
        return true;
    }

    false
}

/// Check if a destructive pattern has been narrowed (catches less than before)
///
/// For regex patterns:
/// - A pattern is narrowed if it becomes more specific
/// - Examples: `dangerous-thing-.*` narrowed to `dangerous-thing-specific`
/// - This is dangerous because it might miss new variants
pub fn is_pattern_narrowed(previous: &str, current: &str) -> bool {
    // Reverse logic of widened - we're looking for patterns that became MORE specific

    // If previous had wildcards but current doesn't, it's narrowed
    let previous_has_wildcard = previous.contains(".*") || previous.contains(".+");
    let current_has_wildcard = current.contains(".*") || current.contains(".+");

    if previous_has_wildcard && !current_has_wildcard {
        return true;
    }

    // If previous was catch-all but current isn't, it's narrowed
    if (previous == ".*" || previous == ".+") && current != ".*" && current != ".+" {
        return true;
    }

    // Character class restriction: . -> [abc] or [a-z] -> [ab]
    let prev_char_class = previous.matches('[').count();
    let curr_char_class = current.matches('[').count();
    if curr_char_class > prev_char_class {
        return true;
    }

    // Added constraints (more anchors, more specific literals)
    // If current has more ^ or $ anchors, it's more specific
    let prev_anchors = previous.matches('^').count() + previous.matches('$').count();
    let curr_anchors = current.matches('^').count() + current.matches('$').count();
    if curr_anchors > prev_anchors {
        return true;
    }

    false
}
