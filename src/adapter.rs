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

use crate::engine::{
    CheckResult, CommandSource, ContentSource, Engine, InputSource, PreToolUseInput,
};

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
    /// OpenCode's in-process plugin API (`tool.execute.before`). The plugin
    /// shells out to `icg hook --harness open-code` — the alias spelling,
    /// chosen because it is the only one the installed 0.1.62 accepts (the
    /// slug `opencode` became primary only after that release) — so the
    /// payload and the response both travel over a subprocess wire even
    /// though the hook itself is in-process. The flag spelling is the
    /// telemetry slug; clap's derived kebab-case of the variant
    /// (`open-code`) is accepted as an alias.
    #[value(name = "opencode", alias = "open-code")]
    OpenCode,
    /// Gemini CLI's `BeforeTool` command hook
    /// (`~/.gemini/settings.json` or project `.gemini/settings.json`).
    GeminiCli,
    /// Cursor's agent hooks (`hooks.json` schema version 1): the generic
    /// `preToolUse` event, plus the dedicated `beforeShellExecution` event
    /// served by [`CursorShellExecutionAdapter`].
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

    /// The harness accepts `permissionDecision: "allow"` as a grant. When
    /// false, an allowing verdict omits the field entirely rather than
    /// asserting a decision the harness will reject: Codex CLI parses the
    /// value on the wire but its runtime honors `deny` alone, logging
    /// `PreToolUse hook returned unsupported permissionDecision:allow` for
    /// every other value. An absent decision is "no opinion" in every
    /// harness ICG speaks to, so omission is the portable spelling of an
    /// allow; asserting one is an optimization that only Claude Code reads.
    ///
    /// This never gates a deny. A harness that cannot be granted to can
    /// still be vetoed, which is the direction that carries the safety.
    pub supports_allow_decision: bool,
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
    WriteFile { file_path: String, content: String },
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
            Some(InputSource::Content(ContentSource::Write { file_path, content })) => {
                CanonicalAction::WriteFile {
                    file_path: file_path.clone(),
                    content: content.clone(),
                }
            }
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

/// Adapter for Gemini CLI's `BeforeTool` hook.
///
/// Payload: JSON on stdin with `tool_name`/`tool_input` (snake_case, the
/// same aliases the engine already reads) plus Gemini-only context fields
/// (`session_id`, `cwd`, `hook_event_name`, `timestamp`, `mcp_context`,
/// `original_request_name`) that the engine's lenient parse ignores. The
/// covered tool names are Gemini's spellings of the canonical actions:
/// `run_shell_command` (shell), `write_file` (file creation/overwrite), and
/// `replace` (text substitution). Response: a denial is Gemini's top-level
/// `{"decision": "deny", "reason": ...}`; a rewrite is
/// `hookSpecificOutput.tool_input`, which Gemini merges with and overrides
/// the model's arguments with before execution. Exit code 0 with stdout JSON
/// is the only channel ICG uses; exit 2 blocks via stderr, and every other
/// exit is a non-fatal warning (fail-open) -- matching ICG's own posture.
/// Source: `docs/hooks/reference.md` in `github.com/google-gemini/gemini-cli`
/// at v0.60.0 (retrieved 2026-09-18) and `docs/tools/` for the tool
/// parameter fields. The gemini-cli wire carries no version field, so the
/// pin lives in the docs and in `docs/notes/harness-adapter-contract.md`.
///
/// MCP tools (named with the `mcp__` prefix) and Gemini's read-only tools
/// are deliberately unmodeled: they classify to `Unsupported` and allow,
/// which is why the ICG installer scopes its matcher to the three covered
/// names. See `docs/notes/harness-adapter-contract.md` §6.4 for the full
/// coverage statement.
pub struct GeminiCliAdapter;

/// Gemini CLI honors `hookSpecificOutput.tool_input` (rewrites) and the
/// common `systemMessage` field (practice-mode and bypass reports ride it).
/// A `BeforeTool` response has no model-directed context channel --
/// `additionalContext` is documented for other events -- so warnings
/// degrade to a bare allow with the text dropped. Deliberately absent from
/// every ICG response: `decision: "allow"`, whose `BeforeTool` impact is
/// unspecified and could stand in for Gemini's own confirmation flow.
const GEMINI_CLI_CAPABILITIES: Capabilities = Capabilities {
    supports_updated_input: true,
    supports_additional_context: false,
    honors_additional_context: false,
    supports_system_message: true,
    // `decision: "allow"` is deliberately never emitted to Gemini (see the
    // doc comment above); the flag records that as contract, not accident.
    supports_allow_decision: false,
};

/// Claude Code honors `additionalContext`, `updatedInput`, and
/// `systemMessage` on a `PreToolUse` response.
const CLAUDE_CODE_CAPABILITIES: Capabilities = Capabilities {
    supports_updated_input: true,
    supports_additional_context: true,
    honors_additional_context: true,
    supports_system_message: true,
    supports_allow_decision: true,
};

/// The Codex CLI release the narrowing documented below is pinned to: the
/// shipped binary whose rejections are quoted here and in
/// `docs/notes/harness-adapter-contract.md` §6.2. `icg-ci`'s
/// `codex-hook-compatibility` matrix must always include it --
/// `tests/codex_compat_matrix_tests.rs` fails the build when the matrix
/// drops behind the pin, which is how the 0.154 narrowing first shipped
/// unnoticed against a matrix still frozen at 0.144-0.146
/// (irrevers-048ce4f8).
pub const CODEX_RUNTIME_PIN: &str = "0.154.0";

