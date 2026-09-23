//! Idempotent OpenCode plugin installer (`icg install-opencode-plugin`).
//!
//! The OpenCode adapter (`src/adapter.rs`, contract §6.3) is served by a
//! plugin that must be registered with OpenCode itself: one file in a
//! plugin directory glob (`{plugin,plugins}/*.{ts,js}`), both spellings
//! honored, global scope loading for every project and evaluating before
//! any project-local plugin (pinned in
//! `docs/research/opencode-1.18.29-plugin-surface.md` §10.1; the global
//! `plugin/` singular directory is the recommended channel). Hand-copying
//! the plugin file is how stale copies and accidental clobbers happen, so
//! this module ships the installer as a subcommand that deploys the plugin
//! **embedded in this binary** (`opencode-plugin/icg.ts`, baked in at
//! compile time) with idempotence guarantees:
//!
//! - The target is recognized as **ICG-owned by content**: its text
//!   carries [`PLUGIN_MARKER`]. Ownership is never judged by filename,
//!   position, or timestamps.
//! - Installing over a target that already holds exactly the embedded
//!   bytes is a no-op (`already up to date`). A target holding a
//!   *different* ICG plugin (marker present, bytes differ — a stale or
//!   hand-tweaked copy) is backed up once as `<target>.icg-backup` and
//!   replaced.
//! - A target **without** the marker is a foreign plugin: the installer
//!   refuses and leaves it untouched, in both directions — it never
//!   clobbers someone else's plugin on install, and never removes one on
//!   uninstall.
//! - Nothing else is written. OpenCode's own configuration — the
//!   `opencode.json(c)` files whose `permissions` section is OpenCode's
//!   native permission system — is never read or modified, and no
//!   `plugin` config array is ever edited: registration is one file in a
//!   plugin directory, which the installed 1.18.29 globs without any
//!   config change (surface §10.1). That is the "does not replace
//!   OpenCode's permission configuration" guarantee: the plugin ADDS a
//!   gate; OpenCode's permission prompts and rules keep working
//!   underneath it (a hook throw even pre-empts the prompt, semantics
//!   §1.2).
//!
//! Deliberately absent, unlike the Gemini installer: no matcher and no
//! timeout. The tool scoping lives inside the plugin ([`GATED_TOOLS`]),
//! and the subprocess stall cap lives there too
//! (`SPAWN_TIMEOUT_MS`) — OpenCode gives an in-process hook no
//! dispatcher-side timeout, so the plugin bounds itself.

use anyhow::{bail, Context, Result};
use std::fs;
use std::path::PathBuf;

/// The plugin file's name in every derived target directory. One file per
/// concern: directory glob order is filesystem readdir order and must never
/// be relied on (surface §10.1), so the installer deploys exactly one file
/// and never depends on where it sorts.
pub const PLUGIN_FILE_NAME: &str = "icg.ts";

/// Content marker proving a target file is the ICG plugin. It is the first
/// line of the embedded plugin; any file carrying it is installer-owned,
/// and any file without it is foreign.
pub const PLUGIN_MARKER: &str = "@icg-opencode-plugin v1";

/// The plugin deployed on install, embedded at compile time from
/// `opencode-plugin/icg.ts`. Embedding (rather than reading a path at
/// runtime) is what makes `icg install-opencode-plugin` self-contained and
/// makes the deployed bytes exactly the bytes this release was built with.
pub const EMBEDDED_PLUGIN: &str = include_str!("../opencode-plugin/icg.ts");

/// Which file the installer manages.
#[derive(Debug, Clone, Default)]
pub struct InstallOptions {
    /// Project directory whose `.opencode/plugin/icg.ts` is managed
    /// instead of the global plugin directory.
    pub project_dir: Option<PathBuf>,
    /// Exact path; overrides both of the above.
    pub file: Option<PathBuf>,
    /// Remove the ICG plugin from the target instead of installing it.
    pub uninstall: bool,
}

impl InstallOptions {
    pub fn resolve_target_path(&self) -> Result<PathBuf> {
        if let Some(file) = &self.file {
            return Ok(file.clone());
        }
        if let Some(project_dir) = &self.project_dir {
            return Ok(project_dir
                .join(".opencode")
                .join("plugin")
                .join(PLUGIN_FILE_NAME));
        }
        // Global plugin directory: <$XDG_CONFIG_HOME|~/.config>/opencode/plugin/icg.ts —
        // the same global config dir resolution OpenCode itself uses
        // (surface §10.1).
        let config_dir = match std::env::var_os("XDG_CONFIG_HOME") {
            Some(dir) if !dir.is_empty() => PathBuf::from(dir),
            _ => {
                let home = std::env::var_os("HOME").map(PathBuf::from).context(
                    "cannot determine the home directory; pass --file <path> explicitly",
                )?;
                home.join(".config")
            }
        };
        Ok(config_dir
            .join("opencode")
            .join("plugin")
            .join(PLUGIN_FILE_NAME))
    }
}

