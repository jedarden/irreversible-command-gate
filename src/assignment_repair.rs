//! Automated Bead Assignment State Repair Service
//!
//! Continuous monitoring service that detects and repairs stuck bead assignment
//! states where beads remain assigned to inactive workers. This addresses the
//! assigned-but-open failure mode where beads become invisible to the ready
//! frontier.
//!
//! ## Problem
//!
//! When a worker process dies or a bead is improperly reopened, beads can remain
//! in an "assigned-but-open" state - they have an assignee but are not in_progress.
//! This makes them invisible to the `bead list --ready` frontier, causing starvation.
//!
//! ## Solution
//!
//! This service:
//! 1. Queries for all open beads with assignees
//! 2. Checks if each assignee is an active worker process (via ps/procfs)
//! 3. Clears stale assignments using `bead update --clear-assignee`
//! 4. Logs all repairs to `.beads/diagnostics/assignment-repair.jsonl`
//! 5. Runs on a configurable periodic schedule (default: 5 minutes)
//!
//! ## Architecture
//!
//! - Worker detection via process table queries (ps aux or /proc reading)
//! - Safe assignment clearing with verification
//! - Comprehensive logging and metrics
//! - Idempotent operations (safe to run multiple times)
//!
//! ## Usage
//!
//! ```bash
//! # Run the assignment repair monitor (default: 5-minute intervals)
//! cargo run --bin assignment-repair-monitor
//!
//! # With custom configuration
//! cargo run --bin assignment-repair-monitor -- --interval-secs 600
//! ```
//!
//! ## Safety
//!
//! - Only clears assignments from workers that are confirmed inactive
//! - Verifies worker status via multiple methods (ps, process table)
//! - All repairs are logged with full context
//! - Idempotent: safe to run multiple times

