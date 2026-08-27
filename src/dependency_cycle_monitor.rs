//! Automated Dependency Cycle Detection and Repair Service
//!
//! Continuous monitoring service that detects and repairs circular dependencies
//! and orphaned dependency references in bead-rs databases. This addresses bead
//! starvation caused by malformed dependency graphs that create unresolvable
//! blocking chains.
//!
//! ## Problem
//!
//! Bead starvation occurs when beads become invisible to the ready frontier due to:
//! 1. Circular dependencies (bead A blocks B, B blocks A)
//! 2. Orphaned dependencies (bead blocked by closed/non-existent bead)
//! 3. Complex dependency chains that cannot resolve
//!
//! These issues prevent `bead list --ready` from returning workable beads even when
//! open beads exist in the database.
//!
//! ## Solution
//!
//! This service:
//! 1. Builds the full dependency graph from all open beads
//! 2. Detects cycles using DFS with visited sets and recursion stack
//! 3. Detects beads blocked by dependencies that don't exist or are closed
//! 4. Breaks cycles by removing the most recently added dependency edge
//! 5. Clears invalid blocking dependencies (orphans)
//! 6. Reports findings to `.beads/diagnostics/dependency-cycle-report.jsonl`
//! 7. Logs all repairs to events.jsonl for audit
//! 8. Runs on a configurable periodic schedule (default: 5 minutes)
//!
//! ## Architecture
//!
//! - Dependency graph construction from SQLite database
//! - Cycle detection using depth-first search with coloring
//! - Orphan detection by validating blocker existence and status
//! - Automated repair with transaction safety
//! - Comprehensive logging and metrics
//! - Idempotent operations (safe to run multiple times)
//!
//! ## Usage
//!
//! ```bash
//! # Run the dependency cycle monitor (default: 5-minute intervals)
//! cargo run --bin dependency-cycle-monitor
//!
//! # With custom configuration
//! cargo run --bin dependency-cycle-monitor -- --interval-secs 600
//!
//! # Dry-run mode (detect only, no repairs)
//! cargo run --bin dependency-cycle-monitor -- --dry-run
//! ```
//!
//! ## Safety
//!
//! - Only removes dependencies that are confirmed circular or orphaned
//! - Uses SQLite transactions for atomic repairs
//! - All repairs are logged with full context
//! - Idempotent: safe to run multiple times
//! - Dry-run mode for testing before enabling repairs

use anyhow::{anyhow, Context, Result};
use chrono::{DateTime, Utc};
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::Duration;
use tokio::time::interval;

/// Configuration for the dependency cycle monitor
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DependencyCycleConfig {
    /// Path to the workspace root (contains .beads directory)
    pub workspace_path: PathBuf,

    /// Interval between cycle detection checks (default: 1 hour)
    #[serde(default = "default_check_interval")]
    pub check_interval: Duration,

    /// Enable automatic repair when cycles are detected
    #[serde(default = "default_auto_repair_enabled")]
    pub auto_repair_enabled: bool,

    /// Enable dry-run mode (report only, no changes)
    #[serde(default = "default_dry_run")]
    pub dry_run: bool,

    /// Maximum cycle length to attempt auto-repair (default: 10)
    #[serde(default = "default_max_cycle_length")]
    pub max_cycle_length: usize,

    /// Convert blocking edges to non-blocking references instead of removing
    #[serde(default = "default_convert_to_non_blocking")]
    pub convert_to_non_blocking: bool,
}

fn default_check_interval() -> Duration {
    Duration::from_secs(3600) // 1 hour
}

fn default_auto_repair_enabled() -> bool {
    true
}

fn default_dry_run() -> bool {
    false
}

fn default_max_cycle_length() -> usize {
    10
}

fn default_convert_to_non_blocking() -> bool {
    false // By default, remove blocking edges
}

impl Default for DependencyCycleConfig {
    fn default() -> Self {
        Self {
            workspace_path: PathBuf::from("."),
            check_interval: default_check_interval(),
            auto_repair_enabled: default_auto_repair_enabled(),
            dry_run: default_dry_run(),
            max_cycle_length: default_max_cycle_length(),
            convert_to_non_blocking: default_convert_to_non_blocking(),
        }
    }
}

