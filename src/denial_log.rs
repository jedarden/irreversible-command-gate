//! Denial log aggregation and analysis infrastructure
//!
//! This module provides structured logging and analysis for all denied operations,
//! enabling audit trails, forensic analysis, and operational intelligence.
//!
//! ## Architecture
//!
//! The denial log system works in these phases:
//! 1. **Collection**: Record each denial with full execution context
//! 2. **Storage**: Append to durable log with rotation and retention
//! 3. **Analysis**: Query patterns, detect abuse, generate reports
//! 4. **Alerting**: Trigger alerts on suspicious patterns
//!
//! ## Log Format
//!
//! Each denial is recorded as a structured JSON entry containing:
//! - Timestamp, rule pack ID, pattern ID
//! - Command or content that was denied
//! - User/session context (if available)
//! - Rule metadata (category, severity, rationale)
//! - System state (health, release ref)
//!
//! ## Usage
//!
//! ```rust
//! use icg::denial_log::{DenialLog, DenialRecord, DenialStore};
//!
//! let store = DenialStore::new("/var/log/icg/denials.jsonl")?;
//! let record = DenialRecord::from_check_result(check_result, context);
//! store.record_denial(record)?;
//! ```

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::fs::{File, OpenOptions};
use std::io::{BufRead, BufReader, BufWriter, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

/// Denial log configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DenialLogConfig {
    /// Maximum size of a single log file before rotation (bytes)
    pub max_file_size: u64,

    /// Maximum number of rotated files to retain
    pub max_rotated_files: usize,

    /// Enable or disable denial logging
    pub enabled: bool,

    /// Include full command/content in logs (may be sensitive)
    pub log_full_content: bool,

    /// Log retention period (older logs are auto-deleted)
    pub retention_days: u32,
}

impl Default for DenialLogConfig {
    fn default() -> Self {
        Self {
            max_file_size: 100 * 1024 * 1024, // 100 MB
            max_rotated_files: 10,
            enabled: true,
            log_full_content: true,
            retention_days: 90,
        }
    }
}

/// A single denial record with full context
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DenialRecord {
    /// Unique identifier for this denial
    pub id: String,

    /// Timestamp when the denial occurred
    pub timestamp: DateTime<Utc>,

    /// Rule pack ID that matched
    pub pack_id: String,

    /// Pattern ID that matched
    pub pattern_id: String,

    /// Category of the rule (e.g., "dangerous", "destructive", "production")
    pub category: String,

    /// Severity level of the denied operation
    pub severity: DenialSeverity,

    /// Human-readable reason for the denial
    pub reason: String,

    /// The denied command or content
    pub denied_input: DeniedInput,

    /// Execution context (session, user, etc.)
    pub context: ExecutionContext,

    /// System state at time of denial
    pub system_state: SystemState,

    /// Optional metadata attached by the rule
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<serde_json::Value>,

    /// Optional alert ID if this denial triggered an alert
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub alert_id: Option<String>,
}

/// Severity classification for denials
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DenialSeverity {
    /// Low severity (e.g., unsafe pattern but not destructive)
    Low,
    /// Medium severity (e.g., destructive operation in dev environment)
    Medium,
    /// High severity (e.g., destructive operation in production)
    High,
    /// Critical severity (e.g., security-related denial)
    Critical,
}

impl DenialSeverity {
    /// Get the priority level (higher = more severe)
    pub fn priority(&self) -> u8 {
        match self {
            DenialSeverity::Low => 0,
            DenialSeverity::Medium => 1,
            DenialSeverity::High => 2,
            DenialSeverity::Critical => 3,
        }
    }
}