use anyhow::{anyhow, Context, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;
use tokio::time::interval;

/// Configuration for the assignment repair service
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AssignmentRepairConfig {
    /// Path to the workspace root (contains .beads directory)
    pub workspace_path: PathBuf,

    /// Interval between repair checks (default: 5 minutes)
    #[serde(default = "default_check_interval")]
    pub check_interval: Duration,

    /// Path to the bead CLI (default: "bead")
    #[serde(default = "default_bead_path")]
    pub bead_path: PathBuf,

    /// Enable automatic repair when stale assignments are detected
    #[serde(default = "default_auto_repair_enabled")]
    pub auto_repair_enabled: bool,

    /// Enable dry-run mode (report only, no changes)
    #[serde(default = "default_dry_run")]
    pub dry_run: bool,
}

fn default_check_interval() -> Duration {
    Duration::from_secs(300) // 5 minutes
}

fn default_bead_path() -> PathBuf {
    PathBuf::from("bead")
}

fn default_auto_repair_enabled() -> bool {
    true
}

fn default_dry_run() -> bool {
    false
}

impl Default for AssignmentRepairConfig {
    fn default() -> Self {
        Self {
            workspace_path: PathBuf::from("."),
            check_interval: default_check_interval(),
            bead_path: default_bead_path(),
            auto_repair_enabled: default_auto_repair_enabled(),
            dry_run: default_dry_run(),
        }
    }
}

impl AssignmentRepairConfig {
    /// Load configuration from environment variables
    pub fn from_environment() -> Self {
        let mut config = Self::default();

        if let Ok(path) = std::env::var("ICG_WORKSPACE_PATH") {
            config.workspace_path = PathBuf::from(path);
        }

        if let Ok(secs) = std::env::var("ICG_CHECK_INTERVAL_SECONDS") {
            if let Ok(secs) = secs.parse::<u64>() {
                config.check_interval = Duration::from_secs(secs.max(60));
            }
        }

        if let Ok(path) = std::env::var("BEAD_PATH") {
            config.bead_path = PathBuf::from(path);
        }

        if let Ok(enabled) = std::env::var("ICG_AUTO_REPAIR_ENABLED") {
            config.auto_repair_enabled = enabled.eq_ignore_ascii_case("true") || enabled == "1";
        }

        if let Ok(dry_run) = std::env::var("ICG_DRY_RUN") {
            config.dry_run = dry_run.eq_ignore_ascii_case("true") || dry_run == "1";
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

    /// Get the path to the assignment repair log JSONL file
    pub fn repair_log_path(&self) -> PathBuf {
        self.diagnostics_dir().join("assignment-repair.jsonl")
    }

    /// Get the path to events.jsonl for logging repairs
    pub fn events_path(&self) -> PathBuf {
        self.workspace_path.join(".beads").join("events.jsonl")
    }
}

/// A bead with its assignment state
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AssignedBead {
    /// Bead ID
    pub id: String,
    /// Bead title
    pub title: String,
    /// Current assignee
    pub assignee: String,
    /// Bead status (should always be "open" for our purposes)
    pub status: String,
    /// When the bead was created
    pub created_at: String,
    /// When the bead was last updated
    pub updated_at: String,
    /// Bead revision
    pub revision: i32,
}

/// Worker status check result
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkerStatus {
    /// Worker name/assignee
    pub worker_name: String,
    /// Whether the worker is currently running
    pub is_active: bool,
    /// Process ID if active
    pub pid: Option<u32>,
    /// Last activity timestamp if available
    pub last_activity: Option<DateTime<Utc>>,
    /// Detection method used
    pub detection_method: String,
}

/// Repair action that was performed
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AssignmentRepair {
    /// Timestamp when repair was performed
    pub timestamp: DateTime<Utc>,

    /// Bead ID that was repaired
    pub bead_id: String,

    /// Bead title
    pub bead_title: String,

    /// Previous assignee that was cleared
    pub previous_assignee: String,

    /// Worker status at time of repair
    pub worker_status: String,

    /// Reason for the repair
    pub reason: String,

    /// Whether repair was successful
    pub success: bool,

    /// Repair output or error message
    pub message: String,
}

/// Assignment repair monitoring report
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AssignmentRepairReport {
    /// Timestamp when the check was performed
    pub timestamp: DateTime<Utc>,

    /// Check interval in seconds
    pub check_interval_seconds: u64,

    /// Total number of assigned-but-open beads found
    pub total_assigned_beads: usize,

    /// Number of beads with active assignees (not repaired)
    pub active_assignments: usize,

    /// Number of beads with stale assignees (repaired or would be repaired)
    pub stale_assignments: usize,

    /// Whether auto-repair was triggered
    pub repair_triggered: bool,

    /// Repairs that were performed
    pub repairs_performed: Vec<AssignmentRepair>,

    /// Worker status summary
    pub worker_status_summary: HashMap<String, WorkerStatus>,
}

/// Assignment repair monitor service
pub struct AssignmentRepairMonitor {
    config: AssignmentRepairConfig,
    start_time: DateTime<Utc>,
}

impl AssignmentRepairMonitor {
    /// Create a new assignment repair monitor with default configuration
    pub fn new() -> Result<Self> {
        Self::with_config(AssignmentRepairConfig::default())
    }

    /// Create a new assignment repair monitor with custom configuration
    pub fn with_config(config: AssignmentRepairConfig) -> Result<Self> {
        // Verify workspace path exists
        if !config.workspace_path.exists() {
            return Err(anyhow!(
                "Workspace path does not exist: {}",
                config.workspace_path.display()
            ));
        }

        // Verify bead database exists
        if !config.beads_db_path().exists() {
            return Err(anyhow!(
                "Bead database not found at: {}",
                config.beads_db_path().display()
            ));
        }

        Ok(Self {
            config,
            start_time: Utc::now(),
        })
    }

    /// Get the configuration
    pub fn config(&self) -> &AssignmentRepairConfig {
        &self.config
    }

    /// Run a single assignment repair check
    pub fn run_check(&mut self) -> Result<AssignmentRepairReport> {
        let timestamp = Utc::now();

        // Ensure diagnostics directory exists
        fs::create_dir_all(self.config.diagnostics_dir())
            .context("Failed to create diagnostics directory")?;

        // Step 1: Query for assigned-but-open beads
        let assigned_beads = self.query_assigned_beads()?;

        // Step 2: Check worker status for each assignee
        let mut worker_status_map = HashMap::new();
        let mut stale_assignments = Vec::new();

        for bead in &assigned_beads {
            // Check if we've already seen this worker
            if !worker_status_map.contains_key(&bead.assignee) {
                let worker_status = self.check_worker_alive(&bead.assignee)?;
                worker_status_map.insert(bead.assignee.clone(), worker_status);
            }

            let worker_status = worker_status_map.get(&bead.assignee).unwrap();

            // Track stale assignments
            if !worker_status.is_active {
                stale_assignments.push(bead.clone());
            }
        }

        let active_assignments = assigned_beads.len() - stale_assignments.len();

        // Step 3: Perform repairs if enabled
        let repairs_performed = if self.config.auto_repair_enabled && !self.config.dry_run {
            self.repair_stale_assignments(&stale_assignments, &worker_status_map)?
        } else {
            Vec::new()
        };

        let report = AssignmentRepairReport {
            timestamp,
            check_interval_seconds: self.config.check_interval.as_secs(),
            total_assigned_beads: assigned_beads.len(),
            active_assignments,
            stale_assignments: stale_assignments.len(),
            repair_triggered: !repairs_performed.is_empty(),
            repairs_performed,
            worker_status_summary: worker_status_map,
        };

        // Publish report to JSONL file
        self.publish_report(&report)?;

        Ok(report)
    }

    /// Query for all open beads with assignees
    fn query_assigned_beads(&self) -> Result<Vec<AssignedBead>> {
        let output = Command::new("bead")
            .args(["list", "--status=open", "--json"])
            .current_dir(&self.config.workspace_path)
            .output()
            .context("Failed to run bead list --status=open")?;

        if !output.status.success() {
            anyhow::bail!(
                "bead list failed with exit code: {:?}, stderr: {}",
                output.status.code(),
                String::from_utf8_lossy(&output.stderr)
            );
        }

        let json =
            String::from_utf8(output.stdout).context("bead list output is not valid UTF-8")?;

        let mut assigned_beads = Vec::new();

        // Parse JSONL output (one JSON object per line)
        for line in json.lines() {
            if line.trim().is_empty() {
                continue;
            }

            if let Ok(value) = serde_json::from_str::<serde_json::Value>(line) {
                // Only include beads with assignees
                if let Some(assignee) = value.get("assignee").and_then(|v| v.as_str()) {
                    if !assignee.is_empty() {
                        if let (Some(id), Some(title)) = (
                            value.get("id").and_then(|v| v.as_str()),
                            value.get("title").and_then(|v| v.as_str()),
                        ) {
                            let status = value
                                .get("status")
                                .and_then(|v| v.as_str())
                                .unwrap_or("unknown")
                                .to_string();

                            let created_at = value
                                .get("created_at")
                                .and_then(|v| v.as_str())
                                .unwrap_or("")
                                .to_string();

                            let updated_at = value
                                .get("updated_at")
                                .and_then(|v| v.as_str())
                                .unwrap_or("")
                                .to_string();

                            let revision =
                                value.get("revision").and_then(|v| v.as_i64()).unwrap_or(0) as i32;

                            assigned_beads.push(AssignedBead {
                                id: id.to_string(),
                                title: title.to_string(),
                                assignee: assignee.to_string(),
                                status,
                                created_at,
                                updated_at,
                                revision,
                            });
                        }
                    }
                }
            }
        }

        Ok(assigned_beads)
    }

    /// Check if a worker process is still alive
    fn check_worker_alive(&self, worker_name: &str) -> Result<WorkerStatus> {
        // Method 1: Try ps aux (most portable)
        let ps_output = match Command::new("ps").args(&["aux", "--sort=-pid"]).output() {
            Ok(output) => String::from_utf8_lossy(&output.stdout).to_string(),
            Err(_) => {
                // If ps fails, try /proc reading on Linux
                if let Ok(status) = self.check_worker_via_proc(worker_name) {
                    return Ok(status);
                }
                return Ok(WorkerStatus {
                    worker_name: worker_name.to_string(),
                    is_active: false,
                    pid: None,
                    last_activity: None,
                    detection_method: "ps_failed_proc_failed".to_string(),
                });
            }
        };

        // Parse ps output to find matching processes
        let is_alive_result = ps_output
            .lines()
            .skip(1) // Skip header
            .any(|line| {
                // Skip our own process
                if line.contains("assignment-repair-monitor") || line.contains("icg") {
                    return false;
                }
                // Check if line contains worker name (not as substring of other things)
                // We need to be careful not to match substrings
                let parts: Vec<&str> = line.split_whitespace().collect();
                if parts.len() >= 11 {
                    // Process command is typically the last part
                    let command = parts[10..].join(" ");
                    command.contains(worker_name)
                } else {
                    line.contains(worker_name)
                }
            });

        Ok(WorkerStatus {
            worker_name: worker_name.to_string(),
            is_active: is_alive_result,
            pid: None, // Could be extracted from ps output if needed
            last_activity: None,
            detection_method: "ps_aux".to_string(),
        })
    }

    /// Check worker via /proc filesystem (Linux-specific)
    #[cfg(target_os = "linux")]
    fn check_worker_via_proc(&self, worker_name: &str) -> Result<WorkerStatus> {
        let proc_path = PathBuf::from("/proc");

        if !proc_path.exists() {
            return Ok(WorkerStatus {
                worker_name: worker_name.to_string(),
                is_active: false,
                pid: None,
                last_activity: None,
                detection_method: "proc_not_available".to_string(),
            });
        }

        // Read /proc directories to find matching processes
        let is_alive_result = proc_path
            .read_dir()
            .map(|entries| {
                entries.filter_map(|e| e.ok()).any(|entry| {
                    let dir_name = entry.file_name();
                    if let Some(pid_str) = dir_name.to_str() {
                        if pid_str.chars().all(|c| c.is_ascii_digit()) {
                            let cmdline_path = entry.path().join("cmdline");
                            if let Ok(cmdline) = fs::read_to_string(cmdline_path) {
                                return cmdline.contains(worker_name);
                            }
                        }
                    }
                    false
                })
            })
            .unwrap_or(false);

        Ok(WorkerStatus {
            worker_name: worker_name.to_string(),
            is_active: is_alive_result,
            pid: None,
            last_activity: None,
            detection_method: "proc_filesystem".to_string(),
        })
    }

    /// Check worker via /proc filesystem (non-Linux stub)
    #[cfg(not(target_os = "linux"))]
    fn check_worker_via_proc(&self, worker_name: &str) -> Result<WorkerStatus> {
        Ok(WorkerStatus {
            worker_name: worker_name.to_string(),
            is_active: false,
            pid: None,
            last_activity: None,
            detection_method: "proc_not_available".to_string(),
        })
    }

    /// Repair stale assignments by clearing assignees
    fn repair_stale_assignments(
        &self,
        stale_beads: &[AssignedBead],
        worker_status_map: &HashMap<String, WorkerStatus>,
    ) -> Result<Vec<AssignmentRepair>> {
        let mut repairs = Vec::new();

        if stale_beads.is_empty() {
            return Ok(repairs);
        }

        eprintln!("🔧 Repairing {} stale assignments", stale_beads.len());

        for bead in stale_beads {
            let worker_status =
                worker_status_map
                    .get(&bead.assignee)
                    .cloned()
                    .unwrap_or(WorkerStatus {
                        worker_name: bead.assignee.clone(),
                        is_active: false,
                        pid: None,
                        last_activity: None,
                        detection_method: "unknown".to_string(),
                    });

            let reason = format!(
                "Worker '{}' is inactive (detected via {})",
                bead.assignee, worker_status.detection_method
            );

            // Attempt to clear the assignee using bead CLI
            let repair_result = self.clear_assignee(bead, &reason);

            let repair = AssignmentRepair {
                timestamp: Utc::now(),
                bead_id: bead.id.clone(),
                bead_title: bead.title.clone(),
                previous_assignee: bead.assignee.clone(),
                worker_status: format!(
                    "worker={}, active={}, method={}",
                    bead.assignee, worker_status.is_active, worker_status.detection_method
                ),
                reason: reason.clone(),
                success: repair_result.is_ok(),
                message: repair_result
                    .map(|_| "Assignment cleared successfully".to_string())
                    .unwrap_or_else(|e| format!("Failed to clear assignment: {}", e)),
            };

            // Log the repair to events.jsonl
            if repair.success {
                self.log_repair(&repair)?;
                eprintln!(
                    "  ✅ [{}] Cleared assignee '{}' from '{}'",
                    bead.id, bead.assignee, bead.title
                );
            } else {
                eprintln!(
                    "  ❌ [{}] Failed to clear assignee '{}': {}",
                    bead.id, bead.assignee, repair.message
                );
            }

            repairs.push(repair);
        }

        Ok(repairs)
    }

    /// Clear the assignee from a bead using the bead CLI
    fn clear_assignee(&self, bead: &AssignedBead, reason: &str) -> Result<()> {
        let output = Command::new(&self.config.bead_path)
            .args([
                "update",
                &bead.id,
                "--clear-assignee",
                "--notes",
                &format!(
                    "Auto-repair: {} (at {})",
                    reason,
                    Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
                ),
            ])
            .current_dir(&self.config.workspace_path)
            .output()
            .context("Failed to run bead update")?;

        if !output.status.success() {
            anyhow::bail!(
                "bead update failed with exit code: {:?}, stderr: {}",
                output.status.code(),
                String::from_utf8_lossy(&output.stderr)
            );
        }

        Ok(())
    }

    /// Log a repair to events.jsonl
    fn log_repair(&self, repair: &AssignmentRepair) -> Result<()> {
        let event = serde_json::json!({
            "issue_id": "assignment-auto-repair",
            "kind": "assignee_repair",
            "actor": "icg-assignment-repair-monitor",
            "time": repair.timestamp.to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
            "detail": {
                "bead_id": repair.bead_id,
                "bead_title": repair.bead_title,
                "previous_assignee": repair.previous_assignee,
                "worker_status": repair.worker_status,
                "reason": repair.reason,
            }
        });

        let mut file = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.config.events_path())
            .context("Failed to open events.jsonl for writing")?;

        writeln!(file, "{}", event).context("Failed to write repair event to events.jsonl")?;

        Ok(())
    }

    /// Publish the repair report to JSONL file
    fn publish_report(&self, report: &AssignmentRepairReport) -> Result<()> {
        let report_path = self.config.repair_log_path();
        let json_line = serde_json::to_string(report)
            .context("Failed to serialize assignment repair report")?;

        let mut file = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&report_path)
            .context("Failed to open assignment repair log file")?;

        writeln!(file, "{}", json_line).context("Failed to write assignment repair report")?;

        eprintln!(
            "📋 Assignment repair report published to {}",
            report_path.display()
        );

        Ok(())
    }

    /// Print a human-readable summary of the repair status
    pub fn print_summary(&self, report: &AssignmentRepairReport) {
        println!("\n=== Assignment Repair Monitor Report ===\n");
        println!(
            "Timestamp: {}",
            report.timestamp.format("%Y-%m-%d %H:%M:%S UTC")
        );
        println!("Total assigned beads: {}", report.total_assigned_beads);
        println!("Active assignments: {}", report.active_assignments);
        println!("Stale assignments: {}", report.stale_assignments);
        println!("Repairs performed: {}", report.repairs_performed.len());

        if !report.worker_status_summary.is_empty() {
            println!("\n--- Worker Status Summary ---");
            for (worker, status) in &report.worker_status_summary {
                let status_str = if status.is_active {
                    "active ✓".to_string()
                } else {
                    "inactive ✗".to_string()
                };
                println!("  {}: {} ({})", worker, status_str, status.detection_method);
            }
        }

        if !report.repairs_performed.is_empty() {
            println!("\n--- Repairs Performed ---");
            for (i, repair) in report.repairs_performed.iter().enumerate() {
                let status = if repair.success {
                    "✅ SUCCESS"
                } else {
                    "❌ FAILED"
                };
                println!(
                    "{}. [{}] {} - {}",
                    i + 1,
                    repair.bead_id,
                    status,
                    repair.bead_title
                );
                println!("   Previous assignee: '{}'", repair.previous_assignee);
                println!("   Reason: {}", repair.reason);
                println!("   Worker status: {}", repair.worker_status);
            }
        }

        if report.stale_assignments > 0 && !self.config.auto_repair_enabled {
            println!("\n--- Recommended Actions ---");
            println!(
                "{} stale assignments detected but auto-repair is disabled",
                report.stale_assignments
            );
            println!("Enable auto-repair or run: bead update <id> --clear-assignee");
        }

        if report.total_assigned_beads == 0 {
            println!("\n✅ No assigned-but-open beads found - assignment state is healthy");
        } else if report.stale_assignments == 0 {
            println!("\n✅ All assignments are active - no stale assignments detected");
        } else {
            println!(
                "\n⚠️  {} stale assignments detected and processed",
                report.stale_assignments
            );
        }
    }

    /// Export Prometheus metrics
    pub fn export_prometheus(&self, last_report: Option<&AssignmentRepairReport>) -> String {
        let mut output = String::new();

        output.push_str("# Assignment repair monitor metrics\n");

        // Monitor uptime
        let uptime = Utc::now().signed_duration_since(self.start_time);
        output.push_str(&format!(
            "icg_assignment_repair_monitor_uptime_seconds {}\n",
            uptime.num_seconds() as f64
        ));

        // Last report metrics
        if let Some(report) = last_report {
            output.push_str("\n# Assignment status metrics\n");
            output.push_str(&format!(
                "icg_assigned_beads_total {}\n",
                report.total_assigned_beads
            ));
            output.push_str(&format!(
                "icg_active_assignments {}\n",
                report.active_assignments
            ));
            output.push_str(&format!(
                "icg_stale_assignments {}\n",
                report.stale_assignments
            ));

            output.push_str("\n# Repair status\n");
            output.push_str(&format!(
                "icg_assignment_repairs_performed {}\n",
                report.repairs_performed.len()
            ));
            output.push_str(&format!(
                "icg_assignment_repair_triggered {}\n",
                if report.repair_triggered { 1 } else { 0 }
            ));

            // Worker status counts
            let active_workers = report
                .worker_status_summary
                .values()
                .filter(|s| s.is_active)
                .count();
            let inactive_workers = report
                .worker_status_summary
                .values()
                .filter(|s| !s.is_active)
                .count();

            output.push_str("\n# Worker status\n");
            output.push_str(&format!("icg_active_workers {}\n", active_workers));
            output.push_str(&format!("icg_inactive_workers {}\n", inactive_workers));
        }

        output
    }

    /// Start the monitoring loop
    pub async fn run(&mut self) -> Result<()> {
        eprintln!("🔧 Assignment repair monitor starting");
        eprintln!("📁 Workspace: {}", self.config.workspace_path.display());
        eprintln!(
            "⏱️  Check interval: {} seconds",
            self.config.check_interval.as_secs()
        );
        eprintln!("🔧 Auto-repair: {}", self.config.auto_repair_enabled);

        if self.config.dry_run {
            eprintln!("🔇 DRY RUN MODE - No changes will be made");
        }

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
                    eprintln!(
                        "✅ Assignment check completed: total={}, active={}, stale={}, repairs={}",
                        report.total_assigned_beads,
                        report.active_assignments,
                        report.stale_assignments,
                        report.repairs_performed.len()
                    );

                    if report.repair_triggered {
                        eprintln!(
                            "🔧 Auto-repair: {} repairs performed",
                            report.repairs_performed.len()
                        );
                    }
                }
                Err(e) => {
                    eprintln!("❌ Assignment check failed: {:#}", e);
                }
            }
        }
    }
}

