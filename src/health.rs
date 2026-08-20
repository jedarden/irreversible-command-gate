//! Guard health tracking and crash monitoring infrastructure.
//!
//! This module implements the foundational health tracking for the guard process,
//! enabling crash detection, uptime monitoring, and state persistence. This is the
//! data layer needed for both fail-open and fail-closed modes.
//!
//! ## Architecture
//!
//! The health system works in three phases:
//! 1. **Tracking**: Records process lifecycle events (start, crash, clean exit)
//! 2. **Analysis**: Computes health metrics from crash history and uptime
//! 3. **Reporting**: Exposes health status via API and integrates with telemetry
//!
//! ## Data Flow
//!
//! ```text
//! Process Start
//!   → HealthStore::mark_start()
//!   → HealthStore::persist()
//!
//! Process Exit (clean)
//!   → HealthStore::mark_clean_exit()
//!   → HealthStore::persist()
//!
//! Process Crash (detect by exit code, signal, or watchdog)
//!   → HealthStore::record_crash()
//!   → HealthStore::persist()
//!   → Telemetry integration
//!
//! Health Status Query
//!   → HealthStore::health_status()
//!   → HealthMetrics (for API/reporting)
//! ```
//!
//! ## Crash Detection
//!
//! The system detects crashes through multiple mechanisms:
//! - **Exit code monitoring**: Non-zero exit codes indicate abnormal termination
//! - **Signal detection**: SIGSEGV, SIGABRT, SIGBUS, SIGFPE indicate fatal errors
//! - **Watchdog timeout**: Process unresponsive for configured threshold
//! - **OOM detection**: Memory exhaustion events (via cgroup or process monitoring)
//! - **Panic detection**: Rust panic events caught before process termination
//!
//! ## Persistence
//!
//! Health state is persisted to disk using the same atomic write pattern as
//! the state store. This ensures durability across guard restarts and provides
//! crash recovery capabilities.

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::time::Duration;

/// Current on-disk health state schema version.
pub const HEALTH_SCHEMA_VERSION: u32 = 1;

/// Health configuration thresholds and limits.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HealthConfig {
    /// Maximum number of crash records retained in history.
    ///
    /// Prevents unbounded growth of crash history while retaining diagnostic data.
    pub max_crash_history: usize,

    /// Time threshold before a process is considered "stable" (no longer a recent startup).
    ///
    /// Used to distinguish between startup crashes and runtime crashes.
    pub stability_threshold: Duration,

    /// Consecutive clean run threshold for considering a process "healthy".
    ///
    /// Process must have this many consecutive clean exits to be considered healthy.
    pub healthy_consecutive_runs: usize,

    /// Maximum crash rate (crashes per hour) before process is considered "unstable".
    pub max_crashes_per_hour: f64,
}

impl Default for HealthConfig {
    fn default() -> Self {
        Self {
            max_crash_history: 100,
            stability_threshold: Duration::from_secs(300), // 5 minutes
            healthy_consecutive_runs: 5,
            max_crashes_per_hour: 10.0,
        }
    }
}

/// Individual crash event record.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CrashRecord {
    /// Stable identifier for this crash event.
    pub id: String,

    /// UTC timestamp when the crash occurred.
    pub timestamp: DateTime<Utc>,

    /// Type of crash that occurred.
    pub crash_type: CrashType,

    /// Optional signal that caused the crash (Unix only).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signal: Option<i32>,

    /// Exit code if available (0-255).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i32>,

    /// Optional additional context about the crash.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context: Option<String>,

    /// Session ID in which the crash occurred.
    #[serde(default)]
    pub session_id: String,
}

impl CrashRecord {
    /// Create a new crash record with the current timestamp.
    pub fn new(crash_type: CrashType) -> Self {
        let timestamp = Utc::now();
        let id = format!(
            "crash-{}-{}",
            timestamp.timestamp_nanos_opt().unwrap_or_default(),
            std::process::id()
        );

        Self {
            id,
            timestamp,
            crash_type,
            signal: None,
            exit_code: None,
            context: None,
            session_id: String::new(),
        }
    }

    /// Set the signal that caused this crash (Unix only).
    pub fn with_signal(mut self, signal: i32) -> Self {
        self.signal = Some(signal);
        self
    }

    /// Set the exit code for this crash.
    pub fn with_exit_code(mut self, exit_code: i32) -> Self {
        self.exit_code = Some(exit_code);
        self
    }

