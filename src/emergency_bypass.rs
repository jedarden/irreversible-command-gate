//! Explicit, auditable emergency-bypass policy shared by every enforcement
//! front end.
//!
//! `ICG_DISABLED=1` is intentionally an operator-controlled escape hatch. It
//! takes precedence over ordinary rule evaluation *and* guard-availability
//! fail-open/fail-closed handling. A failure to write the auxiliary telemetry
//! must not turn an approved emergency operation into an outage, so it is
//! reported to stderr and the bypass remains active.

use crate::telemetry::TelemetryStore;
use std::path::PathBuf;

pub const ENVIRONMENT_VARIABLE: &str = "ICG_DISABLED";
pub const WARNING: &str =
    "WARNING: ICG_DISABLED emergency bypass active; enforcement is disabled for this invocation.";

const DEFAULT_TELEMETRY_PATH: &str = "/var/cache/icg/telemetry.json";

/// The entry point that was explicitly bypassed. This is the only invocation
/// detail retained in emergency-bypass telemetry; commands and tool payloads
/// can contain credentials or other secrets and must never be recorded here.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrontEnd {
    Check,
    Hook,
    Wrapper,
}

impl FrontEnd {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Check => "check",
            Self::Hook => "hook",
            Self::Wrapper => "wrapper",
        }
    }
}

/// Return whether the documented emergency escape hatch is active.
///
/// `1` is the documented spelling. `true` remains accepted for compatibility
/// with the already-released `icg check` behavior; all front ends now apply the
/// same parsing policy.
pub fn is_active() -> bool {
    std::env::var(ENVIRONMENT_VARIABLE)
        .map(|value| value == "1" || value.eq_ignore_ascii_case("true"))
        .unwrap_or(false)
}

/// Emit an activation event without accepting command or tool-input data.
///
/// The stderr event is deliberately structured for log shippers and provides
/// an audit signal even when the telemetry cache is unavailable. The durable
/// telemetry event is best effort: bypass activation remains fail-open with
/// respect to telemetry persistence so an incident recovery is not blocked by
/// a read-only or damaged cache.
pub fn record_activation(front_end: FrontEnd) {
    eprintln!(
        "icg_emergency_bypass event=activated front_end={} command_data=omitted",
        front_end.as_str()
    );

    let path = std::env::var_os("ICG_TELEMETRY_PATH")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(DEFAULT_TELEMETRY_PATH));
    let result = (|| {
        let mut telemetry = TelemetryStore::load_or_create(path.clone())?;
        telemetry.record_emergency_bypass(front_end.as_str());
        telemetry.persist()
    })();

    if let Err(error) = result {
        eprintln!(
            "icg_emergency_bypass event=telemetry_write_failed front_end={} command_data=omitted error={error:#}",
            front_end.as_str(),
        );
    }
}
