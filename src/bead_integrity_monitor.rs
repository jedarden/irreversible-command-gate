//! Bead integrity verification service
//!
//! Continuous monitoring service for bead database health with automatic
//! repair and Prometheus metrics export. Runs on a 5-minute internal loop,
//! checking bead database integrity and triggering repairs when needed.
//!
//! ## Architecture
//!
//! The monitor runs as a background task and:
//! - Executes `bead doctor --json` every 5 minutes
//! - Triggers `bead doctor --repair` when issues are detected
//! - Publishes diagnostic reports to `.beads/diagnostics/integrity-report.jsonl`
//! - Exposes Prometheus metrics for bead counts by status and assignee state
//! - Provides HTTP health check endpoints
//!
//! ## Alert Threshold
//!
//! The service only generates alerts when issues exceed configurable thresholds
//! (default: >10 stuck beads). This shifts from reactive to proactive monitoring.

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::process::Command;
use std::time::Duration;
use tokio::time::interval;
use tokio::sync::{oneshot, Mutex};

/// Configuration for the bead integrity monitor
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IntegrityMonitorConfig {
    /// Path to the workspace root (contains .beads directory)
    pub workspace_path: PathBuf,

    /// Interval between health checks (default: 5 minutes)
    #[serde(default = "default_check_interval")]
    pub check_interval: Duration,

    /// Alert threshold for stuck beads (default: 10)
    #[serde(default = "default_alert_threshold")]
    pub alert_threshold: usize,

    /// Enable auto-repair when issues are detected
    #[serde(default = "default_auto_repair_enabled")]
    pub auto_repair_enabled: bool,

    /// HTTP server configuration
    #[serde(default)]
    pub http_config: HttpServerConfig,
}

fn default_check_interval() -> Duration {
    Duration::from_secs(300) // 5 minutes
}

fn default_alert_threshold() -> usize {
    10
}

fn default_auto_repair_enabled() -> bool {
    true
}

impl Default for IntegrityMonitorConfig {
    fn default() -> Self {
        Self {
            workspace_path: PathBuf::from("."),
            check_interval: default_check_interval(),
            alert_threshold: default_alert_threshold(),
            auto_repair_enabled: default_auto_repair_enabled(),
            http_config: HttpServerConfig::default(),
        }
    }
}

impl IntegrityMonitorConfig {
    /// Load configuration from environment variables
    pub fn from_environment() -> Self {
        let mut config = Self::default();

        if let Ok(path) = std::env::var("ICG_WORKSPACE_PATH") {
            config.workspace_path = PathBuf::from(path);
        }

        if let Ok(seconds) = std::env::var("ICG_CHECK_INTERVAL_SECONDS") {
            if let Ok(seconds) = seconds.parse::<u64>() {
                config.check_interval = Duration::from_secs(seconds.max(60));
            }
        }

        if let Ok(threshold) = std::env::var("ICG_ALERT_THRESHOLD") {
            if let Ok(threshold) = threshold.parse::<usize>() {
                config.alert_threshold = threshold;
            }
        }

        if let Ok(enabled) = std::env::var("ICG_AUTO_REPAIR_ENABLED") {
            config.auto_repair_enabled = enabled.eq_ignore_ascii_case("true") || enabled == "1";
        }

        // HTTP server config from environment
        config.http_config = HttpServerConfig::from_environment();

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

    /// Get the path to the integrity report JSONL file
    pub fn integrity_report_path(&self) -> PathBuf {
        self.diagnostics_dir().join("integrity-report.jsonl")
    }
}

/// HTTP server configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HttpServerConfig {
    /// Host address to bind to (default: "0.0.0.0")
    #[serde(default = "default_http_host")]
    pub host: String,

    /// Port to listen on (default: 9095)
    #[serde(default = "default_http_port")]
    pub port: u16,

    /// Enable liveness probe endpoint
    #[serde(default = "default_liveness_enabled")]
    pub liveness_enabled: bool,

