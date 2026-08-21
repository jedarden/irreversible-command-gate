//! Runtime monitoring integration for health, telemetry, rule packs, and logs.
//!
//! The guard is normally a short-lived hook or PATH-wrapper process, so the
//! durable files are the source of truth for scraping.  This module turns
//! those files into one consistent Prometheus snapshot.  It deliberately
//! treats an unreadable auxiliary file as a metric-bearing condition instead
//! of failing the scrape: an operator must be able to alert on the failure.

use anyhow::{Context, Result};
use chrono::{DateTime, Duration as ChronoDuration, Utc};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::denial_log::{DenialSeverity, DenialStore};
use crate::health::{HealthState, HealthStore};
use crate::metrics::{
    GuardMetrics, MetricsConfig, MetricsExporter, MetricsSnapshot, PackMetrics, RuleMetrics,
    TelemetryMetrics,
};
use crate::rule_pack::Pack;
use crate::telemetry::{compute_baseline, TelemetryStore};

const DEFAULT_HEALTH_PATH: &str = "/var/cache/icg/health-state.json";
const DEFAULT_TELEMETRY_PATH: &str = "/var/cache/icg/telemetry.json";
const DEFAULT_DENIAL_LOG_PATH: &str = "/var/cache/icg/denials.jsonl";
const DEFAULT_RULE_PACK_PATH: &str = "/etc/icg/rule-pack.json";

/// Paths and collection settings used by the health server and scrape tools.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MonitoringConfig {
    /// Durable guard lifecycle state.
    pub health_path: PathBuf,

    /// Rolling evaluation telemetry.
    pub telemetry_path: PathBuf,

    /// Structured JSONL denial log.
    pub denial_log_path: PathBuf,

    /// A rule-pack JSON file or directory of JSON packs.
    pub rule_pack_path: PathBuf,

    /// Instance label used to distinguish hosts in a shared scrape target.
    #[serde(default = "default_instance")]
    pub instance: String,

    /// Window for the recent denial gauges.
    #[serde(default = "default_denial_window")]
    pub denial_window: Duration,
}

fn default_instance() -> String {
    std::env::var("HOSTNAME").unwrap_or_else(|_| "unknown".to_string())
}

fn default_denial_window() -> Duration {
    Duration::from_secs(300)
}

fn env_path(name: &str, default: &str) -> PathBuf {
    std::env::var_os(name)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(default))
}

impl Default for MonitoringConfig {
    fn default() -> Self {
        Self {
            health_path: PathBuf::from(DEFAULT_HEALTH_PATH),
            telemetry_path: PathBuf::from(DEFAULT_TELEMETRY_PATH),
            denial_log_path: PathBuf::from(DEFAULT_DENIAL_LOG_PATH),
            rule_pack_path: PathBuf::from(DEFAULT_RULE_PACK_PATH),
            instance: default_instance(),
            denial_window: default_denial_window(),
        }
    }
}

impl MonitoringConfig {
    /// Build configuration from the environment used by the hook/container.
    pub fn from_environment() -> Self {
        let mut config = Self::default();
        config.health_path = env_path("ICG_HEALTH_PATH", DEFAULT_HEALTH_PATH);
        config.telemetry_path = env_path("ICG_TELEMETRY_PATH", DEFAULT_TELEMETRY_PATH);
        config.denial_log_path = env_path("ICG_DENIAL_LOG", DEFAULT_DENIAL_LOG_PATH);
        config.rule_pack_path = env_path("ICG_RULE_PACK", DEFAULT_RULE_PACK_PATH);
        if let Some(instance) =
            std::env::var_os("ICG_MONITOR_INSTANCE").filter(|value| !value.is_empty())
        {
            config.instance = instance.to_string_lossy().into_owned();
        }
        if let Ok(seconds) = std::env::var("ICG_DENIAL_WINDOW_SECONDS") {
            if let Ok(seconds) = seconds.parse::<u64>() {
                config.denial_window = Duration::from_secs(seconds.max(1));
            }
        }
        config
    }

