//! Automated Checkpoint Health Monitoring and Repair System
//!
//! This module implements continuous monitoring and automatic repair of the
//! bead checkpoint system, preventing invisible beads caused by sync failures
//! between beads.db and the durable checkpoint.
//!
//! ## Problem Statement
//!
//! Bead starvation occurs when `bead list --ready` returns zero candidates despite
//! open beads existing in the database. A common root cause is checkpoint sync
//! failure where the database state diverges from the durable checkpoint.
//!
//! ## Solution
//!
//! This system provides:
//! 1. **Periodic checkpoint sync verification** - Compares beads.db current state
//!    with .beads/checkpoint/current.json
//! 2. **Automatic stale checkpoint repair** - Runs `bead sync flush-only` when
//!    checkpoint timestamp exceeds threshold
//! 3. **Automatic checkpoint rebuild** - Rebuilds from forensic.jsonl using
//!    `bead sync import-only --restore-into-empty` when checkpoint is corrupted/missing
//! 4. **Automatic database rebuild** - Rebuilds beads.db from checkpoint when
//!    database is corrupted
//!
//! ## Usage
//!
//! ```bash
//! # Run the checkpoint monitor (default: 5-minute intervals)
//! cargo run --bin checkpoint-monitor
//!
//! # With custom configuration
//! cargo run --bin checkpoint-monitor -- --interval-secs 600 --stale-threshold-minutes 10
//! ```
//!
//! ## Architecture
//!
//! The monitor runs in a continuous loop:
//! 1. Check checkpoint sync status by comparing database and checkpoint states
//! 2. Detect stale checkpoints (timestamp comparison)
//! 3. Detect corrupted/missing checkpoints
//! 4. Detect corrupted databases
//! 5. Execute appropriate repair actions automatically
//! 6. Publish diagnostic reports to .beads/diagnostics/checkpoint-report.jsonl
//! 7. Export Prometheus metrics for monitoring
//!
//! ## Repair Actions
//!
//! All repair actions are deterministic and require no human intervention:
//! - **Stale checkpoint**: Run `bead sync flush-only` to bring checkpoint up-to-date
//! - **Corrupted/missing checkpoint**: Rebuild from forensic.jsonl
//! - **Corrupted database**: Rebuild from checkpoint using standard recovery
//!
//! ## Safety
//!
//! - All repairs are idempotent and safe to run multiple times
//! - Checkpoint rebuild uses forensic.jsonl which is the authoritative source
//! - Database rebuild uses checkpoint which is the git-tracked durable state
//! - Repairs are logged to events.jsonl for audit trail

use anyhow::{anyhow, Context, Result};
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration as StdDuration;
use tokio::time::interval;

/// Configuration for checkpoint monitoring
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CheckpointMonitorConfig {
    /// Path to the workspace root (contains .beads directory)
    pub workspace_path: PathBuf,

    /// Interval between checkpoint health checks (default: 5 minutes)
    #[serde(default = "default_check_interval")]
    pub check_interval: StdDuration,

    /// Threshold for considering checkpoint stale (default: 5 minutes)
    #[serde(default = "default_stale_threshold")]
    pub stale_threshold_minutes: i64,

    /// Enable automatic repair when issues are detected
    #[serde(default = "default_auto_repair_enabled")]
    pub auto_repair_enabled: bool,

    /// Path to the bead CLI (default: "bead")
    #[serde(default = "default_bead_path")]
    pub bead_path: PathBuf,
}

fn default_check_interval() -> StdDuration {
    StdDuration::from_secs(300) // 5 minutes
}

fn default_stale_threshold() -> i64 {
    5 // 5 minutes
}

fn default_auto_repair_enabled() -> bool {
    true
}

fn default_bead_path() -> PathBuf {
    PathBuf::from("bead")
}

impl Default for CheckpointMonitorConfig {
    fn default() -> Self {
        Self {
            workspace_path: PathBuf::from("."),
            check_interval: default_check_interval(),
            stale_threshold_minutes: default_stale_threshold(),
            auto_repair_enabled: default_auto_repair_enabled(),
            bead_path: default_bead_path(),
        }
    }
}

