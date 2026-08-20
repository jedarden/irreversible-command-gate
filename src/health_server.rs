//! HTTP health check endpoint for operational monitoring
//!
//! This module provides a lightweight HTTP server that exposes health check
//! endpoints suitable for Kubernetes probes, load balancer health checks,
//! and operational monitoring systems.
//!
//! ## Architecture
//!
//! The health server runs as a background task and exposes these endpoints:
//! - **GET /health**: Basic health check (always returns 200 if running)
//! - **GET /health/ready**: Readiness probe (returns 503 if not ready)
//! - **GET /health/live**: Liveness probe (returns 503 if process is dead)
//! - **GET /metrics**: Prometheus metrics export
//!
//! ## Usage
//!
//! ```rust
//! use icg::health_server::{HealthServer, HealthServerConfig};
//!
//! let config = HealthServerConfig::default();
//! let server = HealthServer::new(config)?;
//! server.spawn_background_task();
//! // Server runs in background, accessible via HTTP
//! ```
//!
//! ## Kubernetes Integration
//!
//! Example probe configuration:
//!
//! ```yaml
//! livenessProbe:
//!   httpGet:
//!     path: /health/live
//!     port: 8080
//!   initialDelaySeconds: 5
//!   periodSeconds: 10
//!
//! readinessProbe:
//!   httpGet:
//!     path: /health/ready
//!     port: 8080
//!   initialDelaySeconds: 5
//!   periodSeconds: 5
//! ```

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::sync::oneshot;

/// Health server configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HealthServerConfig {
    /// Host address to bind to (default: "0.0.0.0")
    #[serde(default = "default_health_host")]
    pub host: String,

    /// Port to listen on (default: 8080)
    #[serde(default = "default_health_port")]
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

    /// Request timeout for health checks
    #[serde(default = "default_request_timeout")]
    pub request_timeout: Duration,
}

fn default_health_host() -> String {
    "0.0.0.0".to_string()
}

