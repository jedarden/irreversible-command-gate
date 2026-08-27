//! Checkpoint health monitor example
//!
//! Run this example to test the checkpoint monitoring system:
//!
//! ```bash
//! cargo run --example checkpoint-monitor
//! ```

use anyhow::Result;
use icg::checkpoint_monitor::{CheckpointMonitor, CheckpointMonitorConfig};

#[tokio::main]
async fn main() -> Result<()> {
    // Test with default configuration
    let config = CheckpointMonitorConfig::default();
    println!("🩺 Checkpoint Monitor Configuration:");
    println!("  📁 Workspace: {}", config.workspace_path.display());
    println!("  ⏱️  Check interval: {}s", config.check_interval.as_secs());
    println!(
        "  📊 Stale threshold: {} minutes",
        config.stale_threshold_minutes
    );
    println!("  🔧 Auto-repair: {}", config.auto_repair_enabled);
    println!();

    // Create monitor
    let mut monitor = CheckpointMonitor::with_config(config)?;

    // Run a single check
    println!("Running checkpoint health check...");
    let report = monitor.run_check()?;

    // Print summary
    monitor.print_summary(&report);

    // Export Prometheus metrics
    println!("\n=== Prometheus Metrics ===");
    let metrics = monitor.export_prometheus(Some(&report));
    println!("{}", metrics);

    Ok(())
}