impl CheckpointMonitorConfig {
    /// Load configuration from environment variables
    pub fn from_environment() -> Self {
        let mut config = Self::default();

        if let Ok(path) = std::env::var("ICG_WORKSPACE_PATH") {
            config.workspace_path = PathBuf::from(path);
        }

        if let Ok(secs) = std::env::var("ICG_CHECK_INTERVAL_SECONDS") {
            if let Ok(secs) = secs.parse::<u64>() {
                config.check_interval = StdDuration::from_secs(secs.max(60));
            }
        }

        if let Ok(mins) = std::env::var("ICG_STALE_THRESHOLD_MINUTES") {
            if let Ok(mins) = mins.parse::<i64>() {
                config.stale_threshold_minutes = mins.max(1);
            }
        }

        if let Ok(enabled) = std::env::var("ICG_AUTO_REPAIR_ENABLED") {
            config.auto_repair_enabled = enabled.eq_ignore_ascii_case("true") || enabled == "1";
        }

        if let Ok(path) = std::env::var("BEAD_PATH") {
            config.bead_path = PathBuf::from(path);
        }

        config
    }

    /// Get the path to the beads database
    pub fn beads_db_path(&self) -> PathBuf {
        self.workspace_path.join(".beads").join("beads.db")
    }

    /// Get the path to the checkpoint directory
    pub fn checkpoint_dir(&self) -> PathBuf {
        self.workspace_path.join(".beads").join("checkpoint")
    }

    /// Get the path to the current checkpoint file
    pub fn current_checkpoint_path(&self) -> PathBuf {
        self.checkpoint_dir().join("current.json")
    }

    /// Get the path to the forensic JSONL file
    pub fn forensic_path(&self) -> PathBuf {
        self.checkpoint_dir().join("forensic.jsonl")
    }

    /// Get the path to the diagnostics directory
    pub fn diagnostics_dir(&self) -> PathBuf {
        self.workspace_path.join(".beads").join("diagnostics")
    }

    /// Get the path to the checkpoint report JSONL file
    pub fn checkpoint_report_path(&self) -> PathBuf {
        self.diagnostics_dir().join("checkpoint-report.jsonl")
    }

    /// Get the path to events.jsonl for logging repairs
    pub fn events_path(&self) -> PathBuf {
        self.workspace_path.join(".beads").join("events.jsonl")
    }
}

/// Checkpoint sync status
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CheckpointSyncStatus {
    /// Whether checkpoint file exists
    pub checkpoint_exists: bool,

    /// Timestamp from checkpoint current.json
    pub checkpoint_timestamp: Option<DateTime<Utc>>,

    /// Number of issues in checkpoint
    pub checkpoint_issue_count: Option<i64>,

    /// Whether database file exists
    pub database_exists: bool,

    /// Number of issues in database
    pub database_issue_count: i64,

    /// Sync status: synchronized, stale, missing, corrupted, desynchronized
    pub sync_status: String,

    /// Whether checkpoint is considered stale
    pub stale: bool,

    /// How many minutes stale (if stale)
    pub stale_minutes: Option<i64>,

    /// Whether checkpoint is corrupted
    pub corrupted: bool,

    /// Corruption details
    pub corruption_details: Option<String>,
}

/// Database health status
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DatabaseHealthStatus {
    /// Whether database file exists
    pub exists: bool,

    /// Whether database is readable
    pub readable: bool,

    /// Whether schema is valid
    pub schema_valid: bool,

    /// Whether corruption was detected
    pub corrupted: bool,

    /// Error details if issues found
    pub error_details: Option<String>,
}

/// Repair action that was performed
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RepairAction {
    /// Timestamp when repair was performed
    pub timestamp: DateTime<Utc>,

    /// Type of repair performed
    pub repair_type: String,

    /// Whether repair was successful
    pub success: bool,

    /// Repair output or error message
    pub message: String,

    /// Duration of repair in seconds
    pub duration_seconds: f64,
}

/// Checkpoint monitoring report
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CheckpointMonitorReport {
    /// Timestamp when the check was performed
    pub timestamp: DateTime<Utc>,

    /// Check interval in seconds
    pub check_interval_seconds: u64,

    /// Overall health status
    pub health_status: String,

    /// Checkpoint sync status
    pub checkpoint_sync: CheckpointSyncStatus,

    /// Database health status
    pub database_health: DatabaseHealthStatus,

    /// Whether repair was triggered
    pub repair_triggered: bool,

    /// Repair actions performed
    pub repairs_performed: Vec<RepairAction>,

    /// Recommended actions (if auto-repair is disabled)
    pub recommended_actions: Vec<String>,
}

/// Checkpoint health monitor
pub struct CheckpointMonitor {
    config: CheckpointMonitorConfig,
    start_time: DateTime<Utc>,
}

