//! Alerting infrastructure for operational monitoring and incident response
//!
//! This module provides a flexible alerting system that can send notifications
//! to various endpoints when anomalies or critical events are detected:
//!
//! - **Webhook**: HTTP POST notifications to arbitrary endpoints
//! - **Slack**: Rich-formatted messages to Slack channels
//! - **Email**: SMTP email notifications
//! - **PagerDuty**: Integration with PagerDuty incident response
//! - **Custom**: Extensible backend for other services
//!
//! ## Architecture
//!
//! Alerts flow through these stages:
//! 1. **Trigger**: Anomaly detection, health state change, or rule pack error
//! 2. **Evaluation**: Alert rules determine severity and routing
//! 3. **Enrichment**: Add context, metrics, and metadata
//! 4. **Delivery**: Send to configured backends
//! 5. **Deduplication**: Prevent alert spam with cooldown periods
//!
//! ## Usage
//!
//! ```rust
//! use icg::alerting::{AlertManager, AlertConfig, WebhookBackend};
//!
//! let config = AlertConfig::default();
//! let manager = AlertManager::new(config);
//!
//! // Send a critical alert
//! manager.alert_critical("Guard process crashed", "Crash type: SIGABRT").await?;
//! ```

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::time::Duration;

/// Alert severity levels
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AlertSeverity {
    /// Informational - no action required
    Info,
    /// Warning - attention needed but not critical
    Warning,
    /// Critical - immediate action required
    Critical,
}

impl AlertSeverity {
    /// Get the priority level for sorting (higher = more severe)
    pub fn priority(&self) -> u8 {
        match self {
            AlertSeverity::Info => 0,
            AlertSeverity::Warning => 1,
            AlertSeverity::Critical => 2,
        }
    }

    /// Get the emoji representation
    pub fn emoji(&self) -> &str {
        match self {
            AlertSeverity::Info => "ℹ️",
            AlertSeverity::Warning => "⚠️",
            AlertSeverity::Critical => "🚨",
        }
    }
}

/// Alert event types
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AlertType {
    /// Anomaly detected in deny-rate telemetry
    DenyRateAnomaly,
    /// Guard process crashed
    ProcessCrash,
    /// Guard health state changed
    HealthStateChanged,
    /// Rule pack loading failed
    RulePackLoadFailed,
    /// Rule pack update failed
    RulePackUpdateFailed,
    /// Automatic rollback occurred
    AutomaticRollback,
    /// Rule pack validation failed
    RulePackValidationFailed,
    /// Custom alert type
    Custom(String),
}

/// Alert event with full context
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AlertEvent {
    /// Unique identifier for this alert
    pub id: String,

    /// Timestamp when the alert was triggered
    pub timestamp: DateTime<Utc>,

    /// Alert type
    pub alert_type: AlertType,

    /// Severity level
    pub severity: AlertSeverity,

    /// Alert title/summary
    pub title: String,

    /// Detailed description
    pub description: String,

    /// Optional structured data attached to the alert
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context: Option<serde_json::Value>,

    /// Fingerprint for deduplication (alerts with same fingerprint within cooldown are suppressed)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fingerprint: Option<String>,

    /// Optional grouping key for alert correlation
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub grouping_key: Option<String>,
}

impl AlertEvent {
    /// Create a new alert event
    pub fn new(
        alert_type: AlertType,
        severity: AlertSeverity,
        title: String,
        description: String,
    ) -> Self {
        let timestamp = Utc::now();
        let id = format!(
            "alert-{}-{}",
            timestamp.timestamp_nanos_opt().unwrap_or_default(),
            std::process::id()
        );

        Self {
            id,
            timestamp,
            alert_type,
            severity,
            title,
            description,
            context: None,
            fingerprint: None,
            grouping_key: None,
        }
    }

    /// Add structured context to the alert
    pub fn with_context(mut self, context: serde_json::Value) -> Self {
        self.context = Some(context);
        self
    }

    /// Set fingerprint for deduplication
    pub fn with_fingerprint(mut self, fingerprint: String) -> Self {
        self.fingerprint = Some(fingerprint);
        self
    }

    /// Set grouping key for alert correlation
    pub fn with_grouping_key(mut self, grouping_key: String) -> Self {
        self.grouping_key = Some(grouping_key);
        self
    }

    /// Generate a default fingerprint from alert type and title
    pub fn default_fingerprint(&self) -> String {
        format!("{:?}:{}", self.alert_type, self.title)
    }
}

/// Alert backend trait for extensibility
#[async_trait::async_trait]
pub trait AlertBackend: Send + Sync {
    /// Send an alert to this backend
    async fn send_alert(&self, alert: &AlertEvent) -> Result<()>;

    /// Get the backend name for logging
    fn name(&self) -> &str;
}

