# irreversible-command-gate Plan

## Overview

A guard for AI coding/automation agents that intercepts commands before they
execute and blocks irreversible or destructive operations, while letting
normal operations through unimpeded — currently extending the coverage of
the existing `org-rule-guard.py` PreToolUse hook, not replacing it, but
**`org-rule-guard.py` is expected to be deprecated once this project's
coverage supersedes it** (per user direction 2026-08-13) — coexistence is
an interim state, not the intended end state. `icg-53q` (install-time
smoke test confirming no conflict between the two) is framed accordingly.
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
logic itself. **One deliberate, scoped exception**: `icg-2m8`'s
stale-HEAD-before-push check needs a live remote lookup, justified only
because `git push` is already a network operation — not a precedent for
adding I/O to any other guarded command.

**This fail-open guarantee is scoped to in-process errors** — a single
check throwing or failing to parse always fails open, unconditionally,
matching `org-rule-guard.py`'s exact behavior. **The guard process itself
disappearing (OOM-killed, crashed) is a separate failure class**, governed
by `icg-4bu`'s graduated fail-open→fail-closed policy (Phase 5): fails
open while the guard's reliability is unproven, shifts to fail-closed once
it's validated. These are two different questions — "did this one check
error out" vs. "is the guard even still running" — and only the second one
is allowed to graduate away from fail-open over time.

**Open implementation question `icg-4bu` doesn't resolve on its own:**
"fail closed when the process is dead" needs *something* to notice the
process is gone and substitute a deny — a standing watchdog is exactly the
"guard as a standing daemon" architecture Lens-1 idea #69 was killed for
during ideation (real architecture change for unclear benefit against the
current per-invocation model). Leading hypothesis, **not yet confirmed**:
the fail-closed transition doesn't need icg's own watchdog at all if
Claude Code's and Codex's own PreToolUse hook systems already have
configurable behavior for "the hook command errored, timed out, or never
responded" — in which case "fail closed" means configuring *that* harness
setting once reliability is validated, not building new standing
infrastructure. Needs verifying against both harnesses' actual hook specs
before `icg-4bu` is implemented; if neither harness supports it, this
finalist needs to either accept the standing-daemon cost after all or be
re-scoped.

**Rule data: modular, pack-per-tool.** Not a monolithic rule list —
separate units for `vault`, `storage-class`, `image-tag` (extends
`org-rule-guard.py`'s existing `:latest` check with the bare-SHA half),
`git` (force-push, stale-HEAD-before-push), `beads` (`.beads/` protection
— see the refined check below), `tmux` (bare NATO session names), `secrets`
(Bash-channel credential-value scanning, extending `org-rule-guard.py`'s
existing regex machinery to a path it doesn't currently cover — see Phase
1), and `misc` (`needle cleanup`, deprecated-bead-CLI usage). `kubectl` is
deliberately **not** a pack here — its mutating-verb coverage stays
`org-rule-guard.py`'s job (see "Explicitly not attempted" below), so it's
not one of the binaries the PATH-wrapper needs its own rules for. The
`:latest`/secrets/kubectl claims above are grounded in
`docs/notes/existing-enforcement-infrastructure.md`'s direct read of
`org-rule-guard.py`'s source — re-check that note if the source ever
changes. Modeled on
`destructive_command_guard`'s per-vendor pack structure — see
`docs/research/prior-art.md`. Modularity here means "one file per tool
domain," not "runtime-editable by anyone."

**`misc` pack's deprecated-bead-CLI rule is data-driven by design, not
hardcoded to one tool name.** `br` (beads_rust) is already deprecated in
favor of `bf` (bead-forge) — but `bf` is itself being prepared for
deprecation in favor of `bead-rs` (`~/bead-rs`, binary `bead`, a separate
clean-room reimplementation, not the same lineage as `br` despite the
similar name). The rule's actual policy is "don't invoke a deprecated bead
CLI," which outlives any single tool's canonical status — the pack stores
"currently canonical" and "deprecated" as data (a small list this rule
reads), not as logic hardcoded to `bf` specifically, so the eventual
`bf`→`bead` cutover is a one-line data change, not a rule rewrite. See
Phase 1 and `icg-1vj`.

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
- A **PATH-wrapper binary** shadowing whatever binaries the *currently
  loaded* rule packs cover — `vault`/`bao`/`git` from Phase 1, `docker`
  once Phase 4's `icg-d3i` pack ships (not before; shadowing a binary with
  no pack behind it yet is a pure no-op) — never `kubectl` (see
  Architecture's pack list). Harness-agnostic by construction, proven
  pattern already running in this
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

- **Engine** — two input modes, not one. **Command mode**: reads a command
  description (from PreToolUse JSON on stdin, or from wrapped-command argv
  via the PATH-wrapper), segments shell lines the same way
  `org-rule-guard.py`'s `check_bash` does (splits on `;`/`&&`/`||`, skips
  `sudo`/env-assignment/wrapper prefixes), and matches the resulting tokens
  against loaded rule packs — what `vault`, `git`, `secrets`, `misc`, and
  `tmux` packs use. **Content mode**: reads a file path + content (from
  `Write`/`Edit` PreToolUse JSON, mirroring exactly how `org-rule-guard.py`
  itself is triggered today), matched against packs whose checks are
  content regexes, not command-text ones. `storage-class` and `image-tag`
  are content-mode packs — the target text is a YAML manifest line
  (`storageClassName: ssd`, `image: foo:latest`), never a shell command, so
  they can only ever fire on the hook front-end, never the PATH-wrapper
  (which only ever sees a subprocess exec, not a file write). The `beads`
  pack is content-mode-adjacent but distinct again — its predicate reads
  the filesystem at the target repo root, not the write's own content.