    /// Set additional context for this crash.
    pub fn with_context(mut self, context: String) -> Self {
        self.context = Some(context);
        self
    }

    /// Set the session ID for this crash.
    pub fn with_session_id(mut self, session_id: String) -> Self {
        self.session_id = session_id;
        self
    }
}

/// Classification of crash types.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CrashType {
    /// Segmentation fault (SIGSEGV).
    SegmentationFault,

    /// Abort (SIGABRT), typically from panic or assertion failure.
    Abort,

    /// Bus error (SIGBUS), memory alignment issues.
    BusError,

    /// Floating point exception (SIGFPE).
    FloatingPointException,

    /// Out-of-memory (OOM) kill.
    OutOfMemory,

    /// Timeout or watchdog expiration.
    Timeout,

    /// Unknown or unclassified crash.
    Unknown,

    /// Explicit panic (caught before signal delivery).
    Panic,

    /// Non-zero exit code (general error).
    ExitCodeError,
}

impl CrashType {
    /// Create a CrashType from a Unix signal number.
    #[cfg(unix)]
    pub fn from_signal(signal: i32) -> Option<Self> {
        match signal {
            libc::SIGSEGV => Some(CrashType::SegmentationFault),
            libc::SIGABRT => Some(CrashType::Abort),
            libc::SIGBUS => Some(CrashType::BusError),
            libc::SIGFPE => Some(CrashType::FloatingPointException),
            _ => None,
        }
    }

    /// Get a human-readable description of the crash type.
    pub fn description(&self) -> &'static str {
        match self {
            CrashType::SegmentationFault => "Memory access violation (SIGSEGV)",
            CrashType::Abort => "Process abort (SIGABRT), likely panic or assertion",
            CrashType::BusError => "Memory alignment error (SIGBUS)",
            CrashType::FloatingPointException => "Arithmetic exception (SIGFPE)",
            CrashType::OutOfMemory => "Out of memory (OOM)",
            CrashType::Timeout => "Process timeout or watchdog expiration",
            CrashType::Unknown => "Unknown crash type",
            CrashType::Panic => "Rust panic caught before termination",
            CrashType::ExitCodeError => "Non-zero exit code",
        }
    }

    /// Check if this crash type indicates a fatal process error.
    pub fn is_fatal(&self) -> bool {
        matches!(
            self,
            CrashType::SegmentationFault
                | CrashType::Abort
                | CrashType::BusError
                | CrashType::FloatingPointException
                | CrashType::OutOfMemory
        )
    }

    /// Check if this crash type indicates a timeout condition.
    pub fn is_timeout(&self) -> bool {
        matches!(self, CrashType::Timeout)
    }
}

/// Overall health status of the guard process.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HealthStatus {
    /// Process is healthy with stable history.
    Healthy,

    /// Process is recovering from recent crashes.
    Recovering,

    /// Process is unstable with high crash rate.
    Unstable,

    /// Process is in degraded state (warnings but not crashed).
    Degraded,

    /// Process is dead or not running.
    Dead,

    /// Unknown health state (no data yet).
    Unknown,
}

impl HealthStatus {
    /// Check if the status indicates the process is running.
    pub fn is_running(&self) -> bool {
        matches!(
            self,
            HealthStatus::Healthy | HealthStatus::Recovering | HealthStatus::Unstable | HealthStatus::Degraded
        )
    }

    /// Check if the status indicates the process is healthy or stable.
    pub fn is_healthy(&self) -> bool {
        matches!(self, HealthStatus::Healthy | HealthStatus::Degraded)
    }
}

/// Computed health metrics for reporting.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HealthMetrics {
    /// Current overall health status.
    pub status: HealthStatus,

    /// Total number of crashes recorded.
    pub total_crashes: usize,

    /// Number of crashes in the last hour.
    pub recent_crashes: usize,

    /// Current crash rate (crashes per hour).
    pub crash_rate: f64,

    /// Number of consecutive clean runs.
    pub consecutive_clean_runs: usize,

    /// Uptime of the current process run (None if not running).
    pub current_uptime: Option<Duration>,

    /// Timestamp of the last crash (None if no crashes recorded).
    pub last_crash_at: Option<DateTime<Utc>>,

    /// Timestamp of the last successful run start (None if never started).
    pub last_start_at: Option<DateTime<Utc>>,

    /// Whether the process is currently considered stable.
    pub is_stable: bool,

    /// Time since the process became stable (None if not stable).
    pub time_since_stable: Option<Duration>,
}

