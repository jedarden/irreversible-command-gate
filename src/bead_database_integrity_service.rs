//! Automated bead database integrity verification service
//!
//! Continuous monitoring service that runs `bead doctor --rehearse` on a periodic
//! schedule and automatically applies repairs when the rehearsal succeeds.
//!
//! ## Architecture
//!
//! The service implements a safe, rehearsal-first approach:
//! 1. Runs `bead doctor --rehearse` against a temporary copy of the database
//! 2. Analyzes rehearsal output for data loss indicators
//! 3. If rehearsal succeeds without data loss, applies the same repairs to live database
//! 4. Logs all repairs performed to `.beads/diagnostics/auto-repair-log.jsonl`
//! 5. If rehearsal fails or shows data loss, alerts for human intervention
//!
//! This differs from the standard integrity monitor by using rehearsal mode
//! to validate repairs before applying them to the live database.

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::process::Command;
use std::time::Duration;
use tokio::time::interval;

/// Configuration for the bead database integrity service
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DatabaseIntegrityServiceConfig {
    /// Path to the workspace root (contains .beads directory)
    pub workspace_path: PathBuf,

    /// Interval between integrity checks (default: 10 minutes)
    #[serde(default = "default_check_interval")]
    pub check_interval: Duration,

    /// Enable auto-repair when rehearsal succeeds
    #[serde(default = "default_auto_repair_enabled")]
    pub auto_repair_enabled: bool,

    /// Alert on any data loss detected during rehearsal
    #[serde(default = "default_alert_on_data_loss")]
    pub alert_on_data_loss: bool,
}

fn default_check_interval() -> Duration {
    Duration::from_secs(600) // 10 minutes
}

fn default_auto_repair_enabled() -> bool {
    true
}

fn default_alert_on_data_loss() -> bool {
    true
}

impl Default for DatabaseIntegrityServiceConfig {
    fn default() -> Self {
        Self {
            workspace_path: PathBuf::from("."),
            check_interval: default_check_interval(),
            auto_repair_enabled: default_auto_repair_enabled(),
            alert_on_data_loss: default_alert_on_data_loss(),
        }
    }
}

impl DatabaseIntegrityServiceConfig {
    /// Load configuration from environment variables
    pub fn from_environment() -> Self {
        let mut config = Self::default();

        if let Ok(path) = std::env::var("ICG_WORKSPACE_PATH") {
            config.workspace_path = PathBuf::from(path);
        }

        if let Ok(seconds) = std::env::var("ICG_INTEGRITY_CHECK_INTERVAL_SECONDS") {
            if let Ok(seconds) = seconds.parse::<u64>() {
                config.check_interval = Duration::from_secs(seconds.max(60));
            }
        }

        if let Ok(enabled) = std::env::var("ICG_AUTO_REPAIR_ENABLED") {
            config.auto_repair_enabled = enabled.eq_ignore_ascii_case("true") || enabled == "1";
        }

        if let Ok(alert) = std::env::var("ICG_ALERT_ON_DATA_LOSS") {
            config.alert_on_data_loss = alert.eq_ignore_ascii_case("true") || alert == "1";
        }

        config
    }

    /// Get the path to the beads database
    pub fn beads_db_path(&self) -> PathBuf {
        self.workspace_path.join(".beads").join("beads.db")
    }

    /// Get the path to the diagnostics directory
    pub fn diagnostics_dir(&self) -> PathBuf {
        self.workspace_path.join(".beads").join("diagnostics")
    }

    /// Get the path to the auto-repair log file
    pub fn auto_repair_log_path(&self) -> PathBuf {
        self.diagnostics_dir().join("auto-repair-log.jsonl")
    }
}

/// Rehearsal check result
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RehearsalResult {
    /// Timestamp when the rehearsal was performed
    pub timestamp: DateTime<Utc>,

    /// Whether the rehearsal succeeded
    pub success: bool,

    /// Whether data loss was detected
    pub data_loss_detected: bool,

    /// Number of issues that would be repaired
    pub issues_found: usize,

    /// Rehearsal output
    pub output: String,

    /// Rehearsal error (if any)
    pub error: Option<String>,
}

/// Repair operation result
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RepairOperation {
    /// Timestamp when the repair was performed
    pub timestamp: DateTime<Utc>,

    /// Whether the repair was successful
    pub success: bool,

    /// Number of issues repaired
    pub issues_repaired: usize,

    /// Repair output
    pub output: String,

    /// Repair error (if any)
    pub error: Option<String>,
}

