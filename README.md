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
- **Both hook and PATH-wrapper front-ends are fully implemented.** The
  binary automatically detects when invoked under a shadowed name (via symlinks
  like `vault` → `icg` in PATH) and runs command-mode checks before exec'ing
  the real binary. Hook mode (`icg hook`) handles both command-mode and
  content-mode (Write/Edit) checks via the PreToolUse JSON protocol.
  The wrapper does not cover absolute-path invocations or direct library calls.

## Getting Started

New to icg? Start here:

- **[Onboarding Guide](docs/onboarding-guide.md)** — Structured learning path for operators and developers (recommended starting point)
- **[Quick Start Guide](docs/quick-start.md)** — Get up and running in 5 minutes
- **[Training Manual](docs/operators/training-manual.md)** — Comprehensive operator training (8-hour learning path)
- **[Examples](docs/examples/README.md)** — Real-world scenarios and workflows

## Structure

- `docs/notes/` — features, constraints, design decisions, including the
  existing-infrastructure gap analysis, runtime-config-vs-hardcoded
  exploration, and the release-bound per-repository override contract
- `docs/research/` — external reference material and prior art
  (`destructive_command_guard`, `agent-guard`, `vault-mcp-server`, etc.)
- `docs/operators/` — installation, deployment, upgrade, and troubleshooting
  procedures for the current CLI
  - `training-manual.md` — Comprehensive operator training guide
  - `deny-messages.md` — Complete denial message interpretation guide
- `docs/developers/` — developer documentation for extending icg
  - `rule-pack-best-practices.md` — Best practices for rule pack authoring
- `docs/examples/` — real-world scenarios and workflows
- `docs/onboarding-guide.md` — structured learning path with role-specific tracks
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

## Authoring a rule pack

Start a new pack and its matching regression-test stub together:

```bash
icg new-pack <tool> --pack-type command --output-dir path/to/output
```

`--pack-type` may be `command` (the default) or `content`. The command writes
`<tool>.json` and `<tool>_pack_tests.rs`, pre-filling the pack and guarded-rule
fields. It refuses to overwrite either file, so an existing scaffold must be
removed or renamed deliberately before retrying.
