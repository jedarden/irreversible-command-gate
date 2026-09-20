//! Idempotent Gemini CLI `settings.json` installer.
//!
//! The Gemini adapter (`src/adapter.rs`, contract §6.4) is wired by hand
//! today: an operator merges a `BeforeTool` hook definition into project
//! `.gemini/settings.json` or user `~/.gemini/settings.json` themselves.
//! Hand-merging is how duplicate ICG entries and clobbered unrelated
//! settings happen, so this module ships the installer as a subcommand
//! (`icg install-gemini-hooks`) with merge semantics that make re-running
//! it a no-op:
//!
//! - An existing entry is **ICG-owned** when its `command` invokes the icg
//!   binary with `hook --harness gemini-cli` — recognized by content, not
//!   by position or marker fields, so no non-schema key ever lands in the
//!   file. The predicate is shared with the Cursor installer
//!   ([`hook_command`](super::hook_command)).
//! - Installing removes every ICG-owned entry from the `BeforeTool` event
//!   array's hook-definition groups and appends exactly one fresh group. A
//!   second run removes what the first run wrote and appends an identical
//!   group, so the file is byte-identical afterwards — that is the
//!   idempotence guarantee. A foreign command sharing a group with an ICG
//!   entry survives; only groups the removal emptied are dropped.
//! - Everything else — unrelated hook definitions, other event arrays
//!   (`BeforeAgent`, `AfterTool`, …), and every other top-level setting —
//!   is preserved with its values intact.
//! - A missing (or empty) target file is created fresh with
//!   `{"hooks": {}}`. A file that does not parse, or whose `hooks` /
//!   `BeforeTool` shape is not what the schema says, fails with a clear
//!   error and is **not** modified.
//!
//! Two deliberate differences from the Cursor installer:
//!
//! - **No `version` handling.** Gemini's settings schema carries no
//!   protocol version to validate.
//! - **No `failClosed` option.** Gemini's hook dispatch is natively
//!   fail-open — any non-0/non-2 exit, a crash, or a timeout is a
//!   non-fatal warning and the CLI continues — and its schema has no
//!   fail-closed field to write. The installed entry carries a bounded
//!   `timeout` (milliseconds in this schema) only so a wedged icg cannot
//!   stall a tool call for the 60 s default.
//!
//! The matcher is part of the safety contract: it is anchored to exactly
//! the three tool names the adapter models (contract §6.4), so Gemini's
//! MCP tools (named `mcp_<server>_<tool>`) and read-only tools are never
//! routed to ICG by the config itself.

use anyhow::{bail, Context, Result};
use serde_json::{json, Value};
use std::fs;
use std::path::{Path, PathBuf};

/// Gemini's `BeforeTool` event: the one hook event that gates a tool call
/// before it runs, and the only one this installer manages.
pub const BEFORE_TOOL_EVENT: &str = "BeforeTool";

/// `BeforeTool` matcher: exactly the three tool names the
/// `GeminiCliAdapter` models — `run_shell_command` (the Bash `command`
/// shape), `write_file` (the Write shape), `replace` (the Edit shape).
/// The anchors are the scoping guarantee: Gemini matchers are regular
/// expressions compared against tool names, so an unanchored `write_file`
/// would also match an MCP tool named `mcp_fs_write_file`, and read-only
/// tools would be one upstream rename away from the gate. Anchored, MCP
/// and read-only tools never route to ICG through this entry.
pub const BEFORE_TOOL_MATCHER: &str = "^(run_shell_command|write_file|replace)$";

/// Milliseconds recorded on the installed entry. The engine evaluates in
/// well under a second; Gemini's schema measures `timeout` in
/// milliseconds (default 60000), and a timeout there is a non-fatal
/// warning — the CLI continues — so the bound caps a wedged hook's stall,
/// it does not change any verdict.
const HOOK_TIMEOUT_MS: u64 = 10_000;

/// The one hook group the installer manages, for the `BeforeTool` event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GeminiHookPlan {
    pub matcher: String,
    pub command: String,
    pub timeout_ms: u64,
}