/// Integrity check cycle report
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IntegrityCycleReport {
    /// Timestamp when the cycle started
    pub cycle_start: DateTime<Utc>,

    /// Timestamp when the cycle completed
    pub cycle_end: DateTime<Utc>,

    /// Cycle duration in seconds
    pub duration_seconds: f64,

    /// Rehearsal result
    pub rehearsal: RehearsalResult,

    /// Whether repair was attempted
    pub repair_attempted: bool,

    /// Repair result (if attempted)
    pub repair: Option<RepairOperation>,

    /// Whether human alert was triggered
    pub alert_triggered: bool,

    /// Alert reason (if triggered)
    pub alert_reason: Option<String>,
}

/// Bead database integrity service
pub struct DatabaseIntegrityService {
    config: DatabaseIntegrityServiceConfig,
}

impl DatabaseIntegrityService {
    /// Create a new database integrity service with the given configuration
    pub fn new(config: DatabaseIntegrityServiceConfig) -> Self {
        Self { config }
    }

    /// Get the configuration
    pub fn config(&self) -> &DatabaseIntegrityServiceConfig {
        &self.config
    }

    /// Run a single integrity check cycle
    pub fn run_cycle(&mut self) -> Result<IntegrityCycleReport> {
        let cycle_start = Utc::now();

        eprintln!("🔍 Starting database integrity check cycle");
        eprintln!("📁 Workspace: {}", self.config.workspace_path.display());

        // Ensure diagnostics directory exists
        std::fs::create_dir_all(self.config.diagnostics_dir())
            .context("Failed to create diagnostics directory")?;

        // Run rehearsal
        let rehearsal = self.run_rehearsal()?;

        eprintln!("📋 Rehearsal completed: success={}, data_loss={}, issues_found={}",
            rehearsal.success, rehearsal.data_loss_detected, rehearsal.issues_found);

        // Determine if repair should be attempted
        let repair_attempted = self.config.auto_repair_enabled
            && rehearsal.success
            && !rehearsal.data_loss_detected
            && rehearsal.issues_found > 0;

        let repair = if repair_attempted {
            Some(self.run_repair()?)
        } else {
            None
        };

        // Determine if alert should be triggered
        let (alert_triggered, alert_reason) = if self.config.alert_on_data_loss && rehearsal.data_loss_detected {
            (true, Some("Data loss detected during rehearsal - requires human intervention".to_string()))
        } else if !rehearsal.success {
            (true, Some("Rehearsal failed - requires human intervention".to_string()))
        } else {
            (false, None)
        };

        let cycle_end = Utc::now();
        let duration = cycle_end.signed_duration_since(cycle_start);

        let report = IntegrityCycleReport {
            cycle_start,
            cycle_end,
            duration_seconds: duration.num_seconds() as f64 + duration.num_milliseconds() as f64 / 1000.0,
            rehearsal,
            repair_attempted,
            repair,
            alert_triggered,
            alert_reason,
        };

        // Publish report to log file
        self.publish_report(&report)?;

        // Log status
        eprintln!("✅ Integrity check cycle completed in {:.2}s", report.duration_seconds);
        if report.alert_triggered {
            if let Some(ref reason) = report.alert_reason {
                eprintln!("🚨 ALERT TRIGGERED: {}", reason);
            }
        } else if report.repair_attempted {
            if let Some(ref repair) = report.repair {
                eprintln!("🔧 Auto-repair: success={}, issues_repaired={}",
                    repair.success, repair.issues_repaired);
            }
        } else {
            eprintln!("✅ No repairs needed");
        }

        Ok(report)
    }

    /// Run bead doctor --rehearse and parse the output
    fn run_rehearsal(&self) -> Result<RehearsalResult> {
        let timestamp = Utc::now();

        eprintln!("🧪 Running bead doctor --rehearse");

        let output = Command::new("bead")
            .args(["doctor", "--rehearse"])
            .current_dir(&self.config.workspace_path)
            .output()
            .context("Failed to run bead doctor --rehearse")?;

        let success = output.status.success();
        let stdout = String::from_utf8_lossy(&output.stdout).to_string();
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();

        // Check for data loss indicators in output
        let data_loss_detected = self.check_data_loss_indicators(&stdout, &stderr);

        // Count issues found from output
        let issues_found = self.count_issues_from_output(&stdout);

        let error = if !success {
            Some(format!("Exit code: {:?}, Error: {}", output.status.code(), stderr))
        } else {
            None
        };

        let combined_output = if success { stdout } else { stderr };

        Ok(RehearsalResult {
            timestamp,
            success,
            data_loss_detected,
            issues_found,
            output: combined_output,
            error,
        })
    }

