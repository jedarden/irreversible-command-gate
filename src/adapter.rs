//! The canonical harness-adapter contract, version 1.
//!
//! Every agent harness ICG guards speaks a slightly different wire format,
//! but they all mean one of a small set of things: "run this command", "write
//! this file", "edit this file", "apply this patch", or "call a structured
//! tool". This module is the seam between those wire formats and the policy
//! engine: an adapter translates its harness's payload into one canonical
//! [`CanonicalRequest`], the engine evaluates the request against the packs,
//! and the adapter translates the one canonical [`CanonicalResult`] back out
//! in the harness's own response envelope.
//!
//! The separation exists so that adding a harness never forks policy. An
//! adapter is deliberately thin -- it owns identity, field-name mapping, and
//! response rendering, and nothing else. Evaluation, packs, predicates, and
//! the fail-open/fail-closed availability boundary all stay in
//! [`crate::engine`]; an adapter that reached around the engine would be a
//! policy fork, not an adapter.
//!
//! The full contract -- identity rules, per-harness field mappings for the
//! two shipped adapters and the three specified-but-unimplemented harnesses,
//! timeout and malformed-input behavior, and the telemetry hygiene rules --
//! is `docs/notes/harness-adapter-contract.md`. This module doc records the
//! invariants the code enforces; the doc records the wire-level detail and
//! its official sources.
//!
//! # Contract version
//!
//! [`ADAPTER_CONTRACT_VERSION`] is the version of the canonical request and
//! result schema defined here, not of any harness's own protocol. Changes
//! that only add fields or harness variants keep the version; changes that
//! reinterpret an existing field or verdict bump it and require migrating
//! every adapter in the same commit.
//!
//! # Backward compatibility
//!
//! The `icg hook` front end has always served both Claude Code and the Codex
//! CLI from one command, because their payload spellings are aliases of each
//! other and both read the `hookSpecificOutput` decision envelope. That is
//! preserved exactly: without `--harness` the hook behaves byte-identically
//! to before this module existed, and both shipped adapters render the same
//! envelope they always have.
//!
//! # Telemetry hygiene
//!
//! A harness identifier is recorded in telemetry as one of the fixed slugs
//! returned by [`HarnessId::as_slug`] -- a closed set of non-secret names
//! (`claude-code`, `codex-cli`, ...). Nothing from the payload -- command
//! lines, file paths, file content, argument values -- ever reaches a
//! telemetry record; evaluation telemetry is verdict-shaped only. Keeping the
//! identifier a closed enum is what makes "never a secret" enforceable rather
//! than promised: there is no code path that could store a free-form name.

use clap::ValueEnum;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::engine::{CheckResult, CommandSource, ContentSource, Engine, InputSource, PreToolUseInput};

/// Version of the canonical request/result schema in this module.
pub const ADAPTER_CONTRACT_VERSION: u32 = 1;

/// Which harness is calling, by identity rather than by payload sniffing.
///
/// The set is closed on purpose: telemetry stores the slug and nothing else,
/// so a free-form harness name -- which could carry anything -- never has a
/// way into a record. Adding a harness is a code change to this enum plus an
/// adapter, which is exactly the review a new wire format should get.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum, Serialize, Deserialize)]
pub enum HarnessId {
    /// Claude Code's native `PreToolUse` hook (`~/.claude/settings.json`).
    ClaudeCode,
    /// The local Codex CLI's synchronous `PreToolUse` hook
    /// (`~/.codex/hooks.json`); not cloud-hosted Codex tasks.
    CodexCli,
    /// OpenCode's in-process plugin API (`tool.execute.before`).
    /// Specified in the contract doc; adapter not yet implemented.
    OpenCode,
    /// Gemini CLI's `BeforeTool` command hook. Specified in the contract
    /// doc; adapter not yet implemented.
    GeminiCli,
    /// Cursor's agent hooks (`hooks.json` schema version 1). Specified in
    /// the contract doc; adapter not yet implemented.
    Cursor,
    /// This binary's own PATH-wrapper front end. The wrapper never parses a
    /// payload -- its input is OS-parsed argv -- so it has no request
    /// adapter; the identity is reserved so wrapper-side telemetry would
    /// name itself if it ever records one.
    Wrapper,
}

impl HarnessId {
    /// The non-secret identifier telemetry records for this harness.
    ///
    /// These slugs are part of the telemetry contract: fixed, lowercase,
    /// human-readable, and carrying nothing that could identify a session, a
    /// repository, or a user.
    pub const fn as_slug(self) -> &'static str {
        match self {
            HarnessId::ClaudeCode => "claude-code",
            HarnessId::CodexCli => "codex-cli",
            HarnessId::OpenCode => "opencode",
            HarnessId::GeminiCli => "gemini-cli",
            HarnessId::Cursor => "cursor",
            HarnessId::Wrapper => "wrapper",
        }
    }
}

