//! Engine: command-mode and content-mode input acquisition and pack dispatch
//!
//! This module handles:
//! - Input acquisition from PreToolUse JSON (hook mode) or argv (wrapper mode)
//! - Command-mode: shell line segmentation (splits on ;/&&/||/, skips sudo/env-assignment/wrapper prefixes)
//!   Basename-matching tokens against tool_keywords for pack dispatch
//! - Pack dispatch: routes tokens to matching packs and evaluates guarded_patterns
//! - Content-mode: file path + content reading from Write/Edit PreToolUse JSON
//!   Used for content regex checks (storage-class, image-tag packs)

use anyhow::{Context, Result};
use crate::rule_pack::Pack;
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::Path;

/// PreToolUse hook JSON structure (partial, for input parsing)
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PreToolUseInput {
    tool_name: Option<String>,
    tool_input: Option<ToolInput>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ToolInput {
    command: Option<String>,
    file_path: Option<String>,
    content: Option<String>,
    new_string: Option<String>,
}

/// Command input source
#[derive(Debug, Clone, PartialEq)]
pub enum CommandSource {
    /// From PreToolUse JSON stdin (hook mode)
    Hook(String),
    /// From wrapped-command argv (PATH-wrapper mode)
    Argv(Vec<String>),
}

/// Content input source (for Write/Edit operations)
#[derive(Debug, Clone, PartialEq)]
pub enum ContentSource {
    /// Write operation: full file content
    Write {
        file_path: String,
        content: String,
    },
    /// Edit operation: old and new content
    Edit {
        file_path: String,
        old_content: String,
        new_content: String,
    },
}

impl ContentSource {
    /// Get the file path being written/edited
    pub fn file_path(&self) -> &str {
        match self {
            ContentSource::Write { file_path, .. } => file_path,
            ContentSource::Edit { file_path, .. } => file_path,
        }
    }

    /// Get all content that should be checked (for Write, this is the full content;
    /// for Edit, this is both old and new content to catch introductions in new_content)
    pub fn content_to_check(&self) -> Vec<&str> {
        match self {
            ContentSource::Write { content, .. } => vec![content],
            ContentSource::Edit { old_content, new_content, .. } => {
                vec![old_content, new_content]
            }
        }
    }

    /// Get just the new content (the actual change being made)
    pub fn new_content(&self) -> &str {
        match self {
            ContentSource::Write { content, .. } => content,
            ContentSource::Edit { new_content, .. } => new_content,
        }
    }

    /// Check if this content applies to a specific file glob
    pub fn matches_glob(&self, glob: &str) -> bool {
        path_matches_glob(self.file_path(), glob)
    }
}

/// Match a write target against a file-path glob from a pack's `applies_to` list.
///
/// Globs without a path separator (for example, `*.yaml`) are matched against
/// the target's basename. Globs with a separator are matched against the full
/// path and each path suffix, so a relative selector such as `.beads/**` also
/// works when the hook reports an absolute path. `*` matches within one path
/// component; `**` may also cross path separators.
fn path_matches_glob(path: &str, glob: &str) -> bool {
    let path = normalize_path(path);
    let mut glob = normalize_path(glob);

    if glob.is_empty() {
        return false;
    }

    // A trailing slash denotes a directory and everything below it. This is
    // useful for the beads selector whether it is written as `.beads/` or the
    // more explicit `.beads/**`.
    if glob.ends_with('/') {
        glob.push_str("**");
    }

    let has_separator = glob.contains('/');
    if !has_separator {
        let basename = path.rsplit('/').next().unwrap_or(path.as_str());
        return wildcard_match(basename, &glob);
    }

    path_candidates(&path)
        .iter()
        .any(|candidate| wildcard_match(candidate, &glob))
}

fn normalize_path(value: &str) -> String {
    value.replace('\\', "/").trim_start_matches("./").to_string()
}

/// Return the full path and relative suffixes for matching relative selectors
/// against absolute hook paths.
fn path_candidates(path: &str) -> Vec<String> {
    let mut candidates = Vec::new();
    let without_leading_slash = path.trim_start_matches('/');

    candidates.push(path.to_string());
    if without_leading_slash != path {
        candidates.push(without_leading_slash.to_string());
    }

    for (index, character) in without_leading_slash.char_indices() {
        if character == '/' && index + 1 < without_leading_slash.len() {
            candidates.push(without_leading_slash[index + 1..].to_string());
        }
    }

    candidates
}

/// Match a glob against one normalized path. This deliberately stays local to
/// the engine so pack dispatch does not need filesystem I/O or another runtime
/// dependency.
fn wildcard_match(value: &str, pattern: &str) -> bool {
    let value: Vec<char> = value.chars().collect();
    let pattern: Vec<char> = pattern.chars().collect();
    let mut memo = vec![vec![None; value.len() + 1]; pattern.len() + 1];

    fn visit(
        value: &[char],
        pattern: &[char],
        value_index: usize,
        pattern_index: usize,
        memo: &mut [Vec<Option<bool>>],
    ) -> bool {
        if let Some(result) = memo[pattern_index][value_index] {
            return result;
        }

        let result = if pattern_index == pattern.len() {
            value_index == value.len()
        } else if pattern[pattern_index] == '*' {
            let is_double_star = pattern_index + 1 < pattern.len()
                && pattern[pattern_index + 1] == '*';

            if is_double_star {
                // `**/` may consume zero directories, or any number of path
                // characters before the next slash.
                let after_stars = pattern_index + 2;
                let skip_double_star = if after_stars < pattern.len()
                    && pattern[after_stars] == '/'
                {
                    visit(value, pattern, value_index, after_stars + 1, memo)
                } else {
                    visit(value, pattern, value_index, after_stars, memo)
                };
                skip_double_star
                    || (value_index < value.len()
                        && visit(value, pattern, value_index + 1, pattern_index, memo))
            } else {
                // A single star is confined to one path component.
                visit(value, pattern, value_index, pattern_index + 1, memo)
                    || (value_index < value.len()
                        && value[value_index] != '/'
                        && visit(value, pattern, value_index + 1, pattern_index, memo))
            }
        } else if value_index < value.len()
            && (pattern[pattern_index] == '?'
                || pattern[pattern_index] == value[value_index])
        {
            visit(value, pattern, value_index + 1, pattern_index + 1, memo)
        } else {
            false
        };

        memo[pattern_index][value_index] = Some(result);
        result
    }

    visit(&value, &pattern, 0, 0, &mut memo)
}

/// Input source from PreToolUse hook
#[derive(Debug, Clone, PartialEq)]
pub enum InputSource {
    /// Bash command (for command-mode packs: vault, git, secrets, misc, tmux)
    Command(CommandSource),
    /// File content (for content-mode packs: storage-class, image-tag)
    Content(ContentSource),
}

/// Normalized command token ready for pack matching
#[derive(Debug, Clone, PartialEq)]
pub struct CommandToken {
    /// The basename-matched executable (e.g., "vault", "git", not "/usr/bin/vault")
    pub executable: String,
    /// Remaining arguments after segmentation and prefix-stripping
    pub args: Vec<String>,
}

/// Check result from evaluating a command against a rule pack
#[derive(Debug, Clone, PartialEq)]
pub enum CheckResult {
    /// Command is allowed (no patterns matched or only safe_patterns matched)
    Allowed,
    /// Command is denied with a reason
    Denied {
        reason: String,
        pack_id: String,
        pattern_id: String,
    },
    /// Command should be rewritten (updatedInput channel)
    Rewrite {
        reason: String,
        rewrite: String,
        pack_id: String,
        pattern_id: String,
    },
    /// Command allowed but with warning (additionalContext channel)
    Warning {
        reason: String,
        pack_id: String,
        pattern_id: String,
    },
}

/// Engine: command-mode input acquisition, segmentation, and pack dispatch
pub struct Engine {
    /// Splits on ||, &&, ;, |, &, newline
    segment_splitter: Option<Regex>,
    /// Detects env assignments like VAR=value or FOO_BAR=value
    env_assign_pattern: Option<Regex>,
    /// Prefixes to skip: sudo, command, exec, time, nohup
    ignored_prefixes: Vec<String>,
    /// Loaded rule packs (pack_id -> Pack)
    packs: HashMap<String, crate::rule_pack::Pack>,
    /// tool_keywords -> pack_id mapping for fast dispatch
    keyword_index: HashMap<String, Vec<String>>,
    /// Rule IDs exempted by the currently verified per-repository override.
    /// This is populated only by `load_verified_override`; there is no public
    /// raw-ID bypass.
    exempted_rule_ids: HashSet<String>,
    /// Once an in-process failure occurs, every subsequent check must allow.
    /// This remains set for the lifetime of one engine invocation.
    fail_open: bool,
}

impl Default for Engine {
    fn default() -> Self {
        Self::new()
    }
}

impl Engine {
    /// Create a new Engine with default segmentation patterns
    pub fn new() -> Self {
        // Match shell command separators: ||, &&, ;, |, &, \n
        // Pattern: (?:\|\||&&|[;&|\n])
        let segment_splitter = Regex::new(r"(?:\|\||&&|[;&|\n])").ok();

        // Match env variable assignments: starts with letter or underscore, followed by alphanumerics/underscores, then =
        // Pattern: ^[A-Za-z_][A-Za-z0-9_]*=
        let env_assign_pattern = Regex::new(r"^[A-Za-z_][A-Za-z0-9_]*=").ok();

        let ignored_prefixes = vec![
            "sudo".to_string(),
            "command".to_string(),
            "exec".to_string(),
            "time".to_string(),
            "nohup".to_string(),
        ];

        Self {
            segment_splitter,
            env_assign_pattern,
            ignored_prefixes,
            packs: HashMap::new(),
            keyword_index: HashMap::new(),
            exempted_rule_ids: HashSet::new(),
            fail_open: false,
        }
    }

    fn mark_fail_open(&mut self, reason: &str) {
        self.fail_open = true;
        report_fail_open(reason);
    }

    fn should_fail_open(&self) -> bool {
        self.fail_open
    }

    /// Read input from PreToolUse JSON on stdin (hook mode)
    ///
    /// Returns the appropriate InputSource:
    /// - CommandSource for Bash tool calls
    /// - ContentSource for Write/Edit tool calls
    /// - None for unrecognized tools
    pub fn read_from_stdin(&self) -> Result<Option<InputSource>> {
        let result = catch_unwind(AssertUnwindSafe(|| self.read_from_stdin_inner()));
        match result {
            Ok(Ok(input)) => Ok(input),
            Ok(Err(error)) => {
                report_fail_open(&format!("stdin input failure: {error}"));
                Ok(None)
            }
            Err(_) => {
                report_fail_open("stdin input panicked");
                Ok(None)
            }
        }
    }

    fn read_from_stdin_inner(&self) -> Result<Option<InputSource>> {
        use std::io::{self, Read};

        let mut input = String::new();
        io::stdin()
            .read_to_string(&mut input)
            .context("Failed to read stdin")?;

        let parsed: serde_json::Value = serde_json::from_str(&input)
            .context("Failed to parse PreToolUse JSON")?;

        // Extract tool name and input
        let tool_name = parsed.get("toolName")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        let tool_input = parsed.get("toolInput");

        // Process Bash commands (command-mode)
        if tool_name.as_deref() == Some("Bash") {
            let command = tool_input
                .and_then(|ti| ti.get("command"))
                .and_then(|v| v.as_str())
                .unwrap_or("");

            return Ok(Some(InputSource::Command(CommandSource::Hook(command.to_string()))));
        }

        // Process Write operations (content-mode)
        if tool_name.as_deref() == Some("Write") {
            let file_path = tool_input
                .and_then(|ti| ti.get("filePath"))
                .and_then(|v| v.as_str())
                .unwrap_or("");

            let content = tool_input
                .and_then(|ti| ti.get("content"))
                .and_then(|v| v.as_str())
                .unwrap_or("");

            return Ok(Some(InputSource::Content(ContentSource::Write {
                file_path: file_path.to_string(),
                content: content.to_string(),
            })));
        }

        // Process Edit operations (content-mode)
        if tool_name.as_deref() == Some("Edit") {
            let file_path = tool_input
                .and_then(|ti| ti.get("filePath"))
                .and_then(|v| v.as_str())
                .unwrap_or("");

            // For Edit, we need both old_string and new_string
            let old_string = tool_input
                .and_then(|ti| ti.get("oldString"))
                .and_then(|v| v.as_str())
                .unwrap_or("");

            let new_string = tool_input
                .and_then(|ti| ti.get("newString"))
                .and_then(|v| v.as_str())
                .unwrap_or("");

            return Ok(Some(InputSource::Content(ContentSource::Edit {
                file_path: file_path.to_string(),
                old_content: old_string.to_string(),
                new_content: new_string.to_string(),
            })));
        }

        // Unrecognized tool - allow by default (fail-open)
        Ok(None)
    }

    /// Read command from PreToolUse JSON on stdin (legacy method for backward compatibility)
    ///
    /// Returns None if the input isn't a Bash command (e.g., Write/Edit)
    #[deprecated(note = "Use read_from_stdin() which returns InputSource instead")]
    pub fn read_command_from_stdin(&self) -> Result<Option<CommandSource>> {
        match self.read_from_stdin()? {
            Some(InputSource::Command(cmd)) => Ok(Some(cmd)),
            Some(InputSource::Content(_)) => Ok(None),
            None => Ok(None),
        }
    }

    /// Read command from wrapped-command argv (PATH-wrapper mode)
    ///
    /// Takes the process's argv and returns it as a CommandSource
    pub fn read_from_argv(&self, argv: Vec<String>) -> CommandSource {
        CommandSource::Argv(argv)
    }

    /// Segment a command string into normalized tokens
    ///
    /// This is the core segmentation logic from org-rule-guard.py's check_bash:
    /// - Splits on ;, &&, ||, |, &, newline
    /// - Skips sudo, env assignments, wrapper prefixes
    /// - Basename-matches executables against tool_keywords
    ///
    /// Returns a list of command tokens, one per segment found in the input
    pub fn segment_command(&self, source: &CommandSource) -> Vec<CommandToken> {
        let (Some(segment_splitter), Some(env_assign_pattern)) =
            (&self.segment_splitter, &self.env_assign_pattern)
        else {
            return vec![];
        };

        let command_text = match source {
            CommandSource::Hook(cmd) => cmd.clone(),
            CommandSource::Argv(argv) => {
                if argv.is_empty() {
                    return vec![];
                }
                // Reconstruct command from argv for segmentation
                // Note: this loses quoting information; for proper shell parsing,
                // the hook mode (CommandSource::Hook) should be preferred
                argv.join(" ")
            }
        };

        let mut tokens = Vec::new();

        // Split the command on segment boundaries
        for segment in segment_splitter.split(&command_text) {
            let segment = segment.trim();
            if segment.is_empty() {
                continue;
            }

            // Split segment into whitespace-separated tokens
            let toks: Vec<&str> = segment.split_whitespace().collect();
            if toks.is_empty() {
                continue;
            }

            // Skip sudo / env assignments / command wrappers
            let mut i = 0;
            while i < toks.len() {
                let tok = toks[i];

                // Check if this token should be skipped
                if self.ignored_prefixes.contains(&tok.to_string()) {
                    i += 1;
                    continue;
                }

                // Check if this is an env assignment (e.g., VAR=value)
                if env_assign_pattern.is_match(tok) {
                    i += 1;
                    continue;
                }

                // Found the actual executable
                break;
            }

            // If we've exhausted all tokens or found nothing, skip this segment
            if i >= toks.len() {
                continue;
            }

            // Extract basename from the executable path
            // e.g., /usr/local/bin/vault -> vault
            let executable = toks[i]
                .rsplit('/')
                .next()
                .unwrap_or(toks[i])
                .to_string();

            // Collect remaining arguments
            let args: Vec<String> = toks[i + 1..].iter().map(|s| s.to_string()).collect();

            tokens.push(CommandToken {
                executable,
                args,
            });
        }

        tokens
    }

    /// Get all unique executable basenames from a command source
    ///
    /// This is used for pack dispatch: each pack registers tool_keywords
    /// (e.g., ["vault", "bao"]), and we match tokens against those keywords
    pub fn get_executables(&self, source: &CommandSource) -> Vec<String> {
        self.segment_command(source)
            .into_iter()
            .map(|token| token.executable)
            .collect()
    }

    /// Load a rule pack into the engine for pack dispatch
    ///
    /// This builds an index from tool_keywords to pack_ids so that when
    /// we evaluate a command, we can quickly find which packs to check.
    pub fn load_pack(&mut self, pack: crate::rule_pack::Pack) -> Result<()> {
        let result = catch_unwind(AssertUnwindSafe(|| self.load_pack_inner(pack)));
        match result {
            Ok(Ok(())) => Ok(()),
            Ok(Err(error)) => {
                self.mark_fail_open(&format!("rule pack failure: {error}"));
                Ok(())
            }
            Err(_) => {
                self.mark_fail_open("rule pack loading panicked");
                Ok(())
            }
        }
    }

    fn load_pack_inner(&mut self, pack: crate::rule_pack::Pack) -> Result<()> {
        validate_pack_regexes(&pack)?;
        let pack_id = pack.id.clone();

        // Index tool_keywords for fast dispatch
        for keyword in &pack.tool_keywords {
            self.keyword_index
                .entry(keyword.clone())
                .or_insert_with(Vec::new)
                .push(pack_id.clone());
        }

        // Store the pack itself
        self.packs.insert(pack_id.clone(), pack);

        Ok(())
    }

    /// Load a per-repository override only after proving its scope, trusted
    /// release reference, freshness, and rule IDs against every loaded pack.
    /// A malformed or untrusted override is an error and is never installed.
    pub fn load_verified_override(
        &mut self,
        manifest: &crate::overrides::RepoOverride,
        repository: &str,
        trusted_ref: &str,
    ) -> Result<()> {
        let packs: Vec<crate::rule_pack::Pack> = self.packs.values().cloned().collect();
        crate::overrides::validate_override(manifest, repository, trusted_ref, &packs)?;
        self.exempted_rule_ids = manifest
            .exempted_rule_ids
            .iter()
            .cloned()
            .collect();
        Ok(())
    }

    /// Load a verified override from its release artifact.
    pub fn load_verified_override_from_file<P: AsRef<Path>>(
        &mut self,
        path: P,
        repository: &str,
        trusted_ref: &str,
    ) -> Result<()> {
        let packs: Vec<crate::rule_pack::Pack> = self.packs.values().cloned().collect();
        let manifest = crate::overrides::load_verified_override(
            path,
            repository,
            trusted_ref,
            &packs,
        )?;
        self.exempted_rule_ids = manifest
            .exempted_rule_ids
            .into_iter()
            .collect();
        Ok(())
    }

    /// Load one rule pack file. Any unreadable, malformed, or otherwise
    /// invalid file leaves this engine in its unconditional fail-open state.
    pub fn load_pack_from_file<P: AsRef<Path>>(&mut self, path: P) -> Result<()> {
        let path = path.as_ref().to_path_buf();
        let result = catch_unwind(AssertUnwindSafe(|| {
            let pack = crate::rule_pack::load_pack(&path)
                .with_context(|| format!("Failed to load rule pack from: {}", path.display()))?;
            self.load_pack(pack)
        }));

        match result {
            Ok(Ok(())) => Ok(()),
            Ok(Err(error)) => {
                self.mark_fail_open(&format!("rule pack failure: {error}"));
                Ok(())
            }
            Err(_) => {
                self.mark_fail_open("rule pack loading panicked");
                Ok(())
            }
        }
    }

    /// Load rule packs from a directory
    ///
    /// Reads all .json files from the directory and loads them as packs.
    pub fn load_packs_from_dir<P: AsRef<Path>>(&mut self, dir: P) -> Result<()> {
        let dir = dir.as_ref();
        let result = catch_unwind(AssertUnwindSafe(|| self.load_packs_from_dir_inner(dir)));
        match result {
            Ok(Ok(())) => Ok(()),
            Ok(Err(error)) => {
                self.mark_fail_open(&format!("rule pack directory failure: {error}"));
                Ok(())
            }
            Err(_) => {
                self.mark_fail_open("rule pack directory loading panicked");
                Ok(())
            }
        }
    }

    fn load_packs_from_dir_inner(&mut self, dir: &Path) -> Result<()> {
        let entries = std::fs::read_dir(dir)
            .with_context(|| format!("Failed to read pack directory: {}", dir.display()))?;

        for entry in entries {
            let entry = entry.context("Failed to read directory entry")?;
            let path = entry.path();

            // Only load .json files
            if path.extension().and_then(|e| e.to_str()) == Some("json") {
                let pack = crate::rule_pack::load_pack(&path)
                    .with_context(|| format!("Failed to load pack from: {}", path.display()))?;
                self.load_pack(pack)?;
            }
        }

        Ok(())
    }

    /// Evaluate a command token against loaded rule packs
    ///
    /// This is the core pack dispatch logic:
    /// 1. Find packs whose tool_keywords match the token's executable
    /// 2. Also include packs with empty tool_keywords (unconditional packs like secrets)
    /// 3. For each matching pack, check safe_patterns first (skip the rest if any match)
    /// 4. Then check guarded_patterns (return first match)
    /// 5. Return the most severe result (deny > rewrite > warning > allowed)
    pub fn evaluate_token(&self, token: &CommandToken) -> CheckResult {
        match catch_unwind(AssertUnwindSafe(|| self.evaluate_token_inner(token))) {
            Ok(result) => result,
            Err(_) => {
                report_fail_open("token evaluation panicked");
                CheckResult::Allowed
            }
        }
    }

    fn evaluate_token_inner(&self, token: &CommandToken) -> CheckResult {
        if self.should_fail_open() {
            return CheckResult::Allowed;
        }

        let executable = &token.executable;

        // Reconstruct the full command string for regex matching
        let full_command = format!("{} {}", executable, token.args.join(" "));

        // Find packs that match this executable via tool_keywords
        let mut matching_pack_ids: Vec<String> = self.keyword_index
            .get(executable)
            .map(|ids| ids.iter().cloned().collect())
            .unwrap_or_default();

        // Also include packs with empty tool_keywords (unconditional packs like secrets)
        // These packs scan the entire raw Bash command string regardless of executable
        for (pack_id, pack) in &self.packs {
            if pack.tool_keywords.is_empty() {
                matching_pack_ids.push(pack_id.clone());
            }
        }

        if matching_pack_ids.is_empty() {
            // No packs match this executable - allow by default (fail-open)
            return CheckResult::Allowed;
        }

        // Check each matching pack
        let mut result = CheckResult::Allowed;

        for pack_id in &matching_pack_ids {
            let pack = match self.packs.get(pack_id) {
                Some(p) => p,
                None => continue, // Pack not loaded - skip
            };

            // Check safe_patterns first - if any match, skip this pack entirely
            let safe_match = pack.safe_patterns.iter().find_map(|pattern| {
                match self.pattern_matches_command(pattern, &full_command) {
                    Ok(true) => Some(Ok(true)),
                    Ok(false) => None,
                    Err(()) => Some(Err(())),
                }
            });

            if matches!(safe_match, Some(Err(()))) {
                return CheckResult::Allowed;
            }

            if matches!(safe_match, Some(Ok(true))) {
                // Safe pattern matched - this pack doesn't apply
                continue;
            }

            // Check guarded_patterns
            for guarded_pattern in &pack.guarded_patterns {
                if !guarded_pattern.enabled
                    || self.exempted_rule_ids.contains(&guarded_pattern.id)
                {
                    continue;
                }
                // Create a temporary Pattern wrapper for the guarded pattern's check
                let pattern_wrapper = crate::rule_pack::Pattern {
                    id: guarded_pattern.id.clone(),
                    check: guarded_pattern.check.clone(),
                };

                let matches = match self.pattern_matches_command(&pattern_wrapper, &full_command) {
                    Ok(matches) => matches,
                    Err(()) => return CheckResult::Allowed,
                };

                if matches {
                    // Pattern matched - convert to CheckResult
                    let pattern_result = self.guarded_pattern_to_result(
                        guarded_pattern,
                        pack_id,
                        &full_command,
                    );

                    // Return immediately if this is a deny (most severe)
                    if matches!(pattern_result, CheckResult::Denied { .. }) {
                        return pattern_result;
                    }

                    // Otherwise, track the most severe result so far
                    result = self.most_severe_result(result, pattern_result);
                }
            }
        }

        result
    }

    /// Evaluate all command tokens and return the most severe result
    ///
    /// This is the main entry point for command-mode checking:
    /// - Segments the command into tokens
    /// - Evaluates each token against loaded packs
    /// - Returns the most severe result across all tokens
    pub fn evaluate_command(&self, source: &CommandSource) -> CheckResult {
        match catch_unwind(AssertUnwindSafe(|| self.evaluate_command_inner(source))) {
            Ok(result) => result,
            Err(_) => {
                report_fail_open("command evaluation panicked");
                CheckResult::Allowed
            }
        }
    }

    fn evaluate_command_inner(&self, source: &CommandSource) -> CheckResult {
        if self.should_fail_open() {
            return CheckResult::Allowed;
        }

        let tokens = self.segment_command(source);

        if tokens.is_empty() {
            return CheckResult::Allowed;
        }

        let mut result = CheckResult::Allowed;

        for token in &tokens {
            let token_result = self.evaluate_token_inner(token);
            result = self.most_severe_result(result, token_result);

            // Early exit on deny
            if matches!(result, CheckResult::Denied { .. }) {
                return result;
            }
        }

        result
    }

    /// Check if a pattern matches a command string
    fn pattern_matches_command(
        &self,
        pattern: &crate::rule_pack::Pattern,
        command: &str,
    ) -> std::result::Result<bool, ()> {
        match &pattern.check {
            crate::rule_pack::Check::CommandRegex { regex } => {
                // Compile and match the regex
                match Regex::new(regex) {
                    Ok(re) => Ok(re.is_match(command)),
                    Err(_) => {
                        // An invalid pattern invalidates the check, rather than
                        // merely becoming a non-match that lets another pattern deny.
                        report_fail_open(&format!(
                            "invalid regex in pattern '{}'",
                            pattern.id
                        ));
                        Err(())
                    }
                }
            }
            crate::rule_pack::Check::ContentRegex { .. } => {
                // Content regex doesn't apply to command-mode checks
                Ok(false)
            }
            crate::rule_pack::Check::Predicate { .. } => {
                // Predicate checks are not yet implemented for command-mode
                Ok(false)
            }
        }
    }

    /// Convert a GuardedPattern to a CheckResult
    fn guarded_pattern_to_result(
        &self,
        pattern: &crate::rule_pack::GuardedPattern,
        pack_id: &str,
        command: &str,
    ) -> CheckResult {
        match pattern.redirect.channel {
            crate::rule_pack::Channel::Deny => CheckResult::Denied {
                reason: pattern.redirect.reason_template.clone(),
                pack_id: pack_id.to_string(),
                pattern_id: pattern.id.clone(),
            },
            crate::rule_pack::Channel::UpdatedInput => {
                let rewrite = pattern.redirect.rewrite_template.clone()
                    .unwrap_or_else(|| command.to_string());
                CheckResult::Rewrite {
                    reason: pattern.redirect.reason_template.clone(),
                    rewrite,
                    pack_id: pack_id.to_string(),
                    pattern_id: pattern.id.clone(),
                }
            }
            crate::rule_pack::Channel::AdditionalContext => CheckResult::Warning {
                reason: pattern.redirect.reason_template.clone(),
                pack_id: pack_id.to_string(),
                pattern_id: pattern.id.clone(),
            },
        }
    }

    /// Determine the most severe of two check results
    ///
    /// Severity order: Deny > Rewrite > Warning > Allowed
    fn most_severe_result(&self, a: CheckResult, b: CheckResult) -> CheckResult {
        match (&a, &b) {
            (CheckResult::Denied { .. }, _) => a,
            (_, CheckResult::Denied { .. }) => b,
            (CheckResult::Rewrite { .. }, CheckResult::Allowed) => a,
            (CheckResult::Allowed, CheckResult::Rewrite { .. }) => b,
            (CheckResult::Rewrite { .. }, _) => a,
            (_, CheckResult::Rewrite { .. }) => b,
            (CheckResult::Warning { .. }, CheckResult::Allowed) => a,
            (CheckResult::Allowed, CheckResult::Warning { .. }) => b,
            _ => a,
        }
    }

    /// Return packs whose `applies_to` selectors match a Write/Edit target.
    ///
    /// The returned references preserve the input pack order and include every
    /// matching pack: a YAML write may be relevant to both the storage-class
    /// and image-tag packs. The same selector dispatch is intentionally used by
    /// the beads pack, whose actual check is a filesystem predicate rather than
    /// a content regex.
    pub fn dispatch_content_packs<'a>(
        &self,
        source: &ContentSource,
        packs: &'a [Pack],
    ) -> Vec<&'a Pack> {
        match catch_unwind(AssertUnwindSafe(|| {
            if self.should_fail_open() {
                return Vec::new();
            }
            packs
                .iter()
                .filter(|pack| {
                    pack.applies_to
                        .iter()
                        .any(|glob| source.matches_glob(glob))
                })
                .collect()
        })) {
            Ok(packs) => packs,
            Err(_) => {
                report_fail_open("content pack dispatch panicked");
                Vec::new()
            }
        }
    }

    /// Evaluate content (Write/Edit operations) against loaded rule packs
    ///
    /// This is the main entry point for content-mode checking:
    /// - Dispatches to packs whose applies_to globs match the file path
    /// - Checks safe_patterns first (skip pack if any match)
    /// - Then checks guarded_patterns (return first match)
    /// - Returns the most severe result across all matching packs
    pub fn evaluate_content(&self, source: &ContentSource) -> CheckResult {
        match catch_unwind(AssertUnwindSafe(|| self.evaluate_content_inner(source))) {
            Ok(result) => result,
            Err(_) => {
                report_fail_open("content evaluation panicked");
                CheckResult::Allowed
            }
        }
    }

    fn evaluate_content_inner(&self, source: &ContentSource) -> CheckResult {
        if self.should_fail_open() {
            return CheckResult::Allowed;
        }

        // Collect all loaded packs for dispatch
        let packs: Vec<&Pack> = self.packs.values().collect();
        let packs_slice: &[Pack] = &self.packs.values().cloned().collect::<Vec<_>>();

        // Find packs whose applies_to selectors match this file
        let matching_packs = self.dispatch_content_packs(source, packs_slice);

        if matching_packs.is_empty() {
            // No packs match this file type - allow by default (fail-open)
            return CheckResult::Allowed;
        }

        // Check each matching pack
        let mut result = CheckResult::Allowed;

        for pack in matching_packs {
            // Check safe_patterns first - if any match, skip this pack entirely
            let safe_match = pack.safe_patterns.iter().find_map(|pattern| {
                match self.pattern_matches_content(pattern, source) {
                    Ok(true) => Some(Ok(true)),
                    Ok(false) => None,
                    Err(()) => Some(Err(())),
                }
            });

            if matches!(safe_match, Some(Err(()))) {
                return CheckResult::Allowed;
            }

            if matches!(safe_match, Some(Ok(true))) {
                // Safe pattern matched - this pack doesn't apply
                continue;
            }

            // Check guarded_patterns
            for guarded_pattern in &pack.guarded_patterns {
                if !guarded_pattern.enabled
                    || self.exempted_rule_ids.contains(&guarded_pattern.id)
                {
                    continue;
                }
                // Create a temporary Pattern wrapper for the guarded pattern's check
                let pattern_wrapper = crate::rule_pack::Pattern {
                    id: guarded_pattern.id.clone(),
                    check: guarded_pattern.check.clone(),
                };

                let matches = match self.pattern_matches_content(&pattern_wrapper, source) {
                    Ok(matches) => matches,
                    Err(()) => return CheckResult::Allowed,
                };

                if matches {
                    // Pattern matched - convert to CheckResult
                    let pattern_result = self.guarded_pattern_to_result_content(
                        guarded_pattern,
                        &pack.id,
                        source.file_path(),
                    );

                    // Return immediately if this is a deny (most severe)
                    if matches!(pattern_result, CheckResult::Denied { .. }) {
                        return pattern_result;
                    }

                    // Otherwise, track the most severe result so far
                    result = self.most_severe_result(result, pattern_result);
                }
            }
        }

        result
    }

    /// Check if a pattern matches content (for Write/Edit operations)
    fn pattern_matches_content(
        &self,
        pattern: &crate::rule_pack::Pattern,
        source: &ContentSource,
    ) -> std::result::Result<bool, ()> {
        match &pattern.check {
            crate::rule_pack::Check::ContentRegex { regex } => {
                // Compile and match the regex against the new content
                match Regex::new(regex) {
                    Ok(re) => {
                        let content_to_check = source.new_content();
                        Ok(re.is_match(content_to_check))
                    }
                    Err(_) => {
                        // An invalid pattern invalidates the check
                        report_fail_open(&format!(
                            "invalid regex in pattern '{}'",
                            pattern.id
                        ));
                        Err(())
                    }
                }
            }
            crate::rule_pack::Check::CommandRegex { .. } => {
                // Command regex doesn't apply to content-mode checks
                Ok(false)
            }
            crate::rule_pack::Check::Predicate { .. } => {
                // Predicate checks are not yet implemented for content-mode
                // (e.g., beads pack filesystem check)
                Ok(false)
            }
        }
    }

    /// Convert a GuardedPattern to a CheckResult for content-mode
    fn guarded_pattern_to_result_content(
        &self,
        pattern: &crate::rule_pack::GuardedPattern,
        pack_id: &str,
        file_path: &str,
    ) -> CheckResult {
        match pattern.redirect.channel {
            crate::rule_pack::Channel::Deny => CheckResult::Denied {
                reason: pattern.redirect.reason_template.clone(),
                pack_id: pack_id.to_string(),
                pattern_id: pattern.id.clone(),
            },
            crate::rule_pack::Channel::UpdatedInput => {
                // For content-mode, updatedInput would provide corrected content
                // This is not yet implemented - would require rewrite_template to
                // specify the replacement content
                let reason = format!(
                    "{} (content-mode updatedInput not yet implemented)",
                    pattern.redirect.reason_template
                );
                CheckResult::Rewrite {
                    reason,
                    rewrite: format!("<corrected content for {}>", file_path),
                    pack_id: pack_id.to_string(),
                    pattern_id: pattern.id.clone(),
                }
            }
            crate::rule_pack::Channel::AdditionalContext => CheckResult::Warning {
                reason: pattern.redirect.reason_template.clone(),
                pack_id: pack_id.to_string(),
                pattern_id: pattern.id.clone(),
            },
        }
    }
}

