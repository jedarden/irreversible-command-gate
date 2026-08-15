//! Per-repository, release-bound rule overrides.
//!
//! An override is deliberately not a local escape hatch.  It is an artifact
//! of the same release as the rule pack: the release reference in the file
//! must match the host's trusted reference, every exempted rule must exist in
//! that release, and the manifest must still be fresh.  The release pipeline
//! treats newly exempted rule IDs as coverage regressions.

use crate::engine::{CheckResult, CommandSource, ContentSource, Engine};
use crate::regression::{verify_regression_suite, RegressionSuite};
use crate::rule_pack::{Channel, Pack};
use anyhow::{bail, Context, Result};
use chrono::{Duration, NaiveDate, Utc};
use std::collections::{BTreeSet, HashMap};
use std::fs;
use std::path::Path;

/// Stable schema identifier for `overrides/<repo>.toml`.
pub const OVERRIDE_SCHEMA: &str = "icg-override/v1";

/// An override must be re-justified at least this often, even if its expiry
/// date is farther away.  This keeps a long-lived exception from becoming
/// invisible Swiss-cheese coverage.
pub const REJUSTIFICATION_CADENCE_DAYS: i64 = 90;

/// The checked-in representation of `overrides/<repo>.toml`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepoOverride {
    pub schema: String,
    pub repository: String,
    pub release_ref: String,
    pub exempted_rule_ids: Vec<String>,
    pub expires_at: String,
    pub last_justified_at: String,
    pub justification: String,
}

/// Compatibility name for callers that prefer the file-oriented term.
pub type OverrideManifest = RepoOverride;

/// Result of comparing override manifests across two releases.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct OverrideCoverageDiff {
    /// Adding an exemption removes coverage and therefore requires Layer 2
    /// review, just like a removed guarded pattern.
    pub newly_exempted_rule_ids: Vec<String>,
    /// Removing an exemption strengthens coverage and is informational.
    pub removed_exempted_rule_ids: Vec<String>,
}

impl OverrideCoverageDiff {
    pub fn has_regressions(&self) -> bool {
        !self.newly_exempted_rule_ids.is_empty()
    }
}

/// Freshness state used by status/reporting code.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OverrideFreshness {
    Fresh,
    Stale,
    Expired,
}

impl RepoOverride {
    /// Create a manifest with the current date as its justification date.
    pub fn new(
        repository: impl Into<String>,
        release_ref: impl Into<String>,
        exempted_rule_ids: Vec<String>,
        expires_at: impl Into<String>,
        justification: impl Into<String>,
    ) -> Self {
        Self {
            schema: OVERRIDE_SCHEMA.to_string(),
            repository: repository.into(),
            release_ref: release_ref.into(),
            exempted_rule_ids,
            expires_at: expires_at.into(),
            last_justified_at: Utc::now().date_naive().to_string(),
            justification: justification.into(),
        }
    }

    pub fn exempted_rule_set(&self) -> BTreeSet<&str> {
        self.exempted_rule_ids.iter().map(String::as_str).collect()
    }

    pub fn freshness_at(&self, today: NaiveDate) -> Result<OverrideFreshness> {
        let expires_at = parse_date("expires_at", &self.expires_at)?;
        let justified_at = parse_date("last_justified_at", &self.last_justified_at)?;

        if expires_at <= today {
            return Ok(OverrideFreshness::Expired);
        }
        if today > justified_at + Duration::days(REJUSTIFICATION_CADENCE_DAYS) {
            return Ok(OverrideFreshness::Stale);
        }
        Ok(OverrideFreshness::Fresh)
    }
}

/// Load and parse a TOML override.  This parser intentionally supports the
/// small, flat schema documented for overrides and rejects tables/unknown
/// keys rather than silently discarding policy fields.
pub fn load_override<P: AsRef<Path>>(path: P) -> Result<RepoOverride> {
    let path = path.as_ref();
    let content = fs::read_to_string(path)
        .with_context(|| format!("failed to read override from {}", path.display()))?;
    parse_override_toml(&content)
        .with_context(|| format!("failed to parse override TOML from {}", path.display()))
}

