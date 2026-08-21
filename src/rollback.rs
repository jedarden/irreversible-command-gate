//! Durable poison-pill rollback reaction.
//!
//! Measurement lives in [`crate::state_store`]. This module is deliberately
//! only the reaction: it consumes the current release's durable deny-rate
//! deviation and, when the conservative policy is satisfied, moves the
//! trust pointer back to the exact previous reference.

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::time::Duration;

use crate::state_store::{DenyRatePolicy, StateStore};
use crate::trust_pointer::{TrustPointer, TrustPointerStore};

/// Maximum number of current-release evaluations for an automatic rollback.
///
/// A release that only becomes suspicious after this early observation window
/// is not automatically reverted by this mechanism. That keeps a later change
/// in command mix from being mistaken for a release regression.
pub const DEFAULT_MAX_CURRENT_EVALUATIONS: u64 = 1_000;

/// Cooldown between automatic rollback decisions.
pub const DEFAULT_ROLLBACK_COOLDOWN: Duration = Duration::from_secs(3_600);

/// Configuration for the poison-pill reaction.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PoisonPillConfig {
    /// Whether a qualifying anomaly may change the trust pointer.
    pub enabled: bool,

    /// The maximum current-release sample count at which rollback may occur.
    pub max_current_evaluations: u64,

    /// Conservative release-health policy supplied by durable telemetry.
    pub policy: DenyRatePolicy,

    /// Minimum time between automatic rollback decisions.
    pub cooldown: Duration,
}

impl Default for PoisonPillConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            max_current_evaluations: DEFAULT_MAX_CURRENT_EVALUATIONS,
            policy: DenyRatePolicy::default(),
            cooldown: DEFAULT_ROLLBACK_COOLDOWN,
        }
    }
}

/// Evidence recorded when the reaction rolls a release back.
#[derive(Debug, Clone, PartialEq)]
pub struct RollbackReport {
    /// Release that was trusted when the anomaly was observed.
    pub release_ref: String,

    /// Exact previous trusted release selected by the pointer history.
    pub previous_release: String,

    /// Current release deny rate at the decision point.
    pub current_deny_rate: f64,

    /// Mean deny rate of the prior-release baseline.
    pub baseline_mean: f64,

    /// Standard deviation of the prior-release baseline.
    pub baseline_std_dev: f64,

    /// Signed current-minus-baseline rate difference.
    pub absolute_deviation: f64,

    /// Number of current-release evaluations at the decision point.
    pub current_evaluation_count: u64,

    /// Number of prior releases represented by the baseline.
    pub baseline_release_count: usize,

    /// Threshold used by the sigma portion of the policy.
    pub threshold: f64,
}

