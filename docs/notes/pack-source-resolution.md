# Operator pack-source resolution contract

**Status: shipped.** This note settles the source selection used by the
operator commands, and the versioned API notes carry the source label.
The hook's installed-only behavior is already a separate contract and is not
changed by this note.

The problem is simple to state: a checkout can contain newer or additional
packs than the release installed on the host. If an operator command reports
the union, it can claim that the hook enforces a rule that the hook never
loaded. A source label must make the selected policy observable, and the
resolver must select one source rather than quietly combining trust and
working-tree data.

## Terminology

An **explicit path** is a value supplied with `--pack` (the existing
`--rule-pack` alias is equivalent). A path may name one `.json` pack file or a
directory whose sorted `.json` entries are the pack set. Repeated `--pack`
values are one deliberate explicit selection: all of the named paths are
used, deduplicated and sorted as they are today.

`ICG_PACK_DIR` is the operator-command development override. It is not a
second source to merge with the defaults; when it is set, its one directory
is the complete selected pack set. Its reports use the `explicit` source
origin because the caller deliberately selected it, even though the human
label identifies it as `ICG_PACK_DIR`.

The **installed source** is the trust-directory chain that represents the
release the hook is intended to enforce:

1. `/etc/icg/packs`, the modular directory, when that location is present;
2. otherwise `/etc/icg/rule-pack.json`, the legacy single-file artifact.

The two installed locations are alternatives, not a union. The
`ICG_INSTALLED_PACK_DIR` variable used by tests may substitute a staged
installed directory for this chain; it is an operator-test seam, not a user
development override and is never read by the hook.

The **repository source** is the working directory's `packs/` directory. It
is a fallback for a bare checkout or CI runner with no installed release.

## Precedence: first match, never a union

For `check`, `coverage`, `status`, and `catalog`, one invocation resolves
packs in this exact order:

1. explicit `--pack` paths;
2. `ICG_PACK_DIR`, when no `--pack` was supplied;
3. the installed source (`/etc/icg/packs`, otherwise the legacy artifact);
4. the repository source (`packs/`) when no installed source exists.

The first applicable source wins. Once a source wins, lower-priority sources
are not inspected, loaded, merged, or used to fill gaps. This is a source
tier decision, not a per-pack-name decision: an installed source wins as a
whole, so a checkout-only pack does not join the report and a checkout copy
of an installed pack cannot replace it.

Within a selected directory, only `.json` entries are candidates and their
paths are sorted. The existing pack validation and duplicate-id rules still
apply to the selected set. An explicit invocation may intentionally select
multiple files or directories; those explicit inputs are the one winning
source and do not cause the installed or repository defaults to be read.

The installed chain is also first-match: if `/etc/icg/packs` is present, the
legacy file is not consulted, even if both exist. If the modular directory is
absent, the legacy file may win. If both installed locations are absent, the
resolver may proceed to the repository fallback.

## Merging, duplicates, and conflicts

Source tiers are never merged. A selected directory contributes all of its
`.json` entries; repeated explicit paths are deduplicated, and multiple
distinct explicit paths are one deliberate selected set. `ICG_PACK_DIR` is
one selected file or directory, not an additive layer over `--pack`, the
installed source, or the checkout source.

After loading the selected set, two different files that declare the same
pack `id` are a conflict. Operator commands fail before evaluating or
publishing that ambiguous policy; they do not merge the rules, choose the
first file, or let the last file overwrite it. This keeps coverage, catalog,
health, explanation, and check behavior consistent with the engine's pack-id
index. A repeated path is harmless because path resolution removes it before
loading; two files with the same id are not harmless.

The same id in a lower-priority source is not a conflict: that source is not
inspected after the winning tier is selected. Thus an installed pack and a
checkout copy can have the same id without a collision warning, because the
checkout copy is outside the invocation. The source label is the diagnostic
that matters; text output does not claim a cross-source merge or emit a
shadow warning. `pack-drift` is the explicit comparison tool and reports a
duplicate id on either side as `CONFLICT` and drift rather than claiming the
sets are byte-identical.

## Missing, empty, and unreadable locations

The resolver distinguishes an absent default from a broken location. This
prevents a permissions or deployment failure from being disguised as a
checkout policy:

| Candidate | Contract |
| --- | --- |
| Missing explicit `--pack` path | Error; do not use `ICG_PACK_DIR`, installed packs, or checkout packs. |
| Missing `ICG_PACK_DIR` path | Error; do not fall back to installed or checkout packs. |
| Missing installed modular directory | Try the legacy installed artifact. |
| Missing both installed locations | Try checkout `packs/`. |
| Present but unreadable installed location | Error; do not fall back to checkout or the lower installed candidate. |
| Present installed location with no `.json` packs | Error; it is a broken selected installation, not proof that checkout policy is safe to use. |
| Present but unreadable/empty checkout location | Error; there is no lower-priority source. |
| Present but unreadable/empty explicit or override location | Error; never silently use another source. |