/// Alias used by release tooling.
pub fn load_override_manifest<P: AsRef<Path>>(path: P) -> Result<RepoOverride> {
    load_override(path)
}

/// Write a stable, review-friendly TOML override.
pub fn save_override<P: AsRef<Path>>(manifest: &RepoOverride, path: P) -> Result<()> {
    let path = path.as_ref();
    if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create override directory {}", parent.display()))?;
    }
    fs::write(path, render_override_toml(manifest))
        .with_context(|| format!("failed to write override to {}", path.display()))
}

/// Validate an override's metadata, release binding, scope, and rule IDs.
/// This is the check used before the engine is allowed to honor exemptions.
pub fn validate_override(
    manifest: &RepoOverride,
    repository: &str,
    trusted_ref: &str,
    packs: &[Pack],
) -> Result<()> {
    validate_override_at(
        manifest,
        repository,
        trusted_ref,
        packs,
        Utc::now().date_naive(),
    )
}

/// Deterministic form of [`validate_override`] for CI and tests.
pub fn validate_override_at(
    manifest: &RepoOverride,
    repository: &str,
    trusted_ref: &str,
    packs: &[Pack],
    today: NaiveDate,
) -> Result<()> {
    validate_metadata(manifest, repository, trusted_ref, today)?;

    let mut rules: HashMap<&str, (&str, Channel)> = HashMap::new();
    for pack in packs {
        for pattern in &pack.guarded_patterns {
            if rules
                .insert(
                    pattern.id.as_str(),
                    (pack.id.as_str(), pattern.redirect.channel),
                )
                .is_some()
            {
                bail!(
                    "duplicate guarded rule ID '{}' across rule packs",
                    pattern.id
                );
            }
        }
    }

    let mut seen = BTreeSet::new();
    for rule_id in &manifest.exempted_rule_ids {
        if !seen.insert(rule_id.as_str()) {
            bail!("override lists rule ID '{}' more than once", rule_id);
        }
        let Some((pack_id, channel)) = rules.get(rule_id.as_str()) else {
            bail!("override references unknown guarded rule ID '{}'", rule_id);
        };
        if *channel != Channel::Deny {
            bail!(
                "override rule '{}' in pack '{}' is not a deny rule",
                rule_id,
                pack_id
            );
        }
    }
    Ok(())
}

/// Load an override and prove that it belongs to the trusted release before
/// returning it.  A caller must provide the loaded release packs; there is no
/// path that validates only the TOML and then silently enables it.
pub fn load_verified_override<P: AsRef<Path>>(
    path: P,
    repository: &str,
    trusted_ref: &str,
    packs: &[Pack],
) -> Result<RepoOverride> {
    let path = path.as_ref();
    if path.extension().and_then(|extension| extension.to_str()) != Some("toml") {
        bail!("override must be a .toml file: {}", path.display());
    }
    if path
        .parent()
        .and_then(Path::file_name)
        .and_then(|name| name.to_str())
        != Some("overrides")
    {
        bail!(
            "verified overrides must come from an overrides/ directory: {}",
            path.display()
        );
    }
    let expected_filename = repository.rsplit('/').next().unwrap_or(repository);
    if path.file_stem().and_then(|stem| stem.to_str()) != Some(expected_filename) {
        bail!(
            "override filename must be overrides/{}.toml for repository '{}': {}",
            expected_filename,
            repository,
            path.display()
        );
    }
    let manifest = load_override(path)?;
    validate_override(&manifest, repository, trusted_ref, packs)?;
    Ok(manifest)
}

/// Compare the exempted IDs in the previous and current release.  `None`
/// means no override file exists in that release.
pub fn diff_overrides(
    previous: Option<&RepoOverride>,
    current: Option<&RepoOverride>,
) -> OverrideCoverageDiff {
    let previous_ids: BTreeSet<&str> = previous
        .map(RepoOverride::exempted_rule_set)
        .unwrap_or_default();
    let current_ids: BTreeSet<&str> = current
        .map(RepoOverride::exempted_rule_set)
        .unwrap_or_default();

    let mut newly_exempted_rule_ids: Vec<String> = current_ids
        .difference(&previous_ids)
        .map(|id| (*id).to_string())
        .collect();
    let mut removed_exempted_rule_ids: Vec<String> = previous_ids
        .difference(&current_ids)
        .map(|id| (*id).to_string())
        .collect();
    newly_exempted_rule_ids.sort();
    removed_exempted_rule_ids.sort();

    OverrideCoverageDiff {
        newly_exempted_rule_ids,
        removed_exempted_rule_ids,
    }
}