/// What a harness's PreToolUse wire can express, and what engine results
/// therefore map onto.
///
/// Capabilities are the extension point where future harness divergence
/// lands without forking evaluation: the shared envelope renderer consults
/// them, and the degradation rules below are contract behavior, not
/// adapter discretion.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Capabilities {
    /// The harness accepts a complete replacement for the tool input on an
    /// allow. When false, a Rewrite result degrades to a Deny carrying the
    /// rewrite's reason: the matched input must not run unreplaced, so a
    /// harness that cannot take a replacement loses the call entirely.
    pub supports_updated_input: bool,

    /// The harness accepts advisory context on an allowing response. When
    /// false, a Warning result renders as a bare allow; the context is
    /// dropped, never turned into a block.
    pub supports_additional_context: bool,

    /// Whether the harness's own documentation says it acts on the advisory
    /// context today. Informational: the Codex CLI parses
    /// `additionalContext` but does not yet honor it, so its warning text is
    /// carried on the wire even though the model may never see it. The
    /// adapter still sends it -- the day Codex starts honoring the field the
    /// text is already there.
    pub honors_additional_context: bool,

    /// The harness shows the top-level `systemMessage` field to its user.
    pub supports_system_message: bool,
}

/// Canonical classification of what a harness is about to do.
///
/// This is a readable view over the engine's own [`InputSource`] -- the
/// engine remains the single representation that reaches evaluation -- so a
/// canonical action and the input the engine evaluates can never disagree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CanonicalAction {
    /// One shell command line to evaluate in command mode.
    Command { command: String },
    /// One full-file write.
    WriteFile {
        file_path: String,
        content: String,
    },
    /// One in-place string replacement.
    EditFile {
        file_path: String,
        old_string: String,
        new_string: String,
    },
    /// One patch-shaped edit touching one or more files. The paths are the
    /// files the patch modifies; the content sources themselves stay in the
    /// engine's input, where evaluation needs them.
    Patch { file_paths: Vec<String> },
    /// A structured tool this contract does not model. Unmodeled tools fail
    /// open: they evaluate to no input, which every front end renders as a
    /// plain allow.
    Unsupported,
}

/// One canonical request: what a harness is about to do, with the identity
/// and the raw input needed to answer it.
#[derive(Debug, Clone, PartialEq)]
pub struct CanonicalRequest {
    /// The canonical schema version this request was built under.
    pub contract_version: u32,
    /// The harness that produced the payload.
    pub harness: HarnessId,
    /// The tool name exactly as the harness named it (e.g. `Bash`,
    /// `apply_patch`, `mcp__github__merge_pull_request`).
    pub tool_name: String,
    /// The engine input this request evaluates, or `None` when the tool is
    /// outside the contract's supported set.
    pub input_source: Option<InputSource>,
    /// The harness's original `tool_input` object, preserved untouched. A
    /// rewrite response must return a complete replacement for the tool
    /// input, so every field the harness sent -- including fields this
    /// contract does not model -- travels here from request to response.
    pub original_tool_input: Option<Value>,
}

impl CanonicalRequest {
    /// Canonical classification of this request.
    pub fn action(&self) -> CanonicalAction {
        match &self.input_source {
            Some(InputSource::Command(CommandSource::Hook(command))) => CanonicalAction::Command {
                command: command.clone(),
            },
            // The wrapper front end feeds OS-parsed argv and has no payload
            // adapter (see `HarnessId::Wrapper`); a request built from argv
            // is joined by whoever builds it, because argv was already
            // parsed by the operating system and must not be lexed again.
            Some(InputSource::Command(CommandSource::Argv(argv))) => CanonicalAction::Command {
                command: argv.join(" "),
            },
            Some(InputSource::Content(ContentSource::Write {
                file_path,
                content,
            })) => CanonicalAction::WriteFile {
                file_path: file_path.clone(),
                content: content.clone(),
            },
            Some(InputSource::Content(ContentSource::Edit {
                file_path,
                old_content,
                new_content,
            })) => CanonicalAction::EditFile {
                file_path: file_path.clone(),
                old_string: old_content.clone(),
                new_string: new_content.clone(),
            },
            Some(InputSource::ContentBatch(contents)) => CanonicalAction::Patch {
                file_paths: contents
                    .iter()
                    .map(ContentSource::file_path)
                    .map(str::to_string)
                    .collect(),
            },
            None => CanonicalAction::Unsupported,
        }
    }

