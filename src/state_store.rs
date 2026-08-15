//! Durable runtime state for cross-invocation enforcement and rollback.
//!
//! The state store is deliberately separate from [`crate::trust_pointer`].
//! The trust-pointer file is a root-owned security boundary and remains the
//! source of truth for what a host may load. This store records the runtime
//! history needed by later enforcement components: session ordering markers,
//! deny events, the previous trust reference, and rollback metadata.
//!
//! The on-disk format is a versioned JSON document. JSON is sufficient here:
//! the state is small, locally hosted, and does not need query-oriented
//! storage. Updates take an advisory process lock, write a fully synced
//! temporary file, atomically replace the state file, and sync its parent
//! directory. A crashed writer therefore leaves either the old complete
//! document or the new complete document, never a partially written one.

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use crate::trust_pointer::TrustPointer;

/// Current on-disk state schema version.
pub const STATE_SCHEMA_VERSION: u32 = 1;

/// Maximum number of deny records retained by a default store.
///
/// The history is diagnostic/telemetry state, not an unbounded audit log.
/// Keeping a finite tail prevents a busy host from filling its filesystem.
pub const DEFAULT_MAX_DENY_HISTORY: usize = 10_000;

/// Trust-pointer information recorded in runtime state.
///
/// `previous_trusted_ref` is retained so a rollback component can identify
/// the last pointer without scraping or rewriting the root-owned pointer
/// file. It is intentionally only a record; changing the authoritative
/// pointer still belongs to [`crate::trust_pointer::TrustPointerStore`].
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TrustPointerState {
    /// The currently recorded trusted release reference.
    pub trusted_ref: String,

    /// Timestamp supplied by the trust-pointer update.
    #[serde(default)]
    pub updated_at: String,

    /// Optional explanation for the trust decision.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub justification: Option<String>,

    /// The reference that was current before the most recent change.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub previous_trusted_ref: Option<String>,
}

impl TrustPointerState {
    fn from_pointer(pointer: &TrustPointer, previous_trusted_ref: Option<String>) -> Self {
        Self {
            trusted_ref: pointer.trusted_ref.clone(),
            updated_at: pointer.updated_at.clone(),
            justification: pointer.justification.clone(),
            previous_trusted_ref,
        }
    }

    /// Convert the recorded pointer back to the public trust-pointer model.
    pub fn to_trust_pointer(&self) -> TrustPointer {
        TrustPointer {
            trusted_ref: self.trusted_ref.clone(),
            updated_at: self.updated_at.clone(),
            justification: self.justification.clone(),
        }
    }
}

/// One denied command recorded by the guard.
///
/// The command and reason are optional so callers can record denials from
/// both command and content front-ends. `StateStore::record_deny` fills in
/// `session_id` when the caller leaves it empty.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DenyHistoryEntry {
    /// Stable identifier for this event within the local store.
    pub id: String,

    /// UTC timestamp at which the denial was observed.
    pub timestamp: String,

    /// Session in which the denial occurred.
    #[serde(default)]
    pub session_id: String,

    /// Release reference loaded when the denial occurred, if known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub release_ref: Option<String>,

    /// Rule or pattern identifier that caused the denial, if known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rule_id: Option<String>,

    /// Original command text, if this was a command-mode denial.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,

    /// Human-readable denial reason.
    pub reason: String,
}

impl DenyHistoryEntry {
    /// Create a denial event with the current UTC timestamp.
    pub fn new(
        release_ref: Option<impl Into<String>>,
        rule_id: Option<impl Into<String>>,
        command: Option<impl Into<String>>,
        reason: impl Into<String>,
    ) -> Self {
        let timestamp = chrono::Utc::now().to_rfc3339();
        let id = format!(
            "{}-{}",
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default(),
            std::process::id()
        );

        Self {
            id,
            timestamp,
            session_id: String::new(),
            release_ref: release_ref.map(Into::into),
            rule_id: rule_id.map(Into::into),
            command: command.map(Into::into),
            reason: reason.into(),
        }
    }
}

/// Alias for callers that use the shorter event terminology.
pub type DenyEvent = DenyHistoryEntry;

