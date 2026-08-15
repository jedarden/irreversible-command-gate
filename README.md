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
- **The PATH-wrapper front-end specifically can't see anything that
  doesn't go through `$PATH` resolution** — an absolute-path invocation
  never reaches it. The native-hook front-end doesn't have this gap: it
  basename-matches, so it still catches an absolute-path command the same
  as a bare one. Only a command that bypasses *both* front-ends entirely
  (neither harness's hook fires, and the binary was never PATH-resolved)
  goes fully unguarded.

## Structure

- `docs/notes/` — features, constraints, design decisions, including the
  existing-infrastructure gap analysis, runtime-config-vs-hardcoded
  exploration, and the release-bound per-repository override contract
- `docs/research/` — external reference material and prior art
  (`destructive_command_guard`, `agent-guard`, `vault-mcp-server`, etc.)
- `docs/plan/plan.md` — complete application plan

## Fixed deny-regression suite

Generate one validated deny case for every enabled `guarded_pattern` in a JSON
rule pack. Disabled patterns are intentionally omitted from the fixed suite;
their `enabled: true` → `false` transition is a release-integrity regression
that still requires the normal Layer 1/2 release review. The command derives a
concrete command from command-regex rules, or uses
an optional `example_command` on a guarded-pattern entry:

```bash
icg regression-suite path/to/rule-pack.json --output regression-suite.json
```

The generated JSON records each pack ID, guarded-pattern ID, command, and
expected `deny` verdict. Generation fails if a case is missing, is not a deny
rule, is shadowed by a safe rule, or no longer matches its intended pattern.