impl CheckpointMonitor {
    /// Create a new checkpoint monitor with default configuration
    pub fn new() -> Result<Self> {
        Self::with_config(CheckpointMonitorConfig::default())
    }

    /// Create a new checkpoint monitor with custom configuration
    pub fn with_config(config: CheckpointMonitorConfig) -> Result<Self> {
        // Verify workspace path exists
        if !config.workspace_path.exists() {
            return Err(anyhow!(
                "Workspace path does not exist: {}",
                config.workspace_path.display()
            ));
        }

        Ok(Self {
            config,
            start_time: Utc::now(),
        })
    }

    /// Get the configuration
    pub fn config(&self) -> &CheckpointMonitorConfig {
        &self.config
    }

    /// Run a single checkpoint health check
    pub fn run_check(&mut self) -> Result<CheckpointMonitorReport> {
        let timestamp = Utc::now();

        // Ensure diagnostics directory exists
        fs::create_dir_all(self.config.diagnostics_dir())
            .context("Failed to create diagnostics directory")?;

        // Check checkpoint sync status
        let checkpoint_sync = self.check_checkpoint_sync()?;

        // Check database health
        let database_health = self.check_database_health()?;

        // Determine overall health status
        let health_status = if database_health.corrupted {
            "critical".to_string()
        } else if checkpoint_sync.corrupted {
            "critical".to_string()
        } else if !checkpoint_sync.checkpoint_exists {
            "warning".to_string()
        } else if checkpoint_sync.stale {
            "degraded".to_string()
        } else {
            "healthy".to_string()
        };

        // Determine if repair is needed
        let repair_needed = self.config.auto_repair_enabled && (
            database_health.corrupted ||
            checkpoint_sync.corrupted ||
            !checkpoint_sync.checkpoint_exists ||
            checkpoint_sync.stale
        );

        // Perform repairs if needed
        let (repairs_performed, recommended_actions) = if repair_needed {
            let repairs = self.perform_repairs(&checkpoint_sync, &database_health)?;
            (repairs, Vec::new())
        } else {
            // Generate recommendations if auto-repair is disabled
            let recommendations = self.generate_recommendations(&checkpoint_sync, &database_health);
            (Vec::new(), recommendations)
        };

        let report = CheckpointMonitorReport {
            timestamp,
            check_interval_seconds: self.config.check_interval.as_secs(),
            health_status,
            checkpoint_sync,
            database_health,
            repair_triggered: !repairs_performed.is_empty(),
            repairs_performed,
            recommended_actions,
        };

        // Publish report to JSONL file
        self.publish_report(&report)?;

        Ok(report)
    }

    /// Check checkpoint sync status by comparing database and checkpoint
    fn check_checkpoint_sync(&self) -> Result<CheckpointSyncStatus> {
        let checkpoint_path = self.config.current_checkpoint_path();
        let forensic_path = self.config.forensic_path();
        let db_path = self.config.beads_db_path();

        let checkpoint_exists = checkpoint_path.exists();
        let forensic_exists = forensic_path.exists();
        let database_exists = db_path.exists();

        let mut checkpoint_timestamp = None;
        let mut checkpoint_issue_count = None;
        let mut corrupted = false;
        let mut corruption_details = None;
        let mut stale = false;
        let mut stale_minutes = None;

        // Try to read checkpoint current.json
        if checkpoint_exists {
            match fs::read_to_string(&checkpoint_path) {
                Ok(content) => {
                    match serde_json::from_str::<serde_json::Value>(&content) {
                        Ok(json) => {
                            // Extract timestamp
                            if let Some(ts) = json.get("created_at").and_then(|v| v.as_str()) {
                                match DateTime::parse_from_rfc3339(ts) {
                                    Ok(dt) => {
                                        checkpoint_timestamp = Some(dt.with_timezone(&Utc));
                                        // Check if stale
                                        let age = Utc::now().signed_duration_since(dt.with_timezone(&Utc));
                                        stale_minutes = Some(age.num_minutes());
                                        stale = age.num_minutes() > self.config.stale_threshold_minutes;
                                    }
                                    Err(e) => {
                                        corrupted = true;
                                        corruption_details = Some(format!("Invalid timestamp format: {}", e));
                                    }
                                }
                            }

                            // Extract issue count
                            if let Some(count) = json.get("issue_count").and_then(|v| v.as_i64()) {
                                checkpoint_issue_count = Some(count);
                            }
                        }
                        Err(e) => {
                            corrupted = true;
                            corruption_details = Some(format!("Invalid JSON: {}", e));
                        }
                    }
                }
                Err(e) => {
                    corrupted = true;
                    corruption_details = Some(format!("Read error: {}", e));
                }
            }
        }

        // Count database issues
        let database_issue_count = if database_exists {
            match self.count_database_issues() {
                Ok(count) => count,
                Err(e) => {
                    corrupted = true;
                    corruption_details = Some(format!("Database query failed: {}", e));
                    0
                }
            }
        } else {
            0
        };

        // Determine sync status
        let sync_status = if !checkpoint_exists {
            "missing".to_string()
        } else if corrupted {
            "corrupted".to_string()
        } else if checkpoint_timestamp.is_none() {
            "invalid".to_string()
        } else if stale {
            "stale".to_string()
        } else if checkpoint_issue_count != Some(database_issue_count) {
            "desynchronized".to_string()
        } else {
            "synchronized".to_string()
        };

        Ok(CheckpointSyncStatus {
            checkpoint_exists,
            checkpoint_timestamp,
            checkpoint_issue_count,
            database_exists,
            database_issue_count,
            sync_status,
            stale,
            stale_minutes,
            corrupted,
            corruption_details,
        })
    }

