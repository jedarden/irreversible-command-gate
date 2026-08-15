//! Fixed deny-regression suite generation for rule-pack manifests.
//!
//! A regression case is deliberately small and data-only: it records the
//! command which exercises one guarded pattern and the verdict that must be
//! returned for that command.  Keeping the generated suite independent from
//! the rule-pack representation makes it suitable for CI artifacts as well as
//! for an in-process test runner.

use crate::rule_pack::{Channel, Check, GuardedPattern, Pack};
use anyhow::{Context, Result, bail};
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

const SUITE_VERSION: u32 = 1;

/// The verdict expected from every fixed regression case.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ExpectedVerdict {
    Deny,
}

/// One fixed regression case, paired with exactly one guarded pattern.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RegressionTestCase {
    /// The pack containing the guarded pattern.
    pub pack_id: String,
    /// The guarded pattern exercised by this command.
    pub pattern_id: String,
    /// A concrete command-shaped input. It is never executed by generation.
    pub command: String,
    /// The verdict that must be returned when the command is evaluated.
    pub expected: ExpectedVerdict,
}

/// A generated fixed regression suite.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RegressionSuite {
    /// Format version for generated suite artifacts.
    pub version: u32,
    /// One case for every guarded pattern in the source pack.
    pub cases: Vec<RegressionTestCase>,
}

impl RegressionSuite {
    /// Return the set of guarded-pattern IDs represented by this suite.
    pub fn pattern_ids(&self) -> HashSet<&str> {
        self.cases
            .iter()
            .map(|case| case.pattern_id.as_str())
            .collect()
    }

    /// Serialize the suite as stable, human-readable JSON.
    pub fn to_json(&self) -> Result<String> {
        serde_json::to_string_pretty(self).context("failed to serialize regression suite")
    }
}

/// Generate and validate a fixed deny suite from a loaded rule pack.
///
/// Commands are taken from an optional `example_command`/`test_command`
/// property in the JSON manifest when present. Otherwise a representative
/// command is synthesized from the command regex and the pack's first tool
/// keyword. Generation fails if a command cannot be produced, if the guarded
/// pattern is not a deny rule, or if the command is not denied by that exact
/// pattern. Those failures keep a rule from entering a release without its
/// required regression case.
pub fn generate_regression_suite(pack: &Pack) -> Result<RegressionSuite> {
    generate_regression_suite_with_examples(pack, &Default::default())
}

/// Generate a suite from a JSON rule-pack manifest on disk.
pub fn generate_regression_suite_from_manifest<P: AsRef<Path>>(path: P) -> Result<RegressionSuite> {
    let path = path.as_ref();
    let content = fs::read_to_string(path)
        .with_context(|| format!("failed to read rule pack from {}", path.display()))?;
    let value: serde_json::Value = serde_json::from_str(&content)
        .with_context(|| format!("failed to parse JSON from {}", path.display()))?;
    let pack: Pack = serde_json::from_value(value.clone())
        .with_context(|| format!("failed to parse rule pack from {}", path.display()))?;
    let examples = example_commands_from_manifest(&value)?;

    generate_regression_suite_with_examples(&pack, &examples)
        .with_context(|| format!("failed to generate suite for {}", path.display()))
}

/// Generate suites for several manifests, preserving manifest and pattern
/// order. This is the form used when a release contains one file per pack.
pub fn generate_regression_suite_from_manifests<P: AsRef<Path>>(
    paths: &[P],
) -> Result<RegressionSuite> {
    let mut cases = Vec::new();
    for path in paths {
        cases.extend(generate_regression_suite_from_manifest(path)?.cases);
    }

    Ok(RegressionSuite {
        version: SUITE_VERSION,
        cases,
    })
}

/// Verify a previously generated suite against a rule pack.
///
/// This is the build-gate operation: it requires an exact one-to-one mapping
/// between guarded patterns and cases, then checks that every recorded command
/// is still denied by its recorded pattern. It catches missing cases as well
/// as a regex change which stops matching the fixed command.
pub fn verify_regression_suite(pack: &Pack, suite: &RegressionSuite) -> Result<()> {
    if suite.version != SUITE_VERSION {
        bail!(
            "unsupported regression suite version {}; expected {}",
            suite.version,
            SUITE_VERSION
        );
    }
    if suite.cases.len() != pack.guarded_patterns.len() {
        bail!(
            "regression suite for pack '{}' has {} cases for {} guarded patterns",
            pack.id,
            suite.cases.len(),
            pack.guarded_patterns.len()
        );
    }

    let mut seen = HashSet::new();
    for case in &suite.cases {
        if case.pack_id != pack.id {
            bail!(
                "regression case '{}' belongs to pack '{}', not '{}'",
                case.pattern_id,
                case.pack_id,
                pack.id
            );
        }
        if !seen.insert(case.pattern_id.as_str()) {
            bail!(
                "regression suite contains duplicate case '{}'",
                case.pattern_id
            );
        }
        let pattern = pack
            .guarded_patterns
            .iter()
            .find(|pattern| pattern.id == case.pattern_id)
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "regression suite contains case for unknown guarded pattern '{}'",
                    case.pattern_id
                )
            })?;
        if case.expected != ExpectedVerdict::Deny {
            bail!("regression case '{}' does not expect deny", case.pattern_id);
        }
        validate_deny_case(pack, pattern, &case.command)?;
    }

    for pattern in &pack.guarded_patterns {
        if !seen.contains(pattern.id.as_str()) {
            bail!(
                "guarded pattern '{}' has no regression test case",
                pattern.id
            );
        }
    }
    Ok(())
}

