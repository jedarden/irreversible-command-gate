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

## Structure

- `docs/notes/` — features, constraints, design decisions, including the
  existing-infrastructure gap analysis and the runtime-config-vs-hardcoded
  exploration
- `docs/research/` — external reference material and prior art
  (`destructive_command_guard`, `agent-guard`, `vault-mcp-server`, etc.)
- `docs/plan/plan.md` — complete application plan (currently a stub —
  scope is still being explored)