    /// Enable readiness probe endpoint
    #[serde(default = "default_readiness_enabled")]
    pub readiness_enabled: bool,

    /// Enable metrics endpoint
    #[serde(default = "default_metrics_enabled")]
    pub metrics_enabled: bool,
}

fn default_http_host() -> String {
    "0.0.0.0".to_string()
}

fn default_http_port() -> u16 {
    9095
}

fn default_liveness_enabled() -> bool {
    true
}

fn default_readiness_enabled() -> bool {
    true
}

fn default_metrics_enabled() -> bool {
    true
}

impl Default for HttpServerConfig {
    fn default() -> Self {
        Self {
            host: default_http_host(),
            port: default_http_port(),
            liveness_enabled: default_liveness_enabled(),
            readiness_enabled: default_readiness_enabled(),
            metrics_enabled: default_metrics_enabled(),
        }
    }
}

impl HttpServerConfig {
    /// Load HTTP configuration from environment variables
    pub fn from_environment() -> Self {
        let mut config = Self::default();

        if let Ok(host) = std::env::var("ICG_MONITOR_HOST") {
            config.host = host;
        }

        if let Ok(port) = std::env::var("ICG_MONITOR_PORT") {
            if let Ok(port) = port.parse::<u16>() {
                config.port = port;
            }
        }

        config
    }

    /// Get the bind address
    pub fn bind_address(&self) -> String {
        format!("{}:{}", self.host, self.port)
    }
}

/// Bead doctor check result (JSON output format)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DoctorCheck {
    pub name: String,
    pub status: String,
    pub message: String,
    pub scope: String,
    #[serde(default)]
    pub details: Option<serde_json::Value>,
}

/// Bead doctor output (JSON format)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DoctorOutput {
    pub checks: Vec<DoctorCheck>,
}

/// Integrity check report
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IntegrityReport {
    /// Timestamp when the check was performed
    pub timestamp: DateTime<Utc>,

    /// Check interval in seconds
    pub check_interval_seconds: u64,

    /// Overall health status
    pub status: String,

    /// Total number of checks performed
    pub total_checks: usize,

    /// Number of checks that passed
    pub passed_checks: usize,

    /// Number of checks that failed
    pub failed_checks: usize,

    /// Number of checks with warnings
    pub warning_checks: usize,

    /// Individual check results
    pub checks: Vec<DoctorCheck>,

    /// Whether auto-repair was triggered
    pub repair_triggered: bool,

    /// Repair outcome (if repair was triggered)
    pub repair_result: Option<RepairResult>,

    /// Whether alert threshold was exceeded
    pub alert_triggered: bool,

    /// Alert reason (if triggered)
    pub alert_reason: Option<String>,
}

/// Repair operation result
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RepairResult {
    /// Whether the repair was successful
    pub success: bool,

    /// Number of issues repaired
    pub issues_repaired: usize,

    /// Repair output or error message
    pub message: String,

    /// Timestamp when repair was performed
    pub timestamp: DateTime<Utc>,
}

/// Bead metrics from the database
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BeadMetrics {
    /// Total number of beads
    pub total_beads: usize,

    /// Beads by status
    pub beads_by_status: HashMap<String, usize>,

    /// Beads by assignee
    pub beads_by_assignee: HashMap<String, usize>,

    /// Assigned-but-open beads (stuck beads)
    pub stuck_beads: usize,

    /// Timestamp when metrics were collected
    pub timestamp: DateTime<Utc>,
}

/// Current state of the integrity monitor
#[derive(Debug, Clone)]
pub struct MonitorState {
    /// Last integrity report
    pub last_report: Option<IntegrityReport>,

    /// Current bead metrics
    pub bead_metrics: Option<BeadMetrics>,

    /// Whether the monitor is healthy
    pub healthy: bool,

    /// Process start time
    pub start_time: DateTime<Utc>,

    /// Last check timestamp
    pub last_check: Option<DateTime<Utc>>,
}