/// Codex parses the same envelope on the wire -- its
/// `pre-tool-use.command.output` schema accepts `allow|deny|ask` -- but its
/// runtime honors `deny` alone, and says so: verified against Codex CLI
/// 0.154.0, which carries the rejections
/// `PreToolUse hook returned unsupported permissionDecision:allow`,
/// `... unsupported permissionDecision:ask`,
/// `... updatedInput without permissionDecision:allow`, and
/// `... permissionDecision:deny without a non-empty permissionDecisionReason`.
///
/// So Codex can be vetoed but not granted to, and `updatedInput` is
/// unreachable there because its only documented precondition -- an
/// accepted `permissionDecision: "allow"` -- is itself refused. A Rewrite
/// therefore degrades to a Deny carrying the rewrite's reason, which is the
/// fail-safe the shared renderer already implements: the alternative is the
/// matched command running unrewritten, which is what shipped before.
///
/// `additionalContext` stays on: Codex's schema accepts the field and
/// ignores it today, so the text costs nothing and is already in place the
/// day Codex starts honoring it (irrevers-a0ced256).
const CODEX_CAPABILITIES: Capabilities = Capabilities {
    supports_updated_input: false,
    supports_additional_context: true,
    honors_additional_context: false,
    supports_system_message: true,
    supports_allow_decision: false,
};

/// Adapter for Cursor's native `preToolUse` agent hook.
///
/// Payload: JSON on stdin with `tool_name`/`tool_input`. Cursor names its
/// shell tool `Shell` (the payload is the Bash shape), and delivers its
/// edits under the `Write` tool name shaped as an old/new string pair --
/// both spellings classify in the engine's alias handling. Response:
/// Cursor's flat envelope -- `permission`, `user_message`, `agent_message`,
/// and `updated_input` at the top level, with no `hookSpecificOutput`
/// wrapper. Cursor also accepts Claude Code's nested envelope on this event
/// (third-party imports), but the declared adapter speaks the native shape
/// so the response never depends on a compatibility layer. Sources:
/// <https://cursor.com/docs/agent/hooks> and
/// <https://cursor.com/docs/reference/third-party-hooks>, re-checked
/// 2026-09-19.
pub struct CursorAdapter;

/// Adapter for Cursor's dedicated `beforeShellExecution` agent hook,
/// selected with `icg hook --harness cursor --event before-shell-execution`.
///
/// The event's payload is `{command, cwd, sandbox}` -- **no `tool_name`** --
/// so it would fail the PreToolUse stdin parse and fail open unchecked;
/// `--event` gives it its own admission path, which shapes the payload into
/// the `Shell` tool call the shared front end already evaluates. The
/// event's response schema has **no `updated_input` field**, so its
/// capabilities declare `supports_updated_input: false` and a Rewrite
/// degrades to a Deny carrying the rewrite's reason (§5 degradation): the
/// event can refuse a command but cannot replace it.
pub struct CursorShellExecutionAdapter;

/// Cursor's `preToolUse` decision carries `permission`, `user_message`,
/// `agent_message`, and `updated_input` -- but no advisory-context channel
/// (`additional_context` exists only on the sessionStart/postToolUse
/// events, never on a permission decision) and no `systemMessage`. Cursor's
/// permission hooks **block on a response that does not match the hook's
/// schema**, so nothing outside the documented schema may ever be emitted
/// to it.
const CURSOR_CAPABILITIES: Capabilities = Capabilities {
    supports_updated_input: true,
    supports_additional_context: false,
    honors_additional_context: false,
    supports_system_message: false,
    // Cursor's flat envelope carries `permission` on every response,
    // allow included; it is rendered by render_cursor_envelope, not by the
    // shared `hookSpecificOutput` renderer this flag gates.
    supports_allow_decision: true,
};

/// `beforeShellExecution` output carries only the permission decision and
/// the two messages: there is no `updated_input` field on this event, so a
/// rewrite degrades to a deny.
const CURSOR_SHELL_EVENT_CAPABILITIES: Capabilities = Capabilities {
    supports_updated_input: false,
    ..CURSOR_CAPABILITIES
};

/// Adapter for OpenCode's `tool.execute.before` plugin hook.
///
/// OpenCode's plugin API is in-process JavaScript/TS, so the wire ICG speaks
/// is the one between the ICG plugin and this process: the plugin serializes
/// the hook's payload -- `tool`, `sessionID`, `callID`, and the mutable
/// `args` object -- to stdin, and reads back one JSON object naming the one
/// thing it must do next (`render_opencode_envelope`). `args` spellings are
/// OpenCode's own camelCase: `bash` carries `command`, `write` carries
/// `filePath`/`content`, and `edit` carries `filePath`/`oldString`/
/// `newString`/`replaceAll` -- all engine aliases of the modeled fields
/// except the tool names, which are wired in as spellings of the same three
/// actions. Verified against the installed OpenCode 1.18.29 (binary sha256
/// `ca6c0e1f...`); the pinned evidence and its citations live in
/// `docs/research/opencode-1.18.29-plugin-surface.md` and
/// `docs/research/opencode-1.18.29-deny-rewrite-advisory.md`, and
/// `docs/notes/harness-adapter-contract.md` §6.3 is the normative summary.
///
/// A denial is delivered by the plugin throwing; a rewrite by the plugin
/// copying the replacement's properties onto its `output.args` **in place**
/// (reassigning `output.args` is a no-op in OpenCode -- hook and executor
/// share one args object); an allow by returning untouched. There is no
/// advisory channel at tool-call time, so a warning degrades to a bare
/// allow.
pub struct OpenCodeAdapter;

/// OpenCode's plugin channel can carry a replacement (in-place `args`
/// mutation is execution-real) but no advisory context and no
/// `systemMessage`: the hook's only output is `{args}`, and throw is the
/// only other effect a plugin can produce. A Warn therefore degrades to a
/// bare allow. There is no `permissionDecision` field anywhere on this wire
/// -- an allow is spelled by the plugin returning normally -- so
/// `supports_allow_decision` records that absence; the flag is inert here
/// because `render_opencode_envelope` never consults it.
const OPENCODE_CAPABILITIES: Capabilities = Capabilities {
    supports_updated_input: true,
    supports_additional_context: false,
    honors_additional_context: false,
    supports_system_message: false,
    supports_allow_decision: false,
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

impl HarnessAdapter for GeminiCliAdapter {
    fn harness(&self) -> HarnessId {
        HarnessId::GeminiCli
    }

    fn capabilities(&self) -> &'static Capabilities {
        &GEMINI_CLI_CAPABILITIES
    }

    fn render(
        &self,
        result: &CanonicalResult,
        original_input: Option<&Value>,
        rewrite_key: &str,
    ) -> Value {
        render_gemini_envelope(result, original_input, rewrite_key, self.capabilities())
    }
}

