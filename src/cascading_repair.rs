//! Cascading Repair Strategies for Starvation Recovery
//!
//! When primary auto-repair fails (0 ready beads after repairs), this module
//! implements a multi-stage escalation strategy to recover from starvation conditions.
//!
//! ## Strategy Sequence
//!
//! 1. **Aggressive dependency pruning** - Remove blocking dependencies on beads
//!    that haven't been updated in >24 hours
//! 2. **Emergency assignee clearing** - Clear assignees from any bead with 'human' label
//! 3. **Query filter relaxation** - Temporarily disable label exclusions to surface hidden beads
//! 4. **Bead state reset** - For beads with no activity in 48+ hours, automatically reset
//!    to open/unassigned state
//!
//! ## Safety
//!
//! - Each strategy logs all actions before executing
//! - Strategies execute in sequence with verification between stages
//! - All repairs are reversible and logged to diagnostics
//! - Dry-run mode available for testing

use anyhow::{Context, Result};
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::fs;
use std::path::PathBuf;
use std::process::Command;

/// (id, title, base_status, assignee, updated_at) row from an inactive-beads query
type InactiveBeadRow = (String, String, String, Option<String>, String);

/// Configuration for cascading repair strategies
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CascadingRepairConfig {
    /// Path to the workspace root (contains .beads directory)
    pub workspace_path: PathBuf,

    /// Enable aggressive dependency pruning (24+ hour stale blockers)
    #[serde(default = "default_dependency_pruning_enabled")]
    pub dependency_pruning_enabled: bool,

    /// Enable emergency assignee clearing for 'human' labeled beads
    #[serde(default = "default_assignee_clearing_enabled")]
    pub assignee_clearing_enabled: bool,

    /// Enable query filter relaxation (temporary label exclusions disable)
    #[serde(default = "default_filter_relaxation_enabled")]
    pub filter_relaxation_enabled: bool,

    /// Enable bead state reset (48+ hour inactive beads)
    #[serde(default = "default_state_reset_enabled")]
    pub state_reset_enabled: bool,

    /// Stale threshold for dependency pruning (default: 24 hours)
    #[serde(default = "default_stale_threshold_hours")]
    pub stale_threshold_hours: i64,

    /// Inactive threshold for state reset (default: 48 hours)
    #[serde(default = "default_inactive_threshold_hours")]
    pub inactive_threshold_hours: i64,

    /// Enable dry-run mode (report only, no changes)
    #[serde(default = "default_dry_run")]
    pub dry_run: bool,
}

fn default_dependency_pruning_enabled() -> bool {
    true
}

fn default_assignee_clearing_enabled() -> bool {
    true
}

fn default_filter_relaxation_enabled() -> bool {
    true
}

fn default_state_reset_enabled() -> bool {
    true
}

fn default_stale_threshold_hours() -> i64 {
    24
}

fn default_inactive_threshold_hours() -> i64 {
    48
}

fn default_dry_run() -> bool {
    false
}

impl Default for CascadingRepairConfig {
    fn default() -> Self {
        Self {
            workspace_path: PathBuf::from("."),
            dependency_pruning_enabled: default_dependency_pruning_enabled(),
            assignee_clearing_enabled: default_assignee_clearing_enabled(),
            filter_relaxation_enabled: default_filter_relaxation_enabled(),
            state_reset_enabled: default_state_reset_enabled(),
            stale_threshold_hours: default_stale_threshold_hours(),
            inactive_threshold_hours: default_inactive_threshold_hours(),
            dry_run: default_dry_run(),
        }
    }
}

