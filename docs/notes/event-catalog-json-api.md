# `icg-catalog/v1` — the machine-readable always/never event catalog

ICG owns the authoritative list of events that must never happen — the
force-push, the mutating `kubectl` verb on an ArgoCD-managed resource, the
credential that reaches argv, the OpenBao destroy, the `:latest` tag — and
the always-allowed counterpart, because it is the component that enforces
them at the `PreToolUse` boundary. `icg catalog --json` is the supported way
to read that list. Consumers — TWILL's D-10 gate-gap detector first among
them — read this export instead of parsing rule packs they do not own, and
instead of keeping a second copy of the list that can drift from the one
doing the enforcing. This note fixes the contract;
`tests/catalog_export_tests.rs` pins it, so a shape change fails a build
until the format version moves with it.

## Where the catalog comes from

Every entry is rendered from the same source the engine dispatches on:

- Pack events come from the loaded pack files via
  `rule_pack::load_pack` — the identical result the engine consumes. No
  field is restated or edited by the exporter.
- The engine's built-in guards (`.github/workflows/` writes, `Job`/`CronJob`
  manifests) are code, not pack data, so their entries are built from the
  same constants the engine attributes their denials with
  (`src/github_workflows.rs`, `src/job_cronjob_yaml.rs`). A rename fails
  compile; a divergence between the catalog entry and the actual denial
  reason fails `builtin_catalog_entries_agree_with_engine_denials`.

There is no hand-maintained list anywhere in the pipeline.

## Invocation

```bash
icg catalog --json [--pack <path>]...
```

- `--json` is the documented spelling and is optional: JSON is the only
  output the catalog has, so `icg catalog` and `icg catalog --json` are the
  same command.
- `--pack` may be repeated. Each value is a rule-pack file or a directory;
  a directory contributes its `.json` entries. Repeated values naming the
  same path are deduplicated. A file whose name does not end in `.json` is
  not treated as a pack, even when it exists.
- With no `--pack`, the loader uses `ICG_PACK_DIR` when that environment
  variable is set, and otherwise tries `/etc/icg/rule-pack.json`,
  `/etc/icg/packs`, and the `packs/` directory relative to the working
  directory, and uses whichever of those exist — the same defaults as
  `coverage` and the hook itself.
- The two built-in guards are always included; they are not packs and
  cannot be excluded.

## Document shape

One JSON object on stdout, pretty-printed, with this exact top-level key
set:

| Field | Type | Meaning |
| --- | --- | --- |
| `format` | literal `"icg-catalog/v1"` | Identifies the contract. |
| `catalog_digest` | string | SHA-256 hex of the event set — the drift signal, defined below. |
| `icg_version` | string | The `icg` release that rendered the document, for provenance only. |
| `never` | array of event objects | Events that must never happen, ordered by (`pack`, `id`). |
| `always` | array of event objects | Events that are always allowed, ordered by (`pack`, `id`). |

Each entry of `never`:

| Field | Type | Meaning |
| --- | --- | --- |
| `id` | string | Stable event id — the same id a denial record carries as `pattern_id` and `icg explain --pattern` accepts. |
| `pack` | string | Owning pack id — the denial record's `pack_id`. Built-in guards use their synthetic pack ids `github-workflows` and `job-cronjob-yaml`. |
| `severity` | string | `Critical`, `High`, or `Medium`. |
| `tier` | string | Deterministic-difficulty tier: `tier1`, `tier2`, or `tier3` (lowercase; `coverage/v1` spells the same values `Tier1`… in its debug output). |
| `action` | string | The engine's response channel when caught, in the hook wire spelling: `deny`, `updated_input`, or `additional_context` (see [`pretooluse-response-schema.md`](pretooluse-response-schema.md)). |
| `destructive` | boolean | Whether the event guards an irreversible operation. |
| `enabled` | boolean | `false` when the rule ships disabled: cataloged so the whole policy surface stays visible, but the engine does not enforce it — a gap detector must not treat it as a hole in the gate. Built-in guards are code and always `true`. |
| `check` | string | How the event is recognized: `command_regex`, `content_regex`, or `predicate`. |
| `match` | object | What the check matches — a tagged union, exactly one key: `{"regex": "<pattern>"}` for the regex kinds (verbatim from the pack) or `{"predicate": "<name>"}` for an engine-side predicate. A predicate's logic is code in this repository; a consumer cannot re-implement it from the catalog. |
| `explanation` | string | Why this event must never happen. Non-empty for every entry. |
| `sanctioned_alternative` | object | The alternative the caller is owed instead — see below. |

