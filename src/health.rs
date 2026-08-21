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
use std::fs::{File, OpenOptions};
use std::path::{Path, PathBuf};
use std::process::ExitStatus;
use std::time::Duration;

#[cfg(unix)]
use std::os::unix::process::ExitStatusExt;

/// Current on-disk health state schema version.
pub const HEALTH_SCHEMA_VERSION: u32 = 2;

const DEFAULT_CGROUP_MEMORY_EVENTS: &str = "/sys/fs/cgroup/memory.events";

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

    /// Build a crash record from a child-process exit status.
    ///
    /// A signal is the most reliable evidence available to a supervisor for
    /// segfaults, aborts, and similar fatal failures.  `oom_killed` must come
    /// from the supervisor's cgroup/process accounting; exit code 137 alone
    /// is not sufficient to claim OOM because SIGKILL has other valid causes.
    pub fn from_exit_status(status: &ExitStatus, oom_killed: bool) -> Option<Self> {
        if status.success() {
            return None;
        }

        #[cfg(unix)]
        if let Some(signal) = status.signal() {
            let crash_type = if oom_killed {
                CrashType::OutOfMemory
            } else {
                CrashType::from_signal(signal).unwrap_or(CrashType::Unknown)
            };
            return Some(Self::new(crash_type).with_signal(signal));
        }

        let exit_code = status.code();
        let crash_type = if oom_killed {
            CrashType::OutOfMemory
        } else {
            CrashType::ExitCodeError
        };
        Some(Self::new(crash_type).with_optional_exit_code(exit_code))
    }

    fn with_optional_exit_code(mut self, exit_code: Option<i32>) -> Self {
        self.exit_code = exit_code;
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

/// Crash evidence helper for supervisors and cgroup-aware deployments.
///
/// Signal-based failures are classified from `ExitStatus`. OOM is classified
/// only when the caller supplies supervisor evidence or when the configured
/// cgroup `memory.events` counter increased; this avoids mislabeling an
/// arbitrary SIGKILL as an OOM kill.
#[derive(Debug, Clone, Default)]
pub struct CrashDetector {
    oom_events_path: Option<PathBuf>,
    baseline_oom_kill_count: Option<u64>,
}

impl CrashDetector {
    /// Use the configured cgroup memory-events file, if one is available.
    pub fn new() -> Self {
        let path = std::env::var_os("ICG_CGROUP_MEMORY_EVENTS")
            .filter(|path| !path.is_empty())
            .map(PathBuf::from)
            .or_else(|| Path::new(DEFAULT_CGROUP_MEMORY_EVENTS).is_file().then(|| {
                PathBuf::from(DEFAULT_CGROUP_MEMORY_EVENTS)
            }));
        let baseline_oom_kill_count = path
            .as_ref()
            .and_then(|path| read_oom_kill_count(path).ok());
        Self {
            oom_events_path: path,
            baseline_oom_kill_count,
        }
    }

    /// Construct a detector using an explicit cgroup memory-events file.
    pub fn with_oom_events_path(path: impl Into<PathBuf>) -> Self {
        let path = path.into();
        let baseline_oom_kill_count = read_oom_kill_count(&path).ok();
        Self {
            oom_events_path: Some(path),
            baseline_oom_kill_count,
        }
    }

    /// Read the cgroup's cumulative OOM-kill count.
    pub fn oom_kill_count(&self) -> Result<Option<u64>> {
        let Some(path) = &self.oom_events_path else {
            return Ok(None);
        };
        Ok(Some(read_oom_kill_count(path)?))
    }

    /// Return whether OOM evidence increased since a previously persisted
    /// counter value.
    pub fn oom_killed_since(&self, previous_count: Option<u64>) -> Result<bool> {
        let current_count = self.oom_kill_count()?;
        Ok(match (current_count, previous_count.or(self.baseline_oom_kill_count)) {
            (Some(current), Some(previous)) => current > previous,
            _ => false,
        })
    }

    /// Classify an exit status, consulting cgroup evidence when available.
    pub fn classify_exit_status(&self, status: &ExitStatus) -> Result<Option<CrashRecord>> {
        let oom_killed = self.oom_killed_since(None)?;
        Ok(CrashRecord::from_exit_status(status, oom_killed))
    }
}

fn read_oom_kill_count(path: &Path) -> Result<u64> {
    let content = std::fs::read_to_string(path)
        .with_context(|| format!("Failed to read cgroup memory events from {}", path.display()))?;
    content
        .lines()
        .find_map(|line| {
            let (name, value) = line.split_once(char::is_whitespace)?;
            (name == "oom_kill").then(|| value.trim().parse::<u64>())
        })
        .transpose()
        .with_context(|| format!("Invalid oom_kill value in {}", path.display()))?
        .context("cgroup memory.events did not contain oom_kill")
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

    /// Classify an exit status without recording it.
    pub fn from_exit_status(status: &ExitStatus, oom_killed: bool) -> Option<Self> {
        CrashRecord::from_exit_status(status, oom_killed).map(|record| record.crash_type)
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

impl Default for HealthStatus {
    fn default() -> Self {
        Self::Unknown
    }
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

    /// Stable identifier for the currently running guard invocation.
    ///
    /// This marker is deliberately persisted before work begins.  If the
    /// next invocation finds it still present, the previous invocation did
    /// not reach its clean-exit path (including OOM kills and fatal signals).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub current_run_id: Option<String>,

    /// PID associated with the current run marker, when available.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub current_run_pid: Option<u32>,

    /// Last heartbeat written for the current run marker.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub current_run_heartbeat_at: Option<DateTime<Utc>>,

    /// Cumulative cgroup OOM-kill counter at run start, when available.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub current_run_oom_kill_count: Option<u64>,

    /// Timestamp when the last successful run started.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_start_at: Option<DateTime<Utc>>,

    /// Cumulative uptime of completed and interrupted runs.
    #[serde(default)]
    pub total_uptime: Duration,

    /// Timestamp of the most recent clean exit.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_clean_exit_at: Option<DateTime<Utc>>,

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
            current_run_id: None,
            current_run_pid: None,
            current_run_heartbeat_at: None,
            current_run_oom_kill_count: None,
            last_start_at: None,
            total_uptime: Duration::ZERO,
            last_clean_exit_at: None,
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
        self.current_run_id = Some(new_run_id());
        self.current_run_pid = Some(std::process::id());
        self.current_run_heartbeat_at = Some(now);
        self.current_run_oom_kill_count = CrashDetector::new()
            .oom_kill_count()
            .ok()
            .flatten();
        self.last_start_at = Some(now);
    }

    /// Record a start using a caller-supplied run identifier.
    pub fn mark_start_with_id(&mut self, run_id: impl Into<String>) {
        self.mark_start();
        self.current_run_id = Some(run_id.into());
    }

    /// Record a clean process exit.
    pub fn mark_clean_exit(&mut self) {
        self.accumulate_current_uptime();
        self.consecutive_clean_runs = self.consecutive_clean_runs.saturating_add(1);
        self.current_run_started_at = None;
        self.current_run_id = None;
        self.current_run_pid = None;
        self.current_run_heartbeat_at = None;
        self.current_run_oom_kill_count = None;
        self.last_clean_exit_at = Some(Utc::now());
    }

    /// Record a crash event.
    pub fn record_crash(&mut self, crash: CrashRecord) {
        self.accumulate_current_uptime();
        self.total_crashes = self.total_crashes.saturating_add(1);
        self.consecutive_clean_runs = 0;
        self.last_crash_at = Some(crash.timestamp);
        self.current_run_started_at = None;
        self.current_run_id = None;
        self.current_run_pid = None;
        self.current_run_heartbeat_at = None;
        self.current_run_oom_kill_count = None;

        // Add to crash history, applying retention limit
        self.crash_history.push(crash);
        if self.crash_history.len() > self.config.max_crash_history {
            self.crash_history.remove(0);
        }
    }

    /// Refresh the persisted heartbeat without changing run counters.
    pub fn heartbeat(&mut self) {
        if self.current_run_started_at.is_some() {
            self.current_run_heartbeat_at = Some(Utc::now());
        }
    }

    /// Return the current run identifier, if a run is active.
    pub fn current_run_id(&self) -> Option<&str> {
        self.current_run_id.as_deref()
    }

    fn accumulate_current_uptime(&mut self) {
        if let Some(start) = self.current_run_started_at {
            let elapsed = Utc::now()
                .signed_duration_since(start)
                .to_std()
                .unwrap_or(Duration::ZERO);
            self.total_uptime = self.total_uptime.saturating_add(elapsed);
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
        // The operational threshold is explicitly a one-hour threshold.  A
        // single crash must not become an infinite rate merely because two
        // records have the same timestamp.
        let crash_rate = recent_crashes as f64;

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

impl Clone for HealthStore {
    fn clone(&self) -> Self {
        Self {
            path: self.path.clone(),
        }
    }
}

/// Guard process lifecycle handle.
///
/// Creating a handle persists an active run marker.  Callers must finish it
/// explicitly; if the process is OOM-killed, segfaults, aborts, or otherwise
/// disappears, the next `start_run` call converts the marker into an
/// `Unknown` crash record.  A panic hook can provide the more precise
/// `Panic` classification before termination.
pub struct GuardLifecycle {
    store: HealthStore,
    finished: bool,
}

impl GuardLifecycle {
    /// Start tracking one guard process invocation.
    pub fn start() -> Result<Self> {
        let store = HealthStore::from_environment_or_default()?;
        store.start_run()?;
        store.sync_telemetry_best_effort();

        let panic_store = store.clone();
        let previous_hook = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            let context = info
                .payload()
                .downcast_ref::<&str>()
                .copied()
                .or_else(|| info.payload().downcast_ref::<String>().map(String::as_str))
                .unwrap_or("panic without a string payload")
                .to_string();
            let crash = CrashRecord::new(CrashType::Panic).with_context(context);
            let _ = panic_store.record_crash(crash);
            previous_hook(info);
        }));

        Ok(Self {
            store,
            finished: false,
        })
    }

    /// Mark the invocation as a clean exit.
    pub fn finish_clean(&mut self) -> Result<()> {
        if !self.finished {
            self.store.mark_clean_exit()?;
            self.store.sync_telemetry_best_effort();
            self.finished = true;
        }
        Ok(())
    }

    /// Record a normal process error as an abnormal exit.
    pub fn finish_error(&mut self, context: impl Into<String>) -> Result<()> {
        if !self.finished {
            let crash = CrashRecord::new(CrashType::ExitCodeError).with_context(context.into());
            self.store.record_crash(crash)?;
            self.store.sync_telemetry_best_effort();
            self.finished = true;
        }
        Ok(())
    }

    /// Finish based on the result returned by the guard entry point.
    pub fn finish_result(&mut self, result: &Result<()>) {
        let outcome = match result {
            Ok(()) => self.finish_clean(),
            Err(error) => self.finish_error(format!("{error:#}")),
        };
        if let Err(error) = outcome {
            eprintln!("icg_health_event event=finish_failed error={error:#}");
        }
    }

    /// Return the backing store used by this lifecycle.
    pub fn store(&self) -> &HealthStore {
        &self.store
    }
}

/// A small RAII-compatible owner for a cross-process health lock.
struct HealthLock {
    file: File,
}

impl Drop for HealthLock {
    fn drop(&mut self) {
        #[cfg(unix)]
        unsafe {
            libc::flock(std::os::unix::io::AsRawFd::as_raw_fd(&self.file), libc::LOCK_UN);
        }
    }
}

fn new_run_id() -> String {
    format!("run-{}-{}", Utc::now().timestamp_nanos_opt().unwrap_or_default(), std::process::id())
}

#[cfg(unix)]
fn process_is_alive(pid: u32) -> bool {
    // `/proc/<pid>` avoids sending a signal to a process that may not belong
    // to the guard.  It is sufficient to distinguish concurrent invocations
    // from a stale marker on Linux hosts where the guard runs.
    Path::new(&format!("/proc/{pid}")).exists()
}

#[cfg(not(unix))]
fn process_is_alive(_pid: u32) -> bool {
    false
}

impl HealthStore {
    /// Create a health store at the specified path.
    pub fn new(path: impl AsRef<Path>) -> Self {
        Self {
            path: path.as_ref().to_path_buf(),
        }
    }

    /// Create a store from `ICG_HEALTH_PATH`, falling back to the platform
    /// cache location.  The environment override keeps supervisors and tests
    /// from needing write access to a system directory.
    pub fn from_environment_or_default() -> Result<Self> {
        if let Some(path) = std::env::var_os("ICG_HEALTH_PATH").filter(|path| !path.is_empty()) {
            return Ok(Self::new(path));
        }
        Ok(Self::new(Self::default_path()?))
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

    fn lock_path(&self) -> PathBuf {
        let name = self
            .path
            .file_name()
            .map(|name| format!(".{}.lock", name.to_string_lossy()))
            .unwrap_or_else(|| ".health-state.lock".to_string());
        self.path
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .join(name)
    }

    fn acquire_lock(&self) -> Result<HealthLock> {
        self.ensure_parent_dir()?;
        let file = OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .open(self.lock_path())
            .with_context(|| format!("Failed to open health lock for {}", self.path.display()))?;

        #[cfg(unix)]
        {
            let result = unsafe {
                libc::flock(
                    std::os::unix::io::AsRawFd::as_raw_fd(&file),
                    libc::LOCK_EX,
                )
            };
            if result != 0 {
                return Err(std::io::Error::last_os_error())
                    .with_context(|| format!("Failed to lock health state {}", self.path.display()));
            }
        }

        Ok(HealthLock { file })
    }

    fn load_unlocked(&self) -> Result<HealthState> {
        if self.path.exists() {
            let content = std::fs::read_to_string(&self.path)
                .with_context(|| format!("Failed to read health state from {}", self.path.display()))?;

            let mut state: HealthState = serde_json::from_str(&content)
                .with_context(|| format!("Failed to parse health state from {}", self.path.display()))?;

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

    fn persist_unlocked(&self, state: &HealthState) -> Result<()> {
        self.ensure_parent_dir()?;

        let mut state = state.clone();
        state.schema_version = HEALTH_SCHEMA_VERSION;
        let content = serde_json::to_vec_pretty(&state).context("Failed to serialize health state")?;

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

        std::fs::write(&temp_path, content)
            .with_context(|| format!("Failed to write temporary health file {}", temp_path.display()))?;

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&temp_path, std::fs::Permissions::from_mode(0o600))
                .context("Failed to set health file permissions")?;
        }

        let temp_file = File::open(&temp_path)
            .with_context(|| format!("Failed to open temp file for syncing {}", temp_path.display()))?;
        temp_file
            .sync_all()
            .with_context(|| format!("Failed to sync temporary health file {}", temp_path.display()))?;

        std::fs::rename(&temp_path, &self.path)
            .with_context(|| format!("Failed to rename health file to {}", self.path.display()))?;

        if let Some(parent) = self.path.parent() {
            File::open(parent)
                .with_context(|| format!("Failed to open parent directory for syncing {}", parent.display()))?
                .sync_all()
                .with_context(|| format!("Failed to sync health directory {}", parent.display()))?;
        }

        Ok(())
    }

    /// Start a process run and persist its marker before the caller does work.
    ///
    /// A marker left by a dead prior process is converted into a single crash
    /// record.  A marker owned by a still-live PID is treated as a concurrent
    /// invocation and is not falsely counted as a crash.
    pub fn start_run(&self) -> Result<String> {
        let _lock = self.acquire_lock()?;
        let mut state = self.load_unlocked()?;

        if state.current_run_started_at.is_some() {
            let previous_is_live = state
                .current_run_pid
                .map(process_is_alive)
                .unwrap_or(false);
            if !previous_is_live {
                let detector = CrashDetector::new();
                let oom_killed = detector
                    .oom_killed_since(state.current_run_oom_kill_count)
                    .unwrap_or(false);
                let crash_type = if oom_killed {
                    CrashType::OutOfMemory
                } else {
                    CrashType::Unknown
                };
                let reason = if oom_killed {
                    "previous guard run was killed by its cgroup OOM monitor"
                } else {
                    "previous guard run exited without recording a clean exit"
                };
                let crash = CrashRecord::new(crash_type).with_context(reason.to_string());
                state.record_crash(crash);
                eprintln!(
                    "icg_health_event event=crash_detected crash_type={:?} reason=stale_run_marker",
                    crash_type
                );
            }
        }

        let run_id = new_run_id();
        state.mark_start_with_id(run_id.clone());
        self.persist_unlocked(&state)?;
        eprintln!("icg_health_event event=run_started run_id={run_id}");
        Ok(run_id)
    }

    /// Load health state from disk, or create new if file doesn't exist.
    pub fn load_or_create(&self) -> Result<HealthState> {
        let _lock = self.acquire_lock()?;
        self.load_unlocked()
    }

    /// Persist health state to disk atomically.
    pub fn persist(&self, state: &HealthState) -> Result<()> {
        let _lock = self.acquire_lock()?;
        self.persist_unlocked(state)
    }

    /// Update health state under one transaction.
    pub fn update<F>(&self, f: F) -> Result<HealthState>
    where
        F: FnOnce(&mut HealthState),
    {
        let _lock = self.acquire_lock()?;
        let mut state = self.load_unlocked()?;
        f(&mut state);
        self.persist_unlocked(&state)?;
        Ok(state)
    }

    /// Get the current health metrics.
    pub fn health_metrics(&self) -> Result<HealthMetrics> {
        Ok(self.load_or_create()?.compute_metrics())
    }

    /// Mark that the process has started.
    pub fn mark_start(&self) -> Result<()> {
        self.start_run()?;
        Ok(())
    }

    /// Record a clean process exit.
    pub fn mark_clean_exit(&self) -> Result<()> {
        self.update(HealthState::mark_clean_exit)?;
        Ok(())
    }

    /// Record a crash event.
    pub fn record_crash(&self, crash: CrashRecord) -> Result<()> {
        let crash_type = crash.crash_type;
        let signal = crash.signal;
        let exit_code = crash.exit_code;
        self.update(|state| state.record_crash(crash))?;
        eprintln!(
            "icg_health_event event=crash_recorded crash_type={:?} signal={signal:?} exit_code={exit_code:?}",
            crash_type
        );
        Ok(())
    }

    /// Record a child process's abnormal exit, including supervisor-provided
    /// OOM evidence.
    pub fn record_exit_status(&self, status: &ExitStatus, oom_killed: bool) -> Result<bool> {
        let Some(crash) = CrashRecord::from_exit_status(status, oom_killed) else {
            self.mark_clean_exit()?;
            return Ok(false);
        };
        self.record_crash(crash)?;
        Ok(true)
    }

    /// Persist a heartbeat for a running guard process.
    pub fn heartbeat(&self) -> Result<()> {
        self.update(|state| state.heartbeat())?;
        Ok(())
    }

    /// Copy the latest health snapshot into the existing rolling telemetry
    /// store.  Health tracking must never prevent the guard from making its
    /// normal allow/deny decision, so callers use this best-effort helper at
    /// lifecycle boundaries.
    pub fn sync_telemetry_best_effort(&self) {
        let telemetry_path = std::env::var_os("ICG_TELEMETRY_PATH")
            .filter(|path| !path.is_empty())
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("/var/cache/icg/telemetry.json"));

        let result = (|| -> Result<()> {
            let metrics = self.health_metrics()?;
            let mut telemetry = crate::telemetry::TelemetryStore::load_or_create(telemetry_path)?;
            telemetry.record_health_metrics(&metrics);
            telemetry.persist()
        })();

        if let Err(error) = result {
            eprintln!("icg_health_event event=telemetry_sync_failed error={error:#}");
        }
    }

    /// Clear all health data (useful for testing or reset).
    pub fn clear(&self) -> Result<()> {
        let _lock = self.acquire_lock()?;
        self.persist_unlocked(&HealthState::new())?;
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

    #[test]
    fn stale_run_marker_is_recorded_as_a_crash_on_next_start() -> Result<()> {
        let dir = tempdir()?;
        let store = test_store(dir.path());
        let mut state = HealthState::new();
        state.mark_start();
        // Use a PID that cannot be this test process. This models an OOM,
        // SIGSEGV, or SIGABRT that prevented the clean-exit write.
        state.current_run_pid = Some(u32::MAX);
        store.persist(&state)?;

        store.start_run()?;
        let recovered = store.load_or_create()?;
        assert_eq!(recovered.total_crashes, 1);
        assert_eq!(recovered.crash_history[0].crash_type, CrashType::Unknown);
        assert_eq!(recovered.consecutive_clean_runs, 0);
        assert!(recovered.current_run_started_at.is_some());
        Ok(())
    }

    #[test]
    fn clean_exit_clears_durable_run_marker() -> Result<()> {
        let dir = tempdir()?;
        let store = test_store(dir.path());

        store.start_run()?;
        assert!(store.load_or_create()?.current_run_id.is_some());
        store.mark_clean_exit()?;

        let state = store.load_or_create()?;
        assert!(state.current_run_id.is_none());
        assert!(state.current_run_heartbeat_at.is_none());
        assert_eq!(state.consecutive_clean_runs, 1);
        assert!(state.last_clean_exit_at.is_some());
        Ok(())
    }

    #[test]
    fn exit_status_classifies_signals_and_oom_separately() {
        #[cfg(unix)]
        {
            use std::os::unix::process::ExitStatusExt;

            let segfault = ExitStatus::from_raw(libc::SIGSEGV);
            assert_eq!(
                CrashType::from_exit_status(&segfault, false),
                Some(CrashType::SegmentationFault)
            );

            let killed = ExitStatus::from_raw(libc::SIGKILL);
            assert_eq!(
                CrashType::from_exit_status(&killed, true),
                Some(CrashType::OutOfMemory)
            );
            assert_eq!(
                CrashType::from_exit_status(&killed, false),
                Some(CrashType::Unknown)
            );
        }
    }

    #[test]
    fn cgroup_oom_counter_provides_evidence_for_sigkill() -> Result<()> {
        let dir = tempdir()?;
        let events = dir.path().join("memory.events");
        std::fs::write(&events, "low 0\nome 0\noom_kill 3\n")?;
        let detector = CrashDetector::with_oom_events_path(&events);
        assert_eq!(detector.oom_kill_count()?, Some(3));

        std::fs::write(&events, "low 0\nome 0\noom_kill 4\n")?;
        assert!(detector.oom_killed_since(Some(3))?);

        #[cfg(unix)]
        {
            use std::os::unix::process::ExitStatusExt;
            let killed = ExitStatus::from_raw(libc::SIGKILL);
            assert_eq!(
                detector.classify_exit_status(&killed)?.map(|record| record.crash_type),
                Some(CrashType::OutOfMemory)
            );
        }
        Ok(())
    }
}