/// What an install or uninstall did, so the caller can report honestly —
/// including the no-op that touched nothing.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct InstallReport {
    pub created: bool,
    pub updated: bool,
    pub already_current: bool,
    pub backup_written: bool,
    pub removed: bool,
    pub not_installed: bool,
}

impl InstallReport {
    fn state(&self) -> &'static str {
        if self.created {
            "created"
        } else if self.updated {
            "updated"
        } else if self.removed {
            "removed"
        } else if self.already_current {
            "already up to date"
        } else if self.not_installed {
            "not installed"
        } else {
            "unchanged"
        }
    }
}

/// Entry point for `icg install-opencode-plugin`.
pub fn run_install(options: InstallOptions) -> Result<()> {
    let target = options.resolve_target_path()?;
    let report = if options.uninstall {
        uninstall(&target)?
    } else {
        install(&target)?
    };
    print_report(&target, &report, options.uninstall);
    Ok(())
}

fn install(target: &std::path::Path) -> Result<InstallReport> {
    let mut report = InstallReport::default();

    if target.exists() {
        let existing = fs::read_to_string(target)
            .with_context(|| format!("failed to read {}", target.display()))?;
        if !existing.contains(PLUGIN_MARKER) {
            bail!(
                "refusing to replace {}; it exists and carries no ICG plugin marker \
                 ({}), so it is a foreign plugin — pass --file <path> to manage a \
                 different file instead",
                target.display(),
                PLUGIN_MARKER,
            );
        }
        if existing == EMBEDDED_PLUGIN {
            report.already_current = true;
            return Ok(report);
        }
        write_backup_once(target, &existing)?;
        report.backup_written = true;
        fs::write(target, EMBEDDED_PLUGIN)
            .with_context(|| format!("failed to write {}", target.display()))?;
        report.updated = true;
        return Ok(report);
    }

    if let Some(parent) = target.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent)
                .with_context(|| format!("failed to create {}", parent.display()))?;
        }
    }
    fs::write(target, EMBEDDED_PLUGIN)
        .with_context(|| format!("failed to write {}", target.display()))?;
    report.created = true;
    Ok(report)
}

fn uninstall(target: &std::path::Path) -> Result<InstallReport> {
    let mut report = InstallReport::default();
    if !target.exists() {
        report.not_installed = true;
        return Ok(report);
    }
    let existing = fs::read_to_string(target)
        .with_context(|| format!("failed to read {}", target.display()))?;
    if !existing.contains(PLUGIN_MARKER) {
        bail!(
            "refusing to remove {}; it carries no ICG plugin marker ({}) and may be a \
             foreign plugin — remove it manually if it is really ours",
            target.display(),
            PLUGIN_MARKER,
        );
    }
    fs::remove_file(target).with_context(|| format!("failed to remove {}", target.display()))?;
    report.removed = true;
    Ok(report)
}

/// Back up the pre-installer state, once. The backup always holds the
/// pre-installer bytes (written only when absent) and is never touched by a
/// no-op run.
fn write_backup_once(target: &std::path::Path, existing: &str) -> Result<()> {
    // Built from the OsString, not `format!("{}", target.display())`: a
    // non-UTF-8 path must survive round-trip exactly, and `display()` is
    // lossy by design.
    let mut backup = target.as_os_str().to_os_string();
    backup.push(".icg-backup");
    let backup_path = PathBuf::from(backup);
    if backup_path.exists() {
        return Ok(());
    }
    fs::write(&backup_path, existing)
        .with_context(|| format!("failed to write backup {}", backup_path.display()))
}

fn print_report(target: &std::path::Path, report: &InstallReport, uninstall: bool) {
    println!("OpenCode plugin installer");
    println!("Target: {} ({})", target.display(), report.state());
    if uninstall {
        if report.removed {
            println!("Removed the ICG plugin.");
        } else {
            println!("Nothing removed.");
        }
        return;
    }
    if report.backup_written {
        println!("Prior ICG plugin saved as {}.icg-backup", target.display());
    }
    if report.created || report.updated {
        println!(
            "Gate scope inside the plugin: {} (everything else fails open without spawning icg)",
            ["bash", "write", "edit", "apply_patch"].join(", ")
        );
        println!(
            "Residual risk: `opencode --pure` (or OPENCODE_PURE=1) skips every external \
             plugin, this gate included; the PATH-wrapper layer is the backstop."
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embedded_plugin_carries_the_marker_and_pinned_binary_path() {
        assert!(
            EMBEDDED_PLUGIN.contains(PLUGIN_MARKER),
            "the embedded plugin must carry the installer's ownership marker"
        );
        assert!(
            EMBEDDED_PLUGIN.contains("/usr/local/bin/icg"),
            "the plugin must invoke the root-owned absolute icg path"
        );
        assert!(
            EMBEDDED_PLUGIN.contains("\"tool.execute.before\""),
            "the plugin must register the pinned 1.18.29 gate hook"
        );
        assert!(
            EMBEDDED_PLUGIN.contains("\"hook\", \"--harness\", \"open-code\""),
            "the adapter invocation must use the spelling every shipped icg \
             accepts (0.1.62 takes only the derived kebab-case; later builds \
             keep it as an alias)"
        );
    }
}
