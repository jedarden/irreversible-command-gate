//! Durable graduation policy for guard-availability failures.
//!
//! The guard starts fail-open.  A release can contribute to graduation only
//! after the existing per-release deny-rate telemetry has a complete enough
//! observation and the poison-pill detector says that the release is healthy.
//! This module deliberately consumes that typed signal; it does not maintain a
//! second deny-rate detector.
//!
//! The policy file is separate from the evaluation state store because it is a
//! deployment decision.  In production it belongs in `/etc/icg`, where the
//! guarded process can read it but cannot rewrite its own fail-closed policy.
//! Operators use the policy CLI (or an equivalent deployment controller) to
//! reconcile and promote the durable state.

use anyhow::{bail, Context, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

use crate::rollback::PoisonPillConfig;
use crate::state_store::StateStore;
use crate::trust_pointer::TrustPointerStore;

/// Current fail-closed policy file schema.
pub const POLICY_SCHEMA_VERSION: u32 = 1;

/// Number of eligible clean releases required for graduation.
///
/// Three releases is intentionally conservative while keeping the policy
/// useful for a release-driven deployment.  The value is persisted with the
/// state and can be changed only through an explicit requalification reset.
pub const DEFAULT_GRADUATION_THRESHOLD: u32 = 3;

/// Environment variable used to point a deployment at its policy file.
pub const POLICY_PATH_ENV: &str = "ICG_FAIL_CLOSED_POLICY";

/// Environment variable retained as a local/test override for the old
/// fail-closed switch.  It can only make the process stricter; `false` does
/// not demote a durable Fail-Closed policy.
pub const LEGACY_FAIL_CLOSED_ENV: &str = "ICG_FAIL_CLOSED";

const MAX_COUNTED_RELEASES: usize = 128;
/// Maximum number of structured policy transition events retained locally.
///
/// The policy snapshot is also the graduation telemetry sink.  Keep a bounded
/// event tail so repeated hook invocations cannot grow the administrator
/// controlled file without limit.
pub const MAX_POLICY_EVENTS: usize = 256;

/// Fleet policy state loaded by each guard invocation.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PolicyMode {
    /// Guard-availability failures allow the operation and emit an alert.
    #[default]
    FailOpen,
    /// Guard-availability failures deny the operation.
    FailClosed,
}

/// Structured policy events emitted for logging and telemetry consumers.
///
/// These events describe the policy decision only.  Poison-pill detection and
/// rollback remain owned by [`crate::rollback`]; this module records the
/// typed evidence it observes and never acknowledges or edits that source.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PolicyEventType {
    CleanRelease,
    Graduated,
    ManualGraduation,
    PoisonPill,
    QualificationInvalidated,
    EmergencyDemotion,
    ThresholdChanged,
}

/// One durable policy transition event.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PolicyEvent {
    /// Unique local event identifier for log correlation.
    pub event_id: String,
    /// UTC time at which the transition was committed.
    pub occurred_at: String,
    pub event_type: PolicyEventType,
    pub generation: u64,
    pub mode: PolicyMode,
    pub clean_release_streak: u32,
    pub graduation_threshold: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub release_ref: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub poison_pill_event_ref: Option<String>,
    pub reason: String,
}

impl PolicyMode {
    pub fn is_fail_closed(self) -> bool {
        matches!(self, Self::FailClosed)
    }
}

/// A durable state transition outcome.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PolicyTransition {
    /// No state change was needed (usually a duplicate release observation).
    NoChange,
    /// A new eligible release advanced the clean streak.
    CleanRelease {
        release_ref: String,
        clean_streak: u32,
    },
    /// The threshold was reached and fail-closed was committed.
    Graduated {
        release_ref: String,
        generation: u64,
    },
    /// An explicitly authorized operator graduation.
    ForcedGraduation { generation: u64 },
    /// A poison-pill event invalidated qualification.
    PoisonPill {
        event_ref: String,
        clean_streak: u32,
    },
    /// Release evidence was incomplete or the detector was not armed.
    Invalidated { clean_streak: u32 },
    /// An explicit operator demotion invalidated qualification.
    EmergencyDemotion { generation: u64 },
}