impl HarnessAdapter for CursorAdapter {
    fn harness(&self) -> HarnessId {
        HarnessId::Cursor
    }

    fn capabilities(&self) -> &'static Capabilities {
        &CURSOR_CAPABILITIES
    }

    fn render(
        &self,
        result: &CanonicalResult,
        original_input: Option<&Value>,
        rewrite_key: &str,
    ) -> Value {
        render_cursor_envelope(result, original_input, rewrite_key, self.capabilities())
    }
}

impl HarnessAdapter for CursorShellExecutionAdapter {
    fn harness(&self) -> HarnessId {
        HarnessId::Cursor
    }

    fn capabilities(&self) -> &'static Capabilities {
        &CURSOR_SHELL_EVENT_CAPABILITIES
    }

    fn render(
        &self,
        result: &CanonicalResult,
        original_input: Option<&Value>,
        rewrite_key: &str,
    ) -> Value {
        render_cursor_envelope(result, original_input, rewrite_key, self.capabilities())
    }
}

impl HarnessAdapter for OpenCodeAdapter {
    fn harness(&self) -> HarnessId {
        HarnessId::OpenCode
    }

    fn capabilities(&self) -> &'static Capabilities {
        &OPENCODE_CAPABILITIES
    }

    fn render(
        &self,
        result: &CanonicalResult,
        original_input: Option<&Value>,
        rewrite_key: &str,
    ) -> Value {
        render_opencode_envelope(result, original_input, rewrite_key, self.capabilities())
    }
}

/// The adapter for a declared harness, or `None` for the payload-less
/// `Wrapper` front end, whose input is OS-parsed argv and never a payload.
///
/// The front end refuses an unsupported harness rather than silently serving
/// the wrong wire format: a caller handed an envelope its harness never
/// reads would proceed while looking guarded. Cursor's dedicated
/// `beforeShellExecution` event is selected through
/// [`cursor_shell_execution_adapter`], not through this function: its
/// payload and response differ from `preToolUse`'s.
pub fn adapter_for(harness: HarnessId) -> Option<&'static dyn HarnessAdapter> {
    match harness {
        HarnessId::ClaudeCode => Some(&ClaudeCodeAdapter),
        HarnessId::CodexCli => Some(&CodexAdapter),
        HarnessId::OpenCode => Some(&OpenCodeAdapter),
        HarnessId::GeminiCli => Some(&GeminiCliAdapter),
        HarnessId::Cursor => Some(&CursorAdapter),
        HarnessId::Wrapper => None,
    }
}

/// The adapter for Cursor's `beforeShellExecution` event, served by
/// `icg hook --harness cursor --event before-shell-execution`. Its
/// capabilities degrade a Rewrite to a Deny: the event's response schema has
/// no input-replacement channel.
pub fn cursor_shell_execution_adapter() -> &'static dyn HarnessAdapter {
    &CursorShellExecutionAdapter
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

    // An allowing verdict asserts `permissionDecision: "allow"` only where
    // the harness reads it as a grant. Where it does not, the field is
    // omitted rather than sent and rejected -- an absent decision is "no
    // opinion", which is the same outcome without the per-call hook error.
    // Deny is never routed through here: a veto is always spoken outright.
    let allow_decision = |hook_output: &mut serde_json::Map<String, Value>| {
        if capabilities.supports_allow_decision {
            decision(hook_output, "allow");
        }
    };

    match result.verdict {
        CanonicalVerdict::Allow => {
            allow_decision(&mut hook_output);
            serde_json::json!({ "hookSpecificOutput": hook_output })
        }
        CanonicalVerdict::Warn => {
            allow_decision(&mut hook_output);
            if capabilities.supports_additional_context {
                hook_output.insert(
                    "additionalContext".to_string(),
                    Value::String(result.attributed_reason()),
                );
            }
            serde_json::json!({ "hookSpecificOutput": hook_output })
        }
        CanonicalVerdict::Rewrite if capabilities.supports_updated_input => {
            allow_decision(&mut hook_output);
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
    }
}

/// Render one canonical result in Cursor's native flat envelope, degraded
/// according to the given capabilities.
///
/// Cursor's permission hooks **block a response that does not match the
/// hook's schema**, so this renderer emits exactly the documented fields and
/// nothing else: `permission` always; `user_message` and `agent_message` on
/// a deny; `updated_input` on a rewrite (when the event supports one). The
/// degradation rules are the shared contract's: a Warning is a bare allow --
/// the decision schema has no advisory-context channel (`additional_context`
/// exists only on the sessionStart/postToolUse events) -- and a Rewrite
/// under capabilities without `updated_input` becomes a deny carrying the
/// rewrite's attributed reason, because the matched input must not run
/// unreplaced.
pub fn render_cursor_envelope(
    result: &CanonicalResult,
    original_input: Option<&Value>,
    rewrite_key: &str,
    capabilities: &Capabilities,
) -> Value {
    match result.verdict {
        CanonicalVerdict::Allow | CanonicalVerdict::Warn => {
            serde_json::json!({ "permission": "allow" })
        }
        CanonicalVerdict::Rewrite if capabilities.supports_updated_input => {
            // `updated_input` is Cursor's "modified tool input to use
            // instead": a complete replacement object, so every field the
            // harness sent -- modeled or not -- is copied in and only the
            // rewrite key is substituted.
            let mut updated_input = original_input
                .and_then(Value::as_object)
                .cloned()
                .unwrap_or_default();
            if let Some(rewrite) = &result.rewrite {
                updated_input.insert(rewrite_key.to_string(), Value::String(rewrite.clone()));
            }
            serde_json::json!({ "permission": "allow", "updated_input": updated_input })
        }
        CanonicalVerdict::Rewrite | CanonicalVerdict::Deny => {
            let reason = result.attributed_reason();
            serde_json::json!({
                "permission": "deny",
                "user_message": reason,
                "agent_message": reason,
            })
        }
    }
}