/// Persistent health state across process runs.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HealthState {
    /// Schema version of this health state document.
    #[serde(default = "default_health_schema_version")]
    pub schema_version: u32,

    /// Total number of crashes recorded across all process runs.
    #[serde(default)]
    pub total_crashes: usize,

    /// Number of consecutive clean runs.
    #[serde(default)]
    pub consecutive_clean_runs: usize,

    /// Timestamp of the most recent crash.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_crash_at: Option<DateTime<Utc>>,

    /// Timestamp when the current process run started.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub current_run_started_at: Option<DateTime<Utc>>,

    /// Timestamp when the last successful run started.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_start_at: Option<DateTime<Utc>>,

    /// History of crash records (bounded by HealthConfig::max_crash_history).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub crash_history: Vec<CrashRecord>,

    /// Health configuration.
    #[serde(default)]
    pub config: HealthConfig,
}

fn default_health_schema_version() -> u32 {
    HEALTH_SCHEMA_VERSION
}

impl HealthState {
    /// Create a new empty health state.
    pub fn new() -> Self {
        Self {
            schema_version: HEALTH_SCHEMA_VERSION,
            total_crashes: 0,
            consecutive_clean_runs: 0,
            last_crash_at: None,
            current_run_started_at: None,
            last_start_at: None,
            crash_history: Vec::new(),
            config: HealthConfig::default(),
        }
    }

    /// Create a new health state with custom configuration.
    pub fn with_config(config: HealthConfig) -> Self {
        Self {
            config,
            ..Self::new()
        }
    }

    /// Record that the process has started.
    pub fn mark_start(&mut self) {
        let now = Utc::now();
        self.current_run_started_at = Some(now);
        self.last_start_at = Some(now);
    }

    /// Record a clean process exit.
    pub fn mark_clean_exit(&mut self) {
        self.consecutive_clean_runs = self.consecutive_clean_runs.saturating_add(1);
        self.current_run_started_at = None;
    }

    /// Record a crash event.
    pub fn record_crash(&mut self, crash: CrashRecord) {
        self.total_crashes = self.total_crashes.saturating_add(1);
        self.consecutive_clean_runs = 0;
        self.last_crash_at = Some(crash.timestamp);
        self.current_run_started_at = None;

        // Add to crash history, applying retention limit
        self.crash_history.push(crash);
        if self.crash_history.len() > self.config.max_crash_history {
            self.crash_history.remove(0);
        }
    }

    /// Calculate current health metrics.
    pub fn compute_metrics(&self) -> HealthMetrics {
        let now = Utc::now();

        // Calculate uptime of current run
        let current_uptime = self.current_run_started_at.map(|start| {
            let duration = now.signed_duration_since(start);
            duration.to_std().unwrap_or(Duration::ZERO)
        });

        // Calculate time since becoming stable
        let time_since_stable = if let Some(started) = self.current_run_started_at {
            let elapsed = now.signed_duration_since(started);
            let elapsed_duration = elapsed.to_std().unwrap_or(Duration::ZERO);
            if elapsed_duration >= self.config.stability_threshold {
                Some(elapsed_duration - self.config.stability_threshold)
            } else {
                None
            }
        } else {
            None
        };

        // Count crashes in the last hour
        let one_hour_ago = now - chrono::Duration::hours(1);
        let recent_crashes = self
            .crash_history
            .iter()
            .filter(|crash| crash.timestamp > one_hour_ago)
            .count();

        // Calculate crash rate (crashes per hour)
        let crash_rate = if !self.crash_history.is_empty() {
            if let (Some(oldest), Some(newest)) = (
                self.crash_history.first().map(|c| c.timestamp),
                self.crash_history.last().map(|c| c.timestamp),
            ) {
                let duration_hours = (newest - oldest).num_seconds().max(1) as f64 / 3600.0;
                self.crash_history.len() as f64 / duration_hours
            } else {
                0.0
            }
        } else {
            0.0
        };

        // Determine if process is currently stable
        let is_stable = current_uptime
            .map(|uptime| uptime >= self.config.stability_threshold)
            .unwrap_or(false);

        // Determine overall health status
        let status = if self.current_run_started_at.is_some() {
            // Process is currently running
            if crash_rate > self.config.max_crashes_per_hour {
                HealthStatus::Unstable
            } else if self.consecutive_clean_runs >= self.config.healthy_consecutive_runs {
                HealthStatus::Healthy
            } else if is_stable {
                HealthStatus::Recovering
            } else {
                HealthStatus::Degraded
            }
        } else if self.last_start_at.is_some() {
            // Process has run before but not currently
            HealthStatus::Dead
        } else {
            // No data yet
            HealthStatus::Unknown
        };

        HealthMetrics {
            status,
            total_crashes: self.total_crashes,
            recent_crashes,
            crash_rate,
            consecutive_clean_runs: self.consecutive_clean_runs,
            current_uptime,
            last_crash_at: self.last_crash_at,
            last_start_at: self.last_start_at,
            is_stable,
            time_since_stable,
        }
    }
}

