//! Durable runtime state for cross-invocation enforcement and rollback.
//!
//! The state store is deliberately separate from [`crate::trust_pointer`].
//! The trust-pointer file is a root-owned security boundary and remains the
//! source of truth for what a host may load. This store records the runtime
//! history needed by later enforcement components: session ordering markers,
//! deny events, per-release deny-rate aggregates, the previous trust
//! reference, and rollback metadata.
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

/// Maximum number of release aggregates retained by a default store.
///
/// Release aggregates, rather than individual command evaluations, are the
/// rolling baseline. Keeping a bounded number of releases makes the baseline
/// stable across process invocations without allowing telemetry to grow
/// without limit.
pub const DEFAULT_MAX_RELEASE_TELEMETRY: usize = 32;

/// Minimum number of prior releases needed before a deny-rate deviation is
/// suitable for an automated consumer such as poison-pill rollback.
pub const DEFAULT_MIN_BASELINE_RELEASES: usize = 3;

/// Minimum number of evaluations for the current release before its rate is
/// treated as representative of the release rather than a small sample.
pub const DEFAULT_MIN_CURRENT_EVALUATIONS: u64 = 100;

/// Minimum total evaluations represented by prior releases before their
/// aggregate is eligible as a baseline.
pub const DEFAULT_MIN_BASELINE_EVALUATIONS: u64 = 300;

/// Minimum absolute deny-rate increase required by the conservative default
/// policy. This prevents a tiny baseline from turning ordinary noise into a
/// release-health incident.
pub const DEFAULT_MIN_ABSOLUTE_DEVIATION: f64 = 0.05;

/// Number of baseline standard deviations required by the conservative
/// default policy in addition to the absolute deviation floor.
pub const DEFAULT_BASELINE_SIGMA_MULTIPLIER: f64 = 3.0;

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

/// Per-release aggregate of evaluation and denial counts.
///
/// A release record is updated transactionally for every evaluation. The
/// aggregate is persisted in the state store, so a fresh hook process can
/// continue the same release's denominator and compare it with prior releases.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ReleaseTelemetryRecord {
    /// Release reference loaded when the evaluation occurred.
    pub release_ref: String,

    /// UTC timestamp of the first evaluation observed for this release.
    pub first_seen_at: String,

    /// UTC timestamp of the most recent evaluation observed for this release.
    pub last_seen_at: String,

    /// Number of evaluations observed for this release.
    #[serde(default)]
    pub evaluation_count: u64,

    /// Number of denied evaluations observed for this release.
    #[serde(default)]
    pub deny_count: u64,

    /// Deny rate (`deny_count / evaluation_count`).
    ///
    /// This is stored as a convenience for consumers inspecting the JSON
    /// state file. It is recomputed whenever the record is updated and when a
    /// state file is loaded, so it cannot become stale through normal use.
    #[serde(default)]
    pub deny_rate: f64,
}

impl ReleaseTelemetryRecord {
    fn new(release_ref: impl Into<String>, timestamp: String, denied: bool) -> Self {
        let mut record = Self {
            release_ref: release_ref.into(),
            first_seen_at: timestamp.clone(),
            last_seen_at: timestamp,
            evaluation_count: 1,
            deny_count: u64::from(denied),
            deny_rate: 0.0,
        };
        record.recompute_rate();
        record
    }

    fn recompute_rate(&mut self) {
        self.deny_rate = if self.evaluation_count == 0 {
            0.0
        } else {
            self.deny_count as f64 / self.evaluation_count as f64
        };
    }

    /// Return this release's current deny rate.
    pub fn rate(&self) -> f64 {
        self.deny_rate
    }
}