    /// Return a copy that uses an explicitly supplied health state path.
    pub fn with_health_path(mut self, path: impl Into<PathBuf>) -> Self {
        self.health_path = path.into();
        self
    }
}

/// A denial counter grouped by the rule that produced it.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RuleDenialMetric {
    pub pack_id: String,
    pub pattern_id: String,
    pub count: u64,
}

/// Metrics which do not belong to the existing health or rolling telemetry
/// groups.  These are emitted by [`export_prometheus`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OperationalMetrics {
    pub collection_errors: u64,
    pub rule_pack_loaded: u8,
    pub rule_pack_load_errors: u64,
    pub rule_pack_count: u64,
    pub denial_log_readable: u8,
    pub total_denials: u64,
    pub recent_denials: u64,
    pub recent_critical_denials: u64,
    pub last_denial_timestamp: Option<f64>,
    pub denials_by_rule: Vec<RuleDenialMetric>,
}

/// Complete scrape result, including metric-bearing collection failures.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MonitoringSnapshot {
    pub metrics: MetricsSnapshot,
    pub operational: OperationalMetrics,
    pub packs: Vec<PackMetrics>,
}

/// Read all durable monitoring inputs into one snapshot.
pub fn collect_snapshot(config: &MonitoringConfig) -> Result<MonitoringSnapshot> {
    let mut collection_errors = 0u64;

    let health = match HealthStore::new(&config.health_path).health_metrics() {
        Ok(metrics) => metrics,
        Err(error) => {
            collection_errors += 1;
            eprintln!(
                "icg_monitoring_event event=health_read_failed path={} error={error:#}",
                config.health_path.display()
            );
            HealthState::new().compute_metrics()
        }
    };

    let telemetry = match TelemetryStore::load_or_create(config.telemetry_path.clone()) {
        Ok(store) => store,
        Err(error) => {
            collection_errors += 1;
            eprintln!(
                "icg_monitoring_event event=telemetry_read_failed path={} error={error:#}",
                config.telemetry_path.display()
            );
            TelemetryStore::new(config.telemetry_path.clone())
        }
    };

    let (denials, denial_log_readable) =
        match DenialStore::with_default_config(config.denial_log_path.clone())
            .and_then(|store| store.get_all_denials())
        {
            Ok(denials) => (denials, 1),
            Err(error) => {
                collection_errors += 1;
                eprintln!(
                    "icg_monitoring_event event=denial_log_read_failed path={} error={error:#}",
                    config.denial_log_path.display()
                );
                (Vec::new(), 0)
            }
        };

    let (packs, rule_pack_load_errors) = load_pack_metrics(&config.rule_pack_path);
    let rule_pack_loaded = u8::from(!packs.is_empty());
    if rule_pack_load_errors > 0 || packs.is_empty() {
        collection_errors += 1;
    }

    let baseline = compute_baseline(telemetry.window());
    let health_metrics = GuardMetrics::from_health_metrics(&health);
    let telemetry_metrics = TelemetryMetrics {
        baseline_evaluations: baseline.total_evaluations as u64,
        baseline_denies: baseline.deny_count as u64,
        baseline_deny_rate: baseline.deny_rate,
        baseline_mean: baseline.mean,
        baseline_stddev: baseline.std_dev,
        baseline_min: baseline.min,
        baseline_max: baseline.max,
        baseline_window_start: baseline
            .window_start
            .map(|timestamp| timestamp.timestamp() as f64),
        baseline_window_end: baseline
            .window_end
            .map(|timestamp| timestamp.timestamp() as f64),
        current_deny_rate: baseline.deny_rate,
        rollback_on_cooldown: u8::from(telemetry.is_rollback_on_cooldown()),
        last_rollback_timestamp: telemetry
            .last_rollback_at()
            .map(|timestamp| timestamp.timestamp() as f64),
    };
    let rules = telemetry
        .rule_metrics()
        .map(|rule| {
            let deny_rate = if rule.match_count == 0 {
                0.0
            } else {
                rule.deny_count as f64 / rule.match_count as f64
            };
            RuleMetrics {
                pattern_id: rule.pattern_id.clone(),
                pack_id: rule.pack_id.clone(),
                match_count: rule.match_count,
                deny_count: rule.deny_count,
                deny_rate,
                enabled: 1,
            }
        })
        .collect();

    let now = Utc::now();
    let cutoff = now
        - ChronoDuration::from_std(config.denial_window)
            .context("denial window is outside chrono's supported range")?;
    let recent_denials = denials
        .iter()
        .filter(|record| record.timestamp >= cutoff)
        .collect::<Vec<_>>();
    let recent_critical_denials = recent_denials
        .iter()
        .filter(|record| record.severity == DenialSeverity::Critical)
        .count() as u64;
    let last_denial_timestamp = denials
        .iter()
        .map(|record| record.timestamp)
        .max()
        .map(|timestamp| timestamp.timestamp() as f64);
    let mut by_rule = BTreeMap::<(String, String), u64>::new();
    for record in &denials {
        *by_rule
            .entry((record.pack_id.clone(), record.pattern_id.clone()))
            .or_default() += 1;
    }
    let denials_by_rule = by_rule
        .into_iter()
        .map(|((pack_id, pattern_id), count)| RuleDenialMetric {
            pack_id,
            pattern_id,
            count,
        })
        .collect();

    Ok(MonitoringSnapshot {
        metrics: MetricsSnapshot {
            timestamp: now,
            health: health_metrics,
            telemetry: telemetry_metrics,
            pack: packs.first().cloned(),
            rules,
        },
        operational: OperationalMetrics {
            collection_errors,
            rule_pack_loaded,
            rule_pack_load_errors,
            rule_pack_count: packs.len() as u64,
            denial_log_readable,
            total_denials: denials.len() as u64,
            recent_denials: recent_denials.len() as u64,
            recent_critical_denials,
            last_denial_timestamp,
            denials_by_rule,
        },
        packs,
    })
}