fn validate_pack_regexes(pack: &Pack) -> Result<()> {
    for pattern in &pack.safe_patterns {
        validate_check_regex(&pattern.check)?;
    }
    for pattern in &pack.guarded_patterns {
        validate_check_regex(&pattern.check)?;
    }
    Ok(())
}

fn validate_check_regex(check: &crate::rule_pack::Check) -> Result<()> {
    let regex = match check {
        crate::rule_pack::Check::CommandRegex { regex }
        | crate::rule_pack::Check::ContentRegex { regex } => Some(regex),
        crate::rule_pack::Check::Predicate { .. } => None,
    };

    if let Some(regex) = regex {
        Regex::new(regex).context("invalid regex in rule pack")?;
    }
    Ok(())
}

fn report_fail_open(message: &str) {
    use std::io::Write;
    let _ = writeln!(std::io::stderr(), "Engine: fail-open: {message}");
}

#[cfg(test)]
mod tests {
    use super::*;

    fn default_engine() -> Engine {
        Engine::new()
    }

    // Content-mode tests

    #[test]
    fn test_content_source_write_file_path() {
        let source = ContentSource::Write {
            file_path: "/path/to/file.yaml".to_string(),
            content: "some content".to_string(),
        };
        assert_eq!(source.file_path(), "/path/to/file.yaml");
    }

