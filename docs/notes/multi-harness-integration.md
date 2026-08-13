# Multi-harness integration: both layers, both agents

Resolves the plan's "Integration shape" open question. Decision: run the
PATH-wrapper binary *and* native PreToolUse hooks simultaneously, for both
Claude Code and Codex CLI, as two independent defense layers rather than
choosing one.

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

## A gap neither layer covers

**OpenAI's cloud-hosted Codex** (ChatGPT web / async "Codex cloud tasks")
runs in an OpenAI-managed container, not on this host — a host-level PATH
wrapper has no reach there, and it's unconfirmed whether cloud tasks honor
`hooks.json` at all. Only the local `codex` CLI is covered by either layer.
Worth stating explicitly rather than silently assuming full coverage:
anything routed through cloud-hosted Codex tasks is currently unguarded by
this project.

## How to apply

The engine (see `docs/plan/plan.md` Architecture) needs two thin
front-ends sharing the same rule-pack core: a PreToolUse hook adapter
(works for both Claude Code and Codex CLI, since both speak a compatible
enough deny/`updatedInput` JSON shape) and a PATH-wrapper binary. Build
both. Don't let either one's absence or breakage silently mean zero
coverage.

## Sources

- <https://developers.openai.com/codex/hooks>
- <https://developers.openai.com/codex/concepts/sandboxing>
- <https://developers.openai.com/codex/agent-approvals-security>
- `github.com/openai/codex` issues #14882, #14754, #18491, #19385 (hook
  feature timeline)
- <https://code.claude.com/docs/en/hooks.md#pretooluse> (Claude Code side,
  cross-referenced from `redirect-not-just-block.md`)
