//! Idempotent Cursor `hooks.json` installer.
//!
//! The Cursor adapters (`src/adapter.rs`, contract §6.5) are wired by hand
//! today: an operator merges JSON fragments into `.cursor/hooks.json` or
//! `~/.cursor/hooks.json` themselves. Hand-merging is how duplicate ICG
//! entries and clobbered unrelated hooks happen, so this module ships the
//! installer as a subcommand (`icg install-cursor-hooks`) with merge
//! semantics that make re-running it a no-op:
//!
//! - An existing entry is **ICG-owned** when its `command` invokes the icg
//!   binary with `hook --harness cursor` — recognized by content, not by
//!   position or marker fields, so no non-schema key ever lands in the file
//!   (Cursor blocks a permission-hook response that does not match its
//!   schema; we keep the config file equally clean). The predicate and the
//!   shell-word machinery under it are shared with the Gemini installer in
//!   [`hook_command`](super::hook_command), so both installers recognize
//!   ownership identically.
//! - Installing removes every ICG-owned entry from the target event arrays
//!   and appends exactly one fresh entry per event. A second run removes
//!   what the first run wrote and appends an identical entry, so the file
//!   is byte-identical afterwards — that is the idempotence guarantee.
//! - Everything else — unrelated hooks, matchers, timeouts, `failClosed`
//!   settings, unknown event arrays, unknown top-level keys — is preserved
//!   with its values intact.
//! - A missing (or empty) target file is created fresh with
//!   `{"version": 1, "hooks": {}}`. A file that does not parse, or carries
//!   a `version` other than 1, fails with a clear error and is **not**
//!   modified.
//!
//! Placement is part of the feature: Cursor cloud agents load
//! **project-level** hooks only — `~/.cursor/hooks.json` is not available
//! to them — so the project file is what covers cloud agent sessions, and
//! the user-level file covers local IDE sessions alone. The installer says
//! so on every run, and
//! [`harness-adapter-contract.md`](../docs/notes/harness-adapter-contract.md)
//! §6.5 documents it normatively.

use anyhow::{bail, Context, Result};
use serde_json::{json, Value};
use std::fs;
use std::path::{Path, PathBuf};

/// Cursor's `preToolUse` event: generic tool-call gating.
pub const PRE_TOOL_USE_EVENT: &str = "preToolUse";
/// Cursor's dedicated shell event: one payload per shell command.
pub const BEFORE_SHELL_EXECUTION_EVENT: &str = "beforeShellExecution";

/// Seconds recorded on the installed entries. The engine evaluates in
/// well under a second; 10 matches the timeout quick-start documents for
/// the Claude Code hook.
const HOOK_TIMEOUT_SECS: u64 = 10;

/// `preToolUse` matcher: Cursor's shell tool is named `Shell`, and its
/// edit payloads arrive under the `Write` tool name (contract §6.5, §3.2).
/// `Edit` is included for the same coverage the Claude Code matcher spells
/// `Bash|Write|Edit`; the matcher is a regex.
const PRE_TOOL_USE_MATCHER: &str = "Shell|Write|Edit";

/// `beforeShellExecution` matcher: the empty string matches every command,
/// which is what this event needs — it sees one payload per command and has
/// no tool name to filter on.
const BEFORE_SHELL_EXECUTION_MATCHER: &str = "";

/// One hook entry the installer manages, for a single Cursor event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CursorHookPlan {
    pub event: &'static str,
    pub command: String,
    pub matcher: String,
    pub timeout: u64,
    pub fail_closed: bool,
}

impl CursorHookPlan {
    /// The entry as it lands in `hooks.json`. `failClosed` is only written
    /// when requested: ICG's own posture is fail-open, so the default
    /// entry leaves Cursor's native fail-open in place.
    fn entry_value(&self) -> Value {
        let mut entry = json!({
            "command": self.command,
            "matcher": self.matcher,
            "timeout": self.timeout,
        });
        if self.fail_closed {
            entry
                .as_object_mut()
                .expect("entry was built as an object")
                .insert("failClosed".to_string(), json!(true));
        }
        entry
    }
}

