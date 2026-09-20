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
| `OpenCode` | `opencode` | `OpenCodeAdapter` (shipped; §6.3) | in-process plugin API; the plugin relays `tool.execute.before` over a subprocess wire |
| `GeminiCli` | `gemini-cli` | `GeminiCliAdapter` (shipped) | `BeforeTool` command hook, JSON on stdin/stdout |
| `Cursor` | `cursor` | `CursorAdapter` (shipped), `CursorShellExecutionAdapter` (§6.5) | agent hooks, JSON on stdin/stdout |
| `Wrapper` | `wrapper` | none (no payload) | shadowed argv via `execvp` |

The `--harness` flag spelling for OpenCode is its slug, `opencode` (clap's
derived kebab-case `open-code` is accepted as an alias); every other
harness's flag spelling already equals its slug.

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
| OpenCode | `bash` | `args.command` |

The payload wire is snake_case (`tool_name`/`tool_input`); the engine also
accepts the camelCase aliases (`toolName`/`toolInput`) of ICG's early
fixtures, on both shipped wires. A rewrite replaces the `command` field.

### 3.2 Single-file writes and edits

| Harness | Tool | Fields |
| --- | --- | --- |
| Claude Code | `Write` | `file_path` (alias `filePath`), `content`, optional `encoding`, `mime_type` |
| Claude Code | `Edit` | `file_path`, `old_string`, `new_string` (aliases `filePath`, `oldString`, `newString`) |
| Codex CLI | same shapes | same fields |
| OpenCode | `write` | `filePath`, `content` |
| OpenCode | `edit` | `filePath`, `oldString`, `newString`, optional `replaceAll` |

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
harnesses. Gemini CLI, Cursor, and OpenCode override `render` with their own
native envelopes (§6.4, §6.5, §6.3); the degradation rules below are the
shared contract those renderers implement.

What a harness's wire can express is declared per adapter as
`Capabilities`, and degradation is **contract behavior, not adapter
discretion**:

| Verdict | Full-capability render | Degraded render |
| --- | --- | --- |
| `Allow` | `permissionDecision: "allow"` | `supports_allow_decision: false` → **field omitted**, leaving no opinion — an allow is never spelled as a decision the harness rejects |
| `Warn` | allow + `additionalContext: <attributed reason>` | `supports_additional_context: false` → **bare allow**, context dropped — never turned into a block |
| `Rewrite` | allow + `updatedInput` (complete replacement object: original input with the rewrite value substituted under the request's rewrite key) + `additionalContext` | `supports_updated_input: false` → **deny** carrying the rewrite's attributed reason — the matched input must not run unreplaced |
| `Deny` | `permissionDecision: "deny"` + `permissionDecisionReason` (no `updatedInput`, ever) | same |

A deny is never degraded. Every capability above narrows what ICG may
*grant*; the veto direction is always available, which is the direction that
carries the safety.

Shipped capabilities:

| Adapter | `supports_updated_input` | `supports_additional_context` | `honors_additional_context` | `supports_system_message` | `supports_allow_decision` |
| --- | --- | --- | --- | --- | --- |
| Claude Code | yes | yes | yes | yes | yes |
| Codex CLI | **no** | yes | **no** | yes | **no** |
| OpenCode | yes | **no** | **no** | **no** | **no** (no decision field exists on the wire) |

`honors_additional_context` is informational: the Codex CLI parses
`additionalContext` but does not yet act on it (see
[`multi-harness-integration.md`](multi-harness-integration.md) for the
timeline). The adapter still sends the field, so the day Codex honors it the
warning text is already on the wire.

`supports_updated_input` and `supports_allow_decision` are **not**
informational, and both are false for Codex: see §6.2 for the runtime
narrowing that forces them.

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
- **The schema is wider than the runtime.** Codex's generated
  `pre-tool-use.command.output` schema accepts `permissionDecision` of
  `allow|deny|ask`, but the runtime honors **`deny` alone**. Verified against
  the shipped Codex CLI 0.154.0 binary, which carries these rejections:

  ```
  PreToolUse hook returned unsupported permissionDecision:allow
  PreToolUse hook returned unsupported permissionDecision:ask
  PreToolUse hook returned updatedInput without permissionDecision:allow
  PreToolUse hook returned permissionDecision:deny without a non-empty permissionDecisionReason
  PreToolUse hook returned unsupported decision:approve
  PreToolUse hook returned unsupported continue:false / stopReason / suppressOutput
  ```

  So a hook can veto Codex but cannot grant to it, and `updatedInput` is
  unreachable there — its only documented precondition is an accepted
  `permissionDecision: "allow"`, which Codex refuses. Hence
  `supports_updated_input: false` and `supports_allow_decision: false`, which
  route a `Rewrite` into the deny degradation above rather than emitting a
  rewrite Codex silently discards. **Validating against the published schema
  is not sufficient for this harness**; the runtime is stricter, and the gap
  is silent apart from a per-call hook error.
- **Registration must declare the harness.** `default_adapter()` is Claude
  Code, so a bare `icg hook` in `~/.codex/hooks.json` is served Claude Code's
  wire format regardless of these capabilities. The command must be
  `icg hook --harness codex-cli`.
- **Config:** `~/.codex/hooks.json` or repo `.codex/hooks.json`, gated by
  project trust. Not to be confused with cloud-hosted Codex tasks, which are
  out of reach for any host-level guard.
- **Sources:** <https://developers.openai.com/codex/hooks>; generated wire
  schema `codex-rs/hooks/schema/generated/pre-tool-use.command.input.schema.json`
  in `github.com/openai/codex`. Both re-checked 2026-09-18.

### 6.3 OpenCode (implemented; verified against the installed
1.18.29 — addendum §6.3.1)