/// Render a complete Prometheus text exposition scrape.
pub fn export_prometheus(config: &MonitoringConfig) -> Result<String> {
    let snapshot = collect_snapshot(config)?;
    let mut metrics_config = MetricsConfig::default();
    metrics_config.include_rule_metrics = true;
    let mut output = MetricsExporter::new(metrics_config).export_metrics(&snapshot.metrics)?;

    output.push_str("\n# Operational monitoring metrics\n");
    output.push_str("# HELP icg_monitoring_collection_errors Number of durable monitoring inputs that could not be read\n");
    output.push_str("# TYPE icg_monitoring_collection_errors gauge\n");
    output.push_str(&format!(
        "icg_monitoring_collection_errors {}\n",
        snapshot.operational.collection_errors
    ));
    output.push_str(
        "# HELP icg_rule_pack_loaded Whether at least one rule pack loaded successfully\n",
    );
    output.push_str("# TYPE icg_rule_pack_loaded gauge\n");
    output.push_str(&format!(
        "icg_rule_pack_loaded {}\n",
        snapshot.operational.rule_pack_loaded
    ));
    output.push_str(
        "# HELP icg_rule_pack_load_errors Number of rule-pack files that failed to load\n",
    );
    output.push_str("# TYPE icg_rule_pack_load_errors gauge\n");
    output.push_str(&format!(
        "icg_rule_pack_load_errors {}\n",
        snapshot.operational.rule_pack_load_errors
    ));
    output.push_str("# HELP icg_rule_pack_count Number of rule packs discovered\n");
    output.push_str("# TYPE icg_rule_pack_count gauge\n");
    output.push_str(&format!(
        "icg_rule_pack_count {}\n",
        snapshot.operational.rule_pack_count
    ));
    output.push_str("# HELP icg_denial_log_readable Whether the denial log can be read\n");
    output.push_str("# TYPE icg_denial_log_readable gauge\n");
    output.push_str(&format!(
        "icg_denial_log_readable {}\n",
        snapshot.operational.denial_log_readable
    ));
    output.push_str("# HELP icg_denials_total Total denied operations in the retained log\n");
    output.push_str("# TYPE icg_denials_total counter\n");
    output.push_str(&format!(
        "icg_denials_total {}\n",
        snapshot.operational.total_denials
    ));
    output
        .push_str("# HELP icg_denials_recent Denied operations in the configured recent window\n");
    output.push_str("# TYPE icg_denials_recent gauge\n");
    output.push_str(&format!(
        "icg_denials_recent {}\n",
        snapshot.operational.recent_denials
    ));
    output.push_str(
        "# HELP icg_critical_denials_recent Critical denials in the configured recent window\n",
    );
    output.push_str("# TYPE icg_critical_denials_recent gauge\n");
    output.push_str(&format!(
        "icg_critical_denials_recent {}\n",
        snapshot.operational.recent_critical_denials
    ));
    if let Some(timestamp) = snapshot.operational.last_denial_timestamp {
        output.push_str(
            "# HELP icg_last_denial_timestamp Unix timestamp of the most recent denial\n",
        );
        output.push_str("# TYPE icg_last_denial_timestamp gauge\n");
        output.push_str(&format!("icg_last_denial_timestamp {timestamp}\n"));
    }

    output.push_str("# HELP icg_denials_by_rule_total Denied operations grouped by rule\n");
    output.push_str("# TYPE icg_denials_by_rule_total counter\n");
    for rule in &snapshot.operational.denials_by_rule {
        output.push_str(&format!(
            "icg_denials_by_rule_total{{pack_id=\"{}\",pattern_id=\"{}\"}} {}\n",
            escape_label(&rule.pack_id),
            escape_label(&rule.pattern_id),
            rule.count
        ));
    }

    // A directory can contain several packs.  The legacy exporter has one
    // optional pack slot, so emit the additional pack metadata here.
    if snapshot.packs.len() > 1 {
        output.push_str("# HELP icg_rule_pack_info Information about each discovered rule pack\n");
        output.push_str("# TYPE icg_rule_pack_info info\n");
        for pack in &snapshot.packs {
            output.push_str(&format!(
                "icg_rule_pack_info{{pack_id=\"{}\",version=\"{}\"}} 1\n",
                escape_label(&pack.pack_id),
                escape_label(&pack.pack_version)
            ));
        }
    }

    Ok(output)
}