An unreadable individual `.json` file is a load error after its source has
already won; it does not reopen source selection. The existing command
contracts remain in force: coverage records failed files in its `unreadable`
array and refuses to emit an empty report, while `check`, `status`, and
`catalog` do not evaluate or publish a partial policy when their selected
pack set cannot be loaded completely. No command may turn an unreadable
installed file into a checkout fallback.

## Collision examples

These examples use `git.json` as a deliberately colliding pack id.

### Installed wins over checkout

Suppose the host has:

```text
/etc/icg/packs/git.json       # git rule set from the installed release
checkout/packs/git.json       # a different development copy
checkout/packs/kubectl.json   # checkout-only rule set
```

With `checkout` as the working directory, an unqualified
`icg coverage --list` reports only the installed `git.json`, labels the
source `installed`, and does not report `kubectl.json`. The installed copy of
`git.json` wins byte-for-byte; there is no merge, replacement, or collision
warning because the checkout source was not consulted. `icg pack-drift` is
the separate command for comparing the installed release against a checkout
artifact.

If `/etc/icg/packs` is absent but `/etc/icg/rule-pack.json` exists, the
legacy artifact is the installed winner and `checkout/packs/` is still not
read. If both installed locations are absent, the same command reports the
checkout's packs and labels the source `repository`.

### An explicit source wins over both

With both installed and checkout packs present,
`icg coverage --list --pack /tmp/review-packs` reports only
`/tmp/review-packs`. If that path is missing, the command errors; it does not
quietly inspect `/etc/icg` or `packs/`. The equivalent development override
is `ICG_PACK_DIR=/tmp/review-packs icg coverage --list`; it has the same
no-fallback rule and is labeled as an explicit development source.

## Hook isolation

The hook and the operator resolver have intentionally different entry
points, but neither may broaden the other's trust boundary:

- The hook reads its explicit rule-pack argument or `ICG_RULE_PACK`, then its
  installed chain, and never discovers `./packs/` from the working directory.
- The hook never reads `ICG_PACK_DIR` or `ICG_INSTALLED_PACK_DIR`.
- `icg check --pack packs` is an explicit developer analysis; it does not
  alter or imply what a hook invocation enforces.
- An operator command's default installed label describes the trust source it
  selected, not a checkout that happens to be the current directory.

This keeps a checkout-only pack out of hook evaluation even when an operator
is running commands from that checkout. The engine's fail-open behavior for
evaluation errors remains unchanged; source resolution errors are reported
by the operator command instead of being hidden by a lower-priority source.

## Source labels and affected CLI surfaces

Every affected operator surface identifies the one selected source. The
machine-readable origin vocabulary is deliberately small and stable:

| Origin | Meaning | `root` |
| --- | --- | --- |
| `explicit` | Caller-supplied `--pack` paths or `ICG_PACK_DIR`. | `null` for repeated `--pack` paths; the selected directory for `ICG_PACK_DIR`. |
| `installed` | `/etc/icg/packs` or the legacy installed artifact. | The winning installed location. |
| `repository` | The working directory's `packs/` fallback. | The winning checkout directory. |

`trusted_ref` is present only for an `installed` result, when the installed
release reference can be read. It is `null` for `explicit` and `repository`
results; a checkout must never inherit a trusted-release claim.

The output rules are:

- `coverage --list` prints one `Pack source:` line naming the origin and root
  before the text listing. `coverage --list --format json` adds a
  `pack_source` object with `origin`, `root`, and `trusted_ref`; it emits no
  human header on stdout.
- `check --debug` prints the source label on stderr with its other diagnostic
  lines. Plain `check` keeps stdout reserved for the decision and does not
  add a source line there.
- `status` prints the selected source in its `## Operator Pack Source`
  section. The `health` report selects the same source precedence but is a
  line-oriented human report and does not print a source header; use coverage,
  catalog, `check --debug`, or `status` when the label itself is needed.
- `catalog --json` adds the same `pack_source` object to the JSON document.
  The label describes the pack-derived portion; the catalog's built-in
  guards are not files in any pack source.

The shipped `pack_source` field changes the coverage wire shape to
`coverage/v2` and the catalog shape to `icg-catalog/v2`. The versioned API
notes [`coverage-json-api.md`](coverage-json-api.md) and
[`event-catalog-json-api.md`](event-catalog-json-api.md) define those exact
fields and failure semantics. Source labels must never be emitted as extra
prose on a JSON command's stdout.

## Explicit missing-location examples

These examples pin the no-silent-fallback rule:

```bash
# Even if /etc/icg/packs and ./packs exist, this is an error.
icg coverage --list --pack /tmp/no-such-packs

# The development override is also authoritative when set.
ICG_PACK_DIR=/tmp/no-such-packs icg catalog --json
```

Conversely, when neither `/etc/icg/packs` nor
`/etc/icg/rule-pack.json` exists, a checkout containing a readable `packs/`
directory is a valid `repository` fallback. That fallback is the only
default fallback; it is never used to repair an explicit, empty, or
unreadable higher-priority location.
