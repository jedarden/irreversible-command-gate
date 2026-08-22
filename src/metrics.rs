//! Prometheus metrics export for monitoring and alerting
//!
//! This module exposes guard metrics in Prometheus text-based exposition format
//! for scraping by monitoring systems like Prometheus, VictoriaMetrics, or Grafana Agent.
//!
//! ## Architecture
//!
//! Metrics are organized into these groups:
//! - **Guard Health**: Process uptime, crashes, stability status
//! - **Evaluation Metrics**: Deny rates, operation counts, latency
//! - **Telemetry**: Baseline statistics, anomaly detection
//! - **Rule Pack**: Pack version, pattern counts, coverage
//!
//! ## Metric Types
//!
//! - **Gauge**: Current values (uptime, crash count, health status)
//! - **Counter**: Cumulative values (total evaluations, total denials)
//! - **Histogram**: Bucketed values (evaluation latency)
//! - **Info**: Static metadata (version, pack_id)
//!
//! ## Usage
//!
//! ```rust
//! use icg::metrics::{MetricsExporter, MetricsConfig};
//!
//! let exporter = MetricsExporter::new(MetricsConfig::default());
//! let prometheus_text = exporter.export_metrics()?;
//! println!("{}", prometheus_text);
//! ```

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;
use std::time::Duration;

/// Prometheus metrics configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MetricsConfig {
    /// Enable or disable metrics export
    pub enabled: bool,

    /// Include guard health metrics
    pub include_health: bool,

    /// Include telemetry metrics
    pub include_telemetry: bool,

    /// Include rule pack metadata
    pub include_pack_info: bool,

    /// Include per-rule metrics (can be verbose for large packs)
    pub include_rule_metrics: bool,

    /// Prefix for all metric names (default: "icg_")
    pub metric_prefix: String,
}

impl Default for MetricsConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            include_health: true,
            include_telemetry: true,
            include_pack_info: true,
            include_rule_metrics: false, // Disabled by default for large packs
            metric_prefix: "icg_".to_string(),
        }
    }
}

/// Metrics collected from the guard system
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GuardMetrics {
    /// Process uptime in seconds
    pub uptime_seconds: f64,

    /// Total number of crashes
    pub total_crashes: u64,

    /// Number of crashes in the last hour
    pub recent_crashes: u64,

    /// Current crash rate (crashes per hour)
    pub crash_rate: f64,

    /// Number of consecutive clean runs
    pub consecutive_clean_runs: u64,

    /// Health status code (0=Unknown, 1=Healthy, 2=Recovering, 3=Unstable, 4=Degraded, 5=Dead)
    pub health_status: u8,

    /// Whether the process is currently stable
    pub is_stable: u8,

    /// Time since the process became stable (seconds)
    pub time_since_stable_seconds: Option<f64>,

    /// Timestamp of the last crash
    pub last_crash_timestamp: Option<f64>,

    /// Timestamp of the last process start
    pub last_start_timestamp: Option<f64>,
}

impl GuardMetrics {
    /// Convert the durable health snapshot into the metrics representation
    /// used by the Prometheus exporter.
    pub fn from_health_metrics(metrics: &crate::health::HealthMetrics) -> Self {
        Self {
            uptime_seconds: metrics
                .current_uptime
                .map(|uptime| uptime.as_secs_f64())
                .unwrap_or(0.0),
            total_crashes: metrics.total_crashes as u64,
            recent_crashes: metrics.recent_crashes as u64,
            crash_rate: metrics.crash_rate,
            consecutive_clean_runs: metrics.consecutive_clean_runs as u64,
            health_status: health_status_to_code(metrics.status),
            is_stable: u8::from(metrics.is_stable),
            time_since_stable_seconds: metrics
                .time_since_stable
                .map(|duration| duration.as_secs_f64()),
            last_crash_timestamp: metrics
                .last_crash_at
                .map(|timestamp| timestamp.timestamp() as f64),
            last_start_timestamp: metrics
                .last_start_at
                .map(|timestamp| timestamp.timestamp() as f64),
        }
    }
}

