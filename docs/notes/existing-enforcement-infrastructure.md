# Existing enforcement infrastructure

What's already running in this environment, verified against the actual
source rather than CLAUDE.md's description of it, as the starting point for
this project's scope.

## Hook wiring

`~/.claude/settings.json` registers exactly one `PreToolUse` hook that
enforces behavioral rules: `org-rule-guard.py`, matched on
`Write|Edit|Bash`. Every other hook in this environment (`.ccdash/hooks/*`,
`trail-boss/.claude/trailboss-emit.sh`, `zellij-tab-suffix.sh`,
`herdr-agent-state.sh`, `clear-update-check.sh`) is observability, session
state, or UI plumbing — none of it blocks or alters agent behavior. This
project's predecessor is a single file.

## What `org-rule-guard.py` actually does

Verified by reading the source directly (`~/.claude/hooks/org-rule-guard.py`,
204 lines), not CLAUDE.md's summary of it:

- **Fails open by design**, explicitly: any unparseable input or internal
  exception exits 0 (allow). Rationale stated in its own docstring — "a
  NEEDLE fleet must never be wedged by this hook — a missed violation is
  recoverable, a stuck fleet is not."
- **Five rules total**, all hardcoded as module-level regex/set constants
  in one Python file, no config file, no plugin architecture:
  1. No `.github/workflows/*` writes.
  2. No `kind: Job`/`kind: CronJob` in `.yaml`/`.yml` writes (real manifest
     lines only, via a line-anchored regex — comments don't trip it).
  3. No `:latest` image tags in `.yaml`/`.yml` writes.
  4. No mutating `kubectl` verbs in `Bash` calls — a hardcoded `MUTATING`
     set (`apply`, `delete`, `patch`, `edit`, `replace`, `set`, `annotate`,
     `label`, `scale`, `autoscale`, `cordon`, `uncordon`, `drain`, `taint`,
     `evict`, `rollout`, `create`), with carve-outs for `rollout
     status`/`history` and for `kubectl create` of an Argo Workflow.
  5. No committed credential *values* — five hardcoded token-shape regexes
     (GitHub token/PAT, AWS access key, Slack token, Anthropic API key, PEM
     private key header), with placeholder detection (`example`, `your`,
     `changeme`, all-one-character bodies) and a `gitleaks:allow` escape
     hatch.
- Rule 5 only fires on the `Write`/`Edit` path (`check_write` →
  `check_secrets`). **The `Bash` path never calls `check_secrets` at all** —
  only `check_bash`, which checks kubectl verbs and nothing else. A
  credential value written via `Bash` (`echo "ghp_..." >> notes.md`, a
  heredoc, a `curl -d` body) is invisible to this hook entirely.

## Coverage gap analysis

Cross-referencing the hook's five rules against CLAUDE.md's "Hard
prohibitions" section (eleven items):

| CLAUDE.md prohibition | Mechanically enforced? |
|---|---|
| No `.github/workflows/*` | **Yes** |
| No `kind: Job`/`CronJob` | **Yes** |
| No mutating `kubectl` on ArgoCD-managed resources | **Yes, but broader than the policy** — blocks *all* mutating kubectl, including the policy's own stated exception for genuinely-orphaned non-ArgoCD-owned objects. The hook can't distinguish ownership, so it's stricter than CLAUDE.md in this one direction. |
| No `:latest` / bare git SHA for `ronaldraygun/*` images | **Partial** — catches `:latest` anywhere (not scoped to `ronaldraygun/*` specifically, which is fine, broader is safe here), but has **no check at all** for bare-SHA image pinning, the other half of that rule. |
| No `ssd`/`ssd-large` on Rackspace Spot | **No** |
| No force-push | **No** |
| No hand-editing `.beads/`, `bf sync --flush-only` before pull, `bf` not `br` | **No** |
| No `needle cleanup` | **No** |
| No touching bare NATO tmux sessions | **No** |
| No per-worker git worktrees | **No** (arguably not mechanically detectable as a single command anyway — it's a workflow pattern, not one invocation) |
| No committed credential values | **Yes on Write/Edit, no on Bash** (see above) |

**Zero coverage for HashiCorp Vault/OpenBao destructive operations** —
`vault kv destroy`, `bao secrets disable`, `vault policy delete`, token/lease
revocation. This is the specific gap that started this project (see
`docs/research/prior-art.md` for how `destructive_command_guard`'s Vault
pack handles exactly this).

## How to apply

This project should not duplicate rules 1–5 above — those stay
`org-rule-guard.py`'s job. The open scope is: (a) the Bash-channel secret
gap, (b) Vault/OpenBao destructive verbs, (c) whichever of the
"No" rows in the table above are worth mechanical enforcement rather than
documentation-only trust. Not all of them necessarily are — see
`docs/plan/plan.md`'s open questions on scope.
