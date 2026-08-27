use anyhow::{Context, Result};
use clap::Parser;
use icg::frontier_consistency_service::{
    FrontierConsistencyService, FrontierConsistencyServiceConfig,
};
use std::path::PathBuf;

/// Automated bead frontier consistency checker and repair tool
///
/// Detects and repairs beads that become invisible to the ready frontier
/// while still being open in the database. This addresses "starvation" issues
/// where Pluck sees no candidates despite open beads existing.
#[derive(Parser, Debug, Clone)]
#[command(author, version, about, long_about = None)]
struct Args {
    /// Path to the workspace root (contains .beads directory)
    #[arg(short, long, default_value = ".")]
    workspace: PathBuf,

    /// Run once and exit (default: continuous monitoring)
    #[arg(short, long)]
    once: bool,

    /// Disable auto-repair (diagnose only, don't repair)
    #[arg(long)]
    no_repair: bool,

    /// Disable alerts (log only, don't trigger alerts)
    #[arg(long)]
    no_alert: bool,

    /// Check interval in seconds (default: 300)
    #[arg(short, long, default_value = "300")]
    interval: u64,

    /// JSON output only (suppress human-readable output)
    #[arg(short, long)]
    json: bool,
}

impl Args {
    fn into_config(self) -> FrontierConsistencyServiceConfig {
        let mut config = FrontierConsistencyServiceConfig {
            workspace_path: self.workspace.clone(),
            ..Default::default()
        };

        config.auto_repair_enabled = !self.no_repair;
        config.alert_on_persistent = !self.no_alert;

        // Override interval if specified
        if self.interval != 300 {
            config.check_interval = std::time::Duration::from_secs(self.interval.max(60));
        }

        config
    }
}

fn main() -> Result<()> {
    let args = Args::parse();
    let config = args.clone().into_config();

    if args.json {
        // JSON output mode
        let mut service = FrontierConsistencyService::new(config);
        let report = service.run_once()?;

        // Output JSON report
        let json =
            serde_json::to_string_pretty(&report).context("Failed to serialize report to JSON")?;
        println!("{}", json);

        // Exit with appropriate code
        if report.alert_triggered {
            std::process::exit(1);
        } else if !report.discrepancies.is_empty() {
            std::process::exit(2);
        }
    } else {
        // Human-readable output mode
        run_human_readable(args)?;
    }

    Ok(())
}

fn run_human_readable(args: Args) -> Result<()> {
    let config = args.clone().into_config();
    let mut service = FrontierConsistencyService::new(config);

    println!("🧭 Frontier Consistency Checker");
    println!("═════════════════════════════════════════════════════");
    println!(
        "📁 Workspace: {}",
        service.config().workspace_path.display()
    );
    println!(
        "⏱️  Check interval: {} seconds",
        service.config().check_interval.as_secs()
    );
    println!("🔧 Auto-repair: {}", service.config().auto_repair_enabled);
    println!(
        "🚨 Alert on persistent: {}",
        service.config().alert_on_persistent
    );
    println!();

    if args.once {
        // Single run mode
        let report = service.run_once()?;
        display_report(&report);

        // Exit with appropriate code
        if report.alert_triggered {
            println!("\n❌ Alert triggered - persistent invisibility detected.");
            std::process::exit(1);
        } else if !report.discrepancies.is_empty() {
            println!("\n⚠️  Discrepancies detected - review recommended.");
            std::process::exit(2);
        }

        println!("\n✅ No discrepancies detected.");
    } else {
        // Continuous monitoring mode
        println!("🔄 Starting continuous monitoring mode");
        println!("   Press Ctrl+C to stop\n");

        // Create a runtime for the async service
        let rt = tokio::runtime::Runtime::new().context("Failed to create async runtime")?;

        rt.block_on(async { service.run().await })?;
    }

    Ok(())
}