- **Protocol:** an **in-process JavaScript/TypeScript plugin API**, not a
  subprocess wire. Plugins live in `.opencode/plugins/` (project) or
  `~/.config/opencode/plugins/` (global), or as npm packages via the
  `plugin` config array, typed by the `@opencode-ai/plugin` SDK. The
  relevant hook is `tool.execute.before`: input `{ tool, sessionID, callID }`,
  output `{ args }`, with `args` mutated in place. There is no stdout
  envelope on OpenCode's side of the hook.
- **Mapping:** `tool` → canonical `tool_name`; `args` → canonical
  `tool_input` (field spellings per §3). A Deny is delivered by
  **throwing an error from the hook** (OpenCode aborts the tool call); a
  Rewrite by mutating `args` in place before returning. `tool.execute.before`
  has no advisory-context channel, so OpenCode's capabilities declare
  `supports_additional_context: false` and a Warn degrades to a bare allow
  (§5). The plugin shells out to `icg hook --harness opencode` — the process
  boundary moves inside the plugin, but the canonical request/result and the
  engine are exactly as specified here.
- **Versioning:** the plugin API has no wire-protocol version field.
  This bullet originally called the docs page normative — corrected by the
  §6.3.1 addendum: the docs track *current upstream* (today 1.18.31), not
  the installed pin; the normative artifact for the V1 target is the
  installed binary and its bundled SDK types, both sha256-pinned.
- **Sources:** pinned evidence in
  [`opencode-1.18.29-plugin-surface.md`](../research/opencode-1.18.29-plugin-surface.md)
  (hook inventory §2–§7, registration/failure §10, post-1.18.29 drift §12)
  and
  [`opencode-1.18.29-deny-rewrite-advisory.md`](../research/opencode-1.18.29-deny-rewrite-advisory.md)
  (deny/rewrite/advisory semantics), both against the installed binary;
  upstream docs at <https://opencode.ai/docs/plugins>, retrieved
  2026-09-18 and re-checked 2026-09-20.
- **Implementation (ICG):** `OpenCodeAdapter`, served by
  `icg hook --harness opencode` (the flag spelling is the telemetry slug;
  clap's derived `open-code` is an alias). The plugin serializes the hook's
  payload — `tool`, `sessionID`, `callID`, and the mutable `args` object —
  to the process's stdin, which the engine's OpenCode admission path
  (`read_opencode_payload_from_stdin`, the same fail-open stdin boundary as
  every other reader) shapes into the PreToolUse input the shared front end
  already evaluates. OpenCode's camelCase `args` spellings (`filePath`,
  `oldString`, `newString`) are the aliases the tool-input deserializer
  already reads; the tool names `bash`/`write`/`edit` are wired in as
  spellings of the same three modeled actions, and the canonical request's
  `tool_name` keeps OpenCode's own spelling. The `args` object — not the
  whole payload — is what travels as the preserved original, because a
  rewrite replacement is a replacement *for the args* and the
  `tool`/`sessionID`/`callID` envelope must never leak into it.
