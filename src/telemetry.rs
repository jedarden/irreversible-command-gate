//! Rolling baseline telemetry for deny-rate monitoring and auto-rollback
//!
//! This module implements:
//! - Telemetry collection for engine evaluation results
//! - Rolling baseline calculation over a configurable time window
//! - Anomaly detection for deny-rate spikes
//! - Poison-pill auto-rollback integration with trust pointer
//!
//! ## Architecture
//!
//! The telemetry system works in three phases:
//! 1. **Collection**: Engine records evaluation results (allow/deny/warning/rewrite)
//! 2. **Analysis**: Rolling baseline statistics calculated over sliding window
//! 3. **Reaction**: Anomaly detection triggers automatic trust pointer rollback
//!
//! ## Data Flow
//!
//! ```text
//! Engine::evaluate_*()
//!   → Telemetry::record_result()
//!   → TelemetryStore::persist()
//!   → BaselineCalculator::compute_baseline()
//!   → AnomalyDetector::check_spike()
//!   → RollbackHandler::revert_trust_pointer()
//! ```
//!
//! ## Configuration
//!
//! - Window size: Number of evaluations to include in baseline (default: 1000)
//! - Spike threshold: Multiplier above baseline that triggers rollback (default: 3.0x)
//! - Minimum samples: Required evaluations before baseline is considered valid (default: 100)
//! - Cooldown: Time between rollback attempts to prevent rollback loops (default: 1 hour)

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::time::Duration;

/// Telemetry configuration for baseline calculation and anomaly detection
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TelemetryConfig {
    /// Number of evaluations to include in rolling baseline window
    ///
    /// Larger windows = more stable baseline but slower reaction to real changes
    /// Recommended: 500-2000 for production fleets
    pub window_size: usize,

    /// Spike threshold as multiplier above baseline mean
    ///
    /// If current deny rate > (baseline_mean * threshold), trigger rollback
    /// Recommended: 2.5-5.0 depending on tolerance for false positives
    pub spike_threshold: f64,

    /// Minimum samples before baseline is considered valid
    ///
    /// Prevents false positives during cold start or low-traffic periods
    /// Recommended: 50-200
    pub minimum_samples: usize,

    /// Cooldown period between automatic rollbacks
    ///
    /// Prevents rollback loops if the new release also has issues
    /// Recommended: 30 minutes to 2 hours
    pub rollback_cooldown: Duration,

    /// Enable or disable automatic rollback
    ///
    /// When false, anomalies are logged but no rollback occurs (dry-run mode)
    pub auto_rollback_enabled: bool,
}

impl Default for TelemetryConfig {
    fn default() -> Self {
        Self {
            window_size: 1000,
            spike_threshold: 3.0,
            minimum_samples: 100,
            rollback_cooldown: Duration::from_secs(3600), // 1 hour
            auto_rollback_enabled: true,
        }
    }
}

/// Single evaluation result record
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EvaluationRecord {
    /// Timestamp when this evaluation occurred
    pub timestamp: DateTime<Utc>,

    /// Result of the evaluation
    pub verdict: Verdict,

    /// Optional release reference for this evaluation
    ///
    /// Links telemetry to specific rule pack release
    pub release_ref: Option<String>,

    /// Optional session ID for cross-invocation correlation
    pub session_id: Option<String>,
}

/// Evaluation verdict (matches engine::CheckResult but simpler for serialization)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Verdict {
    /// Operation was allowed
    Allowed,

    /// Operation was denied
    Denied,

    /// Operation was allowed with warning
    Warning,

    /// Operation was rewritten
    Rewrite,
}

impl Verdict {
    /// Check if this verdict represents a deny
    pub fn is_deny(&self) -> bool {
        matches!(self, Verdict::Denied)
    }
}

/// Rolling window of evaluation records with bounded size
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EvaluationWindow {
    /// Records in the window, oldest first
    records: Vec<EvaluationRecord>,

    /// Maximum window size
    capacity: usize,
}

/// Durable health counters carried alongside evaluation telemetry.
///
/// Health remains authoritative in `HealthStore`; this copy makes crash and
/// uptime signals available to the same telemetry/status consumers that read
/// deny-rate data.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct HealthTelemetry {
    pub total_crashes: u64,
    pub consecutive_clean_runs: u64,
    pub uptime_seconds: f64,
    pub crash_rate: f64,
    pub health_status: crate::health::HealthStatus,
    pub last_crash_at: Option<DateTime<Utc>>,
    pub last_start_at: Option<DateTime<Utc>>,
}