/// The denied input (command or content)
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum DeniedInput {
    /// Command-mode denial (Bash command)
    Command {
        /// The full command that was denied
        command: String,

        /// Segments extracted from the command
        segments: Vec<String>,

        /// Working directory (if available)
        #[serde(default, skip_serializing_if = "Option::is_none")]
        working_dir: Option<String>,
    },

    /// Content-mode denial (file write/edit)
    Content {
        /// File path that was denied
        file_path: String,

        /// Content that was being written (may be truncated for size)
        content: String,

        /// Content size in bytes
        content_size: usize,
    },

    /// Batch content denial
    ContentBatch {
        /// Multiple file paths that were denied
        file_paths: Vec<String>,

        /// Total content size
        total_size: usize,
    },
}

/// Execution context for the denial
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecutionContext {
    /// Session ID if available
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,

    /// User identifier if available
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user: Option<String>,

    /// Repository name if available
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repository: Option<String>,

    /// Branch name if available
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,

    /// Tool invocation (e.g., "Bash", "Write", "Edit")
    #[serde(default)]
    pub tool: String,

    /// Hostname where the denial occurred
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hostname: Option<String>,
}

impl Default for ExecutionContext {
    fn default() -> Self {
        Self {
            session_id: None,
            user: None,
            repository: None,
            branch: None,
            tool: String::new(),
            hostname: None,
        }
    }
}

/// System state at the time of denial
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SystemState {
    /// Guard process uptime in seconds
    #[serde(default)]
    pub uptime_seconds: f64,

    /// Current release reference
    #[serde(default)]
    pub release_ref: String,

    /// Guard health status
    #[serde(default)]
    pub health_status: String,

    /// Total crashes seen so far
    #[serde(default)]
    pub total_crashes: u64,

    /// Current deny rate (0.0 to 1.0)
    #[serde(default)]
    pub deny_rate: f64,
}

impl Default for SystemState {
    fn default() -> Self {
        Self {
            uptime_seconds: 0.0,
            release_ref: String::new(),
            health_status: String::new(),
            total_crashes: 0,
            deny_rate: 0.0,
        }
    }
}

impl DenialRecord {
    /// Create a new denial record
    pub fn new(
        pack_id: String,
        pattern_id: String,
        category: String,
        severity: DenialSeverity,
        reason: String,
        denied_input: DeniedInput,
    ) -> Self {
        let timestamp = Utc::now();
        let id = format!(
            "denial-{}-{}",
            timestamp.timestamp_nanos_opt().unwrap_or_default(),
            std::process::id()
        );

        Self {
            id,
            timestamp,
            pack_id,
            pattern_id,
            category,
            severity,
            reason,
            denied_input,
            context: ExecutionContext::default(),
            system_state: SystemState::default(),
            metadata: None,
            alert_id: None,
        }
    }

    /// Set the execution context
    pub fn with_context(mut self, context: ExecutionContext) -> Self {
        self.context = context;
        self
    }

    /// Set the system state
    pub fn with_system_state(mut self, state: SystemState) -> Self {
        self.system_state = state;
        self
    }

    /// Attach metadata to the denial
    pub fn with_metadata(mut self, metadata: serde_json::Value) -> Self {
        self.metadata = Some(metadata);
        self
    }

    /// Associate an alert with this denial
    pub fn with_alert(mut self, alert_id: String) -> Self {
        self.alert_id = Some(alert_id);
        self
    }

    /// Check if this denial matches a pattern
    pub fn matches_pattern(&self, pattern: &Regex) -> bool {
        match &self.denied_input {
            DeniedInput::Command { command, .. } => pattern.is_match(command),
            DeniedInput::Content { file_path, .. } => pattern.is_match(file_path),
            DeniedInput::ContentBatch { file_paths, .. } => {
                file_paths.iter().any(|p| pattern.is_match(p))
            }
        }
    }
}

/// Denial log store manager
pub struct DenialStore {
    /// Path to the denial log file
    log_path: PathBuf,

    /// Configuration for the denial log
    config: DenialLogConfig,

    /// Thread-safe write lock
    write_lock: Arc<Mutex<()>>,
}