- **Rule packs** — one file per tool domain (see Architecture). Each entry
  carries a pattern (or, for non-regex checks like the `beads` pack, a
  predicate — see Data Models), a tier, a severity, an explanation, and a
  redirect specification.
- **Redirect dispatch** — chooses `deny` (default), `updatedInput` (only
  for intent-preserving substitutions — stripping `--force`, never a
  silent intent-swap), or `additionalContext` (non-blocking warning). See
  `docs/notes/redirect-not-just-block.md`.
- **`icg` binary — one binary, two dispatch modes.** Invoked under its own
  name (`icg update`, `icg status`, `icg new-pack`, ...), it runs
  subcommand dispatch for administrative commands. Invoked under a
  shadowed tool's name (`vault`, `git`, `docker`, ...) — via symlinks
  installed earlier in `$PATH` than the real binaries, the same shape as
  the existing `cargo` precedent — it dispatches on `argv[0]` instead:
  runs the engine's command-mode checks, and if allowed, execs the real
  binary (found further down `$PATH`) with the original arguments
  untouched. This is the PATH-wrapper front-end's actual implementation,
  not a separate component from it.
- **`beads` pack's check**, specifically: not a path-prefix block. Gates on
  whether `.git` at the target repo's root is a directory (shared/primary
  tree — the actual concurrent-corruption risk) or a file (linked
  worktree, by construction not shared fleet state). See
  `docs/notes/beads-protection-scope.md` for why the originally-proposed
  `~/`-boundary heuristic doesn't hold and this is the precise substitute.
- **Self-updater** — user-triggered, not polling (resolved 2026-08-13; see
  Phase 0). On trigger, checks the GitHub Releases API once and performs
  an in-memory rule-pack hot-swap without dropping in-flight checks or
  restarting the process — no network I/O on the guarded-check hot path
  itself, since the check only happens on an explicit trigger, never
  automatically. `crates.io`'s API as a secondary version-check signal if
  this ships as a published crate. See
  `docs/notes/self-update-and-release-gating.md` for why release-cutting,
  specifically, needs its own human gate separate from routine
  CI-on-push.
- **Value-derivation helpers** (later phase, not Phase 1) — for cases where
  the correct redirect value is programmatically derivable at check-time
  (e.g. the real semver from `containers/<name>/VERSION`), embed it
  directly in the deny reason rather than pointing at where to look.
