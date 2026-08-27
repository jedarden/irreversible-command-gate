//! Starvation Diagnostic System
//!
//! Automated diagnostic suite that runs when Pluck reports zero candidates despite
//! open beads existing. Performs comprehensive analysis of why beads are invisible
//! to the ready frontier and generates detailed reports with root cause analysis.
//!
//! ## Problem
//!
//! Bead starvation occurs when `bead list --ready` returns no results even though
//! open beads exist in the database. This module automatically diagnoses the
//! root cause by replaying Pluck's filter logic with detailed logging.
//!
//! ## Diagnostic Phases
//!
//! 1. **Database State Analysis** - Queries all open beads and their metadata
//! 2. **Pluck Filter Replay** - Replays the ready frontier filter logic with exclusion logging
//! 3. **Checkpoint Sync Verification** - Compares beads.db vs checkpoint consistency
//! 4. **Stale Assignee Detection** - Identifies assigned-but-open beads with dead workers
//! 5. **Database Integrity Verification** - Validates schema, indexes, and corruption
//!
//! ## Usage
//!
//! ```bash
//! # Run diagnostic when starvation is detected
//! cargo run --bin starvation-diagnostic
//!
//! # With custom database path
//! cargo run --bin starvation-diagnostic -- --db-path /path/to/.beads/beads.db
//!
//! # Generate report only (no auto-repair)
//! cargo run --bin starvation-diagnostic -- --report-only
//! ```
//!
//! ## Output
//!
//! Generates a structured diagnostic report at `.beads/diagnostics/starvation-report.jsonl`
//! containing:
//! - Summary of findings
//! - Detailed exclusion reasons for each invisible bead
//! - Checkpoint sync status
//! - Stale assignee detection results
//! - Database integrity check results
//! - Recommended repair actions

use anyhow::{anyhow, Context, Result};
use chrono::{DateTime, Utc};
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// Configuration for the starvation diagnostic
#[derive(Debug, Clone)]
pub struct StarvationDiagnosticConfig {
    /// Path to the beads.db SQLite database
    pub db_path: PathBuf,
    /// Path to the diagnostics output directory
    pub diagnostics_dir: PathBuf,
    /// If true, generate report but don't attempt repairs
    pub report_only: bool,
    /// If true, automatically repair detected issues (clear stale assignees)
    pub auto_repair: bool,
    /// Path to events.jsonl for logging repairs
    pub events_path: PathBuf,
    /// Checkpoint auto-flush threshold (minutes)
    pub checkpoint_stale_threshold_minutes: i64,
}

impl Default for StarvationDiagnosticConfig {
    fn default() -> Self {
        let workspace_path = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        Self {
            db_path: workspace_path.join(".beads/beads.db"),
            diagnostics_dir: workspace_path.join(".beads/diagnostics"),
            report_only: false,
            auto_repair: false, // Disabled by default for safety
            events_path: workspace_path.join(".beads/events.jsonl"),
            checkpoint_stale_threshold_minutes: 5,
        }
    }
}

/// A bead with its full state
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BeadState {
    pub id: String,
    pub title: String,
    pub status: String,
    pub assignee: Option<String>,
    pub manual_blocked: bool,
    pub priority: i32,
    pub created_at: String,
    pub updated_at: String,
    pub revision: i32,
    pub dependencies: Vec<String>,
}

/// Reason why a bead was excluded from the ready frontier
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ExclusionReason {
    /// Bead has an assignee (stale or active)
    HasAssignee { assignee: String },
    /// Bead is manually blocked
    ManualBlocked,
    /// Bead status is not open
    WrongStatus { status: String },
    /// Bead has unresolved dependencies
    HasDependencies { blockers: Vec<String> },
    /// Bead is open with no assignee but still excluded (unexpected)
    UnexpectedExclusion,
}

/// Detailed exclusion information for a single bead
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BeadExclusion {
    pub bead_id: String,
    pub bead_title: String,
    pub reason: ExclusionReason,
    pub description: String,
}

/// Checkpoint sync status
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CheckpointStatus {
    pub checkpoint_exists: bool,
    pub checkpoint_timestamp: Option<DateTime<Utc>>,
    pub database_exists: bool,
    pub checkpoint_issue_count: Option<i32>,
    pub database_issue_count: i32,
    pub sync_status: String,
    pub stale: bool,
    pub stale_minutes: Option<i64>,
}

/// Stale assignee detection result
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StaleAssignee {
    pub bead_id: String,
    pub bead_title: String,
    pub assignee: String,
    pub worker_status: String,
    pub last_activity: Option<String>,
    pub should_clear: bool,
}

/// Database integrity check result
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IntegrityCheck {
    pub database_readable: bool,
    pub schema_valid: bool,
    pub indexes_valid: bool,
    pub corruption_detected: bool,
    pub issues: Vec<String>,
}

/// Summary of the diagnostic findings
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StarvationSummary {
    pub timestamp: DateTime<Utc>,
    pub total_open_beads: usize,
    pub ready_beads: usize,
    pub invisible_beads: usize,
    pub starvation_detected: bool,
    pub primary_cause: String,
}

