//! Bead Dependency Validator
//!
//! This module provides tools to detect and fix circular dependencies and orphaned
//! references in bead-rs databases. It addresses bead starvation issues caused by
//! malformed dependency graphs that block all workable beads.
//!
//! ## Problem
//!
//! Bead starvation occurs when `bead list --ready` returns no results even though
//! open beads exist. Common causes:
//! 1. Circular dependencies (bead A blocks B, B blocks A)
//! 2. Orphaned dependencies (bead blocked by a closed/non-existent bead)
//! 3. Assigned-but-open beads (stale assignee on an open bead)
//!
//! This tool detects and fixes (1) and (2) automatically.
//!
//! ## Usage
//!
//! ```bash
//! # Run validation and auto-fix
//! cargo run --bin bead-dependency-validator
//!
//! # With custom database path
//! cargo run --bin bead-dependency-validator -- --db-path /path/to/.beads/beads.db
//!
//! # Dry-run (no changes)
//! cargo run --bin bead-dependency-validator -- --dry-run
//! ```
//!
//! ## What it does
//!
//! 1. Loads all open beads and their dependency graphs
//! 2. Detects circular dependencies using DFS cycle detection
//! 3. Detects orphaned dependencies (blocked by closed/non-existent beads)
//! 4. For circular deps: removes blocking edge from younger bead (by creation time)
//! 5. For orphan deps: clears the blocked-by list
//! 6. Logs all fixes to events.jsonl for audit
//!
//! ## Exit codes
//!
//! - 0: Success (no issues found or all issues fixed)
//! - 1: Errors occurred during validation/fix
//! - 2: Circular dependencies found (but fixed)
//! - 3: Orphaned dependencies found (but fixed)
//! - 4: Both circular and orphaned found (but fixed)

use anyhow::{anyhow, Context, Result};
use chrono::{DateTime, Utc};
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::fs::OpenOptions;
use std::io::Write;
use std::path::PathBuf;

/// Configuration for the dependency validator
#[derive(Debug, Clone)]
pub struct ValidatorConfig {
    /// Path to the beads.db SQLite database
    pub db_path: PathBuf,
    /// Path to events.jsonl for logging fixes
    pub events_path: PathBuf,
    /// If true, detect issues but don't fix them
    pub dry_run: bool,
}

impl Default for ValidatorConfig {
    fn default() -> Self {
        let workspace_path = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        Self {
            db_path: workspace_path.join(".beads/beads.db"),
            events_path: workspace_path.join(".beads/events.jsonl"),
            dry_run: false,
        }
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
}

/// A dependency relationship between beads
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Dependency {
    pub blocked_issue_id: String,
    pub blocker_issue_id: String,
    pub kind: String,
}

/// An issue found during validation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Issue {
    /// Circular dependency detected
    CircularDependency {
        cycle: Vec<String>,
        younger_bead: String,
        blocker_to_remove: String,
    },
    /// Orphaned dependency (blocked by closed/non-existent bead)
    OrphanedDependency {
        bead_id: String,
        missing_blocker: String,
    },
}

/// A fix that was applied
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppliedFix {
    pub issue_type: String,
    pub description: String,
    pub bead_ids: Vec<String>,
    pub timestamp: DateTime<Utc>,
}

/// Result of validation
#[derive(Debug, Clone)]
pub struct ValidationResult {
    pub issues_found: Vec<Issue>,
    pub fixes_applied: Vec<AppliedFix>,
    pub total_beads_checked: usize,
    pub open_beads: usize,
}

/// Main validator struct
pub struct DependencyValidator {
    config: ValidatorConfig,
}

impl DependencyValidator {
    /// Create a new validator with default config
    pub fn new() -> Result<Self> {
        Self::with_config(ValidatorConfig::default())
    }

    /// Create a new validator with custom config
    pub fn with_config(config: ValidatorConfig) -> Result<Self> {
        if !config.db_path.exists() {
            return Err(anyhow!(
                "Database not found at {}",
                config.db_path.display()
            ));
        }
        Ok(Self { config })
    }

    /// Run validation and apply fixes
    pub fn validate_and_fix(&mut self) -> Result<ValidationResult> {
        let conn = Connection::open(&self.config.db_path).context("Failed to open database")?;

        let mut result = ValidationResult {
            issues_found: Vec::new(),
            fixes_applied: Vec::new(),
            total_beads_checked: 0,
            open_beads: 0,
        };

        // Load all beads
        let beads = self.load_beads(&conn)?;
        result.total_beads_checked = beads.len();
        result.open_beads = beads.values().filter(|b| b.status == "open").count();

        // Load all dependencies
        let dependencies = self.load_dependencies(&conn)?;

        // Detect circular dependencies
        let circular_issues = self.detect_circular_dependencies(&beads, &dependencies)?;
        for issue in circular_issues {
            result.issues_found.push(issue.clone());
            if let Issue::CircularDependency {
                cycle,
                younger_bead,
                blocker_to_remove,
            } = issue
            {
                let fix =
                    self.fix_circular_dependency(&conn, &younger_bead, &blocker_to_remove, &cycle)?;
                result.fixes_applied.push(fix);
            }
        }

        // Detect orphaned dependencies
        let orphaned_issues = self.detect_orphaned_dependencies(&beads, &dependencies)?;
        for issue in orphaned_issues {
            result.issues_found.push(issue.clone());
            if let Issue::OrphanedDependency {
                bead_id,
                missing_blocker,
            } = issue
            {
                let fix = self.fix_orphaned_dependency(&conn, &bead_id, &missing_blocker)?;
                result.fixes_applied.push(fix);
            }
        }

        Ok(result)
    }

