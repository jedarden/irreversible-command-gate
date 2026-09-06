# Documentation map

Everything under `docs/` in one place, grouped by what you are trying to do.
If you only read one file, read [quick-start.md](quick-start.md).

## Start here

| File | What it is |
| --- | --- |
| [quick-start.md](quick-start.md) | Install, configure the hook, smoke-test, read a verdict. The full coverage table lives here. |
| [onboarding-guide.md](onboarding-guide.md) | A sequenced learning path with operator and developer tracks. |
| [examples/README.md](examples/README.md) | Twelve worked scenarios, from first install to debugging a false positive. |

## Operating it

| File | What it is |
| --- | --- |
| [operators/README.md](operators/README.md) | Index of the operator surface: commands, deployment model, release safety. |
| [operators/deployment-guide.md](operators/deployment-guide.md) | The canonical install, ownership, and hook-registration contract. |
| [operators/deny-messages.md](operators/deny-messages.md) | How to read a denial and what each field means. |
| [operators/troubleshooting.md](operators/troubleshooting.md) | Installation and hook failures. |
| [operators/practice-mode.md](operators/practice-mode.md) | Observe would-be denials without blocking anything. |
| [operators/fail-closed-mode.md](operators/fail-closed-mode.md) | The graduated fail-open → fail-closed availability policy. |
| [operators/migration-from-org-rule-guard.md](operators/migration-from-org-rule-guard.md) | Cutover from the existing Python hook. |
| [operators/training-manual.md](operators/training-manual.md) | Long-form operator training. |
| [operators/argo-integration-guide.md](operators/argo-integration-guide.md) | Guarding CI workflow steps. |

## Runbooks

Short, ordered procedures for a moment when something is on fire.

- [runbooks/incident-response.md](runbooks/incident-response.md)
- [runbooks/emergency-bypass.md](runbooks/emergency-bypass.md) — `ICG_DISABLED=1`, and its audit trail
- [runbooks/rollback.md](runbooks/rollback.md)
- [runbooks/release-cutting.md](runbooks/release-cutting.md)
- [runbooks/rule-pack-updates.md](runbooks/rule-pack-updates.md)
- [runbooks/override-approval.md](runbooks/override-approval.md)

## Extending it

| File | What it is |
| --- | --- |
| [developers/README.md](developers/README.md) | Developer index. |
| [developers/rule-pack-best-practices.md](developers/rule-pack-best-practices.md) | Pack authoring: regex discipline, safe patterns, testing, release gating. |
| [notes/pretooluse-response-schema.md](notes/pretooluse-response-schema.md) | The hook wire format. |
| [notes/redirect-not-just-block.md](notes/redirect-not-just-block.md) | Why every rule owes the caller an alternative. |
| [notes/per-repo-overrides.md](notes/per-repo-overrides.md) | The release-bound per-repository override contract. |

## Design and decisions

| File | What it is |
| --- | --- |
| [plan/plan.md](plan/plan.md) | The complete plan — architecture, phases, and the decisions behind them. |
| [notes/ideas-ledger.md](notes/ideas-ledger.md) | Two rounds of 100 ideas, with the kill reasons. Read this before proposing a feature. |
| [notes/existing-enforcement-infrastructure.md](notes/existing-enforcement-infrastructure.md) | The coverage gap this project starts from, and what stays with the org hook. |
| [notes/multi-harness-integration.md](notes/multi-harness-integration.md) | Claude Code and Codex CLI hook surfaces; the cloud-session gap. |
| [notes/fail-closed-policy.md](notes/fail-closed-policy.md) · [design/fail-closed-transition.md](design/fail-closed-transition.md) | Availability policy design. |
| [notes/runtime-config-vs-hardcoded.md](notes/runtime-config-vs-hardcoded.md) | Why policy is data, and what that costs. |
| [notes/release-integrity-verification.md](notes/release-integrity-verification.md) · [notes/self-update-and-release-gating.md](notes/self-update-and-release-gating.md) | Trust pointers, signed releases, auto-rollback. |
| [notes/beads-protection-scope.md](notes/beads-protection-scope.md) · [notes/auto-denial-regression-corpus.md](notes/auto-denial-regression-corpus.md) · [notes/force-push-updatedinput-example.md](notes/force-push-updatedinput-example.md) | Scoped design notes. |
| [research/prior-art.md](research/prior-art.md) | What existed already and why none of it was forked. |

## Assets

[assets/](assets/) holds the README's demo GIF and flow diagram, plus
[`assets/demo.sh`](assets/demo.sh) and [`assets/demo.tape`](assets/demo.tape)
— the reproducible source for the recording. Nothing in the GIF is staged;
regenerate it with `vhs docs/assets/demo.tape`.

## Not about the command gate

These document the bead-store maintenance services that also live in this
repository. They are unrelated to command interception; see the
"Unrelated subsystem" note in [AGENTS.md](../AGENTS.md).

- [bead-starvation-automation.md](bead-starvation-automation.md)
- [bead-starvation-repair-system.md](bead-starvation-repair-system.md)
- [bead-dependency-validator.md](bead-dependency-validator.md)
- [cascading-repair-strategies.md](cascading-repair-strategies.md)
- [checkpoint-verification.md](checkpoint-verification.md)
- [monitoring-deployment-guide.md](monitoring-deployment-guide.md)
