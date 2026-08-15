//! State store for Tier 2 cross-invocation ordering rules
//!
//! Provides minimal persistent markers for tracking session state across
//! process invocations. This is new surface with no equivalent in
//! org-rule-guard.py.
//!
//! ## Purpose
//!
//! Tier 2 rules need to remember events from earlier in a session:
//! - "Did `git pull` happen before this `bf sync --flush-only`?"
//! - "Did `bf sync --flush-only` happen before this `bf doctor --repair`?"
//!
//! The state store persists this information across invocations so that
//! ordering rules can be enforced correctly.
//!
//! ## Design
//!
//! - **Minimal data model**: Only tracks what Tier 2 rules actually need
//! - **Session-scoped**: State is per-session, not global
//! - **Process-safe**: Uses atomic write-then-replace for consistency
//! - **User-writable location**: Unlike the rule pack and trust pointer,
//!   this IS intentionally user-writable — it's runtime state, not
//!   security-critical configuration

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

/// Session state for Tier 2 ordering rules
///
/// Tracks events that need to be remembered across process invocations
/// to enforce ordering constraints.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionState {
    /// Unique identifier for this session
    ///
    /// Generated from timestamp + PID to distinguish different sessions.
    /// A new session starts when the guard begins evaluating a new
    /// sequence of commands (e.g., a new agent session or conversation).
    #[serde(default)]
    pub session_id: String,

    /// Timestamp when git pull was last executed in this session
    ///
    /// Used to enforce: "deny `bf sync --flush-only` unless `git pull`
    /// has already happened in this session"
    #[serde(skip_serializing_if = "Option::is_none")]
    pub git_pull_timestamp: Option<String>,

    /// Timestamp when flush was last executed in this session
    ///
    /// Used to enforce: "deny `bf doctor --repair` unless a flush has
    /// already happened in this session"
    #[serde(skip_serializing_if = "Option::is_none")]
    pub flush_timestamp: Option<String>,
}

impl SessionState {
    /// Create a new session state with a fresh session ID
    pub fn new() -> Self {
        Self {
            session_id: Self::generate_session_id(),
            git_pull_timestamp: None,
            flush_timestamp: None,
        }
    }

    /// Generate a unique session identifier
    ///
    /// Combines current timestamp (with nanosecond precision) with process ID
    /// for uniqueness. Format: `<timestamp>-<pid>`
    fn generate_session_id() -> String {
        let timestamp = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Nanos, true);
        let pid = std::process::id();
        format!("{}-{}", timestamp.replace(':', "-").replace('+', "-"), pid)
    }

    /// Mark that git pull has occurred in this session
    pub fn mark_git_pull(&mut self) {
        self.git_pull_timestamp = Some(chrono::Utc::now().to_rfc3339());
    }

    /// Mark that flush has occurred in this session
    pub fn mark_flush(&mut self) {
        self.flush_timestamp = Some(chrono::Utc::now().to_rfc3339());
    }

    /// Check if git pull has occurred in this session
    pub fn has_git_pull(&self) -> bool {
        self.git_pull_timestamp.is_some()
    }

    /// Check if flush has occurred in this session
    pub fn has_flush(&self) -> bool {
        self.flush_timestamp.is_some()
    }

    /// Clear all session state (start fresh)
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

/// State store manager
///
/// Handles loading and saving session state to disk.
pub struct StateStore {
    /// Path to the state store file
    path: PathBuf,
}

impl StateStore {
    /// Create a new state store
    ///
    /// The state file is stored in a user-writable location:
    /// - Default: `~/.cache/icg/session-state.json` (or equivalent per platform)
    /// - Or a custom path for testing/CI contexts
    ///
    /// This is intentionally user-writable (unlike rule-pack and trust-pointer)
    /// because it's runtime state, not security-critical configuration.
    pub fn new(path: impl AsRef<Path>) -> Self {
        Self {
            path: path.as_ref().to_path_buf(),
        }
    }