fn load_pack_metrics(path: &Path) -> (Vec<PackMetrics>, u64) {
    let paths = if path.is_dir() {
        let Ok(entries) = std::fs::read_dir(path) else {
            return (Vec::new(), 1);
        };
        let mut paths = entries
            .filter_map(|entry| entry.ok().map(|entry| entry.path()))
            .filter(|entry| entry.extension().and_then(|value| value.to_str()) == Some("json"))
            .collect::<Vec<_>>();
        paths.sort();
        paths
    } else {
        vec![path.to_path_buf()]
    };

    if paths.is_empty() {
        return (Vec::new(), 1);
    }

    let mut metrics = Vec::new();
    let mut errors = 0;
    for path in paths {
        match crate::rule_pack::load_pack(&path) {
            Ok(pack) => metrics.push(pack_metrics(&pack, &path)),
            Err(error) => {
                errors += 1;
                eprintln!(
                    "icg_monitoring_event event=rule_pack_load_failed path={} error={error:#}",
                    path.display()
                );
            }
        }
    }
    (metrics, errors)
}

fn pack_metrics(pack: &Pack, path: &Path) -> PackMetrics {
    let total_patterns = (pack.safe_patterns.len() + pack.guarded_patterns.len()) as u64;
    let enabled_patterns = (pack.safe_patterns.len()
        + pack
            .guarded_patterns
            .iter()
            .filter(|pattern| pattern.enabled)
            .count()) as u64;
    let disabled_patterns = pack
        .guarded_patterns
        .iter()
        .filter(|pattern| !pattern.enabled)
        .count() as u64;
    let pack_created_at = std::fs::metadata(path)
        .and_then(|metadata| metadata.modified())
        .ok()
        .map(DateTime::<Utc>::from)
        .map(|timestamp| timestamp.timestamp() as f64);
    PackMetrics {
        pack_id: pack.id.clone(),
        pack_version: "unknown".to_string(),
        total_patterns,
        enabled_patterns,
        disabled_patterns,
        pack_created_at,
    }
}