    #[test]
    fn test_content_source_edit_file_path() {
        let source = ContentSource::Edit {
            file_path: "/path/to/file.yaml".to_string(),
            old_content: "old".to_string(),
            new_content: "new".to_string(),
        };
        assert_eq!(source.file_path(), "/path/to/file.yaml");
    }

    #[test]
    fn test_content_source_write_new_content() {
        let source = ContentSource::Write {
            file_path: "/path/to/file.yaml".to_string(),
            content: "new content".to_string(),
        };
        assert_eq!(source.new_content(), "new content");
    }

    #[test]
    fn test_content_source_edit_new_content() {
        let source = ContentSource::Edit {
            file_path: "/path/to/file.yaml".to_string(),
            old_content: "old content".to_string(),
            new_content: "new content".to_string(),
        };
        assert_eq!(source.new_content(), "new content");
    }

    #[test]
    fn test_content_source_write_content_to_check() {
        let source = ContentSource::Write {
            file_path: "/path/to/file.yaml".to_string(),
            content: "full content".to_string(),
        };
        let to_check = source.content_to_check();
        assert_eq!(to_check.len(), 1);
        assert_eq!(to_check[0], "full content");
    }

    #[test]
    fn test_content_source_edit_content_to_check() {
        let source = ContentSource::Edit {
            file_path: "/path/to/file.yaml".to_string(),
            old_content: "old content".to_string(),
            new_content: "new content".to_string(),
        };
        let to_check = source.content_to_check();
        assert_eq!(to_check.len(), 2);
        assert_eq!(to_check[0], "old content");
        assert_eq!(to_check[1], "new content");
    }