/// The two entries the installer manages, derived from the running binary.
pub fn installed_entries(
    icg_binary: &Path,
    rule_pack: Option<&Path>,
    fail_closed: bool,
) -> Vec<CursorHookPlan> {
    let rule_pack_suffix = match rule_pack {
        Some(path) => {
            format!(
                " --rule-pack {}",
                crate::hook_command::shell_quote(&path.to_string_lossy())
            )
        }
        None => String::new(),
    };
    let icg = crate::hook_command::shell_quote(&icg_binary.to_string_lossy());

    vec![
        CursorHookPlan {
            event: PRE_TOOL_USE_EVENT,
            command: format!("{icg} hook --harness cursor{rule_pack_suffix}"),
            matcher: PRE_TOOL_USE_MATCHER.to_string(),
            timeout: HOOK_TIMEOUT_SECS,
            fail_closed,
        },
        CursorHookPlan {
            event: BEFORE_SHELL_EXECUTION_EVENT,
            command: format!(
                "{icg} hook --harness cursor --event before-shell-execution{rule_pack_suffix}"
            ),
            matcher: BEFORE_SHELL_EXECUTION_MATCHER.to_string(),
            timeout: HOOK_TIMEOUT_SECS,
            fail_closed,
        },
    ]
}

/// Is this `command` an entry the installer owns?
///
/// Owned means the command invokes the icg binary in hook mode for the
/// cursor harness; the full predicate (and its shell-word machinery) is
/// [`hook_command::is_icg_hook_command_for`](super::hook_command).
pub fn is_icg_hook_command(command: &str) -> bool {
    crate::hook_command::is_icg_hook_command_for(command, "cursor")
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

/// Merge the planned entries into a `hooks.json` document.
///
/// `existing` is the file's current text, or `None` when there is no file.
/// With `uninstall` the merge instead removes every ICG-owned entry and
/// drops event arrays it emptied (only those — arrays that were already
/// empty stay, since they are not ours to clean up). Returns the full
/// serialized document plus a report; the caller compares and writes.
///
/// Key order in the output is serde_json's canonical (sorted) order, and
/// `version` is filled in when absent; no other value is altered. That
/// determinism is what makes "run twice, get identical bytes" hold.
pub fn merge_hooks_json(
    existing: Option<&str>,
    plans: &[CursorHookPlan],
    uninstall: bool,
) -> Result<(String, MergeReport)> {
    let mut report = MergeReport::default();

    let mut root = match existing {
        Some(text) if !text.trim().is_empty() => serde_json::from_str::<Value>(text)
            .map_err(|error| anyhow::anyhow!("existing hooks.json is not valid JSON: {error}"))?,
        _ => {
            report.created = true;
            json!({ "version": 1, "hooks": {} })
        }
    };

    let object = root
        .as_object_mut()
        .ok_or_else(|| anyhow::anyhow!("top level of hooks.json must be a JSON object"))?;

    match object.get("version") {
        None => {
            object.insert("version".to_string(), json!(1));
        }
        Some(value) => {
            if value.as_u64() != Some(1) {
                bail!(
                    "unsupported hooks.json \"version\" ({value}): this installer \
                     manages schema version 1 only"
                );
            }
        }
    }

    if !object.contains_key("hooks") {
        object.insert("hooks".to_string(), json!({}));
    }
    let hooks = object
        .get_mut("hooks")
        .expect("hooks key exists by now")
        .as_object_mut()
        .ok_or_else(|| anyhow::anyhow!("\"hooks\" in hooks.json must be a JSON object"))?;

    // Only entries with a string `command` can be ICG-owned; anything else
    // in an array (a prompt hook, a malformed entry) is not ours to judge.
    let is_icg_entry = |entry: &Value| {
        entry
            .get("command")
            .and_then(Value::as_str)
            .map(is_icg_hook_command)
            .unwrap_or(false)
    };

    if uninstall {
        let mut emptied_events = Vec::new();
        for (event, value) in hooks.iter_mut() {
            let Some(array) = value.as_array_mut() else {
                continue;
            };
            let before = array.len();
            array.retain(|entry| !is_icg_entry(entry));
            let removed = before - array.len();
            if removed > 0 {
                report.removed_icg_entries += removed;
                if array.is_empty() {
                    emptied_events.push(event.clone());
                }
            }
        }
        for event in &emptied_events {
            hooks.remove(event);
        }
    } else {
        for plan in plans {
            let array = hooks
                .entry(plan.event.to_string())
                .or_insert_with(|| Value::Array(Vec::new()));
            let array = array
                .as_array_mut()
                .ok_or_else(|| anyhow::anyhow!("hooks.{} must be a JSON array", plan.event))?;
            let before = array.len();
            array.retain(|entry| !is_icg_entry(entry));
            report.removed_icg_entries += before - array.len();
            array.push(plan.entry_value());
            report.added_icg_entries += 1;
        }
    }

    let mut serialized = serde_json::to_string_pretty(&root)
        .map_err(|error| anyhow::anyhow!("failed to serialize the merged hooks.json: {error}"))?;
    serialized.push('\n');

    report.changed = match existing {
        Some(text) => text != serialized,
        // Uninstalling from a file that does not exist changes nothing.
        None => !uninstall,
    };

    Ok((serialized, report))
}

/// Which file the installer manages.
#[derive(Debug, Clone, Default)]
pub struct InstallOptions {
    /// Manage `~/.cursor/hooks.json` instead of the project file.
    pub user: bool,
    /// Project directory whose `.cursor/hooks.json` is managed.
    pub project_dir: Option<PathBuf>,
    /// Exact path; overrides both of the above.
    pub file: Option<PathBuf>,
    /// Rule-pack path recorded in the installed hook commands.
    pub rule_pack: Option<PathBuf>,
    /// Set `failClosed: true` on the installed entries.
    pub fail_closed: bool,
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
            return Ok(home.join(".cursor").join("hooks.json"));
        }
        let project_dir = match &self.project_dir {
            Some(dir) => dir.clone(),
            None => std::env::current_dir().context(
                "cannot determine the current directory; pass --project-dir <dir> or --file <path>",
            )?,
        };
        Ok(project_dir.join(".cursor").join("hooks.json"))
    }
}

