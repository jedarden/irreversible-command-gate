//! Fixed deny-regression suite generation for rule-pack manifests.
//!
//! A regression case is deliberately small and data-only: it records the
//! command which exercises one guarded pattern and the verdict that must be
//! returned for that command.  Keeping the generated suite independent from
//! the rule-pack representation makes it suitable for CI artifacts as well as
//! for an in-process test runner.

use crate::engine::ContentSource;
use crate::rule_pack::{Channel, Check, GuardedPattern, Pack};
use anyhow::{bail, Context, Result};
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

const SUITE_VERSION: u32 = 1;

/// Default upper bound for the opt-in, traffic-derived corpus in one pack.
///
/// The recorder is intentionally bounded in addition to deduplicating exact
/// repeats. A curation pass can raise or lower this limit deliberately; the
/// hook must not grow a permanent unbounded CI dependency by accident.
pub const DEFAULT_RECORDED_CASE_LIMIT: usize = 256;

/// The verdict expected from every fixed regression case.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ExpectedVerdict {
    Deny,
}

/// One fixed regression case, paired with exactly one enabled guarded pattern.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RegressionTestCase {
    /// The pack containing the guarded pattern.
    pub pack_id: String,
    /// The guarded pattern exercised by this command.
    pub pattern_id: String,
    /// A concrete command-shaped input. It is never executed by generation.
    /// This is empty for a content-mode case, which uses `content` below.
    pub command: String,
    /// Write/Edit target used by content-mode guarded patterns.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file_path: Option<String>,
    /// Concrete content-shaped input for content-mode guarded patterns.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
    /// The verdict that must be returned when the command is evaluated.
    pub expected: ExpectedVerdict,
}

/// A generated fixed regression suite.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RegressionSuite {
    /// Format version for generated suite artifacts.
    pub version: u32,
    /// One case for every enabled guarded pattern in the source pack.
    pub cases: Vec<RegressionTestCase>,
}

/// Result of attempting to add one traffic-derived regression case.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecordOutcome {
    /// A new JSONL case was appended.
    Added,
    /// An identical case was already present in the pack corpus.
    Duplicate,
    /// The bounded corpus is full and needs explicit curation.
    CapacityReached,
}

/// Summary returned by the explicit curation/pruning pass.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PruneReport {
    /// Number of `.cases` files inspected and rewritten.
    pub files_rewritten: usize,
    /// Number of duplicate or over-limit cases removed.
    pub cases_removed: usize,
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
/// required regression case. Disabled guarded patterns are intentionally
/// omitted: their release-integrity transition is reviewed separately, and
/// they are not expected to deny while disabled.
pub fn generate_regression_suite(pack: &Pack) -> Result<RegressionSuite> {
    generate_regression_suite_with_examples(pack, &Default::default())
}

/// Generate a suite from a JSON rule-pack manifest on disk.
pub fn generate_regression_suite_from_manifest<P: AsRef<Path>>(path: P) -> Result<RegressionSuite> {
    let path = path.as_ref();
    let (pack, examples) = load_manifest_and_examples(path)?;

    generate_regression_suite_with_inputs(&pack, &examples)
        .with_context(|| format!("failed to generate suite for {}", path.display()))
}