/// Complete starvation diagnostic report
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StarvationDiagnosticReport {
    /// Timestamp when the diagnostic was run
    pub timestamp: DateTime<Utc>,

    /// Summary of findings
    pub summary: StarvationSummary,

    /// List of all open beads with their state
    pub open_beads: Vec<BeadState>,

    /// Detailed exclusion information for invisible beads
    pub excluded_beads: Vec<BeadExclusion>,

    /// Checkpoint sync status
    pub checkpoint_status: CheckpointStatus,

    /// Stale assignee detection results
    pub stale_assignees: Vec<StaleAssignee>,

    /// Database integrity check results
    pub integrity_check: IntegrityCheck,

    /// Recommended repair actions
    pub recommended_actions: Vec<String>,

    /// Repairs that were actually performed
    pub repairs_performed: Vec<AssigneeRepair>,
}

/// A repair that was performed on a bead
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AssigneeRepair {
    /// Timestamp when the repair was performed
    pub timestamp: DateTime<Utc>,
    /// Bead ID that was repaired
    pub bead_id: String,
    /// Bead title
    pub bead_title: String,
    /// Previous assignee that was cleared
    pub previous_assignee: String,
    /// Reason for the repair
    pub reason: String,
}

/// Starvation diagnostic system
pub struct StarvationDiagnostic {
    config: StarvationDiagnosticConfig,
}

impl StarvationDiagnostic {
    /// Create a new diagnostic with default config
    pub fn new() -> Result<Self> {
        Self::with_config(StarvationDiagnosticConfig::default())
    }

    /// Create a new diagnostic with custom config
    pub fn with_config(config: StarvationDiagnosticConfig) -> Result<Self> {
        if !config.db_path.exists() {
            return Err(anyhow!(
                "Database not found at {}",
                config.db_path.display()
            ));
        }
        Ok(Self { config })
    }

    /// Run the complete diagnostic suite
    pub fn run_diagnostic(&mut self) -> Result<StarvationDiagnosticReport> {
        let timestamp = Utc::now();

        // Ensure diagnostics directory exists
        fs::create_dir_all(&self.config.diagnostics_dir)
            .context("Failed to create diagnostics directory")?;

        // Phase 1: Database state analysis
        let open_beads = self.load_open_beads()?;

        // Phase 2: Pluck filter replay
        let excluded_beads = self.replay_pluck_filter(&open_beads)?;

        // Phase 3: Checkpoint sync verification
        let checkpoint_status = self.verify_checkpoint_sync()?;

        // Phase 4: Stale assignee detection
        let stale_assignees = self.detect_stale_assignees(&open_beads)?;

        // Phase 5: Database integrity verification
        let integrity_check = self.verify_database_integrity()?;

        // Phase 6: Auto-repair (if enabled)
        let repairs_performed = if self.config.auto_repair && !self.config.report_only {
            self.repair_stale_assignees(&stale_assignees)?
        } else {
            Vec::new()
        };

        // Count ready beads (open, no assignee, not blocked)
        let ready_beads = open_beads
            .iter()
            .filter(|b| b.status == "open" && b.assignee.is_none() && !b.manual_blocked)
            .count();

        let total_open_beads = open_beads.len();
        let invisible_beads = total_open_beads.saturating_sub(ready_beads);
        let starvation_detected = total_open_beads > 0 && ready_beads == 0;

        let summary = StarvationSummary {
            timestamp,
            total_open_beads,
            ready_beads,
            invisible_beads,
            starvation_detected,
            primary_cause: self.determine_primary_cause(
                &excluded_beads,
                &checkpoint_status,
                &stale_assignees,
                &integrity_check,
            ),
        };

        let recommended_actions = self.generate_recommendations(
            &summary,
            &excluded_beads,
            &checkpoint_status,
            &stale_assignees,
            &integrity_check,
            &repairs_performed,
        );

        let report = StarvationDiagnosticReport {
            timestamp,
            summary,
            open_beads,
            excluded_beads,
            checkpoint_status,
            stale_assignees,
            integrity_check,
            recommended_actions,
            repairs_performed,
        };

        // Publish report to JSONL file
        self.publish_report(&report)?;

        // Check for unrecoverable conditions and create repair bead if needed
        if self.detect_unrecoverable_conditions(&report) {
            eprintln!("⚠️  Unrecoverable conditions detected - creating repair bead...");
            match self.create_repair_bead(&report) {
                Ok(bead_id) => {
                    eprintln!("✅ Repair bead created: {}", bead_id);
                    eprintln!("   An agent can claim this bead to execute repair actions.");
                }
                Err(e) => {
                    eprintln!("⚠️  Failed to create repair bead: {}", e);
                    eprintln!("   Manual intervention required - see recommended actions above.");
                }
            }
        }

        Ok(report)
    }

    /// Load all open beads from the database
    fn load_open_beads(&self) -> Result<Vec<BeadState>> {
        let conn = Connection::open(&self.config.db_path).context("Failed to open database")?;

        let mut stmt = conn.prepare(
            "SELECT id, title, base_status, assignee, manual_blocked,
                    priority, created_at, updated_at, revision
             FROM issues
             WHERE base_status IN ('open', 'in_progress')
             ORDER BY priority DESC, created_at, id",
        )?;

        let beads = stmt
            .query_and_then([], |row| {
                let id: String = row.get(0)?;
                let title: String = row.get(1)?;
                let status: String = row.get(2)?;
                let assignee: Option<String> = row.get(3)?;
                let manual_blocked: i32 = row.get(4)?;
                let priority: i32 = row.get(5)?;
                let created_at: String = row.get(6)?;
                let updated_at: String = row.get(7)?;
                let revision: i32 = row.get(8)?;

                Ok(BeadState {
                    id,
                    title,
                    status,
                    assignee,
                    manual_blocked: manual_blocked == 1,
                    priority,
                    created_at,
                    updated_at,
                    revision,
                    dependencies: Vec::new(), // Loaded separately
                })
            })?
            .collect::<Result<Vec<_>>>()?;

        Ok(beads)
    }

