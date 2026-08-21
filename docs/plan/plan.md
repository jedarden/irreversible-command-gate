# irreversible-command-gate Plan

## Overview

A guard for AI coding/automation agents that intercepts commands before they
execute and blocks irreversible or destructive operations, while letting
normal operations through unimpeded — currently extending the coverage of
the existing `org-rule-guard.py` PreToolUse hook, not replacing it, but
**`org-rule-guard.py` is expected to shrink toward deprecation as this
project's coverage supersedes it** (per user direction 2026-08-13) —
coexistence is an interim state, not the intended end state. **Realistic
end state, stated precisely rather than aspirationally**: not full
removal. Two things keep it alive under the plan's *current*, actually
scheduled scope: (1) its kubectl-mutation rule is **permanently** excluded
from absorption ("Explicitly not attempted" — zero-I/O determinism
reasons that don't go away), and (2) its `.github/workflows` and `kind:
Job`/`CronJob` rules aren't excluded on principle, just **not yet
scheduled** by any phase (see
`docs/notes/existing-enforcement-infrastructure.md`). So today's accurate
claim is "shrinks to at least a kubectl-only rump, plus whichever of the
workflows/Job-CronJob rules remain unscheduled" — not "kubectl-only,"
until a future phase actually picks up (2). "Deprecated" means
"superseded for everything a phase has scheduled," not "deleted." `irrevers-62c6f748`
(install-time smoke test confirming no conflict between the two) is
framed accordingly. Covers both **Claude Code and Codex CLI** as guarded
harnesses.

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
`org-rule-guard.py`'s proven design — **zero *network* I/O** beyond stdin,
fails open on any parse failure or exception ("a missed violation is
recoverable, a stuck fleet is not"). No reason to depart from this for the
dispatch/parsing logic itself. This is a network-I/O restriction
specifically, not a blanket "no I/O of any kind" claim — local filesystem
reads the design already requires (the `beads` pack's `.git` stat check,
rule-pack loading itself, value-derivation helpers reading a `VERSION`
file) are fine and not exceptions to anything. **One deliberate, scoped
exception to the *network*-I/O restriction**: `irrevers-8cff8cf4`'s
stale-HEAD-before-push check needs a live remote lookup, justified only
because `git push` is already a network operation — not a precedent for
adding network I/O to any other guarded command.

**This fail-open guarantee is scoped to in-process errors** — a single
check throwing or failing to parse always fails open, unconditionally,
matching `org-rule-guard.py`'s exact behavior. **The guard process itself
disappearing (OOM-killed, crashed) is a separate failure class**, governed
by `irrevers-cd3f4c44`'s graduated fail-open→fail-closed policy (Phase 5): fails
open while the guard's reliability is unproven, shifts to fail-closed once
it's validated. These are two different questions — "did this one check
error out" vs. "is the guard even still running" — and only the second one
is allowed to graduate away from fail-open over time.

**Open implementation question `irrevers-cd3f4c44` doesn't resolve on its own:**
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
before `irrevers-cd3f4c44` is implemented; if neither harness supports it, this
finalist needs to either accept the standing-daemon cost after all or be
re-scoped.

**Rule data: modular, pack-per-tool.** Not a monolithic rule list —
separate units for `openbao`, `storage-class`, `image-tag` (extends
`org-rule-guard.py`'s existing `:latest` check with the bare-SHA half),
`git` (force-push, stale-HEAD-before-push, commit-without-pathspec — see
Phase 1), `beads` (`.beads/` protection
— see the refined check below), `tmux` (bare NATO session names), `secrets`
(Bash-channel credential-value scanning, extending `org-rule-guard.py`'s
existing regex machinery to a path it doesn't currently cover — see Phase
1; **hook-front-end-only, not "both front-ends"** — its regex scans the
*entire raw Bash command string* regardless of which executable is being
invoked, which the PATH-wrapper's dispatch model structurally can't do:
the wrapper is only ever invoked as one specific shadowed binary via
`argv[0]`, so it never sees a command line for a binary it doesn't
shadow, e.g. `echo "ghp_..." >> file`. Only the hook, which receives every
Bash call's full command text regardless of executable, can realize
this), and `misc` (`needle cleanup`, deprecated-bead-CLI usage). `kubectl`
is deliberately **not** a pack here — its mutating-verb coverage stays
`org-rule-guard.py`'s job (see "Explicitly not attempted" below), so it's
not one of the binaries the PATH-wrapper needs its own rules for. The
`:latest`/secrets/kubectl claims above are grounded in
`docs/notes/existing-enforcement-infrastructure.md`'s direct read of
`org-rule-guard.py`'s source — re-check that note if the source ever
changes. Modeled on
`destructive_command_guard`'s per-vendor pack structure — see
`docs/research/prior-art.md`. Modularity here means "one file per tool
domain," not "runtime-editable by anyone."

**`openbao` pack, scope resolved and expanded (2026-08-20).** Drafted above
and in Phase 1 as a `vault`-named pack covering only destructive verbs, this
shipped as `openbao` — `tool_keywords: ["bao", "vault"]`, so both binary
names still get PATH-wrapper coverage — with three guarded patterns, not
one:
1. **`openbao-inline-secret-literal`** (new, `deny`) — a `kv put`/`kv patch`
   whose data argument carries a literal value. The leak happens before the
   write ever reaches OpenBao: argv lands in the agent transcript, shell
   history, and `ps` output regardless of whether the destination store is
   trusted.
2. **`openbao-destructive-verb`** (`deny`) — the original motivating-gap
   rule, now covering `kv destroy`, `kv metadata delete`, `secrets disable`,
   `auth disable`, `policy delete`, `token revoke`, `operator rekey`, and
   `lease revoke` — a superset of the verb list originally sketched in
   Phase 1 below.
3. **`openbao-kv-get-to-stdout`** (new, `additionalContext`, not `deny`) —
   a non-blocking warning when a `kv get` prints a value to stdout, where it
   enters the transcript the same way an inline literal would. This
   deliberately does **not** follow Phase 1's "skip
   updatedInput/additionalContext complexity for v1" simplification: by
   2026-08-20 the hook front-end's `additionalContext` realization
   (`irrevers-df96952a`) had already shipped independently and closed
   (2026-08-17), so a pack authored after that point had no reason to
   artificially restrict a read-only nudge to `deny`.

Rules 1 and 3 are new surface beyond the pack originally scoped as `vault`.
All three shipped in one commit (`01b5cf1`) the same day `~/CLAUDE.md`'s
"OpenBao — Agent Read/Write Access" section reversed the standing
no-agent-write-access policy and named this pack as the enforcement
mechanism for that section's "never as an argument" / "never to stdout"
requirements — see that section for the policy rationale this pack encodes.
Tracked as `irrevers-b9e97cab` (destructive-verb rule only; the
argv-literal-leak and stdout-warning rules shipped in the same commit
without a dedicated bead of their own — a minor bead-graph gap, not a code
gap).

**`misc` pack's deprecated-bead-CLI rule is data-driven by design, not
hardcoded to one tool name.** `bf` (bead-forge) is currently canonical and
`br` (beads_rust) is deprecated. `bf` is being prepared for deprecation in
favor of `bead-rs` (`~/bead-rs`, binary `bead`, a separate clean-room
reimplementation, not the same lineage as `br` despite the similar name).
The rule's actual policy is "don't invoke a deprecated bead CLI," which
outlives any single tool's canonical status — the pack stores "currently
canonical" and "deprecated" as data (a small list this rule reads), not as
logic hardcoded to `bf` specifically. When the cutover actually happens,
updating that data to make `bead` canonical and `bf` deprecated is a one-line
manifest change, not a rule rewrite. The CLIs are not syntax-compatible, so
the rule denies deprecated invocations and does not translate commands. See
Phase 1 and `irrevers-692a56c3`.

**Deploy location: root-owned system directories, not user-writable
paths.** The three artifacts must live where the guarded agent's own
process (running as `coding`) cannot write them — deployment shape 2 from
`docs/notes/runtime-config-vs-hardcoded.md`, not shape 1. Concretely:
- **Binary:** `/usr/local/bin/icg` — owned by `root:root`, mode `0755`
- **Rule pack artifact:** `/etc/icg/rule-pack.json` — owned by `root:root`,
  mode `0644`
- **Trust pointer:** `/etc/icg/trust-pointer.json` — owned by `root:root`,
  mode `0644`

This placement means the guarded agent can read but not write any of the
three artifacts. Updates require privilege escalation (`sudo icg update`),
which is intentional: self-updating is only safe because release-cutting
is separately human-gated (per `docs/notes/self-update-and-release-gating.md`),
so the update trigger can only adopt *already-vetted* releases, never cause
untrusted content to load. Absent that separate content gate, a
writable-by-the-agent deploy location would reproduce the exact hole
`org-rule-guard.py` has today, just at fleet scale rather than per-host.
The existing `~/.local/bin/cargo` precedent is user-owned because cargo test
offloading is not a security boundary; this project's guard IS, so it gets
the stricter deployment shape.

**Integration point: resolved — both layers, both harnesses**, not a
choice between them. Two independent, complementary front-ends sharing one
engine:
- A **PATH-wrapper binary** shadowing whatever binaries the *currently
  loaded* rule packs cover — `vault`/`bao` (the `openbao` pack) and `git`
  from Phase 1, `docker`
  once Phase 4's `irrevers-54d477dd` pack ships (not before; shadowing a binary with
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

  **Claude Code installation contract:** the user-level
  `~/.claude/settings.json` registers `icg hook` as its own `PreToolUse`
  command entry for `Bash|Write|Edit`, using the absolute production path
  `/usr/local/bin/icg hook`. This entry is added alongside the existing
  `org-rule-guard.py` entry; it must not replace or wrap that hook. Claude
  Code therefore invokes both guards independently during the coexistence
  period, preserving the existing org rules while adding icg's command and
  content coverage. The operator installation procedure documents the
  corresponding merge and verification steps in
  `docs/operators/deployment-guide.md`.

  **Codex CLI installation contract:** the user-level
  `~/.codex/hooks.json` (or the trusted project-local `.codex/hooks.json`)
  registers `icg hook` as its own `PreToolUse` command entry for
  `Bash|apply_patch`, using the absolute production path
  `/usr/local/bin/icg hook`. Installation must merge this entry with any
  existing hook groups rather than replacing the file, and project-local
  hooks require Codex's project trust review before they run. The `Bash`
  matcher covers command-mode checks; `apply_patch` covers Codex's
  content-mode payloads, including multi-file patches. The operator
  installation procedure documents the corresponding merge and verification
  steps in `docs/operators/deployment-guide.md`.

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
  `sudo`/env-assignment/wrapper prefixes), **basename-matches each token
  against `tool_keywords`** (strips directory components before
  comparing, so `/usr/local/bin/vault kv destroy` matches the `openbao` pack
  the same as a bare `vault kv destroy` would), and matches the resulting
  tokens against loaded rule packs — what `openbao`, `git`, `misc`, and
  `tmux` packs use. **`secrets` is the one command-mode pack that does NOT
  dispatch this way**: per Architecture above it scans the entire raw Bash
  command string regardless of which executable is invoked (`echo
  "ghp_..." >> file` has no guarded executable to basename-match), so it
  needs an unconditional whole-command path rather than a `tool_keywords`
  match. This basename-match only helps the *hook*
  front-end (`icg hook`), which sees the full command string regardless of
  how it would resolve — the PATH-wrapper front-end can't be reached at
  all by an absolute-path invocation, since it never goes through `$PATH`
  resolution in the first place; that's a limitation of the wrapper
  specifically, not of command-mode tokenization. See README's "What this
  does not do" for the wrapper-specific version of this caveat. **Content mode**: reads a file path + content (from
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

  **Each channel is realized differently per front-end — same decision,
  different mechanism.** On the hook front-end (`icg hook`), all three map
  directly onto the native JSON response fields (`permissionDecision:
  deny`, `updatedInput`, `additionalContext`) both harnesses accept in
  their schema — though Codex doesn't yet *honor* `additionalContext`
  specifically, same caveat as Phase 3 below; "accepts in schema" and
  "acts on it" aren't the same claim. On the PATH-wrapper front-end
  there's no such protocol — the
  wrapper itself has to implement the equivalent: `deny` means refusing to
  exec the real binary and printing the reason to stderr with a non-zero
  exit; `updatedInput` means rewriting `argv` before exec (e.g. dropping
  `--force`) rather than filling a JSON field; `additionalContext` means
  printing the warning to stderr and still exec'ing normally. Phase 3's
  flagship `updatedInput` example (force-push stripping, a `git`-pack
  command-mode rule) fires on both front-ends, so both realizations need
  to exist together, not just the hook-side one.
- **`icg` binary — one binary, three dispatch modes.** (1) Invoked under
  its own name (`icg update`, `icg status`, `icg new-pack`, ...), it runs
  subcommand dispatch for administrative commands. (2) Invoked under a
  shadowed tool's name (`vault`, `git`, `docker`, ...) — via symlinks
  installed earlier in `$PATH` than the real binaries, the same shape as
  the existing `cargo` precedent — it dispatches on `argv[0]` instead:
  runs the engine's command-mode checks, and if allowed, execs the real
  binary (found further down `$PATH`) with the original arguments
  untouched. This is the PATH-wrapper front-end's actual implementation,
  not a separate component from it. (3) Invoked as `icg hook` by Claude
  Code's/Codex's own `settings.json`/`hooks.json` wiring, it reads
  PreToolUse JSON from stdin instead of argv — the native-hook front-end's
  implementation, and the only mode content-mode packs (`storage-class`,
  `image-tag`) can run under, since a Write/Edit never goes through the
  PATH-wrapper at all.
- **`beads` pack's check** is a conjunction, not a bare predicate: (a) the
  write target is under `.beads/`, **and** (b) `.git` at that repo's root
  is a directory (shared/primary tree — the actual concurrent-corruption
  risk) rather than a file (linked worktree, by construction not shared
  fleet state). Condition (a) is an ordinary path-prefix match — what's
  refined from the originally-proposed design is condition (b), replacing
  a `~/`-boundary heuristic that didn't hold with the precise git-type
  check. Stating this as "not a path block" without the conjunction would
  be a serious over-match: nearly every normal (non-worktree) repo has
  `.git` as a directory, so condition (b) alone would deny writes
  anywhere in almost any repo, not just `.beads/`. See
  `docs/notes/beads-protection-scope.md` for the full reasoning.
- **Self-updater** — user-triggered, not polling (resolved 2026-08-13; see
  Phase 0). **No persistent process to update** — the guard is
  per-invocation (a fresh process per check, per the "no standing daemon"
  architecture decision — see `irrevers-cd3f4c44`'s discussion in Architecture), so
  "hot-swap" doesn't mean an in-memory reload of a resident process.
  Concretely: on trigger, `icg update` checks the GitHub Releases API
  once, downloads the new rule-pack artifact, and atomically replaces the
  on-disk artifact (write-then-rename) — any check process already
  mid-invocation reads a consistent old-*or*-new version, never a
  partially-written one, and every check spawned after the rename picks
  up the new version automatically, with nothing to "restart." No network
  I/O on the guarded-check hot path itself, since the check only happens
  on an explicit trigger, never automatically. `crates.io`'s API as a
  secondary version-check signal if this ships as a published crate. See
  `docs/notes/self-update-and-release-gating.md` for why release-cutting,
  specifically, needs its own human gate separate from routine
  CI-on-push.
- **Value-derivation helpers** — for cases where the correct redirect value is
  programmatically derivable at check-time (e.g. the real semver from
  `containers/<name>/VERSION`), embed it directly in the deny reason rather
  than pointing at where to look. The image-tag pack uses the
  `{derived_value}` placeholder for this today; unavailable values retain an
  actionable path-based fallback.
- **State store** (later phase, not Phase 1) — minimal persistent marker
  needed only for Tier 2 ordering rules. `org-rule-guard.py` has no
  equivalent today; this is new surface.
- **Per-repo override (`irrevers-e354aca2`), "signed" clarified**: not a bespoke
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
- **Tier 1** — stateless, decidable from a single invocation alone:
  command text, a filesystem predicate, or (the one documented exception,
  `irrevers-8cff8cf4`) a single synchronous network check that doesn't depend on
  anything from a prior invocation. "Stateless" is the actual dividing
  line, not "no I/O" — Tier 1 vs. Tier 2 is about whether a check needs
  memory of past invocations, not about what kind of check it runs. What
  Phase 1 ships.
- **Tier 2** — needs state that persists *across* invocations (e.g. "did a
  `git pull` happen earlier in this session"). What Phase 2's state store
  exists for.
- **Tier 3** — not reliably decidable from command syntax at all; the
  same command is legitimate in some contexts and dangerous in others
  (the canonical example: `git worktree add`). Never a `deny` — at most a
  non-blocking, heuristic `additionalContext` warning, and only if ever
  pursued at all (see "Out of scope for now").

Sketch of a rule pack entry — format (YAML/TOML/Rust struct/etc.) not yet
chosen, this is the field set regardless of format:

```
Pack:
  id: string                     # "openbao", "git", "storage-class", "beads", ...
  tool_keywords: [string]        # command-mode packs (openbao/git/misc/tmux):
                                  # executables this pack inspects, e.g. ["bao", "vault"].
                                  # Unused by content-mode packs, by beads, and by
                                  # secrets -- secrets scans the whole command string
                                  # unconditionally, with no tool_keywords match.
  applies_to: [FileGlob]         # content-mode packs (storage-class/image-tag): which
                                  # Write/Edit targets this pack scans, e.g. ["*.yaml", "*.yml"]
                                  # -- mirrors org-rule-guard.py's own .yaml/.yml scoping.
                                  # ALSO used by beads (Predicate-type check) to scope its
                                  # .beads/ path match, even though beads isn't a content-mode
                                  # pack -- see Components for why it's a third, distinct case.
                                  # Unused by pure command-mode packs (openbao/git/secrets/misc/tmux).
  safe_patterns: [Pattern]       # explicitly-allowed shapes, checked first
  guarded_patterns: [GuardedPattern]

Pattern:                         # lighter than GuardedPattern -- no tier/severity/redirect,
  check: CommandRegex | ContentRegex | Predicate
                                  # just a shape that's explicitly allowed and skips the rest
                                  # of the pack's guarded_patterns for that command/write.

GuardedPattern:
  id: string
  enabled: true | false           # whether this rule participates in evaluation;
                                  # omitted in older manifests means true
  check: CommandRegex | ContentRegex | Predicate
                                  # CommandRegex: matched against shell tokens (openbao/git/misc/tmux
                                  # packs, both front-ends; secrets pack also uses CommandRegex but
                                  # is hook-only despite the shared check type — see Architecture
                                  # for why). ContentRegex: matched against Write/Edit file
                                  # content (storage-class/image-tag/beads packs, hook front-end
                                  # only -- beads is content-mode-adjacent, same front-end
                                  # restriction, since Write/Edit never reaches the wrapper).
                                  # Predicate: a general custom-check-function umbrella, NOT
                                  # filesystem-only -- covers beads' filesystem stat (combined
                                  # with a .beads/ path match via applies_to, not the predicate
                                  # alone), irrevers-8cff8cf4's synchronous network lookup (the Tier 1
                                  # exception), and Phase 2's state-store-backed checks alike.
                                  # Predicates may also carry optional pack data; the misc pack's
                                  # deprecated-command check stores its canonical and deprecated
                                  # executable names there.
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
    deny-must-still-fire regression suite plus a structured `coverage-diff/v1`
    report (removed or disabled patterns, widened `safe_patterns`, narrowed
    `guarded_patterns` (especially those where `destructive: true`), with
    previous/current values and an explicit justification field) as required,
    build-failing `icg-ci` gates (Layer 1); human review informed by that
    generated diff report, not
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
    release. On trigger, `icg update` atomically replaces the on-disk
    rule-pack artifact (write-then-rename — see Components' Self-updater
    for why this, not an in-memory reload, is the right framing given the
    per-invocation architecture); nothing to restart, so the host being
    updated never blocks its own guarded agent sessions while updating.
    No fleet-wide
    synchronization point — triggering one host doesn't require pausing
    or waiting on any other host, consistent with the already-adopted
    canary-rollout design (`irrevers-6de781f4`). This is asymmetric with
    `irrevers-ff4f17da`'s poison-pill auto-rollback by design: adopting a new
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
- [ ] **Phase 1 — Tier 1 rules, both front-ends where applicable,
      deny-only.** Build the PATH-wrapper binary and both hook adapters
      (Claude Code, Codex). Most Phase 1 rules run on both front-ends;
      `secrets` and the image-tag/storage-class content-mode rules are
      hook-only, per Architecture — not every rule reaches the wrapper.
      Rule set: Bash-channel secret-value scanning (reuse
      `org-rule-guard.py`'s existing regex machinery, currently only wired
      to the Write/Edit path), OpenBao destructive verbs (the core
      motivating gap — shipped as the `openbao` pack; see Architecture's
      "`openbao` pack, scope resolved and expanded" paragraph for the actual
      shipped scope, which grew beyond destructive verbs alone),
      `ssd`/`ssd-large` storage class, **both halves**
      of image-tag pinning (`:latest` re-detection *and* bare-SHA — the
      `image-tag` pack fully absorbs this rule from `org-rule-guard.py` as
      of Phase 1, not just the bare-SHA gap; see Architecture and
      `docs/notes/existing-enforcement-infrastructure.md`), force-push,
      stale-HEAD-before-push (`irrevers-8cff8cf4` — the one Tier 1 rule with a live
      remote check, a deliberate, scoped exception to the engine's
      zero-*network*-I/O rule since `git push` is already a network
      operation; see Architecture), **commit-without-pathspec** (added
      2026-08-14, `irrevers-57af0680`). Denies `git commit -a`/`--all` and any bare
      `git commit -m "..."` with no trailing pathspec — the command commits
      the *entire* staged index, not just what the agent's own `git add`
      just staged, so a precisely-scoped `git add` (already required by
      CLAUDE.md) can still be defeated by an imprecise `git commit`.
      Root-caused live 2026-08-14 (commitgraph, worker
      `claude-code-glm-5-adr018`, bead `cg-194i4a`): a correctly-scoped
      `git add <2 files>` followed by a bare `git commit -m` produced a
      commit also containing ~430 unrelated lines from another concurrent
      NEEDLE worker's pre-staged, uncommitted files in the same shared
      checkout; the worker self-corrected before pushing, but only because
      it happened to check its own diff — this rule makes that check
      unconditional rather than dependent on a worker's diligence. Applies
      **globally, not scoped to known-shared-checkout repos** (explicit
      pathspecs on `git commit` cost nothing in an uncontended repo), unlike
      the narrower `.beads/`-protection conjunction check below. Deny-only,
      no `updatedInput` — the safe replacement pathspec isn't derivable from
      the `git commit` call's own text without Tier 2 state (see
      `docs/notes/redirect-not-just-block.md`). `.beads/` protection
      (path-under-`.beads/` **and**
      `.git` file-vs-directory check — see Components, both conditions
      required),
      deprecated-bead-CLI usage (data-driven, `irrevers-692a56c3` — `bf` is
      currently canonical and `br` is deprecated; when the cutover actually
      happens, update the pack data to make `bead` canonical and add `bf` to
      the deprecated list), `needle cleanup`,
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
      happens first — see `irrevers-692a56c3`, don't assume the two are
      interchangeable here).