fn escape_label(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('\n', "\\n")
        .replace('"', "\\\"")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::denial_log::{DenialLogConfig, DenialRecord, DeniedInput};
    use crate::health::CrashType;
    use tempfile::tempdir;

    #[test]
    fn collects_durable_inputs_and_emits_operational_metrics() -> Result<()> {
        let dir = tempdir()?;
        let health_path = dir.path().join("health.json");
        let telemetry_path = dir.path().join("telemetry.json");
        let denial_path = dir.path().join("denials.jsonl");
        let pack_path = dir.path().join("pack.json");

        let health_store = HealthStore::new(&health_path);
        health_store.mark_start()?;
        health_store.record_crash(crate::health::CrashRecord::new(CrashType::Abort))?;

        let mut telemetry = TelemetryStore::new(telemetry_path.clone());
        telemetry.record_evaluation_for_rule(
            crate::telemetry::Verdict::Denied,
            None,
            None,
            Some("git"),
            Some("force-push"),
        );
        telemetry.persist()?;

        let denial_store = DenialStore::new(denial_path.clone(), DenialLogConfig::default())?;
        denial_store.record_denial(DenialRecord::new(
            "git".to_string(),
            "force-push".to_string(),
            "destructive".to_string(),
            DenialSeverity::Critical,
            "blocked".to_string(),
            DeniedInput::Command {
                command: "git push --force".to_string(),
                segments: vec!["git".to_string()],
                working_dir: None,
            },
        ))?;
        std::fs::write(
            &pack_path,
            r#"{
            "id": "git",
            "tool_keywords": ["git"],
            "guarded_patterns": []
        }"#,
        )?;

        let config = MonitoringConfig {
            health_path,
            telemetry_path,
            denial_log_path: denial_path,
            rule_pack_path: pack_path,
            instance: "test".to_string(),
            denial_window: Duration::from_secs(300),
        };
        let snapshot = collect_snapshot(&config)?;
        assert_eq!(snapshot.operational.total_denials, 1);
        assert_eq!(snapshot.operational.recent_critical_denials, 1);
        assert_eq!(snapshot.operational.rule_pack_loaded, 1);
        assert_eq!(snapshot.metrics.telemetry.baseline_deny_rate, 1.0);

        let output = export_prometheus(&config)?;
        assert!(output.contains("icg_rule_pack_loaded 1"));
        assert!(output.contains("icg_rule_deny_count{pack_id=\"git\",pattern_id=\"force-push\"} 1"));
        assert!(output
            .contains("icg_denials_by_rule_total{pack_id=\"git\",pattern_id=\"force-push\"} 1"));
        Ok(())
    }

    #[test]
    fn malformed_pack_is_visible_as_a_metric() -> Result<()> {
        let dir = tempdir()?;
        let config = MonitoringConfig {
            rule_pack_path: dir.path().join("missing.json"),
            health_path: dir.path().join("health.json"),
            telemetry_path: dir.path().join("telemetry.json"),
            denial_log_path: dir.path().join("denials.jsonl"),
            ..Default::default()
        };
        let snapshot = collect_snapshot(&config)?;
        assert_eq!(snapshot.operational.rule_pack_loaded, 0);
        assert_eq!(snapshot.operational.rule_pack_load_errors, 1);
        assert!(export_prometheus(&config)?.contains("icg_rule_pack_load_errors 1"));
        Ok(())
    }
}