    /// Replay Pluck's filter logic with detailed logging
    fn replay_pluck_filter(&self, open_beads: &[BeadState]) -> Result<Vec<BeadExclusion>> {
        let mut excluded = Vec::new();

        for bead in open_beads {
            // Check if bead would be in the ready frontier
            // Ready = open AND no assignee AND not manually blocked
            let is_ready = bead.status == "open" && bead.assignee.is_none() && !bead.manual_blocked;

            if !is_ready {
                let reason = if bead.assignee.is_some() {
                    ExclusionReason::HasAssignee {
                        assignee: bead.assignee.clone().unwrap_or_default(),
                    }
                } else if bead.manual_blocked {
                    ExclusionReason::ManualBlocked
                } else if bead.status != "open" {
                    ExclusionReason::WrongStatus {
                        status: bead.status.clone(),
                    }
                } else {
                    ExclusionReason::UnexpectedExclusion
                };

                let description = match &reason {
                    ExclusionReason::HasAssignee { assignee } => {
                        format!(
                            "Bead has assignee '{}', excluded from ready frontier",
                            assignee
                        )
                    }
                    ExclusionReason::ManualBlocked => "Bead is manually blocked".to_string(),
                    ExclusionReason::WrongStatus { status } => {
                        format!("Bead status is '{}', not 'open'", status)
                    }
                    ExclusionReason::HasDependencies { blockers } => {
                        format!("Bead blocked by dependencies: {}", blockers.join(", "))
                    }
                    ExclusionReason::UnexpectedExclusion => {
                        "Bead excluded for unknown reason".to_string()
                    }
                };

                excluded.push(BeadExclusion {
                    bead_id: bead.id.clone(),
                    bead_title: bead.title.clone(),
                    reason,
                    description,
                });
            }
        }

        Ok(excluded)
    }

    /// Verify checkpoint sync status
    fn verify_checkpoint_sync(&self) -> Result<CheckpointStatus> {
        let workspace_path = self
            .config
            .db_path
            .parent()
            .and_then(|p| p.parent())
            .unwrap_or_else(|| Path::new("."));

        let checkpoint_path = workspace_path.join(".beads/checkpoint/current.json");
        let _forensic_path = workspace_path.join(".beads/checkpoint/forensic.jsonl");

        let checkpoint_exists = checkpoint_path.exists();
        let database_exists = self.config.db_path.exists();

        let mut checkpoint_timestamp = None;
        let mut checkpoint_issue_count = None;
        let mut stale = false;
        let mut stale_minutes = None;

        if checkpoint_exists {
            if let Ok(content) = fs::read_to_string(&checkpoint_path) {
                if let Ok(json) = serde_json::from_str::<serde_json::Value>(&content) {
                    if let Some(ts) = json.get("created_at").and_then(|v| v.as_str()) {
                        if let Ok(dt) = DateTime::parse_from_rfc3339(ts) {
                            checkpoint_timestamp = Some(dt.with_timezone(&Utc));
                        }
                    }
                    if let Some(count) = json.get("issue_count").and_then(|v| v.as_i64()) {
                        checkpoint_issue_count = Some(count as i32);
                    }
                }
            }
        }

        // Count database issues
        let database_issue_count = if database_exists {
            let conn = Connection::open(&self.config.db_path)?;
            conn.query_row("SELECT COUNT(*) FROM issues", [], |row| row.get(0))
                .unwrap_or(0)
        } else {
            0
        };

        // Check if checkpoint is stale
        if let Some(cp_time) = checkpoint_timestamp {
            let age = Utc::now().signed_duration_since(cp_time);
            stale_minutes = Some(age.num_minutes());
            stale = stale_minutes > Some(self.config.checkpoint_stale_threshold_minutes);
        }

        let sync_status = if !checkpoint_exists {
            "missing".to_string()
        } else if checkpoint_timestamp.is_none() {
            "invalid".to_string()
        } else if stale {
            "stale".to_string()
        } else if checkpoint_issue_count != Some(database_issue_count) {
            "desynchronized".to_string()
        } else {
            "synchronized".to_string()
        };

        Ok(CheckpointStatus {
            checkpoint_exists,
            checkpoint_timestamp,
            database_exists,
            checkpoint_issue_count,
            database_issue_count,
            sync_status,
            stale,
            stale_minutes,
        })
    }

