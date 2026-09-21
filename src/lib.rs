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
/// The idempotent Cursor `hooks.json` installer
/// (`icg install-cursor-hooks`); merge semantics are documented on the
/// module itself, the project-vs-user placement rule on the contract's
/// Cursor section (§6.5).
pub mod cursor_hooks;
pub mod denial_log;
pub mod documented_commands;
pub mod emergency_bypass;
pub mod engine;
pub mod fail_closed;
/// The idempotent Gemini CLI `settings.json` installer
/// (`icg install-gemini-hooks`); merge semantics are documented on the
/// module itself, the wire it configures on the contract's Gemini CLI
/// section (§6.4), and ownership of installed entries is decided by the
/// same predicate the Cursor installer uses ([`hook_command`]).
pub mod gemini_hooks;
pub mod github_workflows;
pub mod health;
pub mod health_server;
/// Shared recognition of ICG hook commands across the harness config
/// installers; one ownership predicate so `cursor_hooks` and
/// `gemini_hooks` cannot drift apart in what they treat as their own.
pub mod hook_command;
pub mod job_cronjob_yaml;
pub mod metrics;
pub mod monitoring;
pub mod new_pack;
/// The idempotent OpenCode plugin installer (`icg
/// install-opencode-plugin`); it deploys the plugin embedded from
/// `opencode-plugin/icg.ts` into the plugin directory pinned for the
/// installed 1.18.29 (contract §6.3.1), recognizes its own artifact by
/// content (`--uninstall` demands the same marker before removing),
/// and never touches OpenCode's permission configuration.
pub mod opencode_plugin;
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
