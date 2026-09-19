# Traffic-corpus replay — method and recorded measurements

How the coverage numbers in `packs/coverage-justifications.md` are produced,
and how to reproduce them. Three pieces, all in the repo:

| Piece | Role |
| --- | --- |
| `scripts/extract-traffic-corpus` | builds the corpus from agent transcripts |
| `src/bin/corpus-replay.rs` | replays a corpus through `Engine::evaluate_command` |
| `scripts/analyze-corpus-replay` | diffs two replay result files |

## Corpus derivation

The corpus is the set of **unique real agent Bash commands** taken from the
most recent Claude Code session transcripts under `~/.claude/projects`:

```bash
scripts/extract-traffic-corpus --out /tmp/traffic-corpus.jsonl
```

- Reads the newest `--limit` (default 1500) `*.jsonl` transcripts, newest
  first, and collects the `command` value of every `Bash` `tool_use` entry.
- Deduplicates, preserving first-seen order (deterministic for a given
  transcript set). Output is JSONL, one `{"command": …}` per line, mode 600.
- The corpus is verbatim agent traffic: it can contain secret-shaped text.
  It is **never committed, never printed, and never written inside a
  repository.** The script prints counts only; the analyzer masks
  secret-shaped runs before showing any individual input.

The corpus is time-anchored — transcripts accumulate, so re-running the
extraction a day later yields a different (larger) set. That is expected:
measurements are comparable through their before/after *deltas on the same
corpus*, not through absolute counts across rebuilds.

## Replay and comparison

`corpus-replay` loads the packs from `packs/` and evaluates every corpus
line through the same `Engine::evaluate_command` path the hook uses. No
telemetry store or state store is constructed, so evaluation is
side-effect-free. It writes one result per corpus line, **positional** —
line N of the results is line N of the corpus — carrying verdict, pack id,
pattern id, and the engine's segmentation. Command text is deliberately not
copied into the results: the corpus stays the only file with verbatim
traffic.

To compare across an engine change, build the harness twice from clean
extractions of the two trees (the harness file is copied into the older
tree unchanged; it compiles against both API surfaces):

```bash
git archive <before-commit> | tar -x -C /tmp/icg-before
git archive <after-commit>  | tar -x -C /tmp/icg-after
cp src/bin/corpus-replay.rs /tmp/icg-before/src/bin/
cp src/bin/corpus-replay.rs /tmp/icg-after/src/bin/
(cd /tmp/icg-before && cargo build --release --bin corpus-replay)
(cd /tmp/icg-after  && cargo build --release --bin corpus-replay)

/tmp/icg-before/target/release/corpus-replay packs /tmp/traffic-corpus.jsonl /tmp/results-before.jsonl
/tmp/icg-after/target/release/corpus-replay  packs /tmp/traffic-corpus.jsonl /tmp/results-after.jsonl

scripts/analyze-corpus-replay /tmp/results-before.jsonl /tmp/results-after.jsonl --detail-dir /tmp/details
```

Classification of the `git commit` population uses the **after** tree's
segmentation, since that lexer sees the true command structure; both verdict
columns are then measured over that one population. The analyzer reports:
denials within the population per engine, verdict transitions, newly denied
inputs by pack/rule, newly allowed inputs (regression candidates), and
false-positive candidates (denied by `git-commit-without-pathspec` despite a
pathspec being present). Review individual inputs with
`--corpus … --show INDEX`, which masks long secret-shaped runs.

## Recorded measurements

### 2026-09-06 — the re-quoting widening (v0.1.2 baseline)

20,007 unique commands from the last 1,500 transcripts on ex44.
`git commit` invocations denied by this rule: 35 → **106** of 476; false
positives against the 152 pathspec-passing commits: 0 → **0**. No input
previously denied became allowed. Remaining misses attributed to the lexer's
lack of `$( )` recursion (irrevers-3e313b79).

### 2026-09-19 — after the `$( )` lexer fix (irrevers-c6f6c7b8)

Corpus rebuilt: 30,963 unique commands from the 1,500 most recent
transcripts (31,965 Bash calls seen). Trees compared: `43ac754^` (before) vs
HEAD (after). Whole-corpus verdict totals: before 30,763 allowed / 199
denied / 1 rewrite; after 30,744 allowed / 218 denied / 1 rewrite.

- `git commit` invocations (post-fix segmentation): 881. Denied by
  `git-commit-without-pathspec`: 198 → **217**.
- False positives against pathspec-passing commits: **1 → 0**. The one
  before-side false positive was a heredoc-message commit passing an
  explicit `-- <paths>` pathspec; the pre-fix lexer fragmented the message
  into a shape the anchored regex matched. The post-fix lexer delivers the
  message whole and the pathspec is honored.
- Newly denied by any pack: 20, all `git/git-commit-without-pathspec`, all
  genuine pathspec-less `git commit -m "$(cat <<'EOF' …)"` invocations. No
  rule outside the git pack changed verdict on any input (30,942 of 30,963
  inputs unchanged).
- Residual misses: of 246 pathspec-less commit invocations, 29 remain — all
  pre-existing rule-shape gaps present identically in both engines: 18 with
  a global option before the subcommand (`git -C <path> commit …`,
  `git -c k=v commit …`), 9 with the message via `-F <file>`/`-F -`, 2
  `--amend --no-edit`, 1 short-cluster `-qm …`. None is a `$( )` shape.