    /// Check database health
    fn check_database_health(&self) -> Result<DatabaseHealthStatus> {
        let db_path = self.config.beads_db_path();

        let exists = db_path.exists();

        if !exists {
            return Ok(DatabaseHealthStatus {
                exists: false,
                readable: false,
                schema_valid: false,
                corrupted: true,
                error_details: Some("Database file does not exist".to_string()),
            });
        }

        // Try to read database
        let (readable, schema_valid, corrupted, error_details) = match self.check_database_readable() {
            Ok(()) => (true, true, false, None),
            Err(e) => {
                let error_msg = format!("Database check failed: {}", e);
                (false, false, true, Some(error_msg))
            }
        };

        Ok(DatabaseHealthStatus {
            exists: true,
            readable,
            schema_valid,
            corrupted,
            error_details,
        })
    }

    /// Check if database is readable and has valid schema
    fn check_database_readable(&self) -> Result<()> {
        let output = Command::new(&self.config.bead_path)
            .args(["list", "--json"])
            .current_dir(&self.config.workspace_path)
            .output()
            .context("Failed to run bead list")?;

        if !output.status.success() {
            anyhow::bail!(
                "bead list failed with exit code: {:?}, stderr: {}",
                output.status.code(),
                String::from_utf8_lossy(&output.stderr)
            );
        }

        Ok(())
    }

    /// Count issues in the database
    fn count_database_issues(&self) -> Result<i64> {
        let output = Command::new(&self.config.bead_path)
            .args(["list", "--json"])
            .current_dir(&self.config.workspace_path)
            .output()
            .context("Failed to run bead list")?;

        if !output.status.success() {
            anyhow::bail!("bead list failed");
        }

        // Count JSONL lines
        let json = String::from_utf8(output.stdout)?;
        Ok(json.lines().filter(|line| !line.trim().is_empty()).count() as i64)
    }

