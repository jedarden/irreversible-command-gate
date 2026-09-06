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
cargo test                                  # 562 tests: 257 unit + 305 integration
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

## Unrelated subsystem living in this tree

A second, unrelated body of code ships from this repository: bead-store
starvation detection and repair (`src/{starvation_diagnostic,
assignment_repair, frontier_consistency_service, pluck_query_debugger,
checkpoint_monitor, cascading_repair, bead_*}.rs`, the four extra binaries in
`src/bin/`, `scripts/bead-*`, `containers/{assignment-repair-monitor,
bead-starvation-repair}`, `declarative-config/`, and the `docs/bead-*` and
`docs/{cascading-repair-strategies,checkpoint-verification,
monitoring-deployment-guide}.md` files).

It has nothing to do with command interception. Do not let a change to it
touch the gate's engine, packs, or hook contract, and do not assume a
convention from one half applies to the other. Extracting it into its own
repository is an open recommendation, not a decision — leave it alone unless
asked.

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