/// Alias for callers that use record terminology.
pub type DenyRecord = DenyHistoryEntry;

/// Persistent metadata about rollback activity.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RollbackState {
    /// Release currently recorded after the rollback decision, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub current_release: Option<String>,

    /// Release that was active immediately before the last rollback.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub previous_release: Option<String>,

    /// UTC timestamp of the last rollback decision.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_rollback_at: Option<String>,

    /// Reason supplied for the last rollback decision.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_rollback_reason: Option<String>,

    /// Number of rollback decisions recorded in this state file.
    #[serde(default)]
    pub rollback_count: u64,
}

impl RollbackState {
    /// Return a state with no rollback recorded.
    pub fn new() -> Self {
        Self::default()
    }

    /// Clear the current rollback details while retaining the counter.
    pub fn clear_last_rollback(&mut self) {
        self.current_release = None;
        self.previous_release = None;
        self.last_rollback_at = None;
        self.last_rollback_reason = None;
    }
}

impl Default for RollbackState {
    fn default() -> Self {
        Self {
            current_release: None,
            previous_release: None,
            last_rollback_at: None,
            last_rollback_reason: None,
            rollback_count: 0,
        }
    }
}

/// Session and production runtime state.
///
/// Older state files containing only `session_id`, `git_pull_timestamp`, and
/// `flush_timestamp` deserialize successfully. Missing production fields are
/// populated with safe empty defaults.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionState {
    /// Unique identifier for this session.
    #[serde(default)]
    pub session_id: String,

    /// Timestamp when git pull was last executed in this session.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub git_pull_timestamp: Option<String>,

    /// Timestamp when flush was last executed in this session.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub flush_timestamp: Option<String>,

    /// Schema version of this state document.
    #[serde(default = "default_schema_version")]
    pub schema_version: u32,

    /// Last trust pointer recorded by the runtime.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trust_pointer: Option<TrustPointerState>,

    /// Tail of denied-command history.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub deny_history: Vec<DenyHistoryEntry>,

    /// Rollback metadata for the current host.
    #[serde(default)]
    pub rollback: RollbackState,
}

fn default_schema_version() -> u32 {
    STATE_SCHEMA_VERSION
}

impl SessionState {
    /// Create an empty state document with a fresh session ID.
    pub fn new() -> Self {
        Self {
            session_id: Self::generate_session_id(),
            git_pull_timestamp: None,
            flush_timestamp: None,
            schema_version: STATE_SCHEMA_VERSION,
            trust_pointer: None,
            deny_history: Vec::new(),
            rollback: RollbackState::default(),
        }
    }

    /// Generate a unique session identifier.
    fn generate_session_id() -> String {
        let timestamp = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Nanos, true);
        let pid = std::process::id();
        format!("{}-{}", timestamp.replace(':', "-"), pid)
    }

    /// Mark that git pull has occurred in this session.
    pub fn mark_git_pull(&mut self) {
        self.git_pull_timestamp = Some(chrono::Utc::now().to_rfc3339());
    }

    /// Mark that flush has occurred in this session.
    pub fn mark_flush(&mut self) {
        self.flush_timestamp = Some(chrono::Utc::now().to_rfc3339());
    }

    /// Check if git pull has occurred in this session.
    pub fn has_git_pull(&self) -> bool {
        self.git_pull_timestamp.is_some()
    }

    /// Check if flush has occurred in this session.
    pub fn has_flush(&self) -> bool {
        self.flush_timestamp.is_some()
    }

    /// Record a trust-pointer update while retaining the previous reference.
    pub fn set_trust_pointer(&mut self, pointer: &TrustPointer) {
        let previous_trusted_ref = self.trust_pointer.as_ref().and_then(|current| {
            if current.trusted_ref == pointer.trusted_ref {
                current.previous_trusted_ref.clone()
            } else {
                Some(current.trusted_ref.clone())
            }
        });

        self.trust_pointer = Some(TrustPointerState::from_pointer(
            pointer,
            previous_trusted_ref,
        ));
    }

    /// Return the recorded trust pointer, if one has been stored.
    pub fn trust_pointer(&self) -> Option<TrustPointer> {
        self.trust_pointer
            .as_ref()
            .map(TrustPointerState::to_trust_pointer)
    }

    /// Record one deny event.
    pub fn record_deny(&mut self, entry: DenyHistoryEntry) {
        self.deny_history.push(entry);
    }

    /// Set rollback metadata exactly, useful when restoring a state snapshot.
    pub fn set_rollback(&mut self, rollback: RollbackState) {
        self.rollback = rollback;
    }

    /// Record a rollback decision.
    pub fn record_rollback(
        &mut self,
        from_release: impl Into<String>,
        to_release: impl Into<String>,
        reason: impl Into<String>,
    ) {
        self.rollback.current_release = Some(to_release.into());
        self.rollback.previous_release = Some(from_release.into());
        self.rollback.last_rollback_at = Some(chrono::Utc::now().to_rfc3339());
        self.rollback.last_rollback_reason = Some(reason.into());
        self.rollback.rollback_count = self.rollback.rollback_count.saturating_add(1);
    }

    /// Clear the session's ordering markers and start a fresh session.
    ///
    /// Trust-pointer history, deny history, and rollback metadata are host
    /// state and intentionally survive a new agent session.
    pub fn clear(&mut self) {
        self.session_id = Self::generate_session_id();
        self.git_pull_timestamp = None;
        self.flush_timestamp = None;
    }
}