    /// Name of the tool-input field a rewrite replaces for this request.
    ///
    /// The spelling follows the incoming payload, not the harness: a Codex
    /// patch and a Bash command both rewrite `command`, while an Edit
    /// rewrites whichever of `new_string` / `newString` the harness sent.
    pub fn rewrite_key(&self) -> &'static str {
        // A patch's text always travels in the harness's `command` field --
        // including a single-file patch the engine classifies as a plain
        // Write/Edit for evaluation. Replacing `content` would leave the
        // original patch sitting in `command` for the harness to run.
        if self.tool_name == "apply_patch" {
            return "command";
        }
        match self.action() {
            CanonicalAction::WriteFile { .. } => "content",
            CanonicalAction::EditFile { .. } => {
                if self
                    .original_tool_input
                    .as_ref()
                    .and_then(Value::as_object)
                    .is_some_and(|input| input.contains_key("new_string"))
                {
                    "new_string"
                } else {
                    "newString"
                }
            }
            CanonicalAction::Command { .. }
            | CanonicalAction::Patch { .. }
            | CanonicalAction::Unsupported => "command",
        }
    }
}

/// Canonical verdict, harness-independent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CanonicalVerdict {
    /// The call may proceed unchanged.
    Allow,
    /// The call may proceed, with advisory context attached.
    Warn,
    /// The call may proceed only with the replacement input applied.
    Rewrite,
    /// The call must not proceed.
    Deny,
}

/// One canonical result: the engine's decision, ready for an adapter to
/// render in its harness's own envelope.
#[derive(Debug, Clone, PartialEq)]
pub struct CanonicalResult {
    pub verdict: CanonicalVerdict,
    /// The human/model-facing explanation. Present for every verdict that
    /// carries a reason (warn, rewrite, deny), never for a plain allow.
    pub reason: Option<String>,
    /// The pack that produced a non-allow result.
    pub pack_id: Option<String>,
    /// The pattern that produced a non-allow result.
    pub pattern_id: Option<String>,
    /// The file the detection itself named, when the guard carried one.
    pub matched_path: Option<String>,
    /// The replacement value for a Rewrite verdict. The adapter places this
    /// under the request's rewrite key inside the complete replacement
    /// object; it is a value, never a whole input object.
    pub rewrite: Option<String>,
    /// A caller-supplied file reference for the response's `[file=...]`
    /// attribution, used when the denial carries no matched path of its own.
    pub subject: Option<String>,
}

impl CanonicalResult {
    /// Convert an engine result into the canonical shape. `subject` is the
    /// front end's file reference (the written file, or the comma-joined
    /// file list for a multi-file patch) used for the `file=` attribution
    /// when the engine's own denial named no path.
    pub fn from_engine(result: &CheckResult, subject: Option<&str>) -> Self {
        let subject = subject.map(str::to_string);
        match result {
            CheckResult::Allowed => Self {
                verdict: CanonicalVerdict::Allow,
                reason: None,
                pack_id: None,
                pattern_id: None,
                matched_path: None,
                rewrite: None,
                subject,
            },
            CheckResult::Warning {
                reason,
                pack_id,
                pattern_id,
            } => Self {
                verdict: CanonicalVerdict::Warn,
                reason: Some(reason.clone()),
                pack_id: Some(pack_id.clone()),
                pattern_id: Some(pattern_id.clone()),
                matched_path: None,
                rewrite: None,
                subject,
            },
            CheckResult::Rewrite {
                reason,
                rewrite,
                pack_id,
                pattern_id,
            } => Self {
                verdict: CanonicalVerdict::Rewrite,
                reason: Some(reason.clone()),
                pack_id: Some(pack_id.clone()),
                pattern_id: Some(pattern_id.clone()),
                matched_path: None,
                rewrite: Some(rewrite.clone()),
                subject,
            },
            CheckResult::Denied {
                reason,
                pack_id,
                pattern_id,
                matched_path,
            } => Self {
                verdict: CanonicalVerdict::Deny,
                reason: Some(reason.clone()),
                pack_id: Some(pack_id.clone()),
                pattern_id: Some(pattern_id.clone()),
                matched_path: matched_path.clone(),
                rewrite: None,
                subject,
            },
        }
    }

    /// The reason with its rule attribution appended: the one string every
    /// front end names the same rule with. A path carried by the detection
    /// itself is quoted as `path=`; the caller-supplied subject is the
    /// fallback (`file=`).
    pub fn attributed_reason(&self) -> String {
        let reason = self.reason.as_deref().unwrap_or_default();
        let suffix = match (&self.matched_path, &self.subject) {
            (Some(path), _) => format!(", path={path}"),
            (None, Some(file)) => format!(", file={file}"),
            (None, None) => String::new(),
        };
        format!(
            "{reason} [pack={}, pattern={pattern}{suffix}]",
            self.pack_id.as_deref().unwrap_or_default(),
            pattern = self.pattern_id.as_deref().unwrap_or_default(),
        )
    }
}

/// One harness adapter: identity, request translation, response rendering.
///
/// Implementations must stay thin. Anything that looks like a policy
/// decision -- a rule, a pattern, a severity, an availability posture --
/// belongs in the engine or a pack, never here.
pub trait HarnessAdapter: Send + Sync {
    /// The harness this adapter serves.
    fn harness(&self) -> HarnessId;

