//! Automated bead frontier consistency checker and repair service
//!
//! Service that detects and repairs beads that become invisible to the ready frontier
//! while still being open in the database. This addresses "starvation" issues where
//! Pluck sees no candidates despite open beads existing.
//!
//! ## Architecture
//!
//! The service implements a diagnostic-first approach:
//! 1. Queries the bead database for all open/in_progress beads
//! 2. Calls `bead list --ready` to get the actual ready frontier
//! 3. Identifies discrepancies: beads that are open/in_progress but missing from --ready
//! 4. For each discrepancy, runs `bead doctor` to diagnose the issue
//! 5. If `bead doctor` reports a fixable issue, runs `bead doctor --repair`
//! 6. Re-runs `bead list --ready` to verify the bead is now visible
//! 7. Logs all actions to `.beads/diagnostics/frontier-repair.jsonl`
//! 8. Creates structured diagnostic reports for beads still invisible after repair

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::path::PathBuf;
use std::process::Command;
use std::time::Duration;
use tokio::time::interval;

/// Configuration for the frontier consistency service
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FrontierConsistencyServiceConfig {
    /// Path to the workspace root (contains .beads directory)
    pub workspace_path: PathBuf,

    /// Interval between consistency checks (default: 5 minutes)
    #[serde(default = "default_check_interval")]
    pub check_interval: Duration,

    /// Enable auto-repair when bead doctor reports fixable issues
    #[serde(default = "default_auto_repair_enabled")]
    pub auto_repair_enabled: bool,

    /// Alert on beads that remain invisible after repair attempts
    #[serde(default = "default_alert_on_persistent")]
    pub alert_on_persistent: bool,
}

fn default_check_interval() -> Duration {
    Duration::from_secs(300) // 5 minutes
}

fn default_auto_repair_enabled() -> bool {
    true
}

fn default_alert_on_persistent() -> bool {
    true
}

impl Default for FrontierConsistencyServiceConfig {
    fn default() -> Self {
        Self {
            workspace_path: PathBuf::from("."),
            check_interval: default_check_interval(),
            auto_repair_enabled: default_auto_repair_enabled(),
            alert_on_persistent: default_alert_on_persistent(),
        }
    }
}