- **Response envelope (`render_opencode_envelope`):** OpenCode's hook has
  no envelope to parse — the plugin acts — so the one JSON object on stdout
  names exactly one action the plugin must take:
  `{"action": "allow"}` (return normally, `args` untouched — a Warn renders
  this too, the advisory text dropped per §5);
  `{"action": "rewrite", "args": {...}}` (the complete replacement args
  object: every field the harness sent with only the rewrite key
  substituted, applied by copying its properties onto `output.args`
  **in place** — reassigning `output.args` is a no-op at every 1.18.29
  call site, and OpenCode's transcript records the model's original args,
  so the plugin audits its own rewrites); and
  `{"action": "deny", "message": "ICG: <attributed reason>"}` (throw
  `new Error(message)` verbatim — the thrown message is the only
  model-visible text the gate controls here, reaching the model as
  `Tool execution failed: ICG: …`, and because the agent loop continues,
  retries arrive and are gated again). Capabilities:
  `supports_updated_input: true` (in-place mutation is execution-real),
  `supports_additional_context: false`, `supports_system_message: false`
  (practice-mode and bypass banners go to stderr), and
  `supports_allow_decision: false` as the record of there being no decision
  field anywhere on this wire.
- **Coverage boundary:** OpenCode's `apply_patch` tool carries a *list of
  patch operations* (typed add/update/delete structs), not the `***
  Begin Patch` text Codex sends, so it does not map onto the engine's patch
  classification; it fails open at the classification boundary with a
  stderr diagnostic rather than silently rendering an allow for an
  unrecognized shape. OpenCode's read-only tools (`read`, `glob`, `grep`,
  `webfetch`) and MCP namespaced keys are `Unsupported` by contract (§3.4)
  and render a quiet plain allow. The plugin-side scoping (which tools
  invoke the gate at all) and the liveness self-verification are the plugin
  deployment's concerns, on top of this adapter; the PATH-wrapper layer
  ([`multi-harness-integration.md`](multi-harness-integration.md)) remains
  the backstop for everything the plugin layer cannot see, including the
  `--pure` kill switch (§6.3.1, surface §10.2).

#### 6.3.1 Addendum — pinned against the installed 1.18.29 (2026-09-20)

§6.3 above was written from the current docs site. It has since been
verified against the binary actually installed on codinghome — `opencode`
**1.18.29**, binary sha256 `ca6c0e1f…`, plugin SDK `@opencode-ai/plugin`
1.18.29 (`index.d.ts` sha256 `f3ec1a15…`) — and corrected where the two
disagree. Every claim below traces to that version's evidence: `.d.ts`
file:line citations or byte offsets in the pinned binary, recorded in the
two research files named under **Sources** above (the *surface* and
*semantics* references in the table).

**Assumption scorecard for the original §6.3 text:**

| §6.3 assumption | Verdict | Evidence |
| --- | --- | --- |
| in-process JS/TS plugin API, no stdout envelope | **held** | surface §4.2 — `Plugin.trigger` calls hooks in-process and discards return values |
| registration via `.opencode/plugins/`, `~/.config/opencode/plugins/`, npm `plugin` array | **held, under-complete** | surface §10.1 — both `plugin`/`plugins` dir spellings plus `plugin` arrays in the global config, project configs, `.opencode/opencode.json`, env and remote channels; origin order global→project; last-declaration-wins dedupe; readdir-unsorted dir globs |
| hook `tool.execute.before`, input `{ tool, sessionID, callID }`, output `{ args }` | **held exactly** | surface §4.1 (`index.d.ts:235-241`); binary trigger-site offsets in surface §2 |
| `tool` → `tool_name`, `args` → `tool_input` (§3) | **held; per-tool spellings now pinned** | surface §4.4–4.5 — `bash {command, timeout?, workdir?}`, `edit {filePath, oldString, newString, replaceAll?}`, `write {filePath, content}`, `apply_patch` patch-op list; OpenCode's camelCase spellings are the §3.2 aliases |
| Deny delivered by throwing; OpenCode aborts the tool call | **held, strengthened** | semantics §1 — the throw lands before execution **and before the permission ask** (no prompt is raised for a call about to be denied) and does **not** end the agent loop; the model receives `Tool execution failed: <message>` as its tool result |
| Rewrite by mutating `args` in place | **held as the only working form** | semantics §2 — hook and executor share one args object at all three wrapper sites, so property mutation is what executes; **reassigning `output.args` is a no-op**; the transcript records the model's *original* args, so the adapter must audit its own rewrites |
| no advisory channel → `supports_additional_context: false`; a Warn degrades to a bare allow | **flag held; degradation statement was incomplete** | semantics §3 — no channel at tool-call time is confirmed (output is `{args}` only, returns discarded), so a non-blocking Warn is a bare allow; but **deny-with-message** is a second, model-visible degraded render at the same call site. Escalating a Warn to it is a policy choice, never adapter discretion (§5) |
| "no wire-protocol version field; the docs page is normative" | **wrong** | no version field is right, but the docs page tracks current upstream and is already a *subset* of the binary's registration matrix (surface §10.1); the pin is the sha256-pinned binary + bundled types (surface §1) |
| *(assumed upstream of §6.3, in the umbrella bead)* `shell.create.before` / `permission.evaluate` as candidate gate hooks | **wrong — neither exists** | surface §2 — zero binary matches under any spelling; `permission.ask` is declared but never fired (surface §5.2); `shell.env` exists but is env-injection, fires after permission approval, and cannot veto (surface §5.1) |

