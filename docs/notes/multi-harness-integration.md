# Multi-harness integration: both layers, both agents

Resolves the plan's "Integration shape" open question. Decision: run the
PATH-wrapper binary *and* native PreToolUse hooks simultaneously, for both
Claude Code and Codex CLI, as two independent defense layers rather than
choosing one. Cursor joined later as a third hook harness (below) under
the same two-layer shape.

## Why both, not one

The two layers have different, non-overlapping blind spots:

- A **PATH-wrapper** catches the literal subprocess exec regardless of
  which harness invoked it, and doesn't depend on any agent tool's own
  extension API existing or staying stable. It's blind to anything that
  doesn't shell out through a `$PATH`-resolved binary — structured
  tool/MCP calls, or a harness that calls a library directly instead of
  exec'ing a CLI.
- A **native hook** sees the agent's actual intended tool call before the
  harness even attempts execution, including non-Bash structured calls
  (MCP tools, `apply_patch`) a wrapper would never see — but only for
  whichever harness ships that hook type, and only as reliably as that
  harness's hook implementation actually is.

Running both means a gap in one is covered by the other. This is the same
defense-in-depth logic already used elsewhere in this environment (e.g.
`org-rule-guard.py`'s fail-open design assumes it's a backstop, not the
sole line of defense — the agent's own compliance is the other layer).

## Codex CLI: confirmed to support both shapes

Researched directly against OpenAI's own docs
(`developers.openai.com/codex/hooks`, `/concepts/sandboxing`,
`/agent-approvals-security`) and source-level analysis of `openai/codex`:

- **Codex CLI has a native `PreToolUse` hook**, structurally close to
  Claude Code's: JSON on stdin, JSON on stdout with
  `hookSpecificOutput.permissionDecision: "deny"` (+ reason) or `"allow"`
  with `updatedInput` to rewrite the command — for Bash and `apply_patch`
  specifically. Config in `~/.codex/hooks.json` or a repo's
  `.codex/hooks.json`, gated by project trust.
- **This is young and still moving.** Shipped experimental behind a
  feature flag in ~March 2026 (v0.114), `PreToolUse`/`PostToolUse` landed
  weeks later (v0.117), scope expanded from Bash-only to `apply_patch`/MCP
  afterward, reached "stable" around v0.124 (~April 2026) — roughly 4-5
  months of active churn as of this writing. `additionalContext` is in the
  schema but not yet honored. Treat the Codex-hook front-end as needing
  more maintenance attention than the Claude Code one, which has been
  stable longer.
- **Command execution is `$PATH`-resolved** (`tokio::process::Command`,
  `execvp`-style on Unix) — confirmed via source-level analysis, not
  assumed. The sandbox (Seatbelt on macOS, bubblewrap+Landlock+seccomp on
  Linux) restricts filesystem writes and network, not binary discovery, so
  a PATH-earlier wrapper transparently intercepts Codex-issued commands
  the same way it does Claude Code's.
- **Codex already has its own approval/sandbox system**
  (`sandbox_mode`: read-only/workspace-write/danger-full-access,
  `approval_policy`: on-request/never/untrusted/auto_review) plus a
  built-in `execpolicy` covering a few destructive-git-op categories. This
  project sits alongside that, doesn't duplicate it.

## Cursor: native hooks, cloud-agent limits, and the third-party import

Cursor ships two ICG adapters — the generic `preToolUse` event
(`icg hook --harness cursor`) and the dedicated `beforeShellExecution`
event (`--event before-shell-execution`; its response schema has no
`updated_input`, so a rewrite degrades to a deny) — wired idempotently by
`icg install-cursor-hooks`. The wire-level details live in the contract's
Cursor section; what matters at the integration level is the support
boundaries, all from Cursor's own documentation (retrieved 2026-09-19):

- **Cloud agents are covered, but narrowly.** Unlike OpenAI's cloud-hosted
  Codex (next section), Cursor cloud agents *do* run hooks from the
  repository — but only **command-based hooks from project-level
  `.cursor/hooks.json`** (plus team/enterprise distribution on Enterprise
  plans). Never the user-level file (cloud VMs see no home directory),
  never prompt-based hooks, and none of `sessionStart`, `sessionEnd`,
  `beforeMCPExecution`/`afterMCPExecution`, the Tab hooks
  (`beforeTabFileRead`/`afterTabFileEdit`), or `workspaceOpen`.
- **The early read-only phase is the gap.** Cursor's own words: cloud
  agents "sometimes begin in a read-only environment for early exploratory
  turns. Hooks do not run during those turns." Those turns are unguarded
  by the hook layer — and the PATH wrapper has no reach inside the cloud
  VM either. This is Cursor's partial analog of the Codex cloud gap:
  coverage exists for the writable portion of the session, zero during
  read-only exploration.
- **The third-party import is a second, independent wiring path.** With
  Cursor Settings → Agents → Third-Party Imports enabled (on by default),
  Cursor loads Claude Code hooks from `.claude/settings.local.json` →
  `.claude/settings.json` → `~/.claude/settings.json`, merged *below* the
  four Cursor layers (Enterprise → Team → Project → User), translating
  events (`PreToolUse` → `preToolUse`, `PostToolUse` → `postToolUse`,
  `Stop` → `stop`, …; `Notification`/`PermissionRequest` unsupported) and
  tool names (`Bash` → `Shell`, `Edit` → `Write`; `Glob` unsupported), and
  accepting both response envelopes (`permissionDecision`/`permission`,
  `permissionDecisionReason`/`user_message`, `updatedInput`/
  `updated_input`). The unchanged undecorated ICG hook therefore covers
  local Cursor sessions too — proven end-to-end by
  [`scripts/cursor-dispatch-e2e`](../../scripts/cursor-dispatch-e2e)
  (scenario S5). It stays the compatibility path, not the primary one: it
  depends on a user-visible setting, imports cannot configure `loop_limit`
  (default 5 for native Cursor hooks, `null` — unlimited — for
  Claude-imported ones), and cloud agents read none of the Claude files —
  only project-level `.cursor/hooks.json`.
- **Hooks add a gate; they do not take Cursor's own away.** Cursor's
  native approval and sandbox controls run regardless of hooks (a
  `beforeShellExecution` input reports whether the command will be
  sandboxed; shell/MCP durations exclude approval wait time), and a hook
  `allow` bypasses none of them. The mirror-image boundary: the only
  *gating* events are the permission hooks. `afterFileEdit` fires **after**
  the edit is already applied — a formatter/audit surface with no
  `beforeEditFile` counterpart — so pre-write/edit coverage under Cursor
  is `preToolUse` matching `Write`, and is never claimed on the strength
  of `afterFileEdit`.

## A gap neither layer covers

**OpenAI's cloud-hosted Codex** (ChatGPT web / async "Codex cloud tasks")
runs in an OpenAI-managed container, not on this host — a host-level PATH
wrapper has no reach there, and it's unconfirmed whether cloud tasks honor
`hooks.json` at all. Only the local `codex` CLI is covered by either layer.
Worth stating explicitly rather than silently assuming full coverage:
anything routed through cloud-hosted Codex tasks is currently unguarded by
this project. Cursor cloud agents have a partial analog — repo-level hooks
do run there, but not during the early read-only exploratory turns (see
the Cursor section above).

## How to apply

The engine (see `docs/plan/plan.md` Architecture) needs two thin
front-ends sharing the same rule-pack core: a PreToolUse hook adapter
(works for both Claude Code and Codex CLI, since both speak a compatible
enough deny/`updatedInput` JSON shape) and a PATH-wrapper binary. Build
both. Don't let either one's absence or breakage silently mean zero
coverage.

This is now the adapter contract: `src/adapter.rs` plus
[`harness-adapter-contract.md`](harness-adapter-contract.md) (version 1)
define the canonical request/result both shipped hook front-ends share, the
`--harness` declaration, and the per-harness mappings — including the
shipped Claude Code, Codex CLI, Gemini CLI, Cursor (the last
with its dedicated `beforeShellExecution` event), and OpenCode adapters
(the last relayed by an in-process plugin whose process boundary wraps
`icg hook --harness opencode`), and the official sources each
mapping was taken from. The wrapper remains a separate, payload-less front end
(identity `wrapper` in the contract's closed harness set).

## Sources

- <https://developers.openai.com/codex/hooks>
- <https://developers.openai.com/codex/concepts/sandboxing>
- <https://developers.openai.com/codex/agent-approvals-security>
- `github.com/openai/codex` issues #14882, #14754, #18491, #19385 (hook
  feature timeline)
- <https://code.claude.com/docs/en/hooks.md#pretooluse> (Claude Code side,
  cross-referenced from `redirect-not-just-block.md`)
- <https://cursor.com/docs/agent/hooks> (Cursor event payloads, output
  schemas, exit codes, `failClosed`, `timeout`, cloud-agent support
  tables; retrieved 2026-09-19)
- <https://cursor.com/docs/reference/third-party-hooks> (Claude Code
  import: load locations, merge priority, event/tool-name mapping,
  response-format compatibility, `loop_limit` defaults; retrieved
  2026-09-19)
