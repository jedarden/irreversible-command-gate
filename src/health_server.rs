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
use std::net::SocketAddr;
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
    health_store: Option<crate::health::HealthStore>,
    bound_address: Option<SocketAddr>,
    shutdown_tx: Option<oneshot::Sender<()>>,
}

impl HealthServer {
    /// Create a new health server
    pub fn new(config: HealthServerConfig) -> Self {
        Self {
            config,
            state: Arc::new(Mutex::new(HealthState::default())),
            health_store: None,
            bound_address: None,
            shutdown_tx: None,
        }
    }

    /// Construct a server backed by an explicit durable health store.
    pub fn with_health_store(
        config: HealthServerConfig,
        health_store: crate::health::HealthStore,
    ) -> Self {
        let mut server = Self::new(config);
        server.health_store = Some(health_store);
        server
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
        let health_store = self.health_store.clone();
        let bind_address = self.bind_address();

        let listener = TcpListener::bind(&bind_address)
            .await
            .with_context(|| format!("Failed to bind health server to {}", bind_address))?;
        self.bound_address = Some(listener.local_addr()?);

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
                                let health_store = health_store.clone();

                                tokio::spawn(async move {
                                    if let Err(e) = handle_connection(stream, state, config, health_store).await {
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

    /// Return the actual bound address, including the assigned port when the
    /// configuration requested port `0`.
    pub fn local_addr(&self) -> Option<SocketAddr> {
        self.bound_address
    }
}

/// Handle a single HTTP connection
async fn handle_connection(
    mut stream: tokio::net::TcpStream,
    state: Arc<Mutex<HealthState>>,
    config: HealthServerConfig,
    health_store: Option<crate::health::HealthStore>,
) -> Result<()> {
    // Read the HTTP request
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

    let durable_metrics = health_store
        .as_ref()
        .and_then(|store| store.health_metrics().ok());

    let detailed_status = |metrics: Option<crate::health::HealthMetrics>, uptime: f64| {
        let mut status = HealthStatus::healthy(uptime);
        if let Some(ref metrics) = metrics {
            status.status = serde_json::to_value(metrics.status)
                .ok()
                .and_then(|value| value.as_str().map(str::to_string))
                .unwrap_or_else(|| format!("{:?}", metrics.status).to_lowercase());
            status.uptime_seconds = metrics
                .current_uptime
                .map(|duration| duration.as_secs_f64())
                .unwrap_or(0.0);
            status.details = Some(serde_json::json!({
                "health_metrics": metrics,
            }));
        }
        status
    };

    // Route the request
    let (status_code, status_text, content_type, body) = match path {
        "/" | "/health" | "/health/status" => {
            let local_uptime = state
                .lock()
                .map_err(|e| anyhow::anyhow!("Failed to lock state: {}", e))?
                .uptime_seconds();
            let health = detailed_status(durable_metrics.clone(), local_uptime);
            let status_code = durable_metrics
                .as_ref()
                .map(|metrics| if metrics.status.is_running() { 200 } else { 503 })
                .unwrap_or(200);
            (
                status_code,
                if status_code == 200 { "OK" } else { "Service Unavailable" },
                "application/json",
                serde_json::to_string_pretty(&health)?,
            )
        }
        "/health/ready" => {
            if !config.readiness_enabled {
                (404, "Not Found", "text/plain", "Readiness probe disabled".to_string())
            } else {
                let state_guard = state.lock().map_err(|e| anyhow::anyhow!("Failed to lock state: {}", e))?;
                let uptime = state_guard.uptime_seconds();

                let ready = state_guard.ready
                    && durable_metrics
                        .as_ref()
                        .map(|metrics| metrics.status.is_running())
                        .unwrap_or(true);

                if ready {
                    let health = detailed_status(durable_metrics.clone(), uptime);
                    (200, "OK", "application/json", serde_json::to_string_pretty(&health)?)
                } else {
                    let reason = state_guard
                        .not_ready_reason
                        .clone()
                        .or_else(|| durable_metrics.as_ref().map(|metrics| format!("guard status: {:?}", metrics.status)))
                        .unwrap_or_else(|| "Unknown reason".to_string());
                    let mut health = HealthStatus::not_ready(uptime, reason);
                    health.details = Some(serde_json::json!({
                        "reason": health.details
                            .as_ref()
                            .and_then(|details| details.get("reason"))
                            .cloned()
                            .unwrap_or_else(|| serde_json::Value::String("not ready".to_string())),
                        "health_metrics": durable_metrics.clone(),
                    }));
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

                let alive = state_guard.alive
                    && durable_metrics
                        .as_ref()
                        .map(|metrics| metrics.status.is_running())
                        .unwrap_or(true);

                if alive {
                    let health = detailed_status(durable_metrics.clone(), uptime);
                    (200, "OK", "application/json", serde_json::to_string_pretty(&health)?)
                } else {
                    let reason = state_guard
                        .not_alive_reason
                        .clone()
                        .or_else(|| durable_metrics.as_ref().map(|metrics| format!("guard status: {:?}", metrics.status)))
                        .unwrap_or_else(|| "Unknown reason".to_string());
                    let mut health = HealthStatus::unhealthy(uptime, reason);
                    health.details = Some(serde_json::json!({
                        "reason": health.details
                            .as_ref()
                            .and_then(|details| details.get("reason"))
                            .cloned()
                            .unwrap_or_else(|| serde_json::Value::String("not alive".to_string())),
                        "health_metrics": durable_metrics.clone(),
                    }));
                    (503, "Service Unavailable", "application/json", serde_json::to_string_pretty(&health)?)
                }
            }
        }
        "/metrics" => {
            if !config.metrics_enabled {
                (404, "Not Found", "text/plain", "Metrics endpoint disabled".to_string())
            } else {
                let state_guard = state.lock().map_err(|e| anyhow::anyhow!("Failed to lock state: {}", e))?;
                let uptime = state_guard.uptime_seconds();

                let mut metrics = format!(
                    "# Health server metrics\n\
                    icg_health_server_uptime_seconds {}\n\
                    icg_health_server_ready {}\n\
                    icg_health_server_alive {}\n",
                    uptime,
                    if state_guard.ready { 1 } else { 0 },
                    if state_guard.alive { 1 } else { 0 }
                );

                if let Some(health) = durable_metrics {
                    let guard_metrics = crate::metrics::GuardMetrics::from_health_metrics(&health);
                    metrics.push_str(&format!(
                        "icg_uptime_seconds {}\n\
                        icg_total_crashes {}\n\
                        icg_recent_crashes {}\n\
                        icg_crash_rate {}\n\
                        icg_consecutive_clean_runs {}\n\
                        icg_health_status {}\n\
                        icg_is_stable {}\n",
                        guard_metrics.uptime_seconds,
                        guard_metrics.total_crashes,
                        guard_metrics.recent_crashes,
                        guard_metrics.crash_rate,
                        guard_metrics.consecutive_clean_runs,
                        guard_metrics.health_status,
                        guard_metrics.is_stable,
                    ));
                }

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

    #[tokio::test]
    async fn persistent_health_endpoint_exposes_crash_metrics() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let store = crate::health::HealthStore::new(dir.path().join("health.json"));
        store.record_crash(crate::health::CrashRecord::new(
            crate::health::CrashType::SegmentationFault,
        ))?;

        let mut server = HealthServer::with_health_store(
            HealthServerConfig {
                host: "127.0.0.1".to_string(),
                port: 0,
                ..Default::default()
            },
            store,
        );
        server.spawn_background_task().await?;
        let address = server.local_addr().expect("server should have a local address");

        let mut stream = tokio::net::TcpStream::connect(address).await?;
        stream
            .write_all(b"GET /health HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")
            .await?;
        let mut response = Vec::new();
        stream.read_to_end(&mut response).await?;
        let response = String::from_utf8(response)?;

        assert!(response.starts_with("HTTP/1.1 503 Service Unavailable"));
        assert!(response.contains("\"total_crashes\": 1"));
        assert!(response.contains("\"status\": \"unknown\""));

        server.shutdown().await?;
        Ok(())
    }
}