    /// Load all beads from the database
    fn load_beads(&self, conn: &Connection) -> Result<HashMap<String, Bead>> {
        let mut stmt = conn.prepare(
            "SELECT id, title, base_status, created_at, assignee, manual_blocked
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
    ) -> Result<Vec<Issue>> {
        let mut issues = Vec::new();

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

        // Convert cycles to issues
        for cycle in cycles_found {
            if let Some((younger_bead, blocker_to_remove)) = self.resolve_cycle(&cycle, beads) {
                issues.push(Issue::CircularDependency {
                    cycle,
                    younger_bead,
                    blocker_to_remove,
                });
            }
        }

        Ok(issues)
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
                        let cycle: Vec<String> = path[cycle_start..]
                            .iter()
                            .map(|x| x.to_string() + " -> ")
                            .collect();
                        let mut cycle_str = cycle.join("");
                        cycle_str.push_str(neighbor);
                        let cycle_vec: Vec<String> = path[cycle_start..].to_vec();
                        cycles_found.insert(cycle_vec);
                    }
                }
            }
        }

        path.pop();
        rec_stack.remove(node);
    }

    /// Resolve a cycle by determining which edge to remove
    fn resolve_cycle(
        &self,
        cycle: &[String],
        beads: &HashMap<String, Bead>,
    ) -> Option<(String, String)> {
        if cycle.len() < 2 {
            return None;
        }

        // Find the youngest bead in the cycle
        let mut youngest = &cycle[0];
        let mut youngest_time = beads.get(youngest)?.created_at;

        for bead_id in cycle.iter().skip(1) {
            if let Some(bead) = beads.get(bead_id) {
                if bead.created_at > youngest_time {
                    youngest = bead_id;
                    youngest_time = bead.created_at;
                }
            }
        }

        // Find who blocks the youngest bead
        for (i, bead_id) in cycle.iter().enumerate() {
            if bead_id == youngest {
                // The blocker is the next bead in the cycle
                let blocker_idx = if i + 1 < cycle.len() { i + 1 } else { 0 };
                let blocker = &cycle[blocker_idx];
                return Some((youngest.clone(), blocker.clone()));
            }
        }

        None
    }

    /// Fix a circular dependency by removing the blocking edge
    fn fix_circular_dependency(
        &mut self,
        conn: &Connection,
        younger_bead: &str,
        blocker_to_remove: &str,
        cycle: &[String],
    ) -> Result<AppliedFix> {
        if self.config.dry_run {
            return Ok(AppliedFix {
                issue_type: "circular_dependency".to_string(),
                description: format!(
                    "[DRY-RUN] Would remove blocking edge: {} blocked by {}",
                    younger_bead, blocker_to_remove
                ),
                bead_ids: cycle.to_vec(),
                timestamp: Utc::now(),
            });
        }

        conn.execute(
            "DELETE FROM dependencies
             WHERE blocked_issue_id = ?1 AND blocker_issue_id = ?2 AND kind = 'blocks'",
            params![younger_bead, blocker_to_remove],
        )
        .context("Failed to delete circular dependency")?;

        let fix = AppliedFix {
            issue_type: "circular_dependency".to_string(),
            description: format!(
                "Removed blocking edge: {} was blocked by {} (circular dependency)",
                younger_bead, blocker_to_remove
            ),
            bead_ids: cycle.to_vec(),
            timestamp: Utc::now(),
        };

        self.log_fix(&fix)?;

        Ok(fix)
    }

    /// Detect orphaned dependencies (blocked by closed/non-existent beads)
    fn detect_orphaned_dependencies(
        &self,
        beads: &HashMap<String, Bead>,
        dependencies: &[Dependency],
    ) -> Result<Vec<Issue>> {
        let mut issues = Vec::new();

        for dep in dependencies {
            // Check if the blocker exists and is open
            if let Some(blocker) = beads.get(&dep.blocker_issue_id) {
                if blocker.status == "closed" {
                    issues.push(Issue::OrphanedDependency {
                        bead_id: dep.blocked_issue_id.clone(),
                        missing_blocker: dep.blocker_issue_id.clone(),
                    });
                }
            } else {
                // Blocker doesn't exist at all
                issues.push(Issue::OrphanedDependency {
                    bead_id: dep.blocked_issue_id.clone(),
                    missing_blocker: dep.blocker_issue_id.clone(),
                });
            }
        }

        Ok(issues)
    }

    /// Fix an orphaned dependency by removing it
    fn fix_orphaned_dependency(
        &mut self,
        conn: &Connection,
        bead_id: &str,
        missing_blocker: &str,
    ) -> Result<AppliedFix> {
        if self.config.dry_run {
            return Ok(AppliedFix {
                issue_type: "orphaned_dependency".to_string(),
                description: format!(
                    "[DRY-RUN] Would remove orphaned dependency: {} blocked by {}",
                    bead_id, missing_blocker
                ),
                bead_ids: vec![bead_id.to_string(), missing_blocker.to_string()],
                timestamp: Utc::now(),
            });
        }

        conn.execute(
            "DELETE FROM dependencies
             WHERE blocked_issue_id = ?1 AND blocker_issue_id = ?2",
            params![bead_id, missing_blocker],
        )
        .context("Failed to delete orphaned dependency")?;

        let fix = AppliedFix {
            issue_type: "orphaned_dependency".to_string(),
            description: format!(
                "Removed orphaned dependency: {} was blocked by non-existent/closed bead {}",
                bead_id, missing_blocker
            ),
            bead_ids: vec![bead_id.to_string(), missing_blocker.to_string()],
            timestamp: Utc::now(),
        };

        self.log_fix(&fix)?;

        Ok(fix)
    }

    /// Log a fix to events.jsonl
    fn log_fix(&self, fix: &AppliedFix) -> Result<()> {
        let event = serde_json::json!({
            "issue_id": "dependency-validator",
            "kind": "dependency_fix",
            "actor": "bead-dependency-validator",
            "time": fix.timestamp.to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
            "detail": serde_json::to_value(fix).context("Failed to serialize fix")?,
        });

        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.config.events_path)
            .context("Failed to open events.jsonl")?;

        writeln!(file, "{}", event).context("Failed to write event")?;

        Ok(())
    }

    /// Print a summary of the validation result
    pub fn print_summary(&self, result: &ValidationResult) {
        println!("\n=== Bead Dependency Validator Summary ===\n");
        println!("Total beads checked: {}", result.total_beads_checked);
        println!("Open beads: {}", result.open_beads);
        println!("Issues found: {}", result.issues_found.len());
        println!("Fixes applied: {}", result.fixes_applied.len());

        if !result.issues_found.is_empty() {
            println!("\n--- Issues Found ---");
            for (i, issue) in result.issues_found.iter().enumerate() {
                println!("{}. {}", i + 1, self.format_issue(issue));
            }
        }

        if !result.fixes_applied.is_empty() {
            println!("\n--- Fixes Applied ---");
            for (i, fix) in result.fixes_applied.iter().enumerate() {
                println!("{}. [{}] {}", i + 1, fix.issue_type, fix.description);
            }
        }

        if result.issues_found.is_empty() {
            println!("\n✅ No issues found - dependency graph is healthy!");
        } else {
            println!("\n✅ All issues fixed successfully!");
        }

        if self.config.dry_run {
            println!("\n⚠️  DRY RUN MODE - No changes were actually made");
            println!("Run without --dry-run to apply fixes.");
        }
    }

    /// Format an issue for display
    fn format_issue(&self, issue: &Issue) -> String {
        match issue {
            Issue::CircularDependency { cycle, .. } => {
                format!("Circular dependency detected: {}", cycle.join(" -> "))
            }
            Issue::OrphanedDependency {
                bead_id,
                missing_blocker,
            } => {
                format!(
                    "Bead {} blocked by non-existent/closed bead {}",
                    bead_id, missing_blocker
                )
            }
        }
    }
}