/// Durable counters for rules that produced a non-allowing evaluation.
/// Keeping these counters in telemetry makes rule performance available to a
/// scrape without putting I/O on the evaluation hot path.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RuleEvaluationTelemetry {
    pub pack_id: String,
    pub pattern_id: String,
    pub match_count: u64,
    pub deny_count: u64,
}

impl EvaluationWindow {
    /// Create a new empty window with specified capacity
    pub fn new(capacity: usize) -> Self {
        Self {
            records: Vec::with_capacity(capacity),
            capacity,
        }
    }

    /// Add a record to the window, evicting oldest if at capacity
    pub fn push(&mut self, record: EvaluationRecord) {
        if self.records.len() == self.capacity {
            self.records.remove(0);
        }
        self.records.push(record);
    }

    /// Get all records in the window
    pub fn records(&self) -> &[EvaluationRecord] {
        &self.records
    }

    /// Get number of records in the window
    pub fn len(&self) -> usize {
        self.records.len()
    }

    /// Check if window is empty
    pub fn is_empty(&self) -> bool {
        self.records.is_empty()
    }

    /// Clear all records from the window
    pub fn clear(&mut self) {
        self.records.clear();
    }
}

/// Rolling baseline statistics
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BaselineStats {
    /// Number of samples in the baseline
    pub sample_count: usize,

    /// Total evaluations in baseline
    pub total_evaluations: usize,

    /// Number of deny verdicts in baseline
    pub deny_count: usize,

    /// Deny rate (deny_count / total_evaluations)
    pub deny_rate: f64,

    /// Mean deny rate
    pub mean: f64,

    /// Standard deviation of deny rate
    pub std_dev: f64,

    /// Minimum observed deny rate in window
    pub min: f64,

    /// Maximum observed deny rate in window
    pub max: f64,

    /// Timestamp of oldest record in baseline
    pub window_start: Option<DateTime<Utc>>,

    /// Timestamp of newest record in baseline
    pub window_end: Option<DateTime<Utc>>,
}

impl BaselineStats {
    /// Check if baseline has sufficient samples for valid analysis
    pub fn is_valid(&self, minimum_samples: usize) -> bool {
        self.sample_count >= minimum_samples && self.total_evaluations > 0
    }

    /// Calculate upper bound threshold for anomaly detection
    ///
    /// Returns: mean + (std_dev * spike_multiplier)
    pub fn anomaly_threshold(&self, multiplier: f64) -> f64 {
        self.mean + (self.std_dev * multiplier)
    }
}

/// Anomaly detection result
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnomalyReport {
    /// Timestamp when anomaly was detected
    pub detected_at: DateTime<Utc>,

    /// Current deny rate that triggered the anomaly
    pub current_deny_rate: f64,

    /// Baseline statistics at time of detection
    pub baseline: BaselineStats,

    /// Severity of the anomaly (how far above threshold)
    pub severity: AnomalySeverity,

    /// Whether rollback was triggered
    pub rollback_triggered: bool,

    /// Release reference that was rolled back
    pub rolled_back_release: Option<String>,

    /// Previous release reference (what we rolled back to)
    pub previous_release: Option<String>,
}

/// Severity classification for anomalies
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AnomalySeverity {
    /// Minor spike (1-2x above threshold)
    Low,

    /// Moderate spike (2-3x above threshold)
    Medium,

    /// Severe spike (3-5x above threshold)
    High,

    /// Extreme spike (>5x above threshold)
    Critical,
}

impl AnomalySeverity {
    /// Classify severity based on how much the current rate exceeds the threshold
    pub fn from_excess(current_rate: f64, threshold: f64) -> Self {
        if threshold == 0.0 {
            return AnomalySeverity::Low;
        }

        let ratio = current_rate / threshold;

        if ratio < 2.0 {
            AnomalySeverity::Low
        } else if ratio < 3.0 {
            AnomalySeverity::Medium
        } else if ratio < 5.0 {
            AnomalySeverity::High
        } else {
            AnomalySeverity::Critical
        }
    }
}

