# irreversible-command-gate Plan

## Overview

A guard for AI coding/automation agents that intercepts commands before they
execute and blocks irreversible or destructive operations, while letting
normal read/write operations through unimpeded — extending the coverage of
the existing `org-rule-guard.py` PreToolUse hook rather than replacing it.

**The objective is not simply to block.** Every rule must leave the agent
knowing the sanctioned alternative, actionable in its very next step — not
just that its attempt failed. See `docs/notes/redirect-not-just-block.md`
for the three-channel mechanism (`permissionDecisionReason`, `updatedInput`,
`additionalContext`) and the hard boundary on when auto-correcting a
command (`updatedInput`) is safe versus when it silently misrepresents what
happened and a `deny` is required instead.

## Architecture

_TBD — two architectural questions already explored in `docs/notes/`:_

- _Hardcoded vs. data-driven rule storage — `runtime-config-vs-hardcoded.md`.
  Resolution so far: the meaningful axis is where the rule source lives
  relative to what the guarded agent can write to, not code-vs-config._
- _Redirect mechanism per rule — `redirect-not-just-block.md`. Resolution
  so far: `deny` + specific reason by default, `updatedInput` only for
  intent-preserving substitutions, `additionalContext` for non-blocking
  warnings._

## Components

_TBD_

## Data Models

_TBD_

## Implementation Phases

- [ ] Phase 1: _TBD_

## Open Questions

- Does this stay a single Claude Code PreToolUse hook (like
  `org-rule-guard.py`), or become a standalone binary/service other tools
  (NEEDLE dispatch, other agent harnesses) can shell out to as well?
- Scope: which of the gaps identified in
  `docs/notes/existing-enforcement-infrastructure.md` does this project
  actually take on, versus staying out of scope (e.g. `.beads/` corruption
  protection may belong in `bf` itself, not a generic command gate)?
- Rule-data location and deploy path, once the runtime-config question is
  resolved — see `docs/notes/runtime-config-vs-hardcoded.md`.
