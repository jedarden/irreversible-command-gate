# ADR-001: Absorb the blanket kubectl-mutation block as a pack

**Status:** Accepted — 2026-09-19
**Deciders:** operator (jedarden)
**Supersedes:** plan.md "Explicitly not attempted" (kubectl) as generalized in
`docs/notes/existing-enforcement-infrastructure.md` (Rule 4), `docs/quick-start.md`
("there is deliberately no kubectl pack and there will not be one") and
`tests/coexistence_org_rule_guard_tests.rs` ("PERMANENTLY not absorbed")
**Tracking:** `irrevers-3fc0fbce`; the follow-up gaps are all resolved —
`irrevers-08a4a11b` (timeout/xargs/nice, closed 2026-09-20) and
`irrevers-a4779a37` (`sh -c` payloads, closed 2026-09-25). No open gap
remains from this decision; the remaining boundary is the deliberate one
described under Consequences below.

## Context

`org-rule-guard.py` rule 4 denies mutating `kubectl` verbs. Until now this
project refused to absorb it. The recorded reason, plan.md's "Explicitly not
attempted" item, is specifically about **narrowing** that rule: blocking
mutations only against ArgoCD-managed resources would need live cluster
state, which trades away the engine's zero-I/O determinism. The item ends
"The blanket version stays as-is; this project doesn't touch it."

That reason does not cover the blanket rule itself. The blanket rule is
pure syntax — a verb list with two carve-outs — the same shape as the
`docker` and `git` packs, and it needs no cluster state at all. The
"permanently not absorbed" wording in the notes, quick-start and the
coexistence test generalized the narrowing decision into a decision about
ownership that the original reasoning never made.

What changed since the exclusion was written:

- **icg enforces on the host.** The 2026-09-18 trust-pointer update
  (v0.1.61, "Approved host enforcement cutover") moved the Claude Code hook
  off `--practice`. icg is no longer an observer beside the Python hook.
- **The goal is one enforcement path.** Every other org-rule-guard rule is
  absorbed or has a scheduled channel. Keeping kubectl in the Python hook
  alone leaves its weakest properties in charge of the most-used guarded
  tool: it fails open with no graduation policy, has no regression suite,
  no denial log, no override artifacts, and its segmenter does not respect
  quotes (it denied `printf '...cd /tmp && kubectl apply...'` as a live
  command on 2026-09-19).
- **The wrapper objection is already handled in code.** `icg install`
  skips the `kubectl` keyword ("never shadowed per policy",
  `src/documented_commands.rs`), so a keyword-dispatched pack guards the
  hook front-end without ever becoming a PATH wrapper.

## Decision

**Ship a blanket, syntactic `kubectl` pack** (`packs/kubectl.json`), hook
front-end only:

| Pattern | Severity | Covers |
|---|---|---|
| `kubectl-delete` | Critical | `delete` — irreversible for data-bearing objects |
| `kubectl-mutating-verb` | High | `apply`, `patch`, `edit`, `replace`, `set`, `annotate`, `label`, `scale`, `autoscale`, `cordon`, `uncordon`, `drain`, `taint`, `evict`, `rollout restart/undo/pause/resume` — org-rule-guard's set — plus `expose` and `run` |
| `kubectl-create-outside-argo` | High | `create`, except Argo Workflow submission to `argo-workflows`/`iad-ci` (a safe pattern) |

Safe patterns keep read-only verbs, `rollout status|history` and Argo
Workflow creation allowed. A verb counts only in verb position: after
whitespace, before any `;`, `|` or `&`, so values such as `app=delete-me`
and downstream text such as `| grep delete` do not match. Every redirect
names the sanctioned alternative: edit the manifest in
`jedarden/declarative-config`, commit, push, let ArgoCD sync.

**Still not attempted:** ArgoCD-aware narrowing. The original reason holds
unchanged — it needs live cluster state — so the pack stays blanket.

## Consequences

- During coexistence both guards deny the same commands (consistent
  double-deny, the coexistence PASS criterion). Rule 4 can retire from
  `org-rule-guard.py` on the same terms as rules 1–3, leaving only its
  Write/Edit credential-value rule.
- The pack inherits the engine's prefix handling. `sudo`, `env`, env
  assignments, absolute paths and `&&` chains are normalized, as are
  `timeout`, `xargs` and `nice` (`irrevers-08a4a11b`) and the `-c` command
  string of `sh`/`bash`/`dash`/`ash`/`zsh`/`ksh` (`irrevers-a4779a37`): a
  `-c` payload is, by the shell's own semantics, a command line, so
  segmentation lexes it with the ordinary lexer and its commands dispatch
  like any other segment. These shapes are regression-pinned for the
  kubectl pack in `tests/kubectl_pack_tests.rs`: guarded verbs fire
  through `timeout`, `xargs` and `nice`, through `sh -c`, and through the
  wrappers composed with each other, while read-only verbs stay allowed
  through the same wrappers. The boundary that remains is deliberate:
  commands the guard cannot see as text — a script fed on stdin or by a
  heredoc (`bash <<'EOF'`), a script file (`bash script.sh`),
  `eval`/indirect interpreters, and anything a command substitution
  produces at runtime — stay unevaluated. Modeling those would mean
  resolving runtime values, not lexing static text, which is the same line
  the AST-parsing rejection in the ideas ledger draws: the engine lexes
  what is written, it does not execute or resolve what would be read.
- A future verb added to kubectl is allowed until a pattern names it. That
  is the same fail-open trade the rest of the engine makes.