/// Render one canonical result in Gemini CLI's `BeforeTool` response schema,
/// degraded according to the given capabilities.
///
/// Gemini's schema differs from the Claude/Codex envelope in both channels
/// ICG uses. A denial is the common top-level pair `{"decision": "deny",
/// "reason": ...}` -- `reason` is required when denied and is delivered to
/// the agent as the tool error, which stops the tool while letting the turn
/// continue. A rewrite is `hookSpecificOutput.tool_input`, an object Gemini
/// merges with and overrides the model's arguments with, so the renderer
/// emits the complete replacement input and lets Gemini merge it.
///
/// What is deliberately never emitted: a top-level `decision: "allow"`,
/// whose `BeforeTool` impact is unspecified and could stand in for Gemini's
/// own confirmation flow. An allow -- and a warning, which has no
/// advisory-context channel on this event and degrades to a bare allow per
/// §5 -- therefore renders the permissive object with no `decision` field
/// at all: Gemini proceeds unless told otherwise. The warning's attributed
/// reason rides the common `systemMessage` field, which Gemini shows its
/// user.
pub fn render_gemini_envelope(
    result: &CanonicalResult,
    original_input: Option<&Value>,
    rewrite_key: &str,
    capabilities: &Capabilities,
) -> Value {
    match result.verdict {
        CanonicalVerdict::Allow => serde_json::json!({}),
        CanonicalVerdict::Warn => {
            serde_json::json!({ "systemMessage": result.attributed_reason() })
        }
        CanonicalVerdict::Rewrite if capabilities.supports_updated_input => {
            // `tool_input` overrides the model's arguments field-by-field,
            // so the replacement must be complete: every field the harness
            // sent -- modeled or not -- with only the rewrite key
            // substituted.
            let mut tool_input = original_input
                .and_then(Value::as_object)
                .cloned()
                .unwrap_or_default();
            if let Some(rewrite) = &result.rewrite {
                tool_input.insert(rewrite_key.to_string(), Value::String(rewrite.clone()));
            }
            serde_json::json!({ "hookSpecificOutput": { "tool_input": tool_input } })
        }
        // Degradation (§5): a harness with no input-modification channel
        // cannot run a call whose arguments the policy said to change. The
        // deny is the fail-safe, and `reason` is required when denied.
        CanonicalVerdict::Rewrite | CanonicalVerdict::Deny => {
            serde_json::json!({
                "decision": "deny",
                "reason": result.attributed_reason(),
            })
        }
    }
}