    #[test]
    fn test_matches_glob_yaml_extension() {
        let source = ContentSource::Write {
            file_path: "/path/to/file.yaml".to_string(),
            content: "content".to_string(),
        };
        assert!(source.matches_glob("*.yaml"));
        // .yaml should NOT match *.yml (different extension)
        assert!(!source.matches_glob("*.yml"));
        assert!(!source.matches_glob("*.md"));
    }

    #[test]
    fn test_matches_glob_yml_extension() {
        let source = ContentSource::Edit {
            file_path: "/path/to/file.yml".to_string(),
            old_content: "old".to_string(),
            new_content: "new".to_string(),
        };
        assert!(source.matches_glob("*.yml"));
        assert!(!source.matches_glob("*.yaml"));
    }

    #[test]
    fn test_matches_glob_is_basename_aware() {
        let source = ContentSource::Write {
            file_path: "/repo/manifests/service.yaml".to_string(),
            content: "content".to_string(),
        };

        assert!(source.matches_glob("*.yaml"));
        assert!(source.matches_glob("manifests/*.yaml"));
        assert!(source.matches_glob("**/*.yaml"));
        assert!(!source.matches_glob("manifests/*.yml"));
    }

    #[test]
    fn test_matches_glob_scopes_beads_to_a_path_component() {
        let source = ContentSource::Write {
            file_path: "/repo/.beads/checkpoint/current.json".to_string(),
            content: "content".to_string(),
        };
        let unrelated = ContentSource::Write {
            file_path: "/repo/.beads-old/checkpoint/current.json".to_string(),
            content: "content".to_string(),
        };

        assert!(source.matches_glob(".beads/**"));
        assert!(source.matches_glob(".beads/"));
        assert!(!unrelated.matches_glob(".beads/**"));
    }