impl DenialStore {
    /// Create a denial store at the specified path
    pub fn new(log_path: PathBuf, config: DenialLogConfig) -> Result<Self> {
        // Ensure parent directory exists
        if let Some(parent) = log_path.parent() {
            if !parent.exists() {
                std::fs::create_dir_all(parent)
                    .with_context(|| format!("Failed to create denial log directory: {}", parent.display()))?;
            }
        }

        Ok(Self {
            log_path,
            config,
            write_lock: Arc::new(Mutex::new(())),
        })
    }

    /// Create a denial store with default configuration
    pub fn with_default_config(log_path: PathBuf) -> Result<Self> {
        Self::new(log_path, DenialLogConfig::default())
    }

    /// Get the default denial log path
    pub fn default_path() -> Result<PathBuf> {
        let log_dir = dirs::state_dir()
            .or_else(|| dirs::cache_dir())
            .context("Failed to determine system state/cache directory")?;

        Ok(log_dir.join("icg").join("denials.jsonl"))
    }

    /// Get the path used by this store
    pub fn path(&self) -> &Path {
        &self.log_path
    }

    /// Check if log rotation is needed
    fn needs_rotation(&self) -> Result<bool> {
        if !self.log_path.exists() {
            return Ok(false);
        }

        let metadata = std::fs::metadata(&self.log_path)?;
        Ok(metadata.len() >= self.config.max_file_size)
    }

    /// Rotate the log file
    fn rotate_log(&self) -> Result<()> {
        if !self.log_path.exists() {
            return Ok(());
        }

        // Find the next available rotation slot
        let mut rotation_index = 1;
        let parent = self.log_path
            .parent()
            .unwrap_or_else(|| Path::new("."));

        loop {
            let rotated_path = parent.join(format!("{}.{}", self.log_path.file_name().unwrap_or_default().to_string_lossy(), rotation_index));
            if !rotated_path.exists() {
                std::fs::rename(&self.log_path, &rotated_path)
                    .with_context(|| format!("Failed to rotate denial log to {}", rotated_path.display()))?;
                break;
            }
            rotation_index += 1;

            if rotation_index > self.config.max_rotated_files as u32 {
                // Delete the oldest file and rotate
                let oldest_path = parent.join(format!("{}.{}", self.log_path.file_name().unwrap_or_default().to_string_lossy(), self.config.max_rotated_files));
                if oldest_path.exists() {
                    std::fs::remove_file(&oldest_path)
                        .with_context(|| format!("Failed to remove oldest rotated log: {}", oldest_path.display()))?;
                }
                std::fs::rename(&self.log_path, &parent.join(format!("{}.{}", self.log_path.file_name().unwrap_or_default().to_string_lossy(), self.config.max_rotated_files)))
                    .with_context(|| format!("Failed to rotate denial log"))?;
                break;
            }
        }

        Ok(())
    }

    /// Record a denial to the log
    pub fn record_denial(&self, record: DenialRecord) -> Result<()> {
        if !self.config.enabled {
            return Ok(());
        }

        // Monitoring systems are often multi-tenant.  Keep the structured
        // rule metadata while omitting command arguments and file contents
        // unless an operator explicitly opts into full-content logging.
        let record = if self.config.log_full_content {
            record
        } else {
            redact_denial_input(record)
        };

        // Acquire write lock
        let _lock = self.write_lock.lock().map_err(|e| {
            anyhow::anyhow!("Failed to acquire denial log write lock: {}", e)
        })?;

        // Check if rotation is needed
        if self.needs_rotation()? {
            self.rotate_log()?;
        }

        // Open file in append mode
        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.log_path)
            .with_context(|| format!("Failed to open denial log: {}", self.log_path.display()))?;

        // Write the record as a single JSON line
        let mut writer = BufWriter::new(file);
        let json_line = serde_json::to_string(&record)
            .context("Failed to serialize denial record")?;

        writeln!(writer, "{}", json_line)
            .context("Failed to write denial record")?;