/// Verify the ordinary fixed deny suite and ensure an override is not being
/// used to skip any rule other than the IDs explicitly listed in the file.
/// The suite is generated from the release pack, so every guarded rule still
/// has a test even when the release intentionally exempts one at runtime.
pub fn verify_override_regression_gate(
    pack: &Pack,
    suite: &RegressionSuite,
    manifest: &RepoOverride,
    repository: &str,
    trusted_ref: &str,
) -> Result<()> {
    validate_override(
        manifest,
        repository,
        trusted_ref,
        std::slice::from_ref(pack),
    )?;
    verify_regression_suite(pack, suite)?;

    let mut engine = Engine::new();
    engine.load_pack(pack.clone())?;
    engine.load_verified_override(manifest, repository, trusted_ref)?;

    let exempted = manifest.exempted_rule_set();
    for case in &suite.cases {
        let result = match (&case.file_path, &case.content) {
            (Some(file_path), Some(content)) => engine.evaluate_content(&ContentSource::Write {
                file_path: file_path.clone(),
                content: content.clone(),
            }),
            (None, None) => engine.evaluate_command(&CommandSource::Hook(case.command.clone())),
            _ => bail!(
                "regression case '{}' has an incomplete content input",
                case.pattern_id
            ),
        };
        if exempted.contains(case.pattern_id.as_str()) {
            if matches!(
                result,
                CheckResult::Denied { ref pattern_id, .. } if pattern_id == &case.pattern_id
            ) {
                bail!(
                    "override rule '{}' still denies its fixed regression case",
                    case.pattern_id
                );
            }
        } else if !matches!(
            result,
            CheckResult::Denied { ref pattern_id, .. } if pattern_id == &case.pattern_id
        ) {
            bail!(
                "non-exempted rule '{}' no longer denies its fixed regression case",
                case.pattern_id
            );
        }
    }
    Ok(())
}

fn validate_metadata(
    manifest: &RepoOverride,
    repository: &str,
    trusted_ref: &str,
    today: NaiveDate,
) -> Result<()> {
    if manifest.schema != OVERRIDE_SCHEMA {
        bail!(
            "unsupported override schema '{}'; expected {}",
            manifest.schema,
            OVERRIDE_SCHEMA
        );
    }
    if !valid_repository(repository) || manifest.repository != repository {
        bail!(
            "override repository '{}' does not match requested repository '{}'",
            manifest.repository,
            repository
        );
    }
    if trusted_ref.trim().is_empty() || trusted_ref == "latest" {
        bail!("trusted release reference must be an exact, non-latest reference");
    }
    if manifest.release_ref != trusted_ref {
        bail!(
            "override release_ref '{}' is not the trusted release '{}'",
            manifest.release_ref,
            trusted_ref
        );
    }
    if manifest.exempted_rule_ids.is_empty() {
        bail!("override must exempt at least one rule ID");
    }
    if manifest.justification.trim().is_empty() {
        bail!("override justification must not be blank");
    }

    let expires_at = parse_date("expires_at", &manifest.expires_at)?;
    let justified_at = parse_date("last_justified_at", &manifest.last_justified_at)?;
    if expires_at <= today {
        bail!(
            "override expired on {}; renew it through a reviewed release",
            expires_at
        );
    }
    if justified_at > today {
        bail!("last_justified_at {} is in the future", justified_at);
    }
    match manifest.freshness_at(today)? {
        OverrideFreshness::Fresh => {}
        OverrideFreshness::Stale => bail!(
            "override re-justification is stale (last justified {}; cadence is {} days)",
            justified_at,
            REJUSTIFICATION_CADENCE_DAYS
        ),
        OverrideFreshness::Expired => unreachable!("expiry checked above"),
    }
    for id in &manifest.exempted_rule_ids {
        if id.trim().is_empty() || id.chars().any(char::is_whitespace) {
            bail!("override contains an invalid rule ID '{}': IDs cannot be blank or contain whitespace", id);
        }
    }
    Ok(())
}