/// Consume durable telemetry and automatically roll back the active release
/// when the conservative poison-pill policy is satisfied.
///
/// `Ok(None)` means that no rollback was needed or that the anomaly was
/// conservatively suppressed (for example, because the release was outside
/// the early observation window). A qualifying anomaly without an exact
/// previous pointer is returned as an error so callers can page an operator
/// rather than treating missing rollback history as healthy.
/// Telemetry and rollback failures are returned so callers can log them as
/// operationally significant without changing the already-computed guard
/// decision.
pub fn check_and_rollback(
    state_store: &StateStore,
    trust_store: &TrustPointerStore,
    config: &PoisonPillConfig,
) -> Result<Option<RollbackReport>> {
    let Some(pointer) = trust_store.load()? else {
        return Ok(None);
    };

    let Some(deviation) = state_store.deny_rate_deviation_for(&pointer.trusted_ref)? else {
        return Ok(None);
    };

    // The release record may have been retained from an earlier adoption of
    // this reference. Require the current observation window to begin after
    // the pointer was advanced, unless a legacy pointer has no timestamp.
    if !release_is_fresh(state_store, &pointer)? {
        return Ok(None);
    }

    if !deviation.is_concerning(&config.policy) {
        return Ok(None);
    }

    if deviation.current_evaluation_count > config.max_current_evaluations {
        eprintln!(
            "⚠️  Poison-pill anomaly suppressed: release `{}` crossed the early observation window at {} evaluations (maximum {}).",
            deviation.release_ref,
            deviation.current_evaluation_count,
            config.max_current_evaluations
        );
        return Ok(None);
    }

    let threshold = deviation.baseline.mean_deny_rate
        + deviation.baseline.std_dev * config.policy.baseline_sigma_multiplier;
    let report = RollbackReport {
        release_ref: deviation.release_ref.clone(),
        previous_release: String::new(),
        current_deny_rate: deviation.current_deny_rate,
        baseline_mean: deviation.baseline.mean_deny_rate,
        baseline_std_dev: deviation.baseline.std_dev,
        absolute_deviation: deviation.absolute_deviation,
        current_evaluation_count: deviation.current_evaluation_count,
        baseline_release_count: deviation.baseline.release_count,
        threshold,
    };

    eprintln!(
        "🚨 POISON-PILL DENY-RATE SPIKE: release `{}` is at {:.2}% deny rate versus {:.2}% baseline (threshold {:.2}%, {} evaluations across {} prior releases).",
        report.release_ref,
        report.current_deny_rate * 100.0,
        report.baseline_mean * 100.0,
        report.threshold * 100.0,
        report.current_evaluation_count,
        report.baseline_release_count
    );

    if !config.enabled {
        eprintln!("⚠️  Automatic rollback is disabled; trust pointer was not changed.");
        return Ok(None);
    }

    if rollback_on_cooldown(state_store, config.cooldown)? {
        eprintln!(
            "⚠️  Automatic rollback is on cooldown for {:?}; trust pointer was not changed.",
            config.cooldown
        );
        return Ok(None);
    }

    let previous_release = state_store
        .previous_trusted_ref()?
        .filter(|release| !release.trim().is_empty())
        .context("poison-pill anomaly has no exact previous trusted release")?;

    if previous_release == pointer.trusted_ref {
        anyhow::bail!(
            "poison-pill rollback history is invalid: previous release `{}` equals current release",
            pointer.trusted_ref
        );
    }

    let reason = format!(
        "Automatic poison-pill rollback: release `{}` deny rate {:.4} exceeded prior-release baseline {:.4} at threshold {:.4}; {} evaluations, {} prior releases, deviation {:.4}",
        pointer.trusted_ref,
        deviation.current_deny_rate,
        deviation.baseline.mean_deny_rate,
        threshold,
        deviation.current_evaluation_count,
        deviation.baseline.release_count,
        deviation.absolute_deviation,
    );
    let rolled_back_pointer = TrustPointer::with_justification(&previous_release, &reason);

    eprintln!(
        "🚨 AUTO-ROLLBACK: reverting trust pointer from `{}` to exact prior release `{}`.",
        pointer.trusted_ref, previous_release
    );

    // TrustPointerStore performs the security checks and atomic write. State
    // metadata is updated immediately after so the event remains auditable.
    trust_store.save(&rolled_back_pointer)?;
    state_store.save_trust_pointer(&rolled_back_pointer)?;
    state_store.record_rollback(&pointer.trusted_ref, &previous_release, &reason)?;

    eprintln!(
        "✅ AUTO-ROLLBACK COMPLETE: trust pointer now names `{}`; release `{}` must not be promoted until investigated.",
        previous_release, pointer.trusted_ref
    );

    Ok(Some(RollbackReport {
        previous_release,
        ..report
    }))
}

fn release_is_fresh(state_store: &StateStore, pointer: &TrustPointer) -> Result<bool> {
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
    let first_observation = record
        .first_seen_at
        .parse::<DateTime<Utc>>()
        .with_context(|| {
            format!(
                "invalid first telemetry timestamp for `{}`: {}",
                pointer.trusted_ref, record.first_seen_at
            )
        })?;

    Ok(first_observation >= pointer_time)
}