/// Telemetry metrics for deny-rate monitoring
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TelemetryMetrics {
    /// Total evaluations in the baseline window
    pub baseline_evaluations: u64,

    /// Number of denies in the baseline window
    pub baseline_denies: u64,

    /// Baseline deny rate (0.0 to 1.0)
    pub baseline_deny_rate: f64,

    /// Baseline mean deny rate
    pub baseline_mean: f64,

    /// Baseline standard deviation
    pub baseline_stddev: f64,

    /// Minimum deny rate in baseline
    pub baseline_min: f64,

    /// Maximum deny rate in baseline
    pub baseline_max: f64,

    /// Baseline window start timestamp
    pub baseline_window_start: Option<f64>,

    /// Baseline window end timestamp
    pub baseline_window_end: Option<f64>,

    /// Current deny rate
    pub current_deny_rate: f64,

    /// Whether rollback is on cooldown
    pub rollback_on_cooldown: u8,

    /// Timestamp of last rollback
    pub last_rollback_timestamp: Option<f64>,
}

/// Rule pack metadata
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PackMetrics {
    /// Rule pack ID
    pub pack_id: String,

    /// Pack version or release reference
    pub pack_version: String,

    /// Total number of patterns in the pack
    pub total_patterns: u64,

    /// Number of enabled patterns
    pub enabled_patterns: u64,

    /// Number of disabled patterns
    pub disabled_patterns: u64,

    /// Pack creation timestamp
    pub pack_created_at: Option<f64>,
}

/// Per-rule metrics (only included if include_rule_metrics is enabled)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuleMetrics {
    /// Pattern ID
    pub pattern_id: String,

    /// Pack ID this pattern belongs to
    pub pack_id: String,

    /// Number of times this rule matched
    pub match_count: u64,

    /// Number of times this rule denied
    pub deny_count: u64,

    /// Deny rate for this rule (0.0 to 1.0)
    pub deny_rate: f64,

    /// Whether this rule is enabled
    pub enabled: u8,
}

/// Complete metrics snapshot
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MetricsSnapshot {
    /// Timestamp when this snapshot was taken
    pub timestamp: DateTime<Utc>,

    /// Guard health metrics
    pub health: GuardMetrics,

    /// Telemetry metrics
    pub telemetry: TelemetryMetrics,

    /// Rule pack metadata
    pub pack: Option<PackMetrics>,

    /// Per-rule metrics (optional, can be large)
    pub rules: Vec<RuleMetrics>,
}

/// Prometheus metrics exporter
pub struct MetricsExporter {
    config: MetricsConfig,
}

impl MetricsExporter {
    /// Create a new metrics exporter with default configuration
    pub fn new(config: MetricsConfig) -> Self {
        Self { config }
    }

