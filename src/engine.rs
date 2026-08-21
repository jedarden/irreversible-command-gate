//! Engine: command-mode and content-mode input acquisition and pack dispatch
//!
//! This module handles:
//! - Input acquisition from PreToolUse JSON (hook mode) or argv (wrapper mode)
//! - Command-mode: shell line segmentation (splits on ;/&&/||/, skips sudo/env-assignment/wrapper prefixes)
//!   Basename-matching tokens against tool_keywords for pack dispatch
//! - Pack dispatch: routes tokens to matching packs and evaluates guarded_patterns
//! - Content-mode: file path + content reading from Write/Edit or normalized
//!   Codex apply_patch PreToolUse JSON (storage-class, image-tag packs)

use crate::fail_closed::{PolicyStore, LEGACY_FAIL_CLOSED_ENV};
use crate::rule_pack::Pack;
use crate::telemetry::{CheckResultToVerdict, TelemetryStore, Verdict};
use crate::value_derivation::render_reason;
use anyhow::{Context, Result};
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::{Component, Path, PathBuf};

/// PreToolUse hook JSON structure for Claude Code/Codex.
///
/// Claude Code and Codex use snake_case on the hook wire, while the original
/// ICG parser accepted the camelCase spelling used by its early fixtures.
/// Keep both spellings as aliases so one adapter can serve either harness and
/// old callers remain source-compatible.
///
/// This represents the input format from the PreToolUse hook system,
/// which provides tool invocation context for validation before execution.
#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PreToolUseInput {
    /// The name of the tool being invoked (e.g., "Bash", "Write", "Edit", "apply_patch")
    #[serde(rename = "toolName", alias = "tool_name")]
    pub tool_name: String,

    /// The input parameters for the tool
    #[serde(rename = "toolInput", alias = "tool_input")]
    pub tool_input: ToolInput,

    /// Optional unique identifier for this tool invocation
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "id", alias = "toolUseId", alias = "tool_use_id")]
    pub id: Option<String>,

    /// Optional timestamp of the tool invocation
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timestamp: Option<String>,

    /// Optional session identifier
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(alias = "session_id")]
    pub session_id: Option<String>,
}

/// Tool input parameters vary by tool type
#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolInput {
    /// Bash command string (for Bash tool)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,

    /// File path for Write/Edit operations
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "filePath", alias = "file_path")]
    pub file_path: Option<String>,

    /// Full file content (for Write tool)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,

    /// Old string being replaced (for Edit tool)
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "oldString", alias = "old_string")]
    pub old_string: Option<String>,

    /// New string to replace with (for Edit tool)
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "newString", alias = "new_string")]
    pub new_string: Option<String>,

    /// Encoding for file operations (e.g., "base64")
    #[serde(skip_serializing_if = "Option::is_none")]
    pub encoding: Option<String>,

    /// MIME type for file operations
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "mimeType", alias = "mime_type")]
    pub mime_type: Option<String>,
}

/// Validation error for PreToolUse input
#[derive(Debug, thiserror::Error)]
pub enum PreToolUseError {
    #[error("Invalid JSON format: {0}")]
    InvalidJson(String),

    #[error("Missing required field: {0}")]
    MissingField(String),

    #[error("Invalid tool name: {0}")]
    InvalidToolName(String),

    #[error("Invalid tool input for {tool}: {reason}")]
    InvalidInput { tool: String, reason: String },

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
}

/// Result type for PreToolUse parsing
pub type PreToolUseResult<T> = std::result::Result<T, PreToolUseError>;

/// Command input source
#[derive(Debug, Clone, PartialEq)]
pub enum CommandSource {
    /// From PreToolUse JSON stdin (hook mode)
    Hook(String),
    /// From wrapped-command argv (PATH-wrapper mode)
    Argv(Vec<String>),
}