    /// Perform automatic repairs based on detected issues
    fn perform_repairs(
        &self,
        checkpoint_sync: &CheckpointSyncStatus,
        database_health: &DatabaseHealthStatus,
    ) -> Result<Vec<RepairAction>> {
        let mut repairs = Vec::new();

        // Priority 1: Repair corrupted database
        if database_health.corrupted && database_health.exists {
            eprintln!("🔧 Database corrupted - attempting repair from checkpoint");
            match self.repair_database_from_checkpoint() {
                Ok(repair) => {
                    eprintln!("✅ Database repaired successfully");
                    repairs.push(repair);
                }
                Err(e) => {
                    eprintln!("❌ Database repair failed: {}", e);
                    repairs.push(RepairAction {
                        timestamp: Utc::now(),
                        repair_type: "database_repair".to_string(),
                        success: false,
                        message: format!("Database repair failed: {}", e),
                        duration_seconds: 0.0,
                    });
                }
            }
        }

        // Priority 2: Repair corrupted or missing checkpoint
        if checkpoint_sync.corrupted || !checkpoint_sync.checkpoint_exists {
            eprintln!("🔧 Checkpoint corrupted or missing - rebuilding from forensic.jsonl");
            match self.repair_checkpoint_from_forensic() {
                Ok(repair) => {
                    eprintln!("✅ Checkpoint repaired successfully");
                    repairs.push(repair);
                }
                Err(e) => {
                    eprintln!("❌ Checkpoint repair failed: {}", e);
                    repairs.push(RepairAction {
                        timestamp: Utc::now(),
                        repair_type: "checkpoint_repair".to_string(),
                        success: false,
                        message: format!("Checkpoint repair failed: {}", e),
                        duration_seconds: 0.0,
                    });
                }
            }
        }

        // Priority 3: Flush stale checkpoint
        if checkpoint_sync.stale && !checkpoint_sync.corrupted {
            eprintln!("🔧 Checkpoint stale by {} minutes - flushing",
                checkpoint_sync.stale_minutes.unwrap_or(0));
            match self.repair_stale_checkpoint() {
                Ok(repair) => {
                    eprintln!("✅ Checkpoint flushed successfully");
                    repairs.push(repair);
                }
                Err(e) => {
                    eprintln!("❌ Checkpoint flush failed: {}", e);
                    repairs.push(RepairAction {
                        timestamp: Utc::now(),
                        repair_type: "checkpoint_flush".to_string(),
                        success: false,
                        message: format!("Checkpoint flush failed: {}", e),
                        duration_seconds: 0.0,
                    });
                }
            }
        }

        Ok(repairs)
    }

    /// Repair database by rebuilding from checkpoint
    fn repair_database_from_checkpoint(&self) -> Result<RepairAction> {
        let start_time = Utc::now();

        // Use bead doctor to repair database
        let output = Command::new(&self.config.bead_path)
            .args(["doctor", "--repair", "--json"])
            .current_dir(&self.config.workspace_path)
            .output()
            .context("Failed to run bead doctor --repair")?;

        let success = output.status.success();
        let message = if success {
            let stdout = String::from_utf8_lossy(&output.stdout);
            format!("Database repaired: {}", stdout.trim())
        } else {
            let stderr = String::from_utf8_lossy(&output.stderr);
            format!("Database repair failed: {}", stderr.trim())
        };

        let duration = Utc::now().signed_duration_since(start_time);

        Ok(RepairAction {
            timestamp: start_time,
            repair_type: "database_repair".to_string(),
            success,
            message,
            duration_seconds: duration.num_milliseconds() as f64 / 1000.0,
        })
    }

    /// Repair checkpoint by rebuilding from forensic.jsonl
    fn repair_checkpoint_from_forensic(&self) -> Result<RepairAction> {
        let start_time = Utc::now();

        // Rebuild checkpoint from forensic.jsonl
        let output = Command::new(&self.config.bead_path)
            .args([
                "sync",
                "import-only",
                "--restore-into-empty",
                "--input",
                self.config.forensic_path().to_str().unwrap(),
                "--actor",
                "checkpoint-monitor",
            ])
            .current_dir(&self.config.workspace_path)
            .output()
            .context("Failed to run bead sync import-only")?;

        let success = output.status.success();
        let message = if success {
            let stdout = String::from_utf8_lossy(&output.stdout);
            format!("Checkpoint rebuilt from forensic.jsonl: {}", stdout.trim())
        } else {
            let stderr = String::from_utf8_lossy(&output.stderr);
            format!("Checkpoint rebuild failed: {}", stderr.trim())
        };

        // Log the repair to events.jsonl
        if success {
            self.log_repair_event("checkpoint_rebuild", &message)?;
        }

        let duration = Utc::now().signed_duration_since(start_time);

        Ok(RepairAction {
            timestamp: start_time,
            repair_type: "checkpoint_rebuild".to_string(),
            success,
            message,
            duration_seconds: duration.num_milliseconds() as f64 / 1000.0,
        })
    }

    /// Repair stale checkpoint by flushing database state
    fn repair_stale_checkpoint(&self) -> Result<RepairAction> {
        let start_time = Utc::now();

        // Flush checkpoint from current database state
        let output = Command::new(&self.config.bead_path)
            .args(["sync", "flush-only"])
            .current_dir(&self.config.workspace_path)
            .output()
            .context("Failed to run bead sync flush-only")?;

        let success = output.status.success();
        let message = if success {
            let stdout = String::from_utf8_lossy(&output.stdout);
            format!("Checkpoint flushed: {}", stdout.trim())
        } else {
            let stderr = String::from_utf8_lossy(&output.stderr);
            format!("Checkpoint flush failed: {}", stderr.trim())
        };

        // Log the repair to events.jsonl
        if success {
            self.log_repair_event("checkpoint_flush", &message)?;
        }

        let duration = Utc::now().signed_duration_since(start_time);

        Ok(RepairAction {
            timestamp: start_time,
            repair_type: "checkpoint_flush".to_string(),
            success,
            message,
            duration_seconds: duration.num_milliseconds() as f64 / 1000.0,
        })
    }

