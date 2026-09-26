# Operator commands resolve rule packs from one labeled source — design intent

**Status: design intent, not shipped.** Nothing in this note describes HEAD
behavior; at HEAD the documented-commands pack set (`icg check`, `icg
coverage`, `icg status`, `icg catalog`) is still the union of every location
that exists, and the coverage JSON contract is still the one
[coverage-json-api.md](coverage-json-api.md) specifies.

Owning bead: `irrevers-041127e6` (open). This note salvages the design from
an abandoned, un-beaded working-tree attempt at that bead's scope (discarded
2026-09-26 under `irrevers-3d418f71`, the reconciliation bead; the raw
module, wiring and tests are parked in the named git stash
`irrevers-041127e6 orphaned pack-source/coverage-v2 attempt`). Claim the bead
and start from the stash — do not re-derive the precedence rules.

## The problem

The hook exists to answer "what is enforced" by loading one location: the
installed trust directory (`/etc/icg/packs`, or the legacy single-file
`/etc/icg/rule-pack.json`). The operator commands exist to answer the same
question, but used to load *every* location that existed — installed
directory *and* the working directory's `packs/` — and report the union. A
checkout ahead of the deployed release therefore reported coverage the
deployed hook did not enforce.

That stopped being hypothetical on 2026-09-25: the union reported a
`kubectl` pack that the trust pointer's v0.1.61 install did not carry,
overstating deployed enforcement exactly when the rule-4 retirement decision
needed the truth. The documented verification snippets (AGENTS.md,
quick-start: "confirm the 11 packs load") read the checkout while the hook
enforces the release — the sanctioned check could not see the gap it was
being used to close.

## The design — first-match precedence, never a union

One invocation resolves its pack set from exactly one source, in this order:

1. **Explicit** — caller-supplied `--pack` paths, or `ICG_PACK_DIR`.
   Development against a checkout stays possible, but only by saying so:
   `icg coverage --list --pack packs`.
2. **Installed** — the same directory chain the hook resolves (modular
   directory, then the legacy single-file artifact; the hook's
   `ICG_RULE_PACK` override belongs to the hook process, not to operator
   commands), so the default report describes the deployed policy.
3. **Repository** — the working directory's `packs/`, only when no release
   is installed (a bare checkout or CI runner).

No fallback ladder *within* an invocation: a location either wins or is not
consulted. Expansion keeps the long-standing rules — a candidate is a
`.json` file or a directory contributing its sorted `.json` entries, and an
explicit candidate that does not exist is an error rather than a silent gap.

## The labeling design

Every consumer of the resolved set labels the source in its output, so a
reader can tell deployed coverage from repo coverage instead of inferring it
from paths:

- `coverage --format json` gains a `pack_source` object next to `packs`:
  `origin` (one of `installed`, `repository`, `explicit`), `root` (the
  single location the packs resolved from; always present for the installed
  and repository origins, `null` when the caller named explicit paths), and
  `trusted_ref` (the installed release's trusted reference, readable only
  ever on an `installed` report — a reference cannot ride on a checkout).
- `coverage --list` (text) opens with a source header line naming the root
  and, for the installed origin, the release it corresponds to.
- `check --debug` names the source on stderr only — it is diagnostics, not
  output contract.
- `status`/health reporting names the pack source it validated.
- The bug report attributes its rule-pack inventory to a source.

**Open in the owning bead:** the catalog document carries no source label in
the salvaged attempt — it loaded from the resolved set but did not label the
`icg-catalog/v1` output. Labeling coverage *and* catalog output is
`irrevers-041127e6` item (2).

## Wire-format consequence

Adding `pack_source` to the coverage JSON is a shape change, so per
[coverage-json-api.md](coverage-json-api.md)'s Versioning policy it is a
`coverage/v2` bump — and that policy requires rewriting the note *in the
same commit* as the code. The abandoned attempt bumped the code and tests
but never rewrote the note, which is why the note-sync suite ran red against
that tree; the policy makes the attempt unshippable piecemeal, by design.
An implementer ships code, tests, the rewritten API note, and the AGENTS.md
/ quick-start verification-snippet updates (`irrevers-041127e6` item (4))
as one atomic change, with the extraction DoD as the gate.