    fn test_pack(id: &str, applies_to: &[&str]) -> Pack {
        Pack {
            id: id.to_string(),
            tool_keywords: Vec::new(),
            applies_to: applies_to.iter().map(|glob| (*glob).to_string()).collect(),
            safe_patterns: Vec::new(),
            guarded_patterns: Vec::new(),
        }
    }

    #[test]
    fn test_dispatch_content_packs_returns_all_matching_packs_in_order() {
        let engine = default_engine();
        let source = ContentSource::Write {
            file_path: "/repo/manifests/service.yaml".to_string(),
            content: "content".to_string(),
        };
        let packs = vec![
            test_pack("storage-class", &["*.yaml", "*.yml"]),
            test_pack("image-tag", &["*.yaml", "*.yml"]),
            test_pack("markdown", &["*.md"]),
            test_pack("command-only", &[]),
        ];

        let matching = engine.dispatch_content_packs(&source, &packs);

        assert_eq!(
            matching.iter().map(|pack| pack.id.as_str()).collect::<Vec<_>>(),
            vec!["storage-class", "image-tag"]
        );
    }

    #[test]
    fn test_dispatch_content_packs_also_routes_beads_selector() {
        let engine = default_engine();
        let source = ContentSource::Edit {
            file_path: "/repo/.beads/events.jsonl".to_string(),
            old_content: "old".to_string(),
            new_content: "new".to_string(),
        };
        let packs = vec![
            test_pack("storage-class", &["*.yaml", "*.yml"]),
            test_pack("beads", &[".beads/**"]),
        ];

        let matching = engine.dispatch_content_packs(&source, &packs);

        assert_eq!(matching.len(), 1);
        assert_eq!(matching[0].id, "beads");
    }

    // Command-mode tests (existing)

    #[test]
    fn test_segment_simple_command() {
        let engine = default_engine();
        let source = CommandSource::Hook("vault kv destroy secret/foo".to_string());
        let tokens = engine.segment_command(&source);

        assert_eq!(tokens.len(), 1);
        assert_eq!(tokens[0].executable, "vault");
        assert_eq!(tokens[0].args, vec!["kv", "destroy", "secret/foo"]);
    }

    #[test]
    fn test_segment_with_path() {
        let engine = default_engine();
        let source = CommandSource::Hook("/usr/local/bin/vault kv destroy secret/foo".to_string());
        let tokens = engine.segment_command(&source);

        assert_eq!(tokens.len(), 1);
        assert_eq!(tokens[0].executable, "vault");
        assert_eq!(tokens[0].args, vec!["kv", "destroy", "secret/foo"]);
    }

    #[test]
    fn test_segment_with_sudo() {
        let engine = default_engine();
        let source = CommandSource::Hook("sudo vault kv destroy secret/foo".to_string());
        let tokens = engine.segment_command(&source);

        assert_eq!(tokens.len(), 1);
        assert_eq!(tokens[0].executable, "vault");
        assert_eq!(tokens[0].args, vec!["kv", "destroy", "secret/foo"]);
    }

    #[test]
    fn test_segment_with_env_assignment() {
        let engine = default_engine();
        let source = CommandSource::Hook("VAULT_TOKEN=xyz vault kv destroy secret/foo".to_string());
        let tokens = engine.segment_command(&source);

        assert_eq!(tokens.len(), 1);
        assert_eq!(tokens[0].executable, "vault");
        assert_eq!(tokens[0].args, vec!["kv", "destroy", "secret/foo"]);
    }

    #[test]
    fn test_segment_with_wrapper_prefix() {
        let engine = default_engine();
        let source = CommandSource::Hook("command vault kv destroy secret/foo".to_string());
        let tokens = engine.segment_command(&source);

        assert_eq!(tokens.len(), 1);
        assert_eq!(tokens[0].executable, "vault");
        assert_eq!(tokens[0].args, vec!["kv", "destroy", "secret/foo"]);
    }

    #[test]
    fn test_segment_multiple_commands_semicolon() {
        let engine = default_engine();
        let source = CommandSource::Hook("vault status; git status".to_string());
        let tokens = engine.segment_command(&source);

        assert_eq!(tokens.len(), 2);
        assert_eq!(tokens[0].executable, "vault");
        assert_eq!(tokens[1].executable, "git");
    }

    #[test]
    fn test_segment_multiple_commands_and() {
        let engine = default_engine();
        let source = CommandSource::Hook("vault status && git status".to_string());
        let tokens = engine.segment_command(&source);

        assert_eq!(tokens.len(), 2);
        assert_eq!(tokens[0].executable, "vault");
        assert_eq!(tokens[1].executable, "git");
    }

    #[test]
    fn test_segment_multiple_commands_or() {
        let engine = default_engine();
        let source = CommandSource::Hook("vault status || git status".to_string());
        let tokens = engine.segment_command(&source);

        assert_eq!(tokens.len(), 2);
        assert_eq!(tokens[0].executable, "vault");
        assert_eq!(tokens[1].executable, "git");
    }

    #[test]
    fn test_segment_pipe() {
        let engine = default_engine();
        let source = CommandSource::Hook("cat file | grep foo".to_string());
        let tokens = engine.segment_command(&source);

        assert_eq!(tokens.len(), 2);
        assert_eq!(tokens[0].executable, "cat");
        assert_eq!(tokens[1].executable, "grep");
    }

    #[test]
    fn test_get_executables() {
        let engine = default_engine();
        let source = CommandSource::Hook("vault status; git status; kubectl get pods".to_string());
        let execs = engine.get_executables(&source);

        assert_eq!(execs, vec!["vault", "git", "kubectl"]);
    }