/// Telemetry store: persists evaluation history and loads on startup
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TelemetryStore {
    /// Current evaluation window
    window: EvaluationWindow,

    /// Telemetry configuration
    config: TelemetryConfig,

    /// Timestamp of last automatic rollback (for cooldown enforcement)
    last_rollback_at: Option<DateTime<Utc>>,

    /// Most recent durable guard-health snapshot.
    #[serde(default)]
    health: HealthTelemetry,

    /// Bounded-by-pack rule counters for dashboard and alert consumers.
    #[serde(default)]
    rule_metrics: std::collections::BTreeMap<String, RuleEvaluationTelemetry>,

    /// Path to telemetry file on disk
    store_path: PathBuf,
}

impl TelemetryStore {
    /// Create a new telemetry store with default configuration
    pub fn new(store_path: PathBuf) -> Self {
        Self {
            window: EvaluationWindow::new(1000),
            config: TelemetryConfig::default(),
            last_rollback_at: None,
            health: HealthTelemetry::default(),
            rule_metrics: std::collections::BTreeMap::new(),
            store_path,
        }
    }

    /// Return the path used by this store.
    pub fn path(&self) -> &Path {
        &self.store_path
    }

    /// Create telemetry store with custom configuration
    pub fn with_config(store_path: PathBuf, config: TelemetryConfig) -> Self {
        Self {
            window: EvaluationWindow::new(config.window_size),
            config,
            last_rollback_at: None,
            health: HealthTelemetry::default(),
            rule_metrics: std::collections::BTreeMap::new(),
            store_path,
        }
    }

    /// Load telemetry from disk, or create new if file doesn't exist
    pub fn load_or_create(path: PathBuf) -> Result<Self> {
        if path.exists() {
            let content = std::fs::read_to_string(&path)
                .with_context(|| format!("Failed to read telemetry store from {}", path.display()))?;

            let mut store: TelemetryStore = serde_json::from_str(&content)
                .with_context(|| format!("Failed to parse telemetry store from {}", path.display()))?;

            store.store_path = path;
            Ok(store)
        } else {
            // Ensure parent directory exists
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent)
                    .with_context(|| format!("Failed to create telemetry directory {}", parent.display()))?;
            }

