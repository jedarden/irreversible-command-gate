# irreversible-command-gate Plan

## Overview

A guard for AI coding/automation agents that intercepts commands before they
execute and blocks irreversible or destructive operations, while letting
normal operations through unimpeded — extending the coverage of the
existing `org-rule-guard.py` PreToolUse hook rather than replacing it.

**The objective is not simply to block.** Every rule must leave the agent
knowing the sanctioned alternative, actionable in its very next step — not
just that its attempt failed. See `docs/notes/redirect-not-just-block.md`.

**Threat model, stated explicitly:** this is a backstop for an honest,
fallible agent that might miss a subtlety of a rule it's already trying to
follow — not a defense against a genuinely adversarial or compromised one.
`org-rule-guard.py` doesn't protect its own source from being edited by the
agent it guards, and nothing in this plan changes that fact for whatever
this project ships either, unless the deploy-location open question below
gets resolved in a way that removes the guarded agent's write access to the
rule source. Don't oversell this project's guarantees past that line.

## Architecture

**Evaluation engine: hardcoded, not data-parsed.** Matches
`org-rule-guard.py`'s proven design — zero I/O beyond stdin, fails open on
any parse failure or exception ("a missed violation is recoverable, a stuck
fleet is not"). No reason to depart from this for the dispatch/parsing
logic itself.

**Rule data: modular, pack-per-tool.** Not a monolithic rule list —
separate units for `vault`, `storage-class`, `image-tag` (extends
`org-rule-guard.py`'s existing `:latest` check with the bare-SHA half),
`git` (force-push), `beads` (`.beads/` path block), `tmux` (bare NATO
session names), and `misc` (`needle cleanup`, `br` vs `bf`). Modeled on
`destructive_command_guard`'s per-vendor pack structure — see
`docs/research/prior-art.md`. Modularity here means "one file per tool
domain," not "runtime-editable by anyone" — see the next point.

**Deploy location: rule source must not simply live under
`~/.claude/hooks/` again.** Per `docs/notes/runtime-config-vs-hardcoded.md`,
the axis that matters is not hardcoded-vs-configurable, it's whether the
rule source (in either form) sits somewhere the guarded agent's own process
can write to. `org-rule-guard.py` currently doesn't clear this bar. This
project should: this repo is the authoring location (human-edited,
git-tracked, PR/commit-reviewed like everything else here), and the
*deployed, live-checked* copy needs to reach the guarded agent's hook
invocation through a path the agent's own tool calls don't control — a
pull-based sync (matching this environment's own declarative-config /
ArgoCD pattern) rather than a file the agent can `Write`/`Edit` directly.
The exact mechanism is an open question below; the constraint is settled.

**Integration point: still open between two shapes**, both with working
precedent already in this environment:
- A second Claude Code PreToolUse hook, wired via `settings.json` like
  `org-rule-guard.py` — simplest, but only protects Claude Code sessions
  specifically.
- A standalone binary agents shell out through via PATH interception —
  this environment already runs exactly this pattern for a different
  purpose: `~/.local/bin/cargo` transparently intercepts `cargo test` calls
  and redirects them to remote CI (see `CLAUDE.md`'s "Rust Build/Test
  Offloading" section) without the calling agent doing anything different.
  A `vault`/`bao`/`git`/`kubectl` wrapper on PATH could apply the same
  proven interception shape across NEEDLE-dispatched workers and any other
  harness, not just Claude Code.

See Open Questions for why this isn't decided yet.

## Components

- **Engine** — reads a command description (from PreToolUse JSON on stdin,
  or from wrapped-command argv if the PATH-wrapper shape is chosen),
  segments shell lines the same way `org-rule-guard.py`'s `check_bash` does
  (splits on `;`/`&&`/`||`, skips `sudo`/env-assignment/wrapper prefixes),
  and matches the resulting tokens against loaded rule packs.
- **Rule packs** — one file per tool domain (see Architecture). Each entry
  carries a pattern, a tier (from the deterministic-difficulty scoping
  below), a severity, an explanation, and a redirect specification.
- **Redirect dispatch** — chooses `deny` (default), `updatedInput` (only
  for intent-preserving substitutions — stripping `--force`, not swapping
  in a different operation), or `additionalContext` (non-blocking warning).
  See `docs/notes/redirect-not-just-block.md` for the boundary between
  these.
- **Value-derivation helpers** (later phase, not Phase 1) — for cases where
  the correct redirect value is programmatically derivable at check-time
  (e.g. the real semver from `containers/<name>/VERSION`), embed it
  directly in the deny reason rather than pointing at where to look.
- **State store** (later phase, not Phase 1) — minimal persistent marker
  needed only for Tier 2 ordering rules (see Implementation Phases).
  `org-rule-guard.py` has no equivalent today; this is new surface.

## Data Models

Sketch of a rule pack entry — format (YAML/TOML/Rust struct/etc.) not yet
chosen, this is the field set regardless of format:

```
Pack:
  id: string                     # "vault", "git", "storage-class", ...
  tool_keywords: [string]        # executables this pack inspects, e.g. ["vault", "bao"]
  safe_patterns: [Pattern]       # explicitly-allowed shapes, checked first
  guarded_patterns: [GuardedPattern]

GuardedPattern:
  id: string
  pattern: regex
  tier: 1 | 2 | 3                # deterministic-difficulty tier, see Implementation Phases
  severity: Critical | High | Medium
  explanation: string            # why this is dangerous
  redirect:
    channel: deny | updatedInput | additionalContext
    reason_template: string      # supports {derived_value} placeholders
    rewrite_template: string     # only used when channel = updatedInput
```

## Implementation Phases

- [ ] **Phase 0 — deploy path.** Resolve and implement the rule-source
      deploy location (see Architecture's deploy-location constraint and
      the open question below). Nothing shipped in Phase 1 is meaningfully
      tamper-resistant until this lands — without it, this project just
      reproduces `org-rule-guard.py`'s existing self-edit gap under a new
      name.
- [ ] **Phase 1 — Tier 1 rules, deny-only.** The rules already scoped as
      "same difficulty as what `org-rule-guard.py` already does":
      Bash-channel secret-value scanning (reuse `org-rule-guard.py`'s
      existing regex machinery, currently only wired to the Write/Edit
      path), Vault/OpenBao destructive verbs (the core motivating gap —
      `kv destroy`, `secrets disable`, `policy delete`, token/lease
      revoke), `ssd`/`ssd-large` storage class, bare-SHA image pinning,
      force-push, `.beads/` path block, `br` vs `bf`, `needle cleanup`,
      bare NATO tmux session targeting. Redirect channel: `deny` +
      specific reason for all of these — skip `updatedInput`/
      `additionalContext` complexity for v1.
- [ ] **Phase 2 — cross-invocation state.** `bf` flush-before-pull /
      flush-before-repair ordering (Tier 2) — needs the state-store
      component, which nothing in Phase 1 requires.
- [ ] **Phase 3 — redirect-mechanism richness.** Introduce `updatedInput`
      for confirmed intent-preserving cases (force-push flag stripping is
      the clearest candidate) and `additionalContext` for non-blocking
      warnings; add value-derivation helpers so deny reasons embed real
      answers instead of pointers.
- [ ] **Out of scope for now:** per-worker git worktree isolation for
      NEEDLE (Tier 3) — not a good fit for command-pattern matching, since
      the identical `git worktree add` command is legitimate in other
      contexts elsewhere in this environment. If ever pursued, it would be
      a heuristic, non-blocking `additionalContext` warning, not a `deny`.
- [ ] **Explicitly not attempted:** narrowing the existing `org-rule-guard.py`
      kubectl-mutation block down to "only ArgoCD-managed resources" —
      doing so accurately requires live cluster state, which trades away
      the zero-I/O determinism that makes the current blanket block
      trustworthy. The blanket version stays as-is; this project doesn't
      touch it.

## Open Questions

- **Integration shape**: second Claude Code PreToolUse hook (simple,
  Claude-Code-only) vs. PATH-wrapper standalone binary (broader coverage
  across NEEDLE/other harnesses, proven pattern via the existing
  `cargo`/`cargo-remote` wrapper, more engineering). Blocks Phase 1's
  concrete implementation, not just Phase 0.
- **Deploy mechanism specifics**: given "pull-based, agent doesn't control
  the reload" is the resolved constraint, what actually triggers the pull —
  a timer, a separate always-running process, something else? And does the
  live copy live outside `~/.claude/hooks/` entirely, or in a
  root-owned/agent-unwritable path within it?
- **`.beads/` scope boundary**: does path-blocking `.beads/` writes belong
  in this project at all, or should that protection live inside `bf`
  itself? Flagged as unresolved since the project's original scaffold.
- **Value-derivation helpers' Phase 1 inclusion**: scoped to Phase 3 above
  as a judgment call, not an explicit decision — worth revisiting once
  Phase 1's actual rule count makes the manual-authoring cost concrete.