/// Generate the fixed deny corpus used to gate a production pack release.
///
/// A release ships the modular `packs/` directory, so this reads each source
/// manifest directly instead of serializing it through the legacy merged
/// `rule-pack.json` compatibility artifact. That preserves hand-authored
/// regression examples, which are intentionally not part of the runtime Pack
/// schema. Only enabled, destructive deny rules with a regex check participate:
/// predicate rules require live state and UpdatedInput rules do not deny.
///
/// `path` may name one manifest or a directory of JSON manifests. Directory
/// entries are processed in lexical order to keep the emitted artifact stable.
pub fn generate_release_regression_suite<P: AsRef<Path>>(path: P) -> Result<RegressionSuite> {
    let paths = release_manifest_paths(path.as_ref())?;
    let mut cases = Vec::new();

    for path in paths {
        let (mut pack, examples) = load_manifest_and_examples(&path)?;
        pack.guarded_patterns
            .retain(requires_release_regression_case);
        if pack.guarded_patterns.is_empty() {
            continue;
        }

        let suite = generate_regression_suite_with_inputs(&pack, &examples)
            .with_context(|| format!("failed to generate release suite for {}", path.display()))?;
        cases.extend(suite.cases);
    }

    if cases.is_empty() {
        bail!(
            "no enabled destructive deny regex rules found in {}",
            path.as_ref().display()
        );
    }

    Ok(RegressionSuite {
        version: SUITE_VERSION,
        cases,
    })
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

fn load_manifest_and_examples(
    path: &Path,
) -> Result<(Pack, std::collections::HashMap<String, RegressionInput>)> {
    let content = fs::read_to_string(path)
        .with_context(|| format!("failed to read rule pack from {}", path.display()))?;
    let value: serde_json::Value = serde_json::from_str(&content)
        .with_context(|| format!("failed to parse JSON from {}", path.display()))?;
    let pack: Pack = serde_json::from_value(value.clone())
        .with_context(|| format!("failed to parse rule pack from {}", path.display()))?;
    let examples = example_inputs_from_manifest(&value)?;
    Ok((pack, examples))
}

fn release_manifest_paths(path: &Path) -> Result<Vec<PathBuf>> {
    if path.is_file() {
        return Ok(vec![path.to_path_buf()]);
    }
    if !path.is_dir() {
        bail!(
            "release regression source {} does not exist",
            path.display()
        );
    }

    let mut paths = fs::read_dir(path)
        .with_context(|| format!("failed to read release pack directory {}", path.display()))?
        .map(|entry| entry.map(|entry| entry.path()))
        .collect::<std::io::Result<Vec<_>>>()?;
    paths.retain(|entry| {
        entry.is_file()
            && entry.extension().and_then(|extension| extension.to_str()) == Some("json")
    });
    paths.sort();
    if paths.is_empty() {
        bail!(
            "release pack directory {} contains no JSON manifests",
            path.display()
        );
    }
    Ok(paths)
}

fn requires_release_regression_case(pattern: &GuardedPattern) -> bool {
    pattern.enabled
        && pattern.destructive
        && pattern.redirect.channel == Channel::Deny
        && matches!(
            &pattern.check,
            Check::CommandRegex { .. } | Check::ContentRegex { .. }
        )
}

/// Verify a previously generated suite against a rule pack.
///
/// This is the build-gate operation: it requires an exact one-to-one mapping
/// between enabled guarded patterns and cases, then checks that every recorded command
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
    let enabled_pattern_count = pack
        .guarded_patterns
        .iter()
        .filter(|pattern| pattern.enabled)
        .count();
    if suite.cases.len() != enabled_pattern_count {
        bail!(
            "regression suite for pack '{}' has {} cases for {} enabled guarded patterns",
            pack.id,
            suite.cases.len(),
            enabled_pattern_count
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
        if !pattern.enabled {
            bail!(
                "regression suite contains case for disabled guarded pattern '{}'",
                case.pattern_id
            );
        }
        if case.expected != ExpectedVerdict::Deny {
            bail!("regression case '{}' does not expect deny", case.pattern_id);
        }
        let input = match (&case.file_path, &case.content) {
            (Some(file_path), Some(content)) => RegressionInput::Content {
                file_path: file_path.clone(),
                content: content.clone(),
            },
            (None, None) => RegressionInput::Command(case.command.clone()),
            _ => bail!(
                "regression case '{}' must provide both file_path and content for content-mode",
                case.pattern_id
            ),
        };
        validate_deny_input(pack, pattern, &input)?;
    }

    for pattern in pack
        .guarded_patterns
        .iter()
        .filter(|pattern| pattern.enabled)
    {
        if !seen.contains(pattern.id.as_str()) {
            bail!(
                "enabled guarded pattern '{}' has no regression test case",
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

/// Append a deny observed by the opt-in hook recorder to a pack-local corpus.
///
/// The corpus is newline-delimited JSON so each deny can be appended without
/// rewriting the existing file. Exact duplicate cases are ignored, and a
/// bounded per-file limit prevents unattended traffic from creating an
/// unbounded CI artifact. The resulting file is intentionally separate from
/// the generated one-case-per-rule `RegressionSuite`; it is evidence for a
/// later curation pass, not an automatic release gate by itself.
pub fn record_denial_as_test<P: AsRef<Path>>(
    directory: P,
    case: RegressionTestCase,
) -> Result<RecordOutcome> {
    record_denial_as_test_with_limit(directory, case, DEFAULT_RECORDED_CASE_LIMIT)
}

/// Variant of [`record_denial_as_test`] with an explicit bound for tests and
/// maintenance tooling.
pub fn record_denial_as_test_with_limit<P: AsRef<Path>>(
    directory: P,
    case: RegressionTestCase,
    limit: usize,
) -> Result<RecordOutcome> {
    if limit == 0 {
        bail!("recorded regression case limit must be greater than zero");
    }
    let path = recorded_cases_path(directory.as_ref(), &case.pack_id)?;
    let existing = read_recorded_cases(&path)?;

    if existing.iter().any(|previous| previous == &case) {
        return Ok(RecordOutcome::Duplicate);
    }
    if existing.len() >= limit {
        return Ok(RecordOutcome::CapacityReached);
    }

    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).with_context(|| {
            format!("failed to create regression directory {}", parent.display())
        })?;
    }

    let line =
        serde_json::to_string(&case).context("failed to serialize recorded regression case")?;
    let needs_separator = fs::metadata(&path)
        .map(|metadata| metadata.len() > 0)
        .unwrap_or(false)
        && !fs::read(&path)
            .with_context(|| format!("failed to read regression corpus {}", path.display()))?
            .ends_with(b"\n");
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .with_context(|| format!("failed to open regression corpus {}", path.display()))?;
    if needs_separator {
        file.write_all(b"\n")
            .with_context(|| format!("failed to append to regression corpus {}", path.display()))?;
    }
    file.write_all(line.as_bytes())
        .and_then(|_| file.write_all(b"\n"))
        .with_context(|| format!("failed to append to regression corpus {}", path.display()))?;

    Ok(RecordOutcome::Added)
}

/// Remove exact duplicates and trim pack-local traffic corpora to `limit`.
///
/// This operation is deliberately explicit: operators or pack authors should
/// review the cases before running it, then commit the curated result when it
/// is intended to become part of a release. Files are kept in their original
/// order so the first observed representative wins.
pub fn prune_recorded_cases<P: AsRef<Path>>(directory: P, limit: usize) -> Result<PruneReport> {
    prune_recorded_cases_internal(directory.as_ref(), None, limit)
}

/// Curate pack-local traffic corpora against the current rule-pack manifests.
///
/// A case is retained only when its input is still denied by an enabled
/// `guarded_pattern` in the matching current pack. This removes cases for
/// deleted or disabled rules and cases whose command/content no longer reaches
/// a deny rule. If a rule was renamed but still denies the observed input, the
/// case is reassigned to the current rule ID before deduplication. This is the
/// form release maintainers should use periodically; the pack-agnostic helper
/// remains useful for a structural cleanup when the current manifests are not
/// available.
pub fn prune_recorded_cases_against_packs<P: AsRef<Path>>(
    directory: P,
    packs: &[Pack],
    limit: usize,
) -> Result<PruneReport> {
    prune_recorded_cases_internal(directory.as_ref(), Some(packs), limit)
}

fn prune_recorded_cases_internal(
    directory: &Path,
    packs: Option<&[Pack]>,
    limit: usize,
) -> Result<PruneReport> {
    if limit == 0 {
        bail!("pruned regression case limit must be greater than zero");
    }
    let paths = recorded_case_files(directory)?;
    let mut report = PruneReport::default();

    for path in paths {
        let original = read_recorded_cases(&path)?;
        let original_count = original.len();
        let mut retained = Vec::with_capacity(original.len().min(limit));
        for case in original.iter().cloned() {
            let Some(case) = curate_case(case, packs)? else {
                continue;
            };
            if retained.iter().any(|previous| previous == &case) {
                continue;
            }
            if retained.len() < limit {
                retained.push(case);
            }
        }

        let removed = original_count.saturating_sub(retained.len());
        let changed = removed > 0 || retained != original;
        if !changed {
            continue;
        }

        let contents = retained
            .iter()
            .map(serde_json::to_string)
            .collect::<Result<Vec<_>, _>>()?
            .join("\n");
        let contents = if contents.is_empty() {
            String::new()
        } else {
            format!("{contents}\n")
        };
        fs::write(&path, contents).with_context(|| {
            format!(
                "failed to write curated regression corpus {}",
                path.display()
            )
        })?;
        report.files_rewritten += 1;
        report.cases_removed += removed;
    }

    Ok(report)
}

/// Return the current representation of a recorded case, or `None` when no
/// current deny rule can reach it. With no pack set, this performs only the
/// structural checks shared by the legacy pack-agnostic pruning command.
fn curate_case(
    case: RegressionTestCase,
    packs: Option<&[Pack]>,
) -> Result<Option<RegressionTestCase>> {
    let Some(packs) = packs else {
        return Ok(Some(case));
    };

    if case.expected != ExpectedVerdict::Deny {
        return Ok(None);
    }
    let Some(pack) = packs.iter().find(|pack| pack.id == case.pack_id) else {
        return Ok(None);
    };
    let Some(pattern) = reachable_deny_pattern(pack, &case)? else {
        return Ok(None);
    };

    let mut curated = case;
    curated.pattern_id = pattern.id.clone();
    Ok(Some(curated))
}

/// Find the first current deny pattern that would reach a recorded input.
/// Regexes are compiled here rather than treating a malformed manifest as a
/// non-match, so a curation run cannot silently discard evidence because of a
/// broken rule pack.
fn reachable_deny_pattern<'a>(
    pack: &'a Pack,
    case: &RegressionTestCase,
) -> Result<Option<&'a GuardedPattern>> {
    let input = match (&case.file_path, &case.content) {
        (Some(file_path), Some(content)) => RegressionInput::Content {
            file_path: file_path.clone(),
            content: content.clone(),
        },
        (None, None) => RegressionInput::Command(case.command.clone()),
        _ => return Ok(None),
    };

    match input {
        RegressionInput::Command(command) => {
            reachable_command_pattern(pack, &command, &case.pattern_id)
        }
        RegressionInput::Content { file_path, content } => {
            reachable_content_pattern(pack, &file_path, &content, &case.pattern_id)
        }
    }
}

fn reachable_command_pattern<'a>(
    pack: &'a Pack,
    command: &str,
    recorded_pattern_id: &str,
) -> Result<Option<&'a GuardedPattern>> {
    if pack.tool_keywords.is_empty() {
        if command_matches_safe_pattern(pack, command)? {
            return Ok(None);
        }
        for pattern in &pack.guarded_patterns {
            if !pattern.enabled || pattern.redirect.channel != Channel::Deny {
                continue;
            }
            match &pattern.check {
                Check::CommandRegex { regex } => {
                    if Regex::new(regex)
                        .with_context(|| {
                            format!(
                                "invalid regex for guarded pattern '{}': {regex}",
                                pattern.id
                            )
                        })?
                        .is_match(command)
                    {
                        return Ok(Some(pattern));
                    }
                }
                Check::Predicate { .. } if pattern.id == recorded_pattern_id => {
                    // Predicate reachability depends on runtime state and
                    // cannot be reconstructed safely from a corpus case. A
                    // same-ID predicate is still a live owner for the case.
                    return Ok(Some(pattern));
                }
                Check::ContentRegex { .. } | Check::Predicate { .. } => {}
            }
        }
        return Ok(None);
    }

    for segment in command_segments(command) {
        let executable = segment.split_whitespace().next().unwrap_or_default();
        if !pack
            .tool_keywords
            .iter()
            .any(|keyword| keyword == executable)
        {
            continue;
        }

        if command_matches_safe_pattern(pack, &segment)? {
            continue;
        }

        for pattern in &pack.guarded_patterns {
            if !pattern.enabled || pattern.redirect.channel != Channel::Deny {
                continue;
            }
            match &pattern.check {
                Check::CommandRegex { regex } => {
                    if Regex::new(regex)
                        .with_context(|| {
                            format!(
                                "invalid regex for guarded pattern '{}': {regex}",
                                pattern.id
                            )
                        })?
                        .is_match(&segment)
                    {
                        return Ok(Some(pattern));
                    }
                }
                Check::Predicate { .. } if pattern.id == recorded_pattern_id => {
                    return Ok(Some(pattern));
                }
                Check::ContentRegex { .. } | Check::Predicate { .. } => {}
            }
        }
    }
    Ok(None)
}