fn default_health_port() -> u16 {
    8080
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

fn default_request_timeout() -> Duration {
    Duration::from_secs(5)
}

impl Default for HealthServerConfig {
    fn default() -> Self {
        Self {
            host: default_health_host(),
            port: default_health_port(),
            liveness_enabled: default_liveness_enabled(),
            readiness_enabled: default_readiness_enabled(),
            metrics_enabled: default_metrics_enabled(),
            request_timeout: default_request_timeout(),
        }
    }
}

/// Health status response
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HealthStatus {
    /// Overall health status
    pub status: String,

    /// Timestamp of the health check
    pub timestamp: DateTime<Utc>,

    /// Process uptime in seconds
    pub uptime_seconds: f64,

    /// Additional details about the health status
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub details: Option<serde_json::Value>,
}

impl HealthStatus {
    /// Create a healthy status
    pub fn healthy(uptime_seconds: f64) -> Self {
        Self {
            status: "healthy".to_string(),
            timestamp: Utc::now(),
            uptime_seconds,
            details: None,
        }
    }

    /// Create an unhealthy status with details
    pub fn unhealthy(uptime_seconds: f64, reason: String) -> Self {
        Self {
            status: "unhealthy".to_string(),
            timestamp: Utc::now(),
            uptime_seconds,
            details: Some(serde_json::json!({ "reason": reason })),
        }
    }

    /// Create a not ready status
    pub fn not_ready(uptime_seconds: f64, reason: String) -> Self {
        Self {
            status: "not_ready".to_string(),
            timestamp: Utc::now(),
            uptime_seconds,
            details: Some(serde_json::json!({ "reason": reason })),
        }
    }
}

/// Health check state
#[derive(Debug, Clone)]
pub struct HealthState {
    /// Whether the service is ready to serve traffic
    pub ready: bool,

    /// Whether the service is alive (process running)
    pub alive: bool,

    /// Process start time
    pub start_time: DateTime<Utc>,

    /// Optional reason if not ready
    pub not_ready_reason: Option<String>,

    /// Optional reason if not alive
    pub not_alive_reason: Option<String>,
}

impl Default for HealthState {
    fn default() -> Self {
        Self {
            ready: true,
            alive: true,
            start_time: Utc::now(),
            not_ready_reason: None,
            not_alive_reason: None,
        }
    }
}

impl HealthState {
    /// Get the current uptime in seconds
    pub fn uptime_seconds(&self) -> f64 {
        let elapsed = Utc::now().signed_duration_since(self.start_time);
        elapsed.num_seconds() as f64 + elapsed.num_nanoseconds().unwrap_or(0) as f64 / 1e9
    }

    /// Mark the service as not ready
    pub fn set_not_ready(&mut self, reason: String) {
        self.ready = false;
        self.not_ready_reason = Some(reason);
    }

    /// Mark the service as ready
    pub fn set_ready(&mut self) {
        self.ready = true;
        self.not_ready_reason = None;
    }

    /// Mark the service as not alive
    pub fn set_not_alive(&mut self, reason: String) {
        self.alive = false;
        self.not_alive_reason = Some(reason);
    }

    /// Mark the service as alive
    pub fn set_alive(&mut self) {
        self.alive = true;
        self.not_alive_reason = None;
    }
}

/// HTTP health check server
pub struct HealthServer {
    config: HealthServerConfig,
    state: Arc<Mutex<HealthState>>,
    shutdown_tx: Option<oneshot::Sender<()>>,
}

impl HealthServer {
    /// Create a new health server
    pub fn new(config: HealthServerConfig) -> Self {
        Self {
            config,
            state: Arc::new(Mutex::new(HealthState::default())),
            shutdown_tx: None,
        }
    }

    /// Get the current health state
    pub fn state(&self) -> Arc<Mutex<HealthState>> {
        self.state.clone()
    }

    /// Get the bind address
    pub fn bind_address(&self) -> String {
        format!("{}:{}", self.config.host, self.config.port)
    }

    /// Spawn the background task to run the server
    pub async fn spawn_background_task(&mut self) -> Result<()> {
        let state = self.state.clone();
        let config = self.config.clone();
        let bind_address = self.bind_address();

        let listener = TcpListener::bind(&bind_address)
            .await
            .with_context(|| format!("Failed to bind health server to {}", bind_address))?;

        eprintln!("🩺 Health server listening on http://{}", bind_address);

        let (shutdown_tx, mut shutdown_rx) = oneshot::channel();
        self.shutdown_tx = Some(shutdown_tx);

        tokio::spawn(async move {
            eprintln!("🩺 Health server accepting connections");

            loop {
                tokio::select! {
                    result = listener.accept() => {
                        match result {
                            Ok((stream, addr)) => {
                                let state = state.clone();
                                let config = config.clone();

                                tokio::spawn(async move {
                                    if let Err(e) = handle_connection(stream, state, config).await {
                                        eprintln!("⚠️  Health server connection error from {}: {}", addr, e);
                                    }
                                });
                            }
                            Err(e) => {
                                eprintln!("⚠️  Health server accept error: {}", e);
                            }
                        }
                    }
                    _ = &mut shutdown_rx => {
                        eprintln!("🩺 Health server shutting down");
                        break;
                    }
                }
            }
        });

        Ok(())
    }

    /// Shutdown the health server
    pub async fn shutdown(&mut self) -> Result<()> {
        if let Some(shutdown_tx) = self.shutdown_tx.take() {
            let _ = shutdown_tx.send(());
        }
        Ok(())
    }

    /// Check if the server is running
    pub fn is_running(&self) -> bool {
        self.shutdown_tx.is_some()
    }
}

/// Handle a single HTTP connection
async fn handle_connection(
    mut stream: tokio::net::TcpStream,
    state: Arc<Mutex<HealthState>>,
    config: HealthServerConfig,
) -> Result<()> {
    // Read the HTTP request
    let mut request_line = String::new();
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

    let request = String::from_utf8(bytes)
        .context("Invalid UTF-8 in request")?;

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

    // Route the request
    let (status_code, status_text, content_type, body) = match path {
        "/" | "/health" => {
            // Basic health check - always return 200 if server is running
            let health = HealthStatus::healthy(
                state.lock().map_err(|e| anyhow::anyhow!("Failed to lock state: {}", e))?.uptime_seconds()
            );
            (200, "OK", "application/json", serde_json::to_string_pretty(&health)?)
        }
        "/health/ready" => {
            if !config.readiness_enabled {
                (404, "Not Found", "text/plain", "Readiness probe disabled".to_string())
            } else {
                let state_guard = state.lock().map_err(|e| anyhow::anyhow!("Failed to lock state: {}", e))?;
                let uptime = state_guard.uptime_seconds();

                if state_guard.ready {
                    let health = HealthStatus::healthy(uptime);
                    (200, "OK", "application/json", serde_json::to_string_pretty(&health)?)
                } else {
                    let reason = state_guard.not_ready_reason.clone().unwrap_or_else(|| "Unknown reason".to_string());
                    let health = HealthStatus::not_ready(uptime, reason);
                    (503, "Service Unavailable", "application/json", serde_json::to_string_pretty(&health)?)
                }
            }
        }
        "/health/live" => {
            if !config.liveness_enabled {
                (404, "Not Found", "text/plain", "Liveness probe disabled".to_string())
            } else {
                let state_guard = state.lock().map_err(|e| anyhow::anyhow!("Failed to lock state: {}", e))?;
                let uptime = state_guard.uptime_seconds();

                if state_guard.alive {
                    let health = HealthStatus::healthy(uptime);
                    (200, "OK", "application/json", serde_json::to_string_pretty(&health)?)
                } else {
                    let reason = state_guard.not_alive_reason.as_ref().unwrap_or(&"Unknown reason".to_string()).clone();
                    let health = HealthStatus::unhealthy(uptime, reason);
                    (503, "Service Unavailable", "application/json", serde_json::to_string_pretty(&health)?)
                }
            }
        }
        "/metrics" => {
            if !config.metrics_enabled {
                (404, "Not Found", "text/plain", "Metrics endpoint disabled".to_string())
            } else {
                // Basic metrics - in a full implementation, this would call the MetricsExporter
                let state_guard = state.lock().map_err(|e| anyhow::anyhow!("Failed to lock state: {}", e))?;
                let uptime = state_guard.uptime_seconds();

                let metrics = format!(
                    "# Health server metrics\n\
                    icg_health_server_uptime_seconds {}\n\
                    icg_health_server_ready {}\n\
                    icg_health_server_alive {}\n",
                    uptime,
                    if state_guard.ready { 1 } else { 0 },
                    if state_guard.alive { 1 } else { 0 }
                );

                (200, "OK", "text/plain", metrics)
            }
        }
        _ => {
            (404, "Not Found", "text/plain", "Not found".to_string())
        }
    };

    send_response(&mut stream, status_code, status_text, content_type, &body).await
}

/// Send an HTTP response
async fn send_response(
    stream: &mut tokio::net::TcpStream,
    status_code: u16,
    status_text: &str,
    content_type: &str,
    body: &str,
) -> Result<()> {
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

    stream
        .write_all(response.as_bytes())
        .await
        .context("Failed to write response")?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_health_server_config_default() {
        let config = HealthServerConfig::default();
        assert_eq!(config.host, "0.0.0.0");
        assert_eq!(config.port, 8080);
        assert!(config.liveness_enabled);
        assert!(config.readiness_enabled);
        assert!(config.metrics_enabled);
        assert_eq!(config.request_timeout, Duration::from_secs(5));
    }

    #[test]
    fn test_health_status_healthy() {
        let status = HealthStatus::healthy(3600.0);
        assert_eq!(status.status, "healthy");
        assert_eq!(status.uptime_seconds, 3600.0);
        assert!(status.details.is_none());
    }

    #[test]
    fn test_health_status_unhealthy() {
        let status = HealthStatus::unhealthy(3600.0, "test failure".to_string());
        assert_eq!(status.status, "unhealthy");
        assert_eq!(status.uptime_seconds, 3600.0);
        assert!(status.details.is_some());
    }

    #[test]
    fn test_health_status_not_ready() {
        let status = HealthStatus::not_ready(3600.0, "initializing".to_string());
        assert_eq!(status.status, "not_ready");
        assert_eq!(status.uptime_seconds, 3600.0);
        assert!(status.details.is_some());
    }

    #[test]
    fn test_health_state_default() {
        let state = HealthState::default();
        assert!(state.ready);
        assert!(state.alive);
        assert!(state.not_ready_reason.is_none());
        assert!(state.not_alive_reason.is_none());
    }

    #[test]
    fn test_health_state_not_ready() {
        let mut state = HealthState::default();
        state.set_not_ready("initializing".to_string());
        assert!(!state.ready);
        assert_eq!(state.not_ready_reason, Some("initializing".to_string()));
    }

    #[test]
    fn test_health_state_not_alive() {
        let mut state = HealthState::default();
        state.set_not_alive("crashed".to_string());
        assert!(!state.alive);
        assert_eq!(state.not_alive_reason, Some("crashed".to_string()));
    }

    #[test]
    fn test_health_server_bind_address() {
        let server = HealthServer::new(HealthServerConfig {
            host: "127.0.0.1".to_string(),
            port: 9090,
            ..Default::default()
        });

        assert_eq!(server.bind_address(), "127.0.0.1:9090");
    }

    #[tokio::test]
    async fn test_health_server_start_shutdown() -> Result<()> {
        let mut server = HealthServer::new(HealthServerConfig {
            port: 0, // Use random port for testing
            ..Default::default()
        });

        assert!(!server.is_running());

        server.spawn_background_task().await?;
        assert!(server.is_running());

        server.shutdown().await?;
        assert!(!server.is_running());

        Ok(())
    }

    #[tokio::test]
    async fn test_health_server_with_custom_state() -> Result<()> {
        let server = HealthServer::new(HealthServerConfig {
            port: 0,
            ..Default::default()
        });

        let state = server.state();

        // Mark as not ready
        {
            let mut guard = state.lock().map_err(|e| anyhow::anyhow!("Failed to lock: {}", e))?;
            guard.set_not_ready("initializing".to_string());
        }

        // Verify state persists
        {
            let guard = state.lock().map_err(|e| anyhow::anyhow!("Failed to lock: {}", e))?;
            assert!(!guard.ready);
            assert_eq!(guard.not_ready_reason, Some("initializing".to_string()));
        }

        Ok(())
    }
}
