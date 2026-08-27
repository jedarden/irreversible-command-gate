//! Pluck Query Debugger
//!
//! SQL-level diagnostic tool that replays Pluck's ready frontier query with
//! progressive filter relaxation to diagnose exactly which filters are excluding
//! beads from the ready frontier.
//!
//! ## Problem
//!
//! When `bead list --ready` returns no results despite open beads existing,
//! the root cause is often one or more filters in Pluck's query being too
//! restrictive. This tool makes the invisible visible by systematically relaxing
//! each filter and reporting which beads appear at each level.
//!
//! ## Diagnostic Approach
//!
//! The debugger runs the ready frontier query at 5 relaxation levels:
//!
//! **Level 0: Exact Pluck Query** - All filters applied (label exclusions,
//! dependency checks, assignee filters, manual blocks)
//!
//! **Level 1: Without Label Exclusions** - Remove label-based filters
//! (e.g., excluding beads with specific labels like "deprecated" or "blocked")
//!
//! **Level 2: Without Dependency Checks** - Remove JOIN to dependencies table
//! (beads with unresolved dependencies become visible)
//!
//! **Level 3: Without Assignee Filters** - Remove `assignee IS NULL` check
//! (assigned beads become visible)
//!
//! **Level 4: Raw Base Query** - Only `base_status IN ('open', 'in_progress')`
//! (all open/in_progress beads visible, no other filters)
//!
//! For each level, the debugger records:
//! - Which beads are visible
//! - Which beads are newly visible compared to the previous level
//! - Which filter was relaxed to make them visible
//!
//! ## Output
//!
//! Generates a structured report at `.beads/diagnostics/pluck-query-debug-report.jsonl`
//! containing:
//! - Per-level query results with SQL used
//! - Per-bead breakdown of which level made it visible and which filter excluded it
//! - Pattern analysis (e.g., "all excluded beads have label 'deprecated'")
//! - Recommended fixes to Pluck query logic if a pattern is detected
//!
//! ## Usage
//!
//! ```bash
//! # Run the debugger
//! cargo run --bin pluck-query-debugger
//!
//! # With custom database path
//! cargo run --bin pluck-query-debugger -- --db-path /path/to/.beads/beads.db
//!
//! # Generate human-readable summary
//! cargo run --bin pluck-query-debugger -- --summary
//! ```

use anyhow::{anyhow, Context, Result};
use chrono::{DateTime, Utc};
use rusqlite::{Connection, params};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Query relaxation level
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum QueryLevel {
    /// Level 0: Exact Pluck query (all filters)
    Exact = 0,
    /// Level 1: Without label exclusions
    WithoutLabels = 1,
    /// Level 2: Without dependency checks
    WithoutDependencies = 2,
    /// Level 3: Without assignee filter
    WithoutAssignee = 3,
    /// Level 4: Raw base query (status only)
    Raw = 4,
}

impl QueryLevel {
    /// Get all levels in order
    pub fn all_levels() -> Vec<QueryLevel> {
        vec![
            QueryLevel::Exact,
            QueryLevel::WithoutLabels,
            QueryLevel::WithoutDependencies,
            QueryLevel::WithoutAssignee,
            QueryLevel::Raw,
        ]
    }

    /// Get description of this level
    pub fn description(&self) -> &str {
        match self {
            QueryLevel::Exact => "Exact Pluck query (all filters applied)",
            QueryLevel::WithoutLabels => "Without label exclusions",
            QueryLevel::WithoutDependencies => "Without dependency checks",
            QueryLevel::WithoutAssignee => "Without assignee filter",
            QueryLevel::Raw => "Raw base query (status only)",
        }
    }

    /// Get which filter was relaxed at this level
    pub fn relaxed_filter(&self) -> Option<&str> {
        match self {
            QueryLevel::Exact => None,
            QueryLevel::WithoutLabels => Some("label exclusions"),
            QueryLevel::WithoutDependencies => Some("dependency checks"),
            QueryLevel::WithoutAssignee => Some("assignee filter"),
            QueryLevel::Raw => Some("all filters except status"),
        }
    }
}

/// Configuration for the Pluck query debugger
#[derive(Debug, Clone)]
pub struct PluckQueryDebuggerConfig {
    /// Path to the beads.db SQLite database
    pub db_path: PathBuf,
    /// Path to the diagnostics output directory
    pub diagnostics_dir: PathBuf,
    /// If true, print human-readable summary to stdout
    pub print_summary: bool,
}

impl Default for PluckQueryDebuggerConfig {
    fn default() -> Self {
        let workspace_path = std::env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."));
        Self {
            db_path: workspace_path.join(".beads/beads.db"),
            diagnostics_dir: workspace_path.join(".beads/diagnostics"),
            print_summary: false,
        }
    }
}

/// A bead with its state from the database
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BeadRecord {
    pub id: String,
    pub title: String,
    pub base_status: String,
    pub assignee: Option<String>,
    pub manual_blocked: bool,
    pub priority: i32,
    pub created_at: String,
    pub updated_at: String,
}

/// Query result at a specific relaxation level
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LevelResult {
    /// The relaxation level
    pub level: QueryLevel,
    /// Description of the level
    pub level_description: String,
    /// SQL query used at this level
    pub sql_query: String,
    /// Beads visible at this level
    pub visible_beads: Vec<BeadRecord>,
    /// Beads that became visible at this level (compared to previous level)
    pub newly_visible_beads: Vec<String>,
    /// Count of beads excluded by the filter relaxed at this level
    pub excluded_count: usize,
}

/// Per-bead exclusion analysis
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BeadExclusionAnalysis {
    /// Bead ID
    pub bead_id: String,
    /// Bead title
    pub bead_title: String,
    /// Current status
    pub status: String,
    /// Assignee (if any)
    pub assignee: Option<String>,
    /// Whether manually blocked
    pub manual_blocked: bool,
    /// Labels on this bead
    pub labels: Vec<String>,
    /// Dependencies blocking this bead
    pub blocking_dependencies: Vec<String>,
    /// The level at which this bead became visible
    pub visible_at_level: Option<QueryLevel>,
    /// Which filter excluded this bead (if known)
    pub excluded_by_filter: Option<String>,
    /// Detailed explanation of why this bead was excluded
    pub exclusion_reason: String,
}