impl Default for SessionState {
    fn default() -> Self {
        Self::new()
    }
}

/// State store manager.
pub struct StateStore {
    /// Path to the state store file.
    path: PathBuf,
    /// Maximum number of deny records retained by this store.
    max_deny_history: usize,
}

impl StateStore {
    /// Create a state store using the default deny-history retention limit.
    pub fn new(path: impl AsRef<Path>) -> Self {
        Self {
            path: path.as_ref().to_path_buf(),
            max_deny_history: DEFAULT_MAX_DENY_HISTORY,
        }
    }

    /// Create a state store with an explicit deny-history retention limit.
    pub fn with_max_deny_history(path: impl AsRef<Path>, max_deny_history: usize) -> Self {
        Self {
            path: path.as_ref().to_path_buf(),
            max_deny_history: max_deny_history.max(1),
        }
    }

    /// Return the path used by this store.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Return the configured deny-history retention limit.
    pub fn max_deny_history(&self) -> usize {
        self.max_deny_history
    }

    /// Get the default state-store path.
    pub fn default_path() -> Result<PathBuf> {
        let cache_dir =
            dirs::cache_dir().context("Failed to determine platform cache directory")?;
        Ok(cache_dir.join("icg").join("session-state.json"))
    }

    fn parent_dir(&self) -> &Path {
        self.path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."))
    }

    /// Ensure the state directory exists.
    fn ensure_parent_dir(&self) -> Result<()> {
        let parent = self.parent_dir();
        if !parent.exists() {
            fs::create_dir_all(parent)
                .with_context(|| format!("Failed to create directory: {}", parent.display()))?;
        }
        Ok(())
    }

    fn lock_path(&self) -> PathBuf {
        self.path.with_extension("lock")
    }

    /// Acquire an inter-process lock for a read/modify/write transaction.
    ///
    /// On Unix, `flock` locks are released by the kernel if the process exits,
    /// including after a crash, so a stale lock file cannot wedge the store.
    fn acquire_lock(&self) -> Result<StateLock> {
        self.ensure_parent_dir()?;
        let lock_file = OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .open(self.lock_path())
            .context("Failed to open state-store lock file")?;

        platform_lock(&lock_file).context("Failed to acquire state-store lock")?;
        Ok(StateLock { _file: lock_file })
    }

    /// Load the current state.
    ///
    /// A missing file means an empty state. Reads do not need to hold the
    /// lock because writers replace the file atomically.
    pub fn load(&self) -> Result<SessionState> {
        self.ensure_parent_dir()?;
        if !self.path.exists() {
            return Ok(SessionState::new());
        }

        let content = fs::read_to_string(&self.path)
            .with_context(|| format!("Failed to read state from {}", self.path.display()))?;
        let mut state: SessionState = serde_json::from_str(&content)
            .with_context(|| format!("Failed to parse state from {}", self.path.display()))?;
        self.validate_and_migrate(&mut state)?;
        Ok(state)
    }

