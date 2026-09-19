# Recorded coverage justifications

`icg coverage-diff` stops a release when a guarded pattern is removed,
disabled, narrowed, or has its regex changed at all — it cannot tell a
widening from a narrowing, so it flags both and asks a human. That is the
right default, but CI has no way to carry a human's approval, so a
legitimate widening would block the pipeline indefinitely.

This file is that approval, in a form CI can check. When `coverage-diff`
flags a pattern, CI requires a `## <pattern-id>` stanza here and fails
otherwise, naming the patterns that lack one.

**A stanza approves only the pattern it names.** Changing a different rule
still stops CI until someone writes down why, so this cannot become a
blanket waiver. Remove a stanza once the released baseline has caught up and
the finding no longer appears.

---

## git-commit-without-pathspec

**Widened, not narrowed.** *2026-09-06, against the v0.1.2 baseline.*

The engine re-quotes a command before matching, so a message containing
quotes becomes the shell escape `'\''`. The previous message alternation
`'[^']*'` stopped at the first inner quote and the anchored match failed, so
the rule silently declined to fire. The new alternation accepts that
re-quoted form.

Measured by replaying 20,007 unique real agent Bash commands, taken from the
last 1,500 agent transcripts on ex44:

| | before | after |
| --- | --- | --- |
| `git commit` invocations denied | 35 | **106** |
| of 476 total | | |
| false positives against the 152 that correctly pass a pathspec | 0 | **0** |

No input that was previously denied is now allowed. The remaining misses at
that measurement were attributed to the lexer's lack of `$( )` recursion —
since fixed (irrevers-c6f6c7b8, 2026-09-13) — not to this rule.

**Re-measured 2026-09-19, after the `$( )` lexer fix.** The corpus was
rebuilt the same way (30,963 unique commands, the last 1,500 transcripts —
method and reproducibility: `docs/notes/traffic-corpus-replay.md`) and
replayed against the engine immediately before and after the lexer change:

| | before | after |
| --- | --- | --- |
| `git commit` invocations denied | 198 | **217** |
| of 881 total | | |
| false positives against commits that correctly pass a pathspec | 1 | **0** |

Every one of the 20 newly denied inputs is a genuine pathspec-less
`git commit -m "$(cat <<'EOF' …)"` invocation — the population the lexer gap
was hiding — and no rule outside the git pack changed verdict on any input.
The one input the fix newly allows was a **false positive before**: a
heredoc-message commit passing an explicit `-- <paths>` pathspec that the
pre-fix lexer fragmented into a match shape. Pathspec-passing commits now
have zero denials.

Of the 246 pathspec-less commit invocations, 217 are denied and 29 still
miss — all pre-existing rule-shape gaps unrelated to the lexer, present
identically in both engines: 18 use a global option before the subcommand
(`git -C <path> commit …`, `git -c k=v commit …` — the anchored regex
requires `git\s+commit`), 9 pass the message via `-F <file>`/`-F -`, 2 are
`--amend --no-edit`, and 1 uses a short-cluster message flag (`-qm …`).
