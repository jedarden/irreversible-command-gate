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
# Default operator resolution: the installed trust source wins; the first
# output line identifies it. A bare checkout is only the final fallback when
# neither installed location exists.
cargo run --release -- coverage --list      # the 11 packs load from installed trust source by default
# Developer-only checkout verification: this override is authoritative and
# does not merge with or describe the installed hook's policy.
ICG_PACK_DIR="$PWD/packs" cargo run --release -- coverage --list
ICG_PACK_DIR="$PWD/packs" cargo run --release -- check --command "git push --force origin main"
cargo test                                  # the whole suite, zero failures
cargo test --test documentation_consistency_tests   # the docs-vs-reality guards
```

`cargo run --release --` rather than a hard-coded binary path: the hosts
this repo is worked on run a cargo wrapper that pins **one target
directory per repo** at `/build/irreversible-command-gate` (derived from
the origin URL; a `git archive` extraction is matched to the repo by its
Cargo.toml identity). The wrapper overrides any other `CARGO_TARGET_DIR`
and refuses a `--target-dir` outside that directory, so the binary does
not land beside the checkout — and build output must never be directed
into `/home`; a relative target dir resolved against `~/.cargo` filled
that filesystem to 99% once. Never set a per-invocation `CARGO_TARGET_DIR`
to dodge the wrapper; when two builds of this repo genuinely need to stay
apart, give each a subdirectory of `/build/irreversible-command-gate/`,
which the wrapper allows. To install what you built, use
`./install.sh --from-checkout`, which resolves the real location via
`cargo metadata`. `tests/cargo_target_doc_tests.rs` fails any doc that
regresses to the superseded model (a shared target directory that
`CARGO_TARGET_DIR` "moves per invocation", or per-run target dirs);
`tests/documentation_consistency_tests.rs` fails any doc that regresses to
a hard-coded build-output path.

`icg check` always exits `0` for allow, warning, rewrite and deny alike —
parse stdout, never the exit status. `--debug` writes the full evaluation
trace to **stderr** while the decision stays on stdout.

To read the enforced policy programmatically rather than scraping text:

```bash
# Default: selected installed trust source (or the repository fallback only
# when both installed locations are absent).
icg coverage --list --format json   # coverage/v2; inspect .pack_source
icg catalog --json                  # icg-catalog/v2; inspect .pack_source

# Developer-only checkout view; explicit and authoritative for this command.
ICG_PACK_DIR="$PWD/packs" icg coverage --list --format json
ICG_PACK_DIR="$PWD/packs" icg catalog --json
```

The first is the pack view: every pack and rule, with severity, redirect
channel, check kind and explanation — plus `pack_source` and an `unreadable`
array naming any pack that failed to load. The second is the event view: what
must never happen and what is always allowed, keyed by the denial attribution
(`pack` + `id`) with severity and the sanctioned alternative, digest-stamped
so a consumer can detect policy drift; it carries the same `pack_source` for
the pack-derived events. Prefer these over parsing `coverage --list`; tools
outside this repository must consume the catalog rather than parse packs or
keep a second copy of the list.

## Which packs an operator command reads

Operator commands (`check`, `explain`, `coverage`, `catalog`, `status`, and
the `health` report) select exactly one pack source: explicit `--pack` paths,
else `ICG_PACK_DIR`, else the installed chain (`/etc/icg/packs`, then the
legacy `/etc/icg/rule-pack.json`), else the working directory's `packs/` only
when both installed locations are absent. Sources are never unioned. Use
`ICG_PACK_DIR="$PWD/packs"` for a deliberate developer checkout override;
it is authoritative and does not describe what the installed hook enforces.

The hook reads only its installed chain (`ICG_RULE_PACK`, then
`/etc/icg/packs`, then the legacy artifact) — never the working directory or
`ICG_PACK_DIR`. Text coverage prints one `Pack source:` line; `check --debug`
prints the same label on stderr; `status` prints its operator source section;
coverage JSON and catalog JSON carry `pack_source` with `origin`, `root`, and
`trusted_ref`. The health report uses the same selection but is line-oriented
and does not print a source header. An explicit or present-but-empty/unreadable
source is an error and never falls back; coverage records an unreadable pack
in `unreadable`, while catalog export fails without a partial document.
`icg pack-drift` remains the explicit installed-versus-release comparison
(`exit 0` identical, `1` drift, `2` could not run). Tests stage the installed
side with `ICG_INSTALLED_PACK_DIR` — an operator-test-only seam the hook never
reads. See [`docs/notes/pack-source-resolution.md`](docs/notes/pack-source-resolution.md)
and the versioned [coverage API](docs/notes/coverage-json-api.md) and
[catalog API](docs/notes/event-catalog-json-api.md).

## The rules that actually bind you here

1. **Never widen a guarded pattern to fix a false positive.** A false
   positive means a *safe* pattern is missing. Widening the guarded regex is
   how coverage disappears silently. `icg coverage-diff` exists to catch
   this; run it.
2. **Every guarded rule owes the caller an alternative.** A `redirect` whose
   reason only says "blocked" is an incomplete rule — enforced: a pack whose
   redirect reasons are empty or block-only fails
   `icg::rule_pack::validate_redirect_actionability` (CI gate in
   `tests/redirect_actionability_tests.rs`). See
   [`docs/notes/redirect-not-just-block.md`](docs/notes/redirect-not-just-block.md).
3. **The engine does no network I/O** and fails open on any error. Keep it
   that way — one scoped exception exists (stale-remote-head lookup before a
   push) and it is documented in
   [`docs/notes/no-network-boundary.md`](docs/notes/no-network-boundary.md).
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

## Host units live in `systemd/`, never only on the host

That removal left a lesson. `092e82c` deleted the scripts but the *installed*
copies of twelve units stayed in `~/.config/systemd/user/` on codinghome, and
one timer kept firing into `203/EXEC` for a week (bead `irrevers-46f2b741`).
A deleted script can strand a live unit only because the two lived in
different places with no link between them. So:

- Any systemd unit this repo installs on a host is tracked in `systemd/` and
  installed as a **symlink** from the host's unit directory via
  `systemd/install.sh` — never a copy. A symlink breaks visibly when the
  tracked unit is deleted; a copy fails silently when it fires.
- A commit that deletes a script must, **in the same commit**, delete its
  tracked unit and say in the commit message that hosts need
  `systemd/uninstall.sh` run.
- `systemd/check-consistency.sh` fails on three shapes of drift: a tracked
  unit referencing a path missing from the working tree, a host unit
  executing a repo path that no longer exists, and a tracked unit installed
  on the host as anything other than `install.sh`'s symlink to the tracked
  file (a copy passes the path checks until the day its script dies — that
  silence is the original incident). Run it before pushing anything that
  touches `systemd/` or a script a unit executes; the repo-side half is in
  the gates — `cargo test` in CI (`tests/systemd_consistency_tests.rs`) and
  an explicit step in `scripts/definition-of-done.sh`.

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