/// Persisted policy state.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PolicyState {
    #[serde(default = "default_policy_schema_version")]
    pub schema_version: u32,

    /// Monotonically increasing policy generation for runtime reporting.
    #[serde(default)]
    pub generation: u64,

    #[serde(default)]
    pub mode: PolicyMode,

    /// Positive number of eligible, unique clean releases required.
    #[serde(default = "default_graduation_threshold")]
    pub graduation_threshold: u32,

    /// Current consecutive eligible clean-release streak.
    #[serde(default)]
    pub clean_release_streak: u32,

    /// Release references already counted in the streak.  This makes
    /// reconciliation idempotent across repeated hook invocations.
    #[serde(default)]
    pub counted_releases: Vec<String>,

    /// Last poison-pill event consumed by this policy.  The event reference is
    /// opaque to this module and is normally the durable rollback count.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_poison_pill_event: Option<String>,

    /// Last rollback count observed in the poison-pill state store.
    #[serde(default)]
    pub last_processed_rollback_count: u64,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_transition_at: Option<String>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_transition_reason: Option<String>,

    /// Bounded, structured transition telemetry for audit and monitoring.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub events: Vec<PolicyEvent>,
}

fn default_policy_schema_version() -> u32 {
    POLICY_SCHEMA_VERSION
}

fn default_graduation_threshold() -> u32 {
    DEFAULT_GRADUATION_THRESHOLD
}

impl Default for PolicyState {
    fn default() -> Self {
        Self::new(DEFAULT_GRADUATION_THRESHOLD).expect("default graduation threshold is valid")
    }
}

impl PolicyState {
    /// Create a fresh fail-open policy.
    pub fn new(graduation_threshold: u32) -> Result<Self> {
        if graduation_threshold == 0 {
            bail!("graduation threshold must be greater than zero");
        }

        Ok(Self {
            schema_version: POLICY_SCHEMA_VERSION,
            generation: 0,
            mode: PolicyMode::FailOpen,
            graduation_threshold,
            clean_release_streak: 0,
            counted_releases: Vec::new(),
            last_poison_pill_event: None,
            last_processed_rollback_count: 0,
            last_transition_at: None,
            last_transition_reason: None,
            events: Vec::new(),
        })
    }

    /// Whether the state currently denies when the guard is unavailable.
    pub fn is_fail_closed(&self) -> bool {
        self.mode.is_fail_closed()
    }

    fn validate(&mut self) -> Result<()> {
        if self.schema_version > POLICY_SCHEMA_VERSION {
            bail!(
                "policy schema version {} is newer than supported version {}",
                self.schema_version,
                POLICY_SCHEMA_VERSION
            );
        }
        if self.schema_version == 0 {
            self.schema_version = POLICY_SCHEMA_VERSION;
        }
        if self.graduation_threshold == 0 {
            bail!("policy graduation threshold must be greater than zero");
        }
        self.clean_release_streak = self.clean_release_streak.min(self.graduation_threshold);
        self.counted_releases
            .retain(|release| !release.trim().is_empty());
        if self.counted_releases.len() > MAX_COUNTED_RELEASES {
            let first = self.counted_releases.len() - MAX_COUNTED_RELEASES;
            self.counted_releases.drain(..first);
        }
        if self.events.len() > MAX_POLICY_EVENTS {
            let first = self.events.len() - MAX_POLICY_EVENTS;
            self.events.drain(..first);
        }
        Ok(())
    }