    fn validate_and_migrate(&self, state: &mut SessionState) -> Result<()> {
        if state.schema_version > STATE_SCHEMA_VERSION {
            bail!(
                "State store schema version {} is newer than supported version {}",
                state.schema_version,
                STATE_SCHEMA_VERSION
            );
        }

        // Files written by the original session-only store had no schema
        // field. Serde supplies the current default for those files, while
        // this branch handles explicitly written version-zero documents.
        if state.schema_version == 0 {
            state.schema_version = STATE_SCHEMA_VERSION;
        }
        if state.session_id.is_empty() {
            state.session_id = SessionState::generate_session_id();
        }
        Ok(())
    }

    fn load_unlocked(&self) -> Result<SessionState> {
        if !self.path.exists() {
            return Ok(SessionState::new());
        }
        let content = fs::read_to_string(&self.path)
            .with_context(|| format!("Failed to read state from {}", self.path.display()))?;
        let mut state: SessionState = serde_json::from_str(&content)
            .with_context(|| format!("Failed to parse state from {}", self.path.display()))?;
        self.validate_and_migrate(&mut state)?;
        Ok(state)
    }

    /// Save state using an atomic, durable replacement.
    pub fn save(&self, state: &SessionState) -> Result<()> {
        let _lock = self.acquire_lock()?;
        self.save_unlocked(state)
    }

    fn save_unlocked(&self, state: &SessionState) -> Result<()> {
        self.ensure_parent_dir()?;
        let mut state = state.clone();
        self.validate_and_migrate(&mut state)?;
        state.schema_version = STATE_SCHEMA_VERSION;

        let content = serde_json::to_vec_pretty(&state).context("Failed to serialize state")?;
        let file_name = self
            .path
            .file_name()
            .context("State path has no file name")?
            .to_string_lossy();
        let temp_path = self
            .parent_dir()
            .join(format!(".{file_name}.tmp-{}", std::process::id()));

        let mut temp = OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .open(&temp_path)
            .with_context(|| {
                format!(
                    "Failed to open temporary state file {}",
                    temp_path.display()
                )
            })?;

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            temp.set_permissions(fs::Permissions::from_mode(0o600))
                .context("Failed to set state-file permissions")?;
        }

        temp.write_all(&content).with_context(|| {
            format!(
                "Failed to write temporary state file {}",
                temp_path.display()
            )
        })?;
        temp.sync_all().with_context(|| {
            format!(
                "Failed to sync temporary state file {}",
                temp_path.display()
            )
        })?;
        drop(temp);

        fs::rename(&temp_path, &self.path).with_context(|| {
            format!(
                "Failed to replace state file {} with {}",
                self.path.display(),
                temp_path.display()
            )
        })?;