**Design constraints for the OpenCode adapter** (same evidence):

- **Deny** = `throw new Error("ICG: <one-line attributed reason>")`. The
  thrown message is the only model-visible text the gate controls; it
  reaches the model as `Tool execution failed: ICG: …`, and because the
  agent loop continues, retries arrive and are gated again.
- **Rewrite** = in-place property mutation under the request's own field
  spelling; the adapter records every rewrite itself — OpenCode's
  transcript will not show that one happened.
- **Warn** = bare allow (no channel). `tool.execute.after` output mutation
  is post-hoc audit only — execution already happened; it is never a
  fallback gate (surface §7).
- **Allow** = return without throwing and without touching `args` — there
  is no decision field to spell, so §5's allow-degradation distinction
  collapses to "did the hook throw". Capabilities when implemented:
  `supports_additional_context: false`, `supports_updated_input: yes`
  (in-place mutation is execution-real).
- **Registration** = one file in the global plugin dir
  (`~/.config/opencode/plugin/`, singular spelling honored) or a `file://`
  entry in the global config's `plugin` array; never an npm spec (install
  machinery + compatibility gate); no reliance on filename order within a
  plugin dir (surface §10.1, §10.5).
- **Liveness** = the gate must self-verify: a plugin that fails to load is
  dropped **quietly** — import-stage failures leave no log line at all —
  and `opencode debug info` lists registrations, not loads (surface
  §10.3–§10.5). `--pure`/`OPENCODE_PURE` silently disables every external
  plugin; that hole is named residual risk, backstopped by the
  PATH-wrapper layer
  ([`multi-harness-integration.md`](multi-harness-integration.md)).
- The plugin still shells out to `icg hook --harness opencode`; the
  process boundary moves inside the plugin, and the canonical
  request/result and engine are exactly as specified here.

**Decision — V1/V2 go/no-go (pinned 2026-09-20):**

> **V1 — the installed 1.18.29 — GO: the sole support target, pinned by
> binary sha256 and verified end to end. V2 — any plugin API after
> 1.18.29 — NO-GO: nothing published after 1.18.29 diverges (the latest
> SDK 1.18.31 ships a byte-identical `dist/` tree; releases 1.18.30 and
> 1.18.31 touch nothing plugin-side; the current docs describe the same
> surface — surface §12), so there is no V2 to be compatible with, and
> speculative compatibility code would weaken the V1 pin. The single
> plugin file targets the 1.18.29 surface only, with no runtime version
> probing.**

The identical-types fact means that same file is *expected* to load
unchanged on 1.18.30/1.18.31 — a types-level expectation, not a support
claim. Re-pin triggers are listed in surface §12.3 (a changed `index.d.ts`
sha256, a plugin-side release note, a changed installed `opencode`, or a
changed docs hook surface); any trigger opens a **new pinned
investigation**, never a runtime guess in the plugin.

### 6.4 Gemini CLI (implemented)

- **Protocol:** `BeforeTool` **command hook**. Config: `hooks` object in
  `~/.gemini/settings.json` or project `.gemini/settings.json` — event
  `BeforeTool` → hook definitions (`matcher` regex over tool names,
  `sequential`, `hooks[]` with `type: "command"`, `command`, optional
  `timeout` in milliseconds, default 60000).
