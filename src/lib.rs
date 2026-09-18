//! Library for irreversible-command-gate
//!
//! This exposes the core functionality for testing and reuse: every guard
//! and predicate is a `pub mod` so integration tests and downstream tooling
//! can drive it directly.

/// The canonical harness-adapter contract, version 1; its normative
/// description lives in `docs/notes/harness-adapter-contract.md`.
pub mod adapter;
pub mod alerting;
pub mod coverage;
pub mod denial_log;
pub mod documented_commands;
pub mod emergency_bypass;
pub mod engine;
pub mod fail_closed;
pub mod github_workflows;
pub mod health;
pub mod health_server;
pub mod job_cronjob_yaml;
pub mod metrics;
pub mod monitoring;
pub mod new_pack;
pub mod overrides;
pub mod pack_manifest;
pub mod regex_safety;
pub mod regression;
pub mod rollback;
pub mod rule_pack;
/// Internal: shared test-process detection for the operational sinks. Not
/// part of the public API -- the guards that use it are documented on the
/// sinks themselves (`denial_log::operational_log_path`,
/// `health::HealthStore::from_environment_or_default`,
/// `telemetry::operational_store_path`), and every operational write reaches
/// the host cache through one of those three resolutions, including
/// emergency-bypass activation and the `telemetry` subcommands.
mod runtime_context;
pub mod state_store;
pub mod telemetry;
pub mod temp_files;
pub mod trust_pointer;
pub mod update;
pub mod value_derivation;