fn valid_repository(repository: &str) -> bool {
    !repository.is_empty()
        && !repository.starts_with('/')
        && !repository.ends_with('/')
        && !repository.contains("..")
        && !repository.chars().any(char::is_whitespace)
}

fn parse_date(field: &str, value: &str) -> Result<NaiveDate> {
    NaiveDate::parse_from_str(value.trim(), "%Y-%m-%d")
        .with_context(|| format!("{field} must be an ISO date (YYYY-MM-DD), got '{value}'"))
}

fn parse_override_toml(content: &str) -> Result<RepoOverride> {
    let mut scalars: HashMap<String, String> = HashMap::new();
    let mut arrays: HashMap<String, Vec<String>> = HashMap::new();

    for (line_number, raw_line) in content.lines().enumerate() {
        let line = strip_comment(raw_line).trim().to_string();
        if line.is_empty() {
            continue;
        }
        if line.starts_with('[') {
            bail!(
                "line {}: tables are not allowed in an override",
                line_number + 1
            );
        }
        let Some(equals) = find_unquoted(line.as_str(), '=') else {
            bail!("line {}: expected key = value", line_number + 1);
        };
        let key = line[..equals].trim();
        if key.is_empty()
            || !key
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
        {
            bail!("line {}: invalid override key '{}", line_number + 1, key);
        }
        let value = line[equals + 1..].trim();
        if value.starts_with('[') {
            if arrays.contains_key(key) || scalars.contains_key(key) {
                bail!(
                    "line {}: duplicate override field '{}'",
                    line_number + 1,
                    key
                );
            }
            arrays.insert(
                key.to_string(),
                parse_array(value).with_context(|| {
                    format!(
                        "line {}: invalid array for '{}': {value}",
                        line_number + 1,
                        key
                    )
                })?,
            );
        } else {
            if scalars.contains_key(key) || arrays.contains_key(key) {
                bail!(
                    "line {}: duplicate override field '{}'",
                    line_number + 1,
                    key
                );
            }
            scalars.insert(
                key.to_string(),
                parse_toml_string(value).with_context(|| {
                    format!(
                        "line {}: invalid value for '{}': {value}",
                        line_number + 1,
                        key
                    )
                })?,
            );
        }
    }

    let schema = take_scalar(&mut scalars, &["schema", "version"])?
        .unwrap_or_else(|| OVERRIDE_SCHEMA.to_string());
    let repository = take_scalar(&mut scalars, &["repository", "repo"])?
        .context("override is missing required 'repository'")?;
    let release_ref = take_scalar(
        &mut scalars,
        &["release_ref", "release", "release_tag", "trusted_ref"],
    )?
    .context("override is missing required 'release_ref'")?;
    let expires_at = take_scalar(
        &mut scalars,
        &["expires_at", "expires", "expiry", "expiry_date"],
    )?
    .context("override is missing required 'expires_at'")?;
    let last_justified_at = take_scalar(
        &mut scalars,
        &["last_justified_at", "justified_at", "reviewed_at"],
    )?
    .context("override is missing required 'last_justified_at'")?;
    let justification = take_scalar(&mut scalars, &["justification", "reason"])?
        .context("override is missing required 'justification'")?;
    let exempted_rule_ids = take_array(
        &mut arrays,
        &["exempted_rule_ids", "exempted_rules", "rule_ids", "rules"],
    )?
    .context("override is missing required 'exempted_rule_ids'")?;

    if let Some(key) = scalars.keys().next().cloned() {
        bail!("unknown override field '{key}'");
    }
    if let Some(key) = arrays.keys().next().cloned() {
        bail!("unknown override field '{key}'");
    }

    Ok(RepoOverride {
        schema,
        repository,
        release_ref,
        exempted_rule_ids,
        expires_at,
        last_justified_at,
        justification,
    })
}