impl Default for HealthState {
    fn default() -> Self {
        Self::new()
    }
}

/// Health state store manager.
pub struct HealthStore {
    /// Path to the health state file.
    path: PathBuf,
}

impl HealthStore {
    /// Create a health store at the specified path.
    pub fn new(path: impl AsRef<Path>) -> Self {
        Self {
            path: path.as_ref().to_path_buf(),
        }
    }

    /// Return the path used by this store.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Get the default health state path.
    pub fn default_path() -> Result<PathBuf> {
        let cache_dir =
            dirs::cache_dir().context("Failed to determine platform cache directory")?;
        Ok(cache_dir.join("icg").join("health-state.json"))
    }

    /// Ensure the parent directory exists.
    fn ensure_parent_dir(&self) -> Result<()> {
        if let Some(parent) = self.path.parent() {
            if !parent.exists() {
                std::fs::create_dir_all(parent)
                    .with_context(|| format!("Failed to create directory: {}", parent.display()))?;
            }
        }
        Ok(())
    }

    /// Load health state from disk, or create new if file doesn't exist.
    pub fn load_or_create(&self) -> Result<HealthState> {
        self.ensure_parent_dir()?;

        if self.path.exists() {
            let content = std::fs::read_to_string(&self.path)
                .with_context(|| format!("Failed to read health state from {}", self.path.display()))?;

            let mut state: HealthState = serde_json::from_str(&content)
                .with_context(|| format!("Failed to parse health state from {}", self.path.display()))?;

            // Schema migration and validation
            if state.schema_version > HEALTH_SCHEMA_VERSION {
                anyhow::bail!(
                    "Health state schema version {} is newer than supported version {}",
                    state.schema_version,
                    HEALTH_SCHEMA_VERSION
                );
            }
            state.schema_version = HEALTH_SCHEMA_VERSION;

            Ok(state)
        } else {
            Ok(HealthState::new())
        }
    }