- **Input (stdin JSON):** `session_id`, `transcript_path`, `cwd`,
  `hook_event_name`, `timestamp`, plus per-event `tool_name`,
  `tool_input` (object), optional `mcp_context`, `original_request_name`.
  The Claude-compatible `tool_name`/`tool_input` pair is the payload the
  `GeminiCliAdapter` translates.
- **Output (stdout JSON):** common fields `decision` (`"deny"`, alias
  `"block"`), `reason` (**required when denied** — sent to the agent as the
  tool error), `systemMessage`, `suppressOutput`, `continue`, `stopReason`;
  event-specific `hookSpecificOutput.tool_input` — an object that
  **merges with and overrides** the model's arguments, which is the rewrite
  channel (a merge-override rather than Claude Code's whole-object
  `updatedInput`; ICG renders the complete replacement and lets Gemini CLI
  merge it).
- **Exit codes:** `0` = success, stdout parsed as JSON (**preferred for all
  logic**; silence on stdout is mandatory — exactly one JSON object, which is
  already this contract's rendering rule); `2` = system block with **stderr**
  as the rejection reason, the turn continues; **any other exit = non-fatal
  warning, the CLI continues** — a native fail-open posture that matches
  ICG's own.
- **Implementation (ICG):** `GeminiCliAdapter`, served by
  `icg hook --harness gemini-cli` (registration must declare the harness —
  `default_adapter()` is Claude Code, §2). The covered tools are Gemini's
  spellings of the three modeled actions, classified through the engine's
  own alias handling, not alongside it: `run_shell_command` (the Bash
  `command` shape), `write_file` (the Write `file_path`/`content` shape),
  and `replace` (the Edit `old_string`/`new_string` shape; `docs/tools/` in
  the gemini-cli repository is the field reference). Rendering
  (`render_gemini_envelope`): a Deny is the top-level
  `{"decision": "deny", "reason": ...}` pair — `reason` is required when
  denied and reaches the agent as the tool error, which stops the tool
  while the turn continues; a Rewrite rides
  `hookSpecificOutput.tool_input` as the complete replacement input, every
  field the harness sent preserved and only the rewrite key substituted,
  because Gemini merge-overrides field-by-field and a partial object would
  leave the matched field intact underneath; a Warn degrades to a bare
  allow carrying the attributed reason on `systemMessage` (§5 — BeforeTool
  has no advisory-context channel); an Allow renders the permissive empty
  object. A top-level `decision: "allow"` is **never emitted** — its
  `BeforeTool` impact is unspecified and could stand in for Gemini's own
  confirmation flow — encoded as `supports_allow_decision: false`.
  Capabilities: `supports_updated_input: true`,
  `supports_additional_context: false`, `honors_additional_context: false`,
  `supports_system_message: true`. Malformed stdin keeps the shared
  fail-open boundary (§8): exit 0, the permissive object on stdout,
  diagnostic on stderr only. Locked by the `gemini-cli-*` golden fixtures
  (`tests/fixtures/adapter/`), the process-boundary suite in
  `tests/gemini_hook_tests.rs` (schema shape per verdict, context-field
  inertness, and fake-target canaries executing Gemini's documented
  dispatch semantics for the denied, failed-adapter, and dead-adapter
  cases), and the unit tests in `src/adapter.rs`.
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
  here with an explicit protocol version. Agent hooks apply to Cmd+K and
  Agent Chat sessions; Tab completions and workspace lifecycle have their
  own separate hook surfaces. Relevant events: `preToolUse`
  (input: `tool_name`, `tool_input`, `tool_use_id`, `cwd`, plus the common
  base fields `model`, `model_id`, `model_params`, `agent_message`,
  `hook_event_name`, `cursor_version`, `workspace_roots`, `user_email`,
  `transcript_path`), `beforeShellExecution` (`command`, `cwd`, `sandbox`
  — **no `tool_name`**), `beforeReadFile`, `afterFileEdit`,
  `beforeMCPExecution`. When several layers match the same event, **all
  matching hooks from every source run** and Cursor merges the responses:
  any `deny` wins over `ask`, and `ask` wins over `allow`, regardless of
  source; `user_message`/`agent_message` concatenate; every other field is
  last-response-wins in the priority walk.
- **Timeout:** per-script `timeout` in **seconds** (platform default when
  unset). A timeout is one of Cursor's hook-failure modes — crash,
  timeout, non-zero exit, no output — which fail open unless the hook sets
  `failClosed: true` (below).
- **Placement:** Cursor **cloud agents read project-level hooks only —
  `~/.cursor/hooks.json` is not available to them** (see *Cloud agents*
  below) — so a cloud-agent session is covered by `.cursor/hooks.json` in
  the repository, and the user-level file covers local IDE sessions alone.
  Wire the layer the session actually runs under. `icg install-cursor-hooks`
  manages this wiring idempotently (project file by default; `--user` and
  `--file`
  select the other layers): ICG-owned entries are recognized by their
  command line (`<icg> hook --harness cursor …`) and replaced rather than
  duplicated, every unrelated hook, matcher and key is preserved
  verbatim, a missing file is created, and a file that does not parse —
  or carries a `version` other than 1 — fails with a clear error and is
  left unchanged. The prior file is backed up once as
  `<target>.icg-backup`.
- **Cloud agents:** coverage is real but bounded. Cloud agents run
  **command-based hooks only** — prompt-based hooks do not execute in the
  cloud environment — loading them from **project-level `.cursor/hooks.json`
  at the repo root**, plus team and enterprise-managed hooks on Enterprise
  plans; the user-level file is never read (cloud VMs have no access to the
  local home directory). Hooks that do not run in cloud agents at all:
  `sessionStart` (hooks don't load in the read-only start, so a cloud
  sessionStart would fire too late to mean session start),
  `sessionEnd` (no editor-lifetime session boundary),
  `beforeMCPExecution`/`afterMCPExecution` (deferred, timing unclear),
  `beforeTabFileRead`/`afterTabFileEdit` (Tab is an IDE feature),
  and `workspaceOpen` (IDE lifecycle). And cloud agents **sometimes begin
  in a read-only environment for early exploratory turns where hooks do
  not run at all** — they start only once the agent has a writable
  environment. That early read-only phase is unguarded by this adapter,
  and the PATH-wrapper layer has no reach inside the cloud VM either
  ([`multi-harness-integration.md`](multi-harness-integration.md)).
- **Native approval controls:** hooks run **alongside** Cursor's own
  approval and sandbox controls; neither layer replaces the other. The
  `beforeShellExecution` input reports whether the command will run
  sandboxed (`sandbox`), and shell/MCP execution durations explicitly
  exclude approval wait time — the native approval step still happens no
  matter what any hook returns. A hook's `allow` bypasses none of Cursor's
  own gates, and nothing in the documented response schema could suppress
  them.
- **Pre-write coverage boundary:** the only events that gate *before*
  execution are the permission hooks — for agent sessions `preToolUse`,
  `beforeShellExecution`, `beforeMCPExecution`, `beforeReadFile` (plus
  `subagentStart` for subagent creation and `beforeTabFileRead` on the
  separate Tab surface). Cursor has **no `beforeEditFile` event**:
  `afterFileEdit` fires after the edit is already applied and is a
  formatter/audit surface. ICG
  claims no pre-write protection through it — under Cursor, edit coverage
  is `preToolUse` matching the `Write` tool (§3.2) and nothing else. A
  Cursor deployment must never be reported as pre-write-capable on the
  strength of `afterFileEdit`.
- **Third-party Claude import:** Cursor can also load Claude Code hooks
  directly, gated by Cursor Settings → Agents → Third-Party Imports
  ("Include Third-Party Plugins, Skills, and Other Configs", **on by
  default**). Claude hooks load from `.claude/settings.local.json`
  (project-local) → `.claude/settings.json` (project) →
  `~/.claude/settings.json` (user), and merge **below** all four Cursor
  layers — Enterprise → Team → Project → User → Claude project-local →
  Claude project → Claude user — with all matching hooks from every source
  run and higher-priority sources winning conflicts. Events map
  `PreToolUse` → `preToolUse`, `PostToolUse` → `postToolUse`,
  `UserPromptSubmit` → `beforeSubmitPrompt`, `Stop` → `stop`,
  `SubagentStop` → `subagentStop`, `SessionStart` → `sessionStart`,
  `SessionEnd` → `sessionEnd`, `PreCompact` → `preCompact`;
  `Notification` and `PermissionRequest` are not supported. Tool names
  translate `Bash` → `Shell` and `Edit` → `Write`, with `Read`, `Write`,
  `Grep`, `Task`, `WebFetch`, `WebSearch` passing through unchanged;
  **`Glob` is unsupported**. Cursor accepts both response envelopes on
  these events — the nested Claude `hookSpecificOutput`
  (`permissionDecision` → `permission`, `permissionDecisionReason` →
  `user_message`, `updatedInput` → `updated_input`) and Cursor's native
  flat one — which is why the undecorated ICG hook works under the import
  unchanged (proven end-to-end by the script below). Exit codes keep
  their meaning across both tools. The import has native-format-only
  gaps: `subagentStart`, team/enterprise dashboard distribution, and
  `loop_limit` configuration — `loop_limit` (the stop/`subagentStop`
  follow-up cap) defaults to **5 for native Cursor hooks** and to
  **`null` — no limit — for Claude-imported hooks**. An ICG hook wired
  only in `.claude/settings.json` therefore also covers local Cursor
  sessions, but the native `.cursor/hooks.json` wiring stays the primary
  path: it does not depend on a user-visible setting, and cloud agents
  read only the project-level Cursor file.
- **Adapters (shipped):** `CursorAdapter` serves the generic `preToolUse`
  event — `icg hook --harness cursor` — and `CursorShellExecutionAdapter`
  serves the dedicated shell event — `icg hook --harness cursor --event
  before-shell-execution` (the event is refused under any other harness).
  The engine classifies Cursor's spellings for every harness: its shell
  tool is named `Shell` (Bash payload shape), and its edits arrive under
  the `Write` tool name shaped as an `old_string`/`new_string` pair with
  no `content` — classified as an edit so the introduced content is
  evaluated (§3.2).
- **Output (stdout JSON):** `permission`: `"allow"` / `"deny"` / `"ask"`,
  `user_message`, `agent_message`,
  `updated_input` (preToolUse input substitution — the rewrite channel).
  `"ask"` is **accepted by the `preToolUse` schema but not enforced there
  today** (on `subagentStart` it is treated as `deny`); ICG emits only
  `allow`/`deny`, and the engine has no `ask` verdict to translate (§4).
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
  exits fail open unless the hook sets `failClosed: true` — which promotes
  crashes, timeouts, and empty output to blocks as well. A Cursor adapter
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
  milliseconds per hook definition (default 60000), Cursor per-script
  `timeout` in **seconds** (platform default when unset), Codex per its
  hooks config. On a timed-out hook every listed harness proceeds without a
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
   deliberately truncated fixture. The per-harness admission paths —
   Cursor's `beforeShellExecution` reader and OpenCode's
   `tool.execute.before` reader, both selected by the declared
   harness/event rather than by payload sniffing — wrap their reads in the
   same boundary with the same posture, rendered in each harness's own
   envelope.
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
| `malformed-input.request.txt` | codex-cli, gemini-cli, opencode | shipped | truncated JSON → exit 0, plain allow, stderr diagnostic |
| `gemini-cli-allow` | gemini-cli | shipped | permissive empty object — no `decision` field |
| `gemini-cli-deny-shell` | gemini-cli | shipped | top-level `decision`/`reason` deny of `run_shell_command` |
| `gemini-cli-rewrite-shell` | gemini-cli | fixture `command-rewrite-pack` | `hookSpecificOutput.tool_input` rewrite preserving unmodeled fields |
| `gemini-cli-warning-shell` | gemini-cli | shared `warning-verdict` pack | warn degraded to bare allow + `systemMessage` |
| `gemini-cli-deny-write-file` | gemini-cli | shipped | deny of `write_file` (snake_case aliases inbound) |
| `gemini-cli-replace-rewrite-preserved-fields` | gemini-cli | fixture `edit-rewrite-pack` | rewrite of a `replace` preserving unmodeled fields |
| `opencode-allow-shell` | opencode | shipped | allow action over `bash`/`command` |
| `opencode-deny-shell` | opencode | shipped | deny action; the message is what the plugin throws |
| `opencode-rewrite-shell` | opencode | fixture `command-rewrite-pack` | `args.command` rewrite preserving `timeout`/`workdir` |
| `opencode-warning-shell` | opencode | shared `warning-verdict` pack | warn degraded to the bare allow action |
| `opencode-deny-write` | opencode | shipped | deny of `write` (`filePath`/`content` inbound) |
| `opencode-rewrite-edit-preserved-fields` | opencode | fixture `edit-rewrite-pack` | rewrite of `edit` under the camelCase `newString` key, preserving `replaceAll` |
| `opencode-allow-unsupported-tool` | opencode | shipped | unmodeled tool → quiet allow action |

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
