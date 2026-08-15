use crate::rule_pack::{Check, GuardedPattern, Pack, Pattern as RulePattern};
use anyhow::Result;
use std::collections::{HashMap, HashSet};
use std::fmt::Write as _;
use std::path::Path;

/// The stable, human-readable coverage-diff report format.
pub const COVERAGE_DIFF_REPORT_FORMAT: &str = "coverage-diff/v1";

/// Coverage diff result
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoverageDiff {
    /// IDs are retained as a convenience for callers that only need membership.
    pub removed_guarded_patterns: Vec<String>,
    /// Removed entries include their previous value so the report can show the
    /// exact coverage that disappeared.
    pub removed_guarded_pattern_changes: Vec<PatternChange>,
    pub widened_safe_patterns: Vec<PatternChange>,
    /// These are guarded_patterns whose `destructive` flag is true in both
    /// manifests and whose check became narrower.
    pub narrowed_guarded_patterns: Vec<PatternChange>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PatternChange {
    pub pattern_id: String,
    pub previous: String,
    pub current: String,
}

impl CoverageDiff {
    pub fn has_regressions(&self) -> bool {
        !self.removed_guarded_patterns.is_empty()
            || !self.widened_safe_patterns.is_empty()
            || !self.narrowed_guarded_patterns.is_empty()
    }

    /// Return whether a supplied justification is explicit and non-blank.
    pub fn has_explicit_justification(justification: Option<&str>) -> bool {
        justification.is_some_and(|value| !value.trim().is_empty())
    }
}

/// Render a coverage-diff report for Layer 2 review.
///
/// The report is intentionally Markdown with stable field names. It is easy
/// for a reviewer to scan and remains useful when attached to a CI job or
/// release record. A regression report always includes a visible
/// `justification` field; callers must provide a non-blank value before the
/// release can be approved.
pub fn render_coverage_diff_report(
    previous_path: &Path,
    current_path: &Path,
    diff: &CoverageDiff,
    justification: Option<&str>,
) -> String {
    let mut report = String::new();
    let status = if diff.has_regressions() {
        "regressions_detected"
    } else {
        "no_regressions"
    };
    let justification = match justification.map(str::trim) {
        Some(value) if !value.is_empty() => value,
        _ if diff.has_regressions() => {
            "REQUIRED: provide --justification with the release approval rationale"
        }
        _ => "not required (no coverage regressions detected)",
    };

    writeln!(report, "# Coverage Diff Report").unwrap();
    writeln!(report).unwrap();
    writeln!(report, "format: {COVERAGE_DIFF_REPORT_FORMAT}").unwrap();
    writeln!(report, "previous_manifest: {}", previous_path.display()).unwrap();
    writeln!(report, "current_manifest: {}", current_path.display()).unwrap();
    writeln!(report, "status: {status}").unwrap();
    writeln!(report, "justification: {justification}").unwrap();
    writeln!(report).unwrap();

    writeln!(report, "## Removed guarded_patterns").unwrap();
    if diff.removed_guarded_pattern_changes.is_empty() {
        writeln!(report, "None.").unwrap();
    } else {
        for change in &diff.removed_guarded_pattern_changes {
            write_change(&mut report, change);
        }
    }
    writeln!(report).unwrap();

    writeln!(report, "## Widened safe_patterns").unwrap();
    if diff.widened_safe_patterns.is_empty() {
        writeln!(report, "None.").unwrap();
    } else {
        for change in &diff.widened_safe_patterns {
            write_change(&mut report, change);
        }
    }
    writeln!(report).unwrap();

    writeln!(report, "## Narrowed guarded_patterns (destructive: true)").unwrap();
    if diff.narrowed_guarded_patterns.is_empty() {
        writeln!(report, "None.").unwrap();
    } else {
        for change in &diff.narrowed_guarded_patterns {
            write_change(&mut report, change);
        }
    }
    writeln!(report).unwrap();

    if diff.has_regressions() {
        writeln!(
            report,
            "Review action: explicit justification is required before approval."
        )
        .unwrap();
    } else {
        writeln!(
            report,
            "Review action: no coverage regression was detected."
        )
        .unwrap();
    }

    report
}

fn write_change(report: &mut String, change: &PatternChange) {
    writeln!(report, "- pattern_id: {}", change.pattern_id).unwrap();
    writeln!(report, "  previous: {}", change.previous).unwrap();
    writeln!(report, "  current: {}", change.current).unwrap();
}

/// Load a rule pack manifest from a file
pub fn load_rule_pack(path: std::path::PathBuf) -> Result<Pack> {
    crate::rule_pack::load_pack(path)
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

    let previous_safe: HashMap<&str, &RulePattern> = previous
        .safe_patterns
        .iter()
        .map(|p| (p.id.as_str(), p))
        .collect();

    let current_safe: HashMap<&str, &RulePattern> = current
        .safe_patterns
        .iter()
        .map(|p| (p.id.as_str(), p))
        .collect();

    // Detect removed guarded patterns
    let previous_ids: HashSet<&str> = previous_guarded.keys().cloned().collect();
    let current_ids: HashSet<&str> = current_guarded.keys().cloned().collect();
    let mut removed: Vec<String> = previous_ids
        .difference(&current_ids)
        .map(|s| s.to_string())
        .collect();
    removed.sort();

    let mut removed_changes: Vec<PatternChange> = removed
        .iter()
        .filter_map(|id| {
            previous_guarded
                .get(id.as_str())
                .map(|pattern| PatternChange {
                    pattern_id: id.clone(),
                    previous: extract_regex(&pattern.check),
                    current: "<removed>".to_string(),
                })
        })
        .collect();
    removed_changes.sort_by(|left, right| left.pattern_id.cmp(&right.pattern_id));

    // Detect widened safe patterns
    let mut widened_safe = Vec::new();
    for (id, prev_pattern) in &previous_safe {
        if let Some(curr_pattern) = current_safe.get(id) {
            let prev_regex = extract_regex(&prev_pattern.check);
            let curr_regex = extract_regex(&curr_pattern.check);
            if is_pattern_widened(&prev_regex, &curr_regex) {
                widened_safe.push(PatternChange {
                    pattern_id: id.to_string(),
                    previous: prev_regex,
                    current: curr_regex,
                });
            }
        }
    }
    widened_safe.sort_by(|left, right| left.pattern_id.cmp(&right.pattern_id));

    // Detect narrowed guarded_patterns where destructive: true
    let mut narrowed_guarded = Vec::new();
    for (id, prev_pattern) in &previous_guarded {
        if let Some(curr_pattern) = current_guarded.get(id) {
            // Only check patterns marked as destructive
            if prev_pattern.destructive && curr_pattern.destructive {
                let prev_regex = extract_regex(&prev_pattern.check);
                let curr_regex = extract_regex(&curr_pattern.check);
                if is_pattern_narrowed(&prev_regex, &curr_regex) {
                    narrowed_guarded.push(PatternChange {
                        pattern_id: id.to_string(),
                        previous: prev_regex,
                        current: curr_regex,
                    });
                }
            }
        }
    }
    narrowed_guarded.sort_by(|left, right| left.pattern_id.cmp(&right.pattern_id));

    Ok(CoverageDiff {
        removed_guarded_patterns: removed,
        removed_guarded_pattern_changes: removed_changes,
        widened_safe_patterns: widened_safe,
        narrowed_guarded_patterns: narrowed_guarded,
    })
}

/// Extract the regex string from a Check enum
fn extract_regex(check: &Check) -> String {
    match check {
        Check::CommandRegex { regex } => regex.clone(),
        Check::ContentRegex { regex } => regex.clone(),
        Check::Predicate { predicate_name } => predicate_name.clone(),
    }
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