    /// Log a repair event to events.jsonl
    fn log_repair_event(&self, event_type: &str, message: &str) -> Result<()> {
        let event = serde_json::json!({
            "issue_id": "checkpoint-auto-repair",
            "kind": event_type,
            "actor": "icg-checkpoint-monitor",
            "time": Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
            "detail": {
                "message": message,
            }
        });

        let mut file = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(self.config.events_path())
            .context("Failed to open events.jsonl for writing")?;

        writeln!(file, "{}", event)
            .context("Failed to write repair event to events.jsonl")?;

        Ok(())
    }

    /// Generate recommended actions (when auto-repair is disabled)
    fn generate_recommendations(
        &self,
        checkpoint_sync: &CheckpointSyncStatus,
        database_health: &DatabaseHealthStatus,
    ) -> Vec<String> {
        let mut actions = Vec::new();

        if database_health.corrupted {
            if database_health.exists {
                actions.push(
                    "Database is corrupted. Run: bead doctor --repair".to_string()
                );
            } else {
                actions.push(
                    "Database is missing. Run: bead sync import-only --restore-into-empty --input .beads/checkpoint/forensic.jsonl --actor <your-name>".to_string()
                );
            }
        }

        if checkpoint_sync.corrupted {
            actions.push(
                "Checkpoint is corrupted. Rebuild from forensic.jsonl: bead sync import-only --restore-into-empty".to_string()
            );
        }

        if !checkpoint_sync.checkpoint_exists {
            actions.push(
                "Checkpoint is missing. Create checkpoint: bead sync flush-only".to_string()
            );
        }

        if checkpoint_sync.stale && !checkpoint_sync.corrupted {
            actions.push(format!(
                "Checkpoint is stale by {} minutes. Run: bead sync flush-only",
                checkpoint_sync.stale_minutes.unwrap_or(0)
            ));
        }

        if actions.is_empty() {
            actions.push("No issues detected. Checkpoint system is healthy.".to_string());
        }

        actions
    }

    /// Publish the checkpoint monitor report to JSONL file
    fn publish_report(&self, report: &CheckpointMonitorReport) -> Result<()> {
        let report_path = self.config.checkpoint_report_path();
        let json_line = serde_json::to_string(report)
            .context("Failed to serialize checkpoint monitor report")?;

        let mut file = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&report_path)
            .context("Failed to open checkpoint monitor report file")?;

        writeln!(file, "{}", json_line)
            .context("Failed to write checkpoint monitor report")?;

        eprintln!(
            "📋 Checkpoint monitor report published to {}",
            report_path.display()
        );