impl CascadingRepairConfig {
    /// Load configuration from environment variables
    pub fn from_environment() -> Self {
        let mut config = Self::default();

        if let Ok(path) = std::env::var("ICG_WORKSPACE_PATH") {
            config.workspace_path = PathBuf::from(path);
        }

        if let Ok(enabled) = std::env::var("ICG_DEPENDENCY_PRUNING_ENABLED") {
            config.dependency_pruning_enabled =
                enabled.eq_ignore_ascii_case("true") || enabled == "1";
        }

        if let Ok(enabled) = std::env::var("ICG_ASSIGNEE_CLEARING_ENABLED") {
            config.assignee_clearing_enabled =
                enabled.eq_ignore_ascii_case("true") || enabled == "1";
        }

        if let Ok(enabled) = std::env::var("ICG_FILTER_RELAXATION_ENABLED") {
            config.filter_relaxation_enabled =
                enabled.eq_ignore_ascii_case("true") || enabled == "1";
        }

        if let Ok(enabled) = std::env::var("ICG_STATE_RESET_ENABLED") {
            config.state_reset_enabled = enabled.eq_ignore_ascii_case("true") || enabled == "1";
        }

        if let Ok(hours) = std::env::var("ICG_STALE_THRESHOLD_HOURS") {
            if let Ok(h) = hours.parse::<i64>() {
                config.stale_threshold_hours = h.max(1);
            }
        }

        if let Ok(hours) = std::env::var("ICG_INACTIVE_THRESHOLD_HOURS") {
            if let Ok(h) = hours.parse::<i64>() {
                config.inactive_threshold_hours = h.max(1);
            }
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

    /// Get the path to the cascading repair log file
    pub fn repair_log_path(&self) -> PathBuf {
        self.diagnostics_dir().join("cascading-repair.jsonl")
    }

    /// Get the path to events.jsonl for logging repairs
    pub fn events_path(&self) -> PathBuf {
        self.workspace_path.join(".beads").join("events.jsonl")
    }
}

/// Result of a single repair strategy execution
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StrategyResult {
    /// Strategy name
    pub strategy_name: String,

    /// Timestamp when strategy was executed
    pub timestamp: DateTime<Utc>,

    /// Whether the strategy was enabled and executed
    pub executed: bool,

    /// Number of beads affected by this strategy
    pub beads_affected: usize,

    /// Actions performed (bead IDs and descriptions)
    pub actions: Vec<String>,

    /// Whether the strategy succeeded in making beads visible
    pub success: bool,

    /// Number of newly visible beads after this strategy
    pub newly_visible_beads: usize,

    /// Error message (if any)
    pub error: Option<String>,
}

/// Cascading repair execution report
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CascadingRepairReport {
    /// Timestamp when cascading repair was triggered
    pub timestamp: DateTime<Utc>,

    /// Ready bead count before cascading repair (should be 0)
    pub ready_beads_before: usize,

    /// Ready bead count after cascading repair
    pub ready_beads_after: usize,

    /// Total duration in seconds
    pub duration_seconds: f64,

    /// Individual strategy results
    pub strategies: Vec<StrategyResult>,

    /// Overall success status
    pub overall_success: bool,

    /// Recommended next steps if repair failed
    pub recommendations: Vec<String>,
}

/// Cascading repair service
pub struct CascadingRepairService {
    config: CascadingRepairConfig,
}

impl CascadingRepairService {
    /// Create a new cascading repair service with the given configuration
    pub fn new(config: CascadingRepairConfig) -> Self {
        Self { config }
    }

    /// Get the configuration
    pub fn config(&self) -> &CascadingRepairConfig {
        &self.config
    }

    /// Execute cascading repair strategies in sequence
    pub fn execute_cascading_repair(&mut self) -> Result<CascadingRepairReport> {
        let start_time = Utc::now();

        eprintln!("🚨 executing cascading repair strategies");
        eprintln!("📁 Workspace: {}", self.config.workspace_path.display());

        // Ensure diagnostics directory exists
        fs::create_dir_all(self.config.diagnostics_dir())
            .context("Failed to create diagnostics directory")?;

        // Get initial ready bead count
        let ready_beads_before = self.get_ready_bead_count()?;
        eprintln!(
            "📊 Ready beads before cascading repair: {}",
            ready_beads_before
        );

        let mut strategies = Vec::new();
        let mut ready_beads_count = ready_beads_before;

        // Strategy 1: Aggressive dependency pruning
        if self.config.dependency_pruning_enabled {
            eprintln!("🔧 Strategy 1: Aggressive dependency pruning");
            let result = self.execute_dependency_pruning(ready_beads_count)?;
            ready_beads_count = self.get_ready_bead_count()?;
            strategies.push(result);
            eprintln!("✅ Strategy 1 complete: {} ready beads", ready_beads_count);

            // If we have ready beads now, we can stop
            if ready_beads_count > 0 {
                eprintln!("🎉 Recovery successful after dependency pruning");
                return self.finalize_report(
                    start_time,
                    ready_beads_before,
                    ready_beads_count,
                    strategies,
                );
            }
        }

        // Strategy 2: Emergency assignee clearing
        if self.config.assignee_clearing_enabled {
            eprintln!("🔧 Strategy 2: Emergency assignee clearing");
            let result = self.execute_assignee_clearing(ready_beads_count)?;
            ready_beads_count = self.get_ready_bead_count()?;
            strategies.push(result);
            eprintln!("✅ Strategy 2 complete: {} ready beads", ready_beads_count);

            if ready_beads_count > 0 {
                eprintln!("🎉 Recovery successful after assignee clearing");
                return self.finalize_report(
                    start_time,
                    ready_beads_before,
                    ready_beads_count,
                    strategies,
                );
            }
        }

        // Strategy 3: Query filter relaxation
        if self.config.filter_relaxation_enabled {
            eprintln!("🔧 Strategy 3: Query filter relaxation");
            let result = self.execute_filter_relaxation(ready_beads_count)?;
            ready_beads_count = self.get_ready_bead_count()?;
            strategies.push(result);
            eprintln!("✅ Strategy 3 complete: {} ready beads", ready_beads_count);

            if ready_beads_count > 0 {
                eprintln!("🎉 Recovery successful after filter relaxation");
                return self.finalize_report(
                    start_time,
                    ready_beads_before,
                    ready_beads_count,
                    strategies,
                );
            }
        }

        // Strategy 4: Bead state reset
        if self.config.state_reset_enabled {
            eprintln!("🔧 Strategy 4: Bead state reset");
            let result = self.execute_state_reset(ready_beads_count)?;
            ready_beads_count = self.get_ready_bead_count()?;
            strategies.push(result);
            eprintln!("✅ Strategy 4 complete: {} ready beads", ready_beads_count);

            if ready_beads_count > 0 {
                eprintln!("🎉 Recovery successful after state reset");
                return self.finalize_report(
                    start_time,
                    ready_beads_before,
                    ready_beads_count,
                    strategies,
                );
            }
        }

        // All strategies exhausted - finalize report
        self.finalize_report(
            start_time,
            ready_beads_before,
            ready_beads_count,
            strategies,
        )
    }

    /// Finalize the cascading repair report
    fn finalize_report(
        &self,
        start_time: DateTime<Utc>,
        ready_beads_before: usize,
        ready_beads_after: usize,
        strategies: Vec<StrategyResult>,
    ) -> Result<CascadingRepairReport> {
        let end_time = Utc::now();
        let duration = end_time.signed_duration_since(start_time);
        let duration_seconds =
            duration.num_seconds() as f64 + duration.num_milliseconds() as f64 / 1000.0;

        let overall_success = ready_beads_after > 0;
        let mut recommendations = Vec::new();

        if !overall_success {
            recommendations.push(
                "Manual investigation required - all automatic repair strategies exhausted"
                    .to_string(),
            );
            recommendations
                .push("Check bead database integrity: bead doctor --rehearse".to_string());
            recommendations.push("Verify no systemic issues with the bead CLI version".to_string());
            recommendations
                .push("Consider NEEDLE worker restart if this is a transient issue".to_string());
        }

        let report = CascadingRepairReport {
            timestamp: start_time,
            ready_beads_before,
            ready_beads_after,
            duration_seconds,
            strategies,
            overall_success,
            recommendations,
        };

        // Publish report to log file
        self.publish_report(&report)?;

        // Log to events.jsonl
        self.log_cascading_repair_event(&report)?;

        Ok(report)
    }

    /// Execute Strategy 1: Aggressive dependency pruning
    fn execute_dependency_pruning(&self, ready_beads_before: usize) -> Result<StrategyResult> {
        let timestamp = Utc::now();
        let mut actions = Vec::new();
        let stale_threshold = Duration::hours(self.config.stale_threshold_hours);
        let cutoff_time = Utc::now() - stale_threshold;

        eprintln!(
            "  🔍 Finding stale blockers (not updated in >{} hours)",
            self.config.stale_threshold_hours
        );

        // Query for dependencies where blocker hasn't been updated in >24 hours
        let stale_blockers = self.find_stale_blockers(cutoff_time)?;

        if stale_blockers.is_empty() {
            eprintln!("  ℹ️  No stale blockers found");
            return Ok(StrategyResult {
                strategy_name: "dependency_pruning".to_string(),
                timestamp,
                executed: true,
                beads_affected: 0,
                actions,
                success: false,
                newly_visible_beads: 0,
                error: None,
            });
        }

        eprintln!("  🎯 Found {} stale blockers", stale_blockers.len());

        for (blocked_id, blocker_id, blocker_updated) in &stale_blockers {
            let action_desc = format!(
                "Removing dependency: {} blocked by {} (blocker last updated: {})",
                blocked_id, blocker_id, blocker_updated
            );

            eprintln!("    - {}", action_desc);
            actions.push(action_desc.clone());

            if !self.config.dry_run {
                // Execute the dependency removal
                match self.remove_dependency(blocked_id, blocker_id) {
                    Ok(_) => {
                        eprintln!("      ✅ Dependency removed");
                    }
                    Err(e) => {
                        eprintln!("      ❌ Failed to remove dependency: {}", e);
                        actions.push(format!("ERROR: {}", e));
                    }
                }
            } else {
                actions.push("[DRY RUN] Would remove dependency".to_string());
            }
        }

        // Check if we made beads visible
        let ready_beads_after = self.get_ready_bead_count()?;
        let newly_visible = ready_beads_after.saturating_sub(ready_beads_before);

        Ok(StrategyResult {
            strategy_name: "dependency_pruning".to_string(),
            timestamp,
            executed: true,
            beads_affected: stale_blockers.len(),
            actions,
            success: newly_visible > 0,
            newly_visible_beads: newly_visible,
            error: None,
        })
    }

    /// Execute Strategy 2: Emergency assignee clearing
    fn execute_assignee_clearing(&self, ready_beads_before: usize) -> Result<StrategyResult> {
        let timestamp = Utc::now();
        let mut actions = Vec::new();

        eprintln!("  🔍 Finding beads with 'human' label");

        // Query for beads with 'human' label that have assignees
        let human_labeled_beads = self.find_human_labeled_assigned_beads()?;

        if human_labeled_beads.is_empty() {
            eprintln!("  ℹ️  No human-labeled assigned beads found");
            return Ok(StrategyResult {
                strategy_name: "assignee_clearing".to_string(),
                timestamp,
                executed: true,
                beads_affected: 0,
                actions,
                success: false,
                newly_visible_beads: 0,
                error: None,
            });
        }

        eprintln!(
            "  🎯 Found {} human-labeled assigned beads",
            human_labeled_beads.len()
        );

        for (bead_id, _title, assignee) in &human_labeled_beads {
            let action_desc = format!(
                "Clearing assignee '{}' from human-labeled bead '{}'",
                assignee, bead_id
            );

            eprintln!("    - [{}] {}", bead_id, action_desc);
            actions.push(action_desc.clone());

            if !self.config.dry_run {
                match self.clear_assignee(bead_id) {
                    Ok(_) => {
                        eprintln!("      ✅ Assignee cleared");
                    }
                    Err(e) => {
                        eprintln!("      ❌ Failed to clear assignee: {}", e);
                        actions.push(format!("ERROR: {}", e));
                    }
                }
            } else {
                actions.push("[DRY RUN] Would clear assignee".to_string());
            }
        }

        // Check if we made beads visible
        let ready_beads_after = self.get_ready_bead_count()?;
        let newly_visible = ready_beads_after.saturating_sub(ready_beads_before);

        Ok(StrategyResult {
            strategy_name: "assignee_clearing".to_string(),
            timestamp,
            executed: true,
            beads_affected: human_labeled_beads.len(),
            actions,
            success: newly_visible > 0,
            newly_visible_beads: newly_visible,
            error: None,
        })
    }

    /// Execute Strategy 3: Query filter relaxation
    fn execute_filter_relaxation(&self, ready_beads_before: usize) -> Result<StrategyResult> {
        let timestamp = Utc::now();
        let mut actions = Vec::new();

        eprintln!("  🔍 Finding beads excluded by label filters");

        // Query for all open/in_progress beads to see what's being filtered out
        let all_open_beads = self.query_all_open_beads()?;
        let ready_bead_ids = self.get_ready_bead_ids()?;

        // Find beads that are open but not in ready frontier
        let excluded_beads: Vec<_> = all_open_beads
            .into_iter()
            .filter(|(id, _, _)| !ready_bead_ids.contains(id))
            .collect();

        if excluded_beads.is_empty() {
            eprintln!("  ℹ️  No beads excluded by label filters");
            return Ok(StrategyResult {
                strategy_name: "filter_relaxation".to_string(),
                timestamp,
                executed: true,
                beads_affected: 0,
                actions,
                success: false,
                newly_visible_beads: 0,
                error: None,
            });
        }

        eprintln!(
            "  🎯 Found {} beads excluded by filters",
            excluded_beads.len()
        );

        for (bead_id, title, labels) in &excluded_beads {
            let action_desc = format!(
                "Bead '{}' ({}) excluded by labels: {:?}",
                bead_id, title, labels
            );

            eprintln!("    - {}", action_desc);
            actions.push(action_desc);

            // For filter relaxation, we log the issue but don't automatically change labels
            // This is diagnostic information for manual intervention
            actions.push(format!(
                "RECOMMENDATION: Review labels on bead '{}'. Consider removing exclusionary labels.",
                bead_id
            ));
        }

        // Filter relaxation is primarily diagnostic - we don't automatically change labels
        // as that could have unintended consequences
        let ready_beads_after = self.get_ready_bead_count()?;
        let newly_visible = ready_beads_after.saturating_sub(ready_beads_before);

        Ok(StrategyResult {
            strategy_name: "filter_relaxation".to_string(),
            timestamp,
            executed: true,
            beads_affected: excluded_beads.len(),
            actions,
            success: false, // This strategy is diagnostic only
            newly_visible_beads: newly_visible,
            error: None,
        })
    }

    /// Execute Strategy 4: Bead state reset
    fn execute_state_reset(&self, ready_beads_before: usize) -> Result<StrategyResult> {
        let timestamp = Utc::now();
        let mut actions = Vec::new();
        let inactive_threshold = Duration::hours(self.config.inactive_threshold_hours);
        let cutoff_time = Utc::now() - inactive_threshold;

        eprintln!(
            "  🔍 Finding inactive beads (no activity in >{} hours)",
            self.config.inactive_threshold_hours
        );

        // Query for beads that haven't been updated in >48 hours
        let inactive_beads = self.find_inactive_beads(cutoff_time)?;

        if inactive_beads.is_empty() {
            eprintln!("  ℹ️  No inactive beads found");
            return Ok(StrategyResult {
                strategy_name: "state_reset".to_string(),
                timestamp,
                executed: true,
                beads_affected: 0,
                actions,
                success: false,
                newly_visible_beads: 0,
                error: None,
            });
        }

        eprintln!("  🎯 Found {} inactive beads", inactive_beads.len());

        for (bead_id, _title, status, assignee, updated) in &inactive_beads {
            let action_desc = format!(
                "Resetting bead '{}' (status: {}, assignee: {:?}, last updated: {})",
                bead_id, status, assignee, updated
            );

            eprintln!("    - {}", action_desc);
            actions.push(action_desc.clone());

            if !self.config.dry_run {
                match self.reset_bead_state(bead_id) {
                    Ok(_) => {
                        eprintln!("      ✅ Bead state reset");
                    }
                    Err(e) => {
                        eprintln!("      ❌ Failed to reset bead state: {}", e);
                        actions.push(format!("ERROR: {}", e));
                    }
                }
            } else {
                actions.push("[DRY RUN] Would reset bead state".to_string());
            }
        }

        // Check if we made beads visible
        let ready_beads_after = self.get_ready_bead_count()?;
        let newly_visible = ready_beads_after.saturating_sub(ready_beads_before);

        Ok(StrategyResult {
            strategy_name: "state_reset".to_string(),
            timestamp,
            executed: true,
            beads_affected: inactive_beads.len(),
            actions,
            success: newly_visible > 0,
            newly_visible_beads: newly_visible,
            error: None,
        })
    }

    /// Find stale blockers (dependencies where blocker hasn't been updated in >24 hours)
    fn find_stale_blockers(
        &self,
        cutoff_time: DateTime<Utc>,
    ) -> Result<Vec<(String, String, String)>> {
        let db_path = self.config.beads_db_path();

        let query = format!(
            "SELECT d.blocked_issue_id, d.blocker_issue_id, i.updated_at
             FROM dependencies d
             JOIN issues i ON d.blocker_issue_id = i.id
             WHERE d.kind = 'blocks'
             AND datetime(i.updated_at) < datetime('{}')
             AND i.base_status IN ('open', 'in_progress')",
            cutoff_time.format("%Y-%m-%d %H:%M:%S")
        );

        let output = Command::new("sqlite3")
            .arg(&db_path)
            .arg(&query)
            .output()
            .context("Failed to query stale blockers")?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(anyhow::anyhow!("Database query failed: {}", stderr));
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        let mut stale_blockers = Vec::new();

        for line in stdout.lines() {
            if line.is_empty() {
                continue;
            }

            let parts: Vec<&str> = line.split('|').collect();
            if parts.len() >= 3 {
                stale_blockers.push((
                    parts[0].to_string(),
                    parts[1].to_string(),
                    parts[2].to_string(),
                ));
            }
        }

        Ok(stale_blockers)
    }

    /// Find beads with 'human' label that have assignees
    fn find_human_labeled_assigned_beads(&self) -> Result<Vec<(String, String, String)>> {
        let db_path = self.config.beads_db_path();

        let query = "
            SELECT DISTINCT i.id, i.title, i.assignee
            FROM issues i
            JOIN labels l ON i.id = l.issue_id
            WHERE l.name = 'human'
            AND i.assignee IS NOT NULL
            AND i.assignee != ''
            AND i.base_status IN ('open', 'in_progress')
        ";

        let output = Command::new("sqlite3")
            .arg(&db_path)
            .arg(query)
            .output()
            .context("Failed to query human-labeled beads")?;

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
            if parts.len() >= 3 {
                beads.push((
                    parts[0].to_string(),
                    parts[1].to_string(),
                    parts[2].to_string(),
                ));
            }
        }

        Ok(beads)
    }

    /// Query all open/in_progress beads
    fn query_all_open_beads(&self) -> Result<Vec<(String, String, Vec<String>)>> {
        let db_path = self.config.beads_db_path();

        let query = "
            SELECT i.id, i.title, i.base_status
            FROM issues i
            WHERE i.base_status IN ('open', 'in_progress')
            ORDER BY i.priority DESC, i.created_at ASC
        ";

        let output = Command::new("sqlite3")
            .arg(&db_path)
            .arg(query)
            .output()
            .context("Failed to query open beads")?;

        if !output.status.success() {
            return Ok(Vec::new());
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        let mut beads = Vec::new();

        for line in stdout.lines() {
            if line.is_empty() {
                continue;
            }

            let parts: Vec<&str> = line.split('|').collect();
            if parts.len() >= 3 {
                let bead_id = parts[0].to_string();
                let title = parts[1].to_string();

                // Get labels for this bead
                let labels = self.get_bead_labels(&bead_id)?;
                beads.push((bead_id, title, labels));
            }
        }

        Ok(beads)
    }

    /// Find inactive beads (no activity in >48 hours)
    fn find_inactive_beads(&self, cutoff_time: DateTime<Utc>) -> Result<Vec<InactiveBeadRow>> {
        let db_path = self.config.beads_db_path();

        let query = format!(
            "SELECT id, title, base_status, assignee, updated_at
             FROM issues
             WHERE datetime(updated_at) < datetime('{}')
             AND base_status IN ('open', 'in_progress')",
            cutoff_time.format("%Y-%m-%d %H:%M:%S")
        );

        let output = Command::new("sqlite3")
            .arg(&db_path)
            .arg(&query)
            .output()
            .context("Failed to query inactive beads")?;

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
            if parts.len() >= 5 {
                let assignee = if parts[3].is_empty() {
                    None
                } else {
                    Some(parts[3].to_string())
                };
                beads.push((
                    parts[0].to_string(),
                    parts[1].to_string(),
                    parts[2].to_string(),
                    assignee,
                    parts[4].to_string(),
                ));
            }
        }

        Ok(beads)
    }

    /// Remove a dependency relationship
    fn remove_dependency(&self, blocked_id: &str, blocker_id: &str) -> Result<()> {
        let output = Command::new("bead")
            .args([
                "dep",
                "remove",
                "--blocked",
                blocked_id,
                "--blocking",
                blocker_id,
            ])
            .current_dir(&self.config.workspace_path)
            .output()
            .context("Failed to run bead dep remove")?;

        if !output.status.success() {
            anyhow::bail!(
                "bead dep remove failed with exit code: {:?}, stderr: {}",
                output.status.code(),
                String::from_utf8_lossy(&output.stderr)
            );
        }

        Ok(())
    }

    /// Clear the assignee from a bead
    fn clear_assignee(&self, bead_id: &str) -> Result<()> {
        let output = Command::new("bead")
            .args([
                "update",
                bead_id,
                "--clear-assignee",
                "--notes",
                &format!(
                    "Emergency assignee clearing via cascading repair (at {})",
                    Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
                ),
            ])
            .current_dir(&self.config.workspace_path)
            .output()
            .context("Failed to run bead update --clear-assignee")?;

        if !output.status.success() {
            anyhow::bail!(
                "bead update --clear-assignee failed with exit code: {:?}, stderr: {}",
                output.status.code(),
                String::from_utf8_lossy(&output.stderr)
            );
        }

        Ok(())
    }

    /// Reset a bead to open/unassigned state
    fn reset_bead_state(&self, bead_id: &str) -> Result<()> {
        let output = Command::new("bead")
            .args([
                "update",
                bead_id,
                "--status",
                "open",
                "--clear-assignee",
                "--notes",
                &format!(
                    "Emergency state reset via cascading repair (at {})",
                    Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
                ),
            ])
            .current_dir(&self.config.workspace_path)
            .output()
            .context("Failed to run bead update for state reset")?;

        if !output.status.success() {
            anyhow::bail!(
                "bead update for state reset failed with exit code: {:?}, stderr: {}",
                output.status.code(),
                String::from_utf8_lossy(&output.stderr)
            );
        }

        Ok(())
    }

    /// Get the current count of ready beads
    fn get_ready_bead_count(&self) -> Result<usize> {
        let ready_beads = self.get_ready_bead_ids()?;
        Ok(ready_beads.len())
    }

    /// Get the set of ready bead IDs
    fn get_ready_bead_ids(&self) -> Result<HashSet<String>> {
        let output = Command::new("bead")
            .args(["list", "--ready", "--json"])
            .current_dir(&self.config.workspace_path)
            .output()
            .context("Failed to run bead list --ready")?;

        if !output.status.success() {
            return Ok(HashSet::new());
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        let mut ready_beads = HashSet::new();

        for line in stdout.lines() {
            if line.is_empty() || line.trim() == "[]" {
                continue;
            }

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

    /// Get labels for a bead
    fn get_bead_labels(&self, bead_id: &str) -> Result<Vec<String>> {
        let db_path = self.config.beads_db_path();

        let output = Command::new("sqlite3")
            .arg(&db_path)
            .arg(format!(
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

    /// Publish the cascading repair report to the JSONL file
    fn publish_report(&self, report: &CascadingRepairReport) -> Result<()> {
        let report_path = self.config.repair_log_path();
        let json_line =
            serde_json::to_string(report).context("Failed to serialize cascading repair report")?;

        let mut file = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&report_path)
            .context("Failed to open cascading repair log file")?;

        use std::io::Write;
        writeln!(file, "{}", json_line).context("Failed to write cascading repair report")?;

        eprintln!(
            "📝 Cascading repair report published to {}",
            report_path.display()
        );

        Ok(())
    }

    /// Log a cascading repair event to events.jsonl
    fn log_cascading_repair_event(&self, report: &CascadingRepairReport) -> Result<()> {
        let event = serde_json::json!({
            "issue_id": "cascading-repair",
            "kind": "cascading_repair_execution",
            "actor": "icg-cascading-repair-service",
            "time": report.timestamp.to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
            "detail": {
                "ready_beads_before": report.ready_beads_before,
                "ready_beads_after": report.ready_beads_after,
                "duration_seconds": report.duration_seconds,
                "strategies_executed": report.strategies.len(),
                "overall_success": report.overall_success,
                "strategies": report.strategies,
            }
        });

        let mut file = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(self.config.events_path())
            .context("Failed to open events.jsonl for writing")?;

        use std::io::Write;
        writeln!(file, "{}", event)
            .context("Failed to write cascading repair event to events.jsonl")?;

        eprintln!(
            "📊 Cascading repair event logged to {}",
            self.config.events_path().display()
        );

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = CascadingRepairConfig::default();
        assert!(config.dependency_pruning_enabled);
        assert!(config.assignee_clearing_enabled);
        assert!(config.filter_relaxation_enabled);
        assert!(config.state_reset_enabled);
        assert_eq!(config.stale_threshold_hours, 24);
        assert_eq!(config.inactive_threshold_hours, 48);
        assert!(!config.dry_run);
    }

    #[test]
    fn test_strategy_result_serialization() {
        let result = StrategyResult {
            strategy_name: "test_strategy".to_string(),
            timestamp: Utc::now(),
            executed: true,
            beads_affected: 5,
            actions: vec!["action 1".to_string(), "action 2".to_string()],
            success: true,
            newly_visible_beads: 3,
            error: None,
        };

        let json = serde_json::to_string(&result).unwrap();
        assert!(json.contains("test_strategy"));
        assert!(json.contains("action 1"));
        assert!(json.contains("newly_visible_beads"));
    }

    #[test]
    fn test_cascading_repair_report_serialization() {
        let report = CascadingRepairReport {
            timestamp: Utc::now(),
            ready_beads_before: 0,
            ready_beads_after: 5,
            duration_seconds: 10.5,
            strategies: vec![],
            overall_success: true,
            recommendations: vec![],
        };

        let json = serde_json::to_string(&report).unwrap();
        assert!(json.contains("ready_beads_before"));
        assert!(json.contains("ready_beads_after"));
        assert!(json.contains("overall_success"));
    }
}