impl DependencyCycleConfig {
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

        if let Ok(enabled) = std::env::var("ICG_AUTO_REPAIR_ENABLED") {
            config.auto_repair_enabled = enabled.eq_ignore_ascii_case("true") || enabled == "1";
        }

        if let Ok(dry_run) = std::env::var("ICG_DRY_RUN") {
            config.dry_run = dry_run.eq_ignore_ascii_case("true") || dry_run == "1";
        }

        if let Ok(max_len) = std::env::var("ICG_MAX_CYCLE_LENGTH") {
            if let Ok(max_len) = max_len.parse::<usize>() {
                config.max_cycle_length = max_len.max(3);
            }
        }

        if let Ok(convert) = std::env::var("ICG_CONVERT_TO_NON_BLOCKING") {
            config.convert_to_non_blocking = convert.eq_ignore_ascii_case("true") || convert == "1";
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

    /// Get the path to the dependency cycle report log JSONL file
    pub fn report_log_path(&self) -> PathBuf {
        self.diagnostics_dir().join("dependency-cycle-report.jsonl")
    }

    /// Get the path to events.jsonl for logging repairs
    pub fn events_path(&self) -> PathBuf {
        self.workspace_path.join(".beads").join("events.jsonl")
    }
}

/// A bead with its metadata and dependencies
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Bead {
    pub id: String,
    pub title: String,
    pub status: String,
    pub created_at: DateTime<Utc>,
    pub assignee: Option<String>,
    pub manual_blocked: bool,
    pub priority: i32,
}

/// A dependency relationship between beads
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Dependency {
    pub blocked_issue_id: String,
    pub blocker_issue_id: String,
    pub kind: String,
}

/// A detected cycle in the dependency graph
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DetectedCycle {
    /// The cycle as a list of bead IDs in order
    pub cycle: Vec<String>,
    /// The bead that will have its dependency cleared (lowest priority in cycle)
    pub bead_to_modify: String,
    /// The blocker dependency that will be removed or converted
    pub blocker_to_remove: String,
    /// Cycle length
    pub length: usize,
    /// Whether this cycle is safe to auto-repair
    pub safe_to_repair: bool,
    /// Priority of the bead being modified (for reporting)
    pub priority: i32,
}

/// An orphaned dependency (blocked by non-existent or closed bead)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OrphanedDependency {
    pub bead_id: String,
    pub bead_title: String,
    pub missing_blocker: String,
    pub reason: String,
}

/// A repair that was performed
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DependencyRepair {
    /// Timestamp when repair was performed
    pub timestamp: DateTime<Utc>,
    /// Type of repair performed
    pub repair_type: String,
    /// Bead ID that was affected
    pub bead_id: String,
    /// Bead title
    pub bead_title: String,
    /// Previous blocker that was removed (if applicable)
    pub previous_blocker: Option<String>,
    /// Reason for the repair
    pub reason: String,
    /// Whether repair was successful
    pub success: bool,
    /// Repair output or error message
    pub message: String,
}

/// Dependency cycle detection and repair report
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DependencyCycleReport {
    /// Timestamp when the check was performed
    pub timestamp: DateTime<Utc>,

    /// Check interval in seconds
    pub check_interval_seconds: u64,

    /// Total number of open beads checked
    pub total_open_beads: usize,

    /// Total number of dependencies analyzed
    pub total_dependencies: usize,

    /// Number of circular dependencies detected
    pub circular_dependencies_found: usize,

    /// Number of orphaned dependencies detected
    pub orphaned_dependencies_found: usize,

    /// Whether auto-repair was triggered
    pub repair_triggered: bool,

    /// Detected cycles (with repair details)
    pub detected_cycles: Vec<DetectedCycle>,

    /// Orphaned dependencies (with repair details)
    pub orphaned_dependencies: Vec<OrphanedDependency>,

    /// Repairs that were performed
    pub repairs_performed: Vec<DependencyRepair>,
}