impl GeminiHookPlan {
    /// The group as it lands in `settings.json`: one hook definition
    /// carrying exactly the schema fields this install needs — `matcher`,
    /// and a one-entry `hooks` array with the required `type: "command"`.
    fn entry_value(&self) -> Value {
        json!({
            "matcher": self.matcher,
            "hooks": [{
                "type": "command",
                "command": self.command,
                "timeout": self.timeout_ms,
            }],
        })
    }
}

/// The entry the installer manages, derived from the running binary.
pub fn installed_entries(icg_binary: &Path, rule_pack: Option<&Path>) -> Vec<GeminiHookPlan> {
    let rule_pack_suffix = match rule_pack {
        Some(path) => format!(
            " --rule-pack {}",
            crate::hook_command::shell_quote(&path.to_string_lossy())
        ),
        None => String::new(),
    };
    let icg = crate::hook_command::shell_quote(&icg_binary.to_string_lossy());

    vec![GeminiHookPlan {
        matcher: BEFORE_TOOL_MATCHER.to_string(),
        command: format!("{icg} hook --harness gemini-cli{rule_pack_suffix}"),
        timeout_ms: HOOK_TIMEOUT_MS,
    }]
}

/// Is this `command` an entry the installer owns?
///
/// Owned means the command invokes the icg binary in hook mode for the
/// gemini-cli harness; the full predicate (and its shell-word machinery)
/// is [`hook_command::is_icg_hook_command_for`](super::hook_command).
pub fn is_icg_hook_command(command: &str) -> bool {
    crate::hook_command::is_icg_hook_command_for(command, "gemini-cli")
}

/// What a merge did, so the caller can report (and skip the write) honestly.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct MergeReport {
    /// The target had no usable content: the configuration was built fresh.
    pub created: bool,
    /// The serialized result differs from the input text (or there was none).
    pub changed: bool,
    pub removed_icg_entries: usize,
    pub added_icg_entries: usize,
}

/// Merge the planned entries into a `settings.json` document.
///
/// `existing` is the file's current text, or `None` when there is no file.
/// With `uninstall` the merge instead removes every ICG-owned entry from
/// the `BeforeTool` array and drops groups it emptied (only those —
/// groups that were already empty stay, since they are not ours to clean
/// up), and a file carrying no `BeforeTool` array is returned untouched:
/// an uninstall never adds keys. Returns the full serialized document
/// plus a report; the caller compares and writes.
///
/// Key order in the output is serde_json's canonical (sorted) order, and
/// no other value is altered. That determinism is what makes "run twice,
/// get identical bytes" hold.
pub fn merge_settings_json(
    existing: Option<&str>,
    plans: &[GeminiHookPlan],
    uninstall: bool,
) -> Result<(String, MergeReport)> {
    let mut report = MergeReport::default();

    let mut root = match existing {
        Some(text) if !text.trim().is_empty() => {
            serde_json::from_str::<Value>(text).map_err(|error| {
                anyhow::anyhow!("existing settings.json is not valid JSON: {error}")
            })?
        }
        _ => {
            report.created = true;
            json!({ "hooks": {} })
        }
    };

    let object = root
        .as_object_mut()
        .ok_or_else(|| anyhow::anyhow!("top level of settings.json must be a JSON object"))?;

    let array = if uninstall {
        // An uninstall only touches a file that actually carries our
        // event array; anything else passes through byte-for-byte.
        match object
            .get_mut("hooks")
            .and_then(Value::as_object_mut)
            .and_then(|hooks| hooks.get_mut(BEFORE_TOOL_EVENT))
            .and_then(Value::as_array_mut)
        {
            Some(array) => array,
            None => {
                report.changed = false;
                return Ok((existing.map(str::to_string).unwrap_or_default(), report));
            }
        }
    } else {
        if !object.contains_key("hooks") {
            object.insert("hooks".to_string(), json!({}));
        }
        let hooks = object
            .get_mut("hooks")
            .and_then(Value::as_object_mut)
            .ok_or_else(|| anyhow::anyhow!("\"hooks\" in settings.json must be a JSON object"))?;
        if !hooks.contains_key(BEFORE_TOOL_EVENT) {
            hooks.insert(BEFORE_TOOL_EVENT.to_string(), Value::Array(Vec::new()));
        }
        hooks
            .get_mut(BEFORE_TOOL_EVENT)
            .and_then(Value::as_array_mut)
            .ok_or_else(|| {
                anyhow::anyhow!("hooks.{BEFORE_TOOL_EVENT} in settings.json must be a JSON array")
            })?
    };

    // Only entries with a string `command` can be ICG-owned; anything else
    // in a group (a prompt hook, a malformed entry) is not ours to judge.
    let is_icg_entry = |entry: &Value| {
        entry
            .get("command")
            .and_then(Value::as_str)
            .map(is_icg_hook_command)
            .unwrap_or(false)
    };

    // Sweep ICG-owned entries out of every definition group's `hooks`
    // array, dropping only groups the sweep emptied. A group without a
    // usable `hooks` array cannot be hiding our entries, so it passes
    // through untouched.
    let mut kept_groups: Vec<Value> = Vec::with_capacity(array.len());
    for group in array.iter() {
        let Some(entries) = group.get("hooks").and_then(Value::as_array) else {
            kept_groups.push(group.clone());
            continue;
        };
        let before = entries.len();
        let surviving: Vec<Value> = entries
            .iter()
            .filter(|entry| !is_icg_entry(entry))
            .cloned()
            .collect();
        let removed = before - surviving.len();
        if removed == 0 {
            kept_groups.push(group.clone());
            continue;
        }
        report.removed_icg_entries += removed;
        if surviving.is_empty() {
            continue; // the group held only ICG entries; it goes with them
        }
        let mut rebuilt = group.as_object().cloned().unwrap_or_default();
        rebuilt.insert("hooks".to_string(), Value::Array(surviving));
        kept_groups.push(Value::Object(rebuilt));
    }
    *array = kept_groups;

    if !uninstall {
        for plan in plans {
            array.push(plan.entry_value());
            report.added_icg_entries += 1;
        }
    }

    let serialized = serialize(&root)?;

    report.changed = match existing {
        Some(text) => text != serialized,
        // Uninstalling from a file that does not exist changes nothing.
        None => !uninstall,
    };

    Ok((serialized, report))
}