/// Pattern analysis of excluded beads
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExclusionPattern {
    /// The pattern detected
    pub pattern: String,
    /// Description of the pattern
    pub description: String,
    /// Beads matching this pattern
    pub matching_beads: Vec<String>,
    /// Suggested fix to Pluck query (if applicable)
    pub suggested_fix: Option<String>,
}

/// Complete Pluck query debug report
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluckQueryDebugReport {
    /// Timestamp when the report was generated
    pub timestamp: DateTime<Utc>,
    /// Database path
    pub db_path: String,
    /// Total open/in_progress beads in database
    pub total_open_beads: usize,
    /// Beads visible at exact Pluck level (Level 0)
    pub ready_frontier_count: usize,
    /// Starvation detected (ready frontier empty but open beads exist)
    pub starvation_detected: bool,
    /// Query results at each relaxation level
    pub level_results: Vec<LevelResult>,
    /// Per-bead exclusion analysis
    pub bead_analyses: Vec<BeadExclusionAnalysis>,
    /// Pattern analysis
    pub exclusion_patterns: Vec<ExclusionPattern>,
    /// Recommended fixes
    pub recommended_fixes: Vec<String>,
}

/// Pluck query debugger
pub struct PluckQueryDebugger {
    config: PluckQueryDebuggerConfig,
}

impl PluckQueryDebugger {
    /// Create a new debugger with default config
    pub fn new() -> Result<Self> {
        Self::with_config(PluckQueryDebuggerConfig::default())
    }

    /// Create a new debugger with custom config
    pub fn with_config(config: PluckQueryDebuggerConfig) -> Result<Self> {
        if !config.db_path.exists() {
            return Err(anyhow!(
                "Database not found at {}",
                config.db_path.display()
            ));
        }
        Ok(Self { config })
    }

    /// Run the complete query debugging analysis
    pub fn run_debug_analysis(&mut self) -> Result<PluckQueryDebugReport> {
        let timestamp = Utc::now();

        // Ensure diagnostics directory exists
        fs::create_dir_all(&self.config.diagnostics_dir)
            .context("Failed to create diagnostics directory")?;

        // Get total open bead count
        let total_open_beads = self.count_total_open_beads()?;

        // Run queries at each relaxation level
        let level_results = self.run_all_level_queries()?;

        // The ready frontier count is Level 0 (Exact)
        let ready_frontier_count = level_results
            .first()
            .map(|r| r.visible_beads.len())
            .unwrap_or(0);

        // Starvation exists when there are open beads that are invisible from the ready frontier
        // If total_open_beads == 0, the frontier is empty - not starved
        let starvation_detected = total_open_beads > 0 && ready_frontier_count == 0;

        // Perform per-bead exclusion analysis
        let bead_analyses = self.analyze_bead_exclusions(&level_results)?;

        // Analyze patterns in excluded beads
        let exclusion_patterns = self.analyze_exclusion_patterns(&bead_analyses);

        // Generate recommended fixes
        let recommended_fixes = self.generate_fixes(&exclusion_patterns);

        let report = PluckQueryDebugReport {
            timestamp,
            db_path: self.config.db_path.display().to_string(),
            total_open_beads,
            ready_frontier_count,
            starvation_detected,
            level_results,
            bead_analyses,
            exclusion_patterns,
            recommended_fixes,
        };

        // Publish report
        self.publish_report(&report)?;

        // Automatically file diagnostic bead if starvation detected with actionable findings
        if starvation_detected && self.has_actionable_findings(&report) {
            self.file_diagnostic_bead(&report)?;
        }

        Ok(report)
    }

    /// Count total open beads in database (excludes in_progress beads)
    ///
    /// Only counts 'open' beads because starvation is about open beads not
    /// appearing in the ready frontier. in_progress beads are correctly
    /// excluded from the ready frontier (they're being worked on), so they
    /// should NOT trigger starvation alerts.
    fn count_total_open_beads(&self) -> Result<usize> {
        let conn = Connection::open(&self.config.db_path)
            .context("Failed to open database")?;

        let count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM issues WHERE base_status = 'open'",
            [],
            |row| row.get(0),
        )?;