fn take_scalar(values: &mut HashMap<String, String>, keys: &[&str]) -> Result<Option<String>> {
    let present: Vec<String> = keys
        .iter()
        .filter_map(|key| values.get(*key).map(|_| (*key).to_string()))
        .collect();
    if present.len() > 1 {
        bail!(
            "override specifies duplicate aliases: {}",
            present.join(", ")
        );
    }
    Ok(present.first().and_then(|key| values.remove(key)))
}

fn take_array(
    values: &mut HashMap<String, Vec<String>>,
    keys: &[&str],
) -> Result<Option<Vec<String>>> {
    let present: Vec<String> = keys
        .iter()
        .filter_map(|key| values.get(*key).map(|_| (*key).to_string()))
        .collect();
    if present.len() > 1 {
        bail!(
            "override specifies duplicate aliases: {}",
            present.join(", ")
        );
    }
    Ok(present.first().and_then(|key| values.remove(key)))
}

fn parse_array(value: &str) -> Result<Vec<String>> {
    let value = value.trim();
    if !value.starts_with('[') || !value.ends_with(']') {
        bail!("array must be enclosed in '[' and ']'");
    }
    let inner = value[1..value.len() - 1].trim();
    if inner.is_empty() {
        return Ok(Vec::new());
    }
    let mut result = Vec::new();
    let mut start = 0;
    let mut quote = false;
    let mut escaped = false;
    for (index, character) in inner.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        if character == '\\' && quote {
            escaped = true;
        } else if character == '"' {
            quote = !quote;
        } else if character == ',' && !quote {
            let item = inner[start..index].trim();
            result.push(parse_toml_string(item)?);
            start = index + 1;
        }
    }
    if quote {
        bail!("unterminated string in array");
    }
    let item = inner[start..].trim();
    if item.is_empty() {
        // TOML permits a trailing comma in an array.
        return Ok(result);
    }
    result.push(parse_toml_string(item)?);
    Ok(result)
}

fn parse_toml_string(value: &str) -> Result<String> {
    let value = value.trim();
    if value.len() >= 2 && value.starts_with('"') && value.ends_with('"') {
        let inner = &value[1..value.len() - 1];
        let mut result = String::new();
        let mut escaped = false;
        for character in inner.chars() {
            if escaped {
                result.push(match character {
                    'n' => '\n',
                    'r' => '\r',
                    't' => '\t',
                    '"' => '"',
                    '\\' => '\\',
                    other => bail!("unsupported TOML escape \\{other}"),
                });
                escaped = false;
            } else if character == '\\' {
                escaped = true;
            } else {
                result.push(character);
            }
        }
        if escaped {
            bail!("unterminated escape sequence");
        }
        return Ok(result);
    }
    if value.is_empty() || value.contains(' ') || value.contains('\t') {
        bail!("value must be a quoted string");
    }
    Ok(value.to_string())
}

fn strip_comment(line: &str) -> &str {
    let mut quote = false;
    let mut escaped = false;
    for (index, character) in line.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        if character == '\\' && quote {
            escaped = true;
        } else if character == '"' {
            quote = !quote;
        } else if character == '#' && !quote {
            return &line[..index];
        }
    }
    line
}

fn find_unquoted(value: &str, needle: char) -> Option<usize> {
    let mut quote = false;
    let mut escaped = false;
    for (index, character) in value.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        if character == '\\' && quote {
            escaped = true;
        } else if character == '"' {
            quote = !quote;
        } else if character == needle && !quote {
            return Some(index);
        }
    }
    None
}

fn render_override_toml(manifest: &RepoOverride) -> String {
    let ids = manifest
        .exempted_rule_ids
        .iter()
        .map(|id| format!("\"{}\"", escape_toml(id)))
        .collect::<Vec<_>>()
        .join(", ");
    format!(
        "schema = \"{}\"\nrepository = \"{}\"\nrelease_ref = \"{}\"\nexempted_rule_ids = [{}]\nexpires_at = \"{}\"\nlast_justified_at = \"{}\"\njustification = \"{}\"\n",
        escape_toml(&manifest.schema),
        escape_toml(&manifest.repository),
        escape_toml(&manifest.release_ref),
        ids,
        escape_toml(&manifest.expires_at),
        escape_toml(&manifest.last_justified_at),
        escape_toml(&manifest.justification),
    )
}