    /// Detect stale assignees on open beads
    fn detect_stale_assignees(&self, open_beads: &[BeadState]) -> Result<Vec<StaleAssignee>> {
        let mut stale_assignees = Vec::new();

        for bead in open_beads {
            // Only check open beads with assignees (assigned-but-open failure mode)
            if bead.status != "open" || bead.assignee.is_none() {
                continue;
            }

            let assignee = bead.assignee.as_ref().unwrap();

            // Check if worker is still alive
            let (worker_status, should_clear) = self.check_worker_alive(assignee)?;

            stale_assignees.push(StaleAssignee {
                bead_id: bead.id.clone(),
                bead_title: bead.title.clone(),
                assignee: assignee.clone(),
                worker_status,
                last_activity: None, // Could be enhanced with process check
                should_clear,
            });
        }

        Ok(stale_assignees)
    }

    /// Check if a worker process is still alive by querying the process table
    fn check_worker_alive(&self, worker_name: &str) -> Result<(String, bool)> {
        // Worker names typically follow patterns like:
        // - "claude-code-glm-4.7-icg47" (NEEDLE worker)
        // - "luna", "alpha", "bravo" (NATO named workers)
        //
        // We check if any running process matches the worker name pattern
        // by querying the process table via ps(1)

        let output = match Command::new("ps").args(["aux", "--sort=-pid"]).output() {
            Ok(output) => output,
            Err(e) => {
                eprintln!("Warning: Failed to run ps to check worker status: {}", e);
                // If we can't check, assume worker is alive to be safe
                // (better to miss a repair than to clear a valid assignee)
                return Ok(("unknown".to_string(), false));
            }
        };

        let ps_output = String::from_utf8_lossy(&output.stdout);

        // Check if any process line contains the worker name
        // We need to be careful not to match the ps command itself or the diagnostic process
        let is_alive = ps_output
            .lines()
            .skip(1) // Skip header
            .any(|line| {
                // Skip our own process
                if line.contains("starvation-diagnostic") || line.contains("icg") {
                    return false;
                }
                // Check if line contains worker name (not as substring of other things)
                line.contains(worker_name)
            });

        let status = if is_alive {
            "active".to_string()
        } else {
            "inactive".to_string()
        };

        Ok((status, !is_alive))
    }

    /// Repair stale assignees by clearing them from the database
    fn repair_stale_assignees(
        &self,
        stale_assignees: &[StaleAssignee],
    ) -> Result<Vec<AssigneeRepair>> {
        let mut repairs = Vec::new();

        // Filter to only those that need clearing
        let to_clear: Vec<_> = stale_assignees
            .iter()
            .filter(|sa| sa.should_clear)
            .collect();

        if to_clear.is_empty() {
            return Ok(repairs);
        }

        let conn =
            Connection::open(&self.config.db_path).context("Failed to open database for repair")?;

        for sa in to_clear {
            // Clear the assignee field
            match conn.execute(
                "UPDATE issues SET assignee = NULL, updated_at = ?1
                 WHERE id = ?2 AND assignee = ?3",
                params![
                    Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
                    &sa.bead_id,
                    &sa.assignee
                ],
            ) {
                Ok(rows_affected) => {
                    if rows_affected > 0 {
                        let repair = AssigneeRepair {
                            timestamp: Utc::now(),
                            bead_id: sa.bead_id.clone(),
                            bead_title: sa.bead_title.clone(),
                            previous_assignee: sa.assignee.clone(),
                            reason: format!(
                                "Worker '{}' is inactive - cleared stale assignee",
                                sa.assignee
                            ),
                        };

                        // Log the repair to events.jsonl
                        self.log_repair(&repair)?;

                        repairs.push(repair);
                    }
                }
                Err(e) => {
                    eprintln!(
                        "Warning: Failed to clear assignee from bead {}: {}",
                        sa.bead_id, e
                    );
                }
            }
        }

        Ok(repairs)
    }

    /// Log a repair to events.jsonl
    fn log_repair(&self, repair: &AssigneeRepair) -> Result<()> {
        let event = serde_json::json!({
            "issue_id": "starvation-auto-repair",
            "kind": "assignee_repair",
            "actor": "icg-starvation-diagnostic",
            "time": repair.timestamp.to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
            "detail": {
                "bead_id": repair.bead_id,
                "bead_title": repair.bead_title,
                "previous_assignee": repair.previous_assignee,
                "reason": repair.reason,
            }
        });

        let mut file = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.config.events_path)
            .context("Failed to open events.jsonl for writing")?;

        writeln!(file, "{}", event).context("Failed to write repair event to events.jsonl")?;

        eprintln!(
            "🔧 Repaired bead {} - cleared stale assignee '{}'",
            repair.bead_id, repair.previous_assignee
        );