impl Default for MonitorState {
    fn default() -> Self {
        Self {
            last_report: None,
            bead_metrics: None,
            healthy: true,
            start_time: Utc::now(),
            last_check: None,
        }
    }
}

/// Bead integrity monitor service
pub struct IntegrityMonitor {
    config: IntegrityMonitorConfig,
    state: MonitorState,
    shutdown_tx: Option<oneshot::Sender<()>>,
}

impl IntegrityMonitor {
    /// Create a new integrity monitor with the given configuration
    pub fn new(config: IntegrityMonitorConfig) -> Self {
        Self {
            config,
            state: MonitorState::default(),
            shutdown_tx: None,
        }
    }

    /// Get the current monitor state
    pub fn state(&self) -> &MonitorState {
        &self.state
    }

    /// Get the configuration
    pub fn config(&self) -> &IntegrityMonitorConfig {
        &self.config
    }

    /// Run a single integrity check
    pub fn run_check(&mut self) -> Result<IntegrityReport> {
        let timestamp = Utc::now();

        // Ensure diagnostics directory exists
        std::fs::create_dir_all(self.config.diagnostics_dir())
            .context("Failed to create diagnostics directory")?;

        // Run bead doctor --json
        let doctor_output = self.run_doctor_check()?;

        // Analyze results
        let mut failed_checks = 0;
        let mut warning_checks = 0;
        let mut passed_checks = 0;

        for check in &doctor_output.checks {
            match check.status.as_str() {
                "ok" => passed_checks += 1,
                "warn" => warning_checks += 1,
                "error" | "failed" => failed_checks += 1,
                _ => passed_checks += 1,
            }
        }

        let total_checks = doctor_output.checks.len();
        let status = if failed_checks > 0 {
            "unhealthy"
        } else if warning_checks > 0 {
            "degraded"
        } else {
            "healthy"
        };

        // Check if repair is needed
        let repair_triggered = self.config.auto_repair_enabled
            && (failed_checks > 0 || self.has_stale_in_progress(&doctor_output));

        let repair_result = if repair_triggered {
            Some(self.run_repair()?)
        } else {
            None
        };

        // Collect bead metrics
        let bead_metrics = self.collect_bead_metrics()?;

        // Check alert threshold
        let stuck_beads = bead_metrics.stuck_beads;
        let alert_triggered = stuck_beads > self.config.alert_threshold;
        let alert_reason = if alert_triggered {
            Some(format!(
                "Stuck bead count ({}) exceeds threshold ({})",
                stuck_beads, self.config.alert_threshold
            ))
        } else {
            None
        };

        let report = IntegrityReport {
            timestamp,
            check_interval_seconds: self.config.check_interval.as_secs(),
            status: status.to_string(),
            total_checks,
            passed_checks,
            failed_checks,
            warning_checks,
            checks: doctor_output.checks,
            repair_triggered,
            repair_result,
            alert_triggered,
            alert_reason,
        };

        // Publish report to JSONL file
        self.publish_report(&report)?;

        // Update state
        self.state.last_report = Some(report.clone());
        self.state.bead_metrics = Some(bead_metrics);
        self.state.last_check = Some(timestamp);
        self.state.healthy = failed_checks == 0;

        Ok(report)
    }

    /// Run bead doctor --json and parse the output
    fn run_doctor_check(&self) -> Result<DoctorOutput> {
        let output = Command::new("bead")
            .args(["doctor", "--json"])
            .current_dir(&self.config.workspace_path)
            .output()
            .context("Failed to run bead doctor")?;

        if !output.status.success() {
            anyhow::bail!(
                "bead doctor failed with exit code: {:?}, stderr: {}",
                output.status.code(),
                String::from_utf8_lossy(&output.stderr)
            );
        }

        let json = String::from_utf8(output.stdout)
            .context("bead doctor output is not valid UTF-8")?;

        serde_json::from_str(&json).context("Failed to parse bead doctor JSON output")
    }

