# irreversible-command-gate Plan

## Overview

A guard for AI coding/automation agents that intercepts commands before they
execute and blocks irreversible or destructive operations, while letting
normal operations through unimpeded — extending the coverage of the
existing `org-rule-guard.py` PreToolUse hook rather than replacing it.
Covers both **Claude Code and Codex CLI** as guarded harnesses.

**The objective is not simply to block.** Every rule must leave the agent
knowing the sanctioned alternative, actionable in its very next step — not
just that its attempt failed. See `docs/notes/redirect-not-just-block.md`.

**Threat model, stated explicitly:** this is a backstop for an honest,
fallible agent that might miss a subtlety of a rule it's already trying to
follow — not a defense against a genuinely adversarial or compromised one.
`org-rule-guard.py` doesn't protect its own source from being edited by the
agent it guards; this project only improves on that if the deploy-location
and release-gating requirements below (Architecture, Phase 0) are actually
implemented, not just designed. Don't oversell this project's guarantees
past that line.

**Known coverage gap, accepted rather than solved:** OpenAI's cloud-hosted
Codex (ChatGPT web / async "Codex cloud tasks") runs in an OpenAI-managed
container this project has no reach into — neither the PATH-wrapper nor a
native hook adapter can see it. Only the local `codex` and Claude Code CLIs
are covered. See `docs/notes/multi-harness-integration.md`.

## Architecture

**Evaluation engine: hardcoded, not data-parsed.** Matches
`org-rule-guard.py`'s proven design — zero I/O beyond stdin, fails open on
any parse failure or exception ("a missed violation is recoverable, a stuck
fleet is not"). No reason to depart from this for the dispatch/parsing
logic itself.

**Rule data: modular, pack-per-tool.** Not a monolithic rule list —
separate units for `vault`, `storage-class`, `image-tag` (extends
`org-rule-guard.py`'s existing `:latest` check with the bare-SHA half),
`git` (force-push), `beads` (`.beads/` protection — see the refined check
below), `tmux` (bare NATO session names), and `misc` (`needle cleanup`,
`br` vs `bf`). Modeled on `destructive_command_guard`'s per-vendor pack
structure — see `docs/research/prior-art.md`. Modularity here means "one
file per tool domain," not "runtime-editable by anyone."

**Deploy location: rule source must not simply live under
`~/.claude/hooks/` again**, and self-updating from GitHub Releases only
counts as satisfying this if release-cutting itself is human-gated — see
Phase 0 and `docs/notes/self-update-and-release-gating.md`. Per
`docs/notes/runtime-config-vs-hardcoded.md`, the axis that matters is not
hardcoded-vs-configurable, it's whether the rule source sits somewhere the
guarded agent's own process can write to (or, for the self-update case,
somewhere the guarded agent can cause to become trusted).

**Integration point: resolved — both layers, both harnesses**, not a
choice between them. Two independent, complementary front-ends sharing one
engine:
- A **PATH-wrapper binary** shadowing `vault`/`bao`/`git`/`kubectl`/etc. —
  harness-agnostic by construction, proven pattern already running in this
  environment (`~/.local/bin/cargo` transparently intercepts `cargo test`;
  see `CLAUDE.md`'s "Rust Build/Test Offloading"). Confirmed to work for
  Codex CLI too: its command execution is `$PATH`-resolved (`execvp`-style),
  and its sandbox restricts filesystem/network, not binary discovery.
- **Native PreToolUse hook adapters** for both Claude Code and Codex CLI —
  Codex ships a structurally similar hook (deny/allow + `updatedInput`, on
  Bash and `apply_patch`), confirmed via OpenAI's own docs, though notably
  younger and still stabilizing (~5 months old as of this writing) than
  Claude Code's.

Rationale for running both rather than picking one: they have non-
overlapping blind spots (a wrapper misses structured/MCP tool calls a hook
sees; a hook is only as reliable as its harness's own implementation, which
for Codex is still maturing). Full reasoning in
`docs/notes/multi-harness-integration.md`.

## Components

- **Engine** — reads a command description (from PreToolUse JSON on stdin,
  or from wrapped-command argv via the PATH-wrapper), segments shell lines
  the same way `org-rule-guard.py`'s `check_bash` does (splits on
  `;`/`&&`/`||`, skips `sudo`/env-assignment/wrapper prefixes), and matches
  the resulting tokens against loaded rule packs.
- **Rule packs** — one file per tool domain (see Architecture). Each entry
  carries a pattern (or, for non-regex checks like the `beads` pack, a
  predicate — see Data Models), a tier, a severity, an explanation, and a
  redirect specification.
- **Redirect dispatch** — chooses `deny` (default), `updatedInput` (only
  for intent-preserving substitutions — stripping `--force`, never a
  silent intent-swap), or `additionalContext` (non-blocking warning). See
  `docs/notes/redirect-not-just-block.md`.
- **`beads` pack's check**, specifically: not a path-prefix block. Gates on
  whether `.git` at the target repo's root is a directory (shared/primary
  tree — the actual concurrent-corruption risk) or a file (linked
  worktree, by construction not shared fleet state). See
  `docs/notes/beads-protection-scope.md` for why the originally-proposed
  `~/`-boundary heuristic doesn't hold and this is the precise substitute.
- **Self-updater** — polls the GitHub Releases API on an interval (not on
  every hook invocation — no network I/O added to the hot path itself),
  swaps in new rule packs without dropping in-flight checks.
  `crates.io`'s API as a secondary version-check signal if this ships as a
  published crate. See `docs/notes/self-update-and-release-gating.md` for
  why release-cutting, specifically, needs its own human gate separate
  from routine CI-on-push.
