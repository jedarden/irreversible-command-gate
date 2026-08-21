# Traffic-derived deny regression corpus

The hook recorder is opt-in. Run the hook with `--record-as-test` (or give it
a directory, such as `--record-as-test tests/regression`) to append denied
inputs as newline-delimited JSON cases in
`tests/regression/<pack-id>.cases`. Without the flag, normal hook behavior and
the fixed generated regression suite are unchanged.

The recorder ignores exact duplicates and stops at 256 cases per pack. A full
corpus is a signal to curate; it is not silently expanded. Review the cases,
remove noisy or redundant entries, and run the explicit curation pass when
needed:

```text
icg regression-prune \
  --path tests/regression \
  --rule-pack packs/git.json \
  --rule-pack packs/openbao.json \
  --max-cases 256
```

Pass every current manifest represented in the corpus. Pack-aware pruning
drops cases whose pack or enabled deny `guarded_pattern` no longer exists, and
drops inputs that no longer match a current deny pattern. If a rule was
renamed but still denies the observed input, the case is reassigned to the
current pattern ID. It then removes exact duplicates and trims each file to
the requested bound, preserving first-observed order.

Without `--rule-pack`, the command performs only the structural duplicate and
bound pass; this is useful when manifests are unavailable but does not remove
stale rule references. The traffic corpus never becomes the release gate
automatically: curated cases still need normal review and fixed-suite
generation before they become release-bound tests.