    /// Get the default state store path
    ///
    /// Uses the platform's cache directory:
    /// - Linux: `~/.cache/icg/session-state.json`
    /// - macOS: `~/Library/Caches/icg/session-state.json`
    /// - Windows: `%LOCALAPPDATA%\icg\session-state.json`
    pub fn default_path() -> Result<PathBuf> {
        let cache_dir = dirs::cache_dir()
            .context("Failed to determine platform cache directory")?;

        let icg_cache = cache_dir.join("icg");
        Ok(icg_cache.join("session-state.json"))
    }

    /// Ensure the state file directory exists
    fn ensure_parent_dir(&self) -> Result<()> {
        let parent = self.path
            .parent()
            .context("State path has no parent directory")?;

        if !parent.exists() {
            fs::create_dir_all(parent)
                .with_context(|| format!("Failed to create directory: {}", parent.display()))?;
        }

        Ok(())
    }

    /// Load the current session state
    ///
    /// Returns a default (empty) session state if the file doesn't exist.
    pub fn load(&self) -> Result<SessionState> {
        self.ensure_parent_dir()?;

        if !self.path.exists() {
            return Ok(SessionState::new());
        }

        let content = fs::read_to_string(&self.path)
            .with_context(|| format!("Failed to read state from {}", self.path.display()))?;

        let state: SessionState = serde_json::from_str(&content)
            .with_context(|| format!("Failed to parse state from {}", self.path.display()))?;

        Ok(state)
    }

    /// Save session state
    ///
    /// Uses atomic write-then-replace for consistency.
    pub fn save(&self, state: &SessionState) -> Result<()> {
        self.ensure_parent_dir()?;

        // Write to a temporary file first, then atomic rename
        let temp_path = self.path.with_extension("tmp");

        let content = serde_json::to_string_pretty(state)
            .context("Failed to serialize session state")?;

        fs::write(&temp_path, content)
            .with_context(|| format!("Failed to write state to {}", temp_path.display()))?;

        // Atomic rename
        fs::rename(&temp_path, &self.path)
            .with_context(|| format!("Failed to rename state from {} to {}", temp_path.display(), self.path.display()))?;

        Ok(())
    }

    /// Update session state with a modification function
    ///
    /// This is the primary interface for state updates:
    /// 1. Load current state
    /// 2. Apply the modification function
    /// 3. Save the updated state
    pub fn update<F>(&self, f: F) -> Result<SessionState>
    where
        F: FnOnce(&mut SessionState),
    {
        let mut state = self.load()?;
        f(&mut state);
        let updated = state.clone();
        self.save(&state)?;
        Ok(updated)
    }

    /// Check if git pull has occurred in the current session
    pub fn has_git_pull(&self) -> Result<bool> {
        Ok(self.load()?.has_git_pull())
    }

    /// Check if flush has occurred in the current session
    pub fn has_flush(&self) -> Result<bool> {
        Ok(self.load()?.has_flush())
    }

    /// Mark that git pull has occurred in this session
    pub fn mark_git_pull(&self) -> Result<()> {
        self.update(|state| state.mark_git_pull())?;
        Ok(())
    }

    /// Mark that flush has occurred in this session
    pub fn mark_flush(&self) -> Result<()> {
        self.update(|state| state.mark_flush())?;
        Ok(())
    }