    fn transition(
        &mut self,
        event_type: PolicyEventType,
        release_ref: Option<String>,
        poison_pill_event_ref: Option<String>,
        reason: impl Into<String>,
    ) -> u64 {
        let reason = reason.into();
        self.generation = self.generation.saturating_add(1);
        let occurred_at = Utc::now().to_rfc3339();
        self.last_transition_at = Some(occurred_at.clone());
        self.last_transition_reason = Some(reason.clone());
        self.events.push(PolicyEvent {
            event_id: format!("policy-{}-{}", self.generation, std::process::id()),
            occurred_at,
            event_type,
            generation: self.generation,
            mode: self.mode,
            clean_release_streak: self.clean_release_streak,
            graduation_threshold: self.graduation_threshold,
            release_ref,
            poison_pill_event_ref,
            reason,
        });
        if self.events.len() > MAX_POLICY_EVENTS {
            let first = self.events.len() - MAX_POLICY_EVENTS;
            self.events.drain(..first);
        }
        self.generation
    }

    /// Record one new eligible clean release.
    ///
    /// A release reference is counted at most once.  Reconciliation may call
    /// this method after every evaluation without inflating the streak.
    pub fn record_clean_release(
        &mut self,
        release_ref: impl Into<String>,
    ) -> Result<PolicyTransition> {
        let release_ref = release_ref.into();
        if release_ref.trim().is_empty() {
            bail!("cannot count an empty release reference");
        }
        if self
            .counted_releases
            .iter()
            .any(|value| value == &release_ref)
        {
            return Ok(PolicyTransition::NoChange);
        }
        if self.mode == PolicyMode::FailClosed {
            return Ok(PolicyTransition::NoChange);
        }

        self.counted_releases.push(release_ref.clone());
        if self.counted_releases.len() > MAX_COUNTED_RELEASES {
            self.counted_releases.remove(0);
        }

        self.clean_release_streak = self.clean_release_streak.saturating_add(1);
        if self.clean_release_streak >= self.graduation_threshold {
            self.mode = PolicyMode::FailClosed;
            let generation = self.transition(
                PolicyEventType::Graduated,
                Some(release_ref.clone()),
                None,
                format!(
                    "graduated after {} consecutive eligible clean releases; latest={release_ref}",
                    self.clean_release_streak
                ),
            );
            Ok(PolicyTransition::Graduated {
                release_ref,
                generation,
            })
        } else {
            self.transition(
                PolicyEventType::CleanRelease,
                Some(release_ref.clone()),
                None,
                format!("eligible clean release counted; release={release_ref}"),
            );
            Ok(PolicyTransition::CleanRelease {
                release_ref,
                clean_streak: self.clean_release_streak,
            })
        }
    }

    /// Consume a poison-pill event and invalidate open-mode qualification.
    ///
    /// Once Fail-Closed has been committed, a bad rule-pack release is rolled
    /// back by the existing poison-pill mechanism but does not silently lower
    /// the guard-availability policy.  Re-consuming the same event is a no-op.
    pub fn record_poison_pill(&mut self, event_ref: impl Into<String>) -> Result<PolicyTransition> {
        let event_ref = event_ref.into();
        if event_ref.trim().is_empty() {
            bail!("cannot record an empty poison-pill event reference");
        }
        if self.last_poison_pill_event.as_deref() == Some(event_ref.as_str()) {
            return Ok(PolicyTransition::NoChange);
        }

        self.last_poison_pill_event = Some(event_ref.clone());
        if self.mode == PolicyMode::FailClosed {
            self.transition(
                PolicyEventType::PoisonPill,
                None,
                Some(event_ref.clone()),
                format!(
                    "poison-pill event consumed while preserving Fail-Closed mode; event={event_ref}"
                ),
            );
            return Ok(PolicyTransition::PoisonPill {
                event_ref,
                clean_streak: self.clean_release_streak,
            });
        }

        self.clean_release_streak = 0;
        self.counted_releases.clear();
        self.transition(
            PolicyEventType::PoisonPill,
            None,
            Some(event_ref.clone()),
            format!("poison-pill event reset qualification; event={event_ref}"),
        );
        Ok(PolicyTransition::PoisonPill {
            event_ref,
            clean_streak: 0,
        })
    }

