# Harness adapter contract (version 1)

This note is the normative description of ICG's harness-adapter contract,
version 1, implemented in `src/adapter.rs` and exercised end-to-end by
`tests/adapter_contract_tests.rs`. The contract separates **shared policy
evaluation** (the engine and rule packs, which never change per harness) from
**harness wire formats** (adapters, which contain no policy).

An adapter translates its harness's inbound payload into exactly one
canonical request, the engine evaluates that request against the packs, and
the adapter translates the one canonical result back out in the harness's own
response envelope. An adapter is deliberately thin: it owns identity, field
mapping, and response rendering, and nothing else. An adapter that reached
around the engine — its own rules, its own severities, its own
fail-open decision — would be a policy fork, not an adapter.

Companion notes: [`pretooluse-response-schema.md`](pretooluse-response-schema.md)
(the shared response envelope, field by field) and
[`multi-harness-integration.md`](multi-harness-integration.md) (why native
hooks and the PATH wrapper run as two independent layers).

## 1. Versioning

`ADAPTER_CONTRACT_VERSION = 1` (in `src/adapter.rs`) is the version of the
canonical request/result schema — it is not any harness's own protocol
version. Harness protocols have their own versioning where they have any at
all (§6).

- Additive changes — a new harness variant, a new optional canonical field —
  keep the version.
- A change that reinterprets an existing field or verdict **must** bump the
  version and migrate every adapter in the same commit. The process-boundary
  test `the_canonical_request_carries_the_current_contract_version` pins the
  constant against a copy in the test, so a bump without acknowledgment fails
  the build.

## 2. Harness and source identity

Identity is **declared, never sniffed**. `icg hook --harness <id>` names the
calling harness; the adapter is selected from that declaration before any
state is touched.

`HarnessId` is a **closed enum**:

| Variant | Telemetry slug | Adapter | Wire |
| --- | --- | --- | --- |
| `ClaudeCode` | `claude-code` | `ClaudeCodeAdapter` (shipped) | `PreToolUse` hook, JSON on stdin/stdout |
| `CodexCli` | `codex-cli` | `CodexAdapter` (shipped) | `PreToolUse` hook, JSON on stdin/stdout |
| `OpenCode` | `opencode` | none yet (§6.3) | in-process plugin API |
| `GeminiCli` | `gemini-cli` | none yet (§6.4) | `BeforeTool` command hook, JSON on stdin/stdout |
| `Cursor` | `cursor` | `CursorAdapter` (shipped), `CursorShellExecutionAdapter` (§6.5) | agent hooks, JSON on stdin/stdout |
| `Wrapper` | `wrapper` | none (no payload) | shadowed argv via `execvp` |

Rules the enum enforces:

1. **Default.** Without `--harness`, the hook uses `default_adapter()` — the
   Claude Code adapter. The undecorated invocation that predates this
   contract behaves byte-identically to before it existed (locked by the
   `undecorated_hook_matches_the_declared_claude_code_envelope` test).
2. **Refusal.** A declared harness whose adapter is specified but not
   implemented is refused with a non-zero exit, a stderr message naming the
   harness, an **empty stdout**, and **no evaluation telemetry record** —
   before any pack is evaluated or stdin is read. A harness must never be
   served another harness's
   response envelope: OpenCode would ignore it, Gemini CLI would fail to
   parse it, and the guarded call would proceed unchecked while looking
   guarded.
3. **Telemetry slug.** Telemetry records `HarnessId::as_slug()` — a fixed,
   lowercase, kebab-case, non-secret name from the closed set above. There is
   no code path that could store a free-form harness name (§8).

## 3. Canonical request

`CanonicalRequest` carries: `contract_version`, `harness`, `tool_name` (the
harness's own spelling, e.g. `Bash`, `apply_patch`,
`mcp__github__merge_pull_request`), `input_source` (the engine's own
representation — the canonical view never diverges from what the engine
evaluates), and `original_tool_input` (the raw JSON object, preserved
untouched so a rewrite can return a *complete* replacement including fields
this contract does not model).

`CanonicalRequest::action()` classifies into `CanonicalAction`. The mapping
per input shape:

### 3.1 Command tools

A single shell command line → `CanonicalAction::Command`.