fn rollback_on_cooldown(state_store: &StateStore, cooldown: Duration) -> Result<bool> {
    let Some(last_rollback_at) = state_store.rollback_state()?.last_rollback_at else {
        return Ok(false);
    };
    let timestamp = last_rollback_at
        .parse::<DateTime<Utc>>()
        .context("invalid last rollback timestamp in state store")?;
    let elapsed = Utc::now().signed_duration_since(timestamp);
    Ok(elapsed.to_std().unwrap_or(Duration::ZERO) < cooldown)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state_store::StateStore;
    use tempfile::tempdir;

    fn record_release(store: &StateStore, release_ref: &str, total: u64, denials: u64) {
        for index in 0..total {
            store
                .record_release_evaluation(release_ref, index < denials)
                .expect("record telemetry");
        }
    }

    fn store_and_pointer() -> (tempfile::TempDir, StateStore, TrustPointerStore) {
        let directory = tempdir().expect("temporary directory");
        let state_path = directory.path().join("state.json");
        let pointer_path = directory.path().join("trust-pointer.json");
        (
            directory,
            StateStore::new(state_path),
            TrustPointerStore::new(pointer_path),
        )
    }

    fn adopt(state_store: &StateStore, trust_store: &TrustPointerStore, release_ref: &str) {
        let pointer = TrustPointer::new(release_ref);
        trust_store.save(&pointer).expect("save pointer");
        state_store
            .save_trust_pointer(&pointer)
            .expect("record pointer");
    }

    #[test]
    fn qualifying_fresh_release_rolls_back_to_exact_prior_pointer() {
        let (_directory, state_store, trust_store) = store_and_pointer();
        adopt(&state_store, &trust_store, "v1");
        record_release(&state_store, "v1", 100, 1);
        adopt(&state_store, &trust_store, "v2");
        record_release(&state_store, "v2", 100, 2);
        adopt(&state_store, &trust_store, "v3");
        record_release(&state_store, "v3", 100, 1);
        adopt(&state_store, &trust_store, "v4");
        record_release(&state_store, "v4", 100, 20);

        let report = check_and_rollback(&state_store, &trust_store, &PoisonPillConfig::default())
            .expect("rollback check")
            .expect("qualifying anomaly");

        assert_eq!(report.release_ref, "v4");
        assert_eq!(report.previous_release, "v3");
        assert_eq!(
            trust_store.get_trusted_ref().expect("load pointer"),
            Some("v3".into())
        );
        let rollback = state_store.rollback_state().expect("rollback state");
        assert_eq!(rollback.current_release.as_deref(), Some("v3"));
        assert_eq!(rollback.previous_release.as_deref(), Some("v4"));
        assert_eq!(rollback.rollback_count, 1);
    }

    #[test]
    fn small_sample_does_not_rollback_even_with_all_denials() {
        let (_directory, state_store, trust_store) = store_and_pointer();
        adopt(&state_store, &trust_store, "v1");
        record_release(&state_store, "v1", 100, 1);
        adopt(&state_store, &trust_store, "v2");
        record_release(&state_store, "v2", 10, 10);

        assert!(
            check_and_rollback(&state_store, &trust_store, &PoisonPillConfig::default())
                .expect("rollback check")
                .is_none()
        );
        assert_eq!(
            trust_store.get_trusted_ref().expect("load pointer"),
            Some("v2".into())
        );
    }

    #[test]
    fn anomaly_after_early_window_does_not_rollback() {
        let (_directory, state_store, trust_store) = store_and_pointer();
        adopt(&state_store, &trust_store, "v1");
        record_release(&state_store, "v1", 100, 1);
        adopt(&state_store, &trust_store, "v2");
        record_release(&state_store, "v2", 100, 20);

        let config = PoisonPillConfig {
            max_current_evaluations: 99,
            ..Default::default()
        };
        assert!(check_and_rollback(&state_store, &trust_store, &config)
            .expect("rollback check")
            .is_none());
        assert_eq!(
            trust_store.get_trusted_ref().expect("load pointer"),
            Some("v2".into())
        );
    }

    #[test]
    fn no_previous_pointer_is_not_guessed() {
        let (_directory, state_store, trust_store) = store_and_pointer();
        adopt(&state_store, &trust_store, "v1");
        record_release(&state_store, "v0", 100, 1);
        record_release(&state_store, "v0.9", 100, 2);
        record_release(&state_store, "v0.8", 100, 1);
        record_release(&state_store, "v1", 100, 20);

        assert!(
            check_and_rollback(&state_store, &trust_store, &PoisonPillConfig::default()).is_err()
        );
        assert_eq!(
            trust_store.get_trusted_ref().expect("load pointer"),
            Some("v1".into())
        );
    }
}