- [ ] **Phase 3 — redirect-mechanism richness.** Introduce `updatedInput`
      for confirmed intent-preserving cases (force-push flag stripping is
      the clearest candidate) and `additionalContext` for non-blocking
      warnings; extend value-derivation helpers to additional rules as
      needed. (Ideation finalist #10 proposed
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
      original finalists adopted, tracked as beads, with the poison-pill
      measurement split tracked alongside them; full
      dossiers and kill-pass objections in `docs/notes/ideas-ledger.md`.
      Deepens Phase 0's release-integrity/self-update work and Phase 1's
      rule coverage rather than opening new phases of its own:
      - `irrevers-aa1b828d` — auto-denial-becomes-test (strengthens Layer 1; needs a
        curation step so the suite doesn't grow unbounded)
      - `irrevers-54b33e0c` — `icg new-pack` scaffolding tool
      - `irrevers-ff4f17da` — poison-pill auto-rollback (extends Phase 0's Layer 4)
      - `irrevers-b6579270` — per-release deny-rate telemetry and a rolling
        baseline in the durable state store. This is the measurement half of
        the poison-pill split: it records release denominators and denials
        across hook/wrapper invocations and exposes a conservative deviation
        signal to rollback and guard-maturity consumers.
      - `irrevers-6de781f4` — canary rollout via NEEDLE `--identifier` (concrete
        Layer 4 staged-rollout implementation)
      - `irrevers-1cad33d2` — `icg status` with blind-spot self-report
      - `irrevers-8b5faeb9` — Codex hook-version compatibility matrix in `icg-ci`
      - `irrevers-e354aca2` — per-repo signed override (routed through Layer 1/2)
      - `irrevers-195d05cc` — practice/dry-run mode (ships only with the mandatory
        persistent active-indicator the kill pass required). Near-miss
        feedback deliberately does **not** rely on `additionalContext` —
        given that channel isn't honored on Codex yet (see Phase 3), a
        practice-mode report needs to reach both harnesses identically;
        the persistent banner requirement already covers this, surfaced
        directly rather than through a hook-response field either harness
        might drop.
      - `irrevers-54d477dd` — Docker destructive-ops pack (new Phase-1-shaped pack,
        same architecture as `openbao`)
- [ ] **Phase 5 — from ideation (2026-08-13 second `/plan-idea-gen` run).**
      Like Phase 4, this isn't a new sequential build phase — its
      findings fold into earlier phases' actual scope (`irrevers-8cff8cf4` is a
      Phase 1 rule; `irrevers-cd3f4c44` is discussed under Architecture's fail-open
      policy; both are listed here only because this is where their
      ideation provenance and bead IDs are tracked). Six finalists adopted
      as beads, one (explicit README non-goals) done directly rather than
      tracked as a bead. Full dossiers and kill-pass objections in
      `docs/notes/ideas-ledger.md`'s second-run section:
      - `irrevers-36244640` — guard CI/build pods on iad-ci, including this
        project's own `icg-ci` release pipeline
      - `irrevers-8cff8cf4` — stale-HEAD push guard, the shipped form of ledger
        finalist #2 ("shared-tree collision protection") after user
        revision (compares tracked vs. actual remote HEAD before
        `git push`, a simpler mechanism than the originally-proposed
        cross-process `/proc` scanning — a deliberate, scoped exception
        to the zero-*network*-I/O rule, since `git push` is already a
        network operation)
      - `irrevers-cd3f4c44` — graduated fail-open→fail-closed policy for guard
        crashes: fails open until the guard's reliability is validated
        (consuming the durable per-release deviation exposed by
        `irrevers-b6579270`, alongside the signal
        `irrevers-ff4f17da`'s poison pill consumes), then shifts to fail-closed
      - `irrevers-b8343704` — ReDoS check on submitted rule packs in `icg-ci`
      - `irrevers-012be0c8` — per-rule enable/disable feature flag, revised from a
        dedicated fast-path kill-switch to reuse the normal Layer 1/2
        release pipeline (tradeoff: no longer sub-release-cycle-fast —
        flagged as a real, unresolved gap if true emergency speed is ever
        needed)
      - `irrevers-62c6f748` — install-time smoke test vs. `org-rule-guard.py`,
        framed as an interim check pending that hook's eventual
        deprecation (see Overview). **Success criterion, precise**: during
        coexistence, icg's `image-tag` pack and `org-rule-guard.py`'s rule
        3 will both independently fire on the same `:latest` violation —
        that's expected, redundant-but-harmless double-deny, not a
        failure. The test passes on consistent verdicts (both deny, or
        both allow) and only fails on a *divergent* verdict (one denies,
        the other doesn't) — that's the actual conflict worth catching.
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
- ~~Value-derivation helpers' Phase 1 inclusion — known tradeoff, not an
  oversight.~~ — **resolved 2026-08-17.** The image-tag rules now replace
  `{derived_value}` with the semver read from the matching
  `containers/<name>/VERSION` file, with an actionable fallback when that
  value cannot be read. The helper can be extended for future derivable
  redirects.
- ~~`beads`-in-`bf` question~~ — **resolved 2026-08-13: stays in this
  project.** With `bf` itself now confirmed heading toward deprecation
  (see the Architecture section and `irrevers-692a56c3`), embedding the `.beads/`
  protection check inside a tool that's about to be superseded would just
  mean redoing this work again at the next cutover. Phase 1 already
  unconditionally depends on this answer, so leaving it formally open any
  longer served no purpose — the deprecation news settles it.