    #[test]
    fn test_argv_mode() {
        let engine = default_engine();
        let argv = vec![
            "/usr/bin/vault".to_string(),
            "kv".to_string(),
            "destroy".to_string(),
            "secret/foo".to_string(),
        ];
        let source = engine.read_from_argv(argv);
        let tokens = engine.segment_command(&source);

        assert_eq!(tokens.len(), 1);
        assert_eq!(tokens[0].executable, "vault");
        assert_eq!(tokens[0].args, vec!["kv", "destroy", "secret/foo"]);
    }

    // Pack dispatch tests

    #[test]
    fn test_load_pack() {
        let mut engine = default_engine();

        let pack = crate::rule_pack::Pack {
            id: "test-pack".to_string(),
            tool_keywords: vec!["vault".to_string(), "bao".to_string()],
            applies_to: vec![],
            safe_patterns: vec![],
            guarded_patterns: vec![],
        };

        engine.load_pack(pack).unwrap();

        // Verify pack is loaded
        assert!(engine.packs.contains_key("test-pack"));
        assert_eq!(engine.keyword_index.get("vault").unwrap().len(), 1);
        assert_eq!(engine.keyword_index.get("bao").unwrap().len(), 1);
    }

    #[test]
    fn test_load_multiple_packs() {
        let mut engine = default_engine();

        let vault_pack = crate::rule_pack::Pack {
            id: "vault".to_string(),
            tool_keywords: vec!["vault".to_string()],
            applies_to: vec![],
            safe_patterns: vec![],
            guarded_patterns: vec![],
        };

        let git_pack = crate::rule_pack::Pack {
            id: "git".to_string(),
            tool_keywords: vec!["git".to_string()],
            applies_to: vec![],
            safe_patterns: vec![],
            guarded_patterns: vec![],
        };

        engine.load_pack(vault_pack).unwrap();
        engine.load_pack(git_pack).unwrap();

        assert_eq!(engine.packs.len(), 2);
        assert_eq!(engine.keyword_index.get("vault").unwrap().len(), 1);
        assert_eq!(engine.keyword_index.get("git").unwrap().len(), 1);
    }

    #[test]
    fn test_invalid_rule_regex_fails_open() {
        let mut engine = default_engine();
        let pack = crate::rule_pack::Pack {
            id: "corrupt-regex".to_string(),
            tool_keywords: vec!["vault".to_string()],
            applies_to: vec![],
            safe_patterns: vec![],
            guarded_patterns: vec![crate::rule_pack::GuardedPattern {
                id: "invalid".to_string(),
                enabled: true,
                check: crate::rule_pack::Check::CommandRegex {
                    regex: "[".to_string(),
                },
                tier: crate::rule_pack::Tier::Tier1,
                severity: crate::rule_pack::Severity::Critical,
                explanation: "Invalid regex must never deny".to_string(),
                redirect: crate::rule_pack::Redirect {
                    channel: crate::rule_pack::Channel::Deny,
                    reason_template: "must not be returned".to_string(),
                    rewrite_template: None,
                },
                destructive: true,
            }],
        };

        engine.load_pack(pack).unwrap();
        let result = engine.evaluate_command(&CommandSource::Hook(
            "vault kv destroy secret/foo".to_string(),
        ));

        assert_eq!(result, CheckResult::Allowed);
    }

    #[test]
    fn test_evaluate_token_no_packs_loaded() {
        let engine = default_engine();
        let token = CommandToken {
            executable: "vault".to_string(),
            args: vec!["kv".to_string(), "destroy".to_string()],
        };

        let result = engine.evaluate_token(&token);
        assert_eq!(result, CheckResult::Allowed);
    }

    #[test]
    fn test_evaluate_token_safe_pattern_matches() {
        let mut engine = default_engine();

        let pack = crate::rule_pack::Pack {
            id: "vault".to_string(),
            tool_keywords: vec!["vault".to_string()],
            applies_to: vec![],
            safe_patterns: vec![crate::rule_pack::Pattern {
                id: "safe-read".to_string(),
                check: crate::rule_pack::Check::CommandRegex {
                    regex: "vault kv get".to_string(),
                },
            }],
            guarded_patterns: vec![],
        };

        engine.load_pack(pack).unwrap();

        let token = CommandToken {
            executable: "vault".to_string(),
            args: vec!["kv".to_string(), "get".to_string(), "secret/foo".to_string()],
        };

        let result = engine.evaluate_token(&token);
        assert_eq!(result, CheckResult::Allowed);
    }

    #[test]
    fn test_evaluate_token_guarded_pattern_deny() {
        let mut engine = default_engine();

        let pack = crate::rule_pack::Pack {
            id: "vault".to_string(),
            tool_keywords: vec!["vault".to_string()],
            applies_to: vec![],
            safe_patterns: vec![],
            guarded_patterns: vec![crate::rule_pack::GuardedPattern {
                id: "vault-kv-destroy".to_string(),
                enabled: true,
                check: crate::rule_pack::Check::CommandRegex {
                    regex: "vault kv destroy".to_string(),
                },
                tier: crate::rule_pack::Tier::Tier1,
                severity: crate::rule_pack::Severity::Critical,
                explanation: "Destructive operation".to_string(),
                redirect: crate::rule_pack::Redirect {
                    channel: crate::rule_pack::Channel::Deny,
                    reason_template: "vault kv destroy is permanently destructive".to_string(),
                    rewrite_template: None,
                },
                destructive: true,
            }],
        };

        engine.load_pack(pack).unwrap();

        let token = CommandToken {
            executable: "vault".to_string(),
            args: vec!["kv".to_string(), "destroy".to_string(), "secret/foo".to_string()],
        };

        let result = engine.evaluate_token(&token);

        match result {
            CheckResult::Denied { reason, pack_id, pattern_id } => {
                assert_eq!(pack_id, "vault");
                assert_eq!(pattern_id, "vault-kv-destroy");
                assert!(reason.contains("permanently destructive"));
            }
            _ => panic!("Expected Denied result, got {:?}", result),
        }
    }

    #[test]
    fn test_evaluate_token_guarded_pattern_rewrite() {
        let mut engine = default_engine();

        let pack = crate::rule_pack::Pack {
            id: "git".to_string(),
            tool_keywords: vec!["git".to_string()],
            applies_to: vec![],
            safe_patterns: vec![],
            guarded_patterns: vec![crate::rule_pack::GuardedPattern {
                id: "git-force-push".to_string(),
                enabled: true,
                check: crate::rule_pack::Check::CommandRegex {
                    regex: "git push.*--force".to_string(),
                },
                tier: crate::rule_pack::Tier::Tier1,
                severity: crate::rule_pack::Severity::Critical,
                explanation: "Force-push rewrites history".to_string(),
                redirect: crate::rule_pack::Redirect {
                    channel: crate::rule_pack::Channel::UpdatedInput,
                    reason_template: "Use --force-with-lease instead".to_string(),
                    rewrite_template: Some("git push --force-with-lease".to_string()),
                },
                destructive: true,
            }],
        };

        engine.load_pack(pack).unwrap();

        let token = CommandToken {
            executable: "git".to_string(),
            args: vec!["push".to_string(), "origin".to_string(), "main".to_string(), "--force".to_string()],
        };

        let result = engine.evaluate_token(&token);

        match result {
            CheckResult::Rewrite { reason, rewrite, pack_id, pattern_id } => {
                assert_eq!(pack_id, "git");
                assert_eq!(pattern_id, "git-force-push");
                assert!(reason.contains("force-with-lease"));
                assert!(rewrite.contains("--force-with-lease"));
            }
            _ => panic!("Expected Rewrite result, got {:?}", result),
        }
    }

    #[test]
    fn test_evaluate_token_guarded_pattern_warning() {
        let mut engine = default_engine();

        let pack = crate::rule_pack::Pack {
            id: "git".to_string(),
            tool_keywords: vec!["git".to_string()],
            applies_to: vec![],
            safe_patterns: vec![],
            guarded_patterns: vec![crate::rule_pack::GuardedPattern {
                id: "git-worktree-add".to_string(),
                enabled: true,
                check: crate::rule_pack::Check::CommandRegex {
                    regex: "git worktree add".to_string(),
                },
                tier: crate::rule_pack::Tier::Tier3,
                severity: crate::rule_pack::Severity::Medium,
                explanation: "Worktree can be shared or isolated".to_string(),
                redirect: crate::rule_pack::Redirect {
                    channel: crate::rule_pack::Channel::AdditionalContext,
                    reason_template: "Verify this is a throwaway worktree, not a shared one".to_string(),
                    rewrite_template: None,
                },
                destructive: false,
            }],
        };

        engine.load_pack(pack).unwrap();

        let token = CommandToken {
            executable: "git".to_string(),
            args: vec!["worktree".to_string(), "add".to_string(), "path".to_string(), "branch".to_string()],
        };

        let result = engine.evaluate_token(&token);

        match result {
            CheckResult::Warning { reason, pack_id, pattern_id } => {
                assert_eq!(pack_id, "git");
                assert_eq!(pattern_id, "git-worktree-add");
                assert!(reason.contains("throwaway worktree"));
            }
            _ => panic!("Expected Warning result, got {:?}", result),
        }
    }