/// Render one canonical result in OpenCode's plugin protocol, degraded
/// according to the given capabilities.
///
/// OpenCode's `tool.execute.before` hook has no response envelope to
/// parse -- the plugin acts, and its three possible actions are the whole
/// protocol. The JSON object this renderer emits names exactly one of them:
///
/// - `{"action": "allow"}` -- return normally, `args` untouched. A Warn
///   renders the same object: there is no advisory channel at tool-call
///   time, so the attributed reason is dropped rather than blocking (§5).
/// - `{"action": "rewrite", "args": ...}` -- the complete replacement args
///   object: every field the harness sent -- `workdir`, `replaceAll`,
///   anything this contract does not model -- with only the rewrite key
///   substituted. The plugin copies its properties onto its `output.args`
///   **in place**; reassigning `output.args` is invisible to the executor,
///   and OpenCode's transcript records the model's original args either
///   way, so the rewrite's audit trail is the plugin's own.
/// - `{"action": "deny", "message": "ICG: <attributed reason>"}` -- throw
///   `new Error(message)` verbatim. The thrown message is the only
///   model-visible text the gate controls on this harness (`Tool execution
///   failed: ICG: ...`), and because the agent loop continues, retries
///   arrive and are gated again. OpenCode aborts the call before execution
///   and before its own permission ask.
pub fn render_opencode_envelope(
    result: &CanonicalResult,
    original_input: Option<&Value>,
    rewrite_key: &str,
    capabilities: &Capabilities,
) -> Value {
    match result.verdict {
        CanonicalVerdict::Allow | CanonicalVerdict::Warn => {
            serde_json::json!({ "action": "allow" })
        }
        CanonicalVerdict::Rewrite if capabilities.supports_updated_input => {
            let mut args = original_input
                .and_then(Value::as_object)
                .cloned()
                .unwrap_or_default();
            if let Some(rewrite) = &result.rewrite {
                args.insert(rewrite_key.to_string(), Value::String(rewrite.clone()));
            }
            serde_json::json!({ "action": "rewrite", "args": args })
        }
        // Degradation (§5): a harness with no input-modification channel
        // cannot run a call whose arguments the policy said to change.
        // OpenCode's capabilities declare the replacement channel, so this
        // arm is the Deny verdict; the Rewrite arm above is the only path
        // that carries a replacement.
        CanonicalVerdict::Rewrite | CanonicalVerdict::Deny => {
            serde_json::json!({
                "action": "deny",
                "message": format!("ICG: {}", result.attributed_reason()),
            })
        }
    }
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
        assert!(adapter_for(HarnessId::OpenCode).is_some());
        assert!(adapter_for(HarnessId::GeminiCli).is_some());
        assert!(adapter_for(HarnessId::Cursor).is_some());
        // The wrapper front end has no payload and therefore no adapter:
        // the front end must refuse it rather than serve a wire format it
        // never receives input in.
        assert!(adapter_for(HarnessId::Wrapper).is_none());
        let default = default_adapter();
        assert!(std::ptr::eq(
            adapter_for(HarnessId::ClaudeCode).expect("claude adapter exists"),
            default
        ));
        // Cursor's dedicated shell event has its own adapter with its own
        // capabilities; `--harness cursor` alone serves preToolUse.
        assert!(!std::ptr::eq(
            adapter_for(HarnessId::Cursor).expect("cursor adapter exists"),
            cursor_shell_execution_adapter()
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
        assert!(with_path
            .attributed_reason()
            .contains("path=.beads/beads.db"));
        assert!(!with_path.attributed_reason().contains("file="));

        let subject_only = CanonicalResult {
            matched_path: None,
            ..with_path.clone()
        };
        assert!(subject_only
            .attributed_reason()
            .contains("file=deploy/app.yaml"));
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
            response["hookSpecificOutput"]["permissionDecision"], "allow",
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
        assert!(response["hookSpecificOutput"]
            .get("additionalContext")
            .is_none());
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

        assert_eq!(
            response["hookSpecificOutput"]["permissionDecision"],
            "allow"
        );
        assert_eq!(
            response["hookSpecificOutput"]["additionalContext"],
            "check the target [pack=warning-verdict-e2e, pattern=warn-worktree-add]"
        );
        assert!(response["hookSpecificOutput"].get("updatedInput").is_none());
        assert!(response["hookSpecificOutput"]
            .get("permissionDecisionReason")
            .is_none());
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
            ..CLAUDE_CODE_CAPABILITIES
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

        assert_eq!(
            response["hookSpecificOutput"]["permissionDecision"],
            "allow"
        );
        assert!(response["hookSpecificOutput"]
            .get("additionalContext")
            .is_none());
    }

    #[test]
    fn codex_declares_its_context_limitation_without_losing_the_field() {
        let capabilities = adapter_for(HarnessId::CodexCli).unwrap().capabilities();
        assert!(
            capabilities.supports_additional_context,
            "the field is still sent"
        );
        assert!(
            !capabilities.honors_additional_context,
            "Codex parses additionalContext but does not honor it yet"
        );
        assert!(capabilities.supports_system_message);
        assert!(
            !capabilities.supports_updated_input,
            "Codex refuses updatedInput: its only precondition, an accepted \
             permissionDecision:allow, is itself unsupported there"
        );
        assert!(
            !capabilities.supports_allow_decision,
            "Codex CLI {CODEX_RUNTIME_PIN} honors permissionDecision:deny alone"
        );
    }

    /// Codex logs `PreToolUse hook returned unsupported permissionDecision:allow`
    /// for every call ICG lets through, so an allowing verdict must assert no
    /// decision at all there. The envelope still identifies the event.
    #[test]
    fn an_allow_omits_the_decision_where_the_harness_refuses_to_be_granted_to() {
        let response = render_decision_envelope(
            &CanonicalResult {
                verdict: CanonicalVerdict::Allow,
                reason: None,
                pack_id: None,
                pattern_id: None,
                matched_path: None,
                rewrite: None,
                subject: None,
            },
            None,
            "command",
            &CODEX_CAPABILITIES,
        );

        assert!(
            response["hookSpecificOutput"]
                .get("permissionDecision")
                .is_none(),
            "an allow Codex would reject is spelled as no opinion, not as allow"
        );
        assert_eq!(
            response["hookSpecificOutput"]["hookEventName"],
            "PreToolUse"
        );
    }

    /// The same allow keeps asserting the grant on Claude Code: omission is a
    /// per-harness degradation, not a change of default behavior.
    #[test]
    fn an_allow_still_asserts_the_grant_where_the_harness_reads_it() {
        let response = render_decision_envelope(
            &CanonicalResult {
                verdict: CanonicalVerdict::Allow,
                reason: None,
                pack_id: None,
                pattern_id: None,
                matched_path: None,
                rewrite: None,
                subject: None,
            },
            None,
            "command",
            &CLAUDE_CODE_CAPABILITIES,
        );

        assert_eq!(
            response["hookSpecificOutput"]["permissionDecision"],
            "allow"
        );
    }

    /// The regression this whole change exists for: under the shipped Codex
    /// capabilities a force-push came back as `allow` + `updatedInput`, both
    /// of which Codex refuses -- so the unstripped `--force` ran. It must now
    /// be a deny carrying the rewrite's reason.
    #[test]
    fn codex_denies_a_force_push_instead_of_silently_failing_to_rewrite_it() {
        let response = render_decision_envelope(
            &CanonicalResult::from_engine(&rewrite("git", "git-force-push"), None),
            Some(&json!({ "command": "git push --force origin main" })),
            "command",
            &CODEX_CAPABILITIES,
        );
        let out = &response["hookSpecificOutput"];

        assert_eq!(out["permissionDecision"], "deny");
        assert_eq!(
            out["permissionDecisionReason"],
            "use the safe form [pack=git, pattern=git-force-push]"
        );
        assert!(
            out.get("updatedInput").is_none(),
            "Codex rejects updatedInput; sending it is what let the force-push through"
        );
    }

    /// Codex requires a non-empty `permissionDecisionReason` on every deny --
    /// it rejects a bare one -- so no deny path may render without text.
    #[test]
    fn every_deny_carries_a_non_empty_reason_for_codex() {
        for result in [
            CanonicalResult::from_engine(&rewrite("git", "git-force-push"), None),
            CanonicalResult {
                verdict: CanonicalVerdict::Deny,
                reason: Some("never force-push".to_string()),
                pack_id: Some("git".to_string()),
                pattern_id: Some("git-force-push".to_string()),
                matched_path: None,
                rewrite: None,
                subject: None,
            },
        ] {
            let out = render_decision_envelope(&result, None, "command", &CODEX_CAPABILITIES);
            let out = &out["hookSpecificOutput"];

            assert_eq!(out["permissionDecision"], "deny");
            assert!(
                out["permissionDecisionReason"]
                    .as_str()
                    .is_some_and(|reason| !reason.is_empty()),
                "Codex rejects a deny whose reason is empty or absent"
            );
        }
    }

    #[test]
    fn cursor_classifies_shell_and_edit_shaped_writes_through_the_engine() {
        let engine = Engine::new();
        let cursor = adapter_for(HarnessId::Cursor).expect("cursor adapter exists");

        let shell = cursor.build_request(
            &engine,
            payload(
                "Shell",
                json!({ "command": "git status", "working_directory": "/project" }),
            ),
            None,
        );
        assert_eq!(shell.harness, HarnessId::Cursor);
        assert_eq!(shell.tool_name, "Shell");
        assert_eq!(shell.rewrite_key(), "command");

        // Cursor's edits arrive under the `Write` tool name with an
        // old/new pair; the rewrite must follow the incoming spelling.
        let edit = cursor.build_request(
            &engine,
            payload(
                "Write",
                json!({
                    "file_path": "deploy/app.yaml",
                    "old_string": "storageClassName: sata",
                    "new_string": "storageClassName: ssd"
                }),
            ),
            Some(&json!({
                "file_path": "deploy/app.yaml",
                "old_string": "storageClassName: sata",
                "new_string": "storageClassName: ssd"
            })),
        );
        assert!(matches!(edit.action(), CanonicalAction::EditFile { .. }));
        assert_eq!(edit.rewrite_key(), "new_string");
    }

    #[test]
    fn cursor_declares_no_context_channel_and_no_system_message() {
        let capabilities = adapter_for(HarnessId::Cursor).unwrap().capabilities();
        assert!(capabilities.supports_updated_input);
        assert!(
            !capabilities.supports_additional_context,
            "additional_context exists only on Cursor's after-tool events, \
             never on a permission decision"
        );
        assert!(!capabilities.honors_additional_context);
        assert!(
            !capabilities.supports_system_message,
            "Cursor's permission hooks block a response that does not match \
             the schema; systemMessage is not in it"
        );

        let shell_event = cursor_shell_execution_adapter().capabilities();
        assert!(
            !shell_event.supports_updated_input,
            "beforeShellExecution output has no updated_input field"
        );
        assert!(!shell_event.supports_system_message);
    }

    #[test]
    fn cursor_renders_its_native_flat_envelope() {
        let cursor = adapter_for(HarnessId::Cursor).unwrap();
        let input = json!({
            "command": "git push --force origin main",
            "working_directory": "/project"
        });

        let allow = cursor.render(
            &CanonicalResult::from_engine(&CheckResult::Allowed, None),
            None,
            "command",
        );
        assert_eq!(allow, json!({ "permission": "allow" }));

        let deny = cursor.render(
            &CanonicalResult::from_engine(&denied("git", "git-force-push"), None),
            Some(&input),
            "command",
        );
        assert_eq!(
            deny,
            json!({
                "permission": "deny",
                "user_message": "denied by policy [pack=git, pattern=git-force-push]",
                "agent_message": "denied by policy [pack=git, pattern=git-force-push]",
            })
        );

        let rewrite = cursor.render(
            &CanonicalResult::from_engine(&rewrite("git", "git-force-push"), None),
            Some(&input),
            "command",
        );
        assert_eq!(
            rewrite,
            json!({
                "permission": "allow",
                "updated_input": {
                    "command": "git push origin main",
                    "working_directory": "/project"
                }
            })
        );
    }

    #[test]
    fn a_cursor_warning_is_a_bare_allow_and_a_deny_is_schema_exact() {
        let cursor = adapter_for(HarnessId::Cursor).unwrap();
        let warning = CanonicalResult {
            verdict: CanonicalVerdict::Warn,
            reason: Some("check the target".to_string()),
            pack_id: Some("warning-verdict-e2e".to_string()),
            pattern_id: Some("warn-worktree-add".to_string()),
            matched_path: None,
            rewrite: None,
            subject: None,
        };
        let response = cursor.render(&warning, None, "command");
        assert_eq!(
            response,
            json!({ "permission": "allow" }),
            "the decision schema has no advisory-context channel; the warning \
             text is dropped, never turned into a block or an unknown field"
        );

        let deny = cursor.render(
            &CanonicalResult::from_engine(&denied("git", "git-force-push"), None),
            None,
            "command",
        );
        assert!(
            deny.get("updated_input").is_none(),
            "a deny never carries a replacement input"
        );
        assert!(
            deny.get("hookSpecificOutput").is_none(),
            "the native envelope is flat: no Claude envelope may leak into it"
        );
    }

    #[test]
    fn the_cursor_shell_event_degrades_a_rewrite_to_a_deny() {
        let shell_event = cursor_shell_execution_adapter();
        let input = json!({ "command": "git push --force origin main", "sandbox": false });
        let response = shell_event.render(
            &CanonicalResult::from_engine(&rewrite("git", "git-force-push"), None),
            Some(&input),
            "command",
        );

        assert_eq!(
            response,
            json!({
                "permission": "deny",
                "user_message":
                    "use the safe form [pack=git, pattern=git-force-push]",
                "agent_message":
                    "use the safe form [pack=git, pattern=git-force-push]",
            }),
            "beforeShellExecution has no updated_input field: the matched \
             command must not run unreplaced, so the rewrite degrades to a \
             deny carrying the rewrite's reason"
        );
    }

    #[test]
    fn gemini_tools_classify_through_the_engine_not_alongside_it() {
        let engine = Engine::new();
        let gemini = adapter_for(HarnessId::GeminiCli).expect("gemini adapter exists");

        // Shell: `run_shell_command` carries the Bash payload shape, and the
        // adapter's classification is the engine's own.
        let direct = Engine::input_source_from_pre_tool_use(payload(
            "run_shell_command",
            json!({ "command": "git status" }),
        ))
        .expect("engine parses run_shell_command")
        .expect("run_shell_command is supported");
        let shell = gemini.build_request(
            &engine,
            payload("run_shell_command", json!({ "command": "git status" })),
            None,
        );
        assert_eq!(shell.harness, HarnessId::GeminiCli);
        assert_eq!(shell.tool_name, "run_shell_command");
        assert_eq!(shell.input_source, Some(direct));
        assert_eq!(shell.rewrite_key(), "command");

        // Full-file write: `write_file` carries the Write payload shape.
        let write = gemini.build_request(
            &engine,
            payload(
                "write_file",
                json!({ "file_path": "deploy/app.yaml", "content": "x" }),
            ),
            None,
        );
        assert_eq!(
            write.action(),
            CanonicalAction::WriteFile {
                file_path: "deploy/app.yaml".to_string(),
                content: "x".to_string(),
            }
        );
        assert_eq!(write.rewrite_key(), "content");

        // Text substitution: `replace` carries the Edit payload shape, and
        // the rewrite follows the incoming snake_case spelling.
        let edit_input = json!({
            "file_path": "deploy/app.yaml",
            "old_string": "storageClassName: sata",
            "new_string": "storageClassName: ssd"
        });
        let replace = gemini.build_request(
            &engine,
            payload("replace", edit_input.clone()),
            Some(&edit_input),
        );
        assert!(matches!(replace.action(), CanonicalAction::EditFile { .. }));
        assert_eq!(replace.rewrite_key(), "new_string");
    }

    #[test]
    fn gemini_renders_the_native_decision_envelope() {
        let gemini = adapter_for(HarnessId::GeminiCli).unwrap();
        let input = json!({ "command": "git push --force origin main" });

        // An allow is the permissive object: no `decision` field at all --
        // `decision: "allow"` is never emitted, because its BeforeTool
        // impact is unspecified.
        let allow = gemini.render(
            &CanonicalResult::from_engine(&CheckResult::Allowed, None),
            None,
            "command",
        );
        assert_eq!(allow, json!({}));

        // A deny is the top-level pair; `reason` is required when denied and
        // is what Gemini delivers to the agent as the tool error -- which
        // stops the tool while letting the turn continue. No Claude
        // envelope may leak into the native schema.
        let deny = gemini.render(
            &CanonicalResult::from_engine(&denied("git", "git-force-push"), None),
            Some(&input),
            "command",
        );
        assert_eq!(
            deny,
            json!({
                "decision": "deny",
                "reason": "denied by policy [pack=git, pattern=git-force-push]",
            })
        );
        assert!(deny.get("hookSpecificOutput").is_none());
        assert!(deny.get("permissionDecision").is_none());
        assert!(deny.get("updatedInput").is_none());
    }

    #[test]
    fn a_gemini_warning_is_a_bare_allow_plus_system_message() {
        let gemini = adapter_for(HarnessId::GeminiCli).unwrap();
        let warning = CanonicalResult {
            verdict: CanonicalVerdict::Warn,
            reason: Some("check the target".to_string()),
            pack_id: Some("warning-verdict-e2e".to_string()),
            pattern_id: Some("warn-worktree-add".to_string()),
            matched_path: None,
            rewrite: None,
            subject: None,
        };
        let response = gemini.render(&warning, None, "command");

        assert_eq!(
            response,
            json!({
                "systemMessage":
                    "check the target [pack=warning-verdict-e2e, pattern=warn-worktree-add]",
            }),
            "the warning text degrades onto systemMessage -- never into a \
             decision, and never into a block"
        );
        assert!(response.get("decision").is_none());
        assert!(response.get("additionalContext").is_none());
    }

    #[test]
    fn a_gemini_rewrite_rides_hook_specific_output_tool_input() {
        let gemini = adapter_for(HarnessId::GeminiCli).unwrap();
        // Harness-specific fields this contract does not model must survive
        // into the replacement: Gemini merges `tool_input` over the model's
        // arguments, so a partial object would leave the matched field
        // intact underneath.
        let original = json!({
            "command": "git push --force origin main",
            "description": "Push reviewed changes",
            "timeout": 120000,
        });
        let response = gemini.render(
            &CanonicalResult::from_engine(&rewrite("git", "git-force-push"), None),
            Some(&original),
            "command",
        );

        assert_eq!(
            response,
            json!({
                "hookSpecificOutput": {
                    "tool_input": {
                        "command": "git push origin main",
                        "description": "Push reviewed changes",
                        "timeout": 120000,
                    }
                }
            })
        );
        assert!(
            response.get("decision").is_none(),
            "a rewrite allows the call with replacement arguments, and \
             decision allow is never emitted"
        );
        assert!(
            response.get("reason").is_none(),
            "`reason` is required when denied and meaningless otherwise"
        );
    }

    #[test]
    fn gemini_declares_its_capability_set() {
        let capabilities = adapter_for(HarnessId::GeminiCli).unwrap().capabilities();
        assert!(
            capabilities.supports_updated_input,
            "hookSpecificOutput.tool_input is the rewrite channel"
        );
        assert!(
            !capabilities.supports_additional_context,
            "a BeforeTool response has no advisory-context channel"
        );
        assert!(!capabilities.honors_additional_context);
        assert!(
            capabilities.supports_system_message,
            "the common systemMessage field carries warning and report text"
        );
    }

    #[test]
    fn a_gemini_harness_without_an_update_channel_degrades_rewrite_to_deny() {
        let no_rewrite = Capabilities {
            supports_updated_input: false,
            ..GEMINI_CLI_CAPABILITIES
        };
        let response = render_gemini_envelope(
            &CanonicalResult::from_engine(&rewrite("git", "git-force-push"), None),
            Some(&json!({ "command": "git push --force origin main" })),
            "command",
            &no_rewrite,
        );
        assert_eq!(
            response,
            json!({
                "decision": "deny",
                "reason": "use the safe form [pack=git, pattern=git-force-push]",
            }),
            "the matched input must not run unreplaced: without the \
             tool_input channel the rewrite degrades to a deny"
        );
    }

    #[test]
    fn gemini_unmodeled_tools_classify_unsupported_and_allow() {
        let engine = Engine::new();
        let gemini = adapter_for(HarnessId::GeminiCli).unwrap();
        let request = gemini.build_request(
            &engine,
            payload("mcp__github__merge_pull_request", json!({ "pr": 7 })),
            Some(&json!({ "pr": 7 })),
        );
        assert_eq!(request.action(), CanonicalAction::Unsupported);

        // §3.4/§8: an unmodeled tool is a contract allow, not a failure --
        // the permissive object, with no diagnostic implied.
        let response = gemini.render(
            &CanonicalResult::from_engine(&CheckResult::Allowed, None),
            None,
            "command",
        );
        assert_eq!(response, json!({}));
    }

    #[test]
    fn opencode_tools_classify_through_the_engine_not_alongside_it() {
        let engine = Engine::new();
        let opencode = adapter_for(HarnessId::OpenCode).expect("opencode adapter exists");

        // Shell: `bash` carries the Bash payload shape, and the adapter's
        // classification is the engine's own.
        let direct = Engine::input_source_from_pre_tool_use(payload(
            "bash",
            json!({ "command": "git status" }),
        ))
        .expect("engine parses bash")
        .expect("bash is supported");
        let shell = opencode.build_request(
            &engine,
            payload("bash", json!({ "command": "git status" })),
            None,
        );
        assert_eq!(shell.harness, HarnessId::OpenCode);
        assert_eq!(shell.tool_name, "bash");
        assert_eq!(shell.input_source, Some(direct));
        assert_eq!(shell.rewrite_key(), "command");

        // Full-file write: `write` carries the camelCase Write shape
        // (`filePath`/`content`).
        let write = opencode.build_request(
            &engine,
            payload(
                "write",
                json!({ "filePath": "deploy/app.yaml", "content": "x" }),
            ),
            None,
        );
        assert_eq!(
            write.action(),
            CanonicalAction::WriteFile {
                file_path: "deploy/app.yaml".to_string(),
                content: "x".to_string(),
            }
        );
        assert_eq!(write.rewrite_key(), "content");

        // Text substitution: `edit` carries the camelCase Edit shape, and
        // the rewrite follows the incoming camelCase spelling -- OpenCode's
        // pinned args spellings (`filePath`/`oldString`/`newString`,
        // `replaceAll` unmodeled) are the §3.2 aliases.
        let edit_input = json!({
            "filePath": "deploy/app.yaml",
            "oldString": "storageClassName: sata",
            "newString": "storageClassName: ssd",
            "replaceAll": true
        });
        let edit = opencode.build_request(
            &engine,
            payload("edit", edit_input.clone()),
            Some(&edit_input),
        );
        assert!(matches!(edit.action(), CanonicalAction::EditFile { .. }));
        assert_eq!(edit.rewrite_key(), "newString");
        // The unmodeled `replaceAll` survives on the request untouched, so
        // the rewrite replacement carries it back.
        assert_eq!(
            edit.original_tool_input.expect("original input")["replaceAll"],
            true
        );
    }

    #[test]
    fn opencode_renders_its_plugin_protocol_envelope() {
        let opencode = adapter_for(HarnessId::OpenCode).unwrap();
        let input = json!({
            "command": "git push --force origin main",
            "workdir": "/project"
        });

        // An allow is an explicit action: the plugin returns normally and
        // leaves `args` untouched.
        let allow = opencode.render(
            &CanonicalResult::from_engine(&CheckResult::Allowed, None),
            None,
            "command",
        );
        assert_eq!(allow, json!({ "action": "allow" }));

        // A deny names the message the plugin must throw verbatim. The
        // thrown text is the only model-visible string the gate controls on
        // this harness, so it carries the ICG prefix and the attribution.
        let deny = opencode.render(
            &CanonicalResult::from_engine(&denied("git", "git-force-push"), None),
            Some(&input),
            "command",
        );
        assert_eq!(
            deny,
            json!({
                "action": "deny",
                "message": "ICG: denied by policy [pack=git, pattern=git-force-push]",
            })
        );
        assert!(deny.get("hookSpecificOutput").is_none());
        assert!(deny.get("permissionDecision").is_none());
        assert!(deny.get("args").is_none(), "a deny never carries args");

        // A rewrite is the complete replacement args object: unmodeled
        // fields survive, and the plugin applies it by in-place property
        // mutation.
        let rewrite = opencode.render(
            &CanonicalResult::from_engine(&rewrite("git", "git-force-push"), None),
            Some(&input),
            "command",
        );
        assert_eq!(
            rewrite,
            json!({
                "action": "rewrite",
                "args": {
                    "command": "git push origin main",
                    "workdir": "/project"
                }
            })
        );
    }

    #[test]
    fn an_opencode_warning_is_a_bare_allow() {
        let opencode = adapter_for(HarnessId::OpenCode).unwrap();
        let warning = CanonicalResult {
            verdict: CanonicalVerdict::Warn,
            reason: Some("check the target".to_string()),
            pack_id: Some("warning-verdict-e2e".to_string()),
            pattern_id: Some("warn-worktree-add".to_string()),
            matched_path: None,
            rewrite: None,
            subject: None,
        };
        let response = opencode.render(&warning, None, "command");

        assert_eq!(
            response,
            json!({ "action": "allow" }),
            "tool.execute.before has no advisory channel: the warning text is \
             dropped, never turned into a throw"
        );
    }

    #[test]
    fn opencode_declares_its_capability_set() {
        let capabilities = adapter_for(HarnessId::OpenCode).unwrap().capabilities();
        assert!(
            capabilities.supports_updated_input,
            "in-place args mutation is execution-real (the hook and the \
             executor share one args object)"
        );
        assert!(
            !capabilities.supports_additional_context,
            "tool.execute.before's only output is the args object; hook \
             return values are discarded"
        );
        assert!(!capabilities.honors_additional_context);
        assert!(
            !capabilities.supports_system_message,
            "the response has no user-facing field; diagnostics go to stderr"
        );
        assert!(
            !capabilities.supports_allow_decision,
            "there is no decision field on this wire: an allow is spelled by \
             the plugin returning normally"
        );
    }

    #[test]
    fn opencode_unmodeled_tools_classify_unsupported_and_allow() {
        let engine = Engine::new();
        let opencode = adapter_for(HarnessId::OpenCode).unwrap();
        // OpenCode's own read-only tools and MCP namespaced keys pass
        // through the same hook; none are modeled here.
        let request = opencode.build_request(
            &engine,
            payload("glob", json!({ "pattern": "**/*.rs" })),
            Some(&json!({ "pattern": "**/*.rs" })),
        );
        assert_eq!(request.action(), CanonicalAction::Unsupported);

        let response = opencode.render(
            &CanonicalResult::from_engine(&CheckResult::Allowed, None),
            None,
            "command",
        );
        assert_eq!(response, json!({ "action": "allow" }));
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