        Ok(count as usize)
    }

    /// Run queries at all relaxation levels
    fn run_all_level_queries(&self) -> Result<Vec<LevelResult>> {
        let mut results = Vec::new();
        let mut previous_visible: HashSet<String> = HashSet::new();

        for level in QueryLevel::all_levels() {
            let sql_query = self.build_query_for_level(level);
            let visible_beads = self.execute_query(&sql_query)?;

            // Calculate newly visible beads
            let current_visible: HashSet<String> = visible_beads
                .iter()
                .map(|b| b.id.clone())
                .collect();

            let newly_visible_beads: Vec<String> = current_visible
                .difference(&previous_visible)
                .cloned()
                .collect();

            let excluded_count = if level == QueryLevel::Exact {
                0 // Exact level is baseline
            } else {
                newly_visible_beads.len()
            };

            results.push(LevelResult {
                level,
                level_description: level.description().to_string(),
                sql_query: sql_query.clone(),
                visible_beads,
                newly_visible_beads,
                excluded_count,
            });

            previous_visible = current_visible;
        }

        Ok(results)
    }

    /// Build the SQL query for a given relaxation level
    fn build_query_for_level(&self, level: QueryLevel) -> String {
        match level {
            QueryLevel::Exact => {
                // Exact Pluck query: all filters applied
                // This mimics what `bead list --ready` does
                r#"
                    SELECT DISTINCT
                        i.id, i.title, i.base_status, i.assignee,
                        i.manual_blocked, i.priority, i.created_at, i.updated_at
                    FROM issues i
                    WHERE i.base_status = 'open'
                      AND i.assignee IS NULL
                      AND i.manual_blocked = 0
                      AND NOT EXISTS (
                          SELECT 1 FROM dependencies d
                          WHERE d.blocked_issue_id = i.id
                            AND d.kind = 'blocks'
                      )
                      AND NOT EXISTS (
                          SELECT 1 FROM labels l
                          WHERE l.issue_id = i.id
                            AND l.label IN ('deprecated', 'blocked', 'on-hold')
                      )
                    ORDER BY i.priority DESC, i.created_at ASC
                "#.to_string()
            }
            QueryLevel::WithoutLabels => {
                // Without label exclusions
                r#"
                    SELECT DISTINCT
                        i.id, i.title, i.base_status, i.assignee,
                        i.manual_blocked, i.priority, i.created_at, i.updated_at
                    FROM issues i
                    WHERE i.base_status = 'open'
                      AND i.assignee IS NULL
                      AND i.manual_blocked = 0
                      AND NOT EXISTS (
                          SELECT 1 FROM dependencies d
                          WHERE d.blocked_issue_id = i.id
                            AND d.kind = 'blocks'
                      )
                    ORDER BY i.priority DESC, i.created_at ASC
                "#.to_string()
            }
            QueryLevel::WithoutDependencies => {
                // Without dependency checks
                r#"
                    SELECT DISTINCT
                        i.id, i.title, i.base_status, i.assignee,
                        i.manual_blocked, i.priority, i.created_at, i.updated_at
                    FROM issues i
                    WHERE i.base_status = 'open'
                      AND i.assignee IS NULL
                      AND i.manual_blocked = 0
                      AND NOT EXISTS (
                          SELECT 1 FROM labels l
                          WHERE l.issue_id = i.id
                            AND l.label IN ('deprecated', 'blocked', 'on-hold')
                      )
                    ORDER BY i.priority DESC, i.created_at ASC
                "#.to_string()
            }
            QueryLevel::WithoutAssignee => {
                // Without assignee filter
                r#"
                    SELECT DISTINCT
                        i.id, i.title, i.base_status, i.assignee,
                        i.manual_blocked, i.priority, i.created_at, i.updated_at
                    FROM issues i
                    WHERE i.base_status = 'open'
                      AND i.manual_blocked = 0
                      AND NOT EXISTS (
                          SELECT 1 FROM labels l
                          WHERE l.issue_id = i.id
                            AND l.label IN ('deprecated', 'blocked', 'on-hold')
                      )
                    ORDER BY i.priority DESC, i.created_at ASC
                "#.to_string()
            }
            QueryLevel::Raw => {
                // Raw base query - only status filter
                r#"
                    SELECT
                        i.id, i.title, i.base_status, i.assignee,
                        i.manual_blocked, i.priority, i.created_at, i.updated_at
                    FROM issues i
                    WHERE i.base_status IN ('open', 'in_progress')
                    ORDER BY i.priority DESC, i.created_at ASC
                "#.to_string()
            }
        }
    }

    /// Execute a query and return the bead records
    fn execute_query(&self, sql_query: &str) -> Result<Vec<BeadRecord>> {
        let conn = Connection::open(&self.config.db_path)
            .context("Failed to open database")?;

        let mut stmt = conn.prepare(sql_query)
            .context("Failed to prepare query")?;

        let beads = stmt.query_and_then([], |row| {
            let id: String = row.get(0)?;
            let title: String = row.get(1)?;
            let base_status: String = row.get(2)?;
            let assignee: Option<String> = row.get(3)?;
            let manual_blocked: i32 = row.get(4)?;
            let priority: i32 = row.get(5)?;
            let created_at: String = row.get(6)?;
            let updated_at: String = row.get(7)?;

            Ok(BeadRecord {
                id,
                title,
                base_status,
                assignee,
                manual_blocked: manual_blocked == 1,
                priority,
                created_at,
                updated_at,
            })
        })?
        .collect::<Result<Vec<_>>>()
        .context("Failed to read bead records")?;

        Ok(beads)
    }

    /// Analyze per-bead exclusion reasons
    fn analyze_bead_exclusions(&self, level_results: &[LevelResult]) -> Result<Vec<BeadExclusionAnalysis>> {
        let mut analyses = Vec::new();

        // Get all beads from the raw level (Level 4) - clone to avoid lifetime issues
        let all_beads: Vec<BeadRecord> = level_results
            .iter()
            .find(|r| r.level == QueryLevel::Raw)
            .map(|r| r.visible_beads.clone())
            .unwrap_or_default();

        // Track which level each bead became visible at
        let mut bead_visibility_level: HashMap<String, QueryLevel> = HashMap::new();

        for result in level_results {
            for bead in &result.visible_beads {
                bead_visibility_level
                    .entry(bead.id.clone())
                    .or_insert(result.level);
            }
        }

        // Build lookup of newly visible beads per level
        let mut newly_visible_at_level: HashMap<QueryLevel, HashSet<String>> = HashMap::new();
        for result in level_results {
            newly_visible_at_level
                .entry(result.level)
                .or_insert_with(HashSet::new)
                .extend(result.newly_visible_beads.iter().cloned());
        }

        // Analyze each bead
        for bead in all_beads {
            let visible_at_level = bead_visibility_level.get(&bead.id).copied();

            // Get additional bead data
            let labels = self.get_bead_labels(&bead.id)?;
            let blocking_dependencies = self.get_blocking_dependencies(&bead.id);

            // Determine which filter excluded this bead
            let (excluded_by_filter, exclusion_reason) = if let Some(level) = visible_at_level {
                if level == QueryLevel::Exact {
                    (None, "Visible in ready frontier".to_string())
                } else {
                    // This bead became visible at a relaxed level
                    // Determine which filter excluded it
                    Self::determine_exclusion_filter(
                        &bead,
                        &labels,
                        &blocking_dependencies,
                        level,
                    )
                }
            } else {
                (None, "Never visible (should not happen at Raw level)".to_string())
            };

            analyses.push(BeadExclusionAnalysis {
                bead_id: bead.id.clone(),
                bead_title: bead.title.clone(),
                status: bead.base_status.clone(),
                assignee: bead.assignee.clone(),
                manual_blocked: bead.manual_blocked,
                labels,
                blocking_dependencies,
                visible_at_level,
                excluded_by_filter: excluded_by_filter.map(|s| s.to_string()),
                exclusion_reason,
            });
        }

        Ok(analyses)
    }

    /// Determine which filter excluded a bead
    fn determine_exclusion_filter(
        bead: &BeadRecord,
        labels: &[String],
        blocking_dependencies: &[String],
        visible_at_level: QueryLevel,
    ) -> (Option<&'static str>, String) {
        match visible_at_level {
            QueryLevel::Exact => {
                (None, "Visible in ready frontier".to_string())
            }
            QueryLevel::WithoutLabels => {
                // Became visible when label filter was removed
                let excluded_labels: Vec<&str> = labels
                    .iter()
                    .filter(|l| ["deprecated", "blocked", "on-hold"].contains(&l.as_str()))
                    .map(|s| s.as_str())
                    .collect();

                let reason = if excluded_labels.is_empty() {
                    "Excluded by label filter (no standard exclusion labels found - may have custom label logic)"
                } else {
                    "Excluded by label filter"
                };

                (Some("label exclusions"), format!("{}: has labels {:?}", reason, excluded_labels))
            }
            QueryLevel::WithoutDependencies => {
                // Became visible when dependency filter was removed
                if blocking_dependencies.is_empty() {
                    (Some("dependency checks"), "Excluded by dependency filter (no blocking dependencies found - may have transitive dependency issue)".to_string())
                } else {
                    (Some("dependency checks"), format!("Excluded by dependency filter: blocked by {:?}", blocking_dependencies))
                }
            }
            QueryLevel::WithoutAssignee => {
                // Became visible when assignee filter was removed
                if let Some(ref assignee) = bead.assignee {
                    (Some("assignee filter"), format!("Excluded by assignee filter: assigned to '{}'", assignee))
                } else {
                    (Some("assignee filter"), "Excluded by assignee filter (no assignee found - unexpected)".to_string())
                }
            }
            QueryLevel::Raw => {
                // Became visible at Raw level - this means the status filter was relaxed
                // Levels 0-3 only query for base_status = 'open', but Raw includes 'in_progress'
                if bead.base_status == "in_progress" {
                    (Some("status filter"), "Excluded by status filter: bead status is 'in_progress' (only 'open' beads appear in ready frontier)".to_string())
                } else if bead.assignee.is_some() {
                    // Assigned open beads also become visible here if they have other blocking factors
                    (Some("assignee filter"), format!("Excluded by assignee filter: assigned to '{}'", bead.assignee.as_ref().unwrap()))
                } else {
                    (Some("unknown"), "Excluded by unknown filter".to_string())
                }
            }
        }
    }

    /// Get labels for a bead
    fn get_bead_labels(&self, bead_id: &str) -> Result<Vec<String>> {
        let conn = Connection::open(&self.config.db_path)
            .context("Failed to open database")?;

        let mut stmt = conn.prepare(
            "SELECT label FROM labels WHERE issue_id = ?1"
        )?;

        let labels = stmt.query_and_then(params![bead_id], |row| {
            let name: String = row.get(0)?;
            Ok(name)
        })?
        .collect::<Result<Vec<_>>>()?;

        Ok(labels)
    }

    /// Get blocking dependencies for a bead
    fn get_blocking_dependencies(&self, bead_id: &str) -> Vec<String> {
        let conn = match Connection::open(&self.config.db_path) {
            Ok(conn) => conn,
            Err(_) => return Vec::new(),
        };

        let mut stmt = match conn.prepare(
            "SELECT blocker_issue_id FROM dependencies
             WHERE blocked_issue_id = ?1 AND kind = 'blocks'"
        ) {
            Ok(stmt) => stmt,
            Err(_) => return Vec::new(),
        };

        // Collect the results immediately to avoid lifetime issues
        let x = match stmt.query_and_then(params![bead_id], |row| {
            let blocker: String = row.get(0)?;
            Ok::<String, rusqlite::Error>(blocker)
        }) {
            Ok(iter) => match iter.collect::<Result<Vec<String>, _>>() {
                Ok(blockers) => blockers,
                Err(_) => Vec::new(),
            },
            Err(_) => Vec::new(),
        };
        x
    }

    /// Analyze patterns in excluded beads
    fn analyze_exclusion_patterns(&self, analyses: &[BeadExclusionAnalysis]) -> Vec<ExclusionPattern> {
        let mut patterns = Vec::new();

        // Group beads by exclusion filter
        let mut by_label: Vec<String> = Vec::new();
        let mut by_dependency: Vec<String> = Vec::new();
        let mut by_assignee: Vec<String> = Vec::new();

        for analysis in analyses {
            if let Some(filter) = &analysis.excluded_by_filter {
                match filter.as_str() {
                    "label exclusions" => by_label.push(analysis.bead_id.clone()),
                    "dependency checks" => by_dependency.push(analysis.bead_id.clone()),
                    "assignee filter" => by_assignee.push(analysis.bead_id.clone()),
                    _ => {}
                }
            }
        }

        // Pattern: All excluded beads have specific labels
        if !by_label.is_empty() {
            let label_patterns = self.analyze_label_patterns(analyses);
            patterns.extend(label_patterns);
        }

        // Pattern: Beads blocked by specific dependencies
        if !by_dependency.is_empty() {
            let dep_patterns = self.analyze_dependency_patterns(analyses);
            patterns.extend(dep_patterns);
        }

        // Pattern: Assigned beads
        if !by_assignee.is_empty() {
            patterns.push(ExclusionPattern {
                pattern: "assignee_exclusion".to_string(),
                description: format!("{} beads are assigned to workers and excluded by assignee filter", by_assignee.len()),
                matching_beads: by_assignee,
                suggested_fix: None, // This is expected behavior, not a bug
            });
        }

        patterns
    }

    /// Analyze label-based exclusion patterns
    fn analyze_label_patterns(&self, analyses: &[BeadExclusionAnalysis]) -> Vec<ExclusionPattern> {
        let mut patterns = Vec::new();
        let mut label_counts: HashMap<String, Vec<String>> = HashMap::new();

        for analysis in analyses {
            if analysis.excluded_by_filter.as_deref() == Some("label exclusions") {
                for label in &analysis.labels {
                    label_counts
                        .entry(label.clone())
                        .or_insert_with(Vec::new)
                        .push(analysis.bead_id.clone());
                }
            }
        }

        // Find significant label patterns
        for (label, bead_ids) in label_counts {
            if bead_ids.len() >= 2 {
                // If multiple beads share the same exclusion label
                let pattern = ExclusionPattern {
                    pattern: format!("label_exclusion_{}", label),
                    description: format!("{} beads have label '{}' causing exclusion", bead_ids.len(), label),
                    matching_beads: bead_ids,
                    suggested_fix: if ["deprecated", "blocked", "on-hold"].contains(&label.as_str()) {
                        None // Standard exclusion labels, not a bug
                    } else {
                        Some(format!("Review whether label '{}' should be excluded by Pluck query", label))
                    },
                };
                patterns.push(pattern);
            }
        }

        patterns
    }

    /// Analyze dependency-based exclusion patterns
    fn analyze_dependency_patterns(&self, analyses: &[BeadExclusionAnalysis]) -> Vec<ExclusionPattern> {
        let mut patterns = Vec::new();
        let mut blocker_counts: HashMap<String, Vec<String>> = HashMap::new();

        for analysis in analyses {
            if analysis.excluded_by_filter.as_deref() == Some("dependency checks") {
                for blocker in &analysis.blocking_dependencies {
                    blocker_counts
                        .entry(blocker.clone())
                        .or_insert_with(Vec::new)
                        .push(analysis.bead_id.clone());
                }
            }
        }

        // Find beads blocked by the same dependency
        for (blocker, bead_ids) in blocker_counts {
            if bead_ids.len() >= 2 {
                patterns.push(ExclusionPattern {
                    pattern: format!("dependency_blocker_{}", blocker),
                    description: format!("{} beads are blocked by dependency on '{}'", bead_ids.len(), blocker),
                    matching_beads: bead_ids,
                    suggested_fix: Some(format!("Check if bead '{}' is stuck and needs to be closed or updated", blocker)),
                });
            }
        }

        patterns
    }

    /// Generate recommended fixes based on patterns
    fn generate_fixes(&self, patterns: &[ExclusionPattern]) -> Vec<String> {
        let mut fixes = Vec::new();

        for pattern in patterns {
            if let Some(ref fix) = pattern.suggested_fix {
                fixes.push(fix.clone());
            }
        }

        if fixes.is_empty() {
            fixes.push("No systematic patterns detected. Exclusions appear to be legitimate filter behavior.".to_string());
        }

        fixes
    }

    /// Check if the report contains actionable findings that warrant a diagnostic bead
    fn has_actionable_findings(&self, report: &PluckQueryDebugReport) -> bool {
        // Filter out beads that are legitimately being worked on
        // Any bead that is in_progress with an assignee is correctly excluded from the
        // ready frontier (status filter), so excluding it is not a bug.
        // This prevents circular false positives where working on any starvation-related
        // bead triggers creation of another starvation-resolution bead.
        let actionable_analyses: Vec<_> = report.bead_analyses.iter()
            .filter(|analysis| {
                // Skip ANY beads that are in_progress with an assignee
                // (they're being worked on, so exclusion is legitimate)
                if analysis.status == "in_progress" && analysis.assignee.is_some() {
                    return false;
                }
                true
            })
            .collect();

        // Actionable if relaxed queries found candidates that shouldn't be excluded
        let has_visible_at_relaxed_level = actionable_analyses.iter().any(|analysis| {
            analysis.visible_at_level.is_some() &&
            analysis.visible_at_level != Some(QueryLevel::Exact)
        });

        // Actionable if patterns have suggested fixes
        let has_recommended_fixes = report.recommended_fixes.iter().any(|fix| {
            !fix.contains("legitimate filter behavior") && !fix.contains("No systematic patterns")
        });

        has_visible_at_relaxed_level || has_recommended_fixes
    }

    /// File a starvation-resolution bead for detected filter issues
    /// This creates a claimable bead for agents (not a human-blocked alert bead)
    fn file_diagnostic_bead(&self, report: &PluckQueryDebugReport) -> Result<()> {
        // Use current working directory as workspace path
        let workspace_path = std::env::current_dir()
            .context("Failed to get current working directory")?;

        // Generate bead title with key information
        let title = format!(
            "Starvation-resolution: Fix Pluck query excluding {} of {} open beads",
            report.total_open_beads - report.ready_frontier_count,
            report.total_open_beads
        );

        // Extract the primary filter causing exclusion
        let primary_filter = self.determine_primary_filter(report);

        // Build summary description with key findings
        let description = self.generate_summary_description(report, &primary_filter);

        // Build action-oriented notes with clear resolution steps
        let notes = self.generate_resolution_notes(report);

        // Construct the bead create command - use full path to bead command
        let bead_path = std::env::var("CARGO_HOME")
            .map(|cargo_home| format!("{}/bin/bead", cargo_home))
            .unwrap_or_else(|_| "/home/coding/.cargo/bin/bead".to_string());

        // Step 1: Create the bead (notes must be added via update)
        let output = Command::new(&bead_path)
            .args([
                "create",
                "--title", &title,
                "--priority", "3",  // Higher than normal (3) to ensure quick pickup
                "--issue-type", "task",
                "--label", "starvation-resolution",
                "--label", "auto-generated",
                "--label", "agent-claimable",
                "--label", &format!("filter-{}", primary_filter.replace(' ', "-")),
                "--description", &description,
            ])
            .current_dir(&workspace_path)
            .output()
            .context("Failed to execute bead create command")?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(anyhow!(
                "bead create failed with exit code {:?}: {}",
                output.status.code(),
                stderr
            ));
        }

        let stdout = String::from_utf8(output.stdout)
            .context("bead create output is not valid UTF-8")?;

        // Extract bead ID from output - format is usually just the ID
        let bead_id = stdout.lines().next().map(|s| s.trim().to_string());

        // Step 2: Add notes via update (if bead was created successfully)
        if let Some(ref id) = bead_id {
            eprintln!("📋 Auto-generated starvation-resolution bead: {}", id);
            eprintln!("   An agent can claim this bead to execute the resolution steps.");

            // Update the bead with notes
            let update_output = Command::new(&bead_path)
                .args([
                    "update",
                    id,
                    "--notes", &notes,
                ])
                .current_dir(&workspace_path)
                .output()
                .context("Failed to execute bead update command for notes")?;

            if !update_output.status.success() {
                let stderr = String::from_utf8_lossy(&update_output.stderr);
                eprintln!("⚠️  Warning: bead update for notes failed (bead still created): {}", stderr);
            } else {
                eprintln!("📝 Added resolution plan to bead: {}", id);
            }
        }

        Ok(())
    }

    /// Determine the primary filter causing the exclusion
    fn determine_primary_filter(&self, report: &PluckQueryDebugReport) -> String {
        // Count beads excluded by each filter
        let mut filter_counts: HashMap<String, usize> = HashMap::new();

        for analysis in &report.bead_analyses {
            if let Some(ref filter) = analysis.excluded_by_filter {
                *filter_counts.entry(filter.clone()).or_insert(0) += 1;
            }
        }

        // Find the filter excluding the most beads
        filter_counts
            .into_iter()
            .max_by_key(|(_, count)| *count)
            .map(|(filter, _)| filter.clone())
            .unwrap_or_else(|| "unknown filter".to_string())
    }

    /// Generate summary description for the diagnostic bead
    fn generate_summary_description(&self, report: &PluckQueryDebugReport, primary_filter: &str) -> String {
        format!(
            "Auto-detected at {}: Pluck ready frontier is empty despite {} open beads existing. \
             Primary filter issue: {}. {} beads become visible with relaxed filters. \
             See bead notes for full diagnostic details.",
            report.timestamp.format("%Y-%m-%d %H:%M:%S UTC"),
            report.total_open_beads,
            primary_filter,
            report.total_open_beads - report.ready_frontier_count
        )
    }

    /// Generate detailed diagnostic notes for the bead
    fn generate_diagnostic_notes(&self, report: &PluckQueryDebugReport) -> String {
        let mut notes = String::new();

        notes.push_str("## Auto-Generated Diagnostic Report\n\n");
        notes.push_str(&format!("**Generated at:** {}\n\n", report.timestamp.format("%Y-%m-%d %H:%M:%S UTC")));
        notes.push_str(&format!("**Database:** {}\n\n", report.db_path));
        notes.push_str(&format!("**Total open beads:** {}\n\n", report.total_open_beads));
        notes.push_str(&format!("**Ready frontier count:** {}\n\n", report.ready_frontier_count));
        notes.push_str(&format!("**Starvation detected:** {}\n\n", report.starvation_detected));

        // Query results by level
        notes.push_str("## Query Results by Relaxation Level\n\n");
        for result in &report.level_results {
            notes.push_str(&format!("### Level {} - {}\n\n", result.level as i32, result.level_description));
            notes.push_str(&format!("- **Visible beads:** {}\n", result.visible_beads.len()));
            notes.push_str(&format!("- **Newly visible at this level:** {}\n", result.newly_visible_beads.len()));
            if !result.newly_visible_beads.is_empty() {
                notes.push_str(&format!("- **Bead IDs:** {:?}\n", result.newly_visible_beads));
            }
            notes.push_str("\n");
        }

        // Excluded beads analysis (top 10 to avoid overwhelming)
        notes.push_str("## Sample of Excluded Beads (first 10)\n\n");
        let excluded_analyses: Vec<_> = report.bead_analyses.iter()
            .filter(|a| a.visible_at_level != Some(QueryLevel::Exact))
            .take(10)
            .collect();

        for analysis in excluded_analyses {
            notes.push_str(&format!("### [{}] {}\n\n", analysis.bead_id, analysis.bead_title));
            notes.push_str(&format!("- **Status:** {}\n", analysis.status));
            if let Some(ref assignee) = analysis.assignee {
                notes.push_str(&format!("- **Assignee:** {}\n", assignee));
            }
            if analysis.manual_blocked {
                notes.push_str("- **Manually blocked:** Yes\n");
            }
            if !analysis.labels.is_empty() {
                notes.push_str(&format!("- **Labels:** {:?}\n", analysis.labels));
            }
            if !analysis.blocking_dependencies.is_empty() {
                notes.push_str(&format!("- **Blocked by:** {:?}\n", analysis.blocking_dependencies));
            }
            if let Some(level) = analysis.visible_at_level {
                notes.push_str(&format!("- **Visible at:** Level {} ({})\n", level as i32, level.description()));
            }
            if let Some(ref filter) = analysis.excluded_by_filter {
                notes.push_str(&format!("- **Excluded by:** {}\n", filter));
            }
            notes.push_str(&format!("- **Reason:** {}\n\n", analysis.exclusion_reason));
        }

        // Patterns and recommendations
        if !report.exclusion_patterns.is_empty() {
            notes.push_str("## Detected Patterns\n\n");
            for (i, pattern) in report.exclusion_patterns.iter().enumerate() {
                notes.push_str(&format!("{}. Pattern: {}\n", i + 1, pattern.pattern));
                notes.push_str(&format!("   {}\n", pattern.description));
                if !pattern.matching_beads.is_empty() {
                    notes.push_str(&format!("   **Affected beads:** {:?}\n", pattern.matching_beads));
                }
                if let Some(ref fix) = pattern.suggested_fix {
                    notes.push_str(&format!("   **Suggested fix:** {}\n", fix));
                }
                notes.push_str("\n");
            }
        }

        if !report.recommended_fixes.is_empty() {
            notes.push_str("## Recommended Fixes\n\n");
            for (i, fix) in report.recommended_fixes.iter().enumerate() {
                notes.push_str(&format!("{}. {}\n\n", i + 1, fix));
            }
        }

        notes.push_str("---\n\n");
        notes.push_str("*This bead was auto-generated by the Pluck query debugger.*\n");
        notes.push_str("*Full diagnostic details available at: `.beads/diagnostics/pluck-query-debug-report.jsonl`*\n");

        notes
    }

    /// Generate action-oriented resolution notes for agents to execute
    fn generate_resolution_notes(&self, report: &PluckQueryDebugReport) -> String {
        let mut notes = String::new();

        notes.push_str("# Starvation Resolution Plan\n\n");
        notes.push_str("**Auto-generated at:** ");
        notes.push_str(&report.timestamp.format("%Y-%m-%d %H:%M:%S UTC").to_string());
        notes.push_str("\n\n");
        notes.push_str("**Purpose:** This bead contains a clear action plan to resolve bead starvation.\n\n");
        notes.push_str("**Agents should claim this bead to execute the resolution steps.**\n\n");

        // Problem statement
        notes.push_str("## Problem Statement\n\n");
        notes.push_str(&format!("- **Total open beads:** {}\n", report.total_open_beads));
        notes.push_str(&format!("- **Ready frontier (visible to agents):** {}\n", report.ready_frontier_count));
        notes.push_str(&format!("- **Invisible beads:** {}\n", report.total_open_beads - report.ready_frontier_count));
        notes.push_str(&format!("- **Starvation detected:** {}\n\n", report.starvation_detected));

        // SQL Query Analysis
        notes.push_str("## SQL Query Analysis Results\n\n");
        notes.push_str("The following filters are excluding beads from the ready frontier:\n\n");
        for result in &report.level_results {
            if result.level != QueryLevel::Exact && !result.newly_visible_beads.is_empty() {
                notes.push_str(&format!("### {} ({})\n\n", result.level_description, result.level as i32));
                notes.push_str(&format!("- **Beads that become visible:** {}\n", result.newly_visible_beads.len()));
                notes.push_str(&format!("- **Filter relaxed:** {}\n",
                    result.level.relaxed_filter().unwrap_or("unknown")));
                notes.push_str(&format!("- **Affected bead IDs:** `{:?}`\n\n", result.newly_visible_beads));
            }
        }

        // Exclusion Reasons
        notes.push_str("## Exclusion Reasons by Bead\n\n");
        let excluded_analyses: Vec<_> = report.bead_analyses.iter()
            .filter(|a| a.visible_at_level != Some(QueryLevel::Exact))
            .take(15)
            .collect();

        for analysis in excluded_analyses {
            notes.push_str(&format!("### [{}] {}\n\n", analysis.bead_id, analysis.bead_title));
            if let Some(ref filter) = analysis.excluded_by_filter {
                notes.push_str(&format!("- **Excluded by:** `{}`\n", filter));
            }
            notes.push_str(&format!("- **Reason:** {}\n", analysis.exclusion_reason));

            // Add specific context based on filter type
            if analysis.excluded_by_filter.as_deref() == Some("label exclusions") {
                if !analysis.labels.is_empty() {
                    notes.push_str(&format!("- **Labels causing exclusion:** {:?}\n", analysis.labels));
                }
            } else if analysis.excluded_by_filter.as_deref() == Some("dependency checks") {
                if !analysis.blocking_dependencies.is_empty() {
                    notes.push_str(&format!("- **Blocking dependencies:** {:?}\n", analysis.blocking_dependencies));
                }
            } else if analysis.excluded_by_filter.as_deref() == Some("assignee filter") {
                if let Some(ref assignee) = analysis.assignee {
                    notes.push_str(&format!("- **Current assignee:** `{}`\n", assignee));
                    notes.push_str("- **Action:** Check if worker is still active; if not, clear assignee\n");
                }
            }
            notes.push_str("\n");
        }

        // Detected Patterns
        if !report.exclusion_patterns.is_empty() {
            notes.push_str("## Detected Patterns\n\n");
            for (i, pattern) in report.exclusion_patterns.iter().enumerate() {
                notes.push_str(&format!("{}. **Pattern:** `{}`\n", i + 1, pattern.pattern));
                notes.push_str(&format!("   **Description:** {}\n", pattern.description));
                if !pattern.matching_beads.is_empty() {
                    notes.push_str(&format!("   **Affected beads:** {:?}\n", pattern.matching_beads));
                }
                if let Some(ref fix) = pattern.suggested_fix {
                    notes.push_str(&format!("   **Suggested fix:** {}\n", fix));
                }
                notes.push_str("\n");
            }
        }

        // Recommended Fixes
        if !report.recommended_fixes.is_empty() {
            notes.push_str("## Recommended Fixes\n\n");
            for (i, fix) in report.recommended_fixes.iter().enumerate() {
                notes.push_str(&format!("{}. {}\n\n", i + 1, fix));
            }
        }

        // Clear Action Plan
        notes.push_str("## Clear Action Plan\n\n");
        notes.push_str("Execute the following steps in order:\n\n");

        let mut step_num = 1;

        // Step 1: Analyze the primary filter issue
        notes.push_str(&format!("{}. **Analyze the primary filter issue**\n", step_num));
        notes.push_str("   - Review the exclusion reasons above\n");
        notes.push_str("   - Identify the most common exclusion pattern\n");
        notes.push_str(&format!("   - Primary filter: {}\n\n", self.determine_primary_filter(report)));
        step_num += 1;

        // Step 2: Check if beads should be excluded
        notes.push_str(&format!("{}. **Verify beads are being excluded correctly**\n", step_num));
        notes.push_str("   - For label exclusions: Verify labels are correct and intentional\n");
        notes.push_str("   - For dependency exclusions: Check if blocking beads are stuck\n");
        notes.push_str("   - For assignee exclusions: Verify workers are still active\n\n");
        step_num += 1;

        // Step 3: Execute specific fixes
        notes.push_str(&format!("{}. **Execute specific resolution actions**\n", step_num));

        let has_stale_assignees = report.bead_analyses.iter()
            .any(|a| a.excluded_by_filter.as_deref() == Some("assignee filter"));

        let has_label_issues = report.bead_analyses.iter()
            .any(|a| a.excluded_by_filter.as_deref() == Some("label exclusions"));

        let has_dep_issues = report.bead_analyses.iter()
            .any(|a| a.excluded_by_filter.as_deref() == Some("dependency checks"));

        if has_stale_assignees {
            notes.push_str("   - **Stale assignees:** Clear assignee from beads with inactive workers:\n");
            for analysis in report.bead_analyses.iter()
                .filter(|a| a.excluded_by_filter.as_deref() == Some("assignee filter")) {
                if let Some(ref assignee) = analysis.assignee {
                    notes.push_str(&format!("     - `bead update {} --clear-assignee` (worker: {})\n",
                        analysis.bead_id, assignee));
                }
            }
            notes.push_str("\n");
        }

        if has_label_issues {
            notes.push_str("   - **Label issues:** Review and correct inappropriate label exclusions:\n");
            for analysis in report.bead_analyses.iter()
                .filter(|a| a.excluded_by_filter.as_deref() == Some("label exclusions")) {
                notes.push_str(&format!("     - [{}] has labels: {:?}\n",
                    analysis.bead_id, analysis.labels));
            }
            notes.push_str("\n");
        }

        if has_dep_issues {
            notes.push_str("   - **Dependency issues:** Investigate blocking beads:\n");
            for analysis in report.bead_analyses.iter()
                .filter(|a| a.excluded_by_filter.as_deref() == Some("dependency checks")) {
                if !analysis.blocking_dependencies.is_empty() {
                    notes.push_str(&format!("     - [{}] blocked by: {:?}\n",
                        analysis.bead_id, analysis.blocking_dependencies));
                }
            }
            notes.push_str("\n");
        }

        step_num += 1;

        // Step 4: Verify resolution
        notes.push_str(&format!("{}. **Verify the resolution**\n", step_num));
        notes.push_str("   - Run `cargo run --bin pluck-query-debugger` to re-check\n");
        notes.push_str("   - Verify ready frontier now contains beads\n");
        notes.push_str("   - If starvation persists, create a new resolution bead\n\n");
        step_num += 1;

        // Step 5: Close this bead
        notes.push_str(&format!("{}. **Close this bead**\n", step_num));
        notes.push_str("   - Once starvation is resolved, close this bead with:\n");
        notes.push_str("   - `bead close <this-bead-id> --reason 'Starvation resolved - ready frontier populated'\n\n");

        notes.push_str("---\n\n");
        notes.push_str("**Reference:** Full diagnostic details available at:\n");
        notes.push_str("`.beads/diagnostics/pluck-query-debug-report.jsonl`\n\n");
        notes.push_str("*This resolution plan was auto-generated by the Pluck query debugger.*\n");

        notes
    }

    /// Publish the debug report to JSONL file
    fn publish_report(&self, report: &PluckQueryDebugReport) -> Result<()> {
        let report_path = self.config.diagnostics_dir.join("pluck-query-debug-report.jsonl");
        let json_line = serde_json::to_string(report)
            .context("Failed to serialize debug report")?;

        let mut file = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&report_path)
            .context("Failed to open debug report file")?;

        writeln!(file, "{}", json_line)
            .context("Failed to write debug report")?;

        eprintln!("📋 Pluck query debug report published to {}", report_path.display());

        Ok(())
    }

    /// Print a human-readable summary of the report
    pub fn print_summary(&self, report: &PluckQueryDebugReport) {
        println!("\n=== Pluck Query Debug Report ===\n");
        println!("Timestamp: {}", report.timestamp.format("%Y-%m-%d %H:%M:%S UTC"));
        println!("Database: {}", report.db_path);
        println!("Total open beads: {}", report.total_open_beads);
        println!("Ready frontier (Level 0): {}", report.ready_frontier_count);
        println!("Starvation detected: {}", report.starvation_detected);

        println!("\n--- Query Results by Level ---");
        for result in &report.level_results {
            println!("\nLevel {} - {}",
                result.level as i32,
                result.level_description
            );
            println!("  Visible beads: {}", result.visible_beads.len());
            println!("  Newly visible: {}", result.newly_visible_beads.len());
            if !result.newly_visible_beads.is_empty() {
                println!("  New bead IDs: {:?}", result.newly_visible_beads);
            }
        }

        if report.starvation_detected {
            println!("\n--- Invisible Beads Analysis ---");
            for analysis in &report.bead_analyses {
                if analysis.visible_at_level != Some(QueryLevel::Exact) {
                    println!("\n[{}] {}", analysis.bead_id, analysis.bead_title);
                    println!("  Status: {}", analysis.status);
                    if let Some(assignee) = &analysis.assignee {
                        println!("  Assignee: {}", assignee);
                    }
                    if !analysis.labels.is_empty() {
                        println!("  Labels: {:?}", analysis.labels);
                    }
                    if !analysis.blocking_dependencies.is_empty() {
                        println!("  Blocked by: {:?}", analysis.blocking_dependencies);
                    }
                    if let Some(level) = analysis.visible_at_level {
                        println!("  Visible at: Level {} ({})", level as i32, level.description());
                    }
                    if let Some(ref filter) = analysis.excluded_by_filter {
                        println!("  Excluded by: {}", filter);
                    }
                    println!("  Reason: {}", analysis.exclusion_reason);
                }
            }
        }

        if !report.exclusion_patterns.is_empty() {
            println!("\n--- Exclusion Patterns ---");
            for (i, pattern) in report.exclusion_patterns.iter().enumerate() {
                println!("{}. Pattern: {}", i + 1, pattern.pattern);
                println!("   {}", pattern.description);
                if !pattern.matching_beads.is_empty() {
                    println!("   Affected beads: {:?}", pattern.matching_beads);
                }
                if let Some(ref fix) = pattern.suggested_fix {
                    println!("   Suggested fix: {}", fix);
                }
            }
        }

        if !report.recommended_fixes.is_empty() {
            println!("\n--- Recommended Fixes ---");
            for (i, fix) in report.recommended_fixes.iter().enumerate() {
                println!("{}. {}", i + 1, fix);
            }
        }

        if report.starvation_detected {
            println!("\n🚨 STARVATION DETECTED: Pluck query returns no ready beads");
        } else {
            println!("\n✅ No starvation detected - ready frontier is populated");
        }
    }
}