    #[test]
    fn test_evaluate_command_with_multiple_tokens() {
        let mut engine = default_engine();

        let vault_pack = crate::rule_pack::Pack {
            id: "vault".to_string(),
            tool_keywords: vec!["vault".to_string()],
            applies_to: vec![],
            safe_patterns: vec![],
            guarded_patterns: vec![crate::rule_pack::GuardedPattern {
                id: "vault-kv-destroy".to_string(),
                enabled: true,
                check: crate::rule_pack::Check::CommandRegex {
                    regex: "vault kv destroy".to_string(),
                },
                tier: crate::rule_pack::Tier::Tier1,
                severity: crate::rule_pack::Severity::Critical,
                explanation: "Destructive".to_string(),
                redirect: crate::rule_pack::Redirect {
                    channel: crate::rule_pack::Channel::Deny,
                    reason_template: "Denied".to_string(),
                    rewrite_template: None,
                },
                destructive: true,
            }],
        };

        engine.load_pack(vault_pack).unwrap();

        let source = CommandSource::Hook("vault status && vault kv destroy secret/foo".to_string());
        let result = engine.evaluate_command(&source);

        // Should deny because one segment is destructive
        match result {
            CheckResult::Denied { .. } => {},
            _ => panic!("Expected Denied result for multi-segment command with destructive operation, got {:?}", result),
        }
    }

    #[test]
    fn test_evaluate_command_all_safe() {
        let mut engine = default_engine();

        let vault_pack = crate::rule_pack::Pack {
            id: "vault".to_string(),
            tool_keywords: vec!["vault".to_string()],
            applies_to: vec![],
            safe_patterns: vec![crate::rule_pack::Pattern {
                id: "safe-status".to_string(),
                check: crate::rule_pack::Check::CommandRegex {
                    regex: "vault status".to_string(),
                },
            }, crate::rule_pack::Pattern {
                id: "safe-read".to_string(),
                check: crate::rule_pack::Check::CommandRegex {
                    regex: "vault kv get".to_string(),
                },
            }],
            guarded_patterns: vec![],
        };

        engine.load_pack(vault_pack).unwrap();

        let source = CommandSource::Hook("vault status && vault kv get secret/foo".to_string());
        let result = engine.evaluate_command(&source);

        assert_eq!(result, CheckResult::Allowed);
    }

    #[test]
    fn test_evaluate_token_safe_pattern_skips_guarded_patterns() {
        // Test that a safe_pattern hit suppresses a guarded_pattern hit that would otherwise fire
        let mut engine = default_engine();

        let pack = crate::rule_pack::Pack {
            id: "vault".to_string(),
            tool_keywords: vec!["vault".to_string()],
            applies_to: vec![],
            safe_patterns: vec![crate::rule_pack::Pattern {
                id: "safe-read".to_string(),
                check: crate::rule_pack::Check::CommandRegex {
                    regex: "vault kv get".to_string(),
                },
            }],
            guarded_patterns: vec![crate::rule_pack::GuardedPattern {
                id: "vault-kv-any".to_string(),
                enabled: true,
                check: crate::rule_pack::Check::CommandRegex {
                    // This broader pattern would match "vault kv get" if safe_pattern didn't skip it
                    regex: "vault kv".to_string(),
                },
                tier: crate::rule_pack::Tier::Tier1,
                severity: crate::rule_pack::Severity::Critical,
                explanation: "Would block all vault kv commands".to_string(),
                redirect: crate::rule_pack::Redirect {
                    channel: crate::rule_pack::Channel::Deny,
                    reason_template: "All vault kv commands are denied".to_string(),
                    rewrite_template: None,
                },
                destructive: true,
            }],
        };

        engine.load_pack(pack).unwrap();

        // This command matches BOTH the safe_pattern (vault kv get) and the guarded_pattern (vault kv)
        // The safe_pattern should take precedence, allowing the command
        let token = CommandToken {
            executable: "vault".to_string(),
            args: vec!["kv".to_string(), "get".to_string(), "secret/foo".to_string()],
        };

        let result = engine.evaluate_token(&token);
        assert_eq!(result, CheckResult::Allowed, "safe_pattern should suppress guarded_pattern");
    }

    #[test]
    fn test_basename_matching_with_absolute_path() {
        let mut engine = default_engine();

        let pack = crate::rule_pack::Pack {
            id: "vault".to_string(),
            tool_keywords: vec!["vault".to_string()], // Only basename, not full path
            applies_to: vec![],
            safe_patterns: vec![],
            guarded_patterns: vec![crate::rule_pack::GuardedPattern {
                id: "vault-kv-destroy".to_string(),
                enabled: true,
                check: crate::rule_pack::Check::CommandRegex {
                    regex: "vault kv destroy".to_string(),
                },
                tier: crate::rule_pack::Tier::Tier1,
                severity: crate::rule_pack::Severity::Critical,
                explanation: "Destructive".to_string(),
                redirect: crate::rule_pack::Redirect {
                    channel: crate::rule_pack::Channel::Deny,
                    reason_template: "Denied".to_string(),
                    rewrite_template: None,
                },
                destructive: true,
            }],
        };

        engine.load_pack(pack).unwrap();

        // Test with absolute path - should still match because we basename-match
        let source = CommandSource::Hook("/usr/local/bin/vault kv destroy secret/foo".to_string());
        let result = engine.evaluate_command(&source);

        match result {
            CheckResult::Denied { .. } => {},
            _ => panic!("Expected Denied result for absolute-path invocation, got {:?}", result),
        }
    }

    #[test]
    fn test_most_severe_result_ordering() {
        let engine = default_engine();

        let deny = CheckResult::Denied {
            reason: "deny".to_string(),
            pack_id: "test".to_string(),
            pattern_id: "test".to_string(),
        };
        let rewrite = CheckResult::Rewrite {
            reason: "rewrite".to_string(),
            rewrite: "rewrite".to_string(),
            pack_id: "test".to_string(),
            pattern_id: "test".to_string(),
        };
        let warning = CheckResult::Warning {
            reason: "warning".to_string(),
            pack_id: "test".to_string(),
            pattern_id: "test".to_string(),
        };
        let allowed = CheckResult::Allowed;

        // Deny is most severe
        assert!(matches!(engine.most_severe_result(deny.clone(), allowed.clone()), CheckResult::Denied { .. }));
        assert!(matches!(engine.most_severe_result(deny.clone(), rewrite.clone()), CheckResult::Denied { .. }));
        assert!(matches!(engine.most_severe_result(deny.clone(), warning.clone()), CheckResult::Denied { .. }));

        // Rewrite is more severe than warning/allowed
        assert!(matches!(engine.most_severe_result(rewrite.clone(), allowed.clone()), CheckResult::Rewrite { .. }));
        assert!(matches!(engine.most_severe_result(rewrite.clone(), warning.clone()), CheckResult::Rewrite { .. }));

        // Warning is more severe than allowed
        assert!(matches!(engine.most_severe_result(warning.clone(), allowed.clone()), CheckResult::Warning { .. }));

        // Same severity returns first
        assert!(matches!(engine.most_severe_result(allowed.clone(), allowed.clone()), CheckResult::Allowed));
    }

    // Content-mode evaluation tests

    #[test]
    fn test_evaluate_content_no_packs_loaded() {
        let engine = default_engine();
        let source = ContentSource::Write {
            file_path: "/path/to/file.yaml".to_string(),
            content: "some content".to_string(),
        };

        let result = engine.evaluate_content(&source);
        assert_eq!(result, CheckResult::Allowed);
    }

    #[test]
    fn test_evaluate_content_safe_pattern_matches() {
        let mut engine = default_engine();

        let pack = crate::rule_pack::Pack {
            id: "storage-class".to_string(),
            tool_keywords: vec![],
            applies_to: vec!["*.yaml".to_string()],
            safe_patterns: vec![crate::rule_pack::Pattern {
                id: "safe-sata".to_string(),
                check: crate::rule_pack::Check::ContentRegex {
                    regex: "storageClassName: sata".to_string(),
                },
            }],
            guarded_patterns: vec![],
        };

        engine.load_pack(pack).unwrap();

        let source = ContentSource::Write {
            file_path: "/path/to/file.yaml".to_string(),
            content: "storageClassName: sata".to_string(),
        };

        let result = engine.evaluate_content(&source);
        assert_eq!(result, CheckResult::Allowed);
    }

