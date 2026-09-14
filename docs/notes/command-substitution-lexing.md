# Command substitution (`$( )`) in the lexer — decision record

Parent bead: `irrevers-3e313b79` (the lexer does not recurse into `$( )`).
Characterization suite: `tests/command_substitution_lexing_tests.rs`, whose
`#[ignore]`d tests reference child `irrevers-f7201bd7` and exist to be
un-ignored by the implementation child.

## The problem

`lex_shell_commands` has no `$( )` awareness — `$` and `(` are ordinary word
characters. Consequences, in the quoting context where each applies:

- **Top level**: the words inside `$( )` are lexed almost normally (a `<<`
  heredoc there is honored, `;`/`&&` still split commands), but the opening
  `$(` glues onto the first inner word. That first command dispatches as
  executable `$(cat` / `$(bao` and matches no pack keyword; only inner
  commands *after* the first separator surface.
- **Inside a double-quoted word**: the whole substitution is one inert blob
  until a `"` inside the nested context closes the outer word. For the
  corpus shape

  ```
  git commit -q -m "$(cat <<'EOF'
  fix: openssl-sys's script says "Could not find"
  EOF
  )"
  ```

  the message arrives as three fragmented argv words after `-m`, and the
  anchored `git-commit-without-pathspec` regex cannot match the fragment.
  That is the residual ~129 of 476 `git commit` invocations still missed
  after `irrevers-9ef830ec`.
- **Inside a single-quoted word**: literal text. Already correct.
- **Backticks**: see the deferral below.

Not a bypass: prose inside such a message still allows, and a real
destructive command after it still denies (verified on the parent bead; both
properties are pinned live in `tests/heredoc_lexing_tests.rs` and
`tests/command_substitution_lexing_tests.rs`). The cost is missed
`git-commit-without-pathspec` coverage, plus the glued first inner command.

## The decision

**Chosen: a nested-context stack in the lexer.** When the lexer sees `$(` it
pushes the current quoting state and lexes the interior as commands (with
its own quote state, heredocs, and separators), popping back to the outer
word at the matching `)`.

**Rejected: relaxing the anchored `git-commit-without-pathspec` regex** so
it tolerates fragments. Two reasons:

1. Fragment-tolerant matching invites false positives on prose. A rule that
   can match `git commit -m <fragment>` can also match a message that merely
   *contains* commit-like text; the anchoring is what keeps quoted prose in
   the "data, not invocation" column.
2. Every other anchored command rule has the same dependency on seeing a
   complete invocation. Fixing the regex would repair one rule's symptom
   while leaving the lexer to hand every other anchored rule the same
   fragments. The lexer is the single place where the defect exists, so it
   is the single place to fix.

This is consistent with the ideas ledger's kill of full AST-based shell
parsing (ledger #35): that was killed because its precision gain mostly
matters for adversarial evasion, which is out of scope for the threat model.
A nested-context stack is not an AST parser — it is the minimal extension
that stops mis-tokenizing a shape real (non-adversarial) agent traffic
produces in volume.

**Deferred: backticks.** `` ` `` gets no substitution treatment today either
(see the live `backtick_substitution_is_explicitly_deferred`
characterization test); the nested-context work covers `$( )` only. Backtick
command substitution is rare in agent traffic compared with `$( )`, and the
deferral is a scope decision, not a claim that backticks are safe to leave
unlexed forever. A bead that adds backtick nesting must flip that test in
the same change.

## Shipped (irrevers-c6f6c7b8, 2026-09-13)

Implemented as decided: a `LexContext` stack in `lex_shell_commands`. `$(`
opens a nested context everywhere the shell would run it — top level and
inside a double-quoted word — and the matching `)` pops back to the
surrounding word. Per-context quote, escape, heredoc, and word state; unquoted
`(`/`)` inside a substitution track grouping depth so `$( (rm -rf /) )` pops
on the right paren; `$(( ))` is consumed as an arithmetic word (never nests,
never swallows a separator — the scan bails to ordinary lexing on any quote or
metacharacter); a substitution inside a single-quoted word stays literal.

Two consequences worth recording:

- **Captured substitutions keep their pre-fix dispatch.** `TOKEN=$(git
  credential fill ...)` must stay allowed (the git pack's
  `safe-git-credential-fill-captured` allowance), so when the word around
  `$(` is an env assignment awaiting its value, the assignment target prefixes
  the substitution's first word and the token dispatches exactly as the glued
  `TOKEN=$(git` word always did. Any other interior word starts clean — the
  shell runs `rm` in `a$(rm -rf /)` even though the output concatenates, so
  that command now dispatches where it used to hide. The residual gap: a
  *captured* destructive command (`TOKEN=$(bao kv destroy ...)`) still
  dispatches as `kv`, invisible to packs, exactly as before this change. It
  is pre-existing, not introduced here.
- **One characterization test was re-pinned, not just un-ignored.**
  `requoted_prose_does_not_become_an_invocation` had pinned `!denied` for
  `git commit -m "$(cat <<'EOF' ... )"` — but with the message arriving whole,
  that shape is a genuine pathspec-less commit and `git-commit-without-
  pathspec` now matches it, which is precisely the missed coverage the parent
  bead filed. No single-word message content can distinguish it from the
  `-q` variant that must deny, so the test now pins the refined property: the
  denial must come from `git-commit-without-pathspec` (the commit's shape),
  never from the words inside the message (no `bao` executable, no openbao
  denial). `requoted_prose_with_a_pathspec_stays_allowed` pins the other
  half.

## Acceptance

Met, verified 2026-09-13: all five `#[ignore]`d tests are un-ignored and
pass; the full suite passes (734 tests, 0 failed, 0 ignored); a differential
harness comparing the pre-change lexer against this one over 36 `$(`-free
inputs (heredocs, herestrings, escapes, separators, prefix stripping,
unterminated quotes) produced identical segmentation on every one.