            Ok(Self::new(path))
        }
    }

    /// Persist telemetry state to disk
    pub fn persist(&self) -> Result<()> {
        let content = serde_json::to_string_pretty(self)
            .context("Failed to serialize telemetry store")?;

        // Atomic write: write to temp file, then rename
        let temp_path = self.store_path.with_extension("tmp");
        std::fs::write(&temp_path, content)
            .with_context(|| format!("Failed to write telemetry temp file {}", temp_path.display()))?;

        std::fs::rename(&temp_path, &self.store_path)
            .with_context(|| format!("Failed to rename telemetry file to {}", self.store_path.display()))?;

        Ok(())
    }

    /// Record an evaluation result
    pub fn record_evaluation(
        &mut self,
        verdict: Verdict,
        release_ref: Option<String>,
        session_id: Option<String>,
    ) {
        let record = EvaluationRecord {
            timestamp: Utc::now(),
            verdict,
            release_ref,
            session_id,
        };

        self.window.push(record);
    }

    /// Record an evaluation and associate it with the rule that matched.
    pub fn record_evaluation_for_rule(
        &mut self,
        verdict: Verdict,
        release_ref: Option<String>,
        session_id: Option<String>,
        pack_id: Option<&str>,
        pattern_id: Option<&str>,
    ) {
        self.record_evaluation(verdict, release_ref, session_id);
        let (Some(pack_id), Some(pattern_id)) = (pack_id, pattern_id) else {
            return;
        };
        let key = format!("{pack_id}\u{1f}{pattern_id}");
        let entry = self
            .rule_metrics
            .entry(key)
            .or_insert_with(|| RuleEvaluationTelemetry {
                pack_id: pack_id.to_string(),
                pattern_id: pattern_id.to_string(),
                ..Default::default()
            });
        entry.match_count = entry.match_count.saturating_add(1);
        if verdict.is_deny() {
            entry.deny_count = entry.deny_count.saturating_add(1);
        }
    }

    /// Return the persisted per-rule counters for monitoring exporters.
    pub fn rule_metrics(&self) -> impl Iterator<Item = &RuleEvaluationTelemetry> {
        self.rule_metrics.values()
    }

    /// Restore rule counters when a configuration update rebuilds the store.
    pub fn restore_rule_metrics(
        &mut self,
        metrics: impl IntoIterator<Item = RuleEvaluationTelemetry>,
    ) {
        self.rule_metrics = metrics
            .into_iter()
            .map(|metric| {
                (
                    format!("{}\u{1f}{}", metric.pack_id, metric.pattern_id),
                    metric,
                )
            })
            .collect();
    }

    /// Get the current evaluation window
    pub fn window(&self) -> &EvaluationWindow {
        &self.window
    }

    /// Get the telemetry configuration
    pub fn config(&self) -> &TelemetryConfig {
        &self.config
    }

    /// Return the time of the last successful automatic rollback, if any.
    pub fn last_rollback_at(&self) -> Option<DateTime<Utc>> {
        self.last_rollback_at
    }

    /// Return the latest health snapshot copied into telemetry.
    pub fn health(&self) -> &HealthTelemetry {
        &self.health
    }

    /// Record the current durable health metrics in the telemetry store.
    pub fn record_health_metrics(&mut self, metrics: &crate::health::HealthMetrics) {
        self.health = HealthTelemetry {
            total_crashes: metrics.total_crashes as u64,
            consecutive_clean_runs: metrics.consecutive_clean_runs as u64,
            uptime_seconds: metrics
                .current_uptime
                .map(|duration| duration.as_secs_f64())
                .unwrap_or(0.0),
            crash_rate: metrics.crash_rate,
            health_status: metrics.status,
            last_crash_at: metrics.last_crash_at,
            last_start_at: metrics.last_start_at,
        };
    }

    /// Check if rollback is on cooldown
    pub fn is_rollback_on_cooldown(&self) -> bool {
        if let Some(last_rollback) = self.last_rollback_at {
            let elapsed = Utc::now().signed_duration_since(last_rollback);
            elapsed.to_std().unwrap_or(Duration::ZERO) < self.config.rollback_cooldown
        } else {
            false
        }
    }

    /// Update last rollback timestamp (call after successful rollback)
    pub fn mark_rollback_performed(&mut self) {
        self.last_rollback_at = Some(Utc::now());
    }

    /// Clear all telemetry data (useful for testing or reset)
    pub fn clear(&mut self) {
        self.window.clear();
        self.last_rollback_at = None;
        self.rule_metrics.clear();
    }
}

/// Calculate baseline statistics from an evaluation window
pub fn compute_baseline(window: &EvaluationWindow) -> BaselineStats {
    let records = window.records();

    if records.is_empty() {
        return BaselineStats {
            sample_count: 0,
            total_evaluations: 0,
            deny_count: 0,
            deny_rate: 0.0,
            mean: 0.0,
            std_dev: 0.0,
            min: 0.0,
            max: 0.0,
            window_start: None,
            window_end: None,
        };
    }

    let total_evaluations = records.len();
    let deny_count = records.iter().filter(|r| r.verdict.is_deny()).count();
    let deny_rate = if total_evaluations > 0 {
        deny_count as f64 / total_evaluations as f64
    } else {
        0.0
    };

    let window_start = records.first().map(|r| r.timestamp);
    let window_end = records.last().map(|r| r.timestamp);

    // For now, use simple deny rate as the mean
    // TODO: Implement sliding window mean/std_dev calculation
    let mean = deny_rate;
    let std_dev = 0.0; // Will be calculated properly in the full implementation

    BaselineStats {
        sample_count: total_evaluations,
        total_evaluations,
        deny_count,
        deny_rate,
        mean,
        std_dev,
        min: deny_rate, // Same for now
        max: deny_rate, // Same for now
        window_start,
        window_end,
    }
}

/// Check if current deny rate represents an anomaly compared to baseline
pub fn check_anomaly(
    current_deny_rate: f64,
    baseline: &BaselineStats,
    config: &TelemetryConfig,
) -> Option<AnomalyReport> {
    // Ensure we have enough baseline data
    if !baseline.is_valid(config.minimum_samples) {
        return None;
    }

    // Calculate anomaly threshold
    let threshold = baseline.anomaly_threshold(config.spike_threshold);

    // Check if current rate exceeds threshold
    if current_deny_rate > threshold {
        let severity = AnomalySeverity::from_excess(current_deny_rate, threshold);

        Some(AnomalyReport {
            detected_at: Utc::now(),
            current_deny_rate,
            baseline: baseline.clone(),
            severity,
            rollback_triggered: false, // Will be set by rollback handler
            rolled_back_release: None,
            previous_release: None,
        })
    } else {
        None
    }
}

