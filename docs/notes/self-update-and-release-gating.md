# Self-update, hot-reload, and why release-gating is the part that matters

Resolves the plan's "deploy mechanism specifics" open question: the
deployed guard (both the PATH-wrapper and the hook adapters) checks for new
releases and hot-reloads without a human re-running a deploy step by hand
on every host.

## Distribution mechanism

**GitHub Releases, built via an Argo WorkflowTemplate — not GitHub
Actions.** This environment already has four working precedents for
exactly this "Rust binary → GitHub Release" pipeline, all running on
iad-ci: `forge-ci` (jedarden/forge), `needle-ci` (jedarden/NEEDLE),
`agentscribe-ci` (jedarden/AgentScribe), `sigil-ci` (jedarden/SIGIL). This
project should get an analogous WorkflowTemplate (`icg-ci` or similar) in
`declarative-config/k8s/iad-ci/argo-workflows/` — a proven pattern, not a
new one, and it keeps this project inside the hard prohibition on
`.github/workflows/*`.

**`crates.io` as a secondary version-check channel**, if this ships as a
published crate. `crates.io`'s API is a lightweight, purpose-built way to
ask "is there a newer version" (the same thing `cargo outdated`/`cargo
update` already do under the hood) without hitting GitHub's stricter
unauthenticated rate limits. Actual artifact distribution (the guard
binary itself) still comes from GitHub Releases; `crates.io` is a version
*signal*, and — if `cargo install irreversible-command-gate` is ever
offered as an install path — a second distribution channel, not required
for the self-update loop itself.

## The part that isn't automatically safe: what triggers a release

Self-updating from "whatever the latest GitHub release says" only
preserves the tamper-resistance property from `runtime-config-vs-hardcoded.md`
if **creating that release** requires something the guarded agent's normal
operation doesn't have. If it doesn't, this mechanism just relocates the
exact hole it's meant to close, one layer removed:

- If a release auto-publishes on every push to `main` (a real pattern
  elsewhere in this environment — `seam-ci` runs on push, for instance),
  then an agent session with ordinary push access to this repo — which,
  per this environment's own git policy, is pre-approved for reversible
  changes — could push a weakened rule pack and have every guard instance
  that self-updates pull it in as the new trusted version. That is a
  materially worse outcome than the current `org-rule-guard.py` self-edit
  gap, because it propagates to every deployed instance automatically
  instead of requiring per-host tampering.
- **CI building and testing on every push is fine and should happen** —
  that's normal continuous integration, not a trust decision. **Cutting a
  release is a distinct, separate action** and needs its own gate: a human
  runs the actual `gh release create` (or tags a specific reviewed commit),
  not something that fires automatically off routine push activity. This
  mirrors the distinction this project already draws elsewhere between
  "agent can push commits" (fine, reversible) and "agent can take an
  action that becomes trusted security-rule truth for every deployed
  instance" (needs a human in the loop).

This isn't optional hardening — it's the difference between the self-update
mechanism actually satisfying `runtime-config-vs-hardcoded.md`'s deploy
constraint (shape 3: "pull-based, agent doesn't control the reload") and
merely looking like it does while leaving the exact same hole open at
fleet scale.

## Hot-reload — resolved 2026-08-13: user-triggered, not polling

Not an automatic background poll on an interval. An operator explicitly
triggers an update (e.g. an `icg update` command) when they want a
specific host to pick up a new release — at that point the guard checks
the GitHub Releases API once and atomically replaces the on-disk rule-pack
artifact (write-then-rename). There's no persistent guard process to
restart in the first place — the architecture is per-invocation (a fresh
process per check, per the "no standing daemon" decision plan.md's
Architecture section makes for `irrevers-cd3f4c44`) — so "hot-swap" means the next
spawned check simply reads the new artifact, not an in-memory reload of a
resident process. No fleet-wide synchronization point
either: triggering one host doesn't require pausing or waiting on any
other host, which is what makes the already-adopted canary-rollout design
(`irrevers-6de781f4`) actually work as "one host first, the rest later" rather than
an all-or-nothing flip. This also means there's no network I/O on the
guarded-check hot path at all under normal operation — the only time the
guard talks to GitHub is the moment a human explicitly asks it to.

Worth noting the asymmetry this creates with `irrevers-ff4f17da`'s poison-pill
auto-rollback: adopting a new release forward is deliberate and manual,
but *reverting* an already-adopted bad one stays fully automatic. That's
intentional, not an inconsistency — the two directions carry different
risk (a human choosing when to move forward vs. a machine needing to react
immediately when something already live turns out to be bad).

## How to apply

Phase 0 (deploy path) now has three concrete sub-requirements: build the
`icg-ci` WorkflowTemplate for the build/release pipeline; make sure
release-cutting specifically — not just pushing to `main` — is the human-
gated step (resolved: manual `gh release create` once `icg-ci` passes, no
additional approval-workflow layer, since Layers 1/2/4 already provide
sufficient protection); and implement the self-updater as user-triggered
rather than polling. Don't let "we have CI" stand in for "releases are
gated"; they're different claims.