    /// Export metrics in Prometheus text-based exposition format
    pub fn export_metrics(&self, snapshot: &MetricsSnapshot) -> Result<String> {
        if !self.config.enabled {
            return Ok(String::new());
        }

        let mut output = String::new();
        let prefix = &self.config.metric_prefix;

        // Add HELP and TYPE meta comments for each metric
        if self.config.include_health {
            output.push_str("# Guard Health Metrics\n\n");

            output.push_str(&format!(
                "# HELP {}uptime_seconds Uptime of the guard process in seconds\n",
                prefix
            ));
            output.push_str(&format!("# TYPE {}uptime_seconds gauge\n", prefix));
            output.push_str(&format!(
                "{}uptime_seconds {}\n",
                prefix, snapshot.health.uptime_seconds
            ));
            output.push('\n');

            output.push_str(&format!(
                "# HELP {}total_crashes Total number of guard process crashes\n",
                prefix
            ));
            output.push_str(&format!("# TYPE {}total_crashes counter\n", prefix));
            output.push_str(&format!(
                "{}total_crashes {}\n",
                prefix, snapshot.health.total_crashes
            ));
            output.push('\n');

            output.push_str(&format!(
                "# HELP {}recent_crashes Number of crashes in the last hour\n",
                prefix
            ));
            output.push_str(&format!("# TYPE {}recent_crashes gauge\n", prefix));
            output.push_str(&format!(
                "{}recent_crashes {}\n",
                prefix, snapshot.health.recent_crashes
            ));
            output.push('\n');

            output.push_str(&format!(
                "# HELP {}crash_rate Current crash rate (crashes per hour)\n",
                prefix
            ));
            output.push_str(&format!("# TYPE {}crash_rate gauge\n", prefix));
            output.push_str(&format!(
                "{}crash_rate {}\n",
                prefix, snapshot.health.crash_rate
            ));
            output.push('\n');

            output.push_str(&format!(
                "# HELP {}consecutive_clean_runs Number of consecutive clean process exits\n",
                prefix
            ));
            output.push_str(&format!("# TYPE {}consecutive_clean_runs gauge\n", prefix));
            output.push_str(&format!(
                "{}consecutive_clean_runs {}\n",
                prefix, snapshot.health.consecutive_clean_runs
            ));
            output.push('\n');

            output.push_str(&format!("# HELP {}health_status Current health status code (0=Unknown, 1=Healthy, 2=Recovering, 3=Unstable, 4=Degraded, 5=Dead)\n", prefix));
            output.push_str(&format!("# TYPE {}health_status gauge\n", prefix));
            output.push_str(&format!(
                "{}health_status {}\n",
                prefix, snapshot.health.health_status
            ));
            output.push('\n');

            output.push_str(&format!(
                "# HELP {}is_stable Whether the process is currently stable (1=yes, 0=no)\n",
                prefix
            ));
            output.push_str(&format!("# TYPE {}is_stable gauge\n", prefix));
            output.push_str(&format!(
                "{}is_stable {}\n",
                prefix, snapshot.health.is_stable
            ));
            output.push('\n');

            if let Some(time_stable) = snapshot.health.time_since_stable_seconds {
                output.push_str(&format!("# HELP {}time_since_stable_seconds Time since the process became stable in seconds\n", prefix));
                output.push_str(&format!(
                    "# TYPE {}time_since_stable_seconds gauge\n",
                    prefix
                ));
                output.push_str(&format!(
                    "{}time_since_stable_seconds {}\n",
                    prefix, time_stable
                ));
                output.push('\n');
            }

            if let Some(last_crash) = snapshot.health.last_crash_timestamp {
                output.push_str(&format!(
                    "# HELP {}last_crash_timestamp Unix timestamp of the last crash\n",
                    prefix
                ));
                output.push_str(&format!("# TYPE {}last_crash_timestamp gauge\n", prefix));
                output.push_str(&format!("{}last_crash_timestamp {}\n", prefix, last_crash));
                output.push('\n');
            }

            if let Some(last_start) = snapshot.health.last_start_timestamp {
                output.push_str(&format!(
                    "# HELP {}last_start_timestamp Unix timestamp of the last process start\n",
                    prefix
                ));
                output.push_str(&format!("# TYPE {}last_start_timestamp gauge\n", prefix));
                output.push_str(&format!("{}last_start_timestamp {}\n", prefix, last_start));
                output.push('\n');
            }
        }

        if self.config.include_telemetry {
            output.push_str("# Telemetry Metrics\n\n");

            output.push_str(&format!(
                "# HELP {}baseline_evaluations Total evaluations in the baseline window\n",
                prefix
            ));
            output.push_str(&format!("# TYPE {}baseline_evaluations gauge\n", prefix));
            output.push_str(&format!(
                "{}baseline_evaluations {}\n",
                prefix, snapshot.telemetry.baseline_evaluations
            ));
            output.push('\n');

            output.push_str(&format!(
                "# HELP {}baseline_denies Number of denies in the baseline window\n",
                prefix
            ));
            output.push_str(&format!("# TYPE {}baseline_denies gauge\n", prefix));
            output.push_str(&format!(
                "{}baseline_denies {}\n",
                prefix, snapshot.telemetry.baseline_denies
            ));
            output.push('\n');

            output.push_str(&format!(
                "# HELP {}baseline_deny_rate Baseline deny rate (0.0 to 1.0)\n",
                prefix
            ));
            output.push_str(&format!("# TYPE {}baseline_deny_rate gauge\n", prefix));
            output.push_str(&format!(
                "{}baseline_deny_rate {}\n",
                prefix, snapshot.telemetry.baseline_deny_rate
            ));
            output.push('\n');

            output.push_str(&format!(
                "# HELP {}baseline_mean Baseline mean deny rate\n",
                prefix
            ));
            output.push_str(&format!("# TYPE {}baseline_mean gauge\n", prefix));
            output.push_str(&format!(
                "{}baseline_mean {}\n",
                prefix, snapshot.telemetry.baseline_mean
            ));
            output.push('\n');

            output.push_str(&format!(
                "# HELP {}baseline_stddev Baseline standard deviation\n",
                prefix
            ));
            output.push_str(&format!("# TYPE {}baseline_stddev gauge\n", prefix));
            output.push_str(&format!(
                "{}baseline_stddev {}\n",
                prefix, snapshot.telemetry.baseline_stddev
            ));
            output.push('\n');

            output.push_str(&format!(
                "# HELP {}baseline_min Minimum deny rate in baseline\n",
                prefix
            ));
            output.push_str(&format!("# TYPE {}baseline_min gauge\n", prefix));
            output.push_str(&format!(
                "{}baseline_min {}\n",
                prefix, snapshot.telemetry.baseline_min
            ));
            output.push('\n');

            output.push_str(&format!(
                "# HELP {}baseline_max Maximum deny rate in baseline\n",
                prefix
            ));
            output.push_str(&format!("# TYPE {}baseline_max gauge\n", prefix));
            output.push_str(&format!(
                "{}baseline_max {}\n",
                prefix, snapshot.telemetry.baseline_max
            ));
            output.push('\n');

            output.push_str(&format!(
                "# HELP {}current_deny_rate Current deny rate (0.0 to 1.0)\n",
                prefix
            ));
            output.push_str(&format!("# TYPE {}current_deny_rate gauge\n", prefix));
            output.push_str(&format!(
                "{}current_deny_rate {}\n",
                prefix, snapshot.telemetry.current_deny_rate
            ));
            output.push('\n');

            output.push_str(&format!(
                "# HELP {}rollback_on_cooldown Whether rollback is on cooldown (1=yes, 0=no)\n",
                prefix
            ));
            output.push_str(&format!("# TYPE {}rollback_on_cooldown gauge\n", prefix));
            output.push_str(&format!(
                "{}rollback_on_cooldown {}\n",
                prefix, snapshot.telemetry.rollback_on_cooldown
            ));
            output.push('\n');

            if let Some(last_rollback) = snapshot.telemetry.last_rollback_timestamp {
                output.push_str(&format!(
                    "# HELP {}last_rollback_timestamp Unix timestamp of the last rollback\n",
                    prefix
                ));
                output.push_str(&format!("# TYPE {}last_rollback_timestamp gauge\n", prefix));
                output.push_str(&format!(
                    "{}last_rollback_timestamp {}\n",
                    prefix, last_rollback
                ));
                output.push('\n');
            }
        }

        if let Some(ref pack) = snapshot.pack {
            if self.config.include_pack_info {
                output.push_str("# Rule Pack Metrics\n\n");

                output.push_str(&format!(
                    "# HELP {}pack_info Information about the rule pack\n",
                    prefix
                ));
                output.push_str(&format!("# TYPE {}pack_info info\n", prefix));
                output.push_str(&format!(
                    "{}pack_info{{pack_id=\"{}\",version=\"{}\"}} 1\n",
                    prefix,
                    escape_label(&pack.pack_id),
                    escape_label(&pack.pack_version)
                ));
                output.push('\n');

                output.push_str(&format!(
                    "# HELP {}total_patterns Total number of patterns in the pack\n",
                    prefix
                ));
                output.push_str(&format!("# TYPE {}total_patterns gauge\n", prefix));
                output.push_str(&format!(
                    "{}total_patterns {}\n",
                    prefix, pack.total_patterns
                ));
                output.push('\n');

                output.push_str(&format!(
                    "# HELP {}enabled_patterns Number of enabled patterns\n",
                    prefix
                ));
                output.push_str(&format!("# TYPE {}enabled_patterns gauge\n", prefix));
                output.push_str(&format!(
                    "{}enabled_patterns {}\n",
                    prefix, pack.enabled_patterns
                ));
                output.push('\n');

                output.push_str(&format!(
                    "# HELP {}disabled_patterns Number of disabled patterns\n",
                    prefix
                ));
                output.push_str(&format!("# TYPE {}disabled_patterns gauge\n", prefix));
                output.push_str(&format!(
                    "{}disabled_patterns {}\n",
                    prefix, pack.disabled_patterns
                ));
                output.push('\n');
            }
        }

        if self.config.include_rule_metrics && !snapshot.rules.is_empty() {
            output.push_str("# Per-Rule Metrics\n\n");

            output.push_str(&format!(
                "# HELP {}rule_match_count Total number of matches for this rule\n",
                prefix
            ));
            output.push_str(&format!("# TYPE {}rule_match_count counter\n", prefix));

            for rule in &snapshot.rules {
                output.push_str(&format!(
                    "{}rule_match_count{{pack_id=\"{}\",pattern_id=\"{}\"}} {}\n",
                    prefix,
                    escape_label(&rule.pack_id),
                    escape_label(&rule.pattern_id),
                    rule.match_count
                ));
            }
            output.push('\n');

            output.push_str(&format!(
                "# HELP {}rule_deny_count Total number of denies for this rule\n",
                prefix
            ));
            output.push_str(&format!("# TYPE {}rule_deny_count counter\n", prefix));

            for rule in &snapshot.rules {
                output.push_str(&format!(
                    "{}rule_deny_count{{pack_id=\"{}\",pattern_id=\"{}\"}} {}\n",
                    prefix,
                    escape_label(&rule.pack_id),
                    escape_label(&rule.pattern_id),
                    rule.deny_count
                ));
            }
            output.push('\n');

            output.push_str(&format!(
                "# HELP {}rule_deny_rate Deny rate for this rule (0.0 to 1.0)\n",
                prefix
            ));
            output.push_str(&format!("# TYPE {}rule_deny_rate gauge\n", prefix));

            for rule in &snapshot.rules {
                output.push_str(&format!(
                    "{}rule_deny_rate{{pack_id=\"{}\",pattern_id=\"{}\"}} {}\n",
                    prefix,
                    escape_label(&rule.pack_id),
                    escape_label(&rule.pattern_id),
                    rule.deny_rate
                ));
            }
            output.push('\n');

            output.push_str(&format!(
                "# HELP {}rule_enabled Whether this rule is enabled (1=yes, 0=no)\n",
                prefix
            ));
            output.push_str(&format!("# TYPE {}rule_enabled gauge\n", prefix));

            for rule in &snapshot.rules {
                output.push_str(&format!(
                    "{}rule_enabled{{pack_id=\"{}\",pattern_id=\"{}\"}} {}\n",
                    prefix,
                    escape_label(&rule.pack_id),
                    escape_label(&rule.pattern_id),
                    rule.enabled
                ));
            }
            output.push('\n');
        }

        // Add scrape timestamp
        let scrape_timestamp = snapshot.timestamp.timestamp();
        output.push_str(&format!(
            "# HELP {}scrape_timestamp Unix timestamp when metrics were scraped\n",
            prefix
        ));
        output.push_str(&format!("# TYPE {}scrape_timestamp gauge\n", prefix));
        output.push_str(&format!(
            "{}scrape_timestamp {}\n",
            prefix, scrape_timestamp
        ));

        Ok(output)
    }