/// Rollback handler for automatic trust pointer reversion
///
/// This function handles the automatic rollback when an anomaly is detected:
/// 1. Checks if rollback is enabled and not on cooldown
/// 2. Reads the current and previous trust pointer references
/// 3. Reverts the trust pointer to the previous reference
/// 4. Updates the telemetry store to mark the rollback
///
/// Returns `Ok(Some(report))` if rollback was performed, `Ok(None)` if skipped,
/// or `Err` if the rollback failed.
pub fn handle_rollback(
    store: &mut TelemetryStore,
    trust_store: &crate::trust_pointer::TrustPointerStore,
    anomaly: AnomalyReport,
) -> Result<Option<AnomalyReport>> {
    // Check if auto-rollback is enabled
    if !store.config.auto_rollback_enabled {
        eprintln!("⚠️  Anomaly detected but auto-rollback is disabled (dry-run mode)");
        eprintln!("   Current deny rate: {:.2}%, Baseline: {:.2}%, Threshold: {:.2}%",
                  anomaly.current_deny_rate * 100.0,
                  anomaly.baseline.mean * 100.0,
                  anomaly.baseline.anomaly_threshold(store.config.spike_threshold) * 100.0);
        return Ok(None);
    }

    // Check if rollback is on cooldown
    if store.is_rollback_on_cooldown() {
        eprintln!("⚠️  Anomaly detected but rollback is on cooldown");
        eprintln!("   Cooldown period: {:?}", store.config.rollback_cooldown);
        return Ok(None);
    }

    // Get current trust pointer
    let current_pointer = trust_store.load()?
        .context("No trust pointer found - cannot perform rollback")?;

    let current_ref = current_pointer.trusted_ref.clone();

    // Get previous reference from trust pointer state
    // We need to read the state store to get the previous ref
    let state_store_path = trust_store.path()
        .parent()
        .context("Trust pointer path has no parent")?
        .join("runtime-state.json");

    let previous_ref = if state_store_path.exists() {
        let state_store = crate::state_store::StateStore::new(&state_store_path);

        // Load the session state to get the trust pointer state
        let session_state = state_store.load()
            .with_context(|| format!("Failed to load session state from {}", state_store_path.display()))?;

        // Get the previous trusted ref from the trust pointer state
        session_state.trust_pointer.as_ref().and_then(|tp| tp.previous_trusted_ref.clone())
    } else {
        eprintln!("⚠️  No state store found - cannot determine previous ref for rollback");
        None
    };

    let previous_ref = previous_ref
        .context("No previous trust reference available - cannot perform rollback")?;

    // Perform the rollback
    eprintln!("🚨 Performing automatic rollback due to anomaly detection");
    eprintln!("   Current ref: {}, Rolling back to: {}", current_ref, previous_ref);
    eprintln!("   Severity: {:?}", anomaly.severity);
    eprintln!("   Current deny rate: {:.2}%, Baseline: {:.2}%",
              anomaly.current_deny_rate * 100.0,
              anomaly.baseline.mean * 100.0);

    trust_store.set_trusted_ref_with_justification(
        &previous_ref,
        format!("Automatic rollback due to anomaly detection. \
                Previous ref: {} had deny rate {:.2}%, \
                current ref: {} had deny rate {:.2}%. \
                Severity: {:?}",
                current_ref, anomaly.baseline.mean * 100.0,
                previous_ref, anomaly.current_deny_rate * 100.0,
                anomaly.severity)
    )?;

    // Mark rollback as performed in telemetry store
    store.mark_rollback_performed();

    // Persist the updated telemetry state
    store.persist()?;

    // Return the updated anomaly report
    let mut report = anomaly;
    report.rollback_triggered = true;
    report.rolled_back_release = Some(current_ref);
    report.previous_release = Some(previous_ref.clone());

    eprintln!("✅ Rollback completed successfully");
    eprintln!("   Trust pointer now points to: {}", previous_ref);

    Ok(Some(report))
}

