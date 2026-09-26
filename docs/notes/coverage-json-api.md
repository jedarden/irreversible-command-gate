# `coverage/v1` — the machine-readable coverage API

`icg coverage --list --format json` is the supported machine interface to
the enforced policy. Agents deciding whether a command will be denied
before they try it, bots rendering the policy, and doc generators read
this document instead of scraping `coverage --list`'s text. This note
fixes the contract; `tests/coverage_json_tests.rs` pins it, and
`tests/coverage_note_sync_tests.rs` pins these field tables against the
emitted document, so a shape change — in code or in this note — fails a
build until the format version moves with it.

## Invocation

```bash
icg coverage --list --format json [--pack <path>]...
```

- `--list` is the documented spelling carried over from the text mode. It
  is accepted with `--format json` and is optional there; future modes may
  give it meaning.
- `--pack` may be repeated. Each value is a rule-pack file or a directory;
  a directory contributes its `.json` entries. Repeated values naming the
  same path are deduplicated. A file whose name does not end in `.json` is
  not treated as a pack, even when it exists.
- With no `--pack`, the loader uses `ICG_PACK_DIR` when that environment
  variable is set, and otherwise tries `/etc/icg/rule-pack.json`,
  `/etc/icg/packs`, and the `packs/` directory relative to the working
  directory, and uses whichever of those exist.
- Any other `--format` value is rejected with
  `unsupported --format ...; use "text" or "json"` before anything is
  written to stdout.

## Document shape

One JSON object on stdout, pretty-printed, with this exact top-level key
set:

| Field | Type | Meaning |
| --- | --- | --- |
| `format` | literal `"coverage/v1"` | Identifies the contract. |
| `packs` | array of pack objects | Every pack that loaded, one entry per resolved pack file. |
| `unreadable` | array of load errors | Every pack path that was attempted and failed, with the reason. |
| `pack_count` | number | `packs.length`. Readable packs only. |
| `guarded_pattern_count` | number | Sum of `guarded_patterns.length` across `packs`. Readable packs only. |

Each entry of `packs`:

| Field | Type | Meaning |
| --- | --- | --- |
| `id` | string | The pack's `id`, as in `coverage --list`. |
| `path` | string | The resolved path the pack was loaded from. |
| `tool_keywords` | array of strings | The pack's `tool_keywords`, verbatim. |
| `applies_to` | array of strings | The pack's `applies_to`, verbatim. |
| `safe_patterns` | array of strings | The safe patterns' ids (allow-list members, not rules). |
| `guarded_patterns` | array of rule objects | The pack's rules, in the pack file's own order. |

Each entry of `guarded_patterns`:

| Field | Type | Meaning |
| --- | --- | --- |
| `id` | string | Rule id, the same id `icg explain --pattern` accepts. |
| `enabled` | boolean | A disabled rule is reported but not enforced. |
| `tier` | string | Debug spelling of the rule's tier: `Tier1`, `Tier2`, or `Tier3`. |
| `severity` | string | `Critical`, `High`, or `Medium`. |
| `channel` | string | One of `Deny`, `UpdatedInput`, `AdditionalContext`. These are the engine's channel names, not the hook's lowercase wire names — see [`pretooluse-response-schema.md`](pretooluse-response-schema.md) for the hook side. |
| `destructive` | boolean | Whether the rule guards an irreversible operation. |
| `check` | string | One of `command_regex`, `content_regex`, `predicate`. |
| `explanation` | string | Why the rule exists. Non-empty for every shipped rule. |
| `redirect` | string | The alternative the rule owes the caller. Non-empty for every shipped rule — repo rule 2, tested. |

Each entry of `unreadable`:

| Field | Type | Meaning |
| --- | --- | --- |
| `path` | string | The pack path that failed to load. |
| `error` | string | The load error, non-empty. |

## Serialization contract

- Field order in the emitted document is the declaration order in
  `src/documented_commands.rs` (`format`, `packs`, `unreadable`,
  `pack_count`, `guarded_pattern_count`, and so on down the levels).
- `packs` is ordered by resolved path: explicit `--pack` values are
  deduplicated and sorted, and a directory's entries are sorted. Rule
  order inside a pack is the pack file's own declaration order.
- Two invocations over an unchanged pack tree emit byte-identical stdout.
  A doc generator may cache the output and a bot may diff coverage
  between releases byte-for-byte.

## Unreadable packs

An unreadable pack is a silent coverage hole in the text listing; here it
is a field you can assert on.

- A pack that fails to load — malformed JSON, failed validation, an
  unreadable file — does not abort the command. It is recorded in
  `unreadable`, and the command still exits 0 and reports every pack that
  did load.
- `pack_count` and `guarded_pattern_count` never include an unreadable
  pack. The counts describe only what loaded, so a consumer that ignores
  `unreadable` reads a smaller policy than the one on disk.
- A consumer whose decision depends on the documented coverage therefore
  must treat a non-empty `unreadable` as a failure. The command itself
  stays exit-0 so a pure reporting pipeline does not break on one bad
  file.

## Failure modes

These exit non-zero, write nothing to stdout, and put an `Error:` line on
stderr. The command never emits a `coverage/v1` document with an empty
`packs` array: an empty report is indistinguishable from "nothing is
enforced", so it is refused instead of printed.

| Input | stderr |
| --- | --- |
| Every requested pack failed to load | `no readable rule packs were found` |
| `--pack` names a path that does not exist | `rule-pack path does not exist: <path>` |
| `--pack` names a directory with no `.json` entries | `no rule packs found; pass --pack <path>` |
| No `--pack` and no default location yields a pack | `no rule packs found; pass --pack <path>` |

## Versioning

The literal `format` value is `coverage/v1`. Consumers key on the full
key sets above, and those sets are pinned by test: adding, removing, or
renaming a field breaks the build on purpose. Any shape change bumps the
version string (`coverage/v2`) and rewrites this note in the same commit,
so a consumer is never surprised by a field it has not heard of.