    /// Check if the doctor output indicates stale in-progress beads
    fn has_stale_in_progress(&self, doctor_output: &DoctorOutput) -> bool {
        doctor_output
            .checks
            .iter()
            .any(|check| check.name == "stale_in_progress" && check.status != "ok")
    }

    /// Run bead doctor --repair and return the result
    fn run_repair(&self) -> Result<RepairResult> {
        let timestamp = Utc::now();

        let output = Command::new("bead")
            .args(["doctor", "--repair", "--json"])
            .current_dir(&self.config.workspace_path)
            .output()
            .context("Failed to run bead doctor --repair")?;

        let success = output.status.success();
        let stdout = String::from_utf8_lossy(&output.stdout).to_string();
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();

        // Parse repair output to count fixed issues
        let issues_repaired = if success {
            self.count_fixed_issues(&stdout)
        } else {
            0
        };

        let message = if success {
            format!("Repair completed: {}", stdout.trim())
        } else {
            format!("Repair failed: {}", stderr.trim())
        };

        Ok(RepairResult {
            success,
            issues_repaired,
            message,
            timestamp,
        })
    }

    /// Count the number of fixed issues from repair output
    fn count_fixed_issues(&self, output: &str) -> usize {
        // Look for patterns like "FIXED - Issue was repaired"
        output
            .lines()
            .filter(|line| line.contains("FIXED") || line.contains("Repaired"))
            .count()
    }

    /// Collect bead metrics from the database
    fn collect_bead_metrics(&self) -> Result<BeadMetrics> {
        let timestamp = Utc::now();

        // Use bead list commands to gather metrics
        let total_beads = self.count_beads()?;

        let mut beads_by_status = HashMap::new();
        for status in &["open", "in_progress", "closed", "deferred"] {
            let count = self.count_beads_by_status(status)?;
            beads_by_status.insert(status.to_string(), count);
        }

        let mut beads_by_assignee = HashMap::new();
        // Get beads by assignee using --json output
        let assignees = self.get_bead_assignees()?;
        for (assignee, count) in assignees {
            beads_by_assignee.insert(assignee, count);
        }

        // Count stuck beads (assigned but open)
        let stuck_beads = self.count_stuck_beads()?;

        Ok(BeadMetrics {
            total_beads,
            beads_by_status,
            beads_by_assignee,
            stuck_beads,
            timestamp,
        })
    }

    /// Count total beads
    fn count_beads(&self) -> Result<usize> {
        let output = Command::new("bead")
            .args(["list", "--json"])
            .current_dir(&self.config.workspace_path)
            .output()
            .context("Failed to run bead list")?;

        if !output.status.success() {
            anyhow::bail!("bead list failed");
        }

        // bead list --json outputs JSONL (one JSON object per line)
        let json = String::from_utf8(output.stdout)?;
        Ok(json.lines().filter(|line| !line.trim().is_empty()).count())
    }

    /// Count beads by status
    fn count_beads_by_status(&self, status: &str) -> Result<usize> {
        let output = Command::new("bead")
            .args(["list", &format!("--status={}", status), "--json"])
            .current_dir(&self.config.workspace_path)
            .output()
            .context("Failed to run bead list --status")?;

        if !output.status.success() {
            return Ok(0);
        }

        // bead list --json outputs JSONL (one JSON object per line)
        let json = String::from_utf8(output.stdout)?;
        Ok(json.lines().filter(|line| !line.trim().is_empty()).count())
    }

    /// Get bead assignees and their counts
    fn get_bead_assignees(&self) -> Result<HashMap<String, usize>> {
        let output = Command::new("bead")
            .args(["list", "--status=in_progress", "--json"])
            .current_dir(&self.config.workspace_path)
            .output()
            .context("Failed to run bead list for assignees")?;

        if !output.status.success() {
            return Ok(HashMap::new());
        }

        // bead list --json outputs JSONL (one JSON object per line)
        let json = String::from_utf8(output.stdout)?;
        let mut assignees = HashMap::new();

        for line in json.lines() {
            if line.trim().is_empty() {
                continue;
            }
            if let Ok(bead) = serde_json::from_str::<serde_json::Value>(line) {
                if let Some(assignee) = bead.get("assignee").and_then(|v| v.as_str()) {
                    *assignees.entry(assignee.to_string()).or_insert(0) += 1;
                }
            }
        }

        Ok(assignees)
    }