/// Content input source (for Write/Edit or normalized apply_patch operations)
#[derive(Debug, Clone, PartialEq)]
pub enum ContentSource {
    /// Write operation: full file content
    Write { file_path: String, content: String },
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
            ContentSource::Edit {
                old_content,
                new_content,
                ..
            } => {
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

/// Normalize the file edits carried by a Codex `apply_patch` command into the
/// content-mode shape used by the rest of the engine.
///
/// Codex sends the patch text in `tool_input.command`, rather than sending a
/// Claude Code-style `filePath` and `content`. Content rules only need the
/// target path and the text that the patch adds (plus context lines), so a
/// patch can be checked without applying it or mutating the workspace. A
/// single patch may contain multiple file headers; callers should evaluate
/// every returned source.
pub fn normalize_apply_patch(command: &str) -> PreToolUseResult<Vec<ContentSource>> {
    let mut saw_begin = false;
    let mut saw_end = false;
    let mut files = Vec::new();
    let mut current: Option<(String, String)> = None;

    for raw_line in command.lines() {
        let line = raw_line.strip_suffix('\r').unwrap_or(raw_line);
        let trimmed = line.trim();

        if trimmed == "*** Begin Patch" {
            if saw_begin {
                return Err(PreToolUseError::InvalidInput {
                    tool: "apply_patch".to_string(),
                    reason: "patch contains more than one begin marker".to_string(),
                });
            }
            saw_begin = true;
            continue;
        }

        if trimmed == "*** End Patch" {
            flush_patch_file(&mut current, &mut files);
            saw_end = true;
            break;
        }

        if !saw_begin {
            // Some wrappers include a shell preamble before the canonical
            // marker. Ignore it, but still require an actual patch below.
            continue;
        }

        if let Some(path) = line.strip_prefix("*** Update File: ") {
            flush_patch_file(&mut current, &mut files);
            current = Some((path.trim().to_string(), String::new()));
            continue;
        }

        if let Some(path) = line.strip_prefix("*** Add File: ") {
            flush_patch_file(&mut current, &mut files);
            current = Some((path.trim().to_string(), String::new()));
            continue;
        }

        if let Some(path) = line.strip_prefix("*** Delete File: ") {
            flush_patch_file(&mut current, &mut files);
            current = Some((path.trim().to_string(), String::new()));
            continue;
        }

        // A move header follows an update header. The destination is the
        // file that Codex will write, so use it for applies_to dispatch.
        if let Some(path) = line.strip_prefix("*** Move to: ") {
            if let Some((current_path, _)) = current.as_mut() {
                *current_path = path.trim().to_string();
            }
            continue;
        }

        // Hunk and patch metadata are not file content. In particular, do
        // not treat a git-style +++ header as an added line.
        if trimmed.starts_with("@@")
            || line.starts_with("--- ")
            || line.starts_with("+++ ")
            || line.starts_with("*** ")
        {
            continue;
        }

        if let Some((_, content)) = current.as_mut() {
            if let Some(added) = line.strip_prefix('+') {
                content.push_str(added);
                content.push('\n');
            } else if let Some(context) = line.strip_prefix(' ') {
                content.push_str(context);
                content.push('\n');
            }
            // Removed lines are deliberately omitted. They are not part of
            // the content that will exist after the patch is applied.
        }
    }

    if !saw_begin || !saw_end {
        return Err(PreToolUseError::InvalidInput {
            tool: "apply_patch".to_string(),
            reason: "patch must contain *** Begin Patch and *** End Patch".to_string(),
        });
    }

    if files.is_empty() {
        return Err(PreToolUseError::InvalidInput {
            tool: "apply_patch".to_string(),
            reason: "patch does not contain a file header".to_string(),
        });
    }

    Ok(files
        .into_iter()
        .map(|(file_path, content)| ContentSource::Write { file_path, content })
        .collect())
}

/// Convenience form for callers that know an `apply_patch` contains exactly
/// one file.
pub fn normalize_single_apply_patch(command: &str) -> PreToolUseResult<ContentSource> {
    let mut sources = normalize_apply_patch(command)?;
    if sources.len() != 1 {
        return Err(PreToolUseError::InvalidInput {
            tool: "apply_patch".to_string(),
            reason: "patch contains more than one file".to_string(),
        });
    }
    Ok(sources.remove(0))
}

fn flush_patch_file(current: &mut Option<(String, String)>, files: &mut Vec<(String, String)>) {
    if let Some(file) = current.take() {
        files.push(file);
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
    value
        .replace('\\', "/")
        .trim_start_matches("./")
        .to_string()
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
            let is_double_star =
                pattern_index + 1 < pattern.len() && pattern[pattern_index + 1] == '*';

            if is_double_star {
                // `**/` may consume zero directories, or any number of path
                // characters before the next slash.
                let after_stars = pattern_index + 2;
                let skip_double_star = if after_stars < pattern.len() && pattern[after_stars] == '/'
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
            && (pattern[pattern_index] == '?' || pattern[pattern_index] == value[value_index])
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

/// Return an absolute, lexically normalized path for a hook-reported target.
///
/// The target may not exist yet (a Write commonly creates a new file), so this
/// intentionally does not use `canonicalize`. Lexical normalization is enough
/// to keep `..` components from changing which repository root is inspected.
fn absolute_normalized_path(path: &Path) -> Option<PathBuf> {
    let mut normalized = if path.is_absolute() {
        PathBuf::new()
    } else {
        std::env::current_dir().ok()?
    };

    for component in path.components() {
        match component {
            Component::Prefix(prefix) => normalized.push(prefix.as_os_str()),
            Component::RootDir => normalized.push(Path::new("/")),
            Component::CurDir => {}
            Component::ParentDir => {
                // Do not allow a relative path to escape the current directory
                // while it is being normalized. Absolute paths are already
                // anchored at the filesystem root.
                let _ = normalized.pop();
            }
            Component::Normal(component) => normalized.push(component),
        }
    }

    Some(normalized)
}

/// Check the beads protection predicate for a Write/Edit target.
///
/// The target must be under the repository root's `.beads/` directory, and
/// that repository root must have `.git` as a directory. A `.git` file
/// identifies a linked worktree and is deliberately allowed because it is not
/// the shared primary checkout.
fn is_shared_beads_target(file_path: &str) -> bool {
    let Some(path) = absolute_normalized_path(Path::new(file_path)) else {
        return false;
    };

    let mut ancestor = Some(path.as_path());
    while let Some(candidate) = ancestor {
        let git_path = candidate.join(".git");
        if git_path.is_dir() || git_path.is_file() {
            return git_path.is_dir() && path.starts_with(candidate.join(".beads"));
        }
        ancestor = candidate.parent();
    }

    false
}

/// Input source from PreToolUse hook
#[derive(Debug, Clone, PartialEq)]
pub enum InputSource {
    /// Bash command (for command-mode packs: vault, git, secrets, misc, tmux)
    Command(CommandSource),
    /// File content (for content-mode packs: storage-class, image-tag)
    Content(ContentSource),
    /// Multiple file contents normalized from one Codex `apply_patch` call.
    ContentBatch(Vec<ContentSource>),
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

/// Split shell input into command-word vectors without invoking a shell.
///
/// This is intentionally a small lexer rather than a shell evaluator. The
/// guard needs to identify command boundaries and argv words, but must not
/// execute expansions or run anything supplied by the caller. Quotes and
/// backslash escapes are removed so rule expressions see the words that the
/// shell would pass to the executable.
fn lex_shell_commands(input: &str) -> Vec<Vec<String>> {
    let mut commands = Vec::new();
    let mut command = Vec::new();
    let mut word = String::new();
    let mut word_started = false;
    let mut quote = None;
    let mut escaped = false;

    let finish_word = |command: &mut Vec<String>, word: &mut String, started: &mut bool| {
        if *started {
            command.push(std::mem::take(word));
            *started = false;
        }
    };

    let finish_command =
        |commands: &mut Vec<Vec<String>>, command: &mut Vec<String>, word: &mut String, started: &mut bool| {
            finish_word(command, word, started);
            if !command.is_empty() {
                commands.push(std::mem::take(command));
            }
        };

    let mut chars = input.chars().peekable();
    while let Some(character) = chars.next() {
        if escaped {
            // A backslash-newline is a shell line continuation. For all other
            // characters, retain the escaped character as part of this word.
            if character != '\n' {
                word.push(character);
                word_started = true;
            }
            escaped = false;
            continue;
        }

        if let Some(active_quote) = quote {
            if character == active_quote {
                quote = None;
                word_started = true;
            } else if active_quote == '"' && character == '\\' {
                // Inside double quotes only shell-special escapes lose their
                // backslash. Keep other backslashes literal.
                match chars.peek().copied() {
                    Some(next @ ('"' | '\\' | '$' | '`' | '\n')) => {
                        chars.next();
                        if next != '\n' {
                            word.push(next);
                            word_started = true;
                        }
                    }
                    _ => {
                        word.push(character);
                        word_started = true;
                    }
                }
            } else {
                word.push(character);
                word_started = true;
            }
            continue;
        }

        match character {
            '\'' | '"' => {
                quote = Some(character);
                word_started = true;
            }
            '\\' => escaped = true,
            '\n' => {
                finish_command(&mut commands, &mut command, &mut word, &mut word_started);
            }
            character if character.is_whitespace() => {
                finish_word(&mut command, &mut word, &mut word_started);
            }
            ';' | '&' | '|' => {
                // Treat both short and compound shell operators as command
                // boundaries. The second character of &&/|| is just another
                // delimiter and therefore yields no empty command.
                finish_command(
                    &mut commands,
                    &mut command,
                    &mut word,
                    &mut word_started,
                );
            }
            _ => {
                word.push(character);
                word_started = true;
            }
        }
    }

    // A trailing backslash is malformed shell, but retaining it gives the
    // caller a conservative best-effort token instead of silently dropping a
    // command word.
    if escaped {
        word.push('\\');
        word_started = true;
    }
    finish_command(
        &mut commands,
        &mut command,
        &mut word,
        &mut word_started,
    );

    commands
}

fn executable_basename(token: &str) -> &str {
    token.rsplit('/').next().unwrap_or(token)
}

fn option_name(token: &str) -> &str {
    token.split_once('=').map_or(token, |(name, _)| name)
}

/// Render a normalized command token for regex matching without losing the
/// boundary between an argument containing whitespace and a following
/// argument. Shell lexing has already removed the original quotes, so add a
/// minimal unambiguous representation back for command-regex consumers.
fn render_command_word(word: &str) -> String {
    if word.chars().any(char::is_whitespace) || word.is_empty() {
        format!("'{}'", word.replace('\'', "'\\''"))
    } else {
        word.to_string()
    }
}

fn render_command(token: &CommandToken) -> String {
    std::iter::once(token.executable.as_str())
        .chain(token.args.iter().map(String::as_str))
        .map(render_command_word)
        .collect::<Vec<_>>()
        .join(" ")
}

fn skip_options(tokens: &[String], mut index: usize, options_with_values: &[&str]) -> usize {
    while index < tokens.len() {
        let token = tokens[index].as_str();
        if token == "--" {
            return index + 1;
        }
        if !token.starts_with('-') || token == "-" {
            break;
        }

        let consumes_next = options_with_values.contains(&option_name(token))
            && !token.contains('=');
        index += 1;
        if consumes_next && index < tokens.len() {
            index += 1;
        }
    }
    index
}

fn strip_command_prefixes(
    tokens: &[String],
    env_assign_pattern: &Regex,
    ignored_prefixes: &[String],
) -> usize {
    let mut index = 0;

    loop {
        while index < tokens.len() && env_assign_pattern.is_match(&tokens[index]) {
            index += 1;
        }
        if index >= tokens.len() {
            break;
        }

        let prefix = executable_basename(&tokens[index]);
        match prefix {
            "sudo" => {
                index += 1;
                index = skip_options(
                    tokens,
                    index,
                    &[
                        "-u", "--user", "-g", "--group", "-C", "--chdir", "-R", "--chroot",
                        "-r", "--role", "-t", "--type", "-p", "--prompt", "-T", "--command-timeout",
                    ],
                );
            }
            "env" => {
                index += 1;
                index = skip_options(
                    tokens,
                    index,
                    &["-u", "--unset", "-C", "--chdir", "-S", "--split-string"],
                );
            }
            _ if ignored_prefixes
                .iter()
                .any(|ignored| ignored == prefix) => {
                index += 1;
                index = skip_options(tokens, index, &["-a", "--argv0", "--format", "-f"]);
            }
            _ => break,
        }
    }

    index
}

fn command_token_from_words(
    words: Vec<String>,
    env_assign_pattern: &Regex,
    ignored_prefixes: &[String],
) -> Option<CommandToken> {
    let executable_index = strip_command_prefixes(&words, env_assign_pattern, ignored_prefixes);
    let executable = words.get(executable_index)?;

    Some(CommandToken {
        executable: executable_basename(executable).to_string(),
        args: words[executable_index + 1..].to_vec(),
    })
}

/// Engine: command-mode input acquisition, segmentation, and pack dispatch
pub struct Engine {
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
    /// Once an in-process failure occurs, subsequent checks use the selected
    /// availability-failure posture for the lifetime of this invocation.
    fail_open: bool,
    /// Fail-closed mode: when enabled, guard-availability failures result in
    /// deny instead of allow.  The normal source is the administrator-owned
    /// graduated policy state; the legacy environment variable is retained as
    /// a stricter local/test override.
    fail_closed: bool,
    /// Optional telemetry store for recording evaluation results
    telemetry_store: Option<std::sync::Arc<std::sync::Mutex<crate::telemetry::TelemetryStore>>>,
    /// Optional session ID for correlation across evaluations
    session_id: Option<String>,
    /// Optional release reference for this evaluation session
    release_ref: Option<String>,
    /// Optional state store for Tier 2 cross-invocation state tracking
    state_store: Option<std::sync::Arc<crate::state_store::StateStore>>,
}

impl Default for Engine {
    fn default() -> Self {
        Self::new()
    }
}

impl Engine {
    /// Create a new Engine with default segmentation patterns
    pub fn new() -> Self {
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

        // Read the durable policy once at process start.  A missing or
        // unreadable policy is deliberately fail-open so a configuration
        // problem cannot wedge the fleet before an operator can repair it.
        let policy_store = PolicyStore::from_env();
        let durable_fail_closed = match policy_store.load() {
            Ok(state) => state.is_fail_closed(),
            Err(error) => {
                eprintln!(
                    "Engine: fail-open: unable to read fail-closed policy {} ({error:#})",
                    policy_store.path().display()
                );
                false
            }
        };
        let legacy_fail_closed = std::env::var(LEGACY_FAIL_CLOSED_ENV)
            .map(|val| val == "1" || val.eq_ignore_ascii_case("true"))
            .unwrap_or(false);
        let fail_closed = durable_fail_closed || legacy_fail_closed;

        Self {
            env_assign_pattern,
            ignored_prefixes,
            packs: HashMap::new(),
            keyword_index: HashMap::new(),
            exempted_rule_ids: HashSet::new(),
            fail_open: false,
            fail_closed,
            telemetry_store: None,
            session_id: None,
            release_ref: None,
            state_store: None,
        }
    }

    /// Set the telemetry store for recording evaluation results
    pub fn with_telemetry_store(mut self, store: std::sync::Arc<std::sync::Mutex<crate::telemetry::TelemetryStore>>) -> Self {
        self.telemetry_store = Some(store);
        self
    }

    /// Set the session ID for correlation across evaluations
    pub fn with_session_id(mut self, session_id: impl Into<String>) -> Self {
        self.session_id = Some(session_id.into());
        self
    }

    /// Set the release reference for this evaluation session
    pub fn with_release_ref(mut self, release_ref: impl Into<String>) -> Self {
        self.release_ref = Some(release_ref.into());
        self
    }

    /// Set the state store for Tier 2 cross-invocation state tracking
    pub fn with_state_store(mut self, store: std::sync::Arc<crate::state_store::StateStore>) -> Self {
        self.state_store = Some(store);
        self
    }

    /// Override the loaded policy for an embedding application or a focused
    /// test.  This is intentionally an in-memory setting; production policy
    /// changes must go through the durable policy store.
    pub fn with_fail_closed(mut self, enabled: bool) -> Self {
        self.fail_closed = enabled;
        self
    }

    /// Return the availability-failure posture used by this process.
    pub fn fail_closed(&self) -> bool {
        self.fail_closed
    }

    /// Whether this engine observed an availability failure while loading or
    /// evaluating the current invocation.  The hook boundary uses this signal
    /// to record caught guard faults in the durable health store; a normal rule
    /// denial does not set it.
    pub fn has_guard_failure(&self) -> bool {
        self.fail_open
    }

    /// Get the current session ID
    pub fn session_id(&self) -> Option<&str> {
        self.session_id.as_deref()
    }

    /// Get the current release reference
    pub fn release_ref(&self) -> Option<&str> {
        self.release_ref.as_deref()
    }

    fn mark_fail_open(&mut self, reason: &str) {
        self.fail_open = true;
        report_failure(self.fail_closed, reason);
    }

    fn should_fail_open(&self) -> bool {
        self.fail_open
    }

    fn mark_fail_closed(&mut self, reason: &str) {
        self.fail_open = true;
        report_failure(self.fail_closed, reason);
    }

    fn should_fail_closed(&self) -> bool {
        self.fail_open && self.fail_closed
    }

    /// Read input from PreToolUse JSON on stdin (hook mode)
    ///
    /// Returns the appropriate InputSource:
    /// - CommandSource for Bash tool calls
    /// - ContentSource for Write/Edit tool calls
    /// - None for unrecognized tools
    pub fn read_from_stdin(&self) -> Result<Option<InputSource>> {
        let Some((input, _)) = self.read_pre_tool_use_payload_from_stdin()? else {
            return Ok(None);
        };

        let result = catch_unwind(AssertUnwindSafe(|| Self::input_source_from_pre_tool_use(input)));
        match result {
            Ok(Ok(input)) => Ok(input),
            Ok(Err(error)) => {
                report_failure(self.fail_closed, &format!("stdin input failure: {error}"));
                Ok(None)
            }
            Err(_) => {
                report_failure(self.fail_closed, "stdin input conversion panicked");
                Ok(None)
            }
        }
    }

    /// Read the validated PreToolUse input together with its original JSON
    /// tool-input object. The typed input intentionally models only fields the
    /// engine evaluates, while the raw object lets the hook preserve any
    /// harness-specific fields in an `updatedInput` response.
    pub fn read_pre_tool_use_payload_from_stdin(
        &self,
    ) -> Result<Option<(PreToolUseInput, serde_json::Value)>> {
        let result = catch_unwind(AssertUnwindSafe(|| self.read_pre_tool_use_payload_inner()));
        match result {
            Ok(Ok(input)) => Ok(Some(input)),
            Ok(Err(error)) => {
                report_failure(self.fail_closed, &format!("stdin input failure: {error}"));
                Ok(None)
            }
            Err(_) => {
                report_failure(self.fail_closed, "stdin input panicked");
                Ok(None)
            }
        }
    }

    fn read_pre_tool_use_payload_inner(
        &self,
    ) -> Result<(PreToolUseInput, serde_json::Value)> {
        use std::io::{self, Read};

        let mut input = String::new();
        io::stdin()
            .read_to_string(&mut input)
            .context("Failed to read stdin")?;

        // Parse and validate the JSON input
        let raw: serde_json::Value = serde_json::from_str(&input)
            .context("Failed to parse hook input JSON")?;
        let parsed = Self::parse_and_validate_pre_tool_use(&input)?;
        let tool_input = raw
            .get("toolInput")
            .or_else(|| raw.get("tool_input"))
            .cloned()
            .unwrap_or(serde_json::Value::Null);

        Ok((parsed, tool_input))
    }

    /// Read command from PreToolUse JSON on stdin (legacy method for backward compatibility)
    ///
    /// Returns None if the input isn't a Bash command (e.g., Write/Edit)
    #[deprecated(note = "Use read_from_stdin() which returns InputSource instead")]
    pub fn read_command_from_stdin(&self) -> Result<Option<CommandSource>> {
        match self.read_from_stdin()? {
            Some(InputSource::Command(cmd)) => Ok(Some(cmd)),
            Some(InputSource::Content(_)) | Some(InputSource::ContentBatch(_)) => Ok(None),
            None => Ok(None),
        }
    }

    /// Parse and validate PreToolUse JSON input
    ///
    /// This function performs comprehensive validation of the JSON input
    /// and returns a strongly-typed PreToolUseInput struct.
    pub fn parse_and_validate_pre_tool_use(input: &str) -> PreToolUseResult<PreToolUseInput> {
        // Step 1: Parse JSON
        let parsed: PreToolUseInput = serde_json::from_str(input)
            .map_err(|e| PreToolUseError::InvalidJson(format!("JSON parse error: {}", e)))?;

        // Step 2: Validate tool name
        let tool_name = &parsed.tool_name;
        if tool_name.is_empty() {
            return Err(PreToolUseError::MissingField("toolName".to_string()));
        }

        // Validate known tool names
        match tool_name.as_str() {
            "Bash" | "Write" | "Edit" | "apply_patch" => {
                // Known tools - continue validation
            }
            _ => {
                // Unknown tool - this is not necessarily an error, but we should log it
                // For now, we'll allow it through (fail-open for unknown tools)
            }
        }

        // Step 3: Validate tool input based on tool type
        Self::validate_tool_input(tool_name, &parsed.tool_input)?;

        Ok(parsed)
    }

    /// Validate tool input based on tool type
    fn validate_tool_input(tool_name: &str, input: &ToolInput) -> PreToolUseResult<()> {
        match tool_name {
            "Bash" => {
                if input.command.is_none() {
                    return Err(PreToolUseError::InvalidInput {
                        tool: tool_name.to_string(),
                        reason: "missing 'command' field".to_string(),
                    });
                }
                // Additional validation for command
                if let Some(ref cmd) = input.command {
                    if cmd.trim().is_empty() {
                        return Err(PreToolUseError::InvalidInput {
                            tool: tool_name.to_string(),
                            reason: "command cannot be empty".to_string(),
                        });
                    }
                }
            }
            "Write" => {
                if input.file_path.is_none() {
                    return Err(PreToolUseError::InvalidInput {
                        tool: tool_name.to_string(),
                        reason: "missing 'filePath' field".to_string(),
                    });
                }
                if input.content.is_none() {
                    return Err(PreToolUseError::InvalidInput {
                        tool: tool_name.to_string(),
                        reason: "missing 'content' field".to_string(),
                    });
                }
            }
            "Edit" => {
                if input.file_path.is_none() {
                    return Err(PreToolUseError::InvalidInput {
                        tool: tool_name.to_string(),
                        reason: "missing 'filePath' field".to_string(),
                    });
                }
                if input.old_string.is_none() || input.new_string.is_none() {
                    return Err(PreToolUseError::InvalidInput {
                        tool: tool_name.to_string(),
                        reason: "Edit requires both 'oldString' and 'newString' fields".to_string(),
                    });
                }
            }
            "apply_patch" => {
                if input.command.is_none() {
                    return Err(PreToolUseError::InvalidInput {
                        tool: tool_name.to_string(),
                        reason: "missing 'command' field".to_string(),
                    });
                }
                if let Some(command) = input.command.as_deref() {
                    if command.trim().is_empty() {
                        return Err(PreToolUseError::InvalidInput {
                            tool: tool_name.to_string(),
                            reason: "command cannot be empty".to_string(),
                        });
                    }
                }
            }
            _ => {
                // Unknown tool - no specific validation
            }
        }

        Ok(())
    }

    /// Convert PreToolUseInput to InputSource
    ///
    /// Maps the validated PreToolUse input to the appropriate InputSource
    /// for engine evaluation.
    pub fn input_source_from_pre_tool_use(
        input: PreToolUseInput,
    ) -> PreToolUseResult<Option<InputSource>> {
        match input.tool_name.as_str() {
            "Bash" => {
                let command =
                    input
                        .tool_input
                        .command
                        .ok_or_else(|| PreToolUseError::InvalidInput {
                            tool: "Bash".to_string(),
                            reason: "missing command".to_string(),
                        })?;

                Ok(Some(InputSource::Command(CommandSource::Hook(command))))
            }
            "Write" => {
                let file_path =
                    input
                        .tool_input
                        .file_path
                        .ok_or_else(|| PreToolUseError::InvalidInput {
                            tool: "Write".to_string(),
                            reason: "missing file_path".to_string(),
                        })?;

                let content =
                    input
                        .tool_input
                        .content
                        .ok_or_else(|| PreToolUseError::InvalidInput {
                            tool: "Write".to_string(),
                            reason: "missing content".to_string(),
                        })?;

                Ok(Some(InputSource::Content(ContentSource::Write {
                    file_path,
                    content,
                })))
            }
            "Edit" => {
                let file_path =
                    input
                        .tool_input
                        .file_path
                        .ok_or_else(|| PreToolUseError::InvalidInput {
                            tool: "Edit".to_string(),
                            reason: "missing file_path".to_string(),
                        })?;

                let old_content =
                    input
                        .tool_input
                        .old_string
                        .ok_or_else(|| PreToolUseError::InvalidInput {
                            tool: "Edit".to_string(),
                            reason: "missing old_string".to_string(),
                        })?;

                let new_content =
                    input
                        .tool_input
                        .new_string
                        .ok_or_else(|| PreToolUseError::InvalidInput {
                            tool: "Edit".to_string(),
                            reason: "missing new_string".to_string(),
                        })?;

                Ok(Some(InputSource::Content(ContentSource::Edit {
                    file_path,
                    old_content,
                    new_content,
                })))
            }
            "apply_patch" => {
                let command =
                    input
                        .tool_input
                        .command
                        .ok_or_else(|| PreToolUseError::InvalidInput {
                            tool: "apply_patch".to_string(),
                            reason: "missing command".to_string(),
                        })?;
                let sources = normalize_apply_patch(&command)?;
                if sources.len() == 1 {
                    Ok(Some(InputSource::Content(
                        sources.into_iter().next().expect("one source"),
                    )))
                } else {
                    Ok(Some(InputSource::ContentBatch(sources)))
                }
            }
            _ => {
                // Unknown tool - return None (fail-open)
                Ok(None)
            }
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
        let Some(env_assign_pattern) = &self.env_assign_pattern
        else {
            return vec![];
        };

        match source {
            CommandSource::Hook(command) => lex_shell_commands(command)
                .into_iter()
                .filter_map(|words| {
                    command_token_from_words(words, env_assign_pattern, &self.ignored_prefixes)
                })
                .collect(),
            // argv has already gone through the operating system's argument
            // parsing. Do not join it and lex again: an argument such as
            // `--message=a;b` is data, not a second shell command.
            CommandSource::Argv(argv) => command_token_from_words(
                argv.clone(),
                env_assign_pattern,
                &self.ignored_prefixes,
            )
            .into_iter()
            .collect(),
        }
    }

    /// Get all unique executable basenames from a command source
    ///
    /// This is used for pack dispatch: each pack registers tool_keywords
    /// (e.g., ["vault", "bao"]), and we match tokens against those keywords
    pub fn get_executables(&self, source: &CommandSource) -> Vec<String> {
        let mut seen = HashSet::new();
        self.segment_command(source)
            .into_iter()
            .map(|token| token.executable)
            .filter(|executable| seen.insert(executable.clone()))
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

        // Replacing a pack must also replace its dispatch entries. Otherwise
        // a reload leaves stale IDs in the index and can evaluate a pack that
        // is no longer installed.
        if let Some(previous) = self.packs.get(&pack_id) {
            for keyword in pack_dispatch_keywords(previous) {
                let keyword = executable_basename(&keyword).to_string();
                if let Some(pack_ids) = self.keyword_index.get_mut(&keyword) {
                    pack_ids.retain(|id| id != &pack_id);
                }
            }
        }

        // Index tool_keywords and any data-driven deprecated executable names
        // for fast dispatch. The latter keeps a manifest-only CLI cutover from
        // requiring executable logic changes.
        for keyword in pack_dispatch_keywords(&pack) {
            let keyword = executable_basename(&keyword).to_string();
            self.keyword_index
                .entry(keyword)
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
        self.exempted_rule_ids = manifest.exempted_rule_ids.iter().cloned().collect();
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
        let manifest =
            crate::overrides::load_verified_override(path, repository, trusted_ref, &packs)?;
        self.exempted_rule_ids = manifest.exempted_rule_ids.into_iter().collect();
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
    /// 2. For each matching pack, check safe_patterns first (skip the rest if any match)
    /// 3. Then check guarded_patterns (return first match)
    /// 4. Return the most severe result (deny > rewrite > warning > allowed)
    pub fn evaluate_token(&self, token: &CommandToken) -> CheckResult {
        match catch_unwind(AssertUnwindSafe(|| self.evaluate_token_inner(token))) {
            Ok(result) => result,
            Err(_) => {
                self.guard_failure_result("token evaluation panicked")
            }
        }
    }

    fn evaluate_token_inner(&self, token: &CommandToken) -> CheckResult {
        if self.should_fail_open() {
            return if self.should_fail_closed() {
                CheckResult::Denied {
                    reason: "Guard crash in fail-closed mode - rejecting all commands".to_string(),
                    pack_id: "fail-closed".to_string(),
                    pattern_id: "guard-crash".to_string(),
                }
            } else {
                CheckResult::Allowed
            };
        }

        let executable = &token.executable;

        // Reconstruct the full command string for regex matching
        let full_command = render_command(token);

        // Find packs that match this executable via tool_keywords
        let mut matching_pack_ids = Vec::new();
        if let Some(ids) = self.keyword_index.get(executable) {
            for pack_id in ids {
                if !matching_pack_ids.contains(pack_id) {
                    matching_pack_ids.push(pack_id.clone());
                }
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
                if !guarded_pattern.enabled || self.exempted_rule_ids.contains(&guarded_pattern.id)
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
                    let pattern_result =
                        self.guarded_pattern_to_result(guarded_pattern, pack_id, &full_command);

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

    /// Evaluate unconditional packs against the entire raw command string
    ///
    /// This handles packs with empty tool_keywords (like secrets) that scan the
    /// whole command regardless of which executable is invoked. This catches
    /// cases like `echo "ghp_..." >> file` where the dangerous pattern has no
    /// guarded executable to basename-match.
    ///
    /// Returns early on the first deny, otherwise returns the most severe result.
    fn evaluate_unconditional_packs(&self, full_command: &str) -> CheckResult {
        let mut result = CheckResult::Allowed;

        for (pack_id, pack) in &self.packs {
            // Skip packs that have tool_keywords (they use basename dispatch)
            if !pack.tool_keywords.is_empty() {
                continue;
            }

            // Check safe_patterns first - if any match, skip this pack entirely
            let safe_match = pack.safe_patterns.iter().find_map(|pattern| {
                match self.pattern_matches_command(pattern, full_command) {
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
                if !guarded_pattern.enabled || self.exempted_rule_ids.contains(&guarded_pattern.id)
                {
                    continue;
                }
                // Create a temporary Pattern wrapper for the guarded pattern's check
                let pattern_wrapper = crate::rule_pack::Pattern {
                    id: guarded_pattern.id.clone(),
                    check: guarded_pattern.check.clone(),
                };

                let matches = match self.pattern_matches_command(&pattern_wrapper, full_command) {
                    Ok(matches) => matches,
                    Err(()) => return CheckResult::Allowed,
                };

                if matches {
                    // Pattern matched - convert to CheckResult
                    let pattern_result =
                        self.guarded_pattern_to_result(guarded_pattern, pack_id, full_command);

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
    /// - Records telemetry result if telemetry store is configured
    pub fn evaluate_command(&self, source: &CommandSource) -> CheckResult {
        let result = match catch_unwind(AssertUnwindSafe(|| self.evaluate_command_inner(source))) {
            Ok(result) => result,
            Err(_) => {
                self.guard_failure_result("command evaluation panicked")
            }
        };

        // Record telemetry result if configured
        self.record_telemetry_result(&result);

        result
    }

    /// Render a deterministic explanation of command dispatch and pattern
    /// evaluation for the human-facing `check --debug` command.
    ///
    /// This deliberately reuses the same tokenization and pattern matcher as
    /// normal evaluation. It is diagnostic output only: callers still use
    /// [`evaluate_command`] for the authoritative result.
    pub fn debug_command_trace(&self, source: &CommandSource, result: &CheckResult) -> String {
        use std::collections::BTreeSet;
        use std::fmt::Write as _;

        let full_command = match source {
            CommandSource::Hook(command) => command.clone(),
            CommandSource::Argv(argv) => argv.join(" "),
        };
        let mut dispatched = BTreeSet::new();

        for (pack_id, pack) in &self.packs {
            if pack.tool_keywords.is_empty() {
                dispatched.insert((pack_id.clone(), full_command.clone()));
            }
        }

        for token in self.segment_command(source) {
            let command = render_command(&token);
            if let Some(pack_ids) = self.keyword_index.get(&token.executable) {
                for pack_id in pack_ids {
                    dispatched.insert((pack_id.clone(), command.clone()));
                }
            }
        }

        let mut trace = String::from("DEBUG: Pattern matching trace\n");
        let _ = writeln!(trace, "Command: {full_command}");

        if dispatched.is_empty() {
            trace.push_str("Pack dispatched: none\n");
        }

        for (pack_id, command) in dispatched {
            let Some(pack) = self.packs.get(&pack_id) else {
                continue;
            };

            let _ = writeln!(trace, "Pack dispatched: {pack_id} (input: {command})");
            trace.push_str("Safe patterns checked:\n");
            if pack.safe_patterns.is_empty() {
                trace.push_str("  (none)\n");
            }
            for pattern in &pack.safe_patterns {
                let matched = self
                    .pattern_matches_command(pattern, &command)
                    .unwrap_or(false);
                let status = if matched { "MATCH" } else { "NO MATCH" };
                let _ = writeln!(
                    trace,
                    "  {}: {status} (check: {})",
                    pattern.id,
                    debug_check_description(&pattern.check)
                );
            }

            trace.push_str("Guarded patterns checked:\n");
            if pack.guarded_patterns.is_empty() {
                trace.push_str("  (none)\n");
            }
            for pattern in &pack.guarded_patterns {
                if !pattern.enabled {
                    let _ = writeln!(trace, "  {}: SKIPPED (disabled)", pattern.id);
                    continue;
                }
                if self.exempted_rule_ids.contains(&pattern.id) {
                    let _ = writeln!(trace, "  {}: SKIPPED (verified override)", pattern.id);
                    continue;
                }

                let wrapper = crate::rule_pack::Pattern {
                    id: pattern.id.clone(),
                    check: pattern.check.clone(),
                };
                let matched = self
                    .pattern_matches_command(&wrapper, &command)
                    .unwrap_or(false);
                let status = if matched { "MATCH" } else { "NO MATCH" };
                let _ = writeln!(
                    trace,
                    "  {}: {status} (check: {})",
                    pattern.id,
                    debug_check_description(&pattern.check)
                );
            }
        }

        match result {
            CheckResult::Denied {
                pack_id,
                pattern_id,
                ..
            } => {
                let _ = writeln!(trace, "Final verdict: DENY ({pack_id}/{pattern_id})");
            }
            CheckResult::Rewrite {
                pack_id,
                pattern_id,
                ..
            } => {
                let _ = writeln!(trace, "Final verdict: REWRITE ({pack_id}/{pattern_id})");
            }
            CheckResult::Warning {
                pack_id,
                pattern_id,
                ..
            } => {
                let _ = writeln!(trace, "Final verdict: WARNING ({pack_id}/{pattern_id})");
            }
            CheckResult::Allowed => trace.push_str("Final verdict: ALLOW\n"),
        }

        trace
    }

    fn evaluate_command_inner(&self, source: &CommandSource) -> CheckResult {
        if self.should_fail_open() {
            return if self.should_fail_closed() {
                CheckResult::Denied {
                    reason: "Guard crash in fail-closed mode - rejecting all commands".to_string(),
                    pack_id: "fail-closed".to_string(),
                    pattern_id: "guard-crash".to_string(),
                }
            } else {
                CheckResult::Allowed
            };
        }

        // First, evaluate unconditional packs (those with empty tool_keywords) against the
        // entire raw command string. This is the secrets path: packs like secrets scan the
        // whole command regardless of which executable is invoked, catching cases like
        // `echo "ghp_..." >> file` that have no guarded executable to basename-match.
        let full_command = match source {
            CommandSource::Hook(cmd) => cmd.clone(),
            CommandSource::Argv(argv) => {
                if argv.is_empty() {
                    return CheckResult::Allowed;
                }
                argv.join(" ")
            }
        };

        let unconditional_result = self.evaluate_unconditional_packs(&full_command);
        if matches!(unconditional_result, CheckResult::Denied { .. }) {
            return unconditional_result;
        }

        // Then proceed with token-based evaluation for basename-dispatched packs
        let tokens = self.segment_command(source);

        if tokens.is_empty() {
            return unconditional_result;
        }

        let mut result = unconditional_result;

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
                        report_fail_open(&format!("invalid regex in pattern '{}'", pattern.id));
                        Err(())
                    }
                }
            }
            crate::rule_pack::Check::ContentRegex { .. } => {
                // Content regex doesn't apply to command-mode checks
                Ok(false)
            }
            crate::rule_pack::Check::Predicate {
                predicate_name,
                data,
            } => {
                // Evaluate state-aware predicates for Tier 2 rules and
                // data-driven predicates whose policy lives in the pack.
                match self.evaluate_command_predicate(predicate_name, data.as_ref(), command) {
                    Ok(result) => Ok(result),
                    Err(_) => {
                        // Predicate evaluation error - fail open
                        report_fail_open(&format!("predicate evaluation failed for '{}'", predicate_name));
                        Err(())
                    }
                }
            }
        }
    }

    /// Evaluate a command-mode predicate against the current state
    fn evaluate_command_predicate(
        &self,
        predicate_name: &str,
        data: Option<&serde_json::Value>,
        command: &str,
    ) -> Result<bool> {
        match predicate_name {
            "deprecated_command" => self.matches_deprecated_command(data, command),
            "requires_flush_first" => {
                // Tier 2: Deny unless flush has already occurred in this session
                // Returns true (pattern matches/deny) when flush has NOT occurred
                if let Some(state_store) = &self.state_store {
                    Ok(!state_store.has_flush()?)
                } else {
                    // No state store available - fail open (allow the command)
                    Ok(false)
                }
            }
            "requires_pull_first" => {
                // Tier 2: Deny unless git pull has already occurred in this session
                // Returns true (pattern matches/deny) when pull has NOT occurred
                if let Some(state_store) = &self.state_store {
                    Ok(!state_store.has_git_pull()?)
                } else {
                    // No state store available - fail open
                    Ok(false)
                }
            }
            "repair_requires_flush" => {
                // Tier 2: Deny bead/bf doctor --repair unless flush has already occurred this session
                // Returns true (pattern matches/deny) when:
                // 1. The command is a repair operation, AND
                // 2. Flush has NOT occurred in this session
                if let Some(state_store) = &self.state_store {
                    let is_repair = command.contains("doctor") && command.contains("repair");
                    let flush_has_occurred = state_store.has_flush()?;
                    // Deny only if this is a repair command AND flush hasn't happened
                    Ok(is_repair && !flush_has_occurred)
                } else {
                    // No state store available - fail open (allow the command)
                    Ok(false)
                }
            }
            "flush_requires_pull" => {
                // Tier 2: Deny bead/bf sync flush-only unless git pull has already occurred this session
                // Returns true (pattern matches/deny) when:
                // 1. The command is a flush operation (bead/bf sync flush-only), AND
                // 2. Git pull has NOT occurred in this session
                if let Some(state_store) = &self.state_store {
                    let is_flush = command.contains("sync") && command.contains("flush-only");
                    let pull_has_occurred = state_store.has_git_pull()?;
                    // Deny only if this is a flush command AND pull hasn't happened
                    Ok(is_flush && !pull_has_occurred)
                } else {
                    // No state store available - fail open (allow the command)
                    Ok(false)
                }
            }
            _ => {
                // Unknown predicate - fail open (allow the command)
                Ok(false)
            }
        }
    }

    /// Match a command against the deprecated executable names supplied by a
    /// rule pack. The canonical name is data too: it is explicitly allowed,
    /// while the predicate never rewrites one CLI's syntax into another's.
    fn matches_deprecated_command(
        &self,
        data: Option<&serde_json::Value>,
        command: &str,
    ) -> Result<bool> {
        let data = data.context("deprecated_command predicate is missing data")?;
        let object = data
            .as_object()
            .context("deprecated_command predicate data must be an object")?;
        let canonical = object
            .get("currently_canonical")
            .or_else(|| object.get("canonical"))
            .and_then(serde_json::Value::as_str)
            .filter(|name| !name.is_empty())
            .context("deprecated_command data needs a canonical name")?;
        let deprecated = object
            .get("deprecated")
            .and_then(serde_json::Value::as_array)
            .context("deprecated_command data needs a deprecated-name list")?;
        let deprecated: Vec<&str> = deprecated
            .iter()
            .map(|name| {
                name.as_str()
                    .filter(|name| !name.is_empty())
                    .context("deprecated_command data contains an invalid name")
            })
            .collect::<Result<_>>()?;

        let Some(env_assign_pattern) = &self.env_assign_pattern else {
            return Ok(false);
        };

        Ok(lex_shell_commands(command)
            .into_iter()
            .filter_map(|words| {
                command_token_from_words(words, env_assign_pattern, &self.ignored_prefixes)
            })
            .any(|token| token.executable != canonical && deprecated.contains(&token.executable.as_str())))
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
                reason: render_reason(&pattern.redirect.reason_template, None, None),
                pack_id: pack_id.to_string(),
                pattern_id: pattern.id.clone(),
            },
            crate::rule_pack::Channel::UpdatedInput => {
                let rewrite = pattern
                    .redirect
                    .rewrite_template
                    .clone()
                    .unwrap_or_else(|| command.to_string());
                CheckResult::Rewrite {
                    reason: render_reason(&pattern.redirect.reason_template, None, None),
                    rewrite,
                    pack_id: pack_id.to_string(),
                    pattern_id: pattern.id.clone(),
                }
            }
            crate::rule_pack::Channel::AdditionalContext => CheckResult::Warning {
                reason: render_reason(&pattern.redirect.reason_template, None, None),
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
                return if self.should_fail_closed() {
                    // Return empty pack list - evaluation will deny below
                    Vec::new()
                } else {
                    Vec::new()
                };
            }
            packs
                .iter()
                .filter(|pack| {
                    pack.applies_to.iter().any(|glob| {
                        // Early manifests used the tool name (Write/Edit)
                        // as the content selector. Keep those manifests
                        // loadable while newer packs use file globs.
                        matches!(glob.as_str(), "Write" | "Edit") || source.matches_glob(glob)
                    })
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
    /// - Records telemetry result if telemetry store is configured
    pub fn evaluate_content(&self, source: &ContentSource) -> CheckResult {
        let result = match catch_unwind(AssertUnwindSafe(|| self.evaluate_content_inner(source))) {
            Ok(result) => result,
            Err(_) => {
                self.guard_failure_result("content evaluation panicked")
            }
        };

        // Record telemetry result if configured
        self.record_telemetry_result(&result);

        result
    }

    /// Evaluate every file normalized from one multi-file Codex patch and
    /// return the most severe result across the complete patch.
    ///
    pub fn evaluate_content_batch(&self, sources: &[ContentSource]) -> CheckResult {
        let mut result = CheckResult::Allowed;
        for source in sources {
            let file_result = match catch_unwind(AssertUnwindSafe(|| {
                self.evaluate_content_inner(source)
            })) {
                Ok(result) => result,
                Err(_) => {
                    self.guard_failure_result("content batch evaluation panicked")
                }
            };
            result = self.most_severe_result(result, file_result);
            if matches!(result, CheckResult::Denied { .. }) {
                break;
            }
        }

        // Record telemetry result if configured
        self.record_telemetry_result(&result);

        result
    }

    fn evaluate_content_inner(&self, source: &ContentSource) -> CheckResult {
        if self.should_fail_open() {
            return if self.should_fail_closed() {
                CheckResult::Denied {
                    reason: "Guard crash in fail-closed mode - rejecting all operations".to_string(),
                    pack_id: "fail-closed".to_string(),
                    pattern_id: "guard-crash".to_string(),
                }
            } else {
                CheckResult::Allowed
            };
        }

        // Collect all loaded packs for dispatch
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
                if !guarded_pattern.enabled || self.exempted_rule_ids.contains(&guarded_pattern.id)
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
                        source,
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
                        report_fail_open(&format!("invalid regex in pattern '{}'", pattern.id));
                        Err(())
                    }
                }
            }
            crate::rule_pack::Check::CommandRegex { .. } => {
                // Command regex doesn't apply to content-mode checks
                Ok(false)
            }
            crate::rule_pack::Check::Predicate {
                predicate_name,
                ..
            } => {
                // Predicates are evaluated against the target path rather
                // than the text being written. The beads pack's
                // `is_shared_checkout` predicate is additionally scoped by
                // `applies_to` during pack dispatch.
                Ok(predicate_name == "is_shared_checkout"
                    && is_shared_beads_target(source.file_path()))
            }
        }
    }

    /// Convert a GuardedPattern to a CheckResult for content-mode
    fn guarded_pattern_to_result_content(
        &self,
        pattern: &crate::rule_pack::GuardedPattern,
        pack_id: &str,
        source: &ContentSource,
    ) -> CheckResult {
        let reason = render_reason(
            &pattern.redirect.reason_template,
            Some(source.new_content()),
            Some(source.file_path()),
        );

        match pattern.redirect.channel {
            crate::rule_pack::Channel::Deny => CheckResult::Denied {
                reason,
                pack_id: pack_id.to_string(),
                pattern_id: pattern.id.clone(),
            },
            crate::rule_pack::Channel::UpdatedInput => {
                let rewrite = pattern
                    .redirect
                    .rewrite_template
                    .clone()
                    .unwrap_or_else(|| source.new_content().to_string());
                CheckResult::Rewrite {
                    reason,
                    rewrite,
                    pack_id: pack_id.to_string(),
                    pattern_id: pattern.id.clone(),
                }
            }
            crate::rule_pack::Channel::AdditionalContext => CheckResult::Warning {
                reason,
                pack_id: pack_id.to_string(),
                pattern_id: pattern.id.clone(),
            },
        }
    }

    /// Record a telemetry evaluation result if telemetry store is configured
    fn record_telemetry_result(&self, result: &CheckResult) {
        let verdict = result.to_verdict();

        if let Some(telemetry_store) = &self.telemetry_store {
            let release_ref = self.release_ref().map(String::from);
            let session_id = self.session_id().map(String::from);

            let (pack_id, pattern_id) = match result {
                CheckResult::Denied {
                    pack_id, pattern_id, ..
                }
                | CheckResult::Rewrite {
                    pack_id, pattern_id, ..
                }
                | CheckResult::Warning {
                    pack_id, pattern_id, ..
                } => (Some(pack_id.as_str()), Some(pattern_id.as_str())),
                CheckResult::Allowed => (None, None),
            };

            if let Ok(mut store) = telemetry_store.lock() {
                store.record_evaluation_for_rule(
                    verdict,
                    release_ref,
                    session_id,
                    pack_id,
                    pattern_id,
                );
            }
        }

        // Release health is durable state, not just the in-process rolling
        // evaluation window above. Keep this best-effort so a telemetry
        // filesystem problem never changes the guard's allow/deny decision.
        if let (Some(state_store), Some(release_ref)) =
            (&self.state_store, self.release_ref.as_deref())
        {
            if release_ref != "unknown" {
                let _ = state_store.record_release_evaluation(release_ref, verdict.is_deny());
            }
        }
    }

    fn guard_failure_result(&self, message: &str) -> CheckResult {
        report_failure(self.fail_closed, message);
        if self.fail_closed {
            CheckResult::Denied {
                reason: "Guard crash in fail-closed mode - rejecting all operations".to_string(),
                pack_id: "fail-closed".to_string(),
                pattern_id: "guard-crash".to_string(),
            }
        } else {
            CheckResult::Allowed
        }
    }
}

fn debug_check_description(check: &crate::rule_pack::Check) -> String {
    match check {
        crate::rule_pack::Check::CommandRegex { regex } => {
            format!("command regex {regex:?}")
        }
        crate::rule_pack::Check::ContentRegex { regex } => {
            format!("content regex {regex:?}")
        }
        crate::rule_pack::Check::Predicate {
            predicate_name, ..
        } => format!("predicate {predicate_name:?}"),
    }
}

/// Return every executable name that can cause a pack to be evaluated.
///
/// Most packs declare these names in `tool_keywords`. A data-driven
/// deprecated-command predicate additionally contributes the deprecated names
/// from its manifest data, so changing that list is sufficient to retarget the
/// rule at a future CLI cutover.
fn pack_dispatch_keywords(pack: &Pack) -> Vec<String> {
    let mut keywords = pack.tool_keywords.clone();

    for pattern in &pack.guarded_patterns {
        let crate::rule_pack::Check::Predicate {
            predicate_name,
            data: Some(data),
        } = &pattern.check
        else {
            continue;
        };

        if predicate_name != "deprecated_command" {
            continue;
        }

        let Some(deprecated) = data
            .as_object()
            .and_then(|object| object.get("deprecated"))
            .and_then(serde_json::Value::as_array)
        else {
            continue;
        };

        keywords.extend(
            deprecated
                .iter()
                .filter_map(serde_json::Value::as_str)
                .map(str::to_owned),
        );
    }

    keywords
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

fn report_failure(fail_closed: bool, message: &str) {
    use std::io::Write;
    let mode = if fail_closed { "fail-closed" } else { "fail-open" };
    let action = if fail_closed { "denying all commands" } else { "allowing all commands" };
    let _ = writeln!(
        std::io::stderr(),
        "Engine: {mode}: {message} ({action})"
    );
}

/// Report a fail-open event where the guard crashed and is allowing operations
fn report_fail_open(message: &str) {
    use std::io::Write;
    let _ = writeln!(
        std::io::stderr(),
        "Engine: fail-open: {message} (allowing all commands)"
    );
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

    fn beads_pack() -> Pack {
        Pack {
            id: "beads".to_string(),
            tool_keywords: Vec::new(),
            applies_to: vec![".beads/**".to_string()],
            safe_patterns: Vec::new(),
            guarded_patterns: vec![crate::rule_pack::GuardedPattern {
                id: "beads-shared-checkout-write".to_string(),
                enabled: true,
                check: crate::rule_pack::Check::Predicate {
                    predicate_name: "is_shared_checkout".to_string(),
                    data: None,
                },
                tier: crate::rule_pack::Tier::Tier1,
                severity: crate::rule_pack::Severity::Critical,
                explanation: "Writing to .beads/ in a shared checkout risks concurrent corruption"
                    .to_string(),
                redirect: crate::rule_pack::Redirect {
                    channel: crate::rule_pack::Channel::Deny,
                    reason_template: "Use a worktree instead".to_string(),
                    rewrite_template: None,
                },
                destructive: true,
            }],
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
            matching
                .iter()
                .map(|pack| pack.id.as_str())
                .collect::<Vec<_>>(),
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

    #[test]
    fn test_content_regex_checks_the_final_edit_content() {
        let mut engine = default_engine();
        engine
            .load_pack(Pack {
                id: "storage-class".to_string(),
                tool_keywords: Vec::new(),
                applies_to: vec!["*.yaml".to_string()],
                safe_patterns: Vec::new(),
                guarded_patterns: vec![crate::rule_pack::GuardedPattern {
                    id: "ssd-storage-class".to_string(),
                    enabled: true,
                    check: crate::rule_pack::Check::ContentRegex {
                        regex: "storageClassName:.*ssd".to_string(),
                    },
                    tier: crate::rule_pack::Tier::Tier1,
                    severity: crate::rule_pack::Severity::High,
                    explanation: "SSD storage class is prohibited".to_string(),
                    redirect: crate::rule_pack::Redirect {
                        channel: crate::rule_pack::Channel::Deny,
                        reason_template: "use sata".to_string(),
                        rewrite_template: None,
                    },
                    destructive: true,
                }],
            })
            .unwrap();

        let source = ContentSource::Edit {
            file_path: "deployment.yaml".to_string(),
            old_content: "storageClassName: ssd".to_string(),
            new_content: "storageClassName: sata".to_string(),
        };

        assert_eq!(engine.evaluate_content(&source), CheckResult::Allowed);
    }

    #[test]
    fn test_beads_predicate_denies_shared_checkout_write() {
        let repository = tempfile::tempdir().unwrap();
        std::fs::create_dir(repository.path().join(".git")).unwrap();
        std::fs::create_dir(repository.path().join(".beads")).unwrap();

        let mut engine = default_engine();
        engine.load_pack(beads_pack()).unwrap();

        let source = ContentSource::Write {
            file_path: repository
                .path()
                .join(".beads/checkpoint/current.json")
                .to_string_lossy()
                .into_owned(),
            content: "{}".to_string(),
        };

        assert!(matches!(
            engine.evaluate_content(&source),
            CheckResult::Denied {
                ref pack_id,
                ref pattern_id,
                ..
            } if pack_id == "beads" && pattern_id == "beads-shared-checkout-write"
        ));
    }

    #[test]
    fn test_beads_predicate_allows_linked_worktree_write() {
        let repository = tempfile::tempdir().unwrap();
        std::fs::write(repository.path().join(".git"), "gitdir: ../main/.git/worktrees/x\n")
            .unwrap();
        std::fs::create_dir(repository.path().join(".beads")).unwrap();

        let mut engine = default_engine();
        engine.load_pack(beads_pack()).unwrap();

        let source = ContentSource::Write {
            file_path: repository.path().join(".beads/events.jsonl").to_string_lossy().into_owned(),
            content: "{}".to_string(),
        };

        assert_eq!(engine.evaluate_content(&source), CheckResult::Allowed);
    }

    #[test]
    fn test_beads_predicate_requires_a_beads_path() {
        let repository = tempfile::tempdir().unwrap();
        std::fs::create_dir(repository.path().join(".git")).unwrap();

        assert!(!is_shared_beads_target(
            &repository.path().join("ordinary.txt").to_string_lossy()
        ));
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
    fn test_segment_with_sudo_options_and_nested_env_prefix() {
        let engine = default_engine();
        let source = CommandSource::Hook(
            "env -i VAULT_TOKEN='x y' sudo --user root /usr/local/bin/vault kv destroy secret/foo"
                .to_string(),
        );
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
    fn test_segment_does_not_split_quoted_or_escaped_separators() {
        let engine = default_engine();
        let source = CommandSource::Hook(
            "vault kv destroy 'secret/foo;bar' && git commit -m \"keep && this\"; echo escaped\\;semi"
                .to_string(),
        );
        let tokens = engine.segment_command(&source);

        assert_eq!(tokens.len(), 3);
        assert_eq!(tokens[0].executable, "vault");
        assert_eq!(tokens[0].args, vec!["kv", "destroy", "secret/foo;bar"]);
        assert_eq!(tokens[1].executable, "git");
        assert_eq!(tokens[1].args, vec!["commit", "-m", "keep && this"]);
        assert_eq!(tokens[2].executable, "echo");
        assert_eq!(tokens[2].args, vec!["escaped;semi"]);
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

    #[test]
    fn test_argv_mode_preserves_separator_arguments() {
        let engine = default_engine();
        let source = engine.read_from_argv(vec![
            "/usr/bin/git".to_string(),
            "commit".to_string(),
            "-m".to_string(),
            "message;still-one-argument".to_string(),
        ]);
        let tokens = engine.segment_command(&source);

        assert_eq!(tokens.len(), 1);
        assert_eq!(tokens[0].executable, "git");
        assert_eq!(
            tokens[0].args,
            vec!["commit", "-m", "message;still-one-argument"]
        );
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
    fn test_command_mode_dispatches_vault_git_misc_and_tmux_packs() {
        let mut engine = default_engine();
        for (id, keyword, command) in [
            ("vault", "vault", "kv destroy"),
            ("git", "git", "push --force"),
            ("misc", "misc", "cleanup"),
            ("tmux", "tmux", "kill-session"),
        ] {
            engine
                .load_pack(crate::rule_pack::Pack {
                    id: id.to_string(),
                    tool_keywords: vec![keyword.to_string()],
                    applies_to: vec![],
                    safe_patterns: vec![],
                    guarded_patterns: vec![crate::rule_pack::GuardedPattern {
                        id: format!("{id}-guard"),
                        enabled: true,
                        check: crate::rule_pack::Check::CommandRegex {
                            regex: format!("{keyword} {command}"),
                        },
                        tier: crate::rule_pack::Tier::Tier1,
                        severity: crate::rule_pack::Severity::High,
                        explanation: "test command rule".to_string(),
                        redirect: crate::rule_pack::Redirect {
                            channel: crate::rule_pack::Channel::Deny,
                            reason_template: "denied".to_string(),
                            rewrite_template: None,
                        },
                        destructive: true,
                    }],
                })
                .unwrap();
        }

        for (command, expected_pack) in [
            ("sudo -n vault kv destroy secret/foo", "vault"),
            ("/usr/bin/git push --force origin main", "git"),
            ("misc cleanup", "misc"),
            ("tmux kill-session -t ABC", "tmux"),
        ] {
            let result = engine.evaluate_command(&CommandSource::Hook(command.to_string()));
            assert!(
                matches!(result, CheckResult::Denied { ref pack_id, .. } if pack_id == expected_pack),
                "expected {expected_pack} to deny {command}, got {result:?}"
            );
        }
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
            args: vec![
                "kv".to_string(),
                "get".to_string(),
                "secret/foo".to_string(),
            ],
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
            args: vec![
                "kv".to_string(),
                "destroy".to_string(),
                "secret/foo".to_string(),
            ],
        };

        let result = engine.evaluate_token(&token);

        match result {
            CheckResult::Denied {
                reason,
                pack_id,
                pattern_id,
            } => {
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
            args: vec![
                "push".to_string(),
                "origin".to_string(),
                "main".to_string(),
                "--force".to_string(),
            ],
        };

        let result = engine.evaluate_token(&token);

        match result {
            CheckResult::Rewrite {
                reason,
                rewrite,
                pack_id,
                pattern_id,
            } => {
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
                    reason_template: "Verify this is a throwaway worktree, not a shared one"
                        .to_string(),
                    rewrite_template: None,
                },
                destructive: false,
            }],
        };

        engine.load_pack(pack).unwrap();

        let token = CommandToken {
            executable: "git".to_string(),
            args: vec![
                "worktree".to_string(),
                "add".to_string(),
                "path".to_string(),
                "branch".to_string(),
            ],
        };

        let result = engine.evaluate_token(&token);

        match result {
            CheckResult::Warning {
                reason,
                pack_id,
                pattern_id,
            } => {
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
            safe_patterns: vec![
                crate::rule_pack::Pattern {
                    id: "safe-status".to_string(),
                    check: crate::rule_pack::Check::CommandRegex {
                        regex: "vault status".to_string(),
                    },
                },
                crate::rule_pack::Pattern {
                    id: "safe-read".to_string(),
                    check: crate::rule_pack::Check::CommandRegex {
                        regex: "vault kv get".to_string(),
                    },
                },
            ],
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
            args: vec![
                "kv".to_string(),
                "get".to_string(),
                "secret/foo".to_string(),
            ],
        };

        let result = engine.evaluate_token(&token);
        assert_eq!(
            result,
            CheckResult::Allowed,
            "safe_pattern should suppress guarded_pattern"
        );
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
            CheckResult::Denied { .. } => {}
            _ => panic!(
                "Expected Denied result for absolute-path invocation, got {:?}",
                result
            ),
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
        assert!(matches!(
            engine.most_severe_result(deny.clone(), allowed.clone()),
            CheckResult::Denied { .. }
        ));
        assert!(matches!(
            engine.most_severe_result(deny.clone(), rewrite.clone()),
            CheckResult::Denied { .. }
        ));
        assert!(matches!(
            engine.most_severe_result(deny.clone(), warning.clone()),
            CheckResult::Denied { .. }
        ));

        // Rewrite is more severe than warning/allowed
        assert!(matches!(
            engine.most_severe_result(rewrite.clone(), allowed.clone()),
            CheckResult::Rewrite { .. }
        ));
        assert!(matches!(
            engine.most_severe_result(rewrite.clone(), warning.clone()),
            CheckResult::Rewrite { .. }
        ));

        // Warning is more severe than allowed
        assert!(matches!(
            engine.most_severe_result(warning.clone(), allowed.clone()),
            CheckResult::Warning { .. }
        ));

        // Same severity returns first
        assert!(matches!(
            engine.most_severe_result(allowed.clone(), allowed.clone()),
            CheckResult::Allowed
        ));
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
            CheckResult::Denied {
                reason,
                pack_id,
                pattern_id,
            } => {
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
            CheckResult::Denied {
                pack_id,
                pattern_id,
                ..
            } => {
                assert_eq!(pack_id, "image-tag");
                assert_eq!(pattern_id, "latest-image-tag");
            }
            _ => panic!(
                "Expected Denied result for edit with :latest, got {:?}",
                result
            ),
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
        assert!(
            matches!(yaml_result, CheckResult::Denied { pack_id, .. } if pack_id == "storage-class")
        );

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

    #[test]
    fn test_unconditional_pack_scans_entire_command() {
        // Test that packs with empty tool_keywords scan the entire raw command string
        // This is the secrets path: catches patterns regardless of executable
        let mut engine = default_engine();

        // Create a secrets-style pack with no tool_keywords
        let secrets_pack = crate::rule_pack::Pack {
            id: "secrets".to_string(),
            tool_keywords: vec![], // Empty means unconditional whole-command scan
            applies_to: vec![],
            safe_patterns: vec![],
            guarded_patterns: vec![crate::rule_pack::GuardedPattern {
                id: "test-token-pattern".to_string(),
                enabled: true,
                check: crate::rule_pack::Check::CommandRegex {
                    regex: "TEST_TOKEN_[0-9]+".to_string(),
                },
                tier: crate::rule_pack::Tier::Tier1,
                severity: crate::rule_pack::Severity::Critical,
                explanation: "Test credential pattern".to_string(),
                redirect: crate::rule_pack::Redirect {
                    channel: crate::rule_pack::Channel::Deny,
                    reason_template: "Test credential detected".to_string(),
                    rewrite_template: None,
                },
                destructive: true,
            }],
        };

        engine.load_pack(secrets_pack).unwrap();

        // Test the canonical example: echo "TEST_TOKEN_..." >> file
        // This has no guarded executable to basename-match, only secrets should catch it
        let source = CommandSource::Hook("echo \"TEST_TOKEN_12345\" >> /tmp/file".to_string());
        let result = engine.evaluate_command(&source);

        match result {
            CheckResult::Denied {
                pack_id,
                pattern_id,
                ..
            } => {
                assert_eq!(pack_id, "secrets");
                assert_eq!(pattern_id, "test-token-pattern");
            }
            _ => panic!(
                "Expected Denied result for secrets in command, got {:?}",
                result
            ),
        }
    }

    #[test]
    fn test_unconditional_pack_runs_alongside_basename_dispatch() {
        // Test that unconditional packs and basename-dispatched packs both run
        let mut engine = default_engine();

        // Unconditional secrets pack
        let secrets_pack = crate::rule_pack::Pack {
            id: "secrets".to_string(),
            tool_keywords: vec![], // Empty = unconditional
            applies_to: vec![],
            safe_patterns: vec![],
            guarded_patterns: vec![crate::rule_pack::GuardedPattern {
                id: "test-credential".to_string(),
                enabled: true,
                check: crate::rule_pack::Check::CommandRegex {
                    regex: "TEST_CRED_[A-Z]+".to_string(),
                },
                tier: crate::rule_pack::Tier::Tier1,
                severity: crate::rule_pack::Severity::Critical,
                explanation: "Test credential pattern".to_string(),
                redirect: crate::rule_pack::Redirect {
                    channel: crate::rule_pack::Channel::Deny,
                    reason_template: "Test credential detected".to_string(),
                    rewrite_template: None,
                },
                destructive: true,
            }],
        };

        // Basename-dispatched vault pack
        let vault_pack = crate::rule_pack::Pack {
            id: "vault".to_string(),
            tool_keywords: vec!["vault".to_string()], // Has tool_keywords = basename dispatch
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
                    reason_template: "Vault destructive operation".to_string(),
                    rewrite_template: None,
                },
                destructive: true,
            }],
        };

        engine.load_pack(secrets_pack).unwrap();
        engine.load_pack(vault_pack).unwrap();

        // Test 1: Secrets fires on echo command (no basename match)
        let echo_source = CommandSource::Hook("echo \"TEST_CRED_ABC\" >> file".to_string());
        let echo_result = engine.evaluate_command(&echo_source);
        assert!(matches!(echo_result, CheckResult::Denied { pack_id, .. } if pack_id == "secrets"));

        // Test 2: Vault fires on vault command (basename match)
        let vault_source = CommandSource::Hook("vault kv destroy secret/foo".to_string());
        let vault_result = engine.evaluate_command(&vault_source);
        assert!(matches!(vault_result, CheckResult::Denied { pack_id, .. } if pack_id == "vault"));

        // Test 3: Both packs can fire in their respective contexts
        let combined_source =
            CommandSource::Hook("vault status; echo \"TEST_CRED_XYZ\"".to_string());
        let combined_result = engine.evaluate_command(&combined_source);
        // Should deny from secrets pack
        assert!(matches!(combined_result, CheckResult::Denied { .. }));
    }

    #[test]
    fn test_unconditional_pack_safe_pattern_skips_guarded() {
        // Test that safe_patterns in unconditional packs suppress guarded_patterns
        let mut engine = default_engine();

        let secrets_pack = crate::rule_pack::Pack {
            id: "secrets".to_string(),
            tool_keywords: vec![], // Unconditional
            applies_to: vec![],
            safe_patterns: vec![crate::rule_pack::Pattern {
                id: "safe-demo".to_string(),
                check: crate::rule_pack::Check::CommandRegex {
                    regex: "echo \"TEST_TOKEN_SAFE\"".to_string(),
                },
            }],
            guarded_patterns: vec![crate::rule_pack::GuardedPattern {
                id: "any-test-token".to_string(),
                enabled: true,
                check: crate::rule_pack::Check::CommandRegex {
                    regex: "TEST_TOKEN_[A-Z0-9]+".to_string(),
                },
                tier: crate::rule_pack::Tier::Tier1,
                severity: crate::rule_pack::Severity::Critical,
                explanation: "Test token pattern".to_string(),
                redirect: crate::rule_pack::Redirect {
                    channel: crate::rule_pack::Channel::Deny,
                    reason_template: "Test token detected".to_string(),
                    rewrite_template: None,
                },
                destructive: true,
            }],
        };

        engine.load_pack(secrets_pack).unwrap();

        // Safe pattern should match, allowing the command
        let safe_source = CommandSource::Hook("echo \"TEST_TOKEN_SAFE\" >> file".to_string());
        let safe_result = engine.evaluate_command(&safe_source);
        assert_eq!(
            safe_result,
            CheckResult::Allowed,
            "safe_pattern should suppress guarded_pattern"
        );

        // Real token pattern should be denied
        let real_source = CommandSource::Hook("echo \"TEST_TOKEN_ABC123\" >> file".to_string());
        let real_result = engine.evaluate_command(&real_source);
        assert!(
            matches!(real_result, CheckResult::Denied { .. }),
            "real token should be denied"
        );
    }

    // Tier 2 state-aware predicate tests

    #[test]
    fn test_requires_flush_first_predicate_denies_without_flush() {
        let temp_dir = tempfile::tempdir().unwrap();
        let state_path = temp_dir.path().join("session-state.json");
        let state_store = std::sync::Arc::new(crate::state_store::StateStore::new(&state_path));

        let mut engine = default_engine().with_state_store(std::sync::Arc::clone(&state_store));

        // Create a misc pack with the requires_flush_first predicate
        let misc_pack = crate::rule_pack::Pack {
            id: "misc".to_string(),
            tool_keywords: vec!["bf".to_string()],
            applies_to: vec![],
            safe_patterns: vec![],
            guarded_patterns: vec![crate::rule_pack::GuardedPattern {
                id: "bf-doctor-repair-before-flush".to_string(),
                enabled: true,
                check: crate::rule_pack::Check::Predicate {
                    predicate_name: "requires_flush_first".to_string(),
                    data: None,
                },
                tier: crate::rule_pack::Tier::Tier2,
                severity: crate::rule_pack::Severity::High,
                explanation: "bf doctor --repair must have flush first".to_string(),
                redirect: crate::rule_pack::Redirect {
                    channel: crate::rule_pack::Channel::Deny,
                    reason_template: "bf doctor --repair requires flush to happen first in this session".to_string(),
                    rewrite_template: None,
                },
                destructive: false,
            }],
        };

        engine.load_pack(misc_pack).unwrap();

        // Test 1: Without flush, the command should be denied
        let repair_command = CommandSource::Hook("bf doctor --repair".to_string());
        let result = engine.evaluate_command(&repair_command);
        assert!(
            matches!(result, CheckResult::Denied { pattern_id, .. } if pattern_id == "bf-doctor-repair-before-flush"),
            "bf doctor --repair should be denied without flush first"
        );
    }

    #[test]
    fn test_requires_flush_first_predicate_allows_after_flush() {
        let temp_dir = tempfile::tempdir().unwrap();
        let state_path = temp_dir.path().join("session-state.json");
        let state_store = std::sync::Arc::new(crate::state_store::StateStore::new(&state_path));

        // Mark flush as having occurred
        state_store.mark_flush().unwrap();

        let mut engine = default_engine().with_state_store(std::sync::Arc::clone(&state_store));

        // Create a misc pack with the requires_flush_first predicate
        let misc_pack = crate::rule_pack::Pack {
            id: "misc".to_string(),
            tool_keywords: vec!["bf".to_string()],
            applies_to: vec![],
            safe_patterns: vec![],
            guarded_patterns: vec![crate::rule_pack::GuardedPattern {
                id: "bf-doctor-repair-before-flush".to_string(),
                enabled: true,
                check: crate::rule_pack::Check::Predicate {
                    predicate_name: "requires_flush_first".to_string(),
                    data: None,
                },
                tier: crate::rule_pack::Tier::Tier2,
                severity: crate::rule_pack::Severity::High,
                explanation: "bf doctor --repair must have flush first".to_string(),
                redirect: crate::rule_pack::Redirect {
                    channel: crate::rule_pack::Channel::Deny,
                    reason_template: "bf doctor --repair requires flush to happen first in this session".to_string(),
                    rewrite_template: None,
                },
                destructive: false,
            }],
        };

        engine.load_pack(misc_pack).unwrap();

        // Test: After flush, the command should be allowed
        let repair_command = CommandSource::Hook("bf doctor --repair".to_string());
        let result = engine.evaluate_command(&repair_command);
        assert_eq!(
            result,
            CheckResult::Allowed,
            "bf doctor --repair should be allowed after flush"
        );
    }

    #[test]
    fn test_requires_pull_first_predicate_denies_without_pull() {
        let temp_dir = tempfile::tempdir().unwrap();
        let state_path = temp_dir.path().join("session-state.json");
        let state_store = std::sync::Arc::new(crate::state_store::StateStore::new(&state_path));

        let mut engine = default_engine().with_state_store(std::sync::Arc::clone(&state_store));

        // Create a beads pack with the requires_pull_first predicate
        let beads_pack = crate::rule_pack::Pack {
            id: "beads".to_string(),
            tool_keywords: vec!["bf".to_string()],
            applies_to: vec![],
            safe_patterns: vec![],
            guarded_patterns: vec![crate::rule_pack::GuardedPattern {
                id: "bf-sync-flush-before-pull".to_string(),
                enabled: true,
                check: crate::rule_pack::Check::Predicate {
                    predicate_name: "requires_pull_first".to_string(),
                    data: None,
                },
                tier: crate::rule_pack::Tier::Tier2,
                severity: crate::rule_pack::Severity::High,
                explanation: "bf sync --flush-only must have pull first".to_string(),
                redirect: crate::rule_pack::Redirect {
                    channel: crate::rule_pack::Channel::Deny,
                    reason_template: "bf sync --flush-only requires git pull to happen first in this session".to_string(),
                    rewrite_template: None,
                },
                destructive: false,
            }],
        };

        engine.load_pack(beads_pack).unwrap();

        // Test: Without pull, the command should be denied
        let flush_command = CommandSource::Hook("bf sync --flush-only".to_string());
        let result = engine.evaluate_command(&flush_command);
        assert!(
            matches!(result, CheckResult::Denied { pattern_id, .. } if pattern_id == "bf-sync-flush-before-pull"),
            "bf sync --flush-only should be denied without pull first"
        );
    }

    #[test]
    fn test_requires_pull_first_predicate_allows_after_pull() {
        let temp_dir = tempfile::tempdir().unwrap();
        let state_path = temp_dir.path().join("session-state.json");
        let state_store = std::sync::Arc::new(crate::state_store::StateStore::new(&state_path));

        // Mark git pull as having occurred
        state_store.mark_git_pull().unwrap();

        let mut engine = default_engine().with_state_store(std::sync::Arc::clone(&state_store));

        // Create a beads pack with the requires_pull_first predicate
        let beads_pack = crate::rule_pack::Pack {
            id: "beads".to_string(),
            tool_keywords: vec!["bf".to_string()],
            applies_to: vec![],
            safe_patterns: vec![],
            guarded_patterns: vec![crate::rule_pack::GuardedPattern {
                id: "bf-sync-flush-before-pull".to_string(),
                enabled: true,
                check: crate::rule_pack::Check::Predicate {
                    predicate_name: "requires_pull_first".to_string(),
                    data: None,
                },
                tier: crate::rule_pack::Tier::Tier2,
                severity: crate::rule_pack::Severity::High,
                explanation: "bf sync --flush-only must have pull first".to_string(),
                redirect: crate::rule_pack::Redirect {
                    channel: crate::rule_pack::Channel::Deny,
                    reason_template: "bf sync --flush-only requires git pull to happen first in this session".to_string(),
                    rewrite_template: None,
                },
                destructive: false,
            }],
        };

        engine.load_pack(beads_pack).unwrap();

        // Test: After pull, the command should be allowed
        let flush_command = CommandSource::Hook("bf sync --flush-only".to_string());
        let result = engine.evaluate_command(&flush_command);
        assert_eq!(
            result,
            CheckResult::Allowed,
            "bf sync --flush-only should be allowed after pull"
        );
    }

    #[test]
    fn test_unknown_predicate_returns_false_and_allows_command() {
        let temp_dir = tempfile::tempdir().unwrap();
        let state_path = temp_dir.path().join("session-state.json");
        let state_store = std::sync::Arc::new(crate::state_store::StateStore::new(&state_path));

        let mut engine = default_engine().with_state_store(std::sync::Arc::clone(&state_store));

        // Create a pack with an unknown predicate
        let test_pack = crate::rule_pack::Pack {
            id: "test".to_string(),
            tool_keywords: vec!["test".to_string()],
            applies_to: vec![],
            safe_patterns: vec![],
            guarded_patterns: vec![crate::rule_pack::GuardedPattern {
                id: "unknown-predicate-test".to_string(),
                enabled: true,
                check: crate::rule_pack::Check::Predicate {
                    predicate_name: "unknown_predicate_xyz".to_string(),
                    data: None,
                },
                tier: crate::rule_pack::Tier::Tier2,
                severity: crate::rule_pack::Severity::Medium,
                explanation: "Unknown predicate test".to_string(),
                redirect: crate::rule_pack::Redirect {
                    channel: crate::rule_pack::Channel::Deny,
                    reason_template: "Should not fire".to_string(),
                    rewrite_template: None,
                },
                destructive: false,
            }],
        };

        engine.load_pack(test_pack).unwrap();

        // Unknown predicate should return false (no match), allowing the command
        let test_command = CommandSource::Hook("test command".to_string());
        let result = engine.evaluate_command(&test_command);
        assert_eq!(
            result,
            CheckResult::Allowed,
            "unknown predicate should not deny command"
        );
    }

    #[test]
    fn test_predicate_without_state_store_fails_open() {
        // Engine without state store should fail open (allow commands)
        let mut engine = default_engine(); // No state store attached

        // Create a pack with the requires_flush_first predicate
        let misc_pack = crate::rule_pack::Pack {
            id: "misc".to_string(),
            tool_keywords: vec!["bf".to_string()],
            applies_to: vec![],
            safe_patterns: vec![],
            guarded_patterns: vec![crate::rule_pack::GuardedPattern {
                id: "bf-doctor-repair-before-flush".to_string(),
                enabled: true,
                check: crate::rule_pack::Check::Predicate {
                    predicate_name: "requires_flush_first".to_string(),
                    data: None,
                },
                tier: crate::rule_pack::Tier::Tier2,
                severity: crate::rule_pack::Severity::High,
                explanation: "bf doctor --repair must have flush first".to_string(),
                redirect: crate::rule_pack::Redirect {
                    channel: crate::rule_pack::Channel::Deny,
                    reason_template: "bf doctor --repair requires flush to happen first in this session".to_string(),
                    rewrite_template: None,
                },
                destructive: false,
            }],
        };

        engine.load_pack(misc_pack).unwrap();

        // Without state store, should fail open and allow the command
        let repair_command = CommandSource::Hook("bf doctor --repair".to_string());
        let result = engine.evaluate_command(&repair_command);
        assert_eq!(
            result,
            CheckResult::Allowed,
            "without state store, predicate should fail open and allow"
        );
    }
}