/// Write a generated suite to a file, creating its parent directory when one
/// is specified.
pub fn write_regression_suite<P: AsRef<Path>>(suite: &RegressionSuite, path: P) -> Result<()> {
    let path = path.as_ref();
    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create output directory {}", parent.display()))?;
    }
    fs::write(path, suite.to_json()?)
        .with_context(|| format!("failed to write regression suite to {}", path.display()))?;
    Ok(())
}

fn generate_regression_suite_with_examples(
    pack: &Pack,
    examples: &std::collections::HashMap<String, String>,
) -> Result<RegressionSuite> {
    if pack.guarded_patterns.is_empty() {
        return Ok(RegressionSuite {
            version: SUITE_VERSION,
            cases: Vec::new(),
        });
    }

    let mut ids = HashSet::new();
    let mut cases = Vec::with_capacity(pack.guarded_patterns.len());

    for pattern in &pack.guarded_patterns {
        if !ids.insert(pattern.id.as_str()) {
            bail!(
                "pack '{}' contains duplicate guarded pattern ID '{}'",
                pack.id,
                pattern.id
            );
        }

        let command = match examples.get(&pattern.id) {
            Some(command) if !command.trim().is_empty() => command.trim().to_owned(),
            Some(_) => bail!(
                "guarded pattern '{}' in pack '{}' has an empty example command",
                pattern.id,
                pack.id
            ),
            None => representative_command(pack, pattern)?,
        };

        validate_deny_case(pack, pattern, &command)?;
        cases.push(RegressionTestCase {
            pack_id: pack.id.clone(),
            pattern_id: pattern.id.clone(),
            command,
            expected: ExpectedVerdict::Deny,
        });
    }

    Ok(RegressionSuite {
        version: SUITE_VERSION,
        cases,
    })
}

fn example_commands_from_manifest(
    value: &serde_json::Value,
) -> Result<std::collections::HashMap<String, String>> {
    let mut examples = std::collections::HashMap::new();
    let Some(patterns) = value
        .get("guarded_patterns")
        .and_then(serde_json::Value::as_array)
    else {
        return Ok(examples);
    };

    for pattern in patterns {
        let Some(id) = pattern.get("id").and_then(serde_json::Value::as_str) else {
            bail!("guarded pattern is missing a string id");
        };

        // `example_command` is the canonical name. The aliases make the
        // generator friendly to early manifests without changing Pack's
        // runtime schema or losing unknown fields during normal loading.
        for key in ["example_command", "test_command", "example"] {
            if let Some(command) = pattern.get(key) {
                let command = command.as_str().ok_or_else(|| {
                    anyhow::anyhow!("guarded pattern '{}' field '{}' must be a string", id, key)
                })?;
                examples.insert(id.to_owned(), command.to_owned());
                break;
            }
        }
    }

    Ok(examples)
}

fn representative_command(pack: &Pack, pattern: &GuardedPattern) -> Result<String> {
    let Check::CommandRegex { regex } = &pattern.check else {
        bail!(
            "guarded pattern '{}' in pack '{}' is not a command_regex; provide a command-mode regression example",
            pattern.id,
            pack.id
        );
    };

    if pack.tool_keywords.is_empty() {
        bail!(
            "guarded pattern '{}' in pack '{}' has no tool_keywords from which to build a regression command",
            pattern.id,
            pack.id
        );
    }

    // Fail early on malformed source instead of emitting a suite which could
    // never be evaluated by the guard.
    Regex::new(regex).with_context(|| {
        format!(
            "invalid regex for guarded pattern '{}': {}",
            pattern.id, regex
        )
    })?;

    let mut command = regex_example(regex);
    let starts_with_keyword = pack.tool_keywords.iter().any(|keyword| {
        command
            .split_whitespace()
            .next()
            .is_some_and(|first| first == keyword)
    });
    if !starts_with_keyword {
        command = format!("{} {}", pack.tool_keywords[0], command);
    }

    if command.trim().is_empty() {
        bail!(
            "could not derive a command for guarded pattern '{}'",
            pattern.id
        );
    }
    Ok(command)
}