    /// Explicitly demote a graduated policy during an incident.
    pub fn emergency_demote(&mut self, reason: impl Into<String>) -> PolicyTransition {
        self.mode = PolicyMode::FailOpen;
        self.clean_release_streak = 0;
        self.counted_releases.clear();
        self.transition(
            PolicyEventType::EmergencyDemotion,
            None,
            None,
            format!("emergency demotion: {}", reason.into()),
        );
        PolicyTransition::EmergencyDemotion {
            generation: self.generation,
        }
    }

    /// Explicitly force Fail-Closed from the administrator control plane.
    ///
    /// This is intentionally separate from automatic graduation and leaves
    /// the observed streak intact for audit.  It is an out-of-band override;
    /// callers must supply an incident/change reason and should use it only
    /// after the deployment gates described in the transition design have
    /// been satisfied.
    pub fn force_graduate(&mut self, reason: impl Into<String>) -> Result<PolicyTransition> {
        let reason = reason.into();
        if reason.trim().is_empty() {
            bail!("forced graduation requires a non-empty reason");
        }
        if self.mode == PolicyMode::FailClosed {
            return Ok(PolicyTransition::NoChange);
        }
        self.mode = PolicyMode::FailClosed;
        let generation = self.transition(
            PolicyEventType::ManualGraduation,
            None,
            None,
            format!("manual force graduation: {reason}"),
        );
        Ok(PolicyTransition::ForcedGraduation { generation })
    }

    /// Reset qualification without pretending that an invalid/incomplete
    /// observation was a poison-pill event.
    pub fn invalidate_qualification(&mut self, reason: impl Into<String>) -> PolicyTransition {
        let reason = reason.into();
        let open_reason = format!("qualification invalidated: {reason}");
        let closed_reason =
            format!("qualification invalidated while preserving Fail-Closed mode: {reason}");
        if self.last_transition_reason.as_deref() == Some(open_reason.as_str())
            || self.last_transition_reason.as_deref() == Some(closed_reason.as_str())
        {
            return PolicyTransition::NoChange;
        }
        if self.mode == PolicyMode::FailClosed {
            self.transition(
                PolicyEventType::QualificationInvalidated,
                None,
                None,
                closed_reason,
            );
            return PolicyTransition::Invalidated {
                clean_streak: self.clean_release_streak,
            };
        }

        self.clean_release_streak = 0;
        self.counted_releases.clear();
        self.transition(
            PolicyEventType::QualificationInvalidated,
            None,
            None,
            open_reason,
        );
        PolicyTransition::Invalidated { clean_streak: 0 }
    }

    /// Change the graduation threshold and force requalification.
    pub fn set_threshold(&mut self, threshold: u32) -> Result<()> {
        if threshold == 0 {
            bail!("graduation threshold must be greater than zero");
        }
        if self.graduation_threshold != threshold {
            if self.mode == PolicyMode::FailClosed {
                bail!("cannot change the threshold while Fail-Closed; demote first");
            }
            self.graduation_threshold = threshold;
            self.clean_release_streak = 0;
            self.counted_releases.clear();
            self.transition(
                PolicyEventType::ThresholdChanged,
                None,
                None,
                format!("graduation threshold changed to {threshold}"),
            );
        }
        Ok(())
    }

    /// Return the structured policy telemetry retained in this snapshot.
    pub fn events(&self) -> &[PolicyEvent] {
        &self.events
    }
}

/// Result of reconciling durable release telemetry with policy state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReconcileOutcome {
    /// No release has a complete observation window yet.
    Pending { reason: String },
    /// An eligible clean release advanced or completed graduation.
    Clean(PolicyTransition),
    /// A poison-pill signal reset qualification or was consumed while already
    /// Fail-Closed.
    PoisonPill(PolicyTransition),
    /// No mutation was required.
    NoChange,
    /// Evidence was incomplete or the detector was not armed, so the clean
    /// streak was invalidated without recording a poison-pill event.
    Invalidated(PolicyTransition),
}