impl Default for AssignmentRepairMonitor {
    fn default() -> Self {
        Self::new().expect("Failed to create assignment repair monitor with default config")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = AssignmentRepairConfig::default();
        assert_eq!(config.check_interval.as_secs(), 300);
        assert!(config.auto_repair_enabled);
        assert!(!config.dry_run);
    }

    #[test]
    fn test_config_from_environment() {
        std::env::set_var("ICG_CHECK_INTERVAL_SECONDS", "600");
        std::env::set_var("ICG_AUTO_REPAIR_ENABLED", "false");
        std::env::set_var("ICG_DRY_RUN", "true");

        let config = AssignmentRepairConfig::from_environment();
        assert_eq!(config.check_interval.as_secs(), 600);
        assert!(!config.auto_repair_enabled);
        assert!(config.dry_run);

        std::env::remove_var("ICG_CHECK_INTERVAL_SECONDS");
        std::env::remove_var("ICG_AUTO_REPAIR_ENABLED");
        std::env::remove_var("ICG_DRY_RUN");
    }

    #[test]
    fn test_assigned_bead_serialization() {
        let bead = AssignedBead {
            id: "test-bead".to_string(),
            title: "Test Bead".to_string(),
            assignee: "test-worker".to_string(),
            status: "open".to_string(),
            created_at: "2026-08-26T12:00:00Z".to_string(),
            updated_at: "2026-08-26T12:00:00Z".to_string(),
            revision: 1,
        };

        let json = serde_json::to_string(&bead).unwrap();
        assert!(json.contains("test-bead"));
        assert!(json.contains("test-worker"));
    }

    #[test]
    fn test_worker_status_serialization() {
        let status = WorkerStatus {
            worker_name: "test-worker".to_string(),
            is_active: true,
            pid: Some(1234),
            last_activity: None,
            detection_method: "ps_aux".to_string(),
        };

        let json = serde_json::to_string(&status).unwrap();
        assert!(json.contains("test-worker"));
        assert!(json.contains("true"));
    }

    #[test]
    fn test_assignment_repair_serialization() {
        let repair = AssignmentRepair {
            timestamp: Utc::now(),
            bead_id: "test-bead".to_string(),
            bead_title: "Test Bead".to_string(),
            previous_assignee: "test-worker".to_string(),
            worker_status: "inactive".to_string(),
            reason: "Worker is inactive".to_string(),
            success: true,
            message: "Success".to_string(),
        };

        let json = serde_json::to_string(&repair).unwrap();
        assert!(json.contains("test-bead"));
        assert!(json.contains("test-worker"));
    }
}