- **Value-derivation helpers** (later phase, not Phase 1) — for cases where
  the correct redirect value is programmatically derivable at check-time
  (e.g. the real semver from `containers/<name>/VERSION`), embed it
  directly in the deny reason rather than pointing at where to look.
- **State store** (later phase, not Phase 1) — minimal persistent marker
  needed only for Tier 2 ordering rules. `org-rule-guard.py` has no
  equivalent today; this is new surface.

## Data Models

Sketch of a rule pack entry — format (YAML/TOML/Rust struct/etc.) not yet
chosen, this is the field set regardless of format:

```
Pack:
  id: string                     # "vault", "git", "storage-class", "beads", ...
  tool_keywords: [string]        # executables this pack inspects, e.g. ["vault", "bao"]
  safe_patterns: [Pattern]       # explicitly-allowed shapes, checked first
  guarded_patterns: [GuardedPattern]

GuardedPattern:
  id: string
  check: Regex | Predicate       # most packs use a command-text regex; `beads`
                                  # uses a filesystem predicate (.git file vs. dir)
  tier: 1 | 2 | 3                # deterministic-difficulty tier, see Implementation Phases
  severity: Critical | High | Medium
  explanation: string            # why this is dangerous
  redirect:
    channel: deny | updatedInput | additionalContext
    reason_template: string      # supports {derived_value} placeholders
    rewrite_template: string     # only used when channel = updatedInput
```

## Implementation Phases

- [ ] **Phase 0 — deploy path.**
  - Build the `icg-ci` Argo WorkflowTemplate (`declarative-config/k8s/iad-ci/argo-workflows/`)
    on the existing `forge-ci`/`needle-ci`/`agentscribe-ci`/`sigil-ci`
    pattern — Rust binary → GitHub Release, never GitHub Actions.
  - Implement release-integrity verification per
    `docs/notes/release-integrity-verification.md`: a fixed
    deny-must-still-fire regression suite plus a structured coverage-diff
    check (removed patterns, widened `safe_patterns`, narrowed
    `destructive_patterns`) as required, build-failing `icg-ci` gates
    (Layer 1); human review informed by that generated diff report, not
    raw regex (Layer 2); and a self-updater that tracks a separately-
    advancing trust pointer rather than bare "latest release" (Layer 4,
    minimal form). Build provenance/signing and staged/canary rollout
    (Layer 3 and Layer 4's full form) are real hardening but explicitly
    deferred, not required for Phase 0 — see that note's "How to apply."
  - Decide and implement the hot-reload trigger cadence and mechanism
    (poll interval; process restart vs. in-memory rule-pack swap).
  - Nothing shipped in later phases is meaningfully tamper-resistant until
    this phase lands — without it, this project reproduces
    `org-rule-guard.py`'s existing self-edit gap under a new name, just
    with extra steps.
- [ ] **Phase 1 — Tier 1 rules, both front-ends, deny-only.** Build the
      PATH-wrapper binary and both hook adapters (Claude Code, Codex).
      Rule set: Bash-channel secret-value scanning (reuse
      `org-rule-guard.py`'s existing regex machinery, currently only wired
      to the Write/Edit path), Vault/OpenBao destructive verbs (the core
      motivating gap — `kv destroy`, `secrets disable`, `policy delete`,
      token/lease revoke), `ssd`/`ssd-large` storage class, bare-SHA image
      pinning, force-push, `.beads/` protection (`.git` file-vs-directory
      check, not a path block), `br` vs `bf`, `needle cleanup`, bare NATO
      tmux session targeting. Redirect channel: `deny` + specific reason
      for all of these — skip `updatedInput`/`additionalContext`
      complexity for v1.
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
      contexts elsewhere in this environment (including the sanctioned
      throwaway-worktree `.beads`-conflict pattern this project's own
      `beads` pack now depends on). If ever pursued, it would be a
      heuristic, non-blocking `additionalContext` warning, not a `deny`.
- [ ] **Explicitly not attempted:** narrowing the existing `org-rule-guard.py`
      kubectl-mutation block down to "only ArgoCD-managed resources" —
      doing so accurately requires live cluster state, which trades away
      the zero-I/O determinism that makes the current blanket block
      trustworthy. The blanket version stays as-is; this project doesn't
      touch it.

## Open Questions

- **Release-gating mechanism, narrowed**: the verification layers are
  chosen (regression suite + coverage-diff + informed review + trust
  pointer, per `docs/notes/release-integrity-verification.md`), but the
  literal trigger for cutting a release still isn't — manual `gh release
  create`, an approval-gated workflow, or something else. Smaller decision
  than before, not a fully open one.
- **Hot-reload trigger specifics**: poll interval, and process-restart vs.
  in-memory rule-pack hot-swap.
- **Value-derivation helpers' Phase 1 inclusion**: scoped to Phase 3 as a
  judgment call, not an explicit decision — worth revisiting once Phase
  1's actual rule count makes the manual-authoring cost concrete.
- **`beads`-in-`bf` question, narrower now than originally framed**: given
  the refined check lives at the filesystem-predicate level (`.git`
  file-vs-directory), does it still make more sense inside `bf` itself
  (which already knows about worktrees and shared trees) than as a generic
  guard pack here? Leaning toward keeping it here since the rest of the
  `beads` pack's context (this is a *guard*, invoked pre-execution) doesn't
  naturally live inside `bf`, but not settled.