/// Process evaluation results and check for anomalies
///
/// This is the main integration function that should be called after each
/// batch of evaluations. It:
/// 1. Computes the current baseline from the telemetry window
/// 2. Calculates the current deny rate
/// 3. Checks for anomalies
/// 4. Triggers rollback if needed
///
/// Returns `Ok(Some(report))` if an anomaly was detected (and possibly handled),
/// `Ok(None)` if no anomaly, or `Err` if processing failed.
pub fn process_evaluation_results(
    store: &mut TelemetryStore,
    trust_store: &crate::trust_pointer::TrustPointerStore,
) -> Result<Option<AnomalyReport>> {
    // Compute baseline from current window
    let baseline = compute_baseline(&store.window);

    // Calculate current deny rate
    let current_deny_rate = baseline.deny_rate;

    // Check for anomaly
    if let Some(anomaly_report) = check_anomaly(current_deny_rate, &baseline, store.config()) {
        // Attempt rollback
        match handle_rollback(store, trust_store, anomaly_report.clone()) {
            Ok(Some(updated_report)) => {
                return Ok(Some(updated_report));
            }
            Ok(None) => {
                // Rollback was skipped (disabled or on cooldown)
                // Still report the anomaly
                return Ok(Some(anomaly_report));
            }
            Err(e) => {
                eprintln!("⚠️  Failed to perform rollback: {}", e);
                // Still report the anomaly even though rollback failed
                return Ok(Some(anomaly_report));
            }
        }
    }

    Ok(None)
}

/// Extension trait to convert engine::CheckResult to telemetry::Verdict
pub trait CheckResultToVerdict {
    fn to_verdict(&self) -> Verdict;
}