fn command_matches_safe_pattern(pack: &Pack, command: &str) -> Result<bool> {
    for pattern in &pack.safe_patterns {
        let Check::CommandRegex { regex } = &pattern.check else {
            continue;
        };
        if Regex::new(regex)
            .with_context(|| format!("invalid regex for safe pattern '{}': {regex}", pattern.id))?
            .is_match(command)
        {
            return Ok(true);
        }
    }
    Ok(false)
}

fn reachable_content_pattern<'a>(
    pack: &'a Pack,
    file_path: &str,
    content: &str,
    recorded_pattern_id: &str,
) -> Result<Option<&'a GuardedPattern>> {
    if pack.applies_to.is_empty()
        || !pack.applies_to.iter().any(|glob| {
            matches!(glob.as_str(), "Write" | "Edit")
                || ContentSource::Write {
                    file_path: file_path.to_owned(),
                    content: content.to_owned(),
                }
                .matches_glob(glob)
        })
    {
        return Ok(None);
    }

    for pattern in &pack.safe_patterns {
        let Check::ContentRegex { regex } = &pattern.check else {
            continue;
        };
        if Regex::new(regex)
            .with_context(|| format!("invalid regex for safe pattern '{}': {regex}", pattern.id))?
            .is_match(content)
        {
            return Ok(None);
        }
    }

    for pattern in &pack.guarded_patterns {
        if !pattern.enabled || pattern.redirect.channel != Channel::Deny {
            continue;
        }
        match &pattern.check {
            Check::ContentRegex { regex } => {
                if Regex::new(regex)
                    .with_context(|| {
                        format!(
                            "invalid regex for guarded pattern '{}': {regex}",
                            pattern.id
                        )
                    })?
                    .is_match(content)
                {
                    return Ok(Some(pattern));
                }
            }
            Check::Predicate { .. } if pattern.id == recorded_pattern_id => {
                return Ok(Some(pattern));
            }
            Check::CommandRegex { .. } | Check::Predicate { .. } => {}
        }
    }
    Ok(None)
}

