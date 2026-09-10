# Closed beads: release verification / fail-closed harness behavior

Plain enumeration only (per irrevers-622aae24). No evidence gathering, no
analysis, no edits to docs/plan/plan.md. Dates are the bead's `updated_at`
timestamp at close (bead-rs does not track a separate close timestamp).

Anchor bead confirmed present: **irrevers-84b36e47**.

## Release verification / release process / distribution integrity

| ID | Title | Close date |
|---|---|---|
| irrevers-84b36e47 | Verify icg-ci produces a real, complete GitHub release | 2026-09-06T13:06:27Z |
| irrevers-e77615c8 | icg-ci: publish the rule-pack artifact as a release asset | 2026-08-24T04:27:11Z |
| irrevers-37eb1100 | Release-cutting runbook | 2026-08-15T13:57:16Z |
| irrevers-340ae322 | Prove ronaldraygun/argo-guarded-builder:0.1.0 is published and pullable by icg-ci | 2026-08-30T01:56:20Z |
| irrevers-e2bb8fbf | Add --channel to icg trust to match icg update (canary rollout) | 2026-08-30T12:19:35Z |
| irrevers-6de781f4 | Canary rollout via NEEDLE --identifier | 2026-08-15T05:16:35Z |
| irrevers-b6579270 | Per-release deny-rate telemetry and rolling baseline | 2026-08-21T00:54:24Z |
| irrevers-eff8909f | Write inventory of shipped releases v0.1.0-v0.1.6 with dates and commits | 2026-09-10T10:30:14Z |
| irrevers-2cb3dbd2 | Gate the actual modular release packs in icg-ci instead of static fixtures | 2026-08-26T04:03:36Z |
| irrevers-c87a3c50 | Base self-updater and trust pointer (icg update) | 2026-08-14T14:37:11Z |
| irrevers-5fdc2e13 | Trust pointer mechanism | 2026-08-15T03:04:33Z |
| irrevers-f59f9313 | icg update: self-updater command | 2026-08-15T03:26:55Z |
| irrevers-96594031 | Migrate the shipped trust-pointer and rule-pack artifact paths off agent-writable locations | 2026-08-26T01:39:15Z |
| irrevers-ca79d63a | Deploy the binary, rule-pack artifact and trust pointer outside the guarded agent's writable filesystem | 2026-08-23T01:16:06Z |
| irrevers-e00a5381 | icg-ci Argo WorkflowTemplate | 2026-08-15T02:02:17Z |
| irrevers-075634b8 | Make icg update atomically deploy the complete modular production pack directory | 2026-08-26T02:55:13Z |

## Fail-closed harness / policy enforcement behavior

| ID | Title | Close date |
|---|---|---|
| irrevers-8d2d4a73 | Engine: unconditional fail-open on parse failure or exception | 2026-08-15T13:22:12Z |
| irrevers-aab3854c | Design fail-closed transition state machine and graduation criteria | 2026-08-16T03:03:17Z |
| irrevers-cd3f4c44 | Graduated fail-open to fail-closed policy for guard crashes | 2026-08-21T01:25:46Z |
| irrevers-8a24ad8d | Implement fail-open baseline and fail-closed enforcement modes | 2026-08-21T02:44:22Z |
| irrevers-019c36d3 | Implement fail-closed policy transition mechanism | 2026-08-21T03:08:53Z |
| irrevers-fffef435 | Integrate with poison-pill mechanism for automatic graduation | 2026-08-21T02:53:17Z |
| irrevers-0f49129d | Poison-pill auto-rollback | 2026-08-15T03:11:18Z |
| irrevers-ff4f17da | Poison-pill auto-rollback: revert the trust pointer on a deny-rate spike | 2026-08-21T01:05:43Z |
| irrevers-3fc4bdde | Apply the documented ICG_DISABLED emergency bypass to hook and PATH-wrapper enforcement | 2026-08-26T02:19:37Z |
| irrevers-f891f555 | Make the fail-closed policy read path lock-free for guarded invocations | 2026-09-08T02:06:43Z |
| irrevers-edb5c4ca | Stop reconciling the fail-closed policy from the hook and wrapper paths | 2026-09-08T02:07:01Z |
| irrevers-9eb4de16 | Add regression tests for a root-owned policy directory on the hook path | 2026-09-08T02:07:02Z |
| irrevers-93baa29a | Verify icg policy status and reconcile as root on an installed host layout | 2026-09-08T03:21:52Z |
| irrevers-3e6c6fde | Hook and wrapper guarded paths take the fail-closed policy lock via crash recovery | 2026-09-08T05:37:51Z |
| irrevers-50077acb | Set explicit root-owned 0755 modes on the icg trust directories in argo-guarded-builder | 2026-09-08T05:42:49Z |
| irrevers-ffdc924b | Add guard health tracking and crash monitoring infrastructure | 2026-08-21T02:33:11Z |
| irrevers-9007792b | Operational monitoring and alerting infrastructure | 2026-08-21T03:28:12Z |
| irrevers-0d710c9a | Write activation documentation and operational runbooks | 2026-08-21T03:06:08Z |
| irrevers-1517a263 | cargo test writes into the production denial log on an instrumented host | 2026-09-07T20:31:16Z |
| irrevers-0aa08f4e | Routine run_started lifecycle telemetry prints to stderr on every guarded invocation | 2026-09-08T02:10:01Z |

## CI gate / test-harness infrastructure (regression-suite, coverage-diff gates in icg-ci)

| ID | Title | Close date |
|---|---|---|
| irrevers-b4b37bf0 | Layer 1: regression-suite CI gate | 2026-08-15T03:40:26Z |
| irrevers-b0a453b2 | Layer 1: verify the regression-suite gate actually fails the build | 2026-08-15T13:33:59Z |
| irrevers-f61efd80 | Layer 1: coverage-diff CI gate | 2026-08-15T04:05:57Z |
| irrevers-29a9131c | Layer 1: verify the coverage-diff gate actually blocks an unjustified change | 2026-08-15T13:48:19Z |
| irrevers-ed77224f | End-to-end integration testing for icg-ci workflow | 2026-08-16T19:07:41Z |

Excluded as noise: bot-generated "Starvation alert" / "[Unravel]" beads, duplicate
"Genesis: irreversible-command-gate Implementation" beads, generic scenario/example
coverage housekeeping, and one-off test/smoke beads unrelated to release or
fail-closed behavior (full closed-bead JSON dump used for this pass is at
`/tmp/claude-1000/-home-coding-irreversible-command-gate/8dc8251b-caf0-4c28-b112-7a857916e7e5/scratchpad/closed_beads.jsonl`,
session-scoped and not committed).
