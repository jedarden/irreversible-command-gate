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
icg regression-prune --path tests/regression --max-cases 256
```

The pruning command removes exact duplicates and trims files to the requested
bound, preserving first-observed order. It does not turn the traffic corpus
into the release gate automatically: curated cases still need the normal
review and fixed-suite generation workflow before they become release-bound
tests.