        writer.flush()
            .context("Failed to flush denial log")?;

        Ok(())
    }

    /// Query denials by pattern
    pub fn query_denials(&self, pattern: &Regex) -> Result<Vec<DenialRecord>> {
        if !self.log_path.exists() {
            return Ok(Vec::new());
        }

        let file = File::open(&self.log_path)
            .with_context(|| format!("Failed to open denial log: {}", self.log_path.display()))?;

        let reader = BufReader::new(file);
        let mut results = Vec::new();

        for line in reader.lines() {
            let line = line.with_context(|| format!("Failed to read line from denial log: {}", self.log_path.display()))?;

            if let Ok(record) = serde_json::from_str::<DenialRecord>(&line) {
                if record.matches_pattern(pattern) {
                    results.push(record);
                }
            }
        }

        Ok(results)
    }

    /// Get all denial records from the log
    pub fn get_all_denials(&self) -> Result<Vec<DenialRecord>> {
        if !self.log_path.exists() {
            return Ok(Vec::new());
        }

        let file = File::open(&self.log_path)
            .with_context(|| format!("Failed to open denial log: {}", self.log_path.display()))?;

        let reader = BufReader::new(file);
        let mut records = Vec::new();

        for line in reader.lines() {
            let line = line.with_context(|| format!("Failed to read line from denial log: {}", self.log_path.display()))?;

            if let Ok(record) = serde_json::from_str::<DenialRecord>(&line) {
                records.push(record);
            }
        }

        Ok(records)
    }

    /// Get denial statistics
    pub fn get_statistics(&self) -> Result<DenialStatistics> {
        let records = self.get_all_denials()?;

        if records.is_empty() {
            return Ok(DenialStatistics::default());
        }

        let total_denials = records.len();
        let mut by_category: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
        let mut by_severity: std::collections::HashMap<DenialSeverity, usize> = std::collections::HashMap::new();
        let mut by_pattern: std::collections::HashMap<String, usize> = std::collections::HashMap::new();

        for record in &records {
            *by_category.entry(record.category.clone()).or_insert(0) += 1;
            *by_severity.entry(record.severity).or_insert(0) += 1;
            *by_pattern.entry(record.pattern_id.clone()).or_insert(0) += 1;
        }

        let most_common_pattern = by_pattern
            .into_iter()
            .max_by_key(|(_, count)| *count)
            .map(|(pattern, _)| pattern);

        let timestamp_range = if let (Some(first), Some(last)) = (
            records.first().map(|r| r.timestamp),
            records.last().map(|r| r.timestamp),
        ) {
            Some((first, last))
        } else {
            None
        };

        Ok(DenialStatistics {
            total_denials,
            unique_patterns: by_category.len(),
            by_category,
            by_severity,
            most_common_pattern,
            timestamp_range,
        })
    }

    /// Clean up old log files based on retention policy
    pub fn cleanup_old_logs(&self) -> Result<usize> {
        if let Some(parent) = self.log_path.parent() {
            let retention_threshold = Utc::now() - chrono::Duration::days(self.config.retention_days as i64);
            let mut removed_count = 0;

            for entry in std::fs::read_dir(parent)? {
                let entry = entry?;
                let path = entry.path();

                // Check if this is a rotated log file
                if path.extension().and_then(|e| e.to_str()) == Some(&*self.log_path.file_name().unwrap_or_default().to_string_lossy())
                    || path.starts_with(self.log_path.with_extension(""))
                {
                    if let Ok(metadata) = entry.metadata() {
                        if let Ok(modified) = metadata.modified() {
                            let modified_chrono: DateTime<Utc> = modified.into();
                            if modified_chrono < retention_threshold {
                                std::fs::remove_file(&path)?;
                                removed_count += 1;
                            }
                        }
                    }
                }
            }

            Ok(removed_count)
        } else {
            Ok(0)
        }
    }
}