        sync_parent_dir(self.parent_dir())?;
        Ok(())
    }

    /// Update state under one inter-process transaction.
    pub fn update<F>(&self, f: F) -> Result<SessionState>
    where
        F: FnOnce(&mut SessionState),
    {
        let _lock = self.acquire_lock()?;
        let mut state = self.load_unlocked()?;
        f(&mut state);
        self.save_unlocked(&state)?;
        Ok(state)
    }

    /// Check if git pull has occurred in the current session.
    pub fn has_git_pull(&self) -> Result<bool> {
        Ok(self.load()?.has_git_pull())
    }

    /// Check if flush has occurred in the current session.
    pub fn has_flush(&self) -> Result<bool> {
        Ok(self.load()?.has_flush())
    }

    /// Mark that git pull has occurred in this session.
    pub fn mark_git_pull(&self) -> Result<()> {
        self.update(SessionState::mark_git_pull)?;
        Ok(())
    }

    /// Mark that flush has occurred in this session.
    pub fn mark_flush(&self) -> Result<()> {
        self.update(SessionState::mark_flush)?;
        Ok(())
    }

    /// Start a new session without clearing host-wide history.
    pub fn new_session(&self) -> Result<()> {
        self.update(SessionState::clear)?;
        Ok(())
    }

    /// Persist a trust-pointer observation.
    pub fn save_trust_pointer(&self, pointer: &TrustPointer) -> Result<()> {
        self.update(|state| state.set_trust_pointer(pointer))?;
        Ok(())
    }

    /// Alias for `save_trust_pointer`.
    pub fn set_trust_pointer(&self, pointer: &TrustPointer) -> Result<()> {
        self.save_trust_pointer(pointer)
    }

    /// Load the last trust-pointer observation.
    pub fn load_trust_pointer(&self) -> Result<Option<TrustPointer>> {
        Ok(self.load()?.trust_pointer())
    }

    /// Return the previous trust reference retained by the store.
    pub fn previous_trusted_ref(&self) -> Result<Option<String>> {
        Ok(self
            .load()?
            .trust_pointer
            .and_then(|pointer| pointer.previous_trusted_ref))
    }

    /// Record one deny event and apply the configured retention limit.
    pub fn record_deny(&self, mut entry: DenyHistoryEntry) -> Result<()> {
        self.update(|state| {
            if entry.session_id.is_empty() {
                entry.session_id = state.session_id.clone();
            }
            state.record_deny(entry);
            let keep_from = state
                .deny_history
                .len()
                .saturating_sub(self.max_deny_history);
            if keep_from > 0 {
                state.deny_history.drain(..keep_from);
            }
        })?;
        Ok(())
    }

    /// Convenience method for recording a denial without constructing an event.
    pub fn record_denial(
        &self,
        release_ref: Option<&str>,
        rule_id: Option<&str>,
        command: Option<&str>,
        reason: &str,
    ) -> Result<()> {
        self.record_deny(DenyHistoryEntry::new(
            release_ref.map(str::to_owned),
            rule_id.map(str::to_owned),
            command.map(str::to_owned),
            reason,
        ))
    }

    /// Return the retained deny history in chronological order.
    pub fn deny_history(&self) -> Result<Vec<DenyHistoryEntry>> {
        Ok(self.load()?.deny_history)
    }

    /// Return the number of denials observed since a UTC instant.
    pub fn deny_count_since(&self, since: chrono::DateTime<chrono::Utc>) -> Result<usize> {
        Ok(self
            .deny_history()?
            .into_iter()
            .filter(|entry| {
                entry
                    .timestamp
                    .parse::<chrono::DateTime<chrono::Utc>>()
                    .map(|timestamp| timestamp >= since)
                    .unwrap_or(false)
            })
            .count())
    }

    /// Return the number of denials for a release since a UTC instant.
    pub fn deny_count_for_release_since(
        &self,
        release_ref: &str,
        since: chrono::DateTime<chrono::Utc>,
    ) -> Result<usize> {
        Ok(self
            .deny_history()?
            .into_iter()
            .filter(|entry| entry.release_ref.as_deref() == Some(release_ref))
            .filter(|entry| {
                entry
                    .timestamp
                    .parse::<chrono::DateTime<chrono::Utc>>()
                    .map(|timestamp| timestamp >= since)
                    .unwrap_or(false)
            })
            .count())
    }

    /// Replace rollback metadata.
    pub fn set_rollback_state(&self, rollback: RollbackState) -> Result<()> {
        self.update(|state| state.set_rollback(rollback))?;
        Ok(())
    }

    /// Record a rollback decision and return the resulting state.
    pub fn record_rollback(
        &self,
        from_release: impl Into<String>,
        to_release: impl Into<String>,
        reason: impl Into<String>,
    ) -> Result<RollbackState> {
        let state = self.update(|state| state.record_rollback(from_release, to_release, reason))?;
        Ok(state.rollback)
    }

    /// Load rollback metadata.
    pub fn rollback_state(&self) -> Result<RollbackState> {
        Ok(self.load()?.rollback)
    }

    /// Clear the last rollback details while retaining the rollback counter.
    pub fn clear_last_rollback(&self) -> Result<()> {
        self.update(|state| state.rollback.clear_last_rollback())?;
        Ok(())
    }
}

struct StateLock {
    _file: File,
}