/// Durable policy file store.
#[derive(Debug, Clone)]
pub struct PolicyStore {
    path: PathBuf,
}

impl PolicyStore {
    pub fn new(path: impl AsRef<Path>) -> Self {
        Self {
            path: path.as_ref().to_path_buf(),
        }
    }

    /// Return the administrator-controlled production path.
    pub fn default_path() -> PathBuf {
        PathBuf::from("/etc/icg/fail-closed-policy.json")
    }

    /// Resolve the deployment path, allowing tests and deployment manifests
    /// to point at a different administrator-controlled location.
    pub fn from_env() -> Self {
        std::env::var_os(POLICY_PATH_ENV)
            .filter(|path| !path.is_empty())
            .map(PathBuf::from)
            .map(Self::new)
            .unwrap_or_else(|| Self::new(Self::default_path()))
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    fn parent_dir(&self) -> &Path {
        self.path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."))
    }

    fn ensure_parent(&self) -> Result<()> {
        if !self.parent_dir().exists() {
            fs::create_dir_all(self.parent_dir()).with_context(|| {
                format!(
                    "failed to create policy directory {}",
                    self.parent_dir().display()
                )
            })?;
        }
        Ok(())
    }

    fn lock_path(&self) -> PathBuf {
        self.path.with_extension("lock")
    }

    fn acquire_lock(&self) -> Result<PolicyLock> {
        self.ensure_parent()?;
        let file = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(self.lock_path())
            .context("failed to open fail-closed policy lock")?;
        platform_lock(&file).context("failed to lock fail-closed policy")?;
        Ok(PolicyLock { file })
    }

    fn backup_path(&self) -> PathBuf {
        self.path.with_extension("json.bak")
    }

    fn load_file(path: &Path) -> Result<PolicyState> {
        let content = fs::read_to_string(path)
            .with_context(|| format!("failed to read policy state {}", path.display()))?;
        let mut state: PolicyState = serde_json::from_str(&content)
            .with_context(|| format!("failed to parse policy state {}", path.display()))?;
        state.validate()?;
        Ok(state)
    }

    fn load_unlocked(&self) -> Result<PolicyState> {
        if !self.path.exists() {
            return PolicyState::new(DEFAULT_GRADUATION_THRESHOLD);
        }
        match Self::load_file(&self.path) {
            Ok(state) => Ok(state),
            Err(current_error) if self.backup_path().exists() => {
                Self::load_file(&self.backup_path()).with_context(|| {
                    format!(
                    "current policy is unreadable ({current_error:#}) and backup is also invalid"
                )
                })
            }
            Err(error) => Err(error),
        }
    }

    /// Load policy state.  Missing state is the safe bootstrap state:
    /// Fail-Open with a clean streak of zero.
    pub fn load(&self) -> Result<PolicyState> {
        if !self.path.exists() {
            return PolicyState::new(DEFAULT_GRADUATION_THRESHOLD);
        }
        self.load_unlocked()
    }

    fn save_unlocked(&self, state: &PolicyState) -> Result<()> {
        let mut state = state.clone();
        state.validate()?;
        self.ensure_parent()?;

        let temp_path = self.path.with_extension(format!(
            "tmp-{}-{}",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        let content =
            serde_json::to_vec_pretty(&state).context("failed to serialize policy state")?;
        let mut file = File::create(&temp_path)
            .with_context(|| format!("failed to create {}", temp_path.display()))?;
        file.write_all(&content)
            .with_context(|| format!("failed to write {}", temp_path.display()))?;
        file.sync_all()
            .with_context(|| format!("failed to sync {}", temp_path.display()))?;
        drop(file);

        if self.path.exists() {
            fs::copy(&self.path, self.backup_path()).with_context(|| {
                format!(
                    "failed to preserve policy backup {}",
                    self.backup_path().display()
                )
            })?;
        }
        if let Err(error) = fs::rename(&temp_path, &self.path) {
            let _ = fs::remove_file(&temp_path);
            return Err(error).with_context(|| {
                format!(
                    "failed to atomically replace policy state {}",
                    self.path.display()
                )
            });
        }

        let directory = File::open(self.parent_dir()).with_context(|| {
            format!(
                "failed to open policy directory {}",
                self.parent_dir().display()
            )
        })?;
        directory.sync_all().with_context(|| {
            format!(
                "failed to sync policy directory {}",
                self.parent_dir().display()
            )
        })?;
        Ok(())
    }

    /// Atomically save policy state while serializing writers.
    pub fn save(&self, state: &PolicyState) -> Result<()> {
        let _lock = self.acquire_lock()?;
        self.save_unlocked(state)
    }

    /// Apply one state mutation transactionally.
    pub fn update<F, T>(&self, mutate: F) -> Result<T>
    where
        F: FnOnce(&mut PolicyState) -> Result<T>,
    {
        let _lock = self.acquire_lock()?;
        let mut state = self.load_unlocked()?;
        let result = mutate(&mut state)?;
        self.save_unlocked(&state)?;
        Ok(result)
    }

    pub fn record_clean_release(&self, release_ref: impl Into<String>) -> Result<PolicyTransition> {
        self.update(|state| state.record_clean_release(release_ref))
    }

    pub fn record_poison_pill(&self, event_ref: impl Into<String>) -> Result<PolicyTransition> {
        self.update(|state| state.record_poison_pill(event_ref))
    }

    pub fn emergency_demote(&self, reason: impl Into<String>) -> Result<PolicyTransition> {
        self.update(|state| Ok(state.emergency_demote(reason)))
    }

    /// Explicitly force the administrator-controlled policy to Fail-Closed.
    pub fn force_graduate(&self, reason: impl Into<String>) -> Result<PolicyTransition> {
        self.update(|state| state.force_graduate(reason))
    }

    /// Alias for the operator-facing force-revert control.
    pub fn force_revert(&self, reason: impl Into<String>) -> Result<PolicyTransition> {
        self.emergency_demote(reason)
    }

    pub fn set_threshold(&self, threshold: u32) -> Result<()> {
        self.update(|state| state.set_threshold(threshold))
    }

    /// Reconcile one host's release-health evidence into the deployment policy.
    ///
    /// The poison-pill detector remains authoritative.  A durable rollback
    /// count is consumed before a release can be counted, and a concerning
    /// deviation is treated as an invalid release even when rollback is
    /// disabled or cooldown-suppressed.  This keeps missing rollback action
    /// from being mistaken for a clean observation.
    pub fn reconcile_release_health(
        &self,
        state_store: &StateStore,
        trust_store: &TrustPointerStore,
        poison_pill_config: &PoisonPillConfig,
    ) -> Result<ReconcileOutcome> {
        let _lock = self.acquire_lock()?;
        let mut state = self.load_unlocked()?;

        let rollback = state_store.rollback_state()?;
        if rollback.rollback_count > state.last_processed_rollback_count {
            let event_ref = format!("rollback:{}", rollback.rollback_count);
            state.last_processed_rollback_count = rollback.rollback_count;
            let transition = state.record_poison_pill(event_ref)?;
            self.save_unlocked(&state)?;
            return Ok(ReconcileOutcome::PoisonPill(transition));
        }

        let Some(pointer) = trust_store.load()? else {
            return Ok(ReconcileOutcome::Pending {
                reason: "no trusted release pointer exists".to_string(),
            });
        };

        let Some(deviation) = state_store.deny_rate_deviation_for(&pointer.trusted_ref)? else {
            return Ok(ReconcileOutcome::Pending {
                reason: format!("no telemetry for release `{}`", pointer.trusted_ref),
            });
        };

        if !release_observation_is_fresh(state_store, &pointer)? {
            return Ok(ReconcileOutcome::Pending {
                reason: format!(
                    "release `{}` predates its trust-pointer adoption",
                    pointer.trusted_ref
                ),
            });
        }
        let deny_rate_policy = &poison_pill_config.policy;
        if !poison_pill_config.enabled {
            let transition = state.invalidate_qualification("poison-pill detector is disabled");
            if matches!(&transition, PolicyTransition::NoChange) {
                return Ok(ReconcileOutcome::NoChange);
            }
            self.save_unlocked(&state)?;
            return Ok(ReconcileOutcome::Invalidated(transition));
        }
        if deviation.current_evaluation_count < deny_rate_policy.minimum_current_evaluations {
            return Ok(ReconcileOutcome::Pending {
                reason: format!(
                    "release `{}` has {} of {} required observations",
                    pointer.trusted_ref,
                    deviation.current_evaluation_count,
                    deny_rate_policy.minimum_current_evaluations
                ),
            });
        }
        if deviation.baseline.release_count < deny_rate_policy.minimum_baseline_releases
            || deviation.baseline.evaluation_count < deny_rate_policy.minimum_baseline_evaluations
        {
            return Ok(ReconcileOutcome::Pending {
                reason: format!(
                    "release `{}` has an incomplete prior-release baseline",
                    pointer.trusted_ref
                ),
            });
        }

        if deviation.current_evaluation_count > poison_pill_config.max_current_evaluations {
            let transition = state.invalidate_qualification(format!(
                "release `{}` exceeded the poison-pill observation window",
                pointer.trusted_ref
            ));
            if matches!(&transition, PolicyTransition::NoChange) {
                return Ok(ReconcileOutcome::NoChange);
            }
            self.save_unlocked(&state)?;
            return Ok(ReconcileOutcome::Invalidated(transition));
        }

        if deviation.is_concerning(deny_rate_policy) {
            let event_ref = format!("deviation:{}", pointer.trusted_ref);
            let transition = state.record_poison_pill(event_ref)?;
            self.save_unlocked(&state)?;
            return Ok(ReconcileOutcome::PoisonPill(transition));
        }

        let transition = state.record_clean_release(pointer.trusted_ref)?;
        if matches!(transition, PolicyTransition::NoChange) {
            return Ok(ReconcileOutcome::NoChange);
        }
        self.save_unlocked(&state)?;
        Ok(ReconcileOutcome::Clean(transition))
    }
}

/// Read the active release's first observation and ensure it was adopted by
/// the current trust pointer.  Old telemetry must never qualify a new release.
fn release_observation_is_fresh(
    state_store: &StateStore,
    pointer: &crate::trust_pointer::TrustPointer,
) -> Result<bool> {
    if pointer.updated_at.trim().is_empty() {
        return Ok(true);
    }
    let Some(record) = state_store.release_telemetry_for(&pointer.trusted_ref)? else {
        return Ok(false);
    };
    let pointer_time = pointer
        .updated_at
        .parse::<DateTime<Utc>>()
        .with_context(|| {
            format!(
                "invalid trust-pointer timestamp for `{}`: {}",
                pointer.trusted_ref, pointer.updated_at
            )
        })?;
    let first_seen = record
        .first_seen_at
        .parse::<DateTime<Utc>>()
        .with_context(|| {
            format!(
                "invalid first telemetry timestamp for `{}`: {}",
                pointer.trusted_ref, record.first_seen_at
            )
        })?;
    Ok(first_seen >= pointer_time)
}

struct PolicyLock {
    file: File,
}

impl Drop for PolicyLock {
    fn drop(&mut self) {
        platform_unlock(&self.file);
    }
}

#[cfg(unix)]
fn platform_lock(file: &File) -> Result<()> {
    let result = unsafe { libc::flock(std::os::fd::AsRawFd::as_raw_fd(file), libc::LOCK_EX) };
    if result == 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error()).context("flock failed")
    }
}

#[cfg(not(unix))]
fn platform_lock(_file: &File) -> Result<()> {
    Ok(())
}

#[cfg(unix)]
fn platform_unlock(file: &File) {
    let _ = unsafe { libc::flock(std::os::fd::AsRawFd::as_raw_fd(file), libc::LOCK_UN) };
}

#[cfg(not(unix))]
fn platform_unlock(_file: &File) {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_state_is_fail_open() {
        let state = PolicyState::default();
        assert_eq!(state.mode, PolicyMode::FailOpen);
        assert_eq!(state.clean_release_streak, 0);
        assert_eq!(state.graduation_threshold, DEFAULT_GRADUATION_THRESHOLD);
    }

    #[test]
    fn duplicate_release_does_not_advance_streak() -> Result<()> {
        let mut state = PolicyState::new(2)?;
        assert!(matches!(
            state.record_clean_release("v1")?,
            PolicyTransition::CleanRelease {
                clean_streak: 1,
                ..
            }
        ));
        assert_eq!(
            state.record_clean_release("v1")?,
            PolicyTransition::NoChange
        );
        assert_eq!(state.clean_release_streak, 1);
        Ok(())
    }

    #[test]
    fn threshold_graduates_and_poison_pill_preserves_fail_closed() -> Result<()> {
        let mut state = PolicyState::new(2)?;
        state.record_clean_release("v1")?;
        assert!(matches!(
            state.record_clean_release("v2")?,
            PolicyTransition::Graduated { generation: 2, .. }
        ));
        assert!(state.is_fail_closed());
        state.record_poison_pill("rollback:1")?;
        assert!(state.is_fail_closed());
        assert_eq!(state.clean_release_streak, 2);
        assert_eq!(state.events.len(), 3);
        assert_eq!(state.events[1].event_type, PolicyEventType::Graduated);
        assert_eq!(state.events[2].event_type, PolicyEventType::PoisonPill);
        Ok(())
    }

    #[test]
    fn force_controls_are_audited_and_revert_requires_requalification() -> Result<()> {
        let mut state = PolicyState::new(3)?;
        assert!(matches!(
            state.force_graduate("approved emergency change")?,
            PolicyTransition::ForcedGraduation { generation: 1 }
        ));
        assert!(state.is_fail_closed());
        assert_eq!(
            state.events.last().unwrap().event_type,
            PolicyEventType::ManualGraduation
        );

        assert_eq!(
            state.emergency_demote("guard incident"),
            PolicyTransition::EmergencyDemotion { generation: 2 }
        );
        assert_eq!(state.mode, PolicyMode::FailOpen);
        assert_eq!(state.clean_release_streak, 0);
        assert_eq!(
            state.events.last().unwrap().event_type,
            PolicyEventType::EmergencyDemotion
        );
        Ok(())
    }

    #[test]
    fn force_graduate_requires_an_audit_reason() -> Result<()> {
        let mut state = PolicyState::new(3)?;
        assert!(state.force_graduate(" ").is_err());
        assert_eq!(state.mode, PolicyMode::FailOpen);
        assert!(state.events.is_empty());
        Ok(())
    }

    #[test]
    fn poison_pill_resets_open_qualification_idempotently() -> Result<()> {
        let mut state = PolicyState::new(3)?;
        state.record_clean_release("v1")?;
        assert!(matches!(
            state.record_poison_pill("rollback:1")?,
            PolicyTransition::PoisonPill {
                clean_streak: 0,
                ..
            }
        ));
        assert_eq!(state.clean_release_streak, 0);
        assert_eq!(
            state.record_poison_pill("rollback:1")?,
            PolicyTransition::NoChange
        );
        Ok(())
    }

    #[test]
    fn policy_store_round_trips_atomically() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let store = PolicyStore::new(directory.path().join("policy.json"));
        store.record_clean_release("v1")?;
        let state = store.load()?;
        assert_eq!(state.clean_release_streak, 1);
        assert_eq!(state.mode, PolicyMode::FailOpen);
        assert!(store.path().exists());
        Ok(())
    }
}
