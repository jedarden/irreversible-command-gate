//! Engine: command-mode and content-mode input acquisition
//!
//! This module handles:
//! - Input acquisition from PreToolUse JSON (hook mode) or argv (wrapper mode)
//! - Command-mode: shell line segmentation (splits on ;/&&/||/, skips sudo/env-assignment/wrapper prefixes)
//!   Basename-matching tokens against tool_keywords for pack dispatch
//! - Content-mode: file path + content reading from Write/Edit PreToolUse JSON
//!   Used for content regex checks (storage-class, image-tag packs)

use anyhow::{Context, Result};
use crate::rule_pack::Pack;
use regex::Regex;
use serde::{Deserialize, Serialize};
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

/// Engine: command-mode input acquisition and segmentation
pub struct Engine {
    /// Splits on ||, &&, ;, |, &, newline
    segment_splitter: Regex,
    /// Detects env assignments like VAR=value or FOO_BAR=value
    env_assign_pattern: Regex,
    /// Prefixes to skip: sudo, command, exec, time, nohup
    ignored_prefixes: Vec<String>,
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
        let segment_splitter = Regex::new(r"(?:\|\||&&|[;&|\n])")
            .expect("Invalid segment regex");

        // Match env variable assignments: starts with letter or underscore, followed by alphanumerics/underscores, then =
        // Pattern: ^[A-Za-z_][A-Za-z0-9_]*=
        let env_assign_pattern = Regex::new(r"^[A-Za-z_][A-Za-z0-9_]*=")
            .expect("Invalid env assign regex");

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
        }
    }

    /// Read input from PreToolUse JSON on stdin (hook mode)
    ///
    /// Returns the appropriate InputSource:
    /// - CommandSource for Bash tool calls
    /// - ContentSource for Write/Edit tool calls
    /// - None for unrecognized tools
    pub fn read_from_stdin(&self) -> Result<Option<InputSource>> {
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
        for segment in self.segment_splitter.split(&command_text) {
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
                if self.env_assign_pattern.is_match(tok) {
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
        packs
            .iter()
            .filter(|pack| {
                pack.applies_to
                    .iter()
                    .any(|glob| source.matches_glob(glob))
            })
            .collect()
    }
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
}
