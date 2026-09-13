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

## Acceptance

The implementation child is done when the `#[ignore]`d tests in
`tests/command_substitution_lexing_tests.rs` (and the ignored
`a_requoted_commit_message_still_reaches_the_pathspec_rule` in
`tests/heredoc_lexing_tests.rs`) are un-ignored and pass, and the corpus
replay from the parent bead shows no new false positives.