fn redact_denial_input(mut record: DenialRecord) -> DenialRecord {
    record.denied_input = match record.denied_input {
        DeniedInput::Command { .. } => DeniedInput::Command {
            command: "<redacted>".to_string(),
            segments: Vec::new(),
            working_dir: None,
        },
        DeniedInput::Content {
            file_path,
            content,
            content_size,
        } => DeniedInput::Content {
            file_path,
            content: "<redacted>".to_string(),
            content_size: content.len().max(content_size),
        },
        DeniedInput::ContentBatch {
            file_paths,
            total_size,
        } => DeniedInput::ContentBatch {
            file_paths,
            total_size,
        },
    };
    record
}

/// Denial log statistics
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DenialStatistics {
    /// Total number of denials in the log
    pub total_denials: usize,

    /// Number of unique patterns that have denied
    pub unique_patterns: usize,

    /// Denials grouped by category
    pub by_category: std::collections::HashMap<String, usize>,

    /// Denials grouped by severity
    pub by_severity: std::collections::HashMap<DenialSeverity, usize>,

    /// Most commonly triggered pattern
    pub most_common_pattern: Option<String>,

    /// Time range of denials in the log (first, last)
    pub timestamp_range: Option<(DateTime<Utc>, DateTime<Utc>)>,
}

impl Default for DenialStatistics {
    fn default() -> Self {
        Self {
            total_denials: 0,
            unique_patterns: 0,
            by_category: std::collections::HashMap::new(),
            by_severity: std::collections::HashMap::new(),
            most_common_pattern: None,
            timestamp_range: None,
        }
    }
}

/// Denial log analysis utility
pub struct DenialAnalyzer {
    store: DenialStore,
}

impl DenialAnalyzer {
    /// Create a new denial analyzer
    pub fn new(store: DenialStore) -> Self {
        Self { store }
    }

    /// Analyze denial patterns and detect anomalies
    pub fn analyze_patterns(&self) -> Result<DenialAnalysis> {
        let records = self.store.get_all_denials()?;
        let stats = self.store.get_statistics()?;

        // Detect patterns of abuse (e.g., repeated attempts by same session)
        let mut session_attempts: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
        let mut user_attempts: std::collections::HashMap<String, usize> = std::collections::HashMap::new();

        for record in &records {
            if let Some(ref session_id) = record.context.session_id {
                *session_attempts.entry(session_id.clone()).or_insert(0) += 1;
            }
            if let Some(ref user) = record.context.user {
                *user_attempts.entry(user.clone()).or_insert(0) += 1;
            }
        }

        // Find suspicious patterns (sessions/users with unusually high denial counts)
        let threshold = if stats.total_denials > 100 {
            stats.total_denials / 20 // 5% of total denials
        } else {
            10 // Fixed threshold for smaller datasets
        };

        let suspicious_sessions: Vec<_> = session_attempts
            .into_iter()
            .filter(|(_, count)| *count > threshold)
            .collect();

        let suspicious_users: Vec<_> = user_attempts
            .into_iter()
            .filter(|(_, count)| *count > threshold)
            .collect();

        let anomaly_detected = !suspicious_sessions.is_empty() || !suspicious_users.is_empty();

        Ok(DenialAnalysis {
            statistics: stats,
            suspicious_sessions,
            suspicious_users,
            anomaly_detected,
        })
    }