    /// What this harness's wire can express.
    fn capabilities(&self) -> &'static Capabilities;

    /// Translate a validated PreToolUse input into a canonical request.
    ///
    /// Classification reuses the engine's own conversion -- including its
    /// fail-open boundary for unparseable tool calls and unexpected panics
    /// -- so an adapter can never classify differently than the engine
    /// evaluates. Malformed stdin never reaches this method: the engine's
    /// stdin read fails open before an adapter is consulted, which is the
    /// contract's malformed-input behavior.
    fn build_request(
        &self,
        engine: &Engine,
        input: PreToolUseInput,
        original_tool_input: Option<&Value>,
    ) -> CanonicalRequest {
        let tool_name = input.tool_name.clone();
        let input_source = engine.input_source_from_pre_tool_use_fail_open(input);
        CanonicalRequest {
            contract_version: ADAPTER_CONTRACT_VERSION,
            harness: self.harness(),
            tool_name,
            input_source,
            original_tool_input: original_tool_input.cloned(),
        }
    }

    /// Render one canonical result in this harness's response envelope.
    fn render(
        &self,
        result: &CanonicalResult,
        original_input: Option<&Value>,
        rewrite_key: &str,
    ) -> Value {
        render_decision_envelope(result, original_input, rewrite_key, self.capabilities())
    }
}

/// Adapter for Claude Code's native `PreToolUse` hook.
///
/// Payload: JSON on stdin with `tool_name`/`tool_input` (the hook wire is
/// snake_case; the engine also accepts the camelCase spellings of ICG's
/// early fixtures as aliases). Response: the `hookSpecificOutput` decision
/// envelope. Source: <https://code.claude.com/docs/en/hooks> (`PreToolUse`
/// decision control), re-checked 2026-09-18.
pub struct ClaudeCodeAdapter;

/// Adapter for the local Codex CLI's synchronous `PreToolUse` hook.
///
/// Payload: the same stdin JSON shape, plus the patch-shaped `apply_patch`
/// tool whose patch text arrives in `tool_input.command`. Response: the same
/// `hookSpecificOutput` envelope; Codex additionally requires
/// `hookEventName`, which the shared envelope always sets. Source:
/// <https://developers.openai.com/codex/hooks> and the generated schema at
/// `github.com/openai/codex` `codex-rs/hooks/schema/generated/
/// pre-tool-use.command.input.schema.json`, re-checked 2026-09-18.
pub struct CodexAdapter;

/// Claude Code honors `additionalContext`, `updatedInput`, and
/// `systemMessage` on a `PreToolUse` response.
const CLAUDE_CODE_CAPABILITIES: Capabilities = Capabilities {
    supports_updated_input: true,
    supports_additional_context: true,
    honors_additional_context: true,
    supports_system_message: true,
};

/// Codex accepts the same fields on the wire; its docs do not yet act on
/// `additionalContext`, so the honoring flag is false even though the field
/// is still sent.
const CODEX_CAPABILITIES: Capabilities = Capabilities {
    supports_updated_input: true,
    supports_additional_context: true,
    honors_additional_context: false,
    supports_system_message: true,
};

impl HarnessAdapter for ClaudeCodeAdapter {
    fn harness(&self) -> HarnessId {
        HarnessId::ClaudeCode
    }

    fn capabilities(&self) -> &'static Capabilities {
        &CLAUDE_CODE_CAPABILITIES
    }
}

impl HarnessAdapter for CodexAdapter {
    fn harness(&self) -> HarnessId {
        HarnessId::CodexCli
    }

    fn capabilities(&self) -> &'static Capabilities {
        &CODEX_CAPABILITIES
    }
}

/// The adapter for a declared harness, or `None` for the harnesses whose
/// adapters are specified but not yet implemented (`OpenCode`, `GeminiCli`,
/// `Cursor`) and for the payload-less `Wrapper` front end.
///
/// The front end refuses an unsupported harness rather than silently serving
/// the wrong wire format: a Gemini CLI caller handed the Claude Code
/// envelope would get a response its harness never reads.
pub fn adapter_for(harness: HarnessId) -> Option<&'static dyn HarnessAdapter> {
    match harness {
        HarnessId::ClaudeCode => Some(&ClaudeCodeAdapter),
        HarnessId::CodexCli => Some(&CodexAdapter),
        HarnessId::OpenCode | HarnessId::GeminiCli | HarnessId::Cursor | HarnessId::Wrapper => {
            None
        }
    }
}

/// The default adapter when a hook invocation declares no harness.
///
/// The Claude Code and Codex CLI wires are alias-compatible and both read
/// the `hookSpecificOutput` envelope, so the undecorated `icg hook` that
/// predates this module keeps working unchanged; declaration is still
/// required for the invocation to be *identified* in telemetry.
pub fn default_adapter() -> &'static dyn HarnessAdapter {
    &ClaudeCodeAdapter
}