/// Webhook configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WebhookConfig {
    /// Webhook URL
    pub url: String,

    /// Optional secret token for authentication
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub secret_token: Option<String>,

    /// Custom headers to include in the request
    #[serde(default)]
    pub headers: std::collections::HashMap<String, String>,

    /// Timeout for webhook requests
    #[serde(default = "default_webhook_timeout")]
    pub timeout: Duration,
}

fn default_webhook_timeout() -> Duration {
    Duration::from_secs(10)
}

impl Default for WebhookConfig {
    fn default() -> Self {
        Self {
            url: String::new(),
            secret_token: None,
            headers: std::collections::HashMap::new(),
            timeout: default_webhook_timeout(),
        }
    }
}

/// Webhook alert backend
pub struct WebhookBackend {
    config: WebhookConfig,
    client: Client,
}

impl WebhookBackend {
    /// Create a new webhook backend
    pub fn new(config: WebhookConfig) -> Self {
        Self {
            config,
            client: Client::new(),
        }
    }
}

#[async_trait::async_trait]
impl AlertBackend for WebhookBackend {
    async fn send_alert(&self, alert: &AlertEvent) -> Result<()> {
        if self.config.url.is_empty() {
            anyhow::bail!("Webhook URL is not configured");
        }

        let mut request = self
            .client
            .post(&self.config.url)
            .timeout(self.config.timeout)
            .json(alert);

        // Add secret token header if configured
        if let Some(ref token) = self.config.secret_token {
            request = request.header("Authorization", format!("Bearer {}", token));
        }

        // Add custom headers
        for (key, value) in &self.config.headers {
            request = request.header(key, value);
        }

        let response = request.send().await?;

        if response.status().is_success() {
            Ok(())
        } else {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            anyhow::bail!("Webhook request failed with status {}: {}", status, body);
        }
    }

    fn name(&self) -> &str {
        "webhook"
    }
}

/// Slack configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SlackConfig {
    /// Slack webhook URL
    pub webhook_url: String,

    /// Optional username for the bot (default: "ICG Monitor")
    #[serde(default)]
    pub username: String,

    /// Optional icon emoji for the bot
    #[serde(default)]
    pub icon_emoji: String,

    /// Optional channel to override the default webhook channel
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub channel: Option<String>,
}

impl Default for SlackConfig {
    fn default() -> Self {
        Self {
            webhook_url: String::new(),
            username: "ICG Monitor".to_string(),
            icon_emoji: ":shield:".to_string(),
            channel: None,
        }
    }
}

/// Slack webhook payload
#[derive(Debug, Serialize)]
struct SlackMessage {
    username: String,
    icon_emoji: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    channel: Option<String>,
    attachments: Vec<SlackAttachment>,
}

#[derive(Debug, Serialize)]
struct SlackAttachment {
    color: String,
    title: String,
    text: String,
    fields: Vec<SlackField>,
    #[serde(skip_serializing_if = "Option::is_none")]
    footer: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    ts: Option<i64>,
}

#[derive(Debug, Serialize)]
struct SlackField {
    title: String,
    value: String,
    short: bool,
}

/// Slack alert backend
pub struct SlackBackend {
    config: SlackConfig,
    client: Client,
}

impl SlackBackend {
    /// Create a new Slack backend
    pub fn new(config: SlackConfig) -> Self {
        Self {
            config,
            client: Client::new(),
        }
    }

    /// Convert alert severity to Slack color
    fn severity_to_color(severity: AlertSeverity) -> String {
        match severity {
            AlertSeverity::Info => "#36a64f".to_string(), // green
            AlertSeverity::Warning => "#ff9900".to_string(), // orange
            AlertSeverity::Critical => "#ff0000".to_string(), // red
        }
    }
}

#[async_trait::async_trait]
impl AlertBackend for SlackBackend {
    async fn send_alert(&self, alert: &AlertEvent) -> Result<()> {
        if self.config.webhook_url.is_empty() {
            anyhow::bail!("Slack webhook URL is not configured");
        }

        let color = Self::severity_to_color(alert.severity);
        let emoji = alert.severity.emoji();

        let fields = vec![
            SlackField {
                title: "Severity".to_string(),
                value: format!("{} {:?}", emoji, alert.severity),
                short: true,
            },
            SlackField {
                title: "Type".to_string(),
                value: format!("{:?}", alert.alert_type),
                short: true,
            },
            SlackField {
                title: "ID".to_string(),
                value: alert.id.clone(),
                short: false,
            },
            SlackField {
                title: "Timestamp".to_string(),
                value: alert.timestamp.to_rfc3339(),
                short: false,
            },
        ];

        let attachment = SlackAttachment {
            color,
            title: alert.title.clone(),
            text: alert.description.clone(),
            fields,
            footer: Some("ICG Guard Monitoring".to_string()),
            ts: Some(alert.timestamp.timestamp()),
        };

        let message = SlackMessage {
            username: self.config.username.clone(),
            icon_emoji: self.config.icon_emoji.clone(),
            channel: self.config.channel.clone(),
            attachments: vec![attachment],
        };

        let response = self
            .client
            .post(&self.config.webhook_url)
            .json(&message)
            .send()
            .await?;

        if response.status().is_success() {
            Ok(())
        } else {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            anyhow::bail!(
                "Slack webhook request failed with status {}: {}",
                status,
                body
            );
        }
    }