fn recorded_cases_path(directory: &Path, pack_id: &str) -> Result<PathBuf> {
    if pack_id.is_empty()
        || pack_id == "."
        || pack_id == ".."
        || !pack_id.chars().all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | '.')
        })
    {
        bail!("invalid pack id for regression corpus path: '{pack_id}'");
    }

    // Accept an explicit .cases file as a convenience for one-pack tooling;
    // the normal hook form supplies the corpus directory.
    if directory
        .extension()
        .and_then(|extension| extension.to_str())
        == Some("cases")
    {
        return Ok(directory.to_path_buf());
    }
    Ok(directory.join(format!("{pack_id}.cases")))
}

fn recorded_case_files(directory: &Path) -> Result<Vec<PathBuf>> {
    if directory
        .extension()
        .and_then(|extension| extension.to_str())
        == Some("cases")
    {
        return if directory.exists() {
            Ok(vec![directory.to_path_buf()])
        } else {
            Ok(Vec::new())
        };
    }
    if !directory.exists() {
        return Ok(Vec::new());
    }

    let mut paths = fs::read_dir(directory)
        .with_context(|| {
            format!(
                "failed to read regression directory {}",
                directory.display()
            )
        })?
        .map(|entry| entry.map(|entry| entry.path()))
        .collect::<std::io::Result<Vec<_>>>()?;
    paths.retain(|path| path.extension().and_then(|extension| extension.to_str()) == Some("cases"));
    paths.sort();
    Ok(paths)
}