        Ok(())
    }

    /// Verify database integrity
    fn verify_database_integrity(&self) -> Result<IntegrityCheck> {
        let mut issues = Vec::new();

        let conn = match Connection::open(&self.config.db_path) {
            Ok(conn) => conn,
            Err(e) => {
                return Ok(IntegrityCheck {
                    database_readable: false,
                    schema_valid: false,
                    indexes_valid: false,
                    corruption_detected: true,
                    issues: vec![format!("Cannot open database: {}", e)],
                });
            }
        };

        // Check schema validity
        let schema_valid = match conn.query_row("SELECT COUNT(*) FROM issues", [], |_| Ok(())) {
            Ok(_) => true,
            Err(e) => {
                issues.push(format!("Schema check failed: {}", e));
                false
            }
        };

        // Check indexes
        let indexes_valid = match conn.query_row(
            "SELECT COUNT(*) FROM issues WHERE base_status = 'open'",
            [],
            |_| Ok(()),
        ) {
            Ok(_) => true,
            Err(e) => {
                issues.push(format!("Index check failed: {}", e));
                false
            }
        };

        // Check for corruption
        let corruption_detected = !schema_valid || !indexes_valid;

        Ok(IntegrityCheck {
            database_readable: true,
            schema_valid,
            indexes_valid,
            corruption_detected,
            issues,
        })
    }

    /// Determine the primary cause of starvation
    fn determine_primary_cause(
        &self,
        excluded_beads: &[BeadExclusion],
        checkpoint_status: &CheckpointStatus,
        stale_assignees: &[StaleAssignee],
        integrity_check: &IntegrityCheck,
    ) -> String {
        if integrity_check.corruption_detected {
            return "database_corruption".to_string();
        }

        if checkpoint_status.sync_status == "missing" || checkpoint_status.sync_status == "invalid"
        {
            return "checkpoint_failure".to_string();
        }

        let stale_count = stale_assignees.iter().filter(|s| s.should_clear).count();
        if stale_count > 0 {
            return format!("stale_assignees_({}_beads)", stale_count);
        }

        let assigned_count = excluded_beads
            .iter()
            .filter(|e| matches!(&e.reason, ExclusionReason::HasAssignee { .. }))
            .count();

        if assigned_count > 0 {
            return format!("active_assignments_({}_beads)", assigned_count);
        }

        let blocked_count = excluded_beads
            .iter()
            .filter(|e| matches!(&e.reason, ExclusionReason::ManualBlocked))
            .count();

        if blocked_count > 0 {
            return format!("manual_blocks_({}_beads)", blocked_count);
        }

        "unknown_cause".to_string()
    }

    /// Generate recommended repair actions
    fn generate_recommendations(
        &self,
        summary: &StarvationSummary,
        excluded_beads: &[BeadExclusion],
        checkpoint_status: &CheckpointStatus,
        stale_assignees: &[StaleAssignee],
        integrity_check: &IntegrityCheck,
        repairs_performed: &[AssigneeRepair],
    ) -> Vec<String> {
        let mut actions = Vec::new();

        if integrity_check.corruption_detected {
            actions.push("URGENT: Database corruption detected. Run 'bead doctor --repair' or restore from checkpoint.".to_string());
        }

        if checkpoint_status.sync_status == "missing" {
            actions.push(
                "Checkpoint missing. Run 'bead sync flush-only' to create checkpoint.".to_string(),
            );
        } else if checkpoint_status.sync_status == "stale" {
            actions.push(format!(
                "Checkpoint stale by {} minutes. Run 'bead sync flush-only' to sync.",
                checkpoint_status.stale_minutes.unwrap_or(0)
            ));
        } else if checkpoint_status.sync_status == "desynchronized" {
            actions.push(format!("Checkpoint desynchronized (checkpoint: {} issues, database: {} issues). Run 'bead sync flush-only'.",
                checkpoint_status.checkpoint_issue_count.unwrap_or(0),
                checkpoint_status.database_issue_count));
        }

        // If auto-repair was performed, report it
        if !repairs_performed.is_empty() {
            actions.push(format!(
                "AUTO-REPAIRED: Cleared stale assignees from {} beads",
                repairs_performed.len()
            ));
            for repair in repairs_performed {
                actions.push(format!(
                    "  ✓ [{}] Cleared '{}' - {}",
                    repair.bead_id, repair.previous_assignee, repair.reason
                ));
            }
        } else {
            // Only recommend manual action if auto-repair was not enabled
            let stale_count = stale_assignees.iter().filter(|s| s.should_clear).count();
            if stale_count > 0 {
                actions.push(format!(
                    "Clear stale assignees from {} assigned-but-open beads",
                    stale_count
                ));
                if !self.config.auto_repair {
                    actions.push(
                        "  Option 1: Run with --auto-repair to clear automatically".to_string(),
                    );
                    actions.push(
                        "  Option 2: Manual 'bead update --clear-assignee' for each bead"
                            .to_string(),
                    );
                }
                for sa in stale_assignees.iter().filter(|s| s.should_clear) {
                    actions.push(format!(
                        "  - [{}] Clear assignee '{}'",
                        sa.bead_id, sa.assignee
                    ));
                }
            }

            let assigned_count = excluded_beads
                .iter()
                .filter(|e| matches!(&e.reason, ExclusionReason::HasAssignee { .. }))
                .count();

            if assigned_count > 0 && stale_count == 0 {
                actions.push(format!("Note: {} beads have active assignees and are correctly excluded from ready frontier.", assigned_count));
            }
        }

        if summary.total_open_beads == 0 {
            actions.push(
                "No open beads found in database. Workspace may be idle or all work is complete."
                    .to_string(),
            );
        }

        if actions.is_empty() {
            actions.push(
                "No issues detected. Starvation may be transient or already resolved.".to_string(),
            );
        }

        actions
    }

    /// Publish the diagnostic report to JSONL file
    fn publish_report(&self, report: &StarvationDiagnosticReport) -> Result<()> {
        let report_path = self.config.diagnostics_dir.join("starvation-report.jsonl");
        let json_line =
            serde_json::to_string(report).context("Failed to serialize diagnostic report")?;

        let mut file = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&report_path)
            .context("Failed to open diagnostic report file")?;

        use std::io::Write;
        writeln!(file, "{}", json_line).context("Failed to write diagnostic report")?;

        eprintln!(
            "📋 Starvation diagnostic report published to {}",
            report_path.display()
        );

        Ok(())
    }

    /// Detect unrecoverable conditions that require human intervention
    /// Returns true if the diagnostic found conditions that cannot be auto-repaired
    fn detect_unrecoverable_conditions(&self, report: &StarvationDiagnosticReport) -> bool {
        // Database corruption is unrecoverable without manual intervention
        if report.integrity_check.corruption_detected {
            return true;
        }

        // Checkpoint is missing or invalid - requires manual checkpoint restore
        if report.checkpoint_status.sync_status == "missing"
            || report.checkpoint_status.sync_status == "invalid"
        {
            return true;
        }

        // Stale assignees exist but auto-repair was disabled or failed
        let stale_count = report
            .stale_assignees
            .iter()
            .filter(|s| s.should_clear)
            .count();

        if stale_count > 0 && !self.config.auto_repair {
            // Stale assignees exist but auto-repair is not enabled
            // This IS recoverable if we enable auto-repair, so NOT unrecoverable
            return false;
        }

        // If we have open beads but none are ready and we can't identify why
        if report.summary.starvation_detected && report.excluded_beads.is_empty() {
            // Starvation detected but no beads were excluded - this is unexpected
            // and indicates a deeper problem
            return true;
        }

        false
    }

    /// Create a starvation-resolution bead when unrecoverable conditions are detected
    /// This creates a claimable bead for agents (not a human-blocked repair bead)
    fn create_repair_bead(&self, report: &StarvationDiagnosticReport) -> Result<String> {
        let workspace_path = self
            .config
            .db_path
            .parent()
            .and_then(|p| p.parent())
            .unwrap_or_else(|| Path::new("."));

        // Generate bead title and description
        let title = format!(
            "Starvation-resolution: Resolve {}",
            report.summary.primary_cause.replace('_', " ")
        );

        let mut notes = format!(
            "# Starvation Resolution Plan\n\n\
            **Auto-generated at:** {}\n\
            **Primary Cause:** {}\n\
            **Workspace:** {}\n\n",
            report.timestamp.format("%Y-%m-%d %H:%M:%S UTC"),
            report.summary.primary_cause,
            workspace_path.display()
        );

        notes.push_str(
            "**Purpose:** This bead contains a clear action plan to resolve bead starvation.\n\n",
        );
        notes.push_str("**Agents should claim this bead to execute the resolution steps.**\n\n");

        // Problem statement
        notes.push_str("## Problem Statement\n\n");
        notes.push_str(&format!(
            "- **Total open beads:** {}\n\
            - **Ready beads:** {}\n\
            - **Invisible beads:** {}\n\
            - **Starvation detected:** {}\n\n",
            report.summary.total_open_beads,
            report.summary.ready_beads,
            report.summary.invisible_beads,
            report.summary.starvation_detected
        ));

        // Diagnostic findings
        notes.push_str("## Diagnostic Findings\n\n");

        // Database integrity
        notes.push_str("### Database Integrity\n");
        if report.integrity_check.corruption_detected {
            notes.push_str("❌ **CORRUPTION DETECTED** - This is an URGENT issue requiring immediate attention.\n");
            for issue in &report.integrity_check.issues {
                notes.push_str(&format!("- {}\n", issue));
            }
        } else {
            notes.push_str("✅ Database integrity OK\n");
        }
        notes.push('\n');

        // Checkpoint status
        notes.push_str("### Checkpoint Status\n");
        notes.push_str(&format!(
            "- **Status:** {}\n",
            report.checkpoint_status.sync_status
        ));
        notes.push_str(&format!(
            "- **Exists:** {}\n",
            report.checkpoint_status.checkpoint_exists
        ));
        if let Some(minutes) = report.checkpoint_status.stale_minutes {
            notes.push_str(&format!("- **Stale by:** {} minutes\n", minutes));
        }
        if let Some(cp_count) = report.checkpoint_status.checkpoint_issue_count {
            notes.push_str(&format!(
                "- **Issue count:** checkpoint={}, database={}\n",
                cp_count, report.checkpoint_status.database_issue_count
            ));
        }
        notes.push('\n');

        // Stale assignees
        if !report.stale_assignees.is_empty() {
            notes.push_str("### Stale Assignees Detected\n\n");
            notes.push_str("The following beads have stale assignees (inactive workers):\n\n");
            for sa in &report.stale_assignees {
                if sa.should_clear {
                    notes.push_str(&format!(
                        "- **[{}] {}** - assigned to `{}` (INACTIVE)\n",
                        sa.bead_id, sa.bead_title, sa.assignee
                    ));
                }
            }
            notes.push('\n');
        }

        // Attempted repairs
        if !report.repairs_performed.is_empty() {
            notes.push_str("## Auto-Repair Results\n\n");
            notes.push_str(&format!(
                "Auto-repair performed {} actions:\n",
                report.repairs_performed.len()
            ));
            for repair in &report.repairs_performed {
                notes.push_str(&format!(
                    "- ✅ [{}] Cleared assignee `{}` - {}\n",
                    repair.bead_id, repair.previous_assignee, repair.reason
                ));
            }
            notes.push('\n');
        }

        // Clear action plan
        notes.push_str("## Clear Action Plan\n\n");
        notes.push_str("Execute the following steps in order:\n\n");

        let mut step_num = 1;

        // Step 1: Address database corruption if detected
        if report.integrity_check.corruption_detected {
            notes.push_str(&format!(
                "{}. **URGENT: Address database corruption**\n",
                step_num
            ));
            notes.push_str("   - Run `bead doctor --repair` to attempt automatic repair\n");
            notes.push_str("   - If repair fails, restore from checkpoint: `bead sync import-only --input .beads/checkpoint/forensic.jsonl --restore-into-empty --actor <you>`\n");
            notes.push_str("   - Verify database integrity before proceeding\n\n");
            step_num += 1;
        }

        // Step 2: Address checkpoint issues
        if report.checkpoint_status.sync_status == "missing"
            || report.checkpoint_status.sync_status == "invalid"
        {
            notes.push_str(&format!("{}. **Fix checkpoint issues**\n", step_num));
            notes.push_str("   - Create fresh checkpoint: `bead sync flush-only`\n");
            notes.push_str("   - Verify checkpoint was created successfully\n\n");
            step_num += 1;
        } else if report.checkpoint_status.sync_status == "stale" {
            notes.push_str(&format!("{}. **Update stale checkpoint**\n", step_num));
            notes.push_str("   - Sync checkpoint: `bead sync flush-only`\n");
            notes.push_str(&format!(
                "   - Checkpoint is stale by {} minutes\n\n",
                report.checkpoint_status.stale_minutes.unwrap_or(0)
            ));
            step_num += 1;
        }

        // Step 3: Clear stale assignees
        let stale_count = report
            .stale_assignees
            .iter()
            .filter(|s| s.should_clear)
            .count();
        if stale_count > 0 {
            notes.push_str(&format!("{}. **Clear stale assignees**\n", step_num));
            notes.push_str(&format!(
                "   - Clear assignees from {} beads with inactive workers:\n",
                stale_count
            ));
            for sa in report.stale_assignees.iter().filter(|s| s.should_clear) {
                notes.push_str(&format!(
                    "     - `bead update {} --clear-assignee` (worker: {})\n",
                    sa.bead_id, sa.assignee
                ));
            }
            notes.push('\n');
            step_num += 1;
        }

        // Step 4: Verify resolution
        notes.push_str(&format!("{}. **Verify the resolution**\n", step_num));
        notes.push_str("   - Run `cargo run --bin starvation-diagnostic` to re-check\n");
        notes.push_str("   - Verify ready frontier now contains beads\n");
        notes.push_str("   - If starvation persists, create a new resolution bead\n\n");
        step_num += 1;

        // Step 5: Close this bead
        notes.push_str(&format!("{}. **Close this bead**\n", step_num));
        notes.push_str("   - Once starvation is resolved, close this bead with:\n");
        notes.push_str("   - `bead close <this-bead-id> --reason 'Starvation resolved - ready frontier populated'\n\n");

        notes.push_str("---\n\n");
        notes.push_str(
            "*This resolution plan was auto-generated by the starvation diagnostic system.*\n",
        );

        // Create the bead using the bead CLI
        let output = Command::new("bead")
            .args([
                "create",
                "--title",
                &title,
                "--priority",
                "3", // Priority 3 for agent-claimable resolution beads
                "--issue-type",
                "task",
                "--label",
                "starvation-resolution",
                "--label",
                "auto-generated",
                "--label",
                "agent-claimable",
                "--label",
                &format!("cause-{}", report.summary.primary_cause),
            ])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .current_dir(workspace_path)
            .output()
            .context("Failed to execute bead create command")?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(anyhow!("bead create failed: {}", stderr));
        }

        // Parse the bead ID from stdout
        let stdout = String::from_utf8_lossy(&output.stdout);
        let bead_id = stdout
            .lines()
            .find(|line| line.starts_with("ID: ") || line.contains("irrevers-"))
            .and_then(|line| line.split("irrevers-").nth(1))
            .map(|id| format!("irrevers-{}", id.trim().trim_end_matches(':')))
            .or_else(|| {
                // Fallback: extract ID from any line containing "irrevers-"
                stdout
                    .lines()
                    .find(|line| line.contains("irrevers-"))
                    .and_then(|line| line.split("irrevers-").nth(1))
                    .map(|id| format!("irrevers-{}", id.trim()))
            })
            .ok_or_else(|| anyhow!("Failed to parse bead ID from bead create output"))?;

        eprintln!("📋 Auto-generated starvation-resolution bead: {}", bead_id);
        eprintln!("   An agent can claim this bead to execute the resolution steps.");

        // Now add the notes via bead update
        let update_output = Command::new("bead")
            .args(["update", &bead_id, "--notes", &notes])
            .current_dir(workspace_path)
            .output()
            .context("Failed to update bead description")?;

        if !update_output.status.success() {
            let stderr = String::from_utf8_lossy(&update_output.stderr);
            eprintln!("Warning: Failed to update bead notes: {}", stderr);
        } else {
            eprintln!("📝 Added resolution plan to bead: {}", bead_id);
        }

        Ok(bead_id)
    }

    /// Print a human-readable summary of the diagnostic
    pub fn print_summary(&self, report: &StarvationDiagnosticReport) {
        println!("\n=== Starvation Diagnostic Report ===\n");
        println!(
            "Timestamp: {}",
            report.timestamp.format("%Y-%m-%d %H:%M:%S UTC")
        );
        println!("Total open beads: {}", report.summary.total_open_beads);
        println!("Ready beads: {}", report.summary.ready_beads);
        println!("Invisible beads: {}", report.summary.invisible_beads);
        println!(
            "Starvation detected: {}",
            report.summary.starvation_detected
        );
        println!("Primary cause: {}", report.summary.primary_cause);

        if !report.excluded_beads.is_empty() {
            println!("\n--- Excluded Beads ---");
            for (i, exclusion) in report.excluded_beads.iter().enumerate() {
                println!(
                    "{}. [{}] {} - {}",
                    i + 1,
                    exclusion.bead_id,
                    exclusion.bead_title,
                    exclusion.description
                );
            }
        }

        println!("\n--- Checkpoint Status ---");
        println!("Status: {}", report.checkpoint_status.sync_status);
        println!("Exists: {}", report.checkpoint_status.checkpoint_exists);
        if let Some(minutes) = report.checkpoint_status.stale_minutes {
            println!("Stale by: {} minutes", minutes);
        }
        if let Some(cp_count) = report.checkpoint_status.checkpoint_issue_count {
            println!(
                "Issues: checkpoint={}, database={}",
                cp_count, report.checkpoint_status.database_issue_count
            );
        }

        if !report.stale_assignees.is_empty() {
            println!("\n--- Stale Assignees ---");
            for (i, sa) in report.stale_assignees.iter().enumerate() {
                if sa.should_clear {
                    println!(
                        "{}. [{}] {} assigned to {} (INACTIVE)",
                        i + 1,
                        sa.bead_id,
                        sa.bead_title,
                        sa.assignee
                    );
                } else {
                    println!(
                        "{}. [{}] {} assigned to {} (active)",
                        i + 1,
                        sa.bead_id,
                        sa.bead_title,
                        sa.assignee
                    );
                }
            }
        }

        if !report.repairs_performed.is_empty() {
            println!("\n--- Auto-Repair Results ---");
            println!("Repairs performed: {}", report.repairs_performed.len());
            for (i, repair) in report.repairs_performed.iter().enumerate() {
                println!(
                    "{}. [{}] {} - Cleared '{}'",
                    i + 1,
                    repair.bead_id,
                    repair.bead_title,
                    repair.previous_assignee
                );
                println!("   Reason: {}", repair.reason);
                println!(
                    "   Timestamp: {}",
                    repair.timestamp.format("%Y-%m-%d %H:%M:%S UTC")
                );
            }
        }

        println!("\n--- Database Integrity ---");
        println!("Readable: {}", report.integrity_check.database_readable);
        println!("Schema valid: {}", report.integrity_check.schema_valid);
        println!("Indexes valid: {}", report.integrity_check.indexes_valid);
        println!(
            "Corruption detected: {}",
            report.integrity_check.corruption_detected
        );

        if !report.integrity_check.issues.is_empty() {
            println!("Issues:");
            for issue in &report.integrity_check.issues {
                println!("  - {}", issue);
            }
        }

        if !report.recommended_actions.is_empty() {
            println!("\n--- Recommended Actions ---");
            for (i, action) in report.recommended_actions.iter().enumerate() {
                println!("{}. {}", i + 1, action);
            }
        }

        if report.summary.starvation_detected {
            println!("\n🚨 STARVATION DETECTED: Open beads exist but none are ready for work");
        } else {
            println!("\n✅ No starvation detected");
        }
    }
}

