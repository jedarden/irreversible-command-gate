# `.beads/` protection scope — validated, refined from the original proposal

Resolves the plan's Open Question 3 ("does `.beads/` path-blocking belong
in this project"). Answer: yes, enforce it — but the proposed refinement
(scope enforcement to `~/` and its subfolders, on the theory that a
worktree outside home is deliberate debugging/triage work) doesn't hold up
against what's actually recorded about how worktrees get used in this
environment. A different, more precise signal does the same job correctly.

## What's confirmed

- `reference_declarative_config_history_rewrites.md` documents a real,
  sanctioned pattern: when `declarative-config`'s shared tree is dirty with
  other agents' `.beads` churn, the correct move is `git worktree add
  --detach <dir> origin/main`, do the work there, push, and "leave the main
  tree alone — never hand-resolve `.beads` conflicts" in the shared tree.
  So the underlying intuition is right: `.beads/` work in a *secondary*
  worktree, away from the fleet's shared tree, is the sanctioned pattern,
  not a violation.
- `feedback_needle_shared_worktree.md` and
  `feedback_needle_repo_live_edit_collision.md` confirm the actual danger
  this rule should guard: NEEDLE workers share **one working tree per
  repo**, and concurrent `git commit`/`pull --rebase`/`reset` in that one
  shared tree can silently discard another actor's in-flight changes. The
  risk lives in the shared tree specifically, not in worktrees generally.

## Where the proposed `~/`-boundary heuristic doesn't hold

Checked directly rather than assumed: `~/scratch/` is real and actively
used, and this very session's own scratch space
(`/home/coding/.tmp/claude-*/.../scratchpad`) is also under `/home/coding`.
Nothing found — in memory or in this session's own tool use — suggests
throwaway/triage worktrees are conventionally placed *outside* `$HOME`.
Every documented example of scratch, throwaway, or triage work on this box
happens under `~/` too. A rule that only enforces `.beads/` protection
"within `~/` and its subfolders" would not actually exempt the sanctioned
throwaway-worktree pattern from `reference_declarative_config_history_rewrites.md`
— that pattern's own `<dir>` would very plausibly land under `~/scratch/`
or similar, still inside the proposed enforcement boundary. The heuristic
doesn't distinguish the case it's meant to distinguish.

## The signal that actually works: is this the shared tree, or a linked worktree

Git itself already marks the difference, mechanically and reliably: a
repository's primary checkout has `.git` as a **directory**; a linked
worktree created via `git worktree add` has `.git` as a **file** containing
a `gitdir:` pointer back to the main repo's `.git/worktrees/<name>` admin
directory. This is standard git internals, not a convention that depends on
where anyone chooses to put things — `test -d "$repo/.git"` (shared/primary
tree) vs. `test -f "$repo/.git"` (linked/secondary worktree) distinguishes
them regardless of filesystem location, inside or outside home.

**This is the actual risk boundary**: the danger `feedback_needle_shared_worktree.md`
describes only exists in the primary/shared tree, where concurrent fleet
workers operate. A linked worktree is by construction not that tree — no
NEEDLE worker treats it as shared state, because NEEDLE doesn't create
per-worker worktrees to begin with (that's the separate, already-settled
prohibition). So gating `.beads/` protection on "is `.git` here a directory"
rather than "is this path under `~/`" directly targets the real risk,
using a signal that's cheap to check (one `stat` call) and doesn't depend
on filesystem-location conventions this environment doesn't actually
follow.

## Rule, refined

Block `.beads/` hand-edits when the target path's repository root has a
**directory** `.git` (the shared/primary tree — where concurrent-worker
corruption risk actually lives). Don't block `.beads/` edits where the
repository root has a **file** `.git` (a linked worktree — by construction
not the tree any NEEDLE worker is concurrently operating in). No dependency
on `~/` vs. elsewhere.

## How to apply

Update the Phase 1 `.beads` pack spec to check `.git`'s file-vs-directory
type at the repo root, not the write path's location relative to `$HOME`.