/// Render one canonical result in the shared `hookSpecificOutput` envelope,
/// degraded according to the harness's capabilities.
///
/// This is the one renderer every payload-speaking adapter shares. It emits
/// exactly one JSON object for stdout and nothing else; diagnostics belong
/// on stderr, and the front end owns practice-mode and emergency-bypass
/// wrapping around this response.
pub fn render_decision_envelope(
    result: &CanonicalResult,
    original_input: Option<&Value>,
    rewrite_key: &str,
    capabilities: &Capabilities,
) -> Value {
    let mut hook_output = serde_json::Map::new();
    // Codex requires `hookEventName` to identify the event; Claude Code
    // ignores it. Always setting it is what makes one envelope serve both.
    hook_output.insert(
        "hookEventName".to_string(),
        Value::String("PreToolUse".to_string()),
    );

    let decision = |hook_output: &mut serde_json::Map<String, Value>, value: &str| {
        hook_output.insert(
            "permissionDecision".to_string(),
            Value::String(value.to_string()),
        );
    };

    let response = match result.verdict {
        CanonicalVerdict::Allow => {
            decision(&mut hook_output, "allow");
            serde_json::json!({ "hookSpecificOutput": hook_output })
        }
        CanonicalVerdict::Warn => {
            decision(&mut hook_output, "allow");
            if capabilities.supports_additional_context {
                hook_output.insert(
                    "additionalContext".to_string(),
                    Value::String(result.attributed_reason()),
                );
            }
            serde_json::json!({ "hookSpecificOutput": hook_output })
        }
        CanonicalVerdict::Rewrite if capabilities.supports_updated_input => {
            decision(&mut hook_output, "allow");
            // `updatedInput` replaces the whole tool-input object, so every
            // field the harness sent -- modeled or not -- must be copied
            // into the replacement, with only the rewrite key substituted.
            let mut updated_input = original_input
                .and_then(Value::as_object)
                .cloned()
                .unwrap_or_default();
            if let Some(rewrite) = &result.rewrite {
                updated_input.insert(rewrite_key.to_string(), Value::String(rewrite.clone()));
            }
            hook_output.insert("updatedInput".to_string(), Value::Object(updated_input));
            if capabilities.supports_additional_context {
                hook_output.insert(
                    "additionalContext".to_string(),
                    Value::String(result.attributed_reason()),
                );
            }
            serde_json::json!({ "hookSpecificOutput": hook_output })
        }
        // Degradation: a harness with no input-modification channel cannot
        // run a call whose arguments the policy said to change. Denying with
        // the rewrite's reason is the fail-safe; allowing the original
        // arguments would execute the thing the rule matched.
        CanonicalVerdict::Rewrite => {
            decision(&mut hook_output, "deny");
            hook_output.insert(
                "permissionDecisionReason".to_string(),
                Value::String(result.attributed_reason()),
            );
            serde_json::json!({ "hookSpecificOutput": hook_output })
        }
        CanonicalVerdict::Deny => {
            decision(&mut hook_output, "deny");
            hook_output.insert(
                "permissionDecisionReason".to_string(),
                Value::String(result.attributed_reason()),
            );
            serde_json::json!({ "hookSpecificOutput": hook_output })
        }
    };

    response
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::ToolInput;
    use serde_json::json;

    fn payload(tool_name: &str, tool_input: Value) -> PreToolUseInput {
        PreToolUseInput {
            tool_name: tool_name.to_string(),
            tool_input: serde_json::from_value(tool_input).expect("tool input parses"),
            id: None,
            timestamp: None,
            session_id: None,
        }
    }

    fn bash_input(command: &str) -> PreToolUseInput {
        payload("Bash", json!({ "command": command }))
    }

    #[test]
    fn contract_version_is_one() {
        assert_eq!(ADAPTER_CONTRACT_VERSION, 1);
    }

    #[test]
    fn harness_slugs_are_the_fixed_non_secret_set() {
        let slugs: Vec<&str> = [
            HarnessId::ClaudeCode,
            HarnessId::CodexCli,
            HarnessId::OpenCode,
            HarnessId::GeminiCli,
            HarnessId::Cursor,
            HarnessId::Wrapper,
        ]
        .iter()
        .map(|h| h.as_slug())
        .collect();

        assert_eq!(
            slugs,
            vec![
                "claude-code",
                "codex-cli",
                "opencode",
                "gemini-cli",
                "cursor",
                "wrapper",
            ]
        );
        for slug in &slugs {
            assert!(
                slug.chars().all(|c| c.is_ascii_lowercase() || c == '-'),
                "telemetry harness slugs stay lowercase kebab-case, got {slug:?}"
            );
        }
    }

    #[test]
    fn only_shipped_adapters_are_selectable() {
        assert!(adapter_for(HarnessId::ClaudeCode).is_some());
        assert!(adapter_for(HarnessId::CodexCli).is_some());
        // Specified but unimplemented: the front end must refuse these
        // rather than serve the wrong wire format.
        assert!(adapter_for(HarnessId::OpenCode).is_none());
        assert!(adapter_for(HarnessId::GeminiCli).is_none());
        assert!(adapter_for(HarnessId::Cursor).is_none());
        assert!(adapter_for(HarnessId::Wrapper).is_none());
        let default = default_adapter();
        assert!(std::ptr::eq(
            adapter_for(HarnessId::ClaudeCode).expect("claude adapter exists"),
            default
        ));
    }

    #[test]
    fn adapters_classify_through_the_engine_not_alongside_it() {
        let engine = Engine::new();
        let codex = adapter_for(HarnessId::CodexCli).expect("codex adapter exists");

        let input = bash_input("git status");
        let direct = Engine::input_source_from_pre_tool_use(clone_input(&input))
            .expect("engine parses Bash")
            .expect("Bash is supported");
        let request = codex.build_request(&engine, input, None);
        assert_eq!(request.contract_version, ADAPTER_CONTRACT_VERSION);
        assert_eq!(request.harness, HarnessId::CodexCli);
        assert_eq!(request.tool_name, "Bash");
        assert_eq!(request.input_source, Some(direct));
    }

    #[test]
    fn apply_patch_classifies_as_a_patch_with_every_file() {
        let engine = Engine::new();
        let adapter = adapter_for(HarnessId::CodexCli).unwrap();
        let patch = "*** Begin Patch\n*** Add File: a.yaml\n+x\n*** Update File: b.yaml\n@@\n-y\n+z\n*** End Patch";
        let request = adapter.build_request(
            &engine,
            payload("apply_patch", json!({ "command": patch })),
            Some(&json!({ "command": patch, "workdir": "/tmp" })),
        );

        assert_eq!(
            request.action(),
            CanonicalAction::Patch {
                file_paths: vec!["a.yaml".to_string(), "b.yaml".to_string()],
            }
        );
        assert_eq!(request.rewrite_key(), "command");
        // The harness's own extra fields survive untouched on the request.
        assert_eq!(
            request.original_tool_input.expect("original input")["workdir"],
            "/tmp"
        );
    }

    #[test]
    fn a_single_file_patch_still_rewrites_command_not_content() {
        let engine = Engine::new();
        let adapter = adapter_for(HarnessId::CodexCli).unwrap();
        // One `Update File` hunk: the engine classifies this as a plain
        // content source for evaluation, but the harness read the patch from
        // `command` and must read the replacement from `command` too -- a
        // rewrite under `content` would leave the original patch in place.
        let patch =
            "*** Begin Patch\n*** Update File: deploy/app.yaml\n@@\n-image: app:latest\n+image: app:1.2.3\n*** End Patch";
        let request = adapter.build_request(
            &engine,
            payload("apply_patch", json!({ "command": patch })),
            Some(&json!({ "command": patch, "description": "Update the deployment" })),
        );
        assert_eq!(request.rewrite_key(), "command");
    }

    #[test]
    fn unmodeled_structured_tools_classify_unsupported() {
        let engine = Engine::new();
        let adapter = adapter_for(HarnessId::ClaudeCode).unwrap();
        let request = adapter.build_request(
            &engine,
            payload("mcp__github__merge_pull_request", json!({ "pr": 7 })),
            Some(&json!({ "pr": 7 })),
        );

        assert_eq!(request.action(), CanonicalAction::Unsupported);
        assert!(request.input_source.is_none());
    }

    #[test]
    fn rewrite_keys_follow_the_incoming_payload_spelling() {
        let engine = Engine::new();
        let claude = adapter_for(HarnessId::ClaudeCode).unwrap();

        let write_request = claude.build_request(
            &engine,
            payload("Write", json!({ "filePath": "a.yaml", "content": "x" })),
            None,
        );
        assert_eq!(write_request.rewrite_key(), "content");

        let camel_edit = claude.build_request(
            &engine,
            payload(
                "Edit",
                json!({ "filePath": "a.yaml", "oldString": "x", "newString": "y" }),
            ),
            Some(&json!({ "filePath": "a.yaml", "oldString": "x", "newString": "y" })),
        );
        assert_eq!(camel_edit.rewrite_key(), "newString");

        let snake_edit = claude.build_request(
            &engine,
            payload(
                "Edit",
                json!({ "file_path": "a.yaml", "old_string": "x", "new_string": "y" }),
            ),
            Some(&json!({ "file_path": "a.yaml", "old_string": "x", "new_string": "y" })),
        );
        assert_eq!(snake_edit.rewrite_key(), "new_string");

        let bash_request = claude.build_request(&engine, bash_input("git status"), None);
        assert_eq!(bash_request.rewrite_key(), "command");
    }

    fn denied(pack: &str, pattern: &str) -> CheckResult {
        CheckResult::Denied {
            reason: "denied by policy".to_string(),
            pack_id: pack.to_string(),
            pattern_id: pattern.to_string(),
            matched_path: None,
        }
    }

    fn rewrite(pack: &str, pattern: &str) -> CheckResult {
        CheckResult::Rewrite {
            reason: "use the safe form".to_string(),
            rewrite: "git push origin main".to_string(),
            pack_id: pack.to_string(),
            pattern_id: pattern.to_string(),
        }
    }

    #[test]
    fn every_verdict_renders_the_shared_envelope() {
        let adapter = adapter_for(HarnessId::ClaudeCode).unwrap();
        let input = json!({ "command": "git push --force origin main" });

        let allow = adapter.render(
            &CanonicalResult::from_engine(&CheckResult::Allowed, None),
            None,
            "command",
        );
        assert_eq!(
            allow,
            json!({
                "hookSpecificOutput": {
                    "hookEventName": "PreToolUse",
                    "permissionDecision": "allow"
                }
            })
        );

        let deny = adapter.render(
            &CanonicalResult::from_engine(&denied("git", "git-force-push"), None),
            Some(&input),
            "command",
        );
        assert_eq!(
            deny,
            json!({
                "hookSpecificOutput": {
                    "hookEventName": "PreToolUse",
                    "permissionDecision": "deny",
                    "permissionDecisionReason":
                        "denied by policy [pack=git, pattern=git-force-push]"
                }
            })
        );

        let rewrite = adapter.render(
            &CanonicalResult::from_engine(&rewrite("git", "git-force-push"), None),
            Some(&input),
            "command",
        );
        assert_eq!(
            rewrite,
            json!({
                "hookSpecificOutput": {
                    "hookEventName": "PreToolUse",
                    "permissionDecision": "allow",
                    "updatedInput": { "command": "git push origin main" },
                    "additionalContext":
                        "use the safe form [pack=git, pattern=git-force-push]"
                }
            })
        );
    }

    #[test]
    fn attribution_prefers_the_matched_path_over_the_subject() {
        let with_path = CanonicalResult {
            verdict: CanonicalVerdict::Deny,
            reason: Some("no".to_string()),
            pack_id: Some("beads".to_string()),
            pattern_id: Some("beads-shared-checkout-write".to_string()),
            matched_path: Some(".beads/beads.db".to_string()),
            rewrite: None,
            subject: Some("deploy/app.yaml".to_string()),
        };
        assert!(with_path.attributed_reason().contains("path=.beads/beads.db"));
        assert!(!with_path.attributed_reason().contains("file="));

        let subject_only = CanonicalResult {
            matched_path: None,
            ..with_path.clone()
        };
        assert!(
            subject_only
                .attributed_reason()
                .contains("file=deploy/app.yaml")
        );
    }

    #[test]
    fn rewrite_replaces_one_field_and_preserves_the_rest_verbatim() {
        let adapter = adapter_for(HarnessId::ClaudeCode).unwrap();
        // Harness-specific fields this contract does not model -- a
        // description, a timeout, a background flag -- must survive into the
        // replacement untouched.
        let original = json!({
            "command": "git push --force origin main",
            "description": "Push reviewed changes",
            "timeout": 120000,
            "run_in_background": false,
        });
        let response = adapter.render(
            &CanonicalResult::from_engine(&rewrite("git", "git-force-push"), None),
            Some(&original),
            "command",
        );

        let updated = &response["hookSpecificOutput"]["updatedInput"];
        assert_eq!(updated["command"], "git push origin main");
        assert_eq!(updated["description"], "Push reviewed changes");
        assert_eq!(updated["timeout"], 120000);
        assert_eq!(updated["run_in_background"], false);
        assert_eq!(
            response["hookSpecificOutput"]["permissionDecision"],
            "allow",
            "a rewrite allows the call with replacement arguments"
        );
        assert!(
            response["hookSpecificOutput"]
                .get("permissionDecisionReason")
                .is_none(),
            "a rewrite is not a denial: no denial reason may accompany it"
        );
    }

    #[test]
    fn a_deny_never_carries_a_replacement_input() {
        let adapter = adapter_for(HarnessId::CodexCli).unwrap();
        let response = adapter.render(
            &CanonicalResult::from_engine(
                &denied("storage-class", "storage-class-ssd"),
                Some("a.yaml"),
            ),
            Some(&json!({ "command": "patch" })),
            "command",
        );
        assert_eq!(response["hookSpecificOutput"]["permissionDecision"], "deny");
        assert!(response["hookSpecificOutput"].get("updatedInput").is_none());
        assert!(
            response["hookSpecificOutput"]
                .get("additionalContext")
                .is_none()
        );
    }

    #[test]
    fn a_warning_allows_and_carries_context_but_never_blocks() {
        let adapter = adapter_for(HarnessId::ClaudeCode).unwrap();
        let warning = CanonicalResult {
            verdict: CanonicalVerdict::Warn,
            reason: Some("check the target".to_string()),
            pack_id: Some("warning-verdict-e2e".to_string()),
            pattern_id: Some("warn-worktree-add".to_string()),
            matched_path: None,
            rewrite: None,
            subject: None,
        };
        let response = adapter.render(&warning, None, "command");

        assert_eq!(response["hookSpecificOutput"]["permissionDecision"], "allow");
        assert_eq!(
            response["hookSpecificOutput"]["additionalContext"],
            "check the target [pack=warning-verdict-e2e, pattern=warn-worktree-add]"
        );
        assert!(response["hookSpecificOutput"].get("updatedInput").is_none());
        assert!(
            response["hookSpecificOutput"]
                .get("permissionDecisionReason")
                .is_none()
        );
    }

    #[test]
    fn a_harness_without_an_update_channel_degrades_rewrite_to_deny() {
        let no_rewrite = Capabilities {
            supports_updated_input: false,
            ..CLAUDE_CODE_CAPABILITIES
        };
        let response = render_decision_envelope(
            &CanonicalResult::from_engine(&rewrite("git", "git-force-push"), None),
            Some(&json!({ "command": "git push --force origin main" })),
            "command",
            &no_rewrite,
        );

        assert_eq!(response["hookSpecificOutput"]["permissionDecision"], "deny");
        assert_eq!(
            response["hookSpecificOutput"]["permissionDecisionReason"],
            "use the safe form [pack=git, pattern=git-force-push]"
        );
        assert!(response["hookSpecificOutput"].get("updatedInput").is_none());
    }

    #[test]
    fn a_harness_without_context_drops_the_warning_text_but_still_allows() {
        let no_context = Capabilities {
            supports_additional_context: false,
            ..CODEX_CAPABILITIES
        };
        let warning = CanonicalResult {
            verdict: CanonicalVerdict::Warn,
            reason: Some("check the target".to_string()),
            pack_id: Some("warning-verdict-e2e".to_string()),
            pattern_id: Some("warn-worktree-add".to_string()),
            matched_path: None,
            rewrite: None,
            subject: None,
        };
        let response = render_decision_envelope(&warning, None, "command", &no_context);

        assert_eq!(response["hookSpecificOutput"]["permissionDecision"], "allow");
        assert!(
            response["hookSpecificOutput"]
                .get("additionalContext")
                .is_none()
        );
    }

    #[test]
    fn codex_declares_its_context_limitation_without_losing_the_field() {
        let capabilities = adapter_for(HarnessId::CodexCli)
            .unwrap()
            .capabilities();
        assert!(
            capabilities.supports_additional_context,
            "the field is still sent"
        );
        assert!(
            !capabilities.honors_additional_context,
            "Codex parses additionalContext but does not honor it yet"
        );
        assert!(capabilities.supports_system_message);
        assert!(capabilities.supports_updated_input);
    }

    /// `PreToolUseInput` is not `Clone`; rebuild an equal input for the
    /// direct-engine comparison.
    fn clone_input(input: &PreToolUseInput) -> PreToolUseInput {
        PreToolUseInput {
            tool_name: input.tool_name.clone(),
            tool_input: ToolInput {
                command: input.tool_input.command.clone(),
                file_path: input.tool_input.file_path.clone(),
                content: input.tool_input.content.clone(),
                old_string: input.tool_input.old_string.clone(),
                new_string: input.tool_input.new_string.clone(),
                encoding: input.tool_input.encoding.clone(),
                mime_type: input.tool_input.mime_type.clone(),
            },
            id: input.id.clone(),
            timestamp: input.timestamp.clone(),
            session_id: input.session_id.clone(),
        }
    }

    #[test]
    fn content_sources_round_trip_through_the_canonical_action() {
        let write = InputSource::Content(ContentSource::Write {
            file_path: "deploy/app.yaml".to_string(),
            content: "storageClassName: ssd\n".to_string(),
        });
        let request = CanonicalRequest {
            contract_version: ADAPTER_CONTRACT_VERSION,
            harness: HarnessId::ClaudeCode,
            tool_name: "Write".to_string(),
            input_source: Some(write.clone()),
            original_tool_input: None,
        };
        assert_eq!(
            request.action(),
            CanonicalAction::WriteFile {
                file_path: "deploy/app.yaml".to_string(),
                content: "storageClassName: ssd\n".to_string(),
            }
        );
        assert_eq!(request.rewrite_key(), "content");
        assert_eq!(request.input_source, Some(write));

        let argv = CanonicalRequest {
            harness: HarnessId::Wrapper,
            tool_name: "git".to_string(),
            input_source: Some(InputSource::Command(CommandSource::Argv(vec![
                "git".to_string(),
                "status".to_string(),
            ]))),
            original_tool_input: None,
            ..request
        };
        assert!(matches!(
            argv.action(),
            CanonicalAction::Command { command } if command == "git status"
        ));
    }
}