    /// Check for data loss indicators in rehearsal output
    fn check_data_loss_indicators(&self, stdout: &str, stderr: &str) -> bool {
        // Common indicators of data loss
        let data_loss_patterns = [
            "data loss",
            "bead loss",
            "checkpoint loss",
            "corruption detected",
            "unrecoverable",
            "cannot repair",
            "missing bead",
            "orphaned",
        ];

        let combined = format!("{} {}", stdout, stderr).to_lowercase();

        for pattern in &data_loss_patterns {
            if combined.contains(pattern) {
                eprintln!("⚠️  Data loss indicator found: '{}'", pattern);
                return true;
            }
        }

        false
    }

    /// Count issues from rehearsal output
    fn count_issues_from_output(&self, output: &str) -> usize {
        // Look for issue indicators
        let lines: Vec<&str> = output.lines().collect();
        let mut count = 0;

        for line in lines {
            if line.contains("issue") || line.contains("error") || line.contains("warning") {
                count += 1;
            }
        }

        // Also look for specific bead doctor output patterns
        if output.contains("stale_in_progress") {
            count += 1;
        }
        if output.contains("missing_index") {
            count += 1;
        }
        if output.contains("schema_drift") {
            count += 1;
        }

        count
    }

    /// Run bead doctor --repair on the live database
    fn run_repair(&self) -> Result<RepairOperation> {
        let timestamp = Utc::now();

        eprintln!("🔧 Running bead doctor --repair on live database");

        let output = Command::new("bead")
            .args(["doctor", "--repair"])
            .current_dir(&self.config.workspace_path)
            .output()
            .context("Failed to run bead doctor --repair")?;

        let success = output.status.success();
        let stdout = String::from_utf8_lossy(&output.stdout).to_string();
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();

        // Count repaired issues from output
        let issues_repaired = self.count_repaired_issues(&stdout);

        let error = if !success {
            Some(format!("Exit code: {:?}, Error: {}", output.status.code(), stderr))
        } else {
            None
        };

        let combined_output = if success { stdout } else { stderr };

        Ok(RepairOperation {
            timestamp,
            success,
            issues_repaired,
            output: combined_output,
            error,
        })
    }

    /// Count the number of repaired issues from repair output
    fn count_repaired_issues(&self, output: &str) -> usize {
        // Look for patterns like "Repaired", "Fixed", "Cleaned"
        let mut count = 0;

        for line in output.lines() {
            if line.contains("Repaired") || line.contains("Fixed") || line.contains("Cleaned") {
                count += 1;
            }
        }

        count
    }

    /// Publish the integrity cycle report to the JSONL file
    fn publish_report(&self, report: &IntegrityCycleReport) -> Result<()> {
        let report_path = self.config.auto_repair_log_path();
        let json_line = serde_json::to_string(report)
            .context("Failed to serialize integrity cycle report")?;

        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&report_path)
            .context("Failed to open auto-repair log file")?;

        use std::io::Write;
        writeln!(file, "{}", json_line)
            .context("Failed to write integrity cycle report")?;

        eprintln!(
            "📝 Integrity cycle report logged to {}",
            report_path.display()
        );