- **State store** (later phase, not Phase 1) — minimal persistent marker
  needed only for Tier 2 ordering rules. `org-rule-guard.py` has no
  equivalent today; this is new surface.
- **Per-repo override (`icg-2i8`), "signed" clarified**: not a bespoke
  cryptographic signing scheme with its own key to manage — an override
  file is "signed" in the sense that it only takes effect after going
  through the exact same Layer 1/2 release-integrity pipeline as any other
  rule-pack change (commit, `icg-ci` gates, human review), and is only
  trusted by a host once it's part of a release the trust pointer (Layer
  4) already vouches for. There is no override-specific private key that
  could itself become a new agent-writable hole — the existing pipeline
  *is* the signature.

## Data Models

**Tier definitions**, for classifying a new `GuardedPattern` (referenced
throughout Implementation Phases but not previously defined in one place):
- **Tier 1** — stateless, decidable from a single invocation's command
  text or a filesystem predicate alone. What Phase 1 ships.
- **Tier 2** — deterministic, but needs state that persists *across*
  invocations (e.g. "did a `git pull` happen earlier in this session").
  What Phase 2's state store exists for.
- **Tier 3** — not reliably decidable from command syntax at all; the
  same command is legitimate in some contexts and dangerous in others
  (the canonical example: `git worktree add`). Never a `deny` — at most a
  non-blocking, heuristic `additionalContext` warning, and only if ever
  pursued at all (see "Out of scope for now").

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
  check: CommandRegex | ContentRegex | Predicate
                                  # CommandRegex: matched against shell tokens (vault/git/secrets/
                                  # misc/tmux packs, both front-ends). ContentRegex: matched
                                  # against Write/Edit file content (storage-class/image-tag
                                  # packs, hook front-end only — see Components' Engine).
                                  # Predicate: filesystem check (beads pack's .git file-vs-dir).
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
  - **Release-cutting trigger, resolved (2026-08-13):** a human manually
    runs `gh release create` once `icg-ci` has passed on the target
    commit — no additional approval-workflow layer beyond that. Layers
    1/2/4 already provide sufficient protection; a heavier gated-workflow
    mechanism isn't needed on top of them.
  - **Hot-reload, resolved (2026-08-13):** user-triggered, not automatic
    background polling — an operator explicitly triggers an update (e.g.
    an `icg update` command) when they want a host to pick up a new
    release. On trigger, the guard performs an in-memory rule-pack
    hot-swap, never a process restart, so the host being updated never
    blocks its own guarded agent sessions while updating. No fleet-wide
    synchronization point — triggering one host doesn't require pausing
    or waiting on any other host, consistent with the already-adopted
    canary-rollout design (`icg-l75`). This is asymmetric with
    `icg-2ck`'s poison-pill auto-rollback by design: adopting a new
    release forward is deliberate/manual, but reverting an already-
    adopted bad one stays automatic — different risk profiles, not a
    contradiction.
  - **`icg update` doesn't need to be restricted to a human-attributable
    session** — a deliberate, documented exception to
    `docs/notes/runtime-config-vs-hardcoded.md`'s general rule (updated to
    state this exception explicitly, so the two documents agree): what
    actually matters is *what* gets loaded, not *who* asked for the check.
    The trust pointer (Layer 4) already constrains that to whatever a
    human already vetted and released; the guarded agent running `icg
    update` itself can only cause an early adoption of something
    already-trusted, never cause anything untrusted to load. This
    exception holds only because release-cutting is separately human-gated
    — it would not hold without that.
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
      token/lease revoke), `ssd`/`ssd-large` storage class, **both halves**
      of image-tag pinning (`:latest` re-detection *and* bare-SHA — the
      `image-tag` pack fully absorbs this rule from `org-rule-guard.py` as
      of Phase 1, not just the bare-SHA gap; see Architecture and
      `docs/notes/existing-enforcement-infrastructure.md`), force-push,
      stale-HEAD-before-push (`icg-2m8` — the one Tier 1 rule with a live
      remote check, a deliberate, scoped exception to the engine's
      zero-I/O rule since `git push` is already a network operation; see
      Architecture), `.beads/` protection (`.git` file-vs-directory check,
      not a path block),
      deprecated-bead-CLI usage (data-driven, `icg-1vj` — currently `br`;
      ready to retarget at `bf` once it's deprecated), `needle cleanup`,
      bare NATO tmux session targeting. Redirect channel: `deny` + specific
      reason for all of these — skip `updatedInput`/`additionalContext`
      complexity for v1.
- [ ] **Phase 2 — cross-invocation state (Tier 2).** Two rules, stated by
      enforcement direction, not by parallel naming — they have *opposite*
      required polarity and "flush-before-X" phrasing for both would
      invite building one of them backwards:
      - **Deny `bf sync --flush-only` unless a `git pull` has already
        happened in this session.** Flushing before pulling is the
        prohibited sequence — pull must come first.
      - **Deny `bf doctor --repair` unless a flush has already happened in
        this session.** Here flush must come *first* — the opposite
        requirement from the rule above, despite the superficially similar
        name.

      Needs the state-store component, which nothing in Phase 1 requires.
      Author against whichever bead CLI is canonical at implementation
      time (`bf`'s `sync --flush-only` flag today; `bead-rs`'s `sync
      flush-only` subcommand has different syntax entirely if the cutover
      happens first — see `icg-1vj`, don't assume the two are
      interchangeable here).
- [ ] **Phase 3 — redirect-mechanism richness.** Introduce `updatedInput`
      for confirmed intent-preserving cases (force-push flag stripping is
      the clearest candidate) and `additionalContext` for non-blocking
      warnings; add value-derivation helpers so deny reasons embed real
      answers instead of pointers. (Ideation finalist #10 proposed
      building the force-push `updatedInput` case in Phase 1 instead, to
      de-risk the mechanism early — considered and deliberately deferred
      to notes only, not adopted as a bead; see
      `docs/notes/ideas-ledger.md`.)

      **`additionalContext`-channel rules are Claude-Code-only for now** —
      per `docs/notes/multi-harness-integration.md`, Codex's hook schema
      accepts the field but doesn't yet honor it, so a warning sent that
      way would be silently dropped on Codex, violating the "every rule
      needs an actionable redirect" principle on that harness. Any rule
      that reaches for `additionalContext` needs an explicit Codex
      fallback (most likely downgrading to `deny` there until Codex's
      hook implementation catches up) rather than assuming the same
      behavior on both front-ends.
- [ ] **Out of scope for now:** per-worker git worktree isolation for
      NEEDLE (Tier 3) — not a good fit for command-pattern matching, since
      the identical `git worktree add` command is legitimate in other
      contexts elsewhere in this environment (including the sanctioned
      throwaway-worktree `.beads`-conflict pattern this project's own
      `beads` pack now depends on). If ever pursued, it would be a
      heuristic, non-blocking `additionalContext` warning, not a `deny`.
- [ ] **Phase 4 — from ideation (2026-08-13 `/plan-idea-gen` run).** Nine
      finalists adopted, tracked as beads (`bf` prefix `icg`), full
      dossiers and kill-pass objections in `docs/notes/ideas-ledger.md`.
      Deepens Phase 0's release-integrity/self-update work and Phase 1's
      rule coverage rather than opening new phases of its own:
      - `icg-rri` — auto-denial-becomes-test (strengthens Layer 1; needs a
        curation step so the suite doesn't grow unbounded)
      - `icg-ncf` — `icg new-pack` scaffolding tool
      - `icg-2ck` — poison-pill auto-rollback (extends Phase 0's Layer 4)
      - `icg-l75` — canary rollout via NEEDLE `--identifier` (concrete
        Layer 4 staged-rollout implementation)
      - `icg-1tj` — `icg status` with blind-spot self-report
      - `icg-z5n` — Codex hook-version compatibility matrix in `icg-ci`
      - `icg-2i8` — per-repo signed override (routed through Layer 1/2)
      - `icg-59u` — practice/dry-run mode (ships only with the mandatory
        persistent active-indicator the kill pass required). Near-miss
        feedback deliberately does **not** rely on `additionalContext` —
        given that channel isn't honored on Codex yet (see Phase 3), a
        practice-mode report needs to reach both harnesses identically;
        the persistent banner requirement already covers this, surfaced
        directly rather than through a hook-response field either harness
        might drop.
      - `icg-d3i` — Docker destructive-ops pack (new Phase-1-shaped pack,
        same architecture as `vault`)
- [ ] **Phase 5 — from ideation (2026-08-13 second `/plan-idea-gen` run).**
      Six finalists adopted as beads, one (explicit README non-goals)
      done directly rather than tracked as a bead. Full dossiers and
      kill-pass objections in `docs/notes/ideas-ledger.md`'s second-run
      section:
      - `icg-4p8` — guard CI/build pods on iad-ci, including this
        project's own `icg-ci` release pipeline
      - `icg-2m8` — stale-HEAD push guard, the shipped form of ledger
        finalist #2 ("shared-tree collision protection") after user
        revision (compares tracked vs. actual remote HEAD before
        `git push`, a simpler mechanism than the originally-proposed
        cross-process `/proc` scanning — a deliberate, scoped exception
        to the no-I/O-hot-path rule, since `git push` is already a
        network operation)
      - `icg-4bu` — graduated fail-open→fail-closed policy for guard
        crashes: fails open until the guard's reliability is validated
        (tied to `icg-2ck`'s poison-pill health signal), then shifts to
        fail-closed
      - `icg-3xz` — ReDoS check on submitted rule packs in `icg-ci`
      - `icg-4mu` — per-rule enable/disable feature flag, revised from a
        dedicated fast-path kill-switch to reuse the normal Layer 1/2
        release pipeline (tradeoff: no longer sub-release-cycle-fast —
        flagged as a real, unresolved gap if true emergency speed is ever
        needed)
      - `icg-53q` — install-time smoke test vs. `org-rule-guard.py`,
        framed as an interim check pending that hook's eventual
        deprecation (see Overview)
      - README's "What this does not do" section — done directly, not a
        bead (see `README.md`)

      Finalists #3 (dead-man's-switch SCRAM), #4 (guard-as-MCP-server),
      and #5 (separation of duties: author ≠ approver) were considered and
      **deliberately deferred to notes only, not adopted as beads**, per
      user direction — same treatment as run 1's finalist #10. See
      `docs/notes/ideas-ledger.md`'s second-run section for their full
      reasoning if revisited later.
- [ ] **Explicitly not attempted:** narrowing the existing `org-rule-guard.py`
      kubectl-mutation block down to "only ArgoCD-managed resources" —
      doing so accurately requires live cluster state, which trades away
      the zero-I/O determinism that makes the current blanket block
      trustworthy. The blanket version stays as-is; this project doesn't
      touch it.

## Open Questions

- ~~Release-gating mechanism~~ — **resolved 2026-08-13**, see Phase 0.
- ~~Hot-reload trigger specifics~~ — **resolved 2026-08-13**, see Phase 0
  and Components.
- **Value-derivation helpers' Phase 1 inclusion — known tradeoff, not an
  oversight.** `docs/notes/redirect-not-just-block.md` uses the exact
  `image-tag` bare-SHA case as its own canonical illustration of an
  inadequate redirect ("pin a semver tag read from
  `containers/<name>/VERSION`" vs. embedding the real value). Phase 1
  ships that identical rule without value-derivation anyway, deliberately
  deferred to Phase 3 for scope reasons — this project reproduces, for one
  phase, the specific shortfall its own design notes single out. Accepted
  consciously, not silently; revisit once Phase 1's actual rule count
  makes the manual-authoring cost of doing it earlier concrete.
- ~~`beads`-in-`bf` question~~ — **resolved 2026-08-13: stays in this
  project.** With `bf` itself now confirmed heading toward deprecation
  (see the Architecture section and `icg-1vj`), embedding the `.beads/`
  protection check inside a tool that's about to be superseded would just
  mean redoing this work again at the next cutover. Phase 1 already
  unconditionally depends on this answer, so leaving it formally open any
  longer served no purpose — the deprecation news settles it.