#[cfg(unix)]
fn platform_lock(file: &File) -> io::Result<()> {
    use std::os::raw::c_int;
    use std::os::unix::io::AsRawFd;

    unsafe extern "C" {
        fn flock(fd: c_int, operation: c_int) -> c_int;
    }

    const LOCK_EX: c_int = 2;
    if unsafe { flock(file.as_raw_fd(), LOCK_EX) } == -1 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

#[cfg(not(unix))]
fn platform_lock(_file: &File) -> io::Result<()> {
    // The deployment target is Unix (the trust-pointer implementation uses
    // Unix ownership/mode checks). Keep non-Unix builds functional; atomic
    // replacement still provides crash consistency there.
    Ok(())
}

#[cfg(unix)]
fn sync_parent_dir(parent: &Path) -> Result<()> {
    File::open(parent)
        .with_context(|| {
            format!(
                "Failed to open state directory {} for syncing",
                parent.display()
            )
        })?
        .sync_all()
        .with_context(|| format!("Failed to sync state directory {}", parent.display()))?;
    Ok(())
}

#[cfg(not(unix))]
fn sync_parent_dir(_parent: &Path) -> Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use tempfile::tempdir;

    fn test_store(dir: &Path) -> StateStore {
        StateStore::new(dir.join("session-state.json"))
    }

    #[test]
    fn session_state_new_is_empty() {
        let state = SessionState::new();
        assert!(!state.session_id.is_empty());
        assert_eq!(state.schema_version, STATE_SCHEMA_VERSION);
        assert!(state.git_pull_timestamp.is_none());
        assert!(state.flush_timestamp.is_none());
        assert!(state.trust_pointer.is_none());
        assert!(state.deny_history.is_empty());
        assert_eq!(state.rollback, RollbackState::default());
    }

    #[test]
    fn session_markers_and_new_session_preserve_host_history() -> Result<()> {
        let dir = tempdir()?;
        let store = test_store(dir.path());
        store.save_trust_pointer(&TrustPointer::new("v1.0.0"))?;
        store.record_denial(Some("v1.0.0"), Some("rule-1"), None, "blocked")?;
        store.record_rollback("v1.0.0", "v0.9.0", "deny-rate spike")?;
        store.mark_git_pull()?;
        store.mark_flush()?;

        store.new_session()?;
        let state = store.load()?;
        assert!(!state.has_git_pull());
        assert!(!state.has_flush());
        assert!(state.trust_pointer.is_some());
        assert_eq!(state.deny_history.len(), 1);
        assert_eq!(state.rollback.rollback_count, 1);
        Ok(())
    }

    #[test]
    fn trust_pointer_retains_previous_reference() -> Result<()> {
        let dir = tempdir()?;
        let store = test_store(dir.path());
        store.save_trust_pointer(&TrustPointer::new("v1.0.0"))?;
        store.save_trust_pointer(&TrustPointer::with_justification(
            "v1.1.0",
            "release gate passed",
        ))?;

        let state = store.load()?;
        let pointer = state.trust_pointer.as_ref().expect("pointer recorded");
        assert_eq!(pointer.trusted_ref, "v1.1.0");
        assert_eq!(pointer.previous_trusted_ref.as_deref(), Some("v1.0.0"));
        assert_eq!(store.previous_trusted_ref()?.as_deref(), Some("v1.0.0"));
        assert_eq!(store.load_trust_pointer()?.unwrap().trusted_ref, "v1.1.0");
        Ok(())
    }

    #[test]
    fn deny_history_is_bounded_and_persistent() -> Result<()> {
        let dir = tempdir()?;
        let path = dir.path().join("state.json");
        let store = StateStore::with_max_deny_history(&path, 2);
        for index in 0..3 {
            store.record_deny(DenyHistoryEntry::new(
                Some("v1"),
                Some(format!("rule-{index}")),
                None::<String>,
                "blocked",
            ))?;
        }

        let reopened = StateStore::with_max_deny_history(&path, 2);
        let history = reopened.deny_history()?;
        assert_eq!(history.len(), 2);
        assert_eq!(history[0].rule_id.as_deref(), Some("rule-1"));
        assert_eq!(history[1].rule_id.as_deref(), Some("rule-2"));
        assert_eq!(history[0].session_id, reopened.load()?.session_id);
        Ok(())
    }

    #[test]
    fn rollback_state_round_trips() -> Result<()> {
        let dir = tempdir()?;
        let store = test_store(dir.path());
        let rollback = store.record_rollback("v2.0.0", "v1.9.0", "regression detected")?;
        assert_eq!(rollback.current_release.as_deref(), Some("v1.9.0"));
        assert_eq!(rollback.previous_release.as_deref(), Some("v2.0.0"));
        assert_eq!(rollback.rollback_count, 1);
        assert_eq!(
            store.rollback_state()?.last_rollback_reason.as_deref(),
            Some("regression detected")
        );
        store.clear_last_rollback()?;
        let cleared = store.rollback_state()?;
        assert_eq!(cleared.rollback_count, 1);
        assert!(cleared.last_rollback_at.is_none());
        Ok(())
    }

    #[test]
    fn concurrent_updates_are_not_lost() -> Result<()> {
        let dir = tempdir()?;
        let store = Arc::new(test_store(dir.path()));
        let mut workers = Vec::new();
        for index in 0..8 {
            let store = Arc::clone(&store);
            workers.push(std::thread::spawn(move || {
                store.record_deny(DenyHistoryEntry::new(
                    Some("v1"),
                    Some(format!("rule-{index}")),
                    None::<String>,
                    "blocked",
                ))
            }));
        }
        for worker in workers {
            worker.join().expect("worker panicked")?;
        }
        assert_eq!(store.deny_history()?.len(), 8);
        Ok(())
    }

    #[test]
    fn old_session_only_document_migrates() -> Result<()> {
        let dir = tempdir()?;
        let path = dir.path().join("session-state.json");
        fs::write(
            &path,
            r#"{
                "session_id": "old-session",
                "git_pull_timestamp": "2026-08-15T00:00:00Z",
                "flush_timestamp": null
            }"#,
        )?;
        let state = StateStore::new(&path).load()?;
        assert_eq!(state.session_id, "old-session");
        assert!(state.has_git_pull());
        assert_eq!(state.schema_version, STATE_SCHEMA_VERSION);
        assert!(state.deny_history.is_empty());
        assert_eq!(state.rollback, RollbackState::default());
        Ok(())
    }

    #[test]
    fn future_schema_is_rejected() -> Result<()> {
        let dir = tempdir()?;
        let path = dir.path().join("session-state.json");
        fs::write(&path, r#"{"schema_version": 999}"#)?;
        let error = StateStore::new(&path)
            .load()
            .expect_err("future schema must fail");
        assert!(error.to_string().contains("newer than supported"));
        Ok(())
    }

    #[test]
    fn saved_document_is_valid_json_after_replacement() -> Result<()> {
        let dir = tempdir()?;
        let store = test_store(dir.path());
        store.mark_git_pull()?;
        let content = fs::read_to_string(store.path())?;
        let parsed: SessionState = serde_json::from_str(&content)?;
        assert!(parsed.has_git_pull());
        assert!(!store.path().with_extension("tmp").exists());
        Ok(())
    }

    #[test]
    fn default_path_points_into_icg_cache() {
        let path = StateStore::default_path().unwrap();
        assert!(
            path.ends_with("icg/session-state.json") || path.ends_with("icg\\session-state.json")
        );
    }

    #[test]
    fn deny_event_has_timestamp_and_id() {
        let event =
            DenyHistoryEntry::new(None::<String>, None::<String>, None::<String>, "blocked");
        assert!(!event.id.is_empty());
        assert!(!event.timestamp.is_empty());
    }

    #[test]
    fn deny_count_can_be_filtered_by_release() -> Result<()> {
        let dir = tempdir()?;
        let store = test_store(dir.path());
        let since = chrono::Utc::now() - chrono::Duration::seconds(1);
        store.record_denial(Some("v1"), None, None, "blocked")?;
        store.record_denial(Some("v2"), None, None, "blocked")?;
        assert_eq!(store.deny_count_since(since)?, 2);
        assert_eq!(store.deny_count_for_release_since("v1", since)?, 1);
        Ok(())
    }
}
