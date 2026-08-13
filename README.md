# irreversible-command-gate

A guard for AI coding/automation agents that intercepts commands before they
execute and blocks the ones that cause irreversible or hard-to-reverse
damage (destroying secrets, deleting cluster state, force-pushing, purging
data), while letting normal read/write operations through unimpeded.

This exists because the enforcement infrastructure already running in this
environment (`~/.claude/hooks/org-rule-guard.py`) only covers a handful of
org-specific rules — GitHub Actions files, `kind: Job`/`CronJob`, `:latest`
image tags, committed credential values, and mutating `kubectl` verbs. It
says nothing about HashiCorp Vault/OpenBao destructive operations, `git
push --force`, `bf`/beads corruption risks, or the many other irreversible
actions an agent with real credentials could take. See
`docs/notes/existing-enforcement-infrastructure.md` for the full coverage
gap analysis this project starts from.

## What this does not do

Stated plainly, because it's easy to assume more than is actually true:

- **This is a backstop for an honest, fallible agent, not a defense against
  a genuinely adversarial or compromised one.** An agent that deliberately
  wants to defeat this guard can — the deploy-location work that would
  make the rule source tamper-resistant against the very agent it guards
  is still in progress (`docs/plan/plan.md` Phase 0). See
  `docs/notes/runtime-config-vs-hardcoded.md`.
- **It does not reach OpenAI's cloud-hosted Codex** (ChatGPT web / async
  "Codex cloud tasks"). Only the local `codex` and Claude Code CLIs are
  covered. See `docs/notes/multi-harness-integration.md`.
- **It does not defend against prompt injection or a malicious/compromised
  repository trying to trick an otherwise-honest agent.** That's a
  different threat class, explicitly out of scope (see the ideas ledger's
  second run for why that idea was killed rather than adopted).
- **A command that never goes through `$PATH` resolution or a guarded
  harness's own hook system bypasses it entirely** — an absolute-path
  invocation, for instance, may only be caught by whichever of the two
  front-ends (PATH-wrapper or native hook) happens to still see it.

## Structure

- `docs/notes/` — features, constraints, design decisions, including the
  existing-infrastructure gap analysis and the runtime-config-vs-hardcoded
  exploration
- `docs/research/` — external reference material and prior art
  (`destructive_command_guard`, `agent-guard`, `vault-mcp-server`, etc.)
- `docs/plan/plan.md` — complete application plan
