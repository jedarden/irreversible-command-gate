# Should rules be configurable at runtime, or hardcoded?

## The question as asked

Should this project's block-list (which commands/patterns get denied) live
as compiled/hardcoded logic — matching `org-rule-guard.py`'s current
design — or as external configuration (YAML/TOML/JSON) loaded at
invocation time, letting rules be added or changed without touching code?

## The naive tradeoffs

**For hardcoding**, matching the proven `org-rule-guard.py` design:
- Auditable in one pass — reading the source is reading the complete rule
  set, no second file to cross-reference.
- No config-loading step on a security-critical hot path. The hook already
  runs under a 10-second timeout; a config parse failure is a new failure
  mode the current design (zero I/O beyond stdin) doesn't have.
- This environment's prohibition list changes rarely — each rule in
  CLAUDE.md's hard-prohibitions section traces to a specific past incident,
  not a fast-moving requirement. Low rule churn means hardcoding's
  maintenance cost stays low.

**For runtime configuration:**
- Lower friction to add a rule — edit data, no redeploy of a compiled
  artifact or hook restart. Matches this environment's existing culture:
  everything else here is GitOps-style (edit `declarative-config`, commit,
  push, let a controller reconcile) rather than code-and-redeploy.
- `destructive_command_guard`'s per-vendor "pack" architecture (one Rust
  module per tool: `vault.rs`, `aws.rs`, `azure.rs`, ...) shows a real
  middle ground already exists in the wild — modular by file, so adding a
  new tool's rules doesn't touch shared dispatch code, without necessarily
  being end-user-editable data. Worth noting this is *not* the same axis as
  hardcoded-vs-configurable — it's monolithic-vs-modular, and both ends of
  that axis can be compiled.

Neither side of this framing is the load-bearing question, though — see below.

## The actual finding that reframes this

Verified directly (not assumed): `~/.claude/hooks/org-rule-guard.py` is
owned by `coding:coding`, mode `775` — the same user the guarded agent's
own process runs as. **This means the agent being constrained by the hook
can freely `Write`/`Edit` the hook's own source file.** And critically,
none of the hook's five content-checks would catch a self-edit that simply
deletes or weakens a rule: removing a line from the `MUTATING` set, or
emptying `SECRET_PATTERNS`, doesn't look like a `.github/workflows/` path,
a `Job`/`CronJob` manifest, a `:latest` tag, or a credential value. **The
hook would allow an edit that disables itself.**

This means "hardcoded" was never actually providing tamper-resistance in
this deployment. It only ever worked as a backstop for an honest, fallible
agent — one that might miss a subtlety of a rule it's already trying to
follow — not as a defense against a genuinely adversarial or
compromised agent that would simply edit the guard out of its own way.
CLAUDE.md itself says as much, obliquely: "the rule still binds you
regardless of whether the hook catches it" — the hook was designed as a
mechanical backstop for good-faith mistakes, not a security boundary
against bad-faith action. That framing has been correct all along; the
"hardcoded" property just never contributed to it the way it might
intuitively seem to.

**So the real axis isn't hardcoded-vs-configurable — it's whether the rule
source, in *either* form, lives somewhere the guarded agent's own process
can write to.** A YAML config sitting next to the Python file is exactly as
tamperable as the Python file itself. Hardcoding buys nothing here that a
config file loses.

## What this means for scope

Three deployment shapes, ranked by whether they actually solve the tamper
question — independent of code-vs-data style:

1. **Rule source lives in the agent's own writable filesystem** (like
   `org-rule-guard.py` today, hardcoded or not). Fine *only* if the threat
   model stays "backstop for an honest agent's mistake." Doesn't survive an
   adversarial or prompt-injected agent, regardless of format.
2. **Rule source lives somewhere the agent's process genuinely cannot
   write** — root-owned and immutable, a separate unprivileged-for-the-agent
   service, or a different host entirely (the SEAM/proxy model from the
   OpenBao rotation research: enforcement moves server-side, out of the
   agent's blast radius by construction).
3. **Rule source is git-tracked and deployed through a pull the agent
   doesn't control** — matches this environment's own
   declarative-config/ArgoCD pattern: a human or CI pushes a rule change,
   a separate process pulls and reloads on its own schedule. Editing the
   *repo* and having the *live* rule set change are two different actions,
   and only the second one needs to stay outside the agent's control. This
   gets the maintainability win of "configurable" without reopening the
   self-edit hole, as long as reload isn't triggered by anything the
   guarded agent's own session can invoke — **unless a separate,
   independently-verified content gate already fully constrains what a
   reload could possibly adopt.** `irreversible-command-gate` (see
   `docs/plan/plan.md` Phase 0) relaxes this specific requirement on
   exactly that basis: release-cutting is already human-gated, so the set
   of things a reload could ever pull in is already fully human-controlled
   before the agent's `icg update` trigger enters the picture at all — the
   trigger can only cause *early* adoption of something already vetted,
   never adoption of something untrusted. Absent that separate content
   gate, the stricter original rule stands: don't let the agent trigger
   reload either.

**Recommendation:** keep the evaluation engine simple and hardcoded — no
reason to depart from `org-rule-guard.py`'s proven zero-I/O, fails-open
design for the parsing/dispatch logic itself. But make the rule *data*
modular (one file per tool, `destructive_command_guard`-style: `vault.*`,
`kubectl.*`, `git.*`, `beads.*`), and deploy it via shape 3 above rather
than shape 1 — i.e., not simply another file under `~/.claude/hooks/` that
this same agent session can already freely edit. That gets the lower-churn
rule-authoring this environment already prefers, without inheriting a
tamper surface the current hook has always had but never actually needed,
because its real job was catching honest mistakes all along.
