//! Library for irreversible-command-gate
//!
//! This exposes the core functionality for testing and reuse.

pub mod alerting;
pub mod coverage;
pub mod denial_log;
pub mod documented_commands;
pub mod emergency_bypass;
pub mod engine;
pub mod fail_closed;
pub mod health;
pub mod health_server;
pub mod metrics;
pub mod monitoring;
pub mod new_pack;
pub mod overrides;
pub mod pack_manifest;
pub mod regex_safety;
pub mod regression;
pub mod rollback;
pub mod rule_pack;
pub mod state_store;
pub mod telemetry;
pub mod trust_pointer;
pub mod update;
pub mod value_derivation;