fn serialize(root: &Value) -> Result<String> {
    let mut serialized = serde_json::to_string_pretty(root).map_err(|error| {
        anyhow::anyhow!("failed to serialize the merged settings.json: {error}")
    })?;
    serialized.push('\n');
    Ok(serialized)
}

/// Which file the installer manages.
#[derive(Debug, Clone, Default)]
pub struct InstallOptions {
    /// Manage `~/.gemini/settings.json` instead of the project file.
    pub user: bool,
    /// Project directory whose `.gemini/settings.json` is managed.
    pub project_dir: Option<PathBuf>,
    /// Exact path; overrides both of the above.
    pub file: Option<PathBuf>,
    /// Rule-pack path recorded in the installed hook command.
    pub rule_pack: Option<PathBuf>,
    /// Remove ICG entries instead of installing them.
    pub uninstall: bool,
}

impl InstallOptions {
    pub fn resolve_target_path(&self) -> Result<PathBuf> {
        if let Some(file) = &self.file {
            return Ok(file.clone());
        }
        if self.user {
            let home = std::env::var_os("HOME")
                .map(PathBuf::from)
                .context("cannot determine the home directory; pass --file <path> explicitly")?;
            return Ok(home.join(".gemini").join("settings.json"));
        }
        let project_dir = match &self.project_dir {
            Some(dir) => dir.clone(),
            None => std::env::current_dir().context(
                "cannot determine the current directory; pass --project-dir <dir> or --file <path>",
            )?,
        };
        Ok(project_dir.join(".gemini").join("settings.json"))
    }
}

/// Entry point for `icg install-gemini-hooks`.
pub fn run_install(options: InstallOptions) -> Result<()> {
    let target = options.resolve_target_path()?;
    let existing = if target.exists() {
        Some(
            fs::read_to_string(&target)
                .with_context(|| format!("failed to read {}", target.display()))?,
        )
    } else {
        None
    };

    let plans = if options.uninstall {
        Vec::new()
    } else {
        let icg_binary =
            std::env::current_exe().context("could not determine the icg binary path")?;
        if !icg_binary.exists() {
            bail!("icg binary not found at {}", icg_binary.display());
        }
        installed_entries(&icg_binary, options.rule_pack.as_deref())
    };

    let (serialized, report) = merge_settings_json(existing.as_deref(), &plans, options.uninstall)
        .with_context(|| {
            format!(
                "refusing to modify {}; the file is left unchanged",
                target.display()
            )
        })?;

    if report.changed {
        write_target(&target, &serialized, existing.as_deref())?;
    }
    print_report(&target, &report, &plans, options.uninstall);
    Ok(())
}