fn escape_toml(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rule_pack::{Check, GuardedPattern, Redirect, Severity, Tier};
    use tempfile::tempdir;

    fn pack() -> Pack {
        Pack {
            id: "git".to_string(),
            tool_keywords: vec!["git".to_string()],
            applies_to: Vec::new(),
            safe_patterns: Vec::new(),
            guarded_patterns: vec![GuardedPattern {
                id: "git-force-push".to_string(),
                check: Check::CommandRegex {
                    regex: "git push.*--force".to_string(),
                },
                tier: Tier::Tier1,
                severity: Severity::Critical,
                explanation: "rewrites remote history".to_string(),
                redirect: Redirect {
                    channel: Channel::Deny,
                    reason_template: "force push denied".to_string(),
                    rewrite_template: None,
                },
                destructive: true,
            }],
        }
    }

    fn manifest() -> RepoOverride {
        RepoOverride {
            schema: OVERRIDE_SCHEMA.to_string(),
            repository: "jedarden/example".to_string(),
            release_ref: "v1.2.3".to_string(),
            exempted_rule_ids: vec!["git-force-push".to_string()],
            expires_at: "2026-12-31".to_string(),
            last_justified_at: "2026-08-15".to_string(),
            justification: "A migration tool owns this operation.".to_string(),
        }
    }

    #[test]
    fn parses_and_round_trips_toml() {
        let directory = tempdir().unwrap();
        let path = directory.path().join("example.toml");
        save_override(&manifest(), &path).unwrap();
        assert_eq!(load_override(&path).unwrap(), manifest());
    }

    #[test]
    fn verifies_scope_release_expiry_and_rule_ids() {
        let item = manifest();
        validate_override_at(
            &item,
            "jedarden/example",
            "v1.2.3",
            &[pack()],
            NaiveDate::from_ymd_opt(2026, 8, 15).unwrap(),
        )
        .unwrap();

        let wrong_release = validate_override_at(
            &item,
            "jedarden/example",
            "v1.2.4",
            &[pack()],
            NaiveDate::from_ymd_opt(2026, 8, 15).unwrap(),
        );
        assert!(wrong_release
            .unwrap_err()
            .to_string()
            .contains("trusted release"));
    }

    #[test]
    fn stale_rejustification_is_rejected_before_expiry() {
        let mut item = manifest();
        item.last_justified_at = "2026-01-01".to_string();
        let error = validate_override_at(
            &item,
            "jedarden/example",
            "v1.2.3",
            &[pack()],
            NaiveDate::from_ymd_opt(2026, 8, 15).unwrap(),
        )
        .unwrap_err();
        assert!(error.to_string().contains("re-justification is stale"));
    }

    #[test]
    fn newly_added_exemptions_are_coverage_regressions() {
        let previous = manifest();
        let mut current = manifest();
        current.exempted_rule_ids.push("other-rule".to_string());
        let diff = diff_overrides(Some(&previous), Some(&current));
        assert_eq!(diff.newly_exempted_rule_ids, vec!["other-rule"]);
        assert!(diff.has_regressions());
    }

    #[test]
    fn engine_honors_only_a_verified_override_from_overrides_directory() {
        let directory = tempdir().unwrap();
        let overrides_directory = directory.path().join("overrides");
        let path = overrides_directory.join("example.toml");
        save_override(&manifest(), &path).unwrap();

        let mut engine = Engine::new();
        engine.load_pack(pack()).unwrap();
        engine
            .load_verified_override_from_file(&path, "jedarden/example", "v1.2.3")
            .unwrap();
        assert_eq!(
            engine.evaluate_command(&CommandSource::Hook(
                "git push origin main --force".to_string()
            )),
            CheckResult::Allowed
        );
    }
}