    /// Count stuck beads (assigned but open)
    fn count_stuck_beads(&self) -> Result<usize> {
        let output = Command::new("bead")
            .args(["list", "--status=open", "--json"])
            .current_dir(&self.config.workspace_path)
            .output()
            .context("Failed to run bead list for stuck beads")?;

        if !output.status.success() {
            return Ok(0);
        }

        // bead list --json outputs JSONL (one JSON object per line)
        let json = String::from_utf8(output.stdout)?;
        let mut stuck_count = 0;

        for line in json.lines() {
            if line.trim().is_empty() {
                continue;
            }
            if let Ok(bead) = serde_json::from_str::<serde_json::Value>(line) {
                if bead.get("assignee").and_then(|v| v.as_str()).is_some() {
                    stuck_count += 1;
                }
            }
        }

        Ok(stuck_count)
    }

    /// Publish the integrity report to the JSONL file
    fn publish_report(&self, report: &IntegrityReport) -> Result<()> {
        let report_path = self.config.integrity_report_path();
        let json_line = serde_json::to_string(report)
            .context("Failed to serialize integrity report")?;

        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&report_path)
            .context("Failed to open integrity report file")?;

        use std::io::Write;
        writeln!(file, "{}", json_line)
            .context("Failed to write integrity report")?;

        eprintln!(
            "📋 Integrity report published to {}",
            report_path.display()
        );