/// Write the merged document, backing up a non-empty prior state once.
///
/// The backup is the installer's one concession to caution: the merge
/// preserves values but normalizes key order, so a pre-install snapshot of
/// a hand-maintained file is worth one file on disk. It is written only
/// when absent, so the backup always holds the pre-installer state, and
/// never on a no-op run.
fn write_target(target: &Path, serialized: &str, existing: Option<&str>) -> Result<()> {
    let backup_path = PathBuf::from(format!("{}.icg-backup", target.display()));
    if let Some(text) = existing {
        if !text.trim().is_empty() && !backup_path.exists() {
            fs::write(&backup_path, text)
                .with_context(|| format!("failed to write backup {}", backup_path.display()))?;
        }
    }

    if let Some(parent) = target.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent)
                .with_context(|| format!("failed to create {}", parent.display()))?;
        }
    }
    fs::write(target, serialized).with_context(|| format!("failed to write {}", target.display()))
}

fn print_report(target: &Path, report: &MergeReport, plans: &[GeminiHookPlan], uninstall: bool) {
    let state = if report.changed {
        if report.created {
            "created"
        } else {
            "updated"
        }
    } else {
        "already up to date"
    };
    println!("Gemini CLI hooks installer");
    println!("Target: {} ({state})", target.display());

    if uninstall {
        if report.removed_icg_entries == 0 {
            println!("No ICG entries found; nothing removed.");
        } else {
            println!(
                "Removed {} ICG entr{}.",
                report.removed_icg_entries,
                if report.removed_icg_entries == 1 {
                    "y"
                } else {
                    "ies"
                }
            );
        }
        return;
    }

    for plan in plans {
        println!(
            "  {}: matcher {:?}, timeout {}ms: {}",
            BEFORE_TOOL_EVENT, plan.matcher, plan.timeout_ms, plan.command
        );
    }
    println!(
        "ICG entries: {} replaced, {} installed. Unrelated hooks, event \
         arrays and settings are never touched; ICG-owned entries are \
         recognized by their command line and replaced in place, so \
         re-running is a no-op.",
        report.removed_icg_entries, report.added_icg_entries
    );
    println!(
        "Matcher: {BEFORE_TOOL_MATCHER} — scoped to the three tools the \
         adapter models, so MCP tools (mcp_<server>_<tool>) and read-only \
         tools are never routed to ICG."
    );
    println!(
        "Placement: the project file (.gemini/settings.json) covers Gemini \
         CLI sessions started in that directory; ~/.gemini/settings.json \
         covers every other session for the account. Install at the \
         project level to gate the repositories that need it, or --user \
         for account-wide coverage. Gemini's dispatch is natively \
         fail-open (a failed hook is a warning and the CLI continues), so \
         there is no failClosed option to set."
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn install_plans() -> Vec<GeminiHookPlan> {
        installed_entries(Path::new("/usr/local/bin/icg"), None)
    }

    #[test]
    fn recognizes_icg_entries_by_their_command_line() {
        assert!(is_icg_hook_command(
            "/usr/local/bin/icg hook --harness gemini-cli"
        ));
        assert!(is_icg_hook_command(
            "icg hook --harness gemini-cli --rule-pack /etc/icg/packs"
        ));
        assert!(is_icg_hook_command(
            "'/opt/my tools/icg' hook --harness gemini-cli"
        ));

        // Not icg, not hook mode, or the cursor harness: not ours.
        assert!(!is_icg_hook_command("echo icg hook --harness gemini-cli"));
        assert!(!is_icg_hook_command("/usr/local/bin/icg hook"));
        assert!(!is_icg_hook_command(
            "/usr/local/bin/icg hook --harness cursor"
        ));
        assert!(!is_icg_hook_command("/usr/local/bin/icg coverage --list"));
        assert!(!is_icg_hook_command("icg-hook --harness gemini-cli"));
        assert!(!is_icg_hook_command(""));
    }

    #[test]
    fn merge_into_missing_document_creates_the_before_tool_event() {
        let (serialized, report) = merge_settings_json(None, &install_plans(), false)
            .expect("a missing document always merges");

        let root: Value = serde_json::from_str(&serialized).expect("output must be valid JSON");
        let groups = root["hooks"]["BeforeTool"].as_array().unwrap();
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0]["matcher"], BEFORE_TOOL_MATCHER);
        let entries = groups[0]["hooks"].as_array().unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0]["type"], "command");
        assert_eq!(entries[0]["timeout"], 10_000);
        assert!(report.created && report.changed);
        assert_eq!(report.added_icg_entries, 1);
    }

    #[test]
    fn second_merge_is_byte_identical_and_reports_no_change() {
        let (first, _) =
            merge_settings_json(None, &install_plans(), false).expect("first merge succeeds");
        let (second, report) = merge_settings_json(Some(&first), &install_plans(), false)
            .expect("second merge succeeds");

        assert_eq!(
            first, second,
            "re-running the installer must not change bytes"
        );
        assert!(!report.changed);
        assert_eq!(report.removed_icg_entries, 1);
        assert_eq!(report.added_icg_entries, 1);
    }

    #[test]
    fn merge_preserves_unrelated_groups_events_and_settings() {
        let existing = r#"{
  "model": "gemini-2.5-pro",
  "hooks": {
    "BeforeTool": [
      {
        "matcher": "run_shell_command",
        "hooks": [
          {"type": "command", "command": "echo mine", "timeout": 500},
          {"type": "command", "command": "/usr/local/bin/icg hook --harness gemini-cli", "timeout": 99}
        ]
      },
      {
        "matcher": "write_file",
        "hooks": [{"type": "command", "command": "./hooks/audit.sh"}]
      }
    ],
    "BeforeAgent": [
      {"matcher": "", "hooks": [{"type": "command", "command": "./hooks/prompt.sh"}]}
    ]
  },
  "theme": "auto"
}"#;

        let (serialized, report) =
            merge_settings_json(Some(existing), &install_plans(), false).expect("merge succeeds");
        let root: Value = serde_json::from_str(&serialized).expect("output must be valid JSON");

        // Unrelated top-level settings and event arrays keep every value.
        assert_eq!(root["model"], json!("gemini-2.5-pro"));
        assert_eq!(root["theme"], json!("auto"));
        assert_eq!(
            root["hooks"]["BeforeAgent"],
            json!([{"matcher": "", "hooks": [{"type": "command", "command": "./hooks/prompt.sh"}]}])
        );

        // The unrelated command sharing our group keeps its entry verbatim;
        // the stale ICG entry was replaced (removed from the shared group,
        // re-appended as the fresh group), not duplicated: both pre-existing
        // groups survive and our one group joins them.
        let groups = root["hooks"]["BeforeTool"].as_array().unwrap();
        assert_eq!(groups.len(), 3);
        let ours = groups
            .iter()
            .find(|group| group["matcher"] == json!(BEFORE_TOOL_MATCHER))
            .expect("the installed group is present");
        assert_eq!(ours["hooks"].as_array().unwrap().len(), 1);
        let foreign = groups
            .iter()
            .find(|group| group["matcher"] == json!("run_shell_command"))
            .expect("the pre-existing group survives");
        let commands: Vec<&str> = foreign["hooks"]
            .as_array()
            .unwrap()
            .iter()
            .map(|entry| entry["command"].as_str().unwrap())
            .collect();
        assert_eq!(commands, vec!["echo mine"]);
        assert_eq!(report.removed_icg_entries, 1);
        assert_eq!(report.added_icg_entries, 1);
    }

    #[test]
    fn merge_rejects_malformed_json_and_wrong_shapes() {
        let malformed = "{\"hooks\": {";
        let error = merge_settings_json(Some(malformed), &install_plans(), false)
            .expect_err("malformed JSON must fail");
        assert!(
            error.to_string().contains("not valid JSON"),
            "error should say the JSON is invalid, got: {error}"
        );

        for text in [
            r#"{"hooks": {"BeforeTool": "not-an-array"}}"#,
            r#"{"hooks": "not-an-object"}"#,
            "[1, 2, 3]",
        ] {
            assert!(
                merge_settings_json(Some(text), &install_plans(), false).is_err(),
                "expected {text} to be rejected"
            );
        }
    }

    #[test]
    fn uninstall_removes_only_icg_entries_and_groups_it_emptied() {
        let existing = r#"{
  "hooks": {
    "BeforeTool": [
      {"matcher": "run_shell_command", "hooks": [
        {"type": "command", "command": "echo mine"},
        {"type": "command", "command": "icg hook --harness gemini-cli"}
      ]},
      {"matcher": "write_file", "hooks": [
        {"type": "command", "command": "icg hook --harness gemini-cli"}
      ]}
    ],
    "AfterTool": [{"matcher": "", "hooks": [{"type": "command", "command": "./hooks/audit.sh"}]}]
  }
}"#;

        let (serialized, report) =
            merge_settings_json(Some(existing), &[], true).expect("uninstall succeeds");
        let root: Value = serde_json::from_str(&serialized).expect("output must be valid JSON");

        assert_eq!(report.removed_icg_entries, 2);
        assert!(report.changed);
        let groups = root["hooks"]["BeforeTool"].as_array().unwrap();
        assert_eq!(groups.len(), 1);
        assert_eq!(
            groups[0]["hooks"][0]["command"],
            json!("echo mine"),
            "the foreign entry sharing a group with ours survives"
        );
        // Other events are untouched.
        assert!(root["hooks"]["AfterTool"].is_array());

        // Uninstalling again is honest about finding nothing.
        let (again, report) =
            merge_settings_json(Some(&serialized), &[], true).expect("second uninstall succeeds");
        assert_eq!(report.removed_icg_entries, 0);
        assert!(!report.changed, "a no-op uninstall must not rewrite");
        assert_eq!(again, serialized);
    }

    #[test]
    fn uninstall_from_a_file_without_our_event_changes_nothing() {
        let existing = r#"{"hooks": {"AfterTool": []}}"#;
        let (serialized, report) =
            merge_settings_json(Some(existing), &[], true).expect("uninstall succeeds");
        assert_eq!(report.removed_icg_entries, 0);
        assert!(!report.changed);
        let root: Value = serde_json::from_str(&serialized).unwrap();
        assert_eq!(root["hooks"]["AfterTool"], json!([]));
        assert!(root["hooks"].get("BeforeTool").is_none());
    }

    #[test]
    fn rule_pack_lands_in_the_installed_command() {
        let plans = installed_entries(
            Path::new("/usr/local/bin/icg"),
            Some(Path::new("/etc/icg/packs")),
        );

        assert!(plans[0].command.ends_with(" --rule-pack /etc/icg/packs"));
        assert_eq!(
            plans[0].entry_value(),
            json!({
                "matcher": BEFORE_TOOL_MATCHER,
                "hooks": [{
                    "type": "command",
                    "command": "/usr/local/bin/icg hook --harness gemini-cli --rule-pack /etc/icg/packs",
                    "timeout": 10_000,
                }],
            })
        );
    }

    #[test]
    fn paths_with_spaces_are_shell_quoted_in_commands() {
        let plans = installed_entries(Path::new("/opt/my tools/icg"), None);
        assert!(plans[0]
            .command
            .starts_with("'/opt/my tools/icg' hook --harness gemini-cli"));
    }

    #[test]
    fn the_installed_matcher_is_anchored_to_the_three_covered_tools() {
        // Contract §6.4: only run_shell_command, write_file and replace are
        // modeled. The anchors are what keep everything else — MCP tools
        // named mcp_<server>_<tool>, and read-only tools — from ever
        // matching the installed entry.
        assert_eq!(
            BEFORE_TOOL_MATCHER,
            "^(run_shell_command|write_file|replace)$"
        );
        let pattern = regex::Regex::new(BEFORE_TOOL_MATCHER).expect("matcher compiles");
        for name in ["run_shell_command", "write_file", "replace"] {
            assert!(pattern.is_match(name), "{name} must match");
        }
        for name in [
            "mcp_fs_write_file",
            "mcp__fs__write_file",
            "read_file",
            "read_many_files",
            "glob",
            "grep",
            "search_file_content",
            "ls",
            "google_web_search",
            "safe_write_file",
            "replace_all",
            "",
        ] {
            assert!(
                !pattern.is_match(name),
                "{name:?} must never route to ICG through the matcher"
            );
        }
    }
}