    fn name(&self) -> &str {
        "slack"
    }
}

/// Alert manager configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AlertConfig {
    /// Cooldown period for alerts with same fingerprint (default: 5 minutes)
    #[serde(default = "default_alert_cooldown")]
    pub cooldown: Duration,

    /// Enable alert deduplication
    #[serde(default = "default_alert_deduplication")]
    pub enable_deduplication: bool,

    /// Minimum severity level to send alerts
    #[serde(default = "default_min_severity")]
    pub min_severity: AlertSeverity,
}

fn default_alert_cooldown() -> Duration {
    Duration::from_secs(300) // 5 minutes
}

fn default_alert_deduplication() -> bool {
    true
}

fn default_min_severity() -> AlertSeverity {
    AlertSeverity::Warning
}

impl Default for AlertConfig {
    fn default() -> Self {
        Self {
            cooldown: default_alert_cooldown(),
            enable_deduplication: default_alert_deduplication(),
            min_severity: default_min_severity(),
        }
    }
}

/// Alert manager
pub struct AlertManager {
    config: AlertConfig,
    backends: Vec<Box<dyn AlertBackend>>,
    recent_alerts: std::collections::HashMap<String, DateTime<Utc>>,
}

impl AlertManager {
    /// Create a new alert manager
    pub fn new(config: AlertConfig) -> Self {
        Self {
            config,
            backends: Vec::new(),
            recent_alerts: std::collections::HashMap::new(),
        }
    }

    /// Add an alert backend
    pub fn add_backend(&mut self, backend: Box<dyn AlertBackend>) {
        self.backends.push(backend);
    }

    /// Check if alert should be sent (severity check and deduplication)
    fn should_send_alert(&mut self, alert: &AlertEvent) -> bool {
        // Check severity threshold
        if alert.severity.priority() < self.config.min_severity.priority() {
            return false;
        }

        // Check deduplication
        if self.config.enable_deduplication {
            let fingerprint = alert
                .fingerprint
                .as_ref()
                .unwrap_or(&alert.default_fingerprint())
                .clone();

            if let Some(&last_sent) = self.recent_alerts.get(&fingerprint) {
                let elapsed = Utc::now().signed_duration_since(last_sent);
                if elapsed.to_std().unwrap_or(Duration::ZERO) < self.config.cooldown {
                    return false; // Still in cooldown
                }
            }

            // Update last sent time
            self.recent_alerts.insert(fingerprint, alert.timestamp);
        }

        true
    }

    /// Send an alert to all configured backends
    pub async fn send_alert(&mut self, alert: AlertEvent) -> Result<()> {
        if !self.should_send_alert(&alert) {
            return Ok(());
        }

        if self.backends.is_empty() {
            anyhow::bail!("No alert backends configured");
        }

        let mut errors = Vec::new();

        for backend in &self.backends {
            if let Err(e) = backend.send_alert(&alert).await {
                errors.push(format!("{}: {}", backend.name(), e));
            }
        }

        if !errors.is_empty() {
            anyhow::bail!(
                "Failed to send alert to some backends: {}",
                errors.join(", ")
            );
        }

        Ok(())
    }

    /// Convenience method to send a critical alert
    pub async fn alert_critical(&mut self, title: &str, description: &str) -> Result<()> {
        let alert = AlertEvent::new(
            AlertType::Custom("critical".to_string()),
            AlertSeverity::Critical,
            title.to_string(),
            description.to_string(),
        );
        self.send_alert(alert).await
    }

    /// Convenience method to send a warning alert
    pub async fn alert_warning(&mut self, title: &str, description: &str) -> Result<()> {
        let alert = AlertEvent::new(
            AlertType::Custom("warning".to_string()),
            AlertSeverity::Warning,
            title.to_string(),
            description.to_string(),
        );
        self.send_alert(alert).await
    }

    /// Clean up old alert fingerprints (call periodically)
    pub fn cleanup_old_alerts(&mut self, older_than: Duration) {
        let now = Utc::now();
        self.recent_alerts.retain(|_, &mut last_sent| {
            now.signed_duration_since(last_sent)
                .to_std()
                .unwrap_or(Duration::ZERO)
                < older_than
        });
    }

