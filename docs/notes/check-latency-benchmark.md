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

**Current record — 2026-09-26, icg 0.1.71** (release build, rustc 1.97.1;
source identical at the commit carrying this record), on `codinghome`.
Exact command — the shipped-pack-set form above, the claim's subject,
isolated from any installed `/etc/icg` set:

```bash
cargo build --release
scripts/bench-check-latency --pack "$PWD/packs" --cwd /tmp --json \
  > docs/notes/evidence/check-latency-record.json
```

The raw record is committed at
[`docs/notes/evidence/check-latency-record.json`](evidence/check-latency-record.json)
— the script's own `--json` report, so the environment (host, CPU, memory,
kernel, load, binary path and version, the pack set the binary itself
reports loading) is captured verbatim rather than paraphrased. Sample
count: **10 warmup + 100 timed runs per case**.

| Case (warm page cache) | p50 | p90 |
| --- | --- | --- |
| allow — `git status` | 15.5 ms | 20.1 ms |
| deny — `bao kv destroy secret/app/db` | 18.7 ms | 43.8 ms |

| | |
|---|---|
| Host | Hetzner EX44-class, 13th Gen Intel Core i5-13500, 20 cores, 64 GB RAM |
| Kernel | Linux 6.18.46 |
| Load during measurement | ~10–12.5 (shared fleet box; medians are robust, tails are not — the deny p90 above is exactly such a tail) |
| Packs | the 11 shipped `packs/*.json` — 29 guarded + 21 safe patterns |

Both medians land inside the quoted **~15–20 ms** band, so the README
figure stands. What changed with this refresh is not the number but the
record's age: the previous record was measured on icg 0.1.66, five
releases before the binary a reader would actually run. Re-measuring is a
release step now, not an ad-hoc repair — see the release-cutting runbook's
re-measure step, and the version tie
(`committed_latency_record_is_current_with_the_release_version`,
`tests/documentation_consistency_tests.rs`) that fails the build when the
committed record names any binary other than the one this tree would
release.

**Superseded record — kept for the scaling analysis.** Measured
2026-09-24, icg **0.1.66** (release build, rustc 1.97.1), commit `75519a9`
tree, same box at load ~9–12:

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
the superseded record's scenario rows rise from 1 pattern to 50 at a few hundred
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
script's own gate is `--assert-under`:

```bash
scripts/bench-check-latency --assert-under 50
```

exit 1 if any selected case's p50 lands at or above the threshold.

**The standing gate is the definition of done**
(`scripts/definition-of-done.sh`). It builds the release profile — the
profile the claim is about; a dev-profile binary measures something else —
then runs the gate at **50 ms** on the shipped pack set (`--pack
$REPO_ROOT/packs --cwd /tmp`, the canonical form above, so the gate
measures what README claims rather than whatever `/etc/icg/packs` holds on
the box running the gate). 50 ms is ~3× the measured median under
reference load: loose enough that background load moving the p50 between
15 and 20 ms cannot flake it — a *median* needs a sustained multi-fold
slowdown to move that far, not one straggler — and tight enough to catch
the rot modes that would hollow the claim out: pack-count growth, hot-path
I/O, a per-pattern compile regression. A tighter threshold, or one for a
different machine class, stays operator-invoked: the number to compare
against is this note's measured record **from a comparable machine and
load** — a busy 2-vCPU runner will not reproduce a 16-core desktop's tail.

Still deliberately **not** wired into `cargo test`: this suite runs a
dev-profile binary (the wrong thing to time), and the fleet auto-reopens
beads on red gates — an absolute-time assertion in the unit suite is a
flake generator. The timing-sensitive work stays in this script. A smoke
test (`tests/check_latency_benchmark_tests.rs`) pins the script's mechanics —
JSON shape, verdict verification, and that the `--assert-under` gate fails
closed — plus the DoD wiring itself, so neither the tool nor its gate can
silently rot.

**CI wiring (2026-09-25, bead `irrevers-4e6b05b0`).** The paragraph above
used to exempt the shared-runner CI as well, on the reasoning that CI
runners' timings are foreign to this record. That half is now revised, in
the order it prescribed: every push to main runs this bench in icg-ci's
build-and-release stage, on the release profile and the shipped pack set
(`--pack <checkout>/packs --cwd /tmp`), **advisory** — the run reports its
p50s into the workflow log and fails nothing, because no iad-ci runner p50
range is on record yet. The stage takes a `bench-budget-ms` parameter
(default `0`): setting it to a millisecond value passes it to
`--assert-under` and makes a breach — or a bench that cannot run at all —
fail the build. The flip is deliberately left for a follow-up with the
advisory runs' numbers in hand: pick the budget from what the runner
actually measures (the DoD's 50 ms is the candidate if the runner clears
it as comfortably as the reference box does), then lean on the same
flake-resistance argument that lets 50 ms stand here — a *median* only
moves under a sustained multi-fold slowdown, whatever the machine. Until
then, an advisory bench that fails (the executor image ships no python3,
so the stage installs it per-run; a broken release build) prints a loud
warning in the run log and leaves the run green — going unmeasured is the
current normal, not an alarm.

When the record here is stale (new packs, new engine, new reference
hardware), re-run the canonical command, update this note with the new table
and environment, and update any doc quoting the old range in the same
commit. That is no longer left to whoever notices the staleness: the
release-cutting runbook's re-measure step runs this bench on every release
candidate and refreshes the committed raw record
(`docs/notes/evidence/check-latency-record.json`), this section, and — if
the range moved — the README figure, in the bump's commit. The tie is
enforced, not hoped for: the raw record must name the binary version this
tree would release
(`committed_latency_record_is_current_with_the_release_version`), and must
exist, parse, and carry its environment
(`benchmark_note_cites_a_committed_raw_record`) — a deleted or orphaned
record, or one produced by a binary other than the current one, fails
`cargo test`.