    /// Generate a denial report
    pub fn generate_report(&self) -> Result<String> {
        let analysis = self.analyze_patterns()?;
        let mut report = String::new();

        report.push_str("# Denial Log Analysis Report\n\n");

        // Overall statistics
        report.push_str("## Overall Statistics\n\n");
        report.push_str(&format!("**Total Denials:** {}\n", analysis.statistics.total_denials));
        report.push_str(&format!("**Unique Patterns:** {}\n", analysis.statistics.unique_patterns));

        if let Some((first, last)) = analysis.statistics.timestamp_range {
            report.push_str(&format!("**Time Range:** {} to {}\n", first, last));
        }

        report.push('\n');

        // Category breakdown
        report.push_str("### Denials by Category\n\n");
        let mut categories: Vec<_> = analysis.statistics.by_category.iter().collect();
        categories.sort_by(|a, b| b.1.cmp(a.1));

        for (category, count) in categories {
            let percentage = (*count as f64 / analysis.statistics.total_denials as f64) * 100.0;
            report.push_str(&format!("- **{}**: {} ({:.1}%)\n", category, count, percentage));
        }

        report.push('\n');

        // Severity breakdown
        report.push_str("### Denials by Severity\n\n");
        let mut severities: Vec<_> = analysis.statistics.by_severity.iter().collect();
        severities.sort_by(|a, b| b.1.cmp(a.1));

        for (severity, count) in severities {
            let percentage = (*count as f64 / analysis.statistics.total_denials as f64) * 100.0;
            report.push_str(&format!("- **{:?}**: {} ({:.1}%)\n", severity, count, percentage));
        }

        report.push('\n');

        // Suspicious activity
        if analysis.anomaly_detected {
            report.push_str("## ⚠️ Suspicious Activity Detected\n\n");

            if !analysis.suspicious_sessions.is_empty() {
                report.push_str("### High-Denial Sessions\n\n");
                for (session_id, count) in &analysis.suspicious_sessions {
                    report.push_str(&format!("- **Session {}**: {} denials\n", session_id, count));
                }
                report.push('\n');
            }

            if !analysis.suspicious_users.is_empty() {
                report.push_str("### High-Denial Users\n\n");
                for (user, count) in &analysis.suspicious_users {
                    report.push_str(&format!("- **User {}**: {} denials\n", user, count));
                }
                report.push('\n');
            }
        }

        // Most common pattern
        if let Some(ref pattern) = analysis.statistics.most_common_pattern {
            report.push_str("## Most Common Pattern\n\n");
            report.push_str(&format!("**Pattern ID:** `{}`\n", pattern));
            report.push('\n');
        }

        Ok(report)
    }
}

/// Denial analysis results
#[derive(Debug, Clone)]
pub struct DenialAnalysis {
    /// Basic statistics
    pub statistics: DenialStatistics,

    /// Sessions with unusually high denial counts
    pub suspicious_sessions: Vec<(String, usize)>,

    /// Users with unusually high denial counts
    pub suspicious_users: Vec<(String, usize)>,

    /// Whether any anomalies were detected
    pub anomaly_detected: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_denial_record_creation() {
        let record = DenialRecord::new(
            "test-pack".to_string(),
            "pattern-1".to_string(),
            "dangerous".to_string(),
            DenialSeverity::High,
            "Dangerous command detected".to_string(),
            DeniedInput::Command {
                command: "rm -rf /".to_string(),
                segments: vec!["rm".to_string(), "-rf".to_string(), "/".to_string()],
                working_dir: Some("/home/user".to_string()),
            },
        );

        assert!(!record.id.is_empty());
        assert_eq!(record.pack_id, "test-pack");
        assert_eq!(record.pattern_id, "pattern-1");
        assert_eq!(record.severity, DenialSeverity::High);
    }

    #[test]
    fn test_denial_record_with_context() {
        let context = ExecutionContext {
            session_id: Some("session-123".to_string()),
            user: Some("test-user".to_string()),
            tool: "Bash".to_string(),
            ..Default::default()
        };

        let record = DenialRecord::new(
            "test-pack".to_string(),
            "pattern-1".to_string(),
            "dangerous".to_string(),
            DenialSeverity::High,
            "Dangerous command".to_string(),
            DeniedInput::Command {
                command: "rm -rf /".to_string(),
                segments: vec!["rm".to_string()],
                working_dir: None,
            },
        )
        .with_context(context);

        assert_eq!(record.context.session_id, Some("session-123".to_string()));
        assert_eq!(record.context.user, Some("test-user".to_string()));
    }