/// Statistics for the rolling baseline of release deny rates.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RollingDenyRateBaseline {
    /// Number of release aggregates included in the baseline.
    pub release_count: usize,

    /// Sum of evaluations represented by the baseline releases.
    pub evaluation_count: u64,

    /// Sum of denials represented by the baseline releases.
    pub deny_count: u64,

    /// Mean of the per-release deny rates. Each release contributes one rate,
    /// so a high-volume release cannot drown out every smaller release.
    pub mean_deny_rate: f64,

    /// Population standard deviation of per-release deny rates.
    pub std_dev: f64,

    /// Lowest per-release deny rate in the baseline.
    pub min_deny_rate: f64,

    /// Highest per-release deny rate in the baseline.
    pub max_deny_rate: f64,

    /// First observation timestamp in the baseline.
    pub window_start: Option<String>,

    /// Most recent observation timestamp in the baseline.
    pub window_end: Option<String>,
}

impl Default for RollingDenyRateBaseline {
    fn default() -> Self {
        Self {
            release_count: 0,
            evaluation_count: 0,
            deny_count: 0,
            mean_deny_rate: 0.0,
            std_dev: 0.0,
            min_deny_rate: 0.0,
            max_deny_rate: 0.0,
            window_start: None,
            window_end: None,
        }
    }
}

/// The current release's signed deviation from its prior-release baseline.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DenyRateDeviation {
    /// Release whose current aggregate was compared.
    pub release_ref: String,

    /// Current release deny rate.
    pub current_deny_rate: f64,

    /// Current release evaluation count.
    pub current_evaluation_count: u64,

    /// Current release denial count.
    pub current_deny_count: u64,

    /// Baseline made from other retained releases.
    pub baseline: RollingDenyRateBaseline,

    /// Signed difference `current_deny_rate - baseline.mean_deny_rate`.
    pub absolute_deviation: f64,

    /// Signed relative difference where the baseline mean is non-zero.
    /// `None` means the baseline mean is zero and a ratio would be undefined.
    pub relative_deviation: Option<f64>,
}

impl DenyRateDeviation {
    /// Return whether the baseline contains enough history to be considered.
    pub fn has_minimum_baseline(&self, minimum_releases: usize) -> bool {
        self.baseline.release_count >= minimum_releases
    }

    /// Apply a conservative poison-pill-style threshold.
    ///
    /// The check requires all of: enough current-release observations, enough
    /// prior releases and baseline volume, a meaningful absolute increase, and
    /// a rate above the baseline's sigma threshold. A single noisy day
    /// therefore cannot trigger a release-health action by itself.
    pub fn is_concerning(&self, policy: &DenyRatePolicy) -> bool {
        self.current_evaluation_count >= policy.minimum_current_evaluations
            && self.has_minimum_baseline(policy.minimum_baseline_releases)
            && self.baseline.evaluation_count >= policy.minimum_baseline_evaluations
            && self.absolute_deviation >= policy.minimum_absolute_deviation
            && self.current_deny_rate
                > self.baseline.mean_deny_rate
                    + self.baseline.std_dev * policy.baseline_sigma_multiplier
    }
}

/// Conservative policy for consumers deciding whether a deviation is
/// significant enough to act on.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DenyRatePolicy {
    /// Minimum prior releases required for a baseline.
    pub minimum_baseline_releases: usize,

    /// Minimum current-release evaluations required for comparison.
    pub minimum_current_evaluations: u64,

    /// Minimum total evaluations represented by prior releases.
    pub minimum_baseline_evaluations: u64,

    /// Minimum absolute increase in deny rate (0.05 means five percentage
    /// points).
    pub minimum_absolute_deviation: f64,

    /// Number of baseline standard deviations required above the mean.
    pub baseline_sigma_multiplier: f64,
}