impl FrontierConsistencyServiceConfig {
    /// Load configuration from environment variables
    pub fn from_environment() -> Self {
        let mut config = Self::default();

        if let Ok(path) = std::env::var("ICG_WORKSPACE_PATH") {
            config.workspace_path = PathBuf::from(path);
        }

        if let Ok(seconds) = std::env::var("ICG_FRONTIER_CHECK_INTERVAL_SECONDS") {
            if let Ok(seconds) = seconds.parse::<u64>() {
                config.check_interval = Duration::from_secs(seconds.max(60));
            }
        }

        if let Ok(enabled) = std::env::var("ICG_AUTO_REPAIR_ENABLED") {
            config.auto_repair_enabled = enabled.eq_ignore_ascii_case("true") || enabled == "1";
        }

        if let Ok(alert) = std::env::var("ICG_ALERT_ON_PERSISTENT") {
            config.alert_on_persistent = alert.eq_ignore_ascii_case("true") || alert == "1";
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

    /// Get the path to the frontier repair log file
    pub fn frontier_repair_log_path(&self) -> PathBuf {
        self.diagnostics_dir().join("frontier-repair.jsonl")
    }
}

/// Bead status from database query
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DatabaseBead {
    pub id: String,
    pub title: String,
    pub base_status: String,
    pub assignee: Option<String>,
    pub manual_blocked: i32,
    pub priority: i32,
    pub created_at: String,
    pub updated_at: String,
}

/// Dependency information for a bead
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BeadDependency {
    pub blocked_issue_id: String,
    pub blocker_issue_id: String,
    pub kind: String,
}

/// Discrepancy found between database and ready frontier
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FrontierDiscrepancy {
    /// Bead ID
    pub bead_id: String,

    /// Bead title
    pub title: String,

    /// Current status in database
    pub status: String,

    /// Assignee (if any)
    pub assignee: Option<String>,

    /// Whether manually blocked
    pub manual_blocked: bool,

    /// Priority
    pub priority: i32,

    /// Dependencies blocking this bead (if any)
    pub blocking_dependencies: Vec<BeadDependency>,

    /// Why the bead is excluded from ready frontier
    pub exclusion_reason: String,

    /// Timestamp when discrepancy was detected
    pub detected_at: DateTime<Utc>,
}

/// Doctor diagnosis result
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DoctorDiagnosis {
    /// Timestamp when diagnosis was performed
    pub timestamp: DateTime<Utc>,

    /// Bead ID
    pub bead_id: String,

    /// Whether doctor found any issues
    pub issues_found: bool,

    /// Issues found (if any)
    pub issues: Vec<String>,

    /// Doctor output
    pub output: String,

    /// Whether the issues are fixable
    pub fixable: bool,

    /// Diagnosis error (if any)
    pub error: Option<String>,
}

/// Repair operation result
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RepairOperation {
    /// Timestamp when repair was performed
    pub timestamp: DateTime<Utc>,

    /// Bead ID
    pub bead_id: String,

    /// Whether the repair was successful
    pub success: bool,

    /// Issues repaired
    pub issues_repaired: Vec<String>,

    /// Repair output
    pub output: String,

    /// Repair error (if any)
    pub error: Option<String>,

    /// Whether the bead became visible after repair
    pub bead_now_visible: bool,
}

/// Verification result after repair
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VerificationResult {
    /// Timestamp when verification was performed
    pub timestamp: DateTime<Utc>,

    /// Bead ID
    pub bead_id: String,

    /// Whether the bead is now visible in ready frontier
    pub visible: bool,

    /// Ready frontier output (for debugging)
    pub ready_output: String,
}

/// Final diagnostic report for beads that remain invisible
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PersistentInvisibilityReport {
    /// Bead ID
    pub bead_id: String,

    /// Bead title
    pub title: String,

    /// Current status
    pub status: String,

    /// Assignee (if any)
    pub assignee: Option<String>,

    /// Dependencies
    pub dependencies: Vec<BeadDependency>,

    /// Labels
    pub labels: Vec<String>,

    /// Last modified timestamp
    pub last_modified: String,

    /// Exclusion reason
    pub exclusion_reason: String,

    /// Doctor diagnosis
    pub doctor_diagnosis: Option<DoctorDiagnosis>,

    /// Repair attempt (if made)
    pub repair_attempt: Option<RepairOperation>,

    /// When this report was generated
    pub reported_at: DateTime<Utc>,

    /// Recommended action
    pub recommended_action: String,
}

/// Consistency check cycle report
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConsistencyCycleReport {
    /// Timestamp when the cycle started
    pub cycle_start: DateTime<Utc>,

    /// Timestamp when the cycle completed
    pub cycle_end: DateTime<Utc>,

    /// Cycle duration in seconds
    pub duration_seconds: f64,

    /// Total open/in_progress beads found in database
    pub total_database_beads: usize,

    /// Total beads in ready frontier
    pub total_ready_beads: usize,

    /// Discrepancies found
    pub discrepancies: Vec<FrontierDiscrepancy>,

    /// Diagnoses performed
    pub diagnoses: Vec<DoctorDiagnosis>,

    /// Repairs attempted
    pub repairs: Vec<RepairOperation>,

    /// Verifications performed
    pub verifications: Vec<VerificationResult>,

    /// Persistent invisibility reports
    pub persistent_reports: Vec<PersistentInvisibilityReport>,

    /// Whether alert was triggered
    pub alert_triggered: bool,

    /// Alert reason (if triggered)
    pub alert_reason: Option<String>,
}

/// Structured event for .beads/events.jsonl monitoring
#[derive(Debug, Clone, Serialize, Deserialize)]
struct FrontierConsistencyEvent {
    /// Event type
    #[serde(rename = "event")]
    event_type: String,

    /// Timestamp
    ts: DateTime<Utc>,

    /// Cycle duration in milliseconds
    duration_ms: i64,

    /// Service name
    service: String,

    /// Total database beads checked
    total_beads: usize,

    /// Beads in ready frontier
    ready_beads: usize,

    /// Discrepancies found
    discrepancies: usize,

    /// Diagnoses performed
    diagnoses: usize,

    /// Repairs attempted
    repairs: usize,

    /// Persistent issues (beads still invisible after repair)
    persistent_issues: usize,

    /// Alert triggered
    alert_triggered: bool,

    /// Alert reason (if any)
    #[serde(skip_serializing_if = "Option::is_none")]
    alert_reason: Option<String>,

    /// Auto-repair was enabled for this cycle
    auto_repair_enabled: bool,
}

/// Bead database frontier consistency service
pub struct FrontierConsistencyService {
    config: FrontierConsistencyServiceConfig,
}

impl FrontierConsistencyService {
    /// Create a new frontier consistency service with the given configuration
    pub fn new(config: FrontierConsistencyServiceConfig) -> Self {
        Self { config }
    }

    /// Get the configuration
    pub fn config(&self) -> &FrontierConsistencyServiceConfig {
        &self.config
    }

    /// Run a single consistency check cycle
    pub fn run_cycle(&mut self) -> Result<ConsistencyCycleReport> {
        let cycle_start = Utc::now();

        eprintln!("🔍 Starting frontier consistency check cycle");
        eprintln!("📁 Workspace: {}", self.config.workspace_path.display());

        // Ensure diagnostics directory exists
        std::fs::create_dir_all(self.config.diagnostics_dir())
            .context("Failed to create diagnostics directory")?;

        // Step 1: Query database for all open/in_progress beads
        let database_beads = self.query_database_beads()?;
        eprintln!("📊 Found {} open/in_progress beads in database", database_beads.len());

        // Step 2: Get ready frontier via bead list --ready
        let ready_beads = self.get_ready_frontier()?;
        eprintln!("✅ Found {} beads in ready frontier", ready_beads.len());

        // Step 3: Identify discrepancies
        let discrepancies = self.identify_discrepancies(&database_beads, &ready_beads)?;
        eprintln!("⚠️  Found {} discrepancies", discrepancies.len());

        // Step 4-6: For each discrepancy, diagnose, repair, and verify
        let mut diagnoses = Vec::new();
        let mut repairs = Vec::new();
        let mut verifications = Vec::new();
        let mut persistent_reports = Vec::new();

        for discrepancy in &discrepancies {
            eprintln!("🔍 Processing discrepancy: {}", discrepancy.bead_id);

            // Step 4: Run bead doctor to diagnose
            let diagnosis = self.run_doctor_diagnosis(&discrepancy.bead_id)?;
            diagnoses.push(diagnosis.clone());

            // Step 5: If fixable, run repair
            if self.config.auto_repair_enabled && diagnosis.fixable && diagnosis.issues_found {
                eprintln!("🔧 Running repair for bead {}", discrepancy.bead_id);
                let repair = self.run_repair(&discrepancy.bead_id)?;
                repairs.push(repair.clone());

                // Step 6: Verify if bead is now visible
                let verification = self.verify_visibility(&discrepancy.bead_id)?;
                verifications.push(verification.clone());

                // If still invisible after repair, create persistent report
                if !verification.visible {
                    let report = self.create_persistent_report(discrepancy, Some(diagnosis), Some(repair))?;
                    persistent_reports.push(report);
                }
            } else {
                // No repair attempted or not fixable - create persistent report
                let report = self.create_persistent_report(discrepancy, Some(diagnosis), None)?;
                persistent_reports.push(report);
            }
        }

        // Determine if alert should be triggered
        let has_persistent_reports = !persistent_reports.is_empty();
        let (alert_triggered, alert_reason) = if self.config.alert_on_persistent && has_persistent_reports {
            (true, Some(format!("{} beads remain invisible after repair attempts", persistent_reports.len())))
        } else {
            (false, None)
        };

        let cycle_end = Utc::now();
        let duration = cycle_end.signed_duration_since(cycle_start);

        let report = ConsistencyCycleReport {
            cycle_start,
            cycle_end,
            duration_seconds: duration.num_seconds() as f64 + duration.num_milliseconds() as f64 / 1000.0,
            total_database_beads: database_beads.len(),
            total_ready_beads: ready_beads.len(),
            discrepancies,
            diagnoses,
            repairs,
            verifications,
            persistent_reports,
            alert_triggered,
            alert_reason,
        };

        // Publish report to log file
        self.publish_report(&report)?;

        // Log status
        eprintln!("✅ Consistency check cycle completed in {:.2}s", report.duration_seconds);
        if report.alert_triggered {
            if let Some(ref reason) = report.alert_reason {
                eprintln!("🚨 ALERT TRIGGERED: {}", reason);
            }
        } else if !report.persistent_reports.is_empty() {
            eprintln!("⚠️  {} beads remain invisible (auto-repair disabled)", report.persistent_reports.len());
        } else {
            eprintln!("✅ All visible beads accounted for - no discrepancies detected");
        }

        Ok(report)
    }

    /// Query the database for all open/in_progress beads
    fn query_database_beads(&self) -> Result<Vec<DatabaseBead>> {
        let db_path = self.config.beads_db_path();

        if !db_path.exists() {
            eprintln!("⚠️  Beads database not found at {}", db_path.display());
            return Ok(Vec::new());
        }

        let output = Command::new("sqlite3")
            .arg(&db_path)
            .arg(
                "SELECT id, title, base_status, assignee, manual_blocked, priority, created_at, updated_at
                 FROM issues
                 WHERE base_status IN ('open', 'in_progress')
                 ORDER BY priority DESC, created_at ASC",
            )
            .output()
            .context("Failed to query beads database")?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(anyhow::anyhow!("Database query failed: {}", stderr));
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        let mut beads = Vec::new();

        for line in stdout.lines() {
            if line.is_empty() {
                continue;
            }

            let parts: Vec<&str> = line.split('|').collect();
            if parts.len() >= 8 {
                let assignee = if parts[3].is_empty() { None } else { Some(parts[3].to_string()) };
                let manual_blocked = parts[4].parse::<i32>().unwrap_or(0);
                let priority = parts[5].parse::<i32>().unwrap_or(0);

                beads.push(DatabaseBead {
                    id: parts[0].to_string(),
                    title: parts[1].to_string(),
                    base_status: parts[2].to_string(),
                    assignee,
                    manual_blocked,
                    priority,
                    created_at: parts[6].to_string(),
                    updated_at: parts[7].to_string(),
                });
            }
        }

        Ok(beads)
    }

    /// Get the ready frontier via bead list --ready
    fn get_ready_frontier(&self) -> Result<HashSet<String>> {
        let output = Command::new("bead")
            .args(["list", "--ready", "--json"])
            .current_dir(&self.config.workspace_path)
            .output()
            .context("Failed to run bead list --ready")?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            eprintln!("⚠️  bead list --ready failed: {}", stderr);
            return Ok(HashSet::new());
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        let mut ready_beads = HashSet::new();

        // Parse JSON lines
        for line in stdout.lines() {
            if line.is_empty() || line.trim() == "[]" {
                continue;
            }

            // Handle both array and single object formats
            let json_str = if line.trim().starts_with('[') {
                line.trim()
            } else {
                &format!("[{}]", line.trim())
            };

            if let Ok(beads) = serde_json::from_str::<Vec<serde_json::Value>>(json_str) {
                for bead in beads {
                    if let Some(id) = bead.get("id").and_then(|v| v.as_str()) {
                        ready_beads.insert(id.to_string());
                    }
                }
            }
        }

        Ok(ready_beads)
    }

    /// Identify discrepancies between database beads and ready frontier
    fn identify_discrepancies(
        &self,
        database_beads: &[DatabaseBead],
        ready_beads: &HashSet<String>,
    ) -> Result<Vec<FrontierDiscrepancy>> {
        let mut discrepancies = Vec::new();

        for bead in database_beads {
            // Check if bead is in ready frontier
            if !ready_beads.contains(&bead.id) {
                // Get dependencies for this bead
                let blocking_dependencies = self.get_blocking_dependencies(&bead.id)?;

                // Determine exclusion reason
                let exclusion_reason = self.determine_exclusion_reason(bead, &blocking_dependencies);

                discrepancies.push(FrontierDiscrepancy {
                    bead_id: bead.id.clone(),
                    title: bead.title.clone(),
                    status: bead.base_status.clone(),
                    assignee: bead.assignee.clone(),
                    manual_blocked: bead.manual_blocked != 0,
                    priority: bead.priority,
                    blocking_dependencies,
                    exclusion_reason,
                    detected_at: Utc::now(),
                });
            }
        }

        Ok(discrepancies)
    }

    /// Get blocking dependencies for a bead
    fn get_blocking_dependencies(&self, bead_id: &str) -> Result<Vec<BeadDependency>> {
        let db_path = self.config.beads_db_path();

        let output = Command::new("sqlite3")
            .arg(&db_path)
            .arg(&format!(
                "SELECT blocked_issue_id, blocker_issue_id, kind
                 FROM dependencies
                 WHERE blocked_issue_id = '{}' AND kind = 'blocks'",
                bead_id
            ))
            .output()
            .context("Failed to query dependencies")?;

        if !output.status.success() {
            return Ok(Vec::new());
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        let mut dependencies = Vec::new();

        for line in stdout.lines() {
            if line.is_empty() {
                continue;
            }

            let parts: Vec<&str> = line.split('|').collect();
            if parts.len() >= 3 {
                dependencies.push(BeadDependency {
                    blocked_issue_id: parts[0].to_string(),
                    blocker_issue_id: parts[1].to_string(),
                    kind: parts[2].to_string(),
                });
            }
        }

        Ok(dependencies)
    }

    /// Determine why a bead is excluded from the ready frontier
    fn determine_exclusion_reason(
        &self,
        bead: &DatabaseBead,
        blocking_dependencies: &[BeadDependency],
    ) -> String {
        if bead.manual_blocked != 0 {
            return "Manually blocked".to_string();
        }

        if bead.assignee.is_some() {
            return format!("Assigned to {}", bead.assignee.as_ref().unwrap());
        }

        if !blocking_dependencies.is_empty() {
            let blockers: Vec<String> = blocking_dependencies
                .iter()
                .map(|d| d.blocker_issue_id.clone())
                .collect();
            return format!("Blocked by dependencies: {}", blockers.join(", "));
        }

        "Unknown cause - needs investigation".to_string()
    }

    /// Run bead doctor diagnosis for a specific bead
    fn run_doctor_diagnosis(&self, bead_id: &str) -> Result<DoctorDiagnosis> {
        let timestamp = Utc::now();

        eprintln!("🩺 Running bead doctor for {}", bead_id);

        let output = Command::new("bead")
            .args(["doctor"])
            .current_dir(&self.config.workspace_path)
            .output()
            .context("Failed to run bead doctor")?;

        let success = output.status.success();
        let stdout = String::from_utf8_lossy(&output.stdout).to_string();
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();

        // Parse issues from output
        let issues = self.parse_doctor_issues(&stdout, &stderr);
        let issues_found = !issues.is_empty();

        // Determine if issues are fixable
        let fixable = self.check_if_fixable(&issues);

        let error = if !success {
            Some(format!("Exit code: {:?}", output.status.code()))
        } else {
            None
        };

        Ok(DoctorDiagnosis {
            timestamp,
            bead_id: bead_id.to_string(),
            issues_found,
            issues,
            output: format!("{}\n{}", stdout, stderr),
            fixable,
            error,
        })
    }

    /// Parse issues from bead doctor output
    fn parse_doctor_issues(&self, stdout: &str, stderr: &str) -> Vec<String> {
        let combined = format!("{} {}", stdout, stderr);
        let mut issues = Vec::new();

        // Look for common issue patterns
        for line in combined.lines() {
            let line_lower = line.to_lowercase();
            if line_lower.contains("error")
                || line_lower.contains("warning")
                || line_lower.contains("issue")
                || line_lower.contains("corruption")
                || line_lower.contains("missing")
            {
                issues.push(line.trim().to_string());
            }
        }

        issues
    }

    /// Check if issues found by doctor are fixable
    fn check_if_fixable(&self, issues: &[String]) -> bool {
        for issue in issues {
            let issue_lower = issue.to_lowercase();
            // These are typically fixable by bead doctor --repair
            if issue_lower.contains("stale")
                || issue_lower.contains("missing index")
                || issue_lower.contains("temporary file")
                || issue_lower.contains("checkpoint view")
            {
                return true;
            }
        }
        false
    }

    /// Run bead doctor --repair for a specific bead
    fn run_repair(&self, bead_id: &str) -> Result<RepairOperation> {
        let timestamp = Utc::now();

        eprintln!("🔧 Running bead doctor --repair");

        let output = Command::new("bead")
            .args(["doctor", "--repair"])
            .current_dir(&self.config.workspace_path)
            .output()
            .context("Failed to run bead doctor --repair")?;

        let success = output.status.success();
        let stdout = String::from_utf8_lossy(&output.stdout).to_string();
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();

        // Parse repaired issues from output
        let issues_repaired = self.parse_repaired_issues(&stdout, &stderr);

        let error = if !success {
            Some(format!("Exit code: {:?}", output.status.code()))
        } else {
            None
        };

        // Verify if bead is now visible
        let ready_beads = self.get_ready_frontier()?;
        let bead_now_visible = ready_beads.contains(bead_id);

        Ok(RepairOperation {
            timestamp,
            bead_id: bead_id.to_string(),
            success,
            issues_repaired,
            output: format!("{}\n{}", stdout, stderr),
            error,
            bead_now_visible,
        })
    }

    /// Parse repaired issues from doctor output
    fn parse_repaired_issues(&self, stdout: &str, stderr: &str) -> Vec<String> {
        let combined = format!("{} {}", stdout, stderr);
        let mut repaired = Vec::new();

        for line in combined.lines() {
            let line_lower = line.to_lowercase();
            if line_lower.contains("repaired")
                || line_lower.contains("fixed")
                || line_lower.contains("cleaned")
                || line_lower.contains("restored")
            {
                repaired.push(line.trim().to_string());
            }
        }

        repaired
    }

    /// Verify if a bead is now visible in the ready frontier
    fn verify_visibility(&self, bead_id: &str) -> Result<VerificationResult> {
        let timestamp = Utc::now();

        let ready_beads = self.get_ready_frontier()?;
        let visible = ready_beads.contains(bead_id);

        // Capture ready output for debugging
        let ready_output = if visible {
            format!("Bead {} is now visible in ready frontier", bead_id)
        } else {
            format!("Bead {} is still invisible in ready frontier", bead_id)
        };

        Ok(VerificationResult {
            timestamp,
            bead_id: bead_id.to_string(),
            visible,
            ready_output,
        })
    }

    /// Create a persistent invisibility report for a bead
    fn create_persistent_report(
        &self,
        discrepancy: &FrontierDiscrepancy,
        doctor_diagnosis: Option<DoctorDiagnosis>,
        repair_attempt: Option<RepairOperation>,
    ) -> Result<PersistentInvisibilityReport> {
        // Get labels for the bead
        let labels = self.get_bead_labels(&discrepancy.bead_id)?;

        let recommended_action = if repair_attempt.is_some() {
            "Manual investigation required - auto-repair did not resolve visibility issue"
        } else if let Some(ref diagnosis) = doctor_diagnosis {
            if diagnosis.fixable {
                "Run bead doctor --repair manually"
            } else {
                "Manual investigation required - issue not automatically fixable"
            }
        } else {
            "Run bead doctor to diagnose the issue"
        };

        Ok(PersistentInvisibilityReport {
            bead_id: discrepancy.bead_id.clone(),
            title: discrepancy.title.clone(),
            status: discrepancy.status.clone(),
            assignee: discrepancy.assignee.clone(),
            dependencies: discrepancy.blocking_dependencies.clone(),
            labels,
            last_modified: discrepancy.detected_at.to_rfc3339(),
            exclusion_reason: discrepancy.exclusion_reason.clone(),
            doctor_diagnosis,
            repair_attempt,
            reported_at: Utc::now(),
            recommended_action: recommended_action.to_string(),
        })
    }

    /// Get labels for a bead
    fn get_bead_labels(&self, bead_id: &str) -> Result<Vec<String>> {
        let db_path = self.config.beads_db_path();

        let output = Command::new("sqlite3")
            .arg(&db_path)
            .arg(&format!(
                "SELECT name FROM labels WHERE issue_id = '{}'",
                bead_id
            ))
            .output()
            .context("Failed to query labels")?;

        if !output.status.success() {
            return Ok(Vec::new());
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        let mut labels = Vec::new();

        for line in stdout.lines() {
            if !line.is_empty() {
                labels.push(line.trim().to_string());
            }
        }

        Ok(labels)
    }

    /// Publish the consistency cycle report to the JSONL file
    fn publish_report(&self, report: &ConsistencyCycleReport) -> Result<()> {
        let report_path = self.config.frontier_repair_log_path();
        let json_line = serde_json::to_string(report)
            .context("Failed to serialize consistency cycle report")?;

        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&report_path)
            .context("Failed to open frontier repair log file")?;

        use std::io::Write;
        writeln!(file, "{}", json_line)
            .context("Failed to write consistency cycle report")?;

        eprintln!(
            "📝 Consistency cycle report logged to {}",
            report_path.display()
        );

        // Also emit to .beads/events.jsonl for monitoring dashboards
        self.emit_monitoring_event(report)?;

        Ok(())
    }

    /// Emit a structured event to .beads/events.jsonl for monitoring dashboards
    fn emit_monitoring_event(&self, report: &ConsistencyCycleReport) -> Result<()> {
        let events_path = self.config.workspace_path.join(".beads").join("events.jsonl");

        let event = FrontierConsistencyEvent {
            event_type: "frontier-consistency-check".to_string(),
            ts: report.cycle_end,
            duration_ms: (report.duration_seconds * 1000.0) as i64,
            service: "frontier-consistency".to_string(),
            total_beads: report.total_database_beads,
            ready_beads: report.total_ready_beads,
            discrepancies: report.discrepancies.len(),
            diagnoses: report.diagnoses.len(),
            repairs: report.repairs.len(),
            persistent_issues: report.persistent_reports.len(),
            alert_triggered: report.alert_triggered,
            alert_reason: report.alert_reason.clone(),
            auto_repair_enabled: self.config.auto_repair_enabled,
        };

        let json_line = serde_json::to_string(&event)
            .context("Failed to serialize monitoring event")?;

        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&events_path)
            .context("Failed to open events.jsonl file")?;

        use std::io::Write;
        writeln!(file, "{}", json_line)
            .context("Failed to write monitoring event")?;

        eprintln!(
            "📊 Monitoring event emitted to {}",
            events_path.display()
        );

        Ok(())
    }

    /// Run a single check and return (synchronous)
    pub fn run_once(&mut self) -> Result<ConsistencyCycleReport> {
        self.run_cycle()
    }

    /// Start the continuous monitoring loop
    pub async fn run(&mut self) -> Result<()> {
        eprintln!("🧭 Bead frontier consistency service starting");
        eprintln!("📁 Workspace: {}", self.config.workspace_path.display());
        eprintln!("⏱️  Check interval: {} seconds", self.config.check_interval.as_secs());
        eprintln!("🔧 Auto-repair: {}", self.config.auto_repair_enabled);
        eprintln!("🚨 Alert on persistent: {}", self.config.alert_on_persistent);

        // Run initial check
        self.run_once()?;

        let mut timer = interval(self.config.check_interval);
        timer.tick().await; // Skip the immediate tick

        loop {
            timer.tick().await;

            match self.run_once() {
                Ok(report) => {
                    eprintln!("✅ Consistency check completed: duration={:.2}s, discrepancies={}, repairs={}",
                        report.duration_seconds,
                        report.discrepancies.len(),
                        report.repairs.len());

                    if report.alert_triggered {
                        if let Some(ref reason) = report.alert_reason {
                            eprintln!("🚨 ALERT: {}", reason);
                        }
                    }
                }
                Err(e) => {
                    eprintln!("❌ Consistency check failed: {:#}", e);
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
        let config = FrontierConsistencyServiceConfig::default();
        assert_eq!(config.check_interval, Duration::from_secs(300));
        assert!(config.auto_repair_enabled);
        assert!(config.alert_on_persistent);
    }

    #[test]
    fn test_exclusion_reason_determination() {
        let dir = tempdir().unwrap();
        let config = FrontierConsistencyServiceConfig {
            workspace_path: dir.path().to_path_buf(),
            ..Default::default()
        };
        let service = FrontierConsistencyService::new(config);

        // Test manual blocked
        let bead = DatabaseBead {
            id: "test-1".to_string(),
            title: "Test Bead".to_string(),
            base_status: "open".to_string(),
            assignee: None,
            manual_blocked: 1,
            priority: 2,
            created_at: "2026-08-26T00:00:00Z".to_string(),
            updated_at: "2026-08-26T00:00:00Z".to_string(),
        };
        let reason = service.determine_exclusion_reason(&bead, &[]);
        assert_eq!(reason, "Manually blocked");

        // Test unassigned with dependencies (before assigning)
        let deps = vec![
            BeadDependency {
                blocked_issue_id: "test-1".to_string(),
                blocker_issue_id: "blocker-1".to_string(),
                kind: "blocks".to_string(),
            },
        ];
        let bead_unassigned = DatabaseBead {
            assignee: None,
            manual_blocked: 0,
            ..bead.clone()
        };
        let reason = service.determine_exclusion_reason(&bead_unassigned, &deps);
        assert!(reason.contains("Blocked by dependencies"));

        // Test assigned (use the original bead since we haven't moved it yet)
        let bead_assigned = DatabaseBead {
            assignee: Some("worker-1".to_string()),
            manual_blocked: 0,
            ..bead
        };
        let reason = service.determine_exclusion_reason(&bead_assigned, &[]);
        assert!(reason.contains("Assigned to worker-1"));
    }

    #[test]
    fn test_fixable_detection() {
        let dir = tempdir().unwrap();
        let config = FrontierConsistencyServiceConfig {
            workspace_path: dir.path().to_path_buf(),
            ..Default::default()
        };
        let service = FrontierConsistencyService::new(config);

        // Fixable issues
        assert!(service.check_if_fixable(&["stale temp file detected".to_string()]));
        assert!(service.check_if_fixable(&["missing index on beads table".to_string()]));
        assert!(service.check_if_fixable(&["checkpoint view corrupted".to_string()]));

        // Non-fixable issues
        assert!(!service.check_if_fixable(&["disk corruption detected".to_string()]));
        assert!(!service.check_if_fixable(&["unrecoverable data loss".to_string()]));
    }

    #[test]
    fn test_report_serialization() {
        let report = ConsistencyCycleReport {
            cycle_start: Utc::now(),
            cycle_end: Utc::now(),
            duration_seconds: 2.5,
            total_database_beads: 10,
            total_ready_beads: 8,
            discrepancies: vec![],
            diagnoses: vec![],
            repairs: vec![],
            verifications: vec![],
            persistent_reports: vec![],
            alert_triggered: false,
            alert_reason: None,
        };

        let json = serde_json::to_string(&report).unwrap();
        assert!(json.contains("\"total_database_beads\":10"));
        assert!(json.contains("\"total_ready_beads\":8"));
        assert!(json.contains("\"discrepancies\":[]"));
    }
}