    /// Start a new session (clear all state)
    pub fn new_session(&self) -> Result<()> {
        self.update(|state| state.clear())?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_session_state_new() {
        let state = SessionState::new();
        assert!(!state.session_id.is_empty());
        assert!(state.git_pull_timestamp.is_none());
        assert!(state.flush_timestamp.is_none());
        assert!(!state.has_git_pull());
        assert!(!state.has_flush());
    }

    #[test]
    fn test_session_state_mark_git_pull() {
        let mut state = SessionState::new();
        assert!(!state.has_git_pull());

        state.mark_git_pull();
        assert!(state.has_git_pull());
        assert!(state.git_pull_timestamp.is_some());
    }

    #[test]
    fn test_session_state_mark_flush() {
        let mut state = SessionState::new();
        assert!(!state.has_flush());

        state.mark_flush();
        assert!(state.has_flush());
        assert!(state.flush_timestamp.is_some());
    }

    #[test]
    fn test_session_state_clear() {
        let mut state = SessionState::new();
        state.mark_git_pull();
        state.mark_flush();
        assert!(state.has_git_pull());
        assert!(state.has_flush());

        let old_session_id = state.session_id.clone();
        state.clear();
        assert!(!state.has_git_pull());
        assert!(!state.has_flush());
        assert_ne!(state.session_id, old_session_id);
    }

    #[test]
    fn test_session_state_serialization() {
        let mut state = SessionState::new();
        state.mark_git_pull();
        state.mark_flush();

        let json = serde_json::to_string_pretty(&state).unwrap();
        println!("Serialized session state:\n{}", json);

        let deserialized: SessionState = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.session_id, state.session_id);
        assert_eq!(deserialized.git_pull_timestamp, state.git_pull_timestamp);
        assert_eq!(deserialized.flush_timestamp, state.flush_timestamp);
    }

    #[test]
    fn test_store_save_and_load() -> Result<()> {
        let dir = tempdir()?;
        let path = dir.path().join("session-state.json");
        let store = StateStore::new(&path);

        // Save initial state
        let mut state = SessionState::new();
        state.mark_git_pull();
        store.save(&state)?;

        // Load it back
        let loaded = store.load()?;
        assert_eq!(loaded.session_id, state.session_id);
        assert!(loaded.has_git_pull());
        assert!(!loaded.has_flush());

        Ok(())
    }

    #[test]
    fn test_store_update() -> Result<()> {
        let dir = tempdir()?;
        let path = dir.path().join("session-state.json");
        let store = StateStore::new(&path);

        // Initial state is empty
        assert!(!store.has_git_pull()?);
        assert!(!store.has_flush()?);

        // Mark git pull
        store.mark_git_pull()?;
        assert!(store.has_git_pull()?);
        assert!(!store.has_flush()?);

        // Mark flush
        store.mark_flush()?;
        assert!(store.has_git_pull()?);
        assert!(store.has_flush()?);

        Ok(())
    }

    #[test]
    fn test_store_new_session() -> Result<()> {
        let dir = tempdir()?;
        let path = dir.path().join("session-state.json");
        let store = StateStore::new(&path);

        // Set up some state
        store.mark_git_pull()?;
        store.mark_flush()?;
        assert!(store.has_git_pull()?);
        assert!(store.has_flush()?);

        // Start new session
        store.new_session()?;
        assert!(!store.has_git_pull()?);
        assert!(!store.has_flush()?);

        Ok(())
    }

    #[test]
    fn test_default_path() {
        let path = StateStore::default_path().unwrap();
        // On Linux, this should be ~/.cache/icg/session-state.json
        // On macOS, this should be ~/Library/Caches/icg/session-state.json
        // On Windows, this should be %LOCALAPPDATA%\icg\session-state.json
        assert!(path.ends_with("icg") || path.ends_with("icg\\session-state.json") || path.ends_with("icg/session-state.json"));
    }

    #[test]
    fn test_session_id_uniqueness() {
        let state1 = SessionState::new();
        let state2 = SessionState::new();
        // Session IDs should be different (generated at different times)
        assert_ne!(state1.session_id, state2.session_id);
    }

    #[test]
    fn test_atomic_write_consistency() -> Result<()> {
        let dir = tempdir()?;
        let path = dir.path().join("session-state.json");
        let store = StateStore::new(&path);

        // First write
        store.mark_git_pull()?;
        assert!(store.has_git_pull()?);

        // Second write (should atomic-replace, not corrupt)
        store.mark_flush()?;
        assert!(store.has_git_pull()?);
        assert!(store.has_flush()?);

        // Verify file contains valid JSON
        let content = fs::read_to_string(&path)?;
        let _parsed: SessionState = serde_json::from_str(&content)?;

        Ok(())
    }
}