impl CheckResultToVerdict for crate::engine::CheckResult {
    fn to_verdict(&self) -> Verdict {
        match self {
            crate::engine::CheckResult::Allowed => Verdict::Allowed,
            crate::engine::CheckResult::Denied { .. } => Verdict::Denied,
            crate::engine::CheckResult::Rewrite { .. } => Verdict::Rewrite,
            crate::engine::CheckResult::Warning { .. } => Verdict::Warning,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_verdict_is_deny() {
        assert!(Verdict::Denied.is_deny());
        assert!(!Verdict::Allowed.is_deny());
        assert!(!Verdict::Warning.is_deny());
        assert!(!Verdict::Rewrite.is_deny());
    }

    #[test]
    fn health_snapshot_round_trips_with_telemetry() {
        let mut telemetry = TelemetryStore::new(std::path::PathBuf::from("/tmp/icg-test-telemetry.json"));
        let mut health = crate::health::HealthState::new();
        health.mark_start();
        health.record_crash(crate::health::CrashRecord::new(
            crate::health::CrashType::OutOfMemory,
        ));

        telemetry.record_health_metrics(&health.compute_metrics());
        assert_eq!(telemetry.health().total_crashes, 1);
        assert_eq!(
            telemetry.health().health_status,
            crate::health::HealthStatus::Dead
        );

        let encoded = serde_json::to_string(&telemetry).unwrap();
        let decoded: TelemetryStore = serde_json::from_str(&encoded).unwrap();
        assert_eq!(decoded.health().total_crashes, 1);
        assert_eq!(decoded.health().crash_rate, 1.0);
    }

    #[test]
    fn test_evaluation_window_push_and_capacity() {
        let mut window = EvaluationWindow::new(3);

        window.push(EvaluationRecord {
            timestamp: Utc::now(),
            verdict: Verdict::Allowed,
            release_ref: None,
            session_id: None,
        });

        assert_eq!(window.len(), 1);

        window.push(EvaluationRecord {
            timestamp: Utc::now(),
            verdict: Verdict::Denied,
            release_ref: None,
            session_id: None,
        });

        assert_eq!(window.len(), 2);

        // Add two more, should evict oldest to stay at capacity
        window.push(EvaluationRecord {
            timestamp: Utc::now(),
            verdict: Verdict::Allowed,
            release_ref: None,
            session_id: None,
        });

        window.push(EvaluationRecord {
            timestamp: Utc::now(),
            verdict: Verdict::Allowed,
            release_ref: None,
            session_id: None,
        });

        assert_eq!(window.len(), 3);
    }

    #[test]
    fn test_baseline_stats_empty_window() {
        let window = EvaluationWindow::new(10);
        let baseline = compute_baseline(&window);

        assert_eq!(baseline.sample_count, 0);
        assert_eq!(baseline.total_evaluations, 0);
        assert_eq!(baseline.deny_count, 0);
        assert_eq!(baseline.deny_rate, 0.0);
    }

    #[test]
    fn test_baseline_stats_with_data() {
        let mut window = EvaluationWindow::new(12);

        for _ in 0..10 {
            window.push(EvaluationRecord {
                timestamp: Utc::now(),
                verdict: Verdict::Allowed,
                release_ref: None,
                session_id: None,
            });
        }

        for _ in 0..2 {
            window.push(EvaluationRecord {
                timestamp: Utc::now(),
                verdict: Verdict::Denied,
                release_ref: None,
                session_id: None,
            });
        }

        let baseline = compute_baseline(&window);

        assert_eq!(baseline.total_evaluations, 12);
        assert_eq!(baseline.deny_count, 2);
        assert!((baseline.deny_rate - 0.1667).abs() < 0.01);
    }

    #[test]
    fn test_baseline_stats_validity() {
        let config = TelemetryConfig {
            minimum_samples: 10,
            ..Default::default()
        };

        let mut window = EvaluationWindow::new(100);

        // Add fewer than minimum_samples
        for _ in 0..5 {
            window.push(EvaluationRecord {
                timestamp: Utc::now(),
                verdict: Verdict::Allowed,
                release_ref: None,
                session_id: None,
            });
        }

        let baseline = compute_baseline(&window);
        assert!(!baseline.is_valid(config.minimum_samples));

        // Add more samples
        for _ in 0..10 {
            window.push(EvaluationRecord {
                timestamp: Utc::now(),
                verdict: Verdict::Allowed,
                release_ref: None,
                session_id: None,
            });
        }

        let baseline = compute_baseline(&window);
        assert!(baseline.is_valid(config.minimum_samples));
    }

    #[test]
    fn test_anomaly_detection() {
        let config = TelemetryConfig {
            minimum_samples: 10,
            spike_threshold: 3.0,
            ..Default::default()
        };

        let mut window = EvaluationWindow::new(100);

        // Build baseline with 1% deny rate
        for i in 0..20 {
            window.push(EvaluationRecord {
                timestamp: Utc::now(),
                verdict: if i % 100 == 0 { Verdict::Denied } else { Verdict::Allowed },
                release_ref: None,
                session_id: None,
            });
        }

        let baseline = compute_baseline(&window);

        // Current rate at 1% should not trigger anomaly
        let anomaly = check_anomaly(0.01, &baseline, &config);
        assert!(anomaly.is_none());

        // Current rate at 10% (10x baseline) should trigger anomaly
        let anomaly = check_anomaly(0.10, &baseline, &config);
        assert!(anomaly.is_some());

        let report = anomaly.unwrap();
        assert_eq!(report.current_deny_rate, 0.10);
        assert!(!report.rollback_triggered);
    }

    #[test]
    fn test_anomaly_severity_classification() {
        // Low severity: just above threshold
        let severity = AnomalySeverity::from_excess(0.04, 0.03);
        assert_eq!(severity, AnomalySeverity::Low);

        // Medium severity: 2x threshold
        let severity = AnomalySeverity::from_excess(0.06, 0.03);
        assert_eq!(severity, AnomalySeverity::Medium);

        // High severity: 4x threshold
        let severity = AnomalySeverity::from_excess(0.12, 0.03);
        assert_eq!(severity, AnomalySeverity::High);

        // Critical severity: >5x threshold
        let severity = AnomalySeverity::from_excess(0.20, 0.03);
        assert_eq!(severity, AnomalySeverity::Critical);
    }

    #[test]
    fn test_telemetry_store_persistence_roundtrip() -> Result<()> {
        let temp_dir = tempfile::tempdir()?;
        let store_path = temp_dir.path().join("telemetry.json");

        let mut store = TelemetryStore::new(store_path.clone());

        // Record some evaluations
        for i in 0..5 {
            store.record_evaluation(
                if i % 2 == 0 { Verdict::Allowed } else { Verdict::Denied },
                Some("v1.0.0".to_string()),
                Some("session-123".to_string()),
            );
        }

        // Persist to disk
        store.persist()?;

        // Load from disk
        let loaded_store = TelemetryStore::load_or_create(store_path)?;

        assert_eq!(loaded_store.window().len(), 5);

        Ok(())
    }
}