/// Turn the useful literal shape of a command regex into a safe, concrete
/// command. This intentionally handles the small regex vocabulary used by
/// command rule packs; unsupported metacharacters become a harmless token and
/// are then checked by `validate_deny_case`.
fn regex_example(regex: &str) -> String {
    let mut output = String::new();
    let chars: Vec<char> = regex.chars().collect();
    let mut index = 0;
    while index < chars.len() {
        let character = chars[index];
        match character {
            '^' | '$' => {}
            '\\' => {
                if let Some(next) = chars.get(index + 1).copied() {
                    match next {
                        'd' => output.push('1'),
                        'D' | 'w' | 'W' => output.push_str("example"),
                        's' => output.push(' '),
                        'S' => output.push_str("example"),
                        _ => output.push(next),
                    }
                    index += 1;
                }
            }
            '[' => {
                let end = chars[index + 1..]
                    .iter()
                    .position(|character| *character == ']')
                    .map(|offset| index + 1 + offset);
                if let Some(end) = end {
                    let first = chars[index + 1..end]
                        .iter()
                        .copied()
                        .find(|character| *character != '^')
                        .unwrap_or('x');
                    output.push(first);
                    index = end;
                } else {
                    output.push('x');
                }
            }
            '(' => {
                // Grouping is not part of the command text. Non-capturing
                // group syntax's '?' is discarded naturally on the next pass.
            }
            ')' | '?' => {}
            '|' => output.push(' '),
            '.' => {
                if chars
                    .get(index + 1)
                    .is_some_and(|next| *next == '*' || *next == '+')
                {
                    if !output.is_empty() && !output.ends_with(' ') {
                        output.push(' ');
                    }
                    output.push_str("example");
                    index += 1;
                    if chars.get(index + 1).is_some_and(|next| *next != ' ') {
                        output.push(' ');
                    }
                } else {
                    output.push('x');
                }
            }
            '*' | '+' => {}
            '{' => {
                // Quantifier body: retain the already-emitted atom once.
                if let Some(end) = chars[index + 1..]
                    .iter()
                    .position(|character| *character == '}')
                {
                    index += end + 1;
                }
            }
            _ => output.push(character),
        }
        index += 1;
    }

    output.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn validate_deny_case(pack: &Pack, target: &GuardedPattern, command: &str) -> Result<()> {
    if target.redirect.channel != Channel::Deny {
        bail!(
            "guarded pattern '{}' in pack '{}' uses {:?}; fixed regression cases require a deny redirect",
            target.id,
            pack.id,
            target.redirect.channel
        );
    }

    for segment in command_segments(command) {
        if !pack.tool_keywords.is_empty() {
            let executable = segment.split_whitespace().next().unwrap_or_default();
            if !pack
                .tool_keywords
                .iter()
                .any(|keyword| keyword == executable)
            {
                continue;
            }
        }

        let mut safe_match = None;
        for pattern in &pack.safe_patterns {
            let Check::CommandRegex { regex } = &pattern.check else {
                continue;
            };
            let regex = Regex::new(regex).with_context(|| {
                format!("invalid regex for safe pattern '{}': {}", pattern.id, regex)
            })?;
            if regex.is_match(&segment) {
                safe_match = Some(regex);
                break;
            }
        }
        if let Some(regex) = safe_match {
            bail!(
                "example command for guarded pattern '{}' in pack '{}' matches a safe pattern ('{}') first: {}",
                target.id,
                pack.id,
                regex.as_str(),
                command
            );
        }

        for pattern in &pack.guarded_patterns {
            let Check::CommandRegex { regex } = &pattern.check else {
                continue;
            };
            let regex = Regex::new(regex).with_context(|| {
                format!(
                    "invalid regex for guarded pattern '{}': {}",
                    pattern.id, regex
                )
            })?;
            if regex.is_match(&segment) {
                if pattern.id == target.id {
                    return Ok(());
                }
                bail!(
                    "example command for guarded pattern '{}' in pack '{}' is caught by guarded pattern '{}' instead: {}",
                    target.id,
                    pack.id,
                    pattern.id,
                    command
                );
            }
        }
    }

    bail!(
        "example command for guarded pattern '{}' in pack '{}' does not match its regex: {}",
        target.id,
        pack.id,
        command
    )
}

/// Normalize the command in the same limited way as command-mode dispatch:
/// split shell segments and remove transparent wrapper prefixes and leading
/// environment assignments. The generated commands do not need this, but it
/// lets a hand-authored `example_command` exercise the real hook shape too.
fn command_segments(command: &str) -> Vec<String> {
    command
        .split(|character| matches!(character, ';' | '&' | '|' | '\n'))
        .filter_map(|segment| {
            let mut tokens: Vec<&str> = segment.split_whitespace().collect();
            while let Some(first) = tokens.first().copied() {
                let is_prefix = matches!(first, "sudo" | "command" | "exec" | "time" | "nohup")
                    || first.split_once('=').is_some_and(|(name, _)| {
                        !name.is_empty()
                            && name.chars().enumerate().all(|(index, character)| {
                                (index == 0
                                    && (character.is_ascii_alphabetic() || character == '_'))
                                    || (index > 0
                                        && (character.is_ascii_alphanumeric() || character == '_'))
                            })
                    });
                if is_prefix {
                    tokens.remove(0);
                } else {
                    break;
                }
            }
            let executable = tokens.first()?.rsplit('/').next()?;
            tokens[0] = executable;
            Some(tokens.join(" "))
        })
        .collect()
}

/// Convenience helper for callers that have a path and want a generated JSON
/// artifact in one operation.
pub fn generate_regression_suite_file<P: AsRef<Path>, Q: AsRef<Path>>(
    manifest: P,
    output: Q,
) -> Result<()> {
    let suite = generate_regression_suite_from_manifest(manifest)?;
    write_regression_suite(&suite, PathBuf::from(output.as_ref()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rule_pack::{Redirect, Severity, Tier};

    fn guarded(id: &str, regex: &str) -> GuardedPattern {
        GuardedPattern {
            id: id.to_owned(),
            check: Check::CommandRegex {
                regex: regex.to_owned(),
            },
            tier: Tier::Tier1,
            severity: Severity::High,
            explanation: "test rule".to_owned(),
            redirect: Redirect {
                channel: Channel::Deny,
                reason_template: "test deny".to_owned(),
                rewrite_template: None,
            },
            destructive: true,
        }
    }

    fn pack(patterns: Vec<GuardedPattern>) -> Pack {
        Pack {
            id: "test-pack".to_owned(),
            tool_keywords: vec!["git".to_owned()],
            applies_to: Vec::new(),
            safe_patterns: Vec::new(),
            guarded_patterns: patterns,
        }
    }

    #[test]
    fn generates_one_deny_case_for_every_guarded_pattern() {
        let suite = generate_regression_suite(&pack(vec![
            guarded("force-push", "git push.*--force"),
            guarded("reset", "git reset --hard"),
        ]))
        .unwrap();

        assert_eq!(suite.version, SUITE_VERSION);
        assert_eq!(suite.cases.len(), 2);
        assert_eq!(suite.pattern_ids(), HashSet::from(["force-push", "reset"]));
        assert!(suite.cases.iter().all(|case| {
            case.expected == ExpectedVerdict::Deny && case.command.starts_with("git ")
        }));
    }

    #[test]
    fn accepts_an_explicit_example_command() {
        let mut examples = std::collections::HashMap::new();
        examples.insert("reset".to_owned(), "git reset --hard HEAD".to_owned());
        let suite = generate_regression_suite_with_examples(
            &pack(vec![guarded("reset", "git reset --hard")]),
            &examples,
        )
        .unwrap();

        assert_eq!(suite.cases[0].command, "git reset --hard HEAD");
    }

    #[test]
    fn rejects_a_non_deny_guarded_pattern() {
        let mut pattern = guarded("warning", "git worktree add");
        pattern.redirect.channel = Channel::AdditionalContext;
        let error = generate_regression_suite(&pack(vec![pattern])).unwrap_err();
        assert!(
            error
                .to_string()
                .contains("fixed regression cases require a deny redirect")
        );
    }

    #[test]
    fn rejects_an_example_shadowed_by_a_safe_pattern() {
        let mut rule_pack = pack(vec![guarded("reset", "git reset --hard")]);
        rule_pack.safe_patterns.push(crate::rule_pack::Pattern {
            id: "all-git".to_owned(),
            check: Check::CommandRegex {
                regex: "git .*".to_owned(),
            },
        });
        let error = generate_regression_suite(&rule_pack).unwrap_err();
        assert!(error.to_string().contains("matches a safe pattern"));
    }

    #[test]
    fn serializes_expected_deny_verdict() {
        let suite =
            generate_regression_suite(&pack(vec![guarded("reset", "git reset --hard")])).unwrap();
        let json = suite.to_json().unwrap();
        assert!(json.contains("\"expected\": \"deny\""));
        assert!(json.contains("\"pattern_id\": \"reset\""));
    }
}