    /// Get the number of backends configured
    pub fn backend_count(&self) -> usize {
        self.backends.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_alert_event_creation() {
        let alert = AlertEvent::new(
            AlertType::ProcessCrash,
            AlertSeverity::Critical,
            "Guard crashed".to_string(),
            "SIGABRT detected".to_string(),
        );

        assert!(!alert.id.is_empty());
        assert_eq!(alert.alert_type, AlertType::ProcessCrash);
        assert_eq!(alert.severity, AlertSeverity::Critical);
        assert_eq!(alert.title, "Guard crashed");
    }

    #[test]
    fn test_alert_event_with_context() {
        let context = serde_json::json!({
            "crash_type": "SIGABRT",
            "exit_code": 134,
            "uptime": 3600
        });

        let alert = AlertEvent::new(
            AlertType::ProcessCrash,
            AlertSeverity::Critical,
            "Guard crashed".to_string(),
            "SIGABRT detected".to_string(),
        )
        .with_context(context);

        assert!(alert.context.is_some());
    }

    #[test]
    fn test_alert_event_default_fingerprint() {
        let alert = AlertEvent::new(
            AlertType::ProcessCrash,
            AlertSeverity::Critical,
            "Guard crashed".to_string(),
            "SIGABRT detected".to_string(),
        );

        let fingerprint = alert.default_fingerprint();
        assert!(fingerprint.contains("ProcessCrash"));
        assert!(fingerprint.contains("Guard crashed"));
    }

    #[test]
    fn test_severity_priority() {
        assert_eq!(AlertSeverity::Info.priority(), 0);
        assert_eq!(AlertSeverity::Warning.priority(), 1);
        assert_eq!(AlertSeverity::Critical.priority(), 2);
    }

    #[test]
    fn test_severity_emoji() {
        assert_eq!(AlertSeverity::Info.emoji(), "ℹ️");
        assert_eq!(AlertSeverity::Warning.emoji(), "⚠️");
        assert_eq!(AlertSeverity::Critical.emoji(), "🚨");
    }

    #[test]
    fn test_alert_manager_deduplication() {
        let config = AlertConfig {
            cooldown: Duration::from_secs(60),
            enable_deduplication: true,
            min_severity: AlertSeverity::Warning,
        };

        let mut manager = AlertManager::new(config);

        // First alert should be sent
        let alert1 = AlertEvent::new(
            AlertType::ProcessCrash,
            AlertSeverity::Critical,
            "Test alert".to_string(),
            "Testing deduplication".to_string(),
        )
        .with_fingerprint("test-fingerprint".to_string());

        assert!(manager.should_send_alert(&alert1));

        // Second alert with same fingerprint should be suppressed
        let alert2 = alert1.clone();
        assert!(!manager.should_send_alert(&alert2));
    }

    #[test]
    fn test_alert_manager_severity_filter() {
        let config = AlertConfig {
            cooldown: Duration::from_secs(60),
            enable_deduplication: false,
            min_severity: AlertSeverity::Warning,
        };

        let mut manager = AlertManager::new(config);

        // Info alert should be filtered out
        let info_alert = AlertEvent::new(
            AlertType::Custom("test".to_string()),
            AlertSeverity::Info,
            "Info".to_string(),
            "Info message".to_string(),
        );

        assert!(!manager.should_send_alert(&info_alert));

        // Warning alert should pass through
        let warning_alert = AlertEvent::new(
            AlertType::Custom("test".to_string()),
            AlertSeverity::Warning,
            "Warning".to_string(),
            "Warning message".to_string(),
        );

        assert!(manager.should_send_alert(&warning_alert));
    }

    #[test]
    fn test_slack_color_mapping() {
        assert_eq!(
            SlackBackend::severity_to_color(AlertSeverity::Info),
            "#36a64f"
        );
        assert_eq!(
            SlackBackend::severity_to_color(AlertSeverity::Warning),
            "#ff9900"
        );
        assert_eq!(
            SlackBackend::severity_to_color(AlertSeverity::Critical),
            "#ff0000"
        );
    }

    #[test]
    fn test_alert_config_default() {
        let config = AlertConfig::default();
        assert_eq!(config.cooldown, Duration::from_secs(300));
        assert!(config.enable_deduplication);
        assert_eq!(config.min_severity, AlertSeverity::Warning);
    }

    #[test]
    fn test_webhook_config_default() {
        let config = WebhookConfig::default();
        assert_eq!(config.url, "");
        assert!(config.secret_token.is_none());
        assert_eq!(config.timeout, Duration::from_secs(10));
    }

    #[test]
    fn test_slack_config_default() {
        let config = SlackConfig::default();
        assert_eq!(config.webhook_url, "");
        assert_eq!(config.username, "ICG Monitor");
        assert_eq!(config.icon_emoji, ":shield:");
        assert!(config.channel.is_none());
    }
}