fn read_recorded_cases(path: &Path) -> Result<Vec<RegressionTestCase>> {
    if !path.exists() {
        return Ok(Vec::new());
    }
    let contents = fs::read_to_string(path)
        .with_context(|| format!("failed to read regression corpus {}", path.display()))?;
    contents
        .lines()
        .enumerate()
        .filter(|(_, line)| !line.trim().is_empty())
        .map(|(index, line)| {
            serde_json::from_str(line).with_context(|| {
                format!(
                    "failed to parse regression corpus {} line {}",
                    path.display(),
                    index + 1
                )
            })
        })
        .collect()
}

fn generate_regression_suite_with_examples(
    pack: &Pack,
    examples: &std::collections::HashMap<String, String>,
) -> Result<RegressionSuite> {
    let inputs = examples
        .iter()
        .map(|(id, command)| (id.clone(), RegressionInput::Command(command.clone())))
        .collect();
    generate_regression_suite_with_inputs(pack, &inputs)
}

#[derive(Debug, Clone)]
enum RegressionInput {
    Command(String),
    Content { file_path: String, content: String },
}

fn generate_regression_suite_with_inputs(
    pack: &Pack,
    examples: &std::collections::HashMap<String, RegressionInput>,
) -> Result<RegressionSuite> {
    if pack.guarded_patterns.is_empty() {
        return Ok(RegressionSuite {
            version: SUITE_VERSION,
            cases: Vec::new(),
        });
    }

    let mut ids = HashSet::new();
    let enabled_pattern_count = pack
        .guarded_patterns
        .iter()
        .filter(|pattern| pattern.enabled)
        .count();
    let mut cases = Vec::with_capacity(enabled_pattern_count);

    for pattern in &pack.guarded_patterns {
        if !ids.insert(pattern.id.as_str()) {
            bail!(
                "pack '{}' contains duplicate guarded pattern ID '{}'",
                pack.id,
                pattern.id
            );
        }
        if !pattern.enabled {
            continue;
        }

        let input = match examples.get(&pattern.id) {
            Some(RegressionInput::Command(command)) if !command.trim().is_empty() => {
                RegressionInput::Command(command.trim().to_owned())
            }
            Some(RegressionInput::Content { file_path, content })
                if !file_path.trim().is_empty() && !content.trim().is_empty() =>
            {
                RegressionInput::Content {
                    file_path: file_path.trim().to_owned(),
                    content: content.trim().to_owned(),
                }
            }
            Some(_) => bail!(
                "guarded pattern '{}' in pack '{}' has an empty regression example",
                pattern.id,
                pack.id
            ),
            None => representative_input(pack, pattern)?,
        };

        validate_deny_input(pack, pattern, &input)?;
        cases.push(RegressionTestCase {
            pack_id: pack.id.clone(),
            pattern_id: pattern.id.clone(),
            command: match &input {
                RegressionInput::Command(command) => command.clone(),
                RegressionInput::Content { .. } => String::new(),
            },
            file_path: match &input {
                RegressionInput::Content { file_path, .. } => Some(file_path.clone()),
                RegressionInput::Command(_) => None,
            },
            content: match &input {
                RegressionInput::Content { content, .. } => Some(content.clone()),
                RegressionInput::Command(_) => None,
            },
            expected: ExpectedVerdict::Deny,
        });
    }

    Ok(RegressionSuite {
        version: SUITE_VERSION,
        cases,
    })
}