impl Default for PluckQueryDebugger {
    fn default() -> Self {
        Self::new().expect("Failed to create debugger with default config")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_query_level_ordering() {
        let levels = QueryLevel::all_levels();
        assert_eq!(levels.len(), 5);
        assert_eq!(levels[0], QueryLevel::Exact);
        assert_eq!(levels[4], QueryLevel::Raw);
    }

    #[test]
    fn test_level_descriptions() {
        assert_eq!(QueryLevel::Exact.description(), "Exact Pluck query (all filters applied)");
        assert_eq!(QueryLevel::Raw.description(), "Raw base query (status only)");
    }

    #[test]
    fn test_relaxed_filter() {
        assert_eq!(QueryLevel::Exact.relaxed_filter(), None);
        assert_eq!(QueryLevel::WithoutLabels.relaxed_filter(), Some("label exclusions"));
        assert_eq!(QueryLevel::WithoutDependencies.relaxed_filter(), Some("dependency checks"));
        assert_eq!(QueryLevel::WithoutAssignee.relaxed_filter(), Some("assignee filter"));
        assert_eq!(QueryLevel::Raw.relaxed_filter(), Some("all filters except status"));
    }

    #[test]
    fn test_exclusion_analysis_serialization() {
        let analysis = BeadExclusionAnalysis {
            bead_id: "test-1".to_string(),
            bead_title: "Test Bead".to_string(),
            status: "open".to_string(),
            assignee: Some("worker-1".to_string()),
            manual_blocked: false,
            labels: vec!["bug".to_string()],
            blocking_dependencies: vec!["test-2".to_string()],
            visible_at_level: Some(QueryLevel::WithoutAssignee),
            excluded_by_filter: Some("assignee filter".to_string()),
            exclusion_reason: "Excluded by assignee filter".to_string(),
        };

        let json = serde_json::to_string(&analysis).unwrap();
        assert!(json.contains("test-1"));
        assert!(json.contains("assignee filter"));
    }

    #[test]
    fn test_pattern_serialization() {
        let pattern = ExclusionPattern {
            pattern: "label_exclusion_deprecated".to_string(),
            description: "5 beads have label 'deprecated'".to_string(),
            matching_beads: vec!["bead-1".to_string(), "bead-2".to_string()],
            suggested_fix: Some("Review label usage".to_string()),
        };

        let json = serde_json::to_string(&pattern).unwrap();
        assert!(json.contains("label_exclusion_deprecated"));
        assert!(json.contains("deprecated"));
    }
}