        Ok(())
    }

    /// Export Prometheus metrics
    pub fn export_prometheus(&self, last_report: Option<&CheckpointMonitorReport>) -> String {
        let mut output = String::new();

        output.push_str("# Checkpoint monitor metrics\n");

        // Monitor uptime
        let uptime = Utc::now().signed_duration_since(self.start_time);
        output.push_str(&format!(
            "icg_checkpoint_monitor_uptime_seconds {}\n",
            uptime.num_seconds() as f64
        ));

        // Last report metrics
        if let Some(report) = last_report {
            output.push_str("\n# Checkpoint sync status\n");
            output.push_str(&format!(
                "icg_checkpoint_sync_status{{status=\"{}\"}} 1\n",
                report.checkpoint_sync.sync_status
            ));
            output.push_str(&format!(
                "icg_checkpoint_stale {}\n",
                if report.checkpoint_sync.stale { 1 } else { 0 }
            ));
            if let Some(minutes) = report.checkpoint_sync.stale_minutes {
                output.push_str(&format!(
                    "icg_checkpoint_stale_minutes {}\n",
                    minutes
                ));
            }
            output.push_str(&format!(
                "icg_checkpoint_corrupted {}\n",
                if report.checkpoint_sync.corrupted { 1 } else { 0 }
            ));
            output.push_str(&format!(
                "icg_checkpoint_issue_count {}\n",
                report.checkpoint_sync.checkpoint_issue_count.unwrap_or(0)
            ));
            output.push_str(&format!(
                "icg_database_issue_count {}\n",
                report.checkpoint_sync.database_issue_count
            ));

            output.push_str("\n# Database health status\n");
            output.push_str(&format!(
                "icg_database_corrupted {}\n",
                if report.database_health.corrupted { 1 } else { 0 }
            ));
            output.push_str(&format!(
                "icg_database_readable {}\n",
                if report.database_health.readable { 1 } else { 0 }
            ));

            output.push_str("\n# Health status\n");
            output.push_str(&format!(
                "icg_checkpoint_monitor_healthy{{status=\"{}\"}} 1\n",
                report.health_status
            ));

            output.push_str("\n# Repair status\n");
            output.push_str(&format!(
                "icg_checkpoint_repair_triggered {}\n",
                if report.repair_triggered { 1 } else { 0 }
            ));
            output.push_str(&format!(
                "icg_checkpoint_repairs_performed {}\n",
                report.repairs_performed.len()
            ));
        }

        output
    }

    /// Print a human-readable summary of the checkpoint status
    pub fn print_summary(&self, report: &CheckpointMonitorReport) {
        println!("\n=== Checkpoint Health Monitor Report ===\n");
        println!("Timestamp: {}", report.timestamp.format("%Y-%m-%d %H:%M:%S UTC"));
        println!("Health Status: {}", report.health_status);

        println!("\n--- Checkpoint Sync Status ---");
        println!("Status: {}", report.checkpoint_sync.sync_status);
        println!("Exists: {}", report.checkpoint_sync.checkpoint_exists);
        if let Some(ts) = report.checkpoint_sync.checkpoint_timestamp {
            println!("Timestamp: {}", ts.format("%Y-%m-%d %H:%M:%S UTC"));
        }
        if let Some(count) = report.checkpoint_sync.checkpoint_issue_count {
            println!("Issues: {}", count);
        }

        if report.checkpoint_sync.stale {
            if let Some(minutes) = report.checkpoint_sync.stale_minutes {
                println!("⚠️  STALE by {} minutes", minutes);
            }
        }

        if report.checkpoint_sync.corrupted {
            if let Some(details) = &report.checkpoint_sync.corruption_details {
                println!("❌ CORRUPTED: {}", details);
            }
        }

        println!("\n--- Database Health Status ---");
        println!("Exists: {}", report.database_health.exists);
        println!("Readable: {}", report.database_health.readable);
        println!("Schema Valid: {}", report.database_health.schema_valid);
        println!("Corrupted: {}", report.database_health.corrupted);
        println!("Database Issues: {}", report.checkpoint_sync.database_issue_count);

        if report.database_health.corrupted {
            if let Some(details) = &report.database_health.error_details {
                println!("❌ {}", details);
            }
        }

        if !report.repairs_performed.is_empty() {
            println!("\n--- Repairs Performed ---");
            for (i, repair) in report.repairs_performed.iter().enumerate() {
                println!("{}. [{}] {} - {}",
                    i + 1,
                    repair.repair_type,
                    if repair.success { "✅ SUCCESS" } else { "❌ FAILED" },
                    repair.message
                );
                println!("   Duration: {:.2}s", repair.duration_seconds);
            }
        }

        if !report.recommended_actions.is_empty() {
            println!("\n--- Recommended Actions ---");
            for (i, action) in report.recommended_actions.iter().enumerate() {
                println!("{}. {}", i + 1, action);
            }
        }

        if report.health_status == "healthy" {
            println!("\n✅ Checkpoint system is healthy");
        } else if report.health_status == "critical" {
            println!("\n🚨 CRITICAL: Immediate attention required");
        } else if report.health_status == "degraded" {
            println!("\n⚠️  DEGRADED: Performance or functionality affected");
        } else {
            println!("\n⚠️  WARNING: Issues detected but system functional");
        }
    }

    /// Start the monitoring loop
    pub async fn run(&mut self) -> Result<()> {
        eprintln!("🩺 Checkpoint health monitor starting");
        eprintln!("📁 Workspace: {}", self.config.workspace_path.display());
        eprintln!("⏱️  Check interval: {} seconds", self.config.check_interval.as_secs());
        eprintln!("📊 Stale threshold: {} minutes", self.config.stale_threshold_minutes);
        eprintln!("🔧 Auto-repair: {}", self.config.auto_repair_enabled);

        // Ensure diagnostics directory exists
        fs::create_dir_all(self.config.diagnostics_dir())
            .context("Failed to create diagnostics directory")?;

        // Run initial check
        let initial_report = self.run_check()?;
        self.print_summary(&initial_report);

        let mut timer = interval(self.config.check_interval);
        timer.tick().await; // Skip the immediate tick

        loop {
            timer.tick().await;

            match self.run_check() {
                Ok(report) => {
                    eprintln!("✅ Checkpoint check completed: status={}, sync={}, db_corrupted={}",
                        report.health_status,
                        report.checkpoint_sync.sync_status,
                        report.database_health.corrupted
                    );

                    if report.repair_triggered {
                        eprintln!("🔧 Auto-repair triggered: {} repairs performed",
                            report.repairs_performed.len());
                        for repair in &report.repairs_performed {
                            if repair.success {
                                eprintln!("  ✅ {}: {}", repair.repair_type, repair.message);
                            } else {
                                eprintln!("  ❌ {}: {}", repair.repair_type, repair.message);
                            }
                        }
                    }
                }
                Err(e) => {
                    eprintln!("❌ Checkpoint check failed: {:#}", e);
                }
            }
        }
    }
}

