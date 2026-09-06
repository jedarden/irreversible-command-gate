# AGENTS.md — working in this repository

Read this before changing anything here. It is short on purpose; the long
form is [`docs/plan/plan.md`](docs/plan/plan.md).

## What this repo is

`icg` is a `PreToolUse` guard for AI coding agents: it reads a command or
file an agent is about to act on and returns allow / warning / rewrite /
deny. Policy is **data** (`packs/*.json`), the engine is **code**
(`src/engine.rs`). Adding coverage almost always means adding a pack rule,
not writing Rust.

## Build, run, test

```bash
cargo build --release                       # no system deps; rustls, not OpenSSL
./target/release/icg coverage --list        # confirm the 10 packs load
./target/release/icg check --command "git push --force origin main"
cargo test                                  # 526 tests, 0 failures
cargo test --test documentation_consistency_tests   # the docs-vs-reality guards
```

`icg check` always exits `0` for allow, warning, rewrite and deny alike —
parse stdout, never the exit status. `--debug` writes the full evaluation
trace to **stderr** while the decision stays on stdout.

To read the enforced policy programmatically rather than scraping text:

```bash
icg coverage --list --format json   # "format": "coverage/v1"
```

Every pack and rule, with severity, redirect channel, check kind and
explanation — plus an `unreadable` array naming any pack that failed to
load. Prefer this over parsing `coverage --list`.

## The rules that actually bind you here

1. **Never widen a guarded pattern to fix a false positive.** A false
   positive means a *safe* pattern is missing. Widening the guarded regex is
   how coverage disappears silently. `icg coverage-diff` exists to catch
   this; run it.
2. **Every guarded rule owes the caller an alternative.** A `redirect` whose
   reason only says "blocked" is an incomplete rule. See
   [`docs/notes/redirect-not-just-block.md`](docs/notes/redirect-not-just-block.md).
3. **The engine does no network I/O** and fails open on any error. Keep it
   that way — one scoped exception exists (stale-remote-head lookup before a
   push) and it is documented in the plan.
4. **Docs are tested.** `tests/documentation_consistency_tests.rs` asserts
   that quick-start's coverage table names every shipped rule id and count,
   that no operator doc cites a pack or pattern that does not exist, and that
   no install path points at a release that has not been cut. If you add a
   pack rule, the doc update is part of the change, not a follow-up.
5. **Check the ideas ledger before proposing a feature.**
   [`docs/notes/ideas-ledger.md`](docs/notes/ideas-ledger.md) records two
   rounds of 100 ideas with explicit kill reasons — allowlist-first mode,
   LLM-assisted triage, a standing daemon, AST shell parsing and full
   capability-grant inversion were all considered and rejected, with reasons
   that still hold.

## Adding a rule

```bash
icg new-pack <tool> --pack-type command --output-dir packs/
```

Then, in order:

1. Write the regex. Prefer anchoring and explicit verbs over breadth.
2. Add the safe patterns that keep the tool's ordinary read-only forms quiet.
3. Write the redirect: what the caller should do instead, concretely.
4. `icg redos-check packs/<tool>.json` — catastrophic backtracking is a
   denial-of-service on every tool call, not just yours.
5. Add the rule id and count to quick-start's coverage table.
6. `cargo test` and `icg regression-suite packs --release-gate`.

## What is deliberately not here

Bead-store health tooling used to ship from this repository — starvation
detection, checkpoint drift, assignee repair, dependency-cycle repair: ten
modules, seven binaries, and a `rusqlite` bundled-SQLite build, all written
here on 2026-08-26 because that was the checkout an agent happened to be
standing in. It was removed on 2026-09-06.

`bead doctor` already does all of it, and correctly: `--starvation-check`,
`--starvation-recovery [--force]`, `--visibility-check`, `--rehearse`,
`--repair`, plus `bead list --ready --verbose` for the exclusion reasons.
The removed code hand-wrote its own SQL against `beads.db` and knew nothing
about `resource_locks`, `leases`, or `claim_epoch` — so it could report a
lock-held bead as starved, and could clear a live worker's claim.

If a bead-store problem needs tooling, it goes to `bead-rs` as a bead. Not
here. Recover the removed code from history if you need to read it:
`git show 6c13171 -- src/starvation_diagnostic.rs`.

## Repository conventions

- Work on `main`; do not open feature branches.
- Stage explicit paths (`git add src/engine.rs packs/git.json`). Never
  `git add -A` — this tree accumulates untracked `.beads/` diagnostics and
  build output.
- Never force-push. Push to the Forgejo `origin`; GitHub is a read-only
  mirror.
- Never hand-edit `.beads/`. This workspace is on the bead-rs CLI (`bead`),
  declared in `.needle.yaml`.
- Rule packs are release data. Changing one is a policy change: run the
  regression gate and the coverage diff, and say what coverage moved.
