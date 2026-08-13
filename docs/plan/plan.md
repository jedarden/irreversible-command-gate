# irreversible-command-gate Plan

## Overview

A guard for AI coding/automation agents that intercepts commands before they
execute and blocks irreversible or destructive operations, while letting
normal read/write operations through unimpeded — extending the coverage of
the existing `org-rule-guard.py` PreToolUse hook rather than replacing it.

## Architecture

_TBD — see `docs/notes/runtime-config-vs-hardcoded.md` for the one
architectural question already explored: whether rule definitions should be
hardcoded (matching `org-rule-guard.py`'s current design) or data-driven,
and why "who can write to the rule source" matters more than the
hardcoded-vs-configurable framing suggests._

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