        Ok(())
    }

    /// Export Prometheus metrics
    pub fn export_prometheus(&self) -> String {
        let mut output = String::new();

        output.push_str("# Bead integrity monitor metrics\n");

        // Monitor uptime
        let uptime = Utc::now().signed_duration_since(self.state.start_time);
        output.push_str(&format!(
            "icg_integrity_monitor_uptime_seconds {}\n",
            uptime.num_seconds() as f64
        ));

        // Last check timestamp
        if let Some(last_check) = self.state.last_check {
            output.push_str(&format!(
                "icg_integrity_monitor_last_check_timestamp {}\n",
                last_check.timestamp()
            ));
        }

        // Health status
        output.push_str(&format!(
            "icg_integrity_monitor_healthy {}\n",
            if self.state.healthy { 1 } else { 0 }
        ));

        // Bead metrics
        if let Some(ref metrics) = self.state.bead_metrics {
            output.push_str("\n# Bead database metrics\n");
            output.push_str(&format!(
                "icg_beads_total {}\n",
                metrics.total_beads
            ));

            for (status, count) in &metrics.beads_by_status {
                let status_safe = status.replace('-', "_");
                output.push_str(&format!(
                    "icg_beads_by_status{{status=\"{}\"}} {}\n",
                    status_safe, count
                ));
            }

            for (assignee, count) in &metrics.beads_by_assignee {
                let assignee_safe = assignee.replace('-', "_").replace(':', "_");
                output.push_str(&format!(
                    "icg_beads_by_assignee{{assignee=\"{}\"}} {}\n",
                    assignee_safe, count
                ));
            }

            output.push_str(&format!(
                "icg_beads_stuck_total {}\n",
                metrics.stuck_beads
            ));
        }

        // Last integrity check results
        if let Some(ref report) = self.state.last_report {
            output.push_str("\n# Last integrity check results\n");
            output.push_str(&format!(
                "icg_integrity_check_total {}\n",
                report.total_checks
            ));
            output.push_str(&format!(
                "icg_integrity_check_passed {}\n",
                report.passed_checks
            ));
            output.push_str(&format!(
                "icg_integrity_check_failed {}\n",
                report.failed_checks
            ));
            output.push_str(&format!(
                "icg_integrity_check_warning {}\n",
                report.warning_checks
            ));
            output.push_str(&format!(
                "icg_integrity_repair_triggered {}\n",
                if report.repair_triggered { 1 } else { 0 }
            ));
            output.push_str(&format!(
                "icg_integrity_alert_triggered {}\n",
                if report.alert_triggered { 1 } else { 0 }
            ));
        }

        output
    }

    /// Start the monitoring loop
    pub async fn run(&mut self) -> Result<()> {
        eprintln!("🩺 Bead integrity monitor starting");
        eprintln!("📁 Workspace: {}", self.config.workspace_path.display());
        eprintln!("⏱️  Check interval: {} seconds", self.config.check_interval.as_secs());
        eprintln!("🚨 Alert threshold: {} stuck beads", self.config.alert_threshold);
        eprintln!("🔧 Auto-repair: {}", self.config.auto_repair_enabled);

        // Create diagnostics directory
        std::fs::create_dir_all(self.config.diagnostics_dir())
            .context("Failed to create diagnostics directory")?;

        // Run initial check
        self.run_check()?;

        let mut timer = interval(self.config.check_interval);
        timer.tick().await; // Skip the immediate tick

        loop {
            timer.tick().await;

            match self.run_check() {
                Ok(report) => {
                    eprintln!("✅ Integrity check completed: status={}, failed={}, warning={}",
                        report.status, report.failed_checks, report.warning_checks);

                    if report.repair_triggered {
                        if let Some(ref repair) = report.repair_result {
                            eprintln!("🔧 Auto-repair: success={}, issues_fixed={}",
                                repair.success, repair.issues_repaired);
                        }
                    }

                    if report.alert_triggered {
                        if let Some(ref reason) = report.alert_reason {
                            eprintln!("🚨 ALERT TRIGGERED: {}", reason);
                        }
                    }
                }
                Err(e) => {
                    eprintln!("❌ Integrity check failed: {:#}", e);
                    self.state.healthy = false;
                }
            }
        }
    }

    /// Start the monitoring loop with HTTP server
    pub async fn run_with_http_server(&mut self) -> Result<()> {
        // Create HTTP server
        let http_config = self.config.http_config.clone();
        let bind_address = http_config.bind_address();

        // Spawn HTTP server in background
        let state = std::sync::Arc::new(Mutex::new(self.state.clone()));
        let http_state = state.clone();

        tokio::spawn(async move {
            if let Err(e) = start_http_server(http_config, http_state).await {
                eprintln!("❌ HTTP server error: {}", e);
            }
        });

        eprintln!("🌐 HTTP server listening on http://{}", bind_address);

        // Run monitoring loop
        self.run().await
    }

    /// Shutdown the monitor
    pub async fn shutdown(&mut self) -> Result<()> {
        if let Some(shutdown_tx) = self.shutdown_tx.take() {
            let _ = shutdown_tx.send(());
        }
        Ok(())
    }
}

/// Start the HTTP server for health and metrics endpoints
async fn start_http_server(
    config: HttpServerConfig,
    state: std::sync::Arc<Mutex<MonitorState>>,
) -> Result<()> {
    use tokio::net::TcpListener;

    let listener = TcpListener::bind(&config.bind_address()).await
        .with_context(|| format!("Failed to bind HTTP server to {}", config.bind_address()))?;

    eprintln!("🌐 HTTP server accepting connections on http://{}", config.bind_address());

    loop {
        let (stream, _) = listener.accept().await
            .context("Failed to accept connection")?;

        let state = state.clone();
        let config = config.clone();

        tokio::spawn(async move {
            if let Err(e) = handle_http_connection(stream, state, config).await {
                eprintln!("⚠️  HTTP connection error: {}", e);
            }
        });
    }
}