        Ok(())
    }

    /// Run a single check and return (synchronous)
    pub fn run_once(&mut self) -> Result<IntegrityCycleReport> {
        self.run_cycle()
    }

    /// Start the continuous monitoring loop
    pub async fn run(&mut self) -> Result<()> {
        eprintln!("🩺 Bead database integrity service starting");
        eprintln!("📁 Workspace: {}", self.config.workspace_path.display());
        eprintln!("⏱️  Check interval: {} seconds", self.config.check_interval.as_secs());
        eprintln!("🔧 Auto-repair: {}", self.config.auto_repair_enabled);
        eprintln!("🚨 Alert on data loss: {}", self.config.alert_on_data_loss);

        // Run initial check
        self.run_once()?;

        let mut timer = interval(self.config.check_interval);
        timer.tick().await; // Skip the immediate tick

        loop {
            timer.tick().await;

            match self.run_once() {
                Ok(report) => {
                    eprintln!("✅ Integrity check completed: duration={:.2}s, issues_found={}, repair_attempted={}",
                        report.duration_seconds,
                        report.rehearsal.issues_found,
                        report.repair_attempted);

                    if report.alert_triggered {
                        if let Some(ref reason) = report.alert_reason {
                            eprintln!("🚨 ALERT: {}", reason);
                        }
                    }
                }
                Err(e) => {
                    eprintln!("❌ Integrity check failed: {:#}", e);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_default_config() {
        let config = DatabaseIntegrityServiceConfig::default();
        assert_eq!(config.check_interval, Duration::from_secs(600));
        assert!(config.auto_repair_enabled);
        assert!(config.alert_on_data_loss);
    }

    #[test]
    fn test_config_from_environment() {
        std::env::set_var("ICG_INTEGRITY_CHECK_INTERVAL_SECONDS", "900");
        std::env::set_var("ICG_AUTO_REPAIR_ENABLED", "false");
        std::env::set_var("ICG_ALERT_ON_DATA_LOSS", "false");

        let config = DatabaseIntegrityServiceConfig::from_environment();
        assert_eq!(config.check_interval, Duration::from_secs(900));
        assert!(!config.auto_repair_enabled);
        assert!(!config.alert_on_data_loss);

        std::env::remove_var("ICG_INTEGRITY_CHECK_INTERVAL_SECONDS");
        std::env::remove_var("ICG_AUTO_REPAIR_ENABLED");
        std::env::remove_var("ICG_ALERT_ON_DATA_LOSS");
    }

    #[test]
    fn test_data_loss_detection() {
        let dir = tempdir().unwrap();
        let config = DatabaseIntegrityServiceConfig {
            workspace_path: dir.path().to_path_buf(),
            ..Default::default()
        };
        let service = DatabaseIntegrityService::new(config);

        // Test data loss patterns
        assert!(service.check_data_loss_indicators("data loss detected", ""));
        assert!(service.check_data_loss_indicators("bead loss detected", ""));
        assert!(service.check_data_loss_indicators("unrecoverable error", ""));
        assert!(!service.check_data_loss_indicators("all systems operational", ""));
    }

    #[test]
    fn test_issue_counting() {
        let dir = tempdir().unwrap();
        let config = DatabaseIntegrityServiceConfig {
            workspace_path: dir.path().to_path_buf(),
            ..Default::default()
        };
        let service = DatabaseIntegrityService::new(config);

        // Test issue counting
        let output = r#"
ERROR: stale_in_progress detected
WARNING: missing_index on beads table
INFO: schema_drift detected
"#;
        assert_eq!(service.count_issues_from_output(output), 3);
    }

    #[test]
    fn test_repair_counting() {
        let dir = tempdir().unwrap();
        let config = DatabaseIntegrityServiceConfig {
            workspace_path: dir.path().to_path_buf(),
            ..Default::default()
        };
        let service = DatabaseIntegrityService::new(config);

        // Test repair counting
        let output = r#"
Repaired stale_in_progress issues
Fixed missing_index on beads table
Cleaned up orphaned records
"#;
        assert_eq!(service.count_repaired_issues(output), 3);
    }

    #[test]
    fn test_report_serialization() {
        let report = IntegrityCycleReport {
            cycle_start: Utc::now(),
            cycle_end: Utc::now(),
            duration_seconds: 1.5,
            rehearsal: RehearsalResult {
                timestamp: Utc::now(),
                success: true,
                data_loss_detected: false,
                issues_found: 2,
                output: "Test output".to_string(),
                error: None,
            },
            repair_attempted: true,
            repair: Some(RepairOperation {
                timestamp: Utc::now(),
                success: true,
                issues_repaired: 2,
                output: "Repair output".to_string(),
                error: None,
            }),
            alert_triggered: false,
            alert_reason: None,
        };

        let json = serde_json::to_string(&report).unwrap();
        assert!(json.contains("\"success\":true"));
        assert!(json.contains("\"issues_found\":2"));
        assert!(json.contains("\"issues_repaired\":2"));
    }
}
