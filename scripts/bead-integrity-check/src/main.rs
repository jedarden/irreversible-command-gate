use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use colored_json::ToColoredJson;
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::fs;
use std::io::BufRead;

/// Find workspace root by searching for .beads directory
fn find_workspace_root(mut current: PathBuf) -> Result<String> {
    loop {
        let beads_dir = current.join(".beads");
        if beads_dir.exists() && beads_dir.is_dir() {
            return Ok(current.to_string_lossy().to_string());
        }

        match current.parent() {
            Some(parent) => current = parent.to_path_buf(),
            None => anyhow::bail!("Could not find .beads directory in current or parent directories"),
        }
    }
}

/// Three views of bead state to compare
#[derive(Debug, Clone, Serialize, Deserialize)]
struct DatabaseView {
    beads: HashMap<String, BeadRecord>,
    timestamp: DateTime<Utc>,
    source: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct CheckpointView {
    beads: HashMap<String, BeadRecord>,
    timestamp: DateTime<Utc>,
    source: String,
    metadata: CheckpointMetadata,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct GitView {
    beads: HashMap<String, BeadRecord>,
    timestamp: DateTime<Utc>,
    commit: String,
    source: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct CheckpointMetadata {
    generation_id: String,
    snapshot_sequence: i64,
    issue_count: i64,
    created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Hash, PartialEq, Eq)]
struct BeadRecord {
    id: String,
    #[serde(alias = "base_status", alias = "status")]
    status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    assignee: Option<String>,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}

#[derive(Debug, Serialize, Deserialize)]
struct DivergenceReport {
    database_view: ViewSummary,
    checkpoint_view: ViewSummary,
    git_view: ViewSummary,
    divergences: Vec<Divergence>,
    recommendations: Vec<String>,
    generated_at: DateTime<Utc>,
}

#[derive(Debug, Serialize, Deserialize)]
struct ViewSummary {
    bead_count: usize,
    timestamp: DateTime<Utc>,
    source: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct Divergence {
    divergence_type: String,
    severity: String,
    description: String,
    affected_beads: Vec<String>,
    recommendation: String,
}

/// Main integrity checker
struct BeadIntegrityChecker {
    workspace_path: String,
}

impl BeadIntegrityChecker {
    fn new(workspace_path: String) -> Self {
        Self { workspace_path }
    }

    /// Run full integrity check
    fn check(&self) -> Result<DivergenceReport> {
        println!("🔍 Checking bead integrity across three views...\n");

        // Load all three views
        let db_view = self.load_database_view()
            .context("Failed to load database view")?;
        println!("✅ Database view: {} beads", db_view.beads.len());

        let checkpoint_view = self.load_checkpoint_view()
            .context("Failed to load checkpoint view")?;
        println!("✅ Checkpoint view: {} beads", checkpoint_view.beads.len());

        let git_view = self.load_git_view()
            .context("Failed to load git view")?;
        println!("✅ Git view: {} beads", git_view.beads.len());
        println!();

        // Detect divergences
        let divergences = self.detect_divergences(&db_view, &checkpoint_view, &git_view);

        // Generate recommendations
        let recommendations = self.generate_recommendations(&divergences);

        Ok(DivergenceReport {
            database_view: ViewSummary {
                bead_count: db_view.beads.len(),
                timestamp: db_view.timestamp,
                source: db_view.source,
            },
            checkpoint_view: ViewSummary {
                bead_count: checkpoint_view.beads.len(),
                timestamp: checkpoint_view.timestamp,
                source: checkpoint_view.source,
            },
            git_view: ViewSummary {
                bead_count: git_view.beads.len(),
                timestamp: git_view.timestamp,
                source: git_view.source,
            },
            divergences,
            recommendations,
            generated_at: Utc::now(),
        })
    }

    /// Load view from SQLite database
    fn load_database_view(&self) -> Result<DatabaseView> {
        let db_path = format!("{}/.beads/beads.db", self.workspace_path);

        let conn = Connection::open(&db_path)
            .context("Failed to open beads.db")?;

        let mut stmt = conn.prepare(
            "SELECT id, base_status, assignee, created_at, updated_at FROM issues"
        )?;

        let beads = stmt.query_map([], |row| {
            let created_str: String = row.get(3)?;
            let updated_str: String = row.get(4)?;
            let created_at = DateTime::parse_from_rfc3339(&created_str)
                .map(|dt| dt.with_timezone(&Utc))
                .unwrap_or_else(|_| Utc::now());
            let updated_at = DateTime::parse_from_rfc3339(&updated_str)
                .map(|dt| dt.with_timezone(&Utc))
                .unwrap_or_else(|_| Utc::now());

            Ok(BeadRecord {
                id: row.get(0)?,
                status: row.get(1)?,
                assignee: row.get(2)?,
                created_at,
                updated_at,
            })
        })?;

        let mut bead_map = HashMap::new();
        for bead in beads {
            let bead = bead?;
            bead_map.insert(bead.id.clone(), bead);
        }

        // Also verify via bead list --json (JSONL format)
        let output = Command::new("bead")
            .args(["list", "--json"])
            .current_dir(&self.workspace_path)
            .output()
            .context("Failed to run bead list --json")?;

        if output.status.success() {
            // bead list --json outputs JSONL (one JSON object per line)
            for line in std::io::BufReader::new(&*output.stdout).lines() {
                let line = line.context("Failed to read line from bead list")?;
                if let Ok(bead) = serde_json::from_str::<BeadRecord>(&line) {
                    bead_map.insert(bead.id.clone(), bead);
                }
            }
        }

        Ok(DatabaseView {
            beads: bead_map,
            timestamp: Utc::now(),
            source: format!("SQLite database: {}", db_path),
        })
    }

    /// Load view from checkpoint files
    fn load_checkpoint_view(&self) -> Result<CheckpointView> {
        let checkpoint_dir = format!("{}/.beads/checkpoint", self.workspace_path);

        // Read current.json for metadata
        let current_path = format!("{}/current.json", checkpoint_dir);
        let current_json: serde_json::Value = serde_json::from_str(
            &fs::read_to_string(&current_path)
                .context("Failed to read current.json")?
        ).context("Failed to parse current.json")?;

        let metadata = CheckpointMetadata {
            generation_id: current_json["generation_id"].as_str()
                .unwrap_or("unknown").to_string(),
            snapshot_sequence: current_json["snapshot_sequence"].as_i64().unwrap_or(0),
            issue_count: current_json["issue_count"].as_i64().unwrap_or(0),
            created_at: current_json["created_at"].as_str()
                .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
                .map(|dt| dt.with_timezone(&Utc))
                .unwrap_or_else(|| Utc::now()),
        };

        // Read forensic.jsonl for bead records
        let forensic_path = format!("{}/forensic.jsonl", checkpoint_dir);
        let file = fs::File::open(&forensic_path)
            .context("Failed to open forensic.jsonl")?;

        let mut bead_map = HashMap::new();
        for line in std::io::BufReader::new(file).lines() {
            let line = line.context("Failed to read line from forensic.jsonl")?;
            if let Ok(value) = serde_json::from_str::<serde_json::Value>(&line) {
                if value["record_type"].as_str() == Some("issue") {
                    if let Ok(bead) = serde_json::from_value::<BeadRecord>(value["issue"].clone()) {
                        bead_map.insert(bead.id.clone(), bead);
                    }
                }
            }
        }

        Ok(CheckpointView {
            beads: bead_map,
            timestamp: Utc::now(),
            source: format!("Checkpoint: {}", checkpoint_dir),
            metadata,
        })
    }

    /// Load view from git-tracked checkpoint state
    fn load_git_view(&self) -> Result<GitView> {
        let repo = git2::Repository::discover(&self.workspace_path)
            .context("Failed to discover git repository")?;

        // Get latest commit that touched checkpoint
        let head = repo.head()
            .context("Failed to get HEAD")?;

        let commit = head.peel_to_commit()
            .context("Failed to peel to commit")?;

        let commit_short = commit.id().to_string()[..8].to_string();

        // Read checkpoint files from git
        let checkpoint_dir = format!("{}/.beads/checkpoint", self.workspace_path);
        let _forensic_path = format!("{}/forensic.jsonl", checkpoint_dir);

        let mut bead_map = HashMap::new();

        // Try to read forensic.jsonl from git
        if let Ok(tree) = commit.tree() {
            if let Ok(forensic_entry) = tree.get_path(Path::new(".beads/checkpoint/forensic.jsonl")) {
                if let Ok(object) = forensic_entry.to_object(&repo) {
                    if let Some(blob) = object.as_blob() {
                        let content = std::str::from_utf8(blob.content())
                            .context("Failed to parse forensic.jsonl as UTF-8")?;

                        for line in content.lines() {
                            if let Ok(value) = serde_json::from_str::<serde_json::Value>(line) {
                                if value["record_type"].as_str() == Some("issue") {
                                    if let Ok(bead) = serde_json::from_value::<BeadRecord>(value["issue"].clone()) {
                                        bead_map.insert(bead.id.clone(), bead);
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        Ok(GitView {
            beads: bead_map,
            timestamp: DateTime::from_timestamp(commit.time().seconds(), 0)
                .unwrap_or_else(|| Utc::now())
                .with_timezone(&Utc),
            commit: commit_short,
            source: format!("Git repository: {}", self.workspace_path),
        })
    }

    /// Detect divergences between views
    fn detect_divergences(
        &self,
        db_view: &DatabaseView,
        checkpoint_view: &CheckpointView,
        git_view: &GitView,
    ) -> Vec<Divergence> {
        let mut divergences = Vec::new();

        let db_ids: HashSet<_> = db_view.beads.keys().cloned().collect();
        let checkpoint_ids: HashSet<_> = checkpoint_view.beads.keys().cloned().collect();
        let git_ids: HashSet<_> = git_view.beads.keys().cloned().collect();

        // Beads in database but not in checkpoint
        let db_only: Vec<_> = db_ids.difference(&checkpoint_ids).cloned().collect();
        if !db_only.is_empty() {
            divergences.push(Divergence {
                divergence_type: "database_ahead_of_checkpoint".to_string(),
                severity: if db_only.len() > 10 { "high" } else { "medium" }.to_string(),
                description: format!(
                    "{} beads exist in database but not in checkpoint (checkpoint is stale)",
                    db_only.len()
                ),
                affected_beads: db_only,
                recommendation: "Run 'bead sync flush-only' to synchronize database to checkpoint".to_string(),
            });
        }

        // Beads in checkpoint but not in database
        let checkpoint_only: Vec<_> = checkpoint_ids.difference(&db_ids).cloned().collect();
        if !checkpoint_only.is_empty() {
            divergences.push(Divergence {
                divergence_type: "checkpoint_ahead_of_database".to_string(),
                severity: "high".to_string(),
                description: format!(
                    "{} beads exist in checkpoint but not in database (database corruption or sync failure)",
                    checkpoint_only.len()
                ),
                affected_beads: checkpoint_only,
                recommendation: "Run 'bead doctor --repair' then 'bead sync import-only' to rebuild database".to_string(),
            });
        }

        // Beads in git but not in current checkpoint
        let git_only: Vec<_> = git_ids.difference(&checkpoint_ids).cloned().collect();
        if !git_only.is_empty() {
            divergences.push(Divergence {
                divergence_type: "git_ahead_of_checkpoint".to_string(),
                severity: "medium".to_string(),
                description: format!(
                    "{} beads exist in git-tracked checkpoint but not in current checkpoint (uncommitted changes or stale checkpoint)",
                    git_only.len()
                ),
                affected_beads: git_only,
                recommendation: "Review uncommitted changes and run 'bead sync flush-only' if needed".to_string(),
            });
        }

        // Count mismatches
        if db_view.beads.len() != checkpoint_view.beads.len() {
            divergences.push(Divergence {
                divergence_type: "count_mismatch_db_checkpoint".to_string(),
                severity: "medium".to_string(),
                description: format!(
                    "Database has {} beads, checkpoint has {} beads",
                    db_view.beads.len(),
                    checkpoint_view.beads.len()
                ),
                affected_beads: vec![],
                recommendation: "Run 'bead sync flush-only' to resynchronize".to_string(),
            });
        }

        if checkpoint_view.beads.len() != git_view.beads.len() {
            divergences.push(Divergence {
                divergence_type: "count_mismatch_checkpoint_git".to_string(),
                severity: "low".to_string(),
                description: format!(
                    "Checkpoint has {} beads, git has {} beads (normal if checkpoint has uncommitted changes)",
                    checkpoint_view.beads.len(),
                    git_view.beads.len()
                ),
                affected_beads: vec![],
                recommendation: "Commit checkpoint changes if needed, or ignore if working locally".to_string(),
            });
        }

        // Check for stale assignees (common issue from reopen bug)
        let stale_assigned: Vec<_> = db_view.beads.iter()
            .filter(|(_, bead)| {
                bead.status == "open" &&
                bead.assignee.is_some() &&
                bead.assignee.as_ref().unwrap().contains("claude-code")
            })
            .map(|(id, _)| id.clone())
            .collect();

        if !stale_assigned.is_empty() {
            divergences.push(Divergence {
                divergence_type: "stale_assignees".to_string(),
                severity: "medium".to_string(),
                description: format!(
                    "{} open beads have assignees but should be unassigned (possible reopen bug)",
                    stale_assigned.len()
                ),
                affected_beads: stale_assigned,
                recommendation: "Run 'bead update <id> --clear-assignee' for each affected bead".to_string(),
            });
        }

        divergences
    }

    /// Generate repair recommendations
    fn generate_recommendations(&self, divergences: &[Divergence]) -> Vec<String> {
        let mut recommendations = Vec::new();

        if divergences.is_empty() {
            recommendations.push("✅ No divergences detected - all views are consistent.".to_string());
            return recommendations;
        }

        let has_critical = divergences.iter().any(|d| d.severity == "high");
        let has_db_ahead = divergences.iter().any(|d| d.divergence_type == "database_ahead_of_checkpoint");
        let has_corruption = divergences.iter().any(|d| d.divergence_type == "checkpoint_ahead_of_database");

        if has_db_ahead && !has_corruption {
            recommendations.push(
                "🔄 Run: bead sync flush-only".to_string()
            );
        }

        if has_corruption {
            recommendations.extend(vec![
                "🩺 Run: bead doctor --repair".to_string(),
                "If doctor fails, rebuild from checkpoint:".to_string(),
                "  1. bead init".to_string(),
                "  2. bead sync import-only --input .beads/checkpoint/forensic.jsonl --restore-into-empty --actor <you>".to_string(),
            ]);
        }

        if has_critical {
            recommendations.push(
                "⚠️  Critical divergences detected - repair immediately before continuing work.".to_string()
            );
        } else {
            recommendations.push(
                "ℹ️  Minor divergences detected - repair when convenient.".to_string()
            );
        }

        recommendations.push(
            "📝 Always run 'git pull' before 'bead sync flush-only' to avoid conflicts.".to_string()
        );

        recommendations
    }
}

fn main() -> Result<()> {
    // Find workspace root by looking for .beads directory
    let current_dir = std::env::current_dir()
        .context("Failed to get current directory")?;

    let workspace_path = find_workspace_root(current_dir)
        .context("Failed to find workspace root - are you in a bead workspace?")?;

    let checker = BeadIntegrityChecker::new(workspace_path);

    let report = checker.check()?;

    // Display results
    println!("╔════════════════════════════════════════════════════════════╗");
    println!("║         Bead Integrity Check Report                       ║");
    println!("╚════════════════════════════════════════════════════════════╝");
    println!();

    println!("📊 View Summary:");
    println!("   Database: {} beads ({})",
        report.database_view.bead_count,
        report.database_view.timestamp.format("%Y-%m-%d %H:%M:%S UTC")
    );
    println!("   Checkpoint: {} beads ({})",
        report.checkpoint_view.bead_count,
        report.checkpoint_view.timestamp.format("%Y-%m-%d %H:%M:%S UTC")
    );
    println!("   Git (HEAD): {} beads (commit {})",
        report.git_view.bead_count,
        report.git_view.source.split(':').last().unwrap_or("unknown")
    );
    println!();

    if report.divergences.is_empty() {
        println!("✅ No divergences detected - all three views are consistent.\n");
    } else {
        println!("⚠️  Detected {} divergence(s):\n", report.divergences.len());

        for (i, div) in report.divergences.iter().enumerate() {
            println!("{}. [{}] {} - {} bead(s) affected",
                i + 1,
                div.severity.to_uppercase(),
                div.divergence_type,
                div.affected_beads.len()
            );
            println!("   {}", div.description);
            if !div.affected_beads.is_empty() && div.affected_beads.len() <= 5 {
                println!("   Affected: {}", div.affected_beads.join(", "));
            } else if div.affected_beads.len() > 5 {
                println!("   Affected: {} beads (first few: {}...)",
                    div.affected_beads.len(),
                    div.affected_beads.iter().take(3).cloned().collect::<Vec<_>>().join(", ")
                );
            }
            println!("   💡 {}", div.recommendation);
            println!();
        }
    }

    println!("📋 Recommendations:");
    for rec in &report.recommendations {
        println!("   {}", rec);
    }
    println!();

    // Output JSON for automation
    if let Ok(json) = serde_json::to_string_pretty(&report) {
        use colored_json::ColorMode;
        use std::io::IsTerminal;
        let color_mode = if std::io::stdout().is_terminal() {
            ColorMode::On
        } else {
            ColorMode::Off
        };
        if let Ok(colored) = json.to_colored_json(color_mode) {
            println!("📄 Full JSON Report:");
            println!("{}", colored);
        }
    }

    // Exit with error code if critical divergences found
    let has_critical = report.divergences.iter().any(|d| d.severity == "high");
    if has_critical {
        println!("\n❌ Critical divergences detected - repair required.");
        std::process::exit(1);
    } else if !report.divergences.is_empty() {
        println!("\n⚠️  Non-critical divergences detected - review recommended.");
        std::process::exit(2);
    }

    println!("✅ Integrity check passed.");
    Ok(())
}