/// Handle a single HTTP connection
async fn handle_http_connection(
    mut stream: tokio::net::TcpStream,
    state: std::sync::Arc<Mutex<MonitorState>>,
    _config: HttpServerConfig,
) -> Result<()> {
    use tokio::io::AsyncReadExt;

    let mut reader = tokio::io::BufReader::new(&mut stream);
    let mut bytes = Vec::new();

    loop {
        let mut buf = [0u8; 1];
        reader.read_exact(&mut buf).await?;
        bytes.push(buf[0]);
        if bytes.ends_with(b"\r\n\r\n") {
            break;
        }
        if bytes.len() > 8192 {
            anyhow::bail!("Request too large");
        }
    }

    let request = String::from_utf8(bytes)?;
    let lines: Vec<&str> = request.lines().collect();
    if lines.is_empty() {
        anyhow::bail!("Empty request");
    }

    let request_line = lines[0];
    let parts: Vec<&str> = request_line.split_whitespace().collect();
    if parts.len() < 2 {
        anyhow::bail!("Invalid request line");
    }

    let method = parts[0];
    let path = parts[1];

    if method != "GET" {
        return send_response(&mut stream, 405, "Method Not Allowed", "text/plain", "Method not allowed").await;
    }

    let state_guard = state.lock().await;

    let uptime = Utc::now().signed_duration_since(state_guard.start_time);

    let (status_code, status_text, content_type, body) = match path {
        "/" | "/health" | "/health/status" => {
            let health_status = if state_guard.healthy {
                "healthy"
            } else {
                "unhealthy"
            };
            let body = serde_json::json!({
                "status": health_status,
                "timestamp": Utc::now(),
                "uptime_seconds": uptime.num_seconds() as f64,
                "last_check": state_guard.last_check,
                "healthy": state_guard.healthy
            });
            (200, "OK", "application/json", serde_json::to_string_pretty(&body)?)
        }
        "/health/live" => {
            let body = serde_json::json!({
                "status": "alive",
                "timestamp": Utc::now(),
                "uptime_seconds": uptime.num_seconds() as f64
            });
            (200, "OK", "application/json", serde_json::to_string_pretty(&body)?)
        }
        "/health/ready" => {
            let ready = state_guard.healthy;
            let body = if ready {
                serde_json::json!({
                    "status": "ready",
                    "timestamp": Utc::now(),
                    "uptime_seconds": uptime.num_seconds() as f64
                })
            } else {
                serde_json::json!({
                    "status": "not_ready",
                    "timestamp": Utc::now(),
                    "uptime_seconds": uptime.num_seconds() as f64,
                    "reason": "integrity checks failing"
                })
            };
            let status = if ready { 200 } else { 503 };
            let status_text = if ready { "OK" } else { "Service Unavailable" };
            (status, status_text, "application/json", serde_json::to_string_pretty(&body)?)
        }
        "/metrics" => {
            let metrics = format!(
                "# Health server metrics\n\
                icg_integrity_monitor_uptime_seconds {}\n\
                icg_integrity_monitor_healthy {}\n",
                uptime.num_seconds() as f64,
                if state_guard.healthy { 1 } else { 0 }
            );

            let mut full_metrics = metrics;
            if let Some(ref bead_metrics) = state_guard.bead_metrics {
                full_metrics.push_str("\n# Bead metrics\n");
                full_metrics.push_str(&format!("icg_beads_total {}\n", bead_metrics.total_beads));
                for (status, count) in &bead_metrics.beads_by_status {
                    let status_safe = status.replace('-', "_");
                    full_metrics.push_str(&format!(
                        "icg_beads_by_status{{status=\"{}\"}} {}\n",
                        status_safe, count
                    ));
                }
                for (assignee, count) in &bead_metrics.beads_by_assignee {
                    let assignee_safe = assignee.replace('-', "_").replace(':', "_");
                    full_metrics.push_str(&format!(
                        "icg_beads_by_assignee{{assignee=\"{}\"}} {}\n",
                        assignee_safe, count
                    ));
                }
                full_metrics.push_str(&format!("icg_beads_stuck_total {}\n", bead_metrics.stuck_beads));
            }

            if let Some(ref report) = state_guard.last_report {
                full_metrics.push_str("\n# Last check results\n");
                full_metrics.push_str(&format!("icg_integrity_check_total {}\n", report.total_checks));
                full_metrics.push_str(&format!("icg_integrity_check_passed {}\n", report.passed_checks));
                full_metrics.push_str(&format!("icg_integrity_check_failed {}\n", report.failed_checks));
                full_metrics.push_str(&format!("icg_integrity_check_warning {}\n", report.warning_checks));
                full_metrics.push_str(&format!("icg_integrity_repair_triggered {}\n", if report.repair_triggered { 1 } else { 0 }));
                full_metrics.push_str(&format!("icg_integrity_alert_triggered {}\n", if report.alert_triggered { 1 } else { 0 }));
            }

            (200, "OK", "text/plain", full_metrics)
        }
        _ => (404, "Not Found", "text/plain", "Not found".to_string()),
    };

    send_response(&mut stream, status_code, status_text, &content_type, &body).await
}