impl Default for DependencyValidator {
    fn default() -> Self {
        Self::new().expect("Failed to create validator with default config")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_resolve_cycle() {
        let mut beads = HashMap::new();
        beads.insert(
            "a".to_string(),
            Bead {
                id: "a".to_string(),
                title: "A".to_string(),
                status: "open".to_string(),
                created_at: DateTime::from_timestamp(1000, 0).unwrap().into(),
                assignee: None,
                manual_blocked: false,
            },
        );
        beads.insert(
            "b".to_string(),
            Bead {
                id: "b".to_string(),
                title: "B".to_string(),
                status: "open".to_string(),
                created_at: DateTime::from_timestamp(2000, 0).unwrap().into(),
                assignee: None,
                manual_blocked: false,
            },
        );
        beads.insert(
            "c".to_string(),
            Bead {
                id: "c".to_string(),
                title: "C".to_string(),
                status: "open".to_string(),
                created_at: DateTime::from_timestamp(1500, 0).unwrap().into(),
                assignee: None,
                manual_blocked: false,
            },
        );

        let validator = DependencyValidator::new().unwrap();
        let cycle = vec!["a".to_string(), "b".to_string(), "c".to_string()];

        let result = validator.resolve_cycle(&cycle, &beads);
        assert!(result.is_some());

        let (younger, blocker) = result.unwrap();
        // B is youngest (timestamp 2000), so we should return it and its blocker (C)
        assert_eq!(younger, "b");
        assert_eq!(blocker, "c");
    }
}