fn example_inputs_from_manifest(
    value: &serde_json::Value,
) -> Result<std::collections::HashMap<String, RegressionInput>> {
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

        // `example_command` is the canonical command example. The aliases
        // keep the generator friendly to early manifests without changing
        // Pack's runtime schema or losing unknown fields during normal loading.
        for key in ["example_command", "test_command", "example"] {
            if let Some(command) = pattern.get(key) {
                let command = command.as_str().ok_or_else(|| {
                    anyhow::anyhow!("guarded pattern '{}' field '{}' must be a string", id, key)
                })?;
                examples.insert(id.to_owned(), RegressionInput::Command(command.to_owned()));
                break;
            }
        }
        if examples.contains_key(id) {
            continue;
        }
        if let Some(content) = pattern
            .get("example_content")
            .or_else(|| pattern.get("test_content"))
        {
            let content = content.as_str().ok_or_else(|| {
                anyhow::anyhow!("guarded pattern '{}' content example must be a string", id)
            })?;
            let file_path = pattern
                .get("example_file_path")
                .or_else(|| pattern.get("file_path"))
                .and_then(serde_json::Value::as_str)
                .unwrap_or("regression.yaml");
            examples.insert(
                id.to_owned(),
                RegressionInput::Content {
                    file_path: file_path.to_owned(),
                    content: content.to_owned(),
                },
            );
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

fn representative_input(pack: &Pack, pattern: &GuardedPattern) -> Result<RegressionInput> {
    match &pattern.check {
        Check::CommandRegex { .. } => Ok(RegressionInput::Command(representative_command(
            pack, pattern,
        )?)),
        Check::ContentRegex { regex } => {
            Regex::new(regex).with_context(|| {
                format!(
                    "invalid regex for guarded pattern '{}': {}",
                    pattern.id, regex
                )
            })?;
            let file_path = pack
                .applies_to
                .first()
                .map(|glob| {
                    if glob.ends_with(".yml") {
                        "regression.yml"
                    } else {
                        "regression.yaml"
                    }
                })
                .unwrap_or("regression.yaml")
                .to_string();
            Ok(RegressionInput::Content {
                file_path,
                content: content_regex_example(regex),
            })
        }
        Check::Predicate { .. } => bail!(
            "guarded pattern '{}' in pack '{}' is a predicate; provide a regression example",
            pattern.id,
            pack.id
        ),
    }
}

fn content_regex_example(regex: &str) -> String {
    if regex.contains("storageClassName") {
        return "storageClassName: ssd".to_string();
    }
    if regex.contains(":latest") {
        return "image: example:latest".to_string();
    }
    if regex.contains("[0-9a-f]") {
        return "image: ronaldraygun/example:deadbeef".to_string();
    }
    regex_example(regex)
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
            if !pattern.enabled {
                continue;
            }
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

fn validate_deny_input(
    pack: &Pack,
    target: &GuardedPattern,
    input: &RegressionInput,
) -> Result<()> {
    match (&target.check, input) {
        (Check::CommandRegex { .. }, RegressionInput::Command(command)) => {
            validate_deny_case(pack, target, command)
        }
        (Check::ContentRegex { regex }, RegressionInput::Content { content, .. }) => {
            if target.redirect.channel != Channel::Deny {
                bail!(
                    "guarded pattern '{}' in pack '{}' uses {:?}; fixed regression cases require a deny redirect",
                    target.id,
                    pack.id,
                    target.redirect.channel
                );
            }
            let target_regex = Regex::new(regex).with_context(|| {
                format!("invalid regex for guarded pattern '{}': {regex}", target.id)
            })?;
            if !target_regex.is_match(content) {
                bail!(
                    "example content for guarded pattern '{}' in pack '{}' does not match its regex: {}",
                    target.id,
                    pack.id,
                    content
                );
            }
            for pattern in &pack.safe_patterns {
                if let Check::ContentRegex { regex } = &pattern.check {
                    let safe_regex = Regex::new(regex).with_context(|| {
                        format!("invalid regex for safe pattern '{}': {regex}", pattern.id)
                    })?;
                    if safe_regex.is_match(content) {
                        bail!(
                            "example content for guarded pattern '{}' in pack '{}' matches safe pattern '{}' first: {}",
                            target.id,
                            pack.id,
                            pattern.id,
                            content
                        );
                    }
                }
            }
            Ok(())
        }
        (Check::CommandRegex { .. }, RegressionInput::Content { .. })
        | (Check::ContentRegex { .. }, RegressionInput::Command(_)) => bail!(
            "regression example for guarded pattern '{}' uses the wrong input mode",
            target.id
        ),
        (Check::Predicate { .. }, _) => bail!(
            "guarded pattern '{}' in pack '{}' is a predicate; provide a supported regression example",
            target.id,
            pack.id
        ),
    }
}

/// Normalize the command in the same limited way as command-mode dispatch:
/// split shell segments and remove transparent wrapper prefixes and leading
/// environment assignments. The generated commands do not need this, but it
/// lets a hand-authored `example_command` exercise the real hook shape too.
fn command_segments(command: &str) -> Vec<String> {
    command
        .split([';', '&', '|', '\n'])
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
            enabled: true,
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
        assert!(error
            .to_string()
            .contains("fixed regression cases require a deny redirect"));
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
