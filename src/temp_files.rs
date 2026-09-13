//! Shared pieces of the atomic state-file write pattern used by the writable
//! telemetry cache (`/var/cache/icg`).
//!
//! [`crate::health::HealthStore`], [`crate::state_store::StateStore`], and
//! [`crate::telemetry::TelemetryStore`] all persist by writing a pid-suffixed
//! temp file and renaming it over the live file while holding an inter-process
//! lock. A writer that dies between creating its temp file and the rename
//! leaves that temp file behind forever; this module holds the lock guard and
//! the reclamation sweep those stores share.
//!
//! ## Why reclamation is safe only under the lock
//!
//! Temp files are named `.{file}.tmp-<pid>` and are only ever created and
//! renamed while the writer holds the flock for the matching state file. A
//! caller holding that lock therefore knows any foreign
//! `.{file}.tmp-<pid>` it observes belongs to a writer that will never
//! rename it: a live writer would be holding the lock itself. Reclaiming
//! such a file without holding the lock would race a live writer's
//! rename, so [`reclaim_orphaned_temp_files`] must only be called from code
//! that holds the lock.

use anyhow::{Context, Result};
use std::fs::{File, OpenOptions};
use std::path::{Path, PathBuf};

/// An exclusive inter-process lock held for the duration of one state write.
pub struct WriteLock {
    _file: File,
}

impl WriteLock {
    /// Open (creating if needed) `path` and take an exclusive `flock` on it.
    ///
    /// Kernel `flock` locks are released when the owning process exits —
    /// including after a crash — so a dead writer cannot wedge the store for
    /// the writers that come after it.
    pub fn acquire(path: &Path) -> Result<Self> {
        let file = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(path)
            .with_context(|| format!("Failed to open lock file {}", path.display()))?;

        #[cfg(unix)]
        {
            let result =
                unsafe { libc::flock(std::os::unix::io::AsRawFd::as_raw_fd(&file), libc::LOCK_EX) };
            if result != 0 {
                return Err(std::io::Error::last_os_error())
                    .with_context(|| format!("Failed to acquire lock on {}", path.display()));
            }
        }

        Ok(Self { _file: file })
    }
}

/// The temp-file name prefix every persist of `file_name` uses.
pub fn temp_prefix(file_name: &str) -> String {
    format!(".{file_name}.tmp-")
}

/// The temp path a persist of `file_name` in `dir` by `pid` uses.
pub fn temp_path(dir: &Path, file_name: &str, pid: u32) -> PathBuf {
    dir.join(format!("{}{pid}", temp_prefix(file_name)))
}

/// Remove temp files orphaned by writers that died mid-persist.
///
/// Scans `dir` for entries shaped `{prefix}<pid>` and removes every one whose
/// pid is not this process's own. Entries whose suffix does not parse as a
/// pid are never touched, so nothing that merely resembles a temp file is at
/// risk. Individual removals are best-effort: the count of removed files is
/// returned and failures are ignored, because a failed cleanup must never
/// fail the persist that ran it.
///
/// The caller must hold the write lock for the state file this prefix
/// belongs to — see the module documentation for why.
pub fn reclaim_orphaned_temp_files(dir: &Path, prefix: &str) -> usize {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return 0;
    };
    let own_pid = std::process::id();
    let mut reclaimed = 0;
    for entry in entries.flatten() {
        let file_name = entry.file_name();
        let lossy = file_name.to_string_lossy();
        let Some(suffix) = lossy.strip_prefix(prefix) else {
            continue;
        };
        let Ok(pid) = suffix.parse::<u32>() else {
            continue;
        };
        if pid == own_pid {
            // Our own in-flight temp, if a concurrent thread in this process
            // is mid-persist. It is renamed or removed by its own write.
            continue;
        }
        if std::fs::remove_file(entry.path()).is_ok() {
            reclaimed += 1;
        }
    }
    reclaimed
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write as _;
    use tempfile::tempdir;

    fn write_file(path: &Path, contents: &str) {
        let mut file = File::create(path).expect("create");
        file.write_all(contents.as_bytes()).expect("write");
    }

    #[test]
    fn temp_path_is_dot_prefixed_and_pid_suffixed() {
        let dir = Path::new("/var/cache/icg");
        assert_eq!(
            temp_path(dir, "health-state.json", 4242),
            dir.join(".health-state.json.tmp-4242")
        );
    }

    #[test]
    fn reclamation_removes_foreign_pids_and_keeps_everything_else() -> Result<()> {
        let dir = tempdir()?;
        let prefix = ".state.json.tmp-";
        write_file(&dir.path().join(".state.json.tmp-999999"), "orphan");
        write_file(&dir.path().join(".state.json.tmp-backup"), "not a pid");
        write_file(&dir.path().join("state.json.tmp-999999"), "wrong shape");
        write_file(&dir.path().join("state.json"), "the live file");

        let reclaimed = reclaim_orphaned_temp_files(dir.path(), prefix);

        assert_eq!(reclaimed, 1);
        assert!(!dir.path().join(".state.json.tmp-999999").exists());
        assert!(dir.path().join(".state.json.tmp-backup").exists());
        assert!(dir.path().join("state.json.tmp-999999").exists());
        assert!(dir.path().join("state.json").exists());
        Ok(())
    }

    #[test]
    fn reclamation_skips_the_current_process_own_temp() -> Result<()> {
        let dir = tempdir()?;
        let own = temp_path(dir.path(), "state.json", std::process::id());
        write_file(&own, "in flight");

        let reclaimed = reclaim_orphaned_temp_files(dir.path(), ".state.json.tmp-");

        assert_eq!(reclaimed, 0);
        assert!(own.exists());
        std::fs::remove_file(&own)?;
        Ok(())
    }

    #[test]
    fn reclamation_tolerates_a_missing_directory() {
        assert_eq!(
            reclaim_orphaned_temp_files(Path::new("/nonexistent-dir-icg"), ".x.tmp-"),
            0
        );
    }
}