    #[test]
    fn test_severity_priority() {
        assert_eq!(DenialSeverity::Low.priority(), 0);
        assert_eq!(DenialSeverity::Medium.priority(), 1);
        assert_eq!(DenialSeverity::High.priority(), 2);
        assert_eq!(DenialSeverity::Critical.priority(), 3);
    }

    #[test]
    fn test_denial_config_default() {
        let config = DenialLogConfig::default();
        assert_eq!(config.max_file_size, 100 * 1024 * 1024);
        assert_eq!(config.max_rotated_files, 10);
        assert!(config.enabled);
        assert!(config.log_full_content);
        assert_eq!(config.retention_days, 90);
    }

    #[test]
    fn test_denial_store_with_tempfile() -> Result<()> {
        let temp_dir = tempfile::tempdir()?;
        let log_path = temp_dir.path().join("denials.jsonl");

        let store = DenialStore::with_default_config(log_path)?;

        // Record a denial
        let record = DenialRecord::new(
            "test-pack".to_string(),
            "pattern-1".to_string(),
            "dangerous".to_string(),
            DenialSeverity::High,
            "Test".to_string(),
            DeniedInput::Command {
                command: "test".to_string(),
                segments: vec!["test".to_string()],
                working_dir: None,
            },
        );

        store.record_denial(record)?;

        // Verify it was written
        let records = store.get_all_denials()?;
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].pack_id, "test-pack");

        Ok(())
    }

    #[test]
    fn redacts_payloads_when_full_content_logging_is_disabled() -> Result<()> {
        let temp_dir = tempfile::tempdir()?;
        let log_path = temp_dir.path().join("denials.jsonl");
        let mut config = DenialLogConfig::default();
        config.log_full_content = false;
        let store = DenialStore::new(log_path.clone(), config)?;
        store.record_denial(DenialRecord::new(
            "secrets".to_string(),
            "token".to_string(),
            "security".to_string(),
            DenialSeverity::Critical,
            "secret was detected".to_string(),
            DeniedInput::Command {
                command: "echo super-secret-token".to_string(),
                segments: vec!["echo".to_string(), "super-secret-token".to_string()],
                working_dir: Some("/tmp".to_string()),
            },
        ))?;

        let serialized = std::fs::read_to_string(log_path)?;
        assert!(!serialized.contains("super-secret-token"));
        assert!(serialized.contains("<redacted>"));
        Ok(())
    }

    #[test]
    fn test_denial_statistics_empty() -> Result<()> {
        let temp_dir = tempfile::tempdir()?;
        let log_path = temp_dir.path().join("denials.jsonl");

        let store = DenialStore::with_default_config(log_path)?;
        let stats = store.get_statistics()?;

        assert_eq!(stats.total_denials, 0);
        assert_eq!(stats.unique_patterns, 0);

        Ok(())
    }

    #[test]
    fn test_denial_analyzer() -> Result<()> {
        let temp_dir = tempfile::tempdir()?;
        let log_path = temp_dir.path().join("denials.jsonl");

        let store = DenialStore::with_default_config(log_path)?;

        // Record multiple denials from the same session
        for i in 0..15 {
            let record = DenialRecord::new(
                "test-pack".to_string(),
                format!("pattern-{}", i),
                "dangerous".to_string(),
                DenialSeverity::High,
                "Test".to_string(),
                DeniedInput::Command {
                    command: format!("test {}", i),
                    segments: vec![format!("test"), i.to_string()],
                    working_dir: None,
                },
            )
            .with_context(ExecutionContext {
                session_id: Some("suspicious-session".to_string()),
                ..Default::default()
            });

            store.record_denial(record)?;
        }

        let analyzer = DenialAnalyzer::new(store);
        let analysis = analyzer.analyze_patterns()?;

        assert!(analysis.anomaly_detected);
        assert!(!analysis.suspicious_sessions.is_empty());

        Ok(())
    }
}