Each `sanctioned_alternative`:

| Field | Type | Meaning |
| --- | --- | --- |
| `reason` | string | The alternative text. For pack rules this is the raw `reason_template` and may carry `{placeholder}` fields the engine fills from the matching invocation; for built-ins it is the literal denial reason. Non-empty for every entry — repo rule 2, tested. |
| `rewrite` | string, optional | Present only when `action` is `updated_input`: the rewrite template the engine substitutes. Presence is pinned to the channel by test. |

Each entry of `always`:

| Field | Type | Meaning |
| --- | --- | --- |
| `id` | string | Stable event id, as accepted by `icg explain --pattern`. |
| `pack` | string | Owning pack id. |
| `check` | string | `command_regex`, `content_regex`, or `predicate`. |
| `match` | object | Same tagged union as `never`. |

Always events carry no severity, action or alternative: matching one skips
the rest of its pack's guarded patterns, and being allowed *is* the
alternative.

## Event identity

An event is identified by the pair (`pack`, `id`) — exactly the attribution
a denial record carries. A consumer matching recorded denials against the
catalog needs no translation table, and TWILL's gate-gap detector is that
match: a denial-record (`pack`, `id`) that appears on a executed command
without a corresponding denial is a hole in the gate. Duplicate ids within
one pack are a pack-authoring bug and fail the export loudly rather than
silently collapsing two events into one.

## The digest — detecting drift

`catalog_digest` is the SHA-256 of the canonical JSON serialization of

```json
{"format": "icg-catalog/v1", "never": [...], "always": [...]}
```

— the entire event set and nothing else. Because the arrays are sorted by
(`pack`, `id`) and nothing installation-specific enters the document,
identical policy renders byte-identical catalogs, and any change to the
event set — a rule added, disabled, reworded, or removed — moves the
digest. A consumer stores the digest it last consumed and compares: equal
means the policy it reasoned about is unchanged, different means re-read.

The `icg` release version is deliberately **not** a digest input, so a
binary bump alone cannot masquerade as a policy change; `icg_version`
exists for provenance, not for drift.

## Ordering and determinism

- `never` and `always` are each ordered by (`pack`, `id`), regardless of
  pack directory iteration order, so a plain diff between two catalogs
  reads as a policy change, not a re-ordering.
- Two invocations over an unchanged pack tree emit byte-identical stdout.

## Unreadable packs fail the export

Unlike `coverage`, which reports unreadable packs in an `unreadable` array
and exits 0, the catalog has no such field — and for a reason: a document
describing only a *subset* of the packs on disk would be indistinguishable
from a legitimate policy change, so a consumer's digest comparison could
not tell a broken pack from an edit. A pack that fails to load — malformed
JSON, failed validation, an unreadable file — therefore fails the command:
exit non-zero, nothing on stdout, an `Error:` line on stderr. A broken pack
can never quietly shrink the catalog a consumer last saw.

## Relationship to other interfaces

- `coverage --format json` (`coverage/v1`) describes *packs*: what loaded,
  with the rules in pack-file order and debug-spelled enums. It is the
  pack-author's view. The catalog is the *event* view consumers reason
  about, keyed by denial attribution, with hook-wire spellings.
- `icg explain --pattern <id>` renders one event's full caller-facing
  redirect. The catalog's `id` is the same key.
- Per-repo runtime overrides change what the hook *does* for a specific
  repository; they are not folded into the catalog. The catalog describes
  the policy surface as shipped, which is the thing that must not drift.

## Versioning

The literal `format` value is `icg-catalog/v1`. Consumers key on the full
key sets above, and those sets are pinned by test: adding, removing, or
renaming a field breaks the build on purpose. Any shape change bumps the
version string (`icg-catalog/v2`) and rewrites this note in the same
commit, so a consumer is never surprised by a field it has not heard of.
The digest changes constantly by design; only `format` marks a contract
change.