    /// Persist health state to disk atomically.
    pub fn persist(&self, state: &HealthState) -> Result<()> {
        self.ensure_parent_dir()?;

        let mut state = state.clone();
        state.schema_version = HEALTH_SCHEMA_VERSION;

        let content = serde_json::to_vec_pretty(&state)
            .context("Failed to serialize health state")?;

        // Atomic write: write to temp file, then rename
        let file_name = self
            .path
            .file_name()
            .context("Health path has no file name")?
            .to_string_lossy();
        let temp_path = self
            .path
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .join(format!(".{file_name}.tmp-{}", std::process::id()));

        // Write temp file
        std::fs::write(&temp_path, content)
            .with_context(|| format!("Failed to write temporary health file {}", temp_path.display()))?;

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&temp_path, std::fs::Permissions::from_mode(0o600))
                .context("Failed to set health file permissions")?;
        }

        // Sync the temp file
        let temp_file = std::fs::File::open(&temp_path)
            .with_context(|| format!("Failed to open temp file for syncing {}", temp_path.display()))?;
        temp_file.sync_all()
            .with_context(|| format!("Failed to sync temporary health file {}", temp_path.display()))?;

        // Atomic rename
        std::fs::rename(&temp_path, &self.path)
            .with_context(|| format!("Failed to rename health file to {}", self.path.display()))?;

        // Sync parent directory
        if let Some(parent) = self.path.parent() {
            let parent_dir = std::fs::File::open(parent)
                .with_context(|| format!("Failed to open parent directory for syncing {}", parent.display()))?;
            parent_dir.sync_all()
                .with_context(|| format!("Failed to sync parent directory {}", parent.display()))?;
        }

        Ok(())
    }

    /// Update health state under one transaction.
    pub fn update<F>(&self, f: F) -> Result<HealthState>
    where
        F: FnOnce(&mut HealthState),
    {
        let mut state = self.load_or_create()?;
        f(&mut state);
        self.persist(&state)?;
        Ok(state)
    }

    /// Get the current health metrics.
    pub fn health_metrics(&self) -> Result<HealthMetrics> {
        Ok(self.load_or_create()?.compute_metrics())
    }

    /// Mark that the process has started.
    pub fn mark_start(&self) -> Result<()> {
        self.update(HealthState::mark_start)?;
        Ok(())
    }

    /// Record a clean process exit.
    pub fn mark_clean_exit(&self) -> Result<()> {
        self.update(HealthState::mark_clean_exit)?;
        Ok(())
    }

    /// Record a crash event.
    pub fn record_crash(&self, crash: CrashRecord) -> Result<()> {
        self.update(|state| state.record_crash(crash))?;
        Ok(())
    }

    /// Clear all health data (useful for testing or reset).
    pub fn clear(&self) -> Result<()> {
        let state = HealthState::new();
        self.persist(&state)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn test_store(dir: &Path) -> HealthStore {
        HealthStore::new(dir.join("health-state.json"))
    }

    #[test]
    fn health_state_new_is_empty() {
        let state = HealthState::new();
        assert_eq!(state.schema_version, HEALTH_SCHEMA_VERSION);
        assert_eq!(state.total_crashes, 0);
        assert_eq!(state.consecutive_clean_runs, 0);
        assert!(state.last_crash_at.is_none());
        assert!(state.current_run_started_at.is_none());
        assert!(state.last_start_at.is_none());
        assert!(state.crash_history.is_empty());
    }

    #[test]
    fn health_state_mark_start() {
        let mut state = HealthState::new();
        assert!(state.current_run_started_at.is_none());

        state.mark_start();
        assert!(state.current_run_started_at.is_some());
        assert!(state.last_start_at.is_some());
    }

    #[test]
    fn health_state_clean_exit_increments_consecutive() {
        let mut state = HealthState::new();
        assert_eq!(state.consecutive_clean_runs, 0);

        state.mark_clean_exit();
        assert_eq!(state.consecutive_clean_runs, 1);

        state.mark_clean_exit();
        assert_eq!(state.consecutive_clean_runs, 2);
    }

    #[test]
    fn health_state_crash_resets_consecutive() {
        let mut state = HealthState::new();
        state.mark_clean_exit();
        state.mark_clean_exit();
        assert_eq!(state.consecutive_clean_runs, 2);

        let crash = CrashRecord::new(CrashType::Abort);
        state.record_crash(crash);
        assert_eq!(state.consecutive_clean_runs, 0);
        assert_eq!(state.total_crashes, 1);
        assert!(state.last_crash_at.is_some());
    }

    #[test]
    fn crash_record_creation() {
        let crash = CrashRecord::new(CrashType::SegmentationFault)
            .with_signal(11)
            .with_exit_code(139)
            .with_context("Test crash".to_string())
            .with_session_id("test-session".to_string());

        assert_eq!(crash.crash_type, CrashType::SegmentationFault);
        assert_eq!(crash.signal, Some(11));
        assert_eq!(crash.exit_code, Some(139));
        assert_eq!(crash.context, Some("Test crash".to_string()));
        assert_eq!(crash.session_id, "test-session");
        assert!(!crash.id.is_empty());
    }

    #[test]
    fn crash_type_from_signal() {
        #[cfg(unix)]
        {
            assert_eq!(
                CrashType::from_signal(libc::SIGSEGV),
                Some(CrashType::SegmentationFault)
            );
            assert_eq!(
                CrashType::from_signal(libc::SIGABRT),
                Some(CrashType::Abort)
            );
            assert_eq!(
                CrashType::from_signal(libc::SIGBUS),
                Some(CrashType::BusError)
            );
            assert_eq!(
                CrashType::from_signal(libc::SIGFPE),
                Some(CrashType::FloatingPointException)
            );
            assert_eq!(CrashType::from_signal(9999), None);
        }
    }

    #[test]
    fn crash_type_properties() {
        assert!(CrashType::SegmentationFault.is_fatal());
        assert!(CrashType::Abort.is_fatal());
        assert!(CrashType::OutOfMemory.is_fatal());
        assert!(!CrashType::Timeout.is_fatal());
        assert!(CrashType::Timeout.is_timeout());
        assert!(!CrashType::Abort.is_timeout());
    }

    #[test]
    fn health_metrics_running_process() {
        let mut state = HealthState::new();
        state.mark_start();

        // Give it some consecutive clean runs
        for _ in 0..5 {
            state.mark_clean_exit();
            state.mark_start();
        }

        let metrics = state.compute_metrics();
        assert_eq!(metrics.consecutive_clean_runs, 5);
        assert!(metrics.current_uptime.is_some());
        assert!(metrics.last_start_at.is_some());
    }

    #[test]
    fn health_metrics_crash_history() {
        let mut state = HealthState::new();

        // Record some crashes
        for _ in 0..3 {
            let crash = CrashRecord::new(CrashType::Abort);
            state.record_crash(crash);
        }

        let metrics = state.compute_metrics();
        assert_eq!(metrics.total_crashes, 3);
        assert_eq!(metrics.consecutive_clean_runs, 0);
        assert!(metrics.last_crash_at.is_some());
    }

    #[test]
    fn health_status_determination() {
        let mut state = HealthState::new();

        // Unknown - no data
        let metrics = state.compute_metrics();
        assert_eq!(metrics.status, HealthStatus::Unknown);

        // Running but not stable
        state.mark_start();
        let metrics = state.compute_metrics();
        assert_eq!(metrics.status, HealthStatus::Degraded);

        // Healthy after consecutive runs
        for _ in 0..5 {
            state.mark_clean_exit();
            state.mark_start();
        }
        let metrics = state.compute_metrics();
        assert_eq!(metrics.status, HealthStatus::Healthy);

        // Dead after process exits
        state.current_run_started_at = None;
        let metrics = state.compute_metrics();
        assert_eq!(metrics.status, HealthStatus::Dead);
    }

    #[test]
    fn health_store_persistence_roundtrip() -> Result<()> {
        let dir = tempdir()?;
        let store = test_store(dir.path());

        // Update state
        store.update(|state| {
            state.mark_start();
            state.consecutive_clean_runs = 3;
        })?;

        // Load and verify
        let loaded_state = store.load_or_create()?;
        assert!(loaded_state.current_run_started_at.is_some());
        assert_eq!(loaded_state.consecutive_clean_runs, 3);

        Ok(())
    }

    #[test]
    fn health_store_clear_resets_state() -> Result<()> {
        let dir = tempdir()?;
        let store = test_store(dir.path());

        // Add some data
        store.update(|state| {
            state.mark_start();
            state.record_crash(CrashRecord::new(CrashType::Abort));
        })?;

        // Clear
        store.clear()?;

        // Verify reset
        let state = store.load_or_create()?;
        assert_eq!(state.total_crashes, 0);
        assert_eq!(state.consecutive_clean_runs, 0);
        assert!(state.current_run_started_at.is_none());

        Ok(())
    }

    #[test]
    fn crash_history_respects_retention_limit() {
        let mut state = HealthState::with_config(HealthConfig {
            max_crash_history: 5,
            ..Default::default()
        });

        // Add more crashes than the limit
        for _ in 0..10 {
            let crash = CrashRecord::new(CrashType::Abort);
            state.record_crash(crash);
        }

        // Should only retain the most recent 5
        assert_eq!(state.crash_history.len(), 5);
        assert_eq!(state.total_crashes, 10); // Total count not affected
    }

    #[test]
    fn health_status_running_checks() {
        assert!(HealthStatus::Healthy.is_running());
        assert!(HealthStatus::Recovering.is_running());
        assert!(HealthStatus::Unstable.is_running());
        assert!(HealthStatus::Degraded.is_running());
        assert!(!HealthStatus::Dead.is_running());
        assert!(!HealthStatus::Unknown.is_running());
    }

    #[test]
    fn health_status_healthy_checks() {
        assert!(HealthStatus::Healthy.is_healthy());
        assert!(HealthStatus::Degraded.is_healthy());
        assert!(!HealthStatus::Recovering.is_healthy());
        assert!(!HealthStatus::Unstable.is_healthy());
        assert!(!HealthStatus::Dead.is_healthy());
        assert!(!HealthStatus::Unknown.is_healthy());
    }
}
