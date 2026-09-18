# The no-network evaluation boundary

The claim, from [AGENTS.md](../../AGENTS.md) rule 3: *the engine does no
network I/O and fails open on any error*, with exactly one scoped exception.
This note is the reference for what the boundary covers and what enforces it;
the enforcement itself lives in
[`tests/no_network_boundary_tests.rs`](../../tests/no_network_boundary_tests.rs)
(bead `irrevers-d95bad06`).

## The boundary

Ordinary evaluation — matching a `--command` string or `--file` content
against the shipped packs — is pure computation over process-local state:
regexes, predicates, file reads for state/telemetry/denial-log sinks. It
spawns no processes and opens no sockets. A verdict (ALLOW / WARNING /
REWRITE / DENIED) is reached without anything leaving the process.

## The one exception

`git-stale-remote-head-push` (`packs/git.json`, tier 2,
`push_requires_current_remote_head` predicate) is the only rule allowed to
observe the outside world. Before a non-force `git push` is judged, the
engine runs the stale-remote-head lookup:

1. `git rev-parse --abbrev-ref --symbolic-full-name @{u}` — resolve upstream
2. `git rev-parse <upstream>` — the locally tracked remote SHA
3. `git ls-remote --heads <remote> <branch>` — the one live query

and denies the push when the remote HEAD has moved past the local tracking
ref. The exception is justified by the operation it guards: a push is itself
a network operation, so one bounded lookup before it adds no new exposure.
The gate is push-only — force pushes never reach the lookup (they take the
tier 1 `git-force-push` rewrite path, which needs no network) — and any
lookup error fails open, per rule 3.

## How the boundary is enforced

The suite locks the claim from three independent directions, so a module
cannot quietly grow a fetch while every document keeps saying "no network":

1. **Behavioral, whole process.** The real `icg check` binary runs with
   every process it could reach the network through (`git`, shells, `curl`,
   `ssh`, cloud CLIs, …) shadowed by a spy shim that logs the attempt and
   forwards to the real binary. Command and content batteries across all
   four verdicts must produce their verdicts with an empty spy log. A child
   process is the only route from evaluation to the network; the structural
   layer pins that too.
2. **The exception, exactly.** An up-to-date push is allowed and costs
   exactly the three spawn lines above; a stale push is denied by
   `git-stale-remote-head-push` with the same three; a non-push command in
   the same repository spawns nothing — the lookup is gated on the push, not
   on the repository.
3. **Structural.** Every `Command::new` in `src/engine.rs` must live inside
   `check_remote_head_stale`, which must have exactly one caller inside the
   push-gated predicate arm. Crate-wide, process spawns and in-process
   network APIs (`reqwest`, `TcpStream`, `tokio::net`, …) may appear only on
   the allowlisted non-evaluation surfaces: `documented_commands.rs`'s
   `icg backup` tar inspection, the wrapper's exec of the real binary in
   `main.rs`, and `alerting.rs` / `update.rs` / `health_server.rs`. A new
   network capability has to edit the suite — and justify itself in review —
   before it can run.

When the host has a working `strace`, a fourth test re-runs the batteries
under `strace -f -e trace=network` and asserts the kernel saw no
internet-protocol socket at all. It skips where strace or ptrace is
unavailable; the other layers do not depend on it.

## Running it

```sh
scripts/definition-of-done.sh --fast   # full gate; includes this suite
cargo test --test no_network_boundary_tests   # just the boundary suite
```