impl Default for DenyRatePolicy {
    fn default() -> Self {
        Self {
            minimum_baseline_releases: DEFAULT_MIN_BASELINE_RELEASES,
            minimum_current_evaluations: DEFAULT_MIN_CURRENT_EVALUATIONS,
            minimum_baseline_evaluations: DEFAULT_MIN_BASELINE_EVALUATIONS,
            minimum_absolute_deviation: DEFAULT_MIN_ABSOLUTE_DEVIATION,
            baseline_sigma_multiplier: DEFAULT_BASELINE_SIGMA_MULTIPLIER,
        }
    }
}

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

    /// Bounded per-release evaluation and denial aggregates used to build the
    /// rolling release-health baseline.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub release_telemetry: Vec<ReleaseTelemetryRecord>,

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
            release_telemetry: Vec::new(),
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

    /// Record one evaluation for a release and return its updated aggregate.
    pub fn record_release_evaluation(
        &mut self,
        release_ref: impl Into<String>,
        denied: bool,
    ) -> ReleaseTelemetryRecord {
        let release_ref = release_ref.into();
        let timestamp = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Nanos, true);

        if let Some(record) = self
            .release_telemetry
            .iter_mut()
            .find(|record| record.release_ref == release_ref)
        {
            record.last_seen_at = timestamp;
            record.evaluation_count = record.evaluation_count.saturating_add(1);
            if denied {
                record.deny_count = record.deny_count.saturating_add(1);
            }
            record.recompute_rate();
            return record.clone();
        }

        let record = ReleaseTelemetryRecord::new(release_ref, timestamp, denied);
        self.release_telemetry.push(record.clone());
        record
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
    /// Maximum number of release aggregates retained by this store.
    max_release_telemetry: usize,
}

impl StateStore {
    /// Create a state store using the default deny-history retention limit.
    pub fn new(path: impl AsRef<Path>) -> Self {
        Self {
            path: path.as_ref().to_path_buf(),
            max_deny_history: DEFAULT_MAX_DENY_HISTORY,
            max_release_telemetry: DEFAULT_MAX_RELEASE_TELEMETRY,
        }
    }

    /// Create a state store with an explicit deny-history retention limit.
    pub fn with_max_deny_history(path: impl AsRef<Path>, max_deny_history: usize) -> Self {
        Self {
            path: path.as_ref().to_path_buf(),
            max_deny_history: max_deny_history.max(1),
            max_release_telemetry: DEFAULT_MAX_RELEASE_TELEMETRY,
        }
    }