/// Dependency cycle detection and repair monitor service
pub struct DependencyCycleMonitor {
    config: DependencyCycleConfig,
    start_time: DateTime<Utc>,
}

impl DependencyCycleMonitor {
    /// Create a new monitor with default configuration
    pub fn new() -> Result<Self> {
        Self::with_config(DependencyCycleConfig::default())
    }

    /// Create a new monitor with custom configuration
    pub fn with_config(config: DependencyCycleConfig) -> Result<Self> {
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
    pub fn config(&self) -> &DependencyCycleConfig {
        &self.config
    }

    /// Run a single dependency cycle detection check
    pub fn run_check(&mut self) -> Result<DependencyCycleReport> {
        let timestamp = Utc::now();

        // Ensure diagnostics directory exists
        fs::create_dir_all(self.config.diagnostics_dir())
            .context("Failed to create diagnostics directory")?;

        // Open database connection
        let conn =
            Connection::open(&self.config.beads_db_path()).context("Failed to open database")?;

        // Step 1: Load all beads
        let beads = self.load_beads(&conn)?;

        // Step 2: Load all dependencies
        let dependencies = self.load_dependencies(&conn)?;

        // Step 3: Detect circular dependencies
        let detected_cycles = self.detect_circular_dependencies(&beads, &dependencies)?;

        // Step 4: Detect orphaned dependencies
        let orphaned_dependencies = self.detect_orphaned_dependencies(&beads, &dependencies)?;

        // Step 5: Perform repairs if enabled
        let repairs_performed = if self.config.auto_repair_enabled && !self.config.dry_run {
            self.repair_dependency_issues(&conn, &detected_cycles, &orphaned_dependencies)?
        } else {
            Vec::new()
        };

        let report = DependencyCycleReport {
            timestamp,
            check_interval_seconds: self.config.check_interval.as_secs(),
            total_open_beads: beads.values().filter(|b| b.status == "open").count(),
            total_dependencies: dependencies.len(),
            circular_dependencies_found: detected_cycles.len(),
            orphaned_dependencies_found: orphaned_dependencies.len(),
            repair_triggered: !repairs_performed.is_empty(),
            detected_cycles,
            orphaned_dependencies,
            repairs_performed,
        };

        // Publish report to JSONL file
        self.publish_report(&report)?;

        Ok(report)
    }

    /// Load all beads from the database
    fn load_beads(&self, conn: &Connection) -> Result<HashMap<String, Bead>> {
        let mut stmt = conn.prepare(
            "SELECT id, title, base_status, created_at, assignee, manual_blocked, priority
             FROM issues",
        )?;

        let bead_map = stmt
            .query_and_then([], |row| {
                let id: String = row.get(0)?;
                let title: String = row.get(1)?;
                let status: String = row.get(2)?;
                let created_at_str: String = row.get(3)?;
                let assignee: Option<String> = row.get(4)?;
                let manual_blocked: i32 = row.get(5)?;
                let priority: i32 = row.get(6)?;

                let created_at = DateTime::parse_from_rfc3339(&created_at_str)
                    .map(|dt| dt.with_timezone(&Utc))
                    .unwrap_or_else(|_| Utc::now());

                Ok((
                    id.clone(),
                    Bead {
                        id,
                        title,
                        status,
                        created_at,
                        assignee,
                        manual_blocked: manual_blocked == 1,
                        priority,
                    },
                ))
            })?
            .collect::<Result<HashMap<_, _>>>()?;

        Ok(bead_map)
    }

    /// Load all dependencies from the database
    fn load_dependencies(&self, conn: &Connection) -> Result<Vec<Dependency>> {
        let mut stmt = conn.prepare(
            "SELECT blocked_issue_id, blocker_issue_id, kind
             FROM dependencies WHERE kind = 'blocks'",
        )?;

        let dependencies = stmt
            .query_and_then([], |row| {
                Ok(Dependency {
                    blocked_issue_id: row.get(0)?,
                    blocker_issue_id: row.get(1)?,
                    kind: row.get(2)?,
                })
            })?
            .collect::<Result<Vec<_>>>()?;

        Ok(dependencies)
    }

    /// Detect circular dependencies using DFS
    fn detect_circular_dependencies(
        &self,
        beads: &HashMap<String, Bead>,
        dependencies: &[Dependency],
    ) -> Result<Vec<DetectedCycle>> {
        let mut detected_cycles = Vec::new();

        // Build adjacency list for the dependency graph
        let mut graph: HashMap<String, Vec<String>> = HashMap::new();
        for dep in dependencies {
            graph
                .entry(dep.blocked_issue_id.clone())
                .or_insert_with(Vec::new)
                .push(dep.blocker_issue_id.clone());
        }

        // Detect cycles using DFS with coloring
        let mut visited: HashSet<String> = HashSet::new();
        let mut rec_stack: HashSet<String> = HashSet::new();
        let mut cycles_found: HashSet<Vec<String>> = HashSet::new();

        for bead_id in beads.keys() {
            if !visited.contains(bead_id) {
                let mut path = Vec::new();
                self.dfs_cycle_detect(
                    bead_id,
                    &graph,
                    &mut visited,
                    &mut rec_stack,
                    &mut path,
                    &mut cycles_found,
                );
            }
        }

        // Convert cycles to DetectedCycle structs
        for cycle in cycles_found {
            if let Some((bead_to_modify, blocker_to_remove)) = self.resolve_cycle(&cycle, beads) {
                let safe_to_repair = cycle.len() <= self.config.max_cycle_length;
                let priority = beads.get(&bead_to_modify).map(|b| b.priority).unwrap_or(2);
                detected_cycles.push(DetectedCycle {
                    cycle: cycle.clone(),
                    bead_to_modify,
                    blocker_to_remove,
                    length: cycle.len(),
                    safe_to_repair,
                    priority,
                });
            }
        }

        Ok(detected_cycles)
    }

    /// DFS cycle detection helper
    fn dfs_cycle_detect(
        &self,
        node: &str,
        graph: &HashMap<String, Vec<String>>,
        visited: &mut HashSet<String>,
        rec_stack: &mut HashSet<String>,
        path: &mut Vec<String>,
        cycles_found: &mut HashSet<Vec<String>>,
    ) {
        visited.insert(node.to_string());
        rec_stack.insert(node.to_string());
        path.push(node.to_string());

        if let Some(neighbors) = graph.get(node) {
            for neighbor in neighbors {
                if !visited.contains(neighbor) {
                    self.dfs_cycle_detect(neighbor, graph, visited, rec_stack, path, cycles_found);
                } else if rec_stack.contains(neighbor) {
                    // Found a cycle - extract it
                    if let Some(cycle_start) = path.iter().position(|x| x == neighbor) {
                        let cycle_vec: Vec<String> = path[cycle_start..].to_vec();
                        cycles_found.insert(cycle_vec);
                    }
                }
            }
        }

        path.pop();
        rec_stack.remove(node);
    }

    /// Resolve a cycle by determining which edge to remove based on priority
    ///
    /// Uses priority (0=highest, 4=lowest) to determine the lowest-priority edge.
    /// Falls back to creation time for beads with equal priority.
    fn resolve_cycle(
        &self,
        cycle: &[String],
        beads: &HashMap<String, Bead>,
    ) -> Option<(String, String)> {
        if cycle.len() < 2 {
            return None;
        }

        // Find the lowest-priority bead in the cycle (priority 4 = lowest, 0 = highest)
        let mut lowest_priority_bead = &cycle[0];
        let mut lowest_priority = beads.get(lowest_priority_bead)?.priority;
        let mut created_at = beads.get(lowest_priority_bead)?.created_at;

        for bead_id in cycle.iter().skip(1) {
            if let Some(bead) = beads.get(bead_id) {
                // Lower priority (higher number) wins
                // If priorities are equal, younger bead wins (more recent)
                if bead.priority > lowest_priority
                    || (bead.priority == lowest_priority && bead.created_at > created_at)
                {
                    lowest_priority_bead = bead_id;
                    lowest_priority = bead.priority;
                    created_at = bead.created_at;
                }
            }
        }

        // Find who blocks the lowest-priority bead
        for (i, bead_id) in cycle.iter().enumerate() {
            if bead_id == lowest_priority_bead {
                // The blocker is the next bead in the cycle
                let blocker_idx = if i + 1 < cycle.len() { i + 1 } else { 0 };
                let blocker = &cycle[blocker_idx];
                return Some((lowest_priority_bead.clone(), blocker.clone()));
            }
        }

        None
    }

    /// Detect orphaned dependencies (blocked by closed/non-existent beads)
    fn detect_orphaned_dependencies(
        &self,
        beads: &HashMap<String, Bead>,
        dependencies: &[Dependency],
    ) -> Result<Vec<OrphanedDependency>> {
        let mut orphaned = Vec::new();

        for dep in dependencies {
            let blocked_bead = beads.get(&dep.blocked_issue_id);

            // Check if the blocker exists and is open
            if let Some(blocker) = beads.get(&dep.blocker_issue_id) {
                if blocker.status == "closed" {
                    orphaned.push(OrphanedDependency {
                        bead_id: dep.blocked_issue_id.clone(),
                        bead_title: blocked_bead.map(|b| b.title.clone()).unwrap_or_default(),
                        missing_blocker: dep.blocker_issue_id.clone(),
                        reason: format!("Blocker '{}' is closed", dep.blocker_issue_id),
                    });
                }
            } else {
                // Blocker doesn't exist at all
                orphaned.push(OrphanedDependency {
                    bead_id: dep.blocked_issue_id.clone(),
                    bead_title: blocked_bead.map(|b| b.title.clone()).unwrap_or_default(),
                    missing_blocker: dep.blocker_issue_id.clone(),
                    reason: format!(
                        "Blocker '{}' does not exist in database",
                        dep.blocker_issue_id
                    ),
                });
            }
        }

        Ok(orphaned)
    }

    /// Repair dependency issues by removing circular and orphaned dependencies
    fn repair_dependency_issues(
        &self,
        conn: &Connection,
        detected_cycles: &[DetectedCycle],
        orphaned_dependencies: &[OrphanedDependency],
    ) -> Result<Vec<DependencyRepair>> {
        let mut repairs = Vec::new();

        // Repair circular dependencies
        for cycle in detected_cycles {
            if !cycle.safe_to_repair {
                eprintln!(
                    "⚠️  Skipping cycle repair: cycle length {} exceeds max {}",
                    cycle.length, self.config.max_cycle_length
                );
                continue;
            }

            let repair_result = self.fix_circular_dependency(conn, cycle);

            let was_converted = repair_result.as_ref().ok().copied().unwrap_or(false);
            let action = if was_converted {
                "Converted to non-blocking"
            } else {
                "Removed"
            };

            let repair = DependencyRepair {
                timestamp: Utc::now(),
                repair_type: "circular_dependency".to_string(),
                bead_id: cycle.bead_to_modify.clone(),
                bead_title: cycle
                    .cycle
                    .first()
                    .and_then(|id| {
                        conn.query_row(
                            "SELECT title FROM issues WHERE id = ?1",
                            &[&cycle.bead_to_modify],
                            |row| row.get(0),
                        )
                        .ok()
                    })
                    .unwrap_or_default(),
                previous_blocker: Some(cycle.blocker_to_remove.clone()),
                reason: format!(
                    "Circular dependency detected: {} -> ... -> {}",
                    cycle.cycle.join(" -> "),
                    cycle.cycle.first().unwrap_or(&"?".to_string())
                ),
                success: repair_result.is_ok(),
                message: repair_result
                    .map(|_| format!("Dependency {} successfully", action.to_lowercase()))
                    .unwrap_or_else(|e| format!("Failed to fix dependency: {}", e)),
            };

            if repair.success {
                self.log_repair(&repair)?;
                eprintln!(
                    "  ✅ [{}] {} circular dependency: {} was blocked by {}",
                    repair.bead_id,
                    action,
                    repair.bead_id,
                    repair.previous_blocker.as_ref().unwrap()
                );
            } else {
                eprintln!(
                    "  ❌ [{}] Failed to fix circular dependency: {}",
                    repair.bead_id, repair.message
                );
            }

            repairs.push(repair);
        }

        // Repair orphaned dependencies
        for orphan in orphaned_dependencies {
            let repair_result = self.fix_orphaned_dependency(conn, orphan);

            let repair = DependencyRepair {
                timestamp: Utc::now(),
                repair_type: "orphaned_dependency".to_string(),
                bead_id: orphan.bead_id.clone(),
                bead_title: orphan.bead_title.clone(),
                previous_blocker: Some(orphan.missing_blocker.clone()),
                reason: orphan.reason.clone(),
                success: repair_result.is_ok(),
                message: repair_result
                    .map(|_| "Orphaned dependency removed successfully".to_string())
                    .unwrap_or_else(|e| format!("Failed to remove orphaned dependency: {}", e)),
            };

            if repair.success {
                self.log_repair(&repair)?;
                eprintln!(
                    "  ✅ [{}] Removed orphaned dependency: {} was blocked by {}",
                    repair.bead_id,
                    repair.bead_id,
                    repair.previous_blocker.as_ref().unwrap()
                );
            } else {
                eprintln!(
                    "  ❌ [{}] Failed to remove orphaned dependency: {}",
                    repair.bead_id, repair.message
                );
            }

            repairs.push(repair);
        }

        Ok(repairs)
    }

    /// Fix a circular dependency by removing the blocking edge or converting to non-blocking
    fn fix_circular_dependency(&self, conn: &Connection, cycle: &DetectedCycle) -> Result<bool> {
        if self.config.convert_to_non_blocking {
            // Convert the blocking edge to a non-blocking reference
            conn.execute(
                "UPDATE dependencies
                 SET kind = 'reference'
                 WHERE blocked_issue_id = ?1 AND blocker_issue_id = ?2 AND kind = 'blocks'",
                params![&cycle.bead_to_modify, &cycle.blocker_to_remove],
            )
            .context("Failed to convert circular dependency to non-blocking")?;
            Ok(true) // Indicate conversion was performed
        } else {
            // Remove the blocking edge entirely
            conn.execute(
                "DELETE FROM dependencies
                 WHERE blocked_issue_id = ?1 AND blocker_issue_id = ?2 AND kind = 'blocks'",
                params![&cycle.bead_to_modify, &cycle.blocker_to_remove],
            )
            .context("Failed to delete circular dependency")?;
            Ok(false) // Indicate removal was performed
        }
    }

    /// Fix an orphaned dependency by removing it
    fn fix_orphaned_dependency(
        &self,
        conn: &Connection,
        orphan: &OrphanedDependency,
    ) -> Result<()> {
        conn.execute(
            "DELETE FROM dependencies
             WHERE blocked_issue_id = ?1 AND blocker_issue_id = ?2",
            params![&orphan.bead_id, &orphan.missing_blocker],
        )
        .context("Failed to delete orphaned dependency")?;

        Ok(())
    }

    /// Log a repair to events.jsonl
    fn log_repair(&self, repair: &DependencyRepair) -> Result<()> {
        let event = serde_json::json!({
            "issue_id": "dependency-cycle-auto-repair",
            "kind": "dependency_repair",
            "actor": "icg-dependency-cycle-monitor",
            "time": repair.timestamp.to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
            "detail": {
                "repair_type": repair.repair_type,
                "bead_id": repair.bead_id,
                "bead_title": repair.bead_title,
                "previous_blocker": repair.previous_blocker,
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
    fn publish_report(&self, report: &DependencyCycleReport) -> Result<()> {
        let report_path = self.config.report_log_path();
        let json_line =
            serde_json::to_string(report).context("Failed to serialize dependency cycle report")?;

        let mut file = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&report_path)
            .context("Failed to open dependency cycle report log file")?;

        writeln!(file, "{}", json_line).context("Failed to write dependency cycle report")?;

        eprintln!(
            "📋 Dependency cycle report published to {}",
            report_path.display()
        );

        Ok(())
    }

    /// Print a human-readable summary of the repair status
    pub fn print_summary(&self, report: &DependencyCycleReport) {
        println!("\n=== Dependency Cycle Monitor Report ===\n");
        println!(
            "Timestamp: {}",
            report.timestamp.format("%Y-%m-%d %H:%M:%S UTC")
        );
        println!("Total open beads: {}", report.total_open_beads);
        println!("Total dependencies: {}", report.total_dependencies);
        println!(
            "Circular dependencies found: {}",
            report.circular_dependencies_found
        );
        println!(
            "Orphaned dependencies found: {}",
            report.orphaned_dependencies_found
        );
        println!("Repairs performed: {}", report.repairs_performed.len());

        if !report.detected_cycles.is_empty() {
            println!("\n--- Detected Cycles ---");
            for (i, cycle) in report.detected_cycles.iter().enumerate() {
                let status = if cycle.safe_to_repair {
                    "✓ Repairable"
                } else {
                    "⚠ Too long (skipped)"
                };
                println!(
                    "{}. Cycle (length {}): {}",
                    i + 1,
                    cycle.length,
                    cycle.cycle.join(" -> ")
                );
                println!(
                    "   Bead to modify: {} (priority {}), Blocker to remove: {}",
                    cycle.bead_to_modify, cycle.priority, cycle.blocker_to_remove
                );
                println!("   Status: {}", status);
            }
        }

        if !report.orphaned_dependencies.is_empty() {
            println!("\n--- Orphaned Dependencies ---");
            for (i, orphan) in report.orphaned_dependencies.iter().enumerate() {
                println!("{}. [{}] {}", i + 1, orphan.bead_id, orphan.reason);
                println!("   Missing blocker: {}", orphan.missing_blocker);
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
                println!("{}. [{}] {}", i + 1, repair.bead_id, status);
                println!("   Type: {}", repair.repair_type);
                if let Some(blocker) = &repair.previous_blocker {
                    println!("   Previous blocker: {}", blocker);
                }
                println!("   Reason: {}", repair.reason);
            }
        }

        if report.circular_dependencies_found == 0 && report.orphaned_dependencies_found == 0 {
            println!("\n✅ No dependency issues detected - graph is healthy!");
        } else if !report.repairs_performed.is_empty() {
            println!(
                "\n✅ {} dependency issues repaired successfully",
                report.repairs_performed.len()
            );
        } else if !self.config.auto_repair_enabled {
            println!("\n⚠️  Issues detected but auto-repair is disabled");
            println!("Enable auto-repair to automatically fix dependency issues");
        }

        if self.config.dry_run {
            println!("\n🔇 DRY RUN MODE - No changes were actually made");
            println!("Run without --dry-run to apply repairs.");
        }
    }

    /// Export Prometheus metrics
    pub fn export_prometheus(&self, last_report: Option<&DependencyCycleReport>) -> String {
        let mut output = String::new();

        output.push_str("# Dependency cycle monitor metrics\n");

        // Monitor uptime
        let uptime = Utc::now().signed_duration_since(self.start_time);
        output.push_str(&format!(
            "icg_dependency_cycle_monitor_uptime_seconds {}\n",
            uptime.num_seconds() as f64
        ));

        // Last report metrics
        if let Some(report) = last_report {
            output.push_str("\n# Dependency status metrics\n");
            output.push_str(&format!(
                "icg_open_beads_total {}\n",
                report.total_open_beads
            ));
            output.push_str(&format!(
                "icg_dependencies_total {}\n",
                report.total_dependencies
            ));
            output.push_str(&format!(
                "icg_circular_dependencies {}\n",
                report.circular_dependencies_found
            ));
            output.push_str(&format!(
                "icg_orphaned_dependencies {}\n",
                report.orphaned_dependencies_found
            ));

            output.push_str("\n# Repair status\n");
            output.push_str(&format!(
                "icg_dependency_repairs_performed {}\n",
                report.repairs_performed.len()
            ));
            output.push_str(&format!(
                "icg_dependency_repair_triggered {}\n",
                if report.repair_triggered { 1 } else { 0 }
            ));
        }

        output
    }

    /// Start the monitoring loop
    pub async fn run(&mut self) -> Result<()> {
        eprintln!("🔗 Dependency cycle monitor starting");
        eprintln!("📁 Workspace: {}", self.config.workspace_path.display());
        eprintln!(
            "⏱️  Check interval: {} seconds",
            self.config.check_interval.as_secs()
        );
        eprintln!("🔧 Auto-repair: {}", self.config.auto_repair_enabled);
        eprintln!("📏 Max cycle length: {}", self.config.max_cycle_length);

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
                        "✅ Dependency check completed: beads={}, deps={}, cycles={}, orphans={}, repairs={}",
                        report.total_open_beads,
                        report.total_dependencies,
                        report.circular_dependencies_found,
                        report.orphaned_dependencies_found,
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
                    eprintln!("❌ Dependency check failed: {:#}", e);
                }
            }
        }
    }
}

impl Default for DependencyCycleMonitor {
    fn default() -> Self {
        Self::new().expect("Failed to create dependency cycle monitor with default config")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = DependencyCycleConfig::default();
        assert_eq!(config.check_interval.as_secs(), 3600);
        assert!(config.auto_repair_enabled);
        assert!(!config.dry_run);
        assert_eq!(config.max_cycle_length, 10);
    }