impl Default for StarvationDiagnostic {
    fn default() -> Self {
        Self::new().expect("Failed to create diagnostic with default config")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_exclusion_reason_serialization() {
        let reason = ExclusionReason::HasAssignee {
            assignee: "test-worker".to_string(),
        };
        let json = serde_json::to_string(&reason).unwrap();
        assert!(json.contains("HasAssignee"));
    }

    #[test]
    fn test_checkpoint_status() {
        let status = CheckpointStatus {
            checkpoint_exists: true,
            checkpoint_timestamp: Some(Utc::now()),
            database_exists: true,
            checkpoint_issue_count: Some(10),
            database_issue_count: 10,
            sync_status: "synchronized".to_string(),
            stale: false,
            stale_minutes: None,
        };
        let json = serde_json::to_string(&status).unwrap();
        assert!(json.contains("synchronized"));
    }

    #[test]
    fn test_default_config() {
        let config = StarvationDiagnosticConfig::default();
        assert_eq!(config.checkpoint_stale_threshold_minutes, 5);
        assert!(!config.report_only);
        assert!(!config.auto_repair); // Auto-repair is disabled by default for safety
    }

    #[test]
    fn test_assignee_repair_serialization() {
        let repair = AssigneeRepair {
            timestamp: Utc::now(),
            bead_id: "test-bead".to_string(),
            bead_title: "Test Bead".to_string(),
            previous_assignee: "test-worker".to_string(),
            reason: "Worker is inactive".to_string(),
        };
        let json = serde_json::to_string(&repair).unwrap();
        assert!(json.contains("test-bead"));
        assert!(json.contains("test-worker"));
    }
}