    #[test]
    fn test_evaluate_content_guarded_pattern_deny() {
        let mut engine = default_engine();

        let pack = crate::rule_pack::Pack {
            id: "storage-class".to_string(),
            tool_keywords: vec![],
            applies_to: vec!["*.yaml".to_string()],
            safe_patterns: vec![],
            guarded_patterns: vec![crate::rule_pack::GuardedPattern {
                id: "ssd-storage-class".to_string(),
                enabled: true,
                check: crate::rule_pack::Check::ContentRegex {
                    regex: "storageClassName: ssd".to_string(),
                },
                tier: crate::rule_pack::Tier::Tier1,
                severity: crate::rule_pack::Severity::Critical,
                explanation: "SSD storage is prohibited".to_string(),
                redirect: crate::rule_pack::Redirect {
                    channel: crate::rule_pack::Channel::Deny,
                    reason_template: "Never use ssd storage class".to_string(),
                    rewrite_template: None,
                },
                destructive: true,
            }],
        };

        engine.load_pack(pack).unwrap();

        let source = ContentSource::Write {
            file_path: "/path/to/file.yaml".to_string(),
            content: "storageClassName: ssd".to_string(),
        };

        let result = engine.evaluate_content(&source);

        match result {
            CheckResult::Denied { reason, pack_id, pattern_id } => {
                assert_eq!(pack_id, "storage-class");
                assert_eq!(pattern_id, "ssd-storage-class");
                assert!(reason.contains("ssd storage"));
            }
            _ => panic!("Expected Denied result, got {:?}", result),
        }
    }

    #[test]
    fn test_evaluate_content_edit_operation() {
        let mut engine = default_engine();

        let pack = crate::rule_pack::Pack {
            id: "image-tag".to_string(),
            tool_keywords: vec![],
            applies_to: vec!["*.yaml".to_string()],
            safe_patterns: vec![],
            guarded_patterns: vec![crate::rule_pack::GuardedPattern {
                id: "latest-image-tag".to_string(),
                enabled: true,
                check: crate::rule_pack::Check::ContentRegex {
                    regex: "image: .*:latest".to_string(),
                },
                tier: crate::rule_pack::Tier::Tier1,
                severity: crate::rule_pack::Severity::Critical,
                explanation: "Latest tag is ambiguous".to_string(),
                redirect: crate::rule_pack::Redirect {
                    channel: crate::rule_pack::Channel::Deny,
                    reason_template: "Never use :latest image tags".to_string(),
                    rewrite_template: None,
                },
                destructive: true,
            }],
        };

        engine.load_pack(pack).unwrap();

        // Test Edit operation - should check new_content
        let source = ContentSource::Edit {
            file_path: "/path/to/deployment.yaml".to_string(),
            old_content: "image: nginx:1.19".to_string(),
            new_content: "image: nginx:latest".to_string(),
        };

        let result = engine.evaluate_content(&source);

        match result {
            CheckResult::Denied { pack_id, pattern_id, .. } => {
                assert_eq!(pack_id, "image-tag");
                assert_eq!(pattern_id, "latest-image-tag");
            }
            _ => panic!("Expected Denied result for edit with :latest, got {:?}", result),
        }
    }

    #[test]
    fn test_evaluate_content_applies_to_filters_by_file_extension() {
        let mut engine = default_engine();

        let yaml_pack = crate::rule_pack::Pack {
            id: "storage-class".to_string(),
            tool_keywords: vec![],
            applies_to: vec!["*.yaml".to_string()],
            safe_patterns: vec![],
            guarded_patterns: vec![crate::rule_pack::GuardedPattern {
                id: "ssd-storage".to_string(),
                enabled: true,
                check: crate::rule_pack::Check::ContentRegex {
                    regex: "storageClassName: ssd".to_string(),
                },
                tier: crate::rule_pack::Tier::Tier1,
                severity: crate::rule_pack::Severity::Critical,
                explanation: "SSD prohibited".to_string(),
                redirect: crate::rule_pack::Redirect {
                    channel: crate::rule_pack::Channel::Deny,
                    reason_template: "No SSD".to_string(),
                    rewrite_template: None,
                },
                destructive: true,
            }],
        };

        let markdown_pack = crate::rule_pack::Pack {
            id: "markdown".to_string(),
            tool_keywords: vec![],
            applies_to: vec!["*.md".to_string()],
            safe_patterns: vec![],
            guarded_patterns: vec![crate::rule_pack::GuardedPattern {
                id: "todo-in-doc".to_string(),
                enabled: true,
                check: crate::rule_pack::Check::ContentRegex {
                    regex: "TODO:".to_string(),
                },
                tier: crate::rule_pack::Tier::Tier1,
                severity: crate::rule_pack::Severity::Medium,
                explanation: "TODOs should be tracked elsewhere".to_string(),
                redirect: crate::rule_pack::Redirect {
                    channel: crate::rule_pack::Channel::Deny,
                    reason_template: "No TODOs in docs".to_string(),
                    rewrite_template: None,
                },
                destructive: false,
            }],
        };

        engine.load_pack(yaml_pack).unwrap();
        engine.load_pack(markdown_pack).unwrap();

        // YAML file should match storage-class pack
        let yaml_source = ContentSource::Write {
            file_path: "/path/to/config.yaml".to_string(),
            content: "storageClassName: ssd".to_string(),
        };

        let yaml_result = engine.evaluate_content(&yaml_source);
        assert!(matches!(yaml_result, CheckResult::Denied { pack_id, .. } if pack_id == "storage-class"));

        // Markdown file should match markdown pack
        let md_source = ContentSource::Write {
            file_path: "/path/to/doc.md".to_string(),
            content: "# TODO: implement this".to_string(),
        };

        let md_result = engine.evaluate_content(&md_source);
        assert!(matches!(md_result, CheckResult::Denied { pack_id, .. } if pack_id == "markdown"));
    }

    #[test]
    fn test_evaluate_content_multiple_packs_match_same_file() {
        let mut engine = default_engine();

        let storage_class_pack = crate::rule_pack::Pack {
            id: "storage-class".to_string(),
            tool_keywords: vec![],
            applies_to: vec!["*.yaml".to_string()],
            safe_patterns: vec![],
            guarded_patterns: vec![crate::rule_pack::GuardedPattern {
                id: "ssd-storage".to_string(),
                enabled: true,
                check: crate::rule_pack::Check::ContentRegex {
                    regex: "storageClassName: ssd".to_string(),
                },
                tier: crate::rule_pack::Tier::Tier1,
                severity: crate::rule_pack::Severity::Critical,
                explanation: "SSD prohibited".to_string(),
                redirect: crate::rule_pack::Redirect {
                    channel: crate::rule_pack::Channel::Deny,
                    reason_template: "No SSD".to_string(),
                    rewrite_template: None,
                },
                destructive: true,
            }],
        };

        let image_tag_pack = crate::rule_pack::Pack {
            id: "image-tag".to_string(),
            tool_keywords: vec![],
            applies_to: vec!["*.yaml".to_string()],
            safe_patterns: vec![],
            guarded_patterns: vec![crate::rule_pack::GuardedPattern {
                id: "latest-tag".to_string(),
                enabled: true,
                check: crate::rule_pack::Check::ContentRegex {
                    regex: "image: .*:latest".to_string(),
                },
                tier: crate::rule_pack::Tier::Tier1,
                severity: crate::rule_pack::Severity::Critical,
                explanation: "Latest tag prohibited".to_string(),
                redirect: crate::rule_pack::Redirect {
                    channel: crate::rule_pack::Channel::Deny,
                    reason_template: "No :latest".to_string(),
                    rewrite_template: None,
                },
                destructive: true,
            }],
        };

        engine.load_pack(storage_class_pack).unwrap();
        engine.load_pack(image_tag_pack).unwrap();

        // Content that triggers both packs - should return the first deny
        let source = ContentSource::Write {
            file_path: "/path/to/deployment.yaml".to_string(),
            content: "storageClassName: ssd\nimage: nginx:latest".to_string(),
        };

        let result = engine.evaluate_content(&source);
        assert!(matches!(result, CheckResult::Denied { .. }));
    }

    #[test]
    fn test_evaluate_content_safe_pattern_skips_guarded_patterns() {
        let mut engine = default_engine();

        let pack = crate::rule_pack::Pack {
            id: "storage-class".to_string(),
            tool_keywords: vec![],
            applies_to: vec!["*.yaml".to_string()],
            safe_patterns: vec![crate::rule_pack::Pattern {
                id: "sata-is-safe".to_string(),
                check: crate::rule_pack::Check::ContentRegex {
                    regex: "storageClassName: sata".to_string(),
                },
            }],
            guarded_patterns: vec![crate::rule_pack::GuardedPattern {
                id: "any-storage-class".to_string(),
                enabled: true,
                check: crate::rule_pack::Check::ContentRegex {
                    regex: "storageClassName:".to_string(),
                },
                tier: crate::rule_pack::Tier::Tier1,
                severity: crate::rule_pack::Severity::Critical,
                explanation: "Should deny all storage classes".to_string(),
                redirect: crate::rule_pack::Redirect {
                    channel: crate::rule_pack::Channel::Deny,
                    reason_template: "No storage classes allowed".to_string(),
                    rewrite_template: None,
                },
                destructive: true,
            }],
        };

        engine.load_pack(pack).unwrap();

        // Content matches safe pattern - should skip guarded patterns
        let source = ContentSource::Write {
            file_path: "/path/to/config.yaml".to_string(),
            content: "storageClassName: sata".to_string(),
        };

        let result = engine.evaluate_content(&source);
        assert_eq!(result, CheckResult::Allowed);
    }
}
