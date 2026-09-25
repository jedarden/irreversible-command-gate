# Check latency — the reproducible record

README once claimed a warm-cache check costs **~10 ms**. Nobody could say
where that number came from: there is no bench target in this repo
(`cargo bench --bench evaluation`, referenced in the developer guide, has
never existed here), and the figure predates the current pack set. This note
is the claim's replacement record: a defined benchmark
(`scripts/bench-check-latency`), the method, the environment it was measured
in, and the measured result — which revises the claim.

## The benchmark

`scripts/bench-check-latency` times whole `icg check` processes, spawn to
exit — that is the cost a harness hook pays per tool call. Method, in one
place (the script also prints/reports it with every run):

- **One sample = one fresh `icg check` process**, wall-clocked around the
  entire child. A check is short-lived; there is no in-process state to
  amortize.
- **Warm cache = page cache.** `--warmup` untimed runs per case first so
  timed samples never pay first-touch disk reads. The engine holds no cache
  across processes — "warm" describes the OS, not icg.
- **Defaults: 10 warmup + 100 timed runs per case**, summarized as min /
  p50 / mean / p90 / p99 / max / stdev. The README claim is a *median*, so
  p50 is the comparison number; the tail matters because a hook runs on
  every tool call, not the average one.
- **Two cases**, both non-push (so neither can reach the
  `git-stale-remote-head-push` network lookup, the documented no-network
  exception): an allow (`git status`) and a deny (`bao kv destroy
  secret/app/db`). Every run's stdout is checked against its expected
  verdict — a binary that mis-evaluates is never benchmarked as healthy.
- **`ICG_DENIAL_LOG` points at a temporary file** for every run, so deny
  samples exercise the operational denial append as production does, while
  benchmark noise never reaches the operator's real log.
- **The environment is part of the report**: host, CPU, core count, load
  average, memory, kernel, binary version and profile, and the pack set the
  binary itself reports loading (via `check --debug`). `--json` emits all of
  it machine-readably.

```bash
# The canonical run (release build from the workspace target dir)
cargo build --release
scripts/bench-check-latency

# Just the shipped packs, isolated from any installed /etc/icg set
scripts/bench-check-latency --pack "$PWD/packs" --cwd /tmp
```

`--pack` paths resolve against `--cwd` (they are handed to icg verbatim), so
pass them absolute.

## Measured record

Measured **2026-09-24**, icg **0.1.66** (release build, rustc 1.97.1),
commit `75519a9` tree, on `codinghome`:

| | |
|---|---|
| Host | Hetzner EX44-class, 13th Gen Intel Core i5-13500 |
| Kernel | Linux 6.18.46, 64 GB RAM |
| Load during measurement | ~9–12 (shared fleet box; medians are robust, tails are not) |
| Packs | the 11 shipped `packs/*.json` — 29 guarded + 21 safe patterns, 30 KB |

| Scenario (all warm page cache) | p50 | p90 |
| --- | --- | --- |
| `--version` (process floor) | 1.2 ms | 1.3 ms |
| check with an empty pack dir | 1.4 ms | 1.5 ms |
| check, tmux pack only (1 guarded pattern) | 2.0 ms | 2.1 ms |
| check, git pack only (4 guarded + 7 safe) | 2.9 ms | 3.1 ms |
| **check, shipped pack set (the claim's subject)** | **15.5 ms** | 20–22 ms |
| check, dev-checkout default (`packs/` + `/etc/icg/packs`, 21 files / 11 unique) | 17–20 ms | 21–29 ms |

Allow and deny cases are within ~0.5 ms of each other at every scale; the
denial append is not a factor. Across repeated full runs on this box the
shipped-set p50 moved between ~15 and ~20 ms with background load.

## Result: the ~10 ms claim is revised

Warm-cache median on the reference environment is **~15–20 ms**, roughly
1.5–2× the documented figure, and it scales with the shipped pattern count:
the per-scenario rows above rise from 1 pattern to 50 at a few hundred
microseconds per pattern, because pattern regexes are compiled at evaluation
time rather than precompiled or cached — `Engine::pattern_matches_command`
calls `Regex::new` per pattern check (`src/engine.rs`), and every check is a
fresh process, so nothing carries over. The ~10 ms figure was plausible for
an earlier, smaller pack set; it is not what the shipped set measures today.
The docs that carried it now carry the measured range and point here. A
precompiled or lazily-cached matcher would claw most of the difference back,
but that is optimization work, not documentation — it has not been done, and
this record does not assume it.

## Regression guidance

A latency number without a threshold invites both rot and flakiness. The
practical gate is the script's own:

```bash
scripts/bench-check-latency --assert-under 50
```

exit 1 if any selected case's p50 lands at or above the threshold. Run it
against a release build before a release cut, or from whatever scheduler
watches the fleet; the number to compare against is this note's measured
record **from a comparable machine and load** — a busy 2-vCPU runner will
not reproduce a 16-core desktop's tail.

Deliberately **not** wired into `cargo test` or CI: an absolute-time
assertion on a dev-profile binary on shared runners is a flake generator,
and the fleet auto-reopens beads on red gates. The suite's timing-sensitive
work stays in this script, where the operator chooses the threshold and the
machine. A smoke test
(`tests/check_latency_benchmark_tests.rs`) does pin the script's mechanics —
JSON shape, verdict verification, and that the `--assert-under` gate fails
closed — so the tool itself cannot silently rot.

When the record here is stale (new packs, new engine, new reference
hardware), re-run the canonical command, update this note with the new table
and environment, and update any doc quoting the old range in the same
commit.