fn display_report(report: &icg::frontier_consistency_service::ConsistencyCycleReport) {
    println!("┌─ Consistency Check Cycle Report ─────────────────────────────────┐");
    println!("│ 📊 Cycle Summary                                                 │");
    println!("├───────────────────────────────────────────────────────────────────┤");
    println!(" │ Duration: {:.2}s", report.duration_seconds);
    println!(" │ Database beads: {}", report.total_database_beads);
    println!(" │ Ready frontier beads: {}", report.total_ready_beads);
    println!(" │ Discrepancies found: {}", report.discrepancies.len());
    println!(" │ Diagnoses performed: {}", report.diagnoses.len());
    println!(" │ Repairs attempted: {}", report.repairs.len());
    println!(" │ Verifications: {}", report.verifications.len());
    println!(" │ Persistent issues: {}", report.persistent_reports.len());
    println!("└───────────────────────────────────────────────────────────────────┘");
    println!();

    // Display discrepancies
    if !report.discrepancies.is_empty() {
        println!("⚠️  Discrepancies Detected:");
        println!("═══════════════════════════════════════════════════════════════");

        for (i, discrepancy) in report.discrepancies.iter().enumerate() {
            println!("{}. {}", i + 1, discrepancy.bead_id);
            println!("   Title: {}", discrepancy.title);
            println!("   Status: {}", discrepancy.status);
            if let Some(ref assignee) = discrepancy.assignee {
                println!("   Assignee: {}", assignee);
            }
            println!("   Priority: P{}", discrepancy.priority);
            println!("   Exclusion: {}", discrepancy.exclusion_reason);
            if !discrepancy.blocking_dependencies.is_empty() {
                println!("   Blocking dependencies:");
                for dep in &discrepancy.blocking_dependencies {
                    println!("     - {} ({})", dep.blocker_issue_id, dep.kind);
                }
            }
            println!(
                "   Detected: {}",
                discrepancy.detected_at.format("%Y-%m-%d %H:%M:%S UTC")
            );
            println!();
        }
    }

    // Display diagnoses
    if !report.diagnoses.is_empty() {
        println!("🩺 Doctor Diagnoses:");
        println!("═══════════════════════════════════════════════════════════════");

        for (i, diagnosis) in report.diagnoses.iter().enumerate() {
            println!("{}. Bead: {}", i + 1, diagnosis.bead_id);
            println!("   Issues found: {}", diagnosis.issues_found);
            println!("   Fixable: {}", diagnosis.fixable);
            if !diagnosis.issues.is_empty() {
                println!("   Issues:");
                for issue in &diagnosis.issues {
                    println!("     - {}", issue);
                }
            }
            if let Some(ref error) = diagnosis.error {
                println!("   Error: {}", error);
            }
            println!();
        }
    }

    // Display repairs
    if !report.repairs.is_empty() {
        println!("🔧 Repair Attempts:");
        println!("═══════════════════════════════════════════════════════════════");

        for (i, repair) in report.repairs.iter().enumerate() {
            println!("{}. Bead: {}", i + 1, repair.bead_id);
            println!("   Success: {}", repair.success);
            if !repair.issues_repaired.is_empty() {
                println!("   Issues repaired:");
                for issue in &repair.issues_repaired {
                    println!("     - {}", issue);
                }
            }
            println!("   Now visible: {}", repair.bead_now_visible);
            if let Some(ref error) = repair.error {
                println!("   Error: {}", error);
            }
            println!();
        }
    }

    // Display persistent reports
    if !report.persistent_reports.is_empty() {
        println!("📋 Persistent Invisibility Reports:");
        println!("═══════════════════════════════════════════════════════════════");

        for (i, rep) in report.persistent_reports.iter().enumerate() {
            println!("{}. Bead: {}", i + 1, rep.bead_id);
            println!("   Title: {}", rep.title);
            println!("   Status: {}", rep.status);
            if let Some(ref assignee) = rep.assignee {
                println!("   Assignee: {}", assignee);
            }
            if !rep.dependencies.is_empty() {
                println!("   Dependencies:");
                for dep in &rep.dependencies {
                    println!(
                        "     - {} blocks {}",
                        dep.blocker_issue_id, dep.blocked_issue_id
                    );
                }
            }
            if !rep.labels.is_empty() {
                println!("   Labels: {}", rep.labels.join(", "));
            }
            println!("   Exclusion: {}", rep.exclusion_reason);
            println!("   Recommended: {}", rep.recommended_action);
            println!();
        }
    }

    // Display alert status
    if report.alert_triggered {
        println!("🚨 ALERT TRIGGERED");
        println!("═══════════════════════════════════════════════════════════════");
        if let Some(ref reason) = report.alert_reason {
            println!("Reason: {}", reason);
        }
        println!();
    }

    // JSON output for detailed inspection
    if let Ok(json) = serde_json::to_string_pretty(report) {
        println!("📄 Full JSON Report:");
        println!("{}", json);
    }
}