/// Send an HTTP response
async fn send_response(
    stream: &mut tokio::net::TcpStream,
    status_code: u16,
    status_text: &str,
    content_type: &str,
    body: &str,
) -> Result<()> {
    use tokio::io::AsyncWriteExt;

    let response = format!(
        "HTTP/1.1 {} {}\r\n\
        Content-Type: {}\r\n\
        Content-Length: {}\r\n\
        Connection: close\r\n\
        \r\n\
        {}",
        status_code,
        status_text,
        content_type,
        body.len(),
        body
    );

    stream.write_all(response.as_bytes()).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_default_config() {
        let config = IntegrityMonitorConfig::default();
        assert_eq!(config.check_interval, Duration::from_secs(300));
        assert_eq!(config.alert_threshold, 10);
        assert!(config.auto_repair_enabled);
    }

    #[test]
    fn test_config_from_environment() {
        std::env::set_var("ICG_CHECK_INTERVAL_SECONDS", "600");
        std::env::set_var("ICG_ALERT_THRESHOLD", "20");
        std::env::set_var("ICG_AUTO_REPAIR_ENABLED", "false");

        let config = IntegrityMonitorConfig::from_environment();
        assert_eq!(config.check_interval, Duration::from_secs(600));
        assert_eq!(config.alert_threshold, 20);
        assert!(!config.auto_repair_enabled);

        std::env::remove_var("ICG_CHECK_INTERVAL_SECONDS");
        std::env::remove_var("ICG_ALERT_THRESHOLD");
        std::env::remove_var("ICG_AUTO_REPAIR_ENABLED");
    }

    #[test]
    fn test_http_config_default() {
        let config = HttpServerConfig::default();
        assert_eq!(config.host, "0.0.0.0");
        assert_eq!(config.port, 9095);
        assert!(config.liveness_enabled);
        assert!(config.readiness_enabled);
        assert!(config.metrics_enabled);
    }

    #[test]
    fn test_http_config_bind_address() {
        let config = HttpServerConfig {
            host: "127.0.0.1".to_string(),
            port: 8080,
            ..Default::default()
        };
        assert_eq!(config.bind_address(), "127.0.0.1:8080");
    }

    #[test]
    fn test_monitor_state_default() {
        let state = MonitorState::default();
        assert!(state.healthy);
        assert!(state.last_report.is_none());
        assert!(state.bead_metrics.is_none());
    }

    #[test]
    fn test_integrity_report_serialization() {
        let report = IntegrityReport {
            timestamp: Utc::now(),
            check_interval_seconds: 300,
            status: "healthy".to_string(),
            total_checks: 5,
            passed_checks: 5,
            failed_checks: 0,
            warning_checks: 0,
            checks: vec![],
            repair_triggered: false,
            repair_result: None,
            alert_triggered: false,
            alert_reason: None,
        };

        let json = serde_json::to_string(&report).unwrap();
        assert!(json.contains("\"status\":\"healthy\""));
        assert!(json.contains("\"total_checks\":5"));
        assert!(json.contains("\"passed_checks\":5"));
    }
}
