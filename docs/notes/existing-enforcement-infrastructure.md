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

The original framing here — "this project should not duplicate rules 1–5" —
assumed indefinite coexistence with `org-rule-guard.py`. That's stale: per
`docs/plan/plan.md`'s Overview (2026-08-13 direction, since refined), that
hook is expected to shrink toward deprecation as this project's coverage
supersedes it — but not to full removal. Its kubectl-mutation rule is
*permanently* excluded from absorption, and its `.github/workflows`/`kind:
Job`/`CronJob` rules (1–2 below) aren't excluded on principle, just not yet
scheduled by any phase — see plan.md's Overview for the precise current
wording, which this note only paraphrases and shouldn't be treated as a
substitute for. Coexistence is an interim state for whichever rules *do*
get scheduled, not a permanent policy for all five. The plan already
absorbs pieces of the five rules above rather than avoiding them:

- **Rule 3 (`:latest` image tags)** — Phase 1's `image-tag` pack extends the
  existing `:latest` check with the bare-SHA half of the same policy
  (plan.md's Architecture section), so as of Phase 1 this project's own pack
  covers both halves of rule 3, not just the gap.
- **Rule 5 (credential values)** — Phase 1 reuses `org-rule-guard.py`'s own
  secret-scanning regex machinery to close exactly the Bash-channel gap
  documented above (rule 5 only fires on the Write/Edit path today). The
  Write/Edit path itself isn't moving yet.
- **Rule 4 (mutating `kubectl`)** — deliberately excluded, not scheduled for
  absorption at all: plan.md's "Explicitly not attempted" phase item rules
  out narrowing the blanket kubectl-mutation block, because doing so
  accurately needs live cluster state that would break this project's
  zero-I/O determinism. This rule stays `org-rule-guard.py`'s alone even
  after the others are absorbed.
- **Rules 1–2 (`.github/workflows`, `kind: Job`/`CronJob`)** — no phase in
  plan.md picks these up yet. They remain solely `org-rule-guard.py`'s job
  for now — not because of a standing "don't duplicate" policy, but simply
  because nothing has scheduled the absorption.

`icg-53q` (Phase 5) is the concrete marker of the trajectory: an
install-time smoke test against `org-rule-guard.py`, explicitly framed as
"interim... pending that hook's eventual deprecation," not as a permanent
coexistence check. Beyond rule absorption, the open scope is still (a)
Vault/OpenBao destructive verbs (the original motivating gap) and (b)
whichever other "No" rows in the table above are worth mechanical
enforcement rather than documentation-only trust — see `docs/plan/plan.md`'s
open questions on scope.