/// Entry point for `icg install-cursor-hooks`.
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
        installed_entries(
            &icg_binary,
            options.rule_pack.as_deref(),
            options.fail_closed,
        )
    };

    let (serialized, report) = merge_hooks_json(existing.as_deref(), &plans, options.uninstall)
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

fn print_report(target: &Path, report: &MergeReport, plans: &[CursorHookPlan], uninstall: bool) {
    let state = if report.changed {
        if report.created {
            "created"
        } else {
            "updated"
        }
    } else {
        "already up to date"
    };
    println!("Cursor hooks installer");
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
            "  {}: matcher {:?}, timeout {}s: {}",
            plan.event, plan.matcher, plan.timeout, plan.command
        );
    }
    println!(
        "ICG entries: {} replaced, {} installed. Unrelated hooks, matchers and \
         keys are never touched; ICG-owned entries are recognized by their \
         command line and replaced in place, so re-running is a no-op.",
        report.removed_icg_entries, report.added_icg_entries
    );
    println!(
        "Placement: Cursor cloud agents read project-level hooks only — \
         ~/.cursor/hooks.json is not available to them. Install at the \
         project level (.cursor/hooks.json in the repository) when cloud \
         agent sessions should carry the gate; the user-level file covers \
         local IDE sessions only."
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn install_plans() -> Vec<CursorHookPlan> {
        installed_entries(Path::new("/usr/local/bin/icg"), None, false)
    }

    #[test]
    fn recognizes_icg_entries_by_their_command_line() {
        assert!(is_icg_hook_command(
            "/usr/local/bin/icg hook --harness cursor"
        ));
        assert!(is_icg_hook_command(
            "icg hook --harness cursor --event before-shell-execution"
        ));
        assert!(is_icg_hook_command(
            "'/opt/my tools/icg' hook --harness cursor --rule-pack /etc/icg/packs"
        ));

        // Not icg, not hook mode, or not the cursor harness: not ours.
        // A foreign command that merely mentions icg mid-line — with or
        // without a rule-pack tail — stays foreign.
        assert!(!is_icg_hook_command("echo icg hook --harness cursor"));
        assert!(!is_icg_hook_command(
            "echo icg hook --harness cursor --rule-pack /etc/icg/packs"
        ));
        assert!(!is_icg_hook_command("/usr/local/bin/icg hook"));
        assert!(!is_icg_hook_command(
            "/usr/local/bin/icg hook --harness claude-code"
        ));
        assert!(!is_icg_hook_command(
            "/usr/local/bin/icg hook --harness gemini-cli"
        ));
        assert!(!is_icg_hook_command("/usr/local/bin/icg coverage --list"));
        assert!(!is_icg_hook_command("icg-hook --harness cursor"));
        assert!(!is_icg_hook_command(""));
    }

    #[test]
    fn shell_quoted_output_of_the_installer_is_recognized_again() {
        // Whatever path the running binary has, the installer must
        // recognize its own output — the quoting styles
        // hook_command::shell_quote produces included — or a re-run would
        // duplicate entries.
        for path in [
            "/usr/local/bin/icg",
            "/opt/my tools/icg",
            "/opt/it's/icg",
            "'/weird leading quote/icg",
        ] {
            let plans = installed_entries(Path::new(path), None, false);
            for plan in plans {
                assert!(
                    is_icg_hook_command(&plan.command),
                    "{:?} (from path {path:?}) must read as ICG-owned",
                    plan.command
                );
            }
        }
    }

    #[test]
    fn merge_into_missing_document_creates_both_events() {
        let (serialized, report) = merge_hooks_json(None, &install_plans(), false)
            .expect("a missing document always merges");

        let root: Value = serde_json::from_str(&serialized).expect("output must be valid JSON");
        assert_eq!(root["version"], json!(1));
        assert_eq!(root["hooks"]["preToolUse"].as_array().unwrap().len(), 1);
        assert_eq!(
            root["hooks"]["beforeShellExecution"]
                .as_array()
                .unwrap()
                .len(),
            1
        );
        assert!(report.created && report.changed);
        assert_eq!(report.added_icg_entries, 2);
    }

    #[test]
    fn second_merge_is_byte_identical_and_reports_no_change() {
        let (first, _) =
            merge_hooks_json(None, &install_plans(), false).expect("first merge succeeds");
        let (second, report) =
            merge_hooks_json(Some(&first), &install_plans(), false).expect("second merge succeeds");

        assert_eq!(
            first, second,
            "re-running the installer must not change bytes"
        );
        assert!(!report.changed);
        assert_eq!(report.removed_icg_entries, 2);
        assert_eq!(report.added_icg_entries, 2);
    }

    #[test]
    fn merge_preserves_unrelated_entries_and_keys() {
        let existing = r#"{
  "version": 1,
  "hooks": {
    "preToolUse": [
      {"command": "echo mine", "matcher": "Read", "timeout": 5, "failClosed": true},
      {"command": "/usr/local/bin/icg hook --harness cursor", "matcher": "Shell", "timeout": 99}
    ],
    "afterFileEdit": [{"command": "./hooks/format.sh", "timeout": 3}]
  },
  "customTopLevel": {"keep": [1, 2, 3]}
}"#;

        let (serialized, report) =
            merge_hooks_json(Some(existing), &install_plans(), false).expect("merge succeeds");
        let root: Value = serde_json::from_str(&serialized).expect("output must be valid JSON");

        // The unrelated entry keeps every key and value verbatim.
        let pre = root["hooks"]["preToolUse"].as_array().unwrap();
        let mine = pre
            .iter()
            .find(|entry| entry["command"] == json!("echo mine"))
            .expect("unrelated hook must survive");
        assert_eq!(mine["matcher"], json!("Read"));
        assert_eq!(mine["timeout"], json!(5));
        assert_eq!(mine["failClosed"], json!(true));

        // The stale ICG entry was replaced, not duplicated.
        assert_eq!(pre.len(), 2);
        assert_eq!(
            pre.iter()
                .filter(|entry| is_icg_hook_command(entry["command"].as_str().unwrap()))
                .count(),
            1
        );

        // Other event arrays and unknown top-level keys are untouched.
        assert_eq!(
            root["hooks"]["afterFileEdit"],
            json!([{"command": "./hooks/format.sh", "timeout": 3}])
        );
        assert_eq!(root["customTopLevel"], json!({"keep": [1, 2, 3]}));
        assert_eq!(report.removed_icg_entries, 1);
    }

    #[test]
    fn merge_rejects_malformed_json_and_unsupported_versions() {
        let malformed = "{\"version\": 1, \"hooks\": {";
        let error = merge_hooks_json(Some(malformed), &install_plans(), false)
            .expect_err("malformed JSON must fail");
        assert!(
            error.to_string().contains("not valid JSON"),
            "error should say the JSON is invalid, got: {error}"
        );

        for text in [
            r#"{"version": 2, "hooks": {}}"#,
            r#"{"hooks": {"preToolUse": "not-an-array"}}"#,
            "[1, 2, 3]",
        ] {
            assert!(
                merge_hooks_json(Some(text), &install_plans(), false).is_err(),
                "expected {text} to be rejected"
            );
        }
    }

    #[test]
    fn uninstall_removes_only_icg_entries_and_arrays_it_emptied() {
        let existing = r#"{
  "version": 1,
  "hooks": {
    "preToolUse": [
      {"command": "echo mine", "matcher": "Read"},
      {"command": "icg hook --harness cursor"}
    ],
    "beforeShellExecution": [{"command": "icg hook --harness cursor --event before-shell-execution"}]
  }
}"#;

        let (serialized, report) =
            merge_hooks_json(Some(existing), &[], true).expect("uninstall succeeds");
        let root: Value = serde_json::from_str(&serialized).expect("output must be valid JSON");

        assert_eq!(report.removed_icg_entries, 2);
        assert!(report.changed);
        let pre = root["hooks"]["preToolUse"].as_array().unwrap();
        assert_eq!(pre.len(), 1);
        assert_eq!(pre[0]["command"], json!("echo mine"));
        // The array the uninstall emptied is dropped; the one with a
        // surviving foreign entry stays.
        assert!(root["hooks"].get("beforeShellExecution").is_none());
    }

    #[test]
    fn rule_pack_lands_in_the_installed_commands() {
        let plans = installed_entries(
            Path::new("/usr/local/bin/icg"),
            Some(Path::new("/etc/icg/packs")),
            true,
        );

        assert!(plans[0].command.ends_with(" --rule-pack /etc/icg/packs"));
        assert!(plans[1].command.contains("--event before-shell-execution"));
        assert!(plans[1].command.ends_with(" --rule-pack /etc/icg/packs"));
        // failClosed is opt-in and written as its own schema field.
        assert_eq!(plans[0].entry_value()["failClosed"], json!(true));
        assert_eq!(
            installed_entries(Path::new("/usr/local/bin/icg"), None, false)[0].entry_value(),
            json!({
                "command": "/usr/local/bin/icg hook --harness cursor",
                "matcher": "Shell|Write|Edit",
                "timeout": 10,
            })
        );
    }

    #[test]
    fn paths_with_spaces_are_shell_quoted_in_commands() {
        let plans = installed_entries(Path::new("/opt/my tools/icg"), None, false);
        assert!(plans[0]
            .command
            .starts_with("'/opt/my tools/icg' hook --harness cursor"));
    }
}