    /// Export metrics to a file (for testing or file-based discovery)
    pub fn export_to_file(&self, snapshot: &MetricsSnapshot, path: &Path) -> Result<()> {
        let content = self.export_metrics(snapshot)?;
        std::fs::write(path, content)
            .with_context(|| format!("Failed to write metrics to {}", path.display()))?;
        Ok(())
    }
}

fn escape_label(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('\n', "\\n")
        .replace('"', "\\\"")
}

impl Default for MetricsExporter {
    fn default() -> Self {
        Self::new(MetricsConfig::default())
    }
}

/// Convert health::HealthStatus to Prometheus status code
pub fn health_status_to_code(status: crate::health::HealthStatus) -> u8 {
    match status {
        crate::health::HealthStatus::Unknown => 0,
        crate::health::HealthStatus::Healthy => 1,
        crate::health::HealthStatus::Recovering => 2,
        crate::health::HealthStatus::Unstable => 3,
        crate::health::HealthStatus::Degraded => 4,
        crate::health::HealthStatus::Dead => 5,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_metrics_config_default() {
        let config = MetricsConfig::default();
        assert!(config.enabled);
        assert!(config.include_health);
        assert!(config.include_telemetry);
        assert!(config.include_pack_info);
        assert!(!config.include_rule_metrics);
        assert_eq!(config.metric_prefix, "icg_");
    }

    #[test]
    fn test_export_basic_metrics() {
        let exporter = MetricsExporter::new(MetricsConfig::default());

        let snapshot = MetricsSnapshot {
            timestamp: Utc::now(),
            health: GuardMetrics {
                uptime_seconds: 3600.0,
                total_crashes: 2,
                recent_crashes: 0,
                crash_rate: 0.5,
                consecutive_clean_runs: 5,
                health_status: 1,
                is_stable: 1,
                time_since_stable_seconds: Some(300.0),
                last_crash_timestamp: Some(1640000000.0),
                last_start_timestamp: Some(1640003600.0),
            },
            telemetry: TelemetryMetrics {
                baseline_evaluations: 1000,
                baseline_denies: 10,
                baseline_deny_rate: 0.01,
                baseline_mean: 0.01,
                baseline_stddev: 0.005,
                baseline_min: 0.005,
                baseline_max: 0.02,
                baseline_window_start: Some(1640000000.0),
                baseline_window_end: Some(1640003600.0),
                current_deny_rate: 0.012,
                rollback_on_cooldown: 0,
                last_rollback_timestamp: None,
            },
            pack: Some(PackMetrics {
                pack_id: "test-pack".to_string(),
                pack_version: "v1.0.0".to_string(),
                total_patterns: 50,
                enabled_patterns: 45,
                disabled_patterns: 5,
                pack_created_at: Some(1640000000.0),
            }),
            rules: vec![],
        };

        let result = exporter.export_metrics(&snapshot);
        assert!(result.is_ok());

        let output = result.unwrap();
        assert!(output.contains("icg_uptime_seconds 3600"));
        assert!(output.contains("icg_total_crashes 2"));
        assert!(output.contains("icg_baseline_deny_rate 0.01"));
        assert!(output.contains("# TYPE icg_uptime_seconds gauge"));
        assert!(output.contains("# HELP icg_uptime_seconds"));
    }

    #[test]
    fn test_export_with_pack_info() {
        let exporter = MetricsExporter::new(MetricsConfig {
            include_pack_info: true,
            ..Default::default()
        });

        let snapshot = MetricsSnapshot {
            timestamp: Utc::now(),
            health: GuardMetrics {
                uptime_seconds: 0.0,
                total_crashes: 0,
                recent_crashes: 0,
                crash_rate: 0.0,
                consecutive_clean_runs: 0,
                health_status: 0,
                is_stable: 0,
                time_since_stable_seconds: None,
                last_crash_timestamp: None,
                last_start_timestamp: None,
            },
            telemetry: TelemetryMetrics {
                baseline_evaluations: 0,
                baseline_denies: 0,
                baseline_deny_rate: 0.0,
                baseline_mean: 0.0,
                baseline_stddev: 0.0,
                baseline_min: 0.0,
                baseline_max: 0.0,
                baseline_window_start: None,
                baseline_window_end: None,
                current_deny_rate: 0.0,
                rollback_on_cooldown: 0,
                last_rollback_timestamp: None,
            },
            pack: Some(PackMetrics {
                pack_id: "my-pack".to_string(),
                pack_version: "v2.0.0".to_string(),
                total_patterns: 100,
                enabled_patterns: 95,
                disabled_patterns: 5,
                pack_created_at: None,
            }),
            rules: vec![],
        };

        let output = exporter.export_metrics(&snapshot).unwrap();
        assert!(output.contains("icg_pack_info{pack_id=\"my-pack\",version=\"v2.0.0\"} 1"));
        assert!(output.contains("icg_total_patterns 100"));
        assert!(output.contains("icg_enabled_patterns 95"));
        assert!(output.contains("icg_disabled_patterns 5"));
    }

    #[test]
    fn test_export_with_rule_metrics() {
        let exporter = MetricsExporter::new(MetricsConfig {
            include_rule_metrics: true,
            ..Default::default()
        });

        let snapshot = MetricsSnapshot {
            timestamp: Utc::now(),
            health: GuardMetrics {
                uptime_seconds: 0.0,
                total_crashes: 0,
                recent_crashes: 0,
                crash_rate: 0.0,
                consecutive_clean_runs: 0,
                health_status: 0,
                is_stable: 0,
                time_since_stable_seconds: None,
                last_crash_timestamp: None,
                last_start_timestamp: None,
            },
            telemetry: TelemetryMetrics {
                baseline_evaluations: 0,
                baseline_denies: 0,
                baseline_deny_rate: 0.0,
                baseline_mean: 0.0,
                baseline_stddev: 0.0,
                baseline_min: 0.0,
                baseline_max: 0.0,
                baseline_window_start: None,
                baseline_window_end: None,
                current_deny_rate: 0.0,
                rollback_on_cooldown: 0,
                last_rollback_timestamp: None,
            },
            pack: None,
            rules: vec![
                RuleMetrics {
                    pattern_id: "rule-1".to_string(),
                    pack_id: "test-pack".to_string(),
                    match_count: 100,
                    deny_count: 5,
                    deny_rate: 0.05,
                    enabled: 1,
                },
                RuleMetrics {
                    pattern_id: "rule-2".to_string(),
                    pack_id: "test-pack".to_string(),
                    match_count: 200,
                    deny_count: 10,
                    deny_rate: 0.05,
                    enabled: 1,
                },
            ],
        };

        let output = exporter.export_metrics(&snapshot).unwrap();
        assert!(output
            .contains("icg_rule_match_count{pack_id=\"test-pack\",pattern_id=\"rule-1\"} 100"));
        assert!(
            output.contains("icg_rule_deny_count{pack_id=\"test-pack\",pattern_id=\"rule-2\"} 10")
        );
        assert!(
            output.contains("icg_rule_deny_rate{pack_id=\"test-pack\",pattern_id=\"rule-1\"} 0.05")
        );
        assert!(output.contains("icg_rule_enabled{pack_id=\"test-pack\",pattern_id=\"rule-2\"} 1"));
    }

    #[test]
    fn test_health_status_to_code() {
        use crate::health::HealthStatus;

        assert_eq!(health_status_to_code(HealthStatus::Unknown), 0);
        assert_eq!(health_status_to_code(HealthStatus::Healthy), 1);
        assert_eq!(health_status_to_code(HealthStatus::Recovering), 2);
        assert_eq!(health_status_to_code(HealthStatus::Unstable), 3);
        assert_eq!(health_status_to_code(HealthStatus::Degraded), 4);
        assert_eq!(health_status_to_code(HealthStatus::Dead), 5);
    }

    #[test]
    fn test_guard_metrics_from_persisted_health() {
        let mut state = crate::health::HealthState::new();
        state.mark_start();
        state.record_crash(crate::health::CrashRecord::new(
            crate::health::CrashType::Abort,
        ));
        let health = state.compute_metrics();
        let metrics = GuardMetrics::from_health_metrics(&health);

        assert_eq!(metrics.total_crashes, 1);
        assert_eq!(metrics.recent_crashes, 1);
        assert_eq!(metrics.health_status, health_status_to_code(health.status));
    }
}
