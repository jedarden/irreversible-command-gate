# irreversible-command-gate

**A last-second guard for AI coding agents.** It sits in the harness's
`PreToolUse` hook, reads the command or file the agent is about to act on,
and stops the handful that cannot be undone — destroying a secret,
force-pushing over history, printing a credential into the transcript,
purging a volume — while everything else passes through untouched.

Every denial says what to do instead. That is the point: the agent should
finish the turn knowing the sanctioned path, not just that it was blocked.

<p align="center">
  <img src="docs/assets/icg-demo.gif"
       alt="Terminal recording: icg allows git status, rewrites a force-push into a plain push, warns on a secret read to stdout, and denies a docker prune and a bare git credential fill."
       width="900">
</p>

<sub>Real `icg check` output — reproduce it with
[`docs/assets/demo.sh`](docs/assets/demo.sh).</sub>

## How it works

<p align="center">
  <img src="docs/assets/icg-flow.svg"
       alt="An agent's tool call goes to the harness PreToolUse hook, which hands it to icg. icg dispatches to a rule pack by tool keyword, checks safe patterns first, then guarded patterns, and returns allow, warning, rewrite, or deny. Rule packs live root-owned in /etc/icg/packs. Only deny stops the command."
       width="1000">
</p>

Four verdicts, one per redirect channel a rule can declare — and **only
`deny` stops the command**:

| Verdict | Hook response | When |
| --- | --- | --- |
| `ALLOW` | `permissionDecision: allow` | No rule matched, or a safe pattern matched first |
| `WARNING` | `allow` + `additionalContext` | The rule cannot decide reliably enough to block, but the agent should know |
| `REWRITE` | `allow` + `updatedInput` | A safe form of the same intent exists — the harness retries with it |
| `DENY` | `permissionDecision: deny` | Irreversible; the reason carries the alternative |

The engine is deterministic and does no network I/O. It **fails open**: an
empty pack directory, an unrecognised tool, or a crashed check allows the
command. A missed violation is recoverable; a wedged agent fleet is not.
A graduated [fail-closed policy](docs/operators/fail-closed-mode.md) exists
for once a release has proven itself.

Median cost of a check on a warm cache: **~10 ms**.

## Try it in a minute

No release has been cut yet, so build from source. There are no system
dependencies beyond a Rust toolchain:

```bash
git clone https://git.ardenone.com/jedarden/irreversible-command-gate.git
cd irreversible-command-gate
cargo build --release

./target/release/icg coverage --list
./target/release/icg check --command "bao kv destroy secret/app/db"
./target/release/icg check --command "git push --force origin main"
./target/release/icg check --command "git status"
```

`icg check` is the human-facing tester and always exits `0` — parse its
output, not its status. `icg hook` is the machine entry point: one
PreToolUse JSON document in, one decision envelope out.

To actually guard an agent, install the binary and packs root-owned and
register the hook — five minutes, in the
**[Quick Start Guide](docs/quick-start.md)**.

## What ships today

Ten rule packs, 26 guarded patterns, 18 safe patterns that keep common
read-only forms fast and quiet.

| Pack | Rules | Blocks |
| --- | --- | --- |
| `openbao` | 3 | `kv destroy`, `metadata delete`, mount/policy deletion, `operator rekey`; secret literals in argv; secret reads to stdout |
| `git` | 4 | bare `git credential fill`; `--force` push (rewritten to a plain push); commits with no pathspec; pushing over a stale remote head |
| `secrets` | 6 | GitHub tokens and PATs, AWS keys, Slack tokens, Anthropic keys, PEM private-key blocks — in commands *and* file content |
| `docker` | 3 | `system prune --all`, `volume rm`, `image rm --force` |
| `image-tag` | 2 | `:latest` and bare-SHA image references in manifests |
| `storage-class` | 1 | storage classes that cannot be expanded or reclassed in place |
| `beads` · `misc` · `tmux` · `argocd-topology` | 7 | conventions of the fleet this was built for — useful mainly as worked examples |

The first four packs describe footguns that exist wherever the tool does.
The last row encodes local convention. The
[coverage table](docs/quick-start.md#what-gets-protected) marks every pack
*General* or *Fleet-specific* and names every rule id, so you can tell at a
glance which ones travel.

Nothing about the engine is fleet-specific — `icg new-pack <tool>`
scaffolds a pack and its regression test together.

## What this deliberately does not do

- **It is a backstop for an honest, fallible agent, not a boundary against
  a hostile one.** Policy lives root-owned in `/etc/icg/` so the guarded
  agent cannot rewrite it, but an agent that sets out to defeat the guard
  can. Keep the harness's own approval and sandbox controls on.
- **It does not defend against prompt injection** or a malicious repository
  trying to trick an honest agent. Different threat class, explicitly out of
  scope.
- **It does not reach cloud-hosted agent sessions** — ChatGPT web, Codex
  cloud tasks, claude.ai. Only local CLIs invoke local hooks. See
  [multi-harness-integration.md](docs/notes/multi-harness-integration.md).
- **It does not cover `kubectl` mutations, `.github/workflows/*`, or
  `kind: Job`/`CronJob`.** Those stay with the org-level Python hook by
  decision, not by omission —
  [existing-enforcement-infrastructure.md](docs/notes/existing-enforcement-infrastructure.md).

## Project status

Honest version: the engine, the packs, both front-ends (hook and PATH
wrapper), the release-integrity machinery, and 562 tests — 257 unit, 305 integration across 51 files
are in the tree and working. **No end-to-end release has been cut yet**, so
build-from-source is the only install path and the trust-pointer /
auto-update flow is unproven in production. Tracked in
[`docs/plan/plan.md`](docs/plan/plan.md), Phase 0.

## Documentation

Start at **[docs/README.md](docs/README.md)** for the full map. The short
version:

| You are | Read |
| --- | --- |
| Trying it out | [Quick Start](docs/quick-start.md) |
| Deploying it | [Deployment guide](docs/operators/deployment-guide.md) → [Operator docs](docs/operators/README.md) |
| Hit a denial | [Deny-message guide](docs/operators/deny-messages.md) |
| Writing a rule pack | [Rule-pack best practices](docs/developers/rule-pack-best-practices.md) |
| An agent working in this repo | [AGENTS.md](AGENTS.md) |
| Curious about the design | [plan.md](docs/plan/plan.md) · [ideas ledger](docs/notes/ideas-ledger.md) |

## Authoring a rule pack

```bash
icg new-pack <tool> --pack-type command --output-dir packs/
```

Writes `<tool>.json` and `<tool>_pack_tests.rs` together, pre-filled, and
refuses to overwrite either. `--pack-type content` scaffolds a file-content
pack instead.

Before proposing a pack change, run the release gate — it builds the fixed
deny-regression corpus and reports any rule that stopped covering what it
used to:

```bash
icg regression-suite packs --release-gate --output regression-suite.json
icg coverage-diff <previous-pack> <current-pack>
```

Per-pack generation (`icg regression-suite packs/<id>.json`) needs a
derivable or explicit `example_command` for every guarded pattern; packs
built on predicates or content regexes are covered by the `--release-gate`
corpus and their own tests instead.

## License

MIT — see [LICENSE](LICENSE).

---

Part of [jedarden.com](https://jedarden.com).

*The GitHub repo is a read-only mirror of
`git.ardenone.com/jedarden/irreversible-command-gate` — issues and PRs are
welcome on either.*