| Harness | Tool | Payload field |
| --- | --- | --- |
| Claude Code | `Bash` | `tool_input.command` |
| Codex CLI | `Bash` | `tool_input.command` |

The payload wire is snake_case (`tool_name`/`tool_input`); the engine also
accepts the camelCase aliases (`toolName`/`toolInput`) of ICG's early
fixtures, on both shipped wires. A rewrite replaces the `command` field.

### 3.2 Single-file writes and edits

| Harness | Tool | Fields |
| --- | --- | --- |
| Claude Code | `Write` | `file_path` (alias `filePath`), `content`, optional `encoding`, `mime_type` |
| Claude Code | `Edit` | `file_path`, `old_string`, `new_string` (aliases `filePath`, `oldString`, `newString`) |
| Codex CLI | same shapes | same fields |

`Write` → `CanonicalAction::WriteFile`; `Edit` → `EditFile`. Both evaluate
against content-mode packs. A Write rewrite replaces `content`; an Edit
rewrite replaces `new_string` **following the incoming payload's spelling**
(`newString` if that is what the harness sent) — `rewrite_key()` derives the
field name from the request, not from a per-harness constant.

### 3.3 Patch-shaped edits

Codex's `apply_patch` tool carries a multi-file patch in
`tool_input.command`. It classifies as `CanonicalAction::Patch` via the
engine's `ContentBatch` — the engine parses `*** Add File:` / `*** Update
File:` hunks and evaluates every touched file against content-mode packs. A
patch rewrite replaces the whole patch text under `command`. `action()` lifts
the affected file paths for attribution; evaluation itself stays in the
engine.

### 3.4 Unsupported structured tools

Any tool outside the contract's supported set — MCP calls
(`mcp__*`), NotebookEdit, harness-internal tools — classifies as
`CanonicalAction::Unsupported` with `input_source: None`. The front end
renders a plain allow for it. This is **contract, not failure**: a wrapper
and the PATH-wrapper layer (see
[`multi-harness-integration.md`](multi-harness-integration.md)) remain the
defense for what structured tools actually execute.

## 4. Canonical result

The engine's `CheckResult` maps losslessly onto `CanonicalResult`:
`Allowed → Allow`, `Warning → Warn`, `Rewrite → Rewrite`,
`Denied → Deny`. `CanonicalResult::from_engine` also keeps the pack id,
pattern id, matched path (when the detection named one), and the rewrite
value (a replacement for **one field**, never a whole input object).

Every non-allow reason is rendered through `attributed_reason()`, which
appends the rule attribution every front end names the same way:

```
<reason> [pack=<pack_id>, pattern=<pattern_id>]
```

with `path=<matched path>` when the detection itself named a file, else
`file=<subject>` (the written file, or the comma-joined file list for a
patch) when the front end supplied one.

## 5. Response rendering and capability degradation

All payload-speaking adapters share one renderer,
`render_decision_envelope`, emitting the `hookSpecificOutput` envelope
described in [`pretooluse-response-schema.md`](pretooluse-response-schema.md):
exactly one JSON object on stdout, nothing else on stdout, diagnostics only
on stderr. `hookEventName: "PreToolUse"` is always set — Codex requires it,
Claude Code ignores it — which is what lets one envelope serve both shipped
harnesses.

What a harness's wire can express is declared per adapter as
`Capabilities`, and degradation is **contract behavior, not adapter
discretion**:

| Verdict | Full-capability render | Degraded render |
| --- | --- | --- |
| `Allow` | `permissionDecision: "allow"` | same |
| `Warn` | allow + `additionalContext: <attributed reason>` | `supports_additional_context: false` → **bare allow**, context dropped — never turned into a block |
| `Rewrite` | allow + `updatedInput` (complete replacement object: original input with the rewrite value substituted under the request's rewrite key) + `additionalContext` | `supports_updated_input: false` → **deny** carrying the rewrite's attributed reason — the matched input must not run unreplaced |
| `Deny` | `permissionDecision: "deny"` + `permissionDecisionReason` (no `updatedInput`, ever) | same |

Shipped capabilities:

| Adapter | `supports_updated_input` | `supports_additional_context` | `honors_additional_context` | `supports_system_message` |
| --- | --- | --- | --- | --- |
| Claude Code | yes | yes | yes | yes |
| Codex CLI | yes | yes | **no** | yes |

`honors_additional_context` is informational: the Codex CLI parses
`additionalContext` but does not yet act on it (see
[`multi-harness-integration.md`](multi-harness-integration.md) for the
timeline). The adapter still sends the field, so the day Codex honors it the
warning text is already on the wire.

## 6. Per-harness specifications and official sources

Versions are stated as observed 2026-09-18. Each mapping below names the
exact official source it was taken from.

### 6.1 Claude Code (implemented)

- **Protocol:** native `PreToolUse` hook. Payload: JSON on stdin
  (`tool_name`, `tool_input`, `session_id`, ...). Response: JSON on stdout,
  `hookSpecificOutput` with `permissionDecision`
  (`allow`/`deny`/`ask` — ICG emits only `allow`/`deny`),
  `permissionDecisionReason`, `updatedInput`, `additionalContext`, plus the
  top-level `systemMessage`. Exit 0 = stdout parsed; exit 2 = blocking error
  with stderr as the reason (ICG does not use it); any other non-zero =
  non-blocking.
- **Timeout:** per-command `timeout` in the hook configuration, default 60 s.
  A timed-out hook is a non-blocking failure: the tool call **proceeds
  without a decision** (§7).
- **Config:** `~/.claude/settings.json` (user), `.claude/settings.json`
  (project).
- **Source:** <https://code.claude.com/docs/en/hooks> (`PreToolUse` decision
  control), re-checked 2026-09-18.

### 6.2 Codex CLI (implemented)

- **Protocol:** synchronous `PreToolUse` hook, structurally close to Claude
  Code's: JSON on stdin, JSON on stdout with the same
  `hookSpecificOutput.permissionDecision` envelope; `hookEventName` is
  required on the response. Covers `Bash` and `apply_patch` (patch text in
  `tool_input.command`).
- **Maturity:** shipped experimental ~v0.114 (March 2026), stable ~v0.124
  (April 2026). `additionalContext` is in the schema but not yet honored —
  encoded as `honors_additional_context: false` (§5).
- **Config:** `~/.codex/hooks.json` or repo `.codex/hooks.json`, gated by
  project trust. Not to be confused with cloud-hosted Codex tasks, which are
  out of reach for any host-level guard.
- **Sources:** <https://developers.openai.com/codex/hooks>; generated wire
  schema `codex-rs/hooks/schema/generated/pre-tool-use.command.input.schema.json`
  in `github.com/openai/codex`. Both re-checked 2026-09-18.

### 6.3 OpenCode (specified, not implemented)

- **Protocol:** an **in-process JavaScript/TypeScript plugin API**, not a
  subprocess wire. Plugins live in `.opencode/plugins/` (project) or
  `~/.config/opencode/plugins/` (global), or as npm packages via the
  `plugin` config array, typed by the `@opencode-ai/plugin` SDK. The
  relevant hook is `tool.execute.before`: input `{ tool, sessionID, callID }`,
  output `{ args }`, with `args` mutated in place. There is no stdout
  envelope to parse.
- **Mapping when implemented:** `tool` → canonical `tool_name`; `args` →
  canonical `tool_input` (field spellings per §3). A Deny is delivered by
  **throwing an error from the hook** (OpenCode aborts the tool call); a
  Rewrite by mutating `args` in place before returning. `tool.execute.before`
  has no advisory-context channel, so OpenCode's capabilities would declare
  `supports_additional_context: false` and a Warn degrades to a bare allow
  (§5). The plugin shells out to `icg hook --harness opencode` — the process
  boundary moves inside the plugin, but the canonical request/result and the
  engine are exactly as specified here.
- **Versioning:** the plugin API has no wire-protocol version field; the
  docs page below is normative.
- **Source:** <https://opencode.ai/docs/plugins> (official plugin
  documentation), retrieved 2026-09-18.

### 6.4 Gemini CLI (specified, not implemented)

- **Protocol:** `BeforeTool` **command hook**. Config: `hooks` object in
  `~/.gemini/settings.json` or project `.gemini/settings.json` — event
  `BeforeTool` → hook definitions (`matcher` regex over tool names,
  `sequential`, `hooks[]` with `type: "command"`, `command`, optional
  `timeout` in milliseconds, default 60000).
- **Input (stdin JSON):** `session_id`, `transcript_path`, `cwd`,
  `hook_event_name`, `timestamp`, plus per-event `tool_name`,
  `tool_input` (object), optional `mcp_context`, `original_request_name`.
  The Claude-compatible `tool_name`/`tool_input` pair is the payload a
  GeminiCli adapter would translate.
- **Output (stdout JSON):** common fields `decision` (`"deny"`, alias
  `"block"`), `reason` (**required when denied** — sent to the agent as the
  tool error), `systemMessage`, `suppressOutput`, `continue`, `stopReason`;
  event-specific `hookSpecificOutput.tool_input` — an object that
  **merges with and overrides** the model's arguments, which is the rewrite
  channel (a merge-override rather than Claude Code's whole-object
  `updatedInput`; the adapter would render the complete replacement and let
  Gemini CLI merge it).
- **Exit codes:** `0` = success, stdout parsed as JSON (**preferred for all
  logic**; silence on stdout is mandatory — exactly one JSON object, which is
  already this contract's rendering rule); `2` = system block with **stderr**
  as the rejection reason, the turn continues; **any other exit = non-fatal
  warning, the CLI continues** — a native fail-open posture that matches
  ICG's own.
- **Versioning:** the hooks system has no wire-protocol version field; the
  hook reference in the repository is normative. Observed against the
  `main` documentation tree of `google-gemini/gemini-cli`; latest release at
  time of writing v0.60.0 (2026-09-15).
- **Source:** `docs/hooks/reference.md` in `github.com/google-gemini/gemini-cli`
  (the official hooks specification: global mechanics, base input schema,
  `BeforeTool` input/output), retrieved 2026-09-18.

### 6.5 Cursor (implemented)

- **Protocol:** Cursor agent hooks, configured in `.cursor/hooks.json`
  (project) or `~/.cursor/hooks.json` (user), plus enterprise-managed files;
  layers merge Enterprise → Team → Project → User. The schema carries a
  **required top-level `"version": 1`** (positive integer) — the one harness
  here with an explicit protocol version. Relevant events: `preToolUse`
  (input: `tool_name`, `tool_input`, `tool_use_id`, `cwd`),
  `beforeShellExecution` (`command`, `cwd`, `sandbox`),
  `beforeReadFile`, `afterFileEdit`, `beforeMCPExecution`.
- **Placement:** Cursor **cloud agents read project-level hooks only —
  `~/.cursor/hooks.json` is not available to them** — so a cloud-agent
  session is covered by `.cursor/hooks.json` in the repository, and the
  user-level file covers local IDE sessions alone. Wire the layer the
  session actually runs under. `icg install-cursor-hooks` manages this
  wiring idempotently (project file by default; `--user` and `--file`
  select the other layers): ICG-owned entries are recognized by their
  command line (`<icg> hook --harness cursor …`) and replaced rather than
  duplicated, every unrelated hook, matcher and key is preserved
  verbatim, a missing file is created, and a file that does not parse —
  or carries a `version` other than 1 — fails with a clear error and is
  left unchanged. The prior file is backed up once as
  `<target>.icg-backup`.
- **Adapters (shipped):** `CursorAdapter` serves the generic `preToolUse`
  event — `icg hook --harness cursor` — and `CursorShellExecutionAdapter`
  serves the dedicated shell event — `icg hook --harness cursor --event
  before-shell-execution` (the event is refused under any other harness).
  The engine classifies Cursor's spellings for every harness: its shell
  tool is named `Shell` (Bash payload shape), and its edits arrive under
  the `Write` tool name shaped as an `old_string`/`new_string` pair with
  no `content` — classified as an edit so the introduced content is
  evaluated (§3.2).
- **Output (stdout JSON):** `permission`: `"allow"` / `"deny"` / `"ask"`
  (ICG emits only `allow`/`deny`), `user_message`, `agent_message`,
  `updated_input` (preToolUse input substitution — the rewrite channel).
  The native envelope is **flat** — no `hookSpecificOutput` wrapper — and
  a permission decision carries **no advisory-context channel**
  (`additional_context` exists only on the after-tool `postToolUse`
  events), so the adapter's capabilities declare
  `supports_additional_context: false`, a Warn degrades to a bare allow,
  and nothing outside the documented schema is ever emitted: Cursor's
  permission hooks **block a response that does not match the schema**.
  `updated_input` is a complete replacement input — every field the
  harness sent is copied through with only the rewrite key substituted.
  `beforeShellExecution` output has **no `updated_input` field**, so that
  event's capabilities declare `supports_updated_input: false` and a
  Rewrite degrades to a Deny carrying the rewrite's attributed reason (§5)
  — the event can refuse a command but cannot replace it. Exit code 2 =
  deny, Claude Code-compatible. Exit 0 = stdout parsed as JSON.
- **Failure semantics:** **invalid JSON or a schema mismatch on a permission
  hook blocks the call** (a natively fail-closed posture); other non-zero
  exits fail open unless the hook sets `failClosed: true`. A Cursor adapter
  must therefore treat stdout correctness as safety-critical to a degree the
  other harnesses do not — which is why it renders its own flat envelope
  rather than aliasing the Claude Code one, and why
  `beforeShellExecution` — whose payload has no `tool_name` and would fail
  the PreToolUse stdin parse fail-open unchecked — has its own admission
  path. The `beforeShellExecution` payload that is not valid JSON, or
  carries no non-empty `command`, fails open with a stderr diagnostic like
  the shared stdin boundary (§8).
- **Versioning:** hooks schema `version: 1` (required field in
  `hooks.json`).
- **Source:** <https://cursor.com/docs/agent/hooks> (official agent-hooks
  documentation: config paths, merge order, event payloads, output schema,
  exit codes, `failClosed`) and
  <https://cursor.com/docs/reference/third-party-hooks> (Claude Code
  compatibility), re-checked 2026-09-19.
- **End-to-end proof:** [`scripts/cursor-dispatch-e2e`](../../scripts/cursor-dispatch-e2e)
  plays Cursor's side of this wire from the same two documentation pages —
  a scratch project `.cursor/hooks.json`, the documented stdin payloads,
  and the documented interpretation of exit codes, invalid JSON, schema
  mismatches, and `failClosed` — and executes the surviving commands
  against harmless fake targets (a marker-creating denied command, a
  deliberately diverged scratch clone for the force-push rewrite, and the
  unchanged Claude hook under the third-party-import interpretation). It
  is the executable form of this section's claims about what Cursor's
  dispatch does with ICG's responses.

## 7. Timeouts

The contract's posture: **ICG is fast, local, and one-shot; the harness owns
timeout policy.**

- `icg hook` performs local file I/O only — pack loading, regex evaluation,
  telemetry append. It opens no network connections and never waits on
  harness state.
- It reads its payload from stdin once, evaluates, writes exactly one JSON
  object to stdout, and exits. Nothing on the hook path blocks or retries.
- Harness-side timeout configuration is the operator's availability lever:
  Claude Code per-command `timeout` (default 60 s), Gemini CLI `timeout`
  milliseconds per hook definition (default 60000), Codex per its hooks
  config. On a timed-out hook every listed harness proceeds without a
  decision — the same fail-open availability posture ICG itself chooses for
  its own faults (§8). Cursor is the exception to fail open: its invalid-JSON
  handling blocks unless configured otherwise (§6.5).
- Because a killed hook looks to the harness like any other failure, an
  operator who needs deterministic behavior under a hung guard should pair a
  short harness timeout with the guard-availability policy in
  [`fail-closed-policy.md`](fail-closed-policy.md) rather than rely on ICG
  being killed.

## 8. Malformed input and fail-open

Two boundaries, both inside the engine, both shared by every adapter:

1. **Unreadable/unparseable stdin.** `read_pre_tool_use_payload_from_stdin`
   wraps the read in a panic boundary: input that is not valid JSON, is
   missing `tool_name`, or cannot be read at all is reported on **stderr**
   (naming the fail-open mode) and yields `None`. The hook then emits a
   **plain allow envelope** and exits **0**. Locked by
   `malformed_input_fails_open_with_a_successful_process` against a
   deliberately truncated fixture.
2. **Unmodelable payload.** After a successful parse,
   `input_source_from_pre_tool_use_fail_open` wraps classification (patch
   parsing, path matching) in a catch-unwind boundary: an error **or panic**
   on input the parsers did not anticipate collapses to `None` → plain allow.
   Adapters classify **through** this boundary, never alongside it, so an
   adapter can never classify differently than the engine evaluates
   (`adapters_classify_through_the_engine_not_alongside_it`).

In both cases the process stays successful and the guarded call proceeds:
the guard never blocks work with its own failure. The stderr diagnostic is
the operator's signal that nothing was checked. The graduated
fail-open→fail-closed availability policy
([`fail-closed-policy.md`](fail-closed-policy.md)) changes what a boundary
failure *maps to* (deny instead of allow, and only when the operator has
opted in); the canonical request/result shapes above are unchanged.

Unrecognized-but-well-formed tools are **not** a failure case: they are
`Unsupported` by contract (§3.4) and render a plain allow without any
stderr diagnostic.

## 9. Telemetry hygiene

- A harness identifier reaches telemetry only as `HarnessId::as_slug()` —
  the closed, fixed, non-secret slug set of §2. The enum is closed
  specifically so a free-form name (which could carry anything) has no path
  into a record.
- Evaluation records stay **verdict-shaped**: `verdict`, `timestamp`,
  `session_id`, `release_ref`, `harness` — and nothing else. Nothing from
  the payload (command lines, file paths, file content, argument values)
  ever reaches a record.
- Records written before this contract (no `harness` field) deserialize with
  `harness: None`; old stores are read, not rejected.
- **No declaration, no identity:** an undecorated invocation records a null
  harness — never a guess from the payload's shape.
- Locked by
  `declared_harness_reaches_telemetry_as_a_slug_and_no_payload_does`, which
  asserts the record's **entire key set** and plants a payload-only marker
  that must not appear anywhere in the store.

## 10. Golden fixtures and the process boundary

`tests/adapter_contract_tests.rs` drives the **real binary**: each fixture
request is written to `icg hook`'s stdin exactly as the harness would send
it, and stdout is compared against the recorded response byte-for-byte. That
locks the whole boundary — adapter parse, engine evaluation, adapter render —
rather than any function the front end could silently stop calling.

Fixtures in `tests/fixtures/adapter/`, each `<name>.request.json` paired
with `<name>.response.json`:

| Fixture | Harness | Pack | Locks |
| --- | --- | --- | --- |
| `claude-code-allow` | claude-code | shipped | plain allow |
| `claude-code-warning` | claude-code | shared `warning-verdict` pack | allow + `additionalContext` |
| `claude-code-rewrite` | claude-code | fixture `command-rewrite-pack` | `updatedInput` rewrite |
| `claude-code-edit-rewrite-preserved-fields` | claude-code | fixture `edit-rewrite-pack` | rewrite of an Edit preserving unmodeled fields |
| `claude-code-deny-write` | claude-code | shipped | deny of a Write (camelCase aliases inbound) |
| `claude-code-unsupported-tool` | claude-code | shipped | MCP tool → plain allow |
| `codex-cli-deny-patch` | codex-cli | shipped | `apply_patch` deny |
| `codex-cli-deny-command` | codex-cli | shipped | command deny |
| `codex-cli-malformed-input.request.txt` | codex-cli | shipped | truncated JSON → exit 0, plain allow, stderr diagnostic |

The rewrite goldens use dedicated fixture packs so their reasons stay
deterministic; the allow/deny/warning goldens deliberately run against the
**shipped** packs and the shared `tests/fixtures/warning-verdict/` pack to
prove adapters evaluate the same policy as everything else. If a deliberate
contract change moves a golden, regenerate the fixture and say so in the
commit message.

## 11. Adding a harness

1. Add the `HarnessId` variant and its slug (`src/adapter.rs`).
2. Implement `HarnessAdapter` — `harness()`, `capabilities()`, and nothing
   else unless the wire needs it; request building and envelope rendering
   have defaults on purpose.
3. Wire any new payload field spellings into the engine's alias handling if
   they are aliases of modeled fields; if the tool is genuinely new, extend
   the engine's classification, not the adapter.
4. Add golden request/response fixtures and a row to §10's table; the
   process-boundary test is the adapter's acceptance test.
5. Record the official protocol source and its version (or its absence) in
   §6, with the date checked.

An adapter must never: carry its own rules or severities, decide its own
fail-open posture, log or record payload data, or emit anything on stdout
except the one response object.