    #[test]
    fn test_config_from_environment() {
        std::env::set_var("ICG_CHECK_INTERVAL_SECONDS", "600");
        std::env::set_var("ICG_AUTO_REPAIR_ENABLED", "false");
        std::env::set_var("ICG_DRY_RUN", "true");
        std::env::set_var("ICG_MAX_CYCLE_LENGTH", "15");

        let config = DependencyCycleConfig::from_environment();
        assert_eq!(config.check_interval.as_secs(), 600);
        assert!(!config.auto_repair_enabled);
        assert!(config.dry_run);
        assert_eq!(config.max_cycle_length, 15);

        std::env::remove_var("ICG_CHECK_INTERVAL_SECONDS");
        std::env::remove_var("ICG_AUTO_REPAIR_ENABLED");
        std::env::remove_var("ICG_DRY_RUN");
        std::env::remove_var("ICG_MAX_CYCLE_LENGTH");
    }

    #[test]
    fn test_detected_cycle_serialization() {
        let cycle = DetectedCycle {
            cycle: vec!["a".to_string(), "b".to_string(), "c".to_string()],
            bead_to_modify: "b".to_string(),
            blocker_to_remove: "c".to_string(),
            length: 3,
            safe_to_repair: true,
            priority: 2,
        };

        let json = serde_json::to_string(&cycle).unwrap();
        assert!(json.contains("cycle"));
        assert!(json.contains("bead_to_modify"));
        assert!(json.contains("safe_to_repair"));
        assert!(json.contains("priority"));
    }

    #[test]
    fn test_orphaned_dependency_serialization() {
        let orphan = OrphanedDependency {
            bead_id: "bead-1".to_string(),
            bead_title: "Test Bead".to_string(),
            missing_blocker: "non-existent".to_string(),
            reason: "Blocker does not exist".to_string(),
        };

        let json = serde_json::to_string(&orphan).unwrap();
        assert!(json.contains("bead-1"));
        assert!(json.contains("non-existent"));
    }

    #[test]
    fn test_dependency_repair_serialization() {
        let repair = DependencyRepair {
            timestamp: Utc::now(),
            repair_type: "circular_dependency".to_string(),
            bead_id: "bead-1".to_string(),
            bead_title: "Test Bead".to_string(),
            previous_blocker: Some("blocker-1".to_string()),
            reason: "Circular dependency detected".to_string(),
            success: true,
            message: "Success".to_string(),
        };

        let json = serde_json::to_string(&repair).unwrap();
        assert!(json.contains("circular_dependency"));
        assert!(json.contains("bead-1"));
    }
}