    /// Create a state store with explicit retention limits for both deny
    /// events and per-release telemetry.
    pub fn with_limits(
        path: impl AsRef<Path>,
        max_deny_history: usize,
        max_release_telemetry: usize,
    ) -> Self {
        Self {
            path: path.as_ref().to_path_buf(),
            max_deny_history: max_deny_history.max(1),
            max_release_telemetry: max_release_telemetry.max(1),
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

    /// Return the configured per-release telemetry retention limit.
    pub fn max_release_telemetry(&self) -> usize {
        self.max_release_telemetry
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
        for record in &mut state.release_telemetry {
            record.recompute_rate();
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

    /// Record one allow/deny evaluation for a release.
    ///
    /// Empty references are rejected because they cannot be attributed to a
    /// release. Callers without a trusted release should omit telemetry rather
    /// than creating an `unknown` bucket that would contaminate the baseline.
    pub fn record_release_evaluation(
        &self,
        release_ref: &str,
        denied: bool,
    ) -> Result<ReleaseTelemetryRecord> {
        if release_ref.trim().is_empty() {
            bail!("release reference must not be empty");
        }

        let record = self.update(|state| {
            state.record_release_evaluation(release_ref, denied);
            if state.release_telemetry.len() > self.max_release_telemetry {
                let oldest_index = state
                    .release_telemetry
                    .iter()
                    .enumerate()
                    .min_by(|(_, left), (_, right)| left.last_seen_at.cmp(&right.last_seen_at))
                    .map(|(index, _)| index);
                if let Some(index) = oldest_index {
                    state.release_telemetry.remove(index);
                }
            }
        })?;

        record
            .release_telemetry
            .into_iter()
            .find(|entry| entry.release_ref == release_ref)
            .context("release telemetry record disappeared during update")
    }

    /// Return all retained per-release telemetry records in storage order.
    pub fn release_telemetry(&self) -> Result<Vec<ReleaseTelemetryRecord>> {
        Ok(self.load()?.release_telemetry)
    }

    /// Return the aggregate for one release, if it has been observed.
    pub fn release_telemetry_for(
        &self,
        release_ref: &str,
    ) -> Result<Option<ReleaseTelemetryRecord>> {
        Ok(self
            .release_telemetry()?
            .into_iter()
            .find(|record| record.release_ref == release_ref))
    }

    /// Compute a rolling baseline from the retained release aggregates.
    pub fn rolling_deny_rate_baseline(&self) -> Result<RollingDenyRateBaseline> {
        Ok(compute_release_baseline(&self.release_telemetry()?))
    }

    /// Compare one release with the rolling baseline made from all other
    /// retained releases.
    pub fn deny_rate_deviation_for(&self, release_ref: &str) -> Result<Option<DenyRateDeviation>> {
        let records = self.release_telemetry()?;
        let Some(current) = records
            .iter()
            .find(|record| record.release_ref == release_ref)
        else {
            return Ok(None);
        };

        let baseline_records = records
            .iter()
            .filter(|record| record.release_ref != release_ref)
            .cloned()
            .collect::<Vec<_>>();
        Ok(Some(make_deviation(
            current,
            &compute_release_baseline(&baseline_records),
        )))
    }

    /// Return the deviation for the most recently observed release.
    pub fn current_deny_rate_deviation(&self) -> Result<Option<DenyRateDeviation>> {
        let records = self.release_telemetry()?;
        let Some(current) = records
            .iter()
            .max_by(|left, right| left.last_seen_at.cmp(&right.last_seen_at))
        else {
            return Ok(None);
        };

        let baseline_records = records
            .iter()
            .filter(|record| record.release_ref != current.release_ref)
            .cloned()
            .collect::<Vec<_>>();
        Ok(Some(make_deviation(
            current,
            &compute_release_baseline(&baseline_records),
        )))
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

/// Compute rolling statistics from per-release telemetry records.
pub fn compute_rolling_deny_rate_baseline(
    records: &[ReleaseTelemetryRecord],
) -> RollingDenyRateBaseline {
    if records.is_empty() {
        return RollingDenyRateBaseline::default();
    }

    let release_count = records.len();
    let evaluation_count = records.iter().fold(0_u64, |total, record| {
        total.saturating_add(record.evaluation_count)
    });
    let deny_count = records.iter().fold(0_u64, |total, record| {
        total.saturating_add(record.deny_count)
    });
    let rates = records
        .iter()
        .map(ReleaseTelemetryRecord::rate)
        .collect::<Vec<_>>();
    let mean_deny_rate = rates.iter().sum::<f64>() / release_count as f64;
    let variance = rates
        .iter()
        .map(|rate| {
            let difference = rate - mean_deny_rate;
            difference * difference
        })
        .sum::<f64>()
        / release_count as f64;

    RollingDenyRateBaseline {
        release_count,
        evaluation_count,
        deny_count,
        mean_deny_rate,
        std_dev: variance.sqrt(),
        min_deny_rate: rates.iter().copied().fold(f64::INFINITY, f64::min),
        max_deny_rate: rates.iter().copied().fold(f64::NEG_INFINITY, f64::max),
        window_start: records
            .iter()
            .map(|record| record.first_seen_at.clone())
            .min(),
        window_end: records
            .iter()
            .map(|record| record.last_seen_at.clone())
            .max(),
    }
}

fn compute_release_baseline(records: &[ReleaseTelemetryRecord]) -> RollingDenyRateBaseline {
    compute_rolling_deny_rate_baseline(records)
}

fn make_deviation(
    current: &ReleaseTelemetryRecord,
    baseline: &RollingDenyRateBaseline,
) -> DenyRateDeviation {
    let absolute_deviation = current.deny_rate - baseline.mean_deny_rate;
    let relative_deviation =
        (baseline.mean_deny_rate > 0.0).then(|| absolute_deviation / baseline.mean_deny_rate);

    DenyRateDeviation {
        release_ref: current.release_ref.clone(),
        current_deny_rate: current.deny_rate,
        current_evaluation_count: current.evaluation_count,
        current_deny_count: current.deny_count,
        baseline: baseline.clone(),
        absolute_deviation,
        relative_deviation,
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

    fn record_release(store: &StateStore, release_ref: &str, total: u64, denials: u64) {
        for index in 0..total {
            store
                .record_release_evaluation(release_ref, index < denials)
                .unwrap();
        }
    }

    #[test]
    fn release_telemetry_persists_counts_and_rate() -> Result<()> {
        let dir = tempdir()?;
        let path = dir.path().join("session-state.json");
        let store = StateStore::new(&path);

        record_release(&store, "v1.2.3", 10, 2);

        let reopened = StateStore::new(&path);
        let record = reopened
            .release_telemetry_for("v1.2.3")?
            .expect("release record");
        assert_eq!(record.evaluation_count, 10);
        assert_eq!(record.deny_count, 2);
        assert!((record.rate() - 0.2).abs() < f64::EPSILON);
        assert_eq!(reopened.load()?.release_telemetry.len(), 1);
        Ok(())
    }

    #[test]
    fn release_telemetry_uses_prior_releases_for_deviation() -> Result<()> {
        let dir = tempdir()?;
        let store = test_store(dir.path());

        record_release(&store, "v1", 100, 1);
        record_release(&store, "v2", 100, 2);
        record_release(&store, "v3", 100, 1);
        record_release(&store, "v4", 100, 20);

        let deviation = store
            .deny_rate_deviation_for("v4")?
            .expect("current release deviation");
        assert_eq!(deviation.baseline.release_count, 3);
        assert_eq!(deviation.baseline.evaluation_count, 300);
        assert_eq!(deviation.current_evaluation_count, 100);
        assert!((deviation.current_deny_rate - 0.2).abs() < f64::EPSILON);
        assert!(deviation.absolute_deviation > 0.18);
        assert!(deviation.is_concerning(&DenyRatePolicy::default()));

        let current = store
            .current_deny_rate_deviation()?
            .expect("latest release deviation");
        assert_eq!(current.release_ref, "v4");

        let baseline = store.rolling_deny_rate_baseline()?;
        assert_eq!(baseline.release_count, 4);
        assert_eq!(baseline.evaluation_count, 400);
        Ok(())
    }

    #[test]
    fn conservative_policy_rejects_small_samples_and_small_deltas() -> Result<()> {
        let dir = tempdir()?;
        let store = test_store(dir.path());
        record_release(&store, "v1", 100, 1);
        record_release(&store, "v2", 100, 1);
        record_release(&store, "v3", 100, 1);

        record_release(&store, "small-sample", 10, 10);
        let small_sample = store
            .deny_rate_deviation_for("small-sample")?
            .expect("small sample deviation");
        assert!(!small_sample.is_concerning(&DenyRatePolicy::default()));

        record_release(&store, "ordinary-noise", 100, 4);
        let ordinary_noise = store
            .deny_rate_deviation_for("ordinary-noise")?
            .expect("ordinary noise deviation");
        assert!(ordinary_noise.absolute_deviation < 0.05);
        assert!(!ordinary_noise.is_concerning(&DenyRatePolicy::default()));
        Ok(())
    }

    #[test]
    fn release_telemetry_retention_is_bounded() -> Result<()> {
        let dir = tempdir()?;
        let path = dir.path().join("session-state.json");
        let store = StateStore::with_limits(&path, 10, 2);
        record_release(&store, "v1", 1, 0);
        record_release(&store, "v2", 1, 0);
        record_release(&store, "v3", 1, 0);

        let records = store.release_telemetry()?;
        assert_eq!(records.len(), 2);
        assert!(!records.iter().any(|record| record.release_ref == "v1"));
        assert!(records.iter().any(|record| record.release_ref == "v2"));
        assert!(records.iter().any(|record| record.release_ref == "v3"));
        Ok(())
    }
}