impl Default for CheckpointMonitor {
    fn default() -> Self {
        Self::new().expect("Failed to create checkpoint monitor with default config")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_default_config() {
        let config = CheckpointMonitorConfig::default();
        assert_eq!(config.check_interval.as_secs(), 300);
        assert_eq!(config.stale_threshold_minutes, 5);
        assert!(config.auto_repair_enabled);
    }

    #[test]
    fn test_config_from_environment() {
        std::env::set_var("ICG_CHECK_INTERVAL_SECONDS", "600");
        std::env::set_var("ICG_STALE_THRESHOLD_MINUTES", "10");
        std::env::set_var("ICG_AUTO_REPAIR_ENABLED", "false");

        let config = CheckpointMonitorConfig::from_environment();
        assert_eq!(config.check_interval.as_secs(), 600);
        assert_eq!(config.stale_threshold_minutes, 10);
        assert!(!config.auto_repair_enabled);

        std::env::remove_var("ICG_CHECK_INTERVAL_SECONDS");
        std::env::remove_var("ICG_STALE_THRESHOLD_MINUTES");
        std::env::remove_var("ICG_AUTO_REPAIR_ENABLED");
    }

    #[test]
    fn test_checkpoint_sync_status_serialization() {
        let status = CheckpointSyncStatus {
            checkpoint_exists: true,
            checkpoint_timestamp: Some(Utc::now()),
            checkpoint_issue_count: Some(10),
            database_exists: true,
            database_issue_count: 10,
            sync_status: "synchronized".to_string(),
            stale: false,
            stale_minutes: None,
            corrupted: false,
            corruption_details: None,
        };

        let json = serde_json::to_string(&status).unwrap();
        assert!(json.contains("synchronized"));
    }

    #[test]
    fn test_repair_action_serialization() {
        let repair = RepairAction {
            timestamp: Utc::now(),
            repair_type: "checkpoint_flush".to_string(),
            success: true,
            message: "Checkpoint flushed successfully".to_string(),
            duration_seconds: 1.5,
        };

        let json = serde_json::to_string(&repair).unwrap();
        assert!(json.contains("checkpoint_flush"));
        assert!(json.contains("successfully"));
    }

    #[test]
    fn test_checkpoint_report_serialization() {
        let report = CheckpointMonitorReport {
            timestamp: Utc::now(),
            check_interval_seconds: 300,
            health_status: "healthy".to_string(),
            checkpoint_sync: CheckpointSyncStatus {
                checkpoint_exists: true,
                checkpoint_timestamp: Some(Utc::now()),
                checkpoint_issue_count: Some(10),
                database_exists: true,
                database_issue_count: 10,
                sync_status: "synchronized".to_string(),
                stale: false,
                stale_minutes: None,
                corrupted: false,
                corruption_details: None,
            },
            database_health: DatabaseHealthStatus {
                exists: true,
                readable: true,
                schema_valid: true,
                corrupted: false,
                error_details: None,
            },
            repair_triggered: false,
            repairs_performed: vec![],
            recommended_actions: vec![],
        };

        let json = serde_json::to_string(&report).unwrap();
        assert!(json.contains("\"health_status\":\"healthy\""));
    }
}
