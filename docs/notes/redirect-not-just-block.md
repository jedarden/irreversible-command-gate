# Redirect, not just block

Stated project objective: a blocked command should leave the agent knowing
exactly what to do instead, in one follow-up step — not just knowing that
what it tried was wrong.

## The existing precedent is already half right

`org-rule-guard.py`'s five `deny()` messages already do this reasonably
well — every one of them names the sanctioned alternative, not just the
violation:

- `.github/workflows` → "All CI runs on Argo Workflows in the iad-ci
  cluster; templates live in declarative-config/k8s/iad-ci/argo-workflows/."
- `Job`/`CronJob` → "Use a Deployment with an internal scheduling loop for
  recurring work, or an Argo WorkflowTemplate for one-shot work."
- mutating `kubectl` → "Change the manifest in jedarden/declarative-config,
  commit, push, and let ArgoCD sync."
- credential values → "write the OpenBao path (secret/<cluster>/<app>/<key>)
  or the command that fetches it (e.g. `gh auth token`) instead."

This wasn't an accident of good writing — it's the actual mechanism working
as designed. But it's all done through one channel (`permissionDecisionReason`
prose), authored by hand per rule, with nothing structurally requiring a new
rule to include a redirect at all. `destructive_command_guard`'s pattern
macro has the same property in a more explicit shape — a dedicated "safer
alternatives" text block is part of every destructive-pattern definition,
not optional.

## The mechanism actually has three channels, not one

Verified against the current Claude Code hooks documentation
(code.claude.com/docs/en/hooks.md#pretooluse) — `permissionDecisionReason`
is not the only redirect surface a PreToolUse hook has:

1. **`permissionDecisionReason`** (with `permissionDecision: "deny"`) — what
   `org-rule-guard.py` already uses exclusively. Shown to the agent as the
   reason the call failed; the agent reads it and decides its own next
   action. Best default for anything where the sanctioned alternative
   requires the agent to make a real decision (which of several valid
   alternatives applies here).

2. **`updatedInput`** — the hook can rewrite `tool_input` and let a
   *corrected* version of the call execute instead of blocking outright.
   E.g. a `git push --force` could have `updatedInput` strip the flag and
   let a plain `git push` through. This is real auto-redirection, not
   deny-and-hope-the-agent-retries-correctly.

3. **`additionalContext`** (10k char limit) — non-blocking guidance
   surfaced alongside an *allowed* call. Useful for the "this is legal but
   here's a relevant warning" case that doesn't warrant a deny at all.

There is no `ask`/interactive-confirmation `permissionDecision` value —
it's binary, `allow` or `deny`. `updatedInput` is the only way to get
something in between "block" and "let it through unchanged."

## `updatedInput` needs a hard boundary, not blanket use

The tempting failure mode: using `updatedInput` to silently substitute a
*different operation* for the one the agent asked for, rather than a safer
version of the *same* operation. The strip-`--force`-from-`git-push`
example is safe because it accomplishes the agent's actual intent (get
these commits to the remote) through a non-destructive path — the agent's
model of what happened stays accurate.

Silently rewriting `kubectl delete X` into `kubectl get X`, however — a
substitution suggested during scoping research — is a different operation
entirely, misrepresented as if it were the requested one. The agent would
receive output that looks nothing like a deletion result, but it has no
signal that its actual request was discarded rather than fulfilled. It
could reasonably report to the user that the delete succeeded, or proceed
with a plan built on the assumption that it did. That's a worse failure
mode than a hard deny: a deny is visible and recoverable in the agent's own
next turn; a silently-swapped operation can propagate a false belief
forward through the rest of the session undetected.

**Rule for this project: `updatedInput` is only for transformations that
preserve the agent's actual intent through a safer mechanism** (strip a
dangerous flag, redirect a write path to the sanctioned location, swap an
unpinned tag for the real pinned one read from `containers/<name>/VERSION`).
Anything where the safe alternative is a genuinely different operation —
which is most of the destructive-verb cases this project actually exists
for (`vault kv destroy`, `kubectl delete`, `bf` repair-before-flush) — stays
a `deny` with a specific, actionable `permissionDecisionReason`, because the
agent needs to actually decide what to do next, not be quietly redirected
into doing something else while believing it did the first thing.

## What "actionable" means for a deny reason

The reason string should let the agent's very next tool call be the
correct one, without a research step in between. Concretely: where the
sanctioned value is programmatically derivable at check-time, the hook
should compute and embed it rather than pointing at where to look — e.g.
for an unpinned-tag violation, read `containers/<name>/VERSION` itself and
put the real semver string directly in the deny message, not just "pin a
semver tag read from containers/<name>/VERSION." The difference is between
the agent doing one more tool call to find the value and copying it
straight out of the denial.

## How to apply

Every rule this project adds needs a redirect as a required field, not
optional prose — same discipline `destructive_command_guard`'s pattern
macro already enforces structurally. Default to `deny` +
a specific, next-step-actionable `permissionDecisionReason`. Reach for
`updatedInput` only when the substitute genuinely preserves intent. Use
`additionalContext` for anything that should warn without blocking. Note
this is also a gap in `org-rule-guard.py` itself worth knowing about, even
though fixing it isn't this project's job: it uses `permissionDecisionReason`
exclusively today and has never used `updatedInput` or `additionalContext`,
even in the one case (`:latest`) where the redirect value is programmatically
derivable; unlike that legacy guard, icg's `image-tag` pack now derives the
value from the matching `containers/<name>/VERSION` file.
