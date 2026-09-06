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

No input that was previously denied is now allowed. The remaining misses are
`irrevers-3e313b79` (the lexer does not recurse into `$( )`), not this rule.
