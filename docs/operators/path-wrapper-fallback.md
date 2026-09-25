# PATH-wrapper fallback for hookless harnesses

Some local harnesses have no blocking native PreToolUse hook that icg can
ride on. For those, icg's argv[0] PATH-wrapper mode is the available
fallback: the tools a rule pack names (`tool_keywords`) resolve to icg
first, icg evaluates the invocation, and only then `exec`s the real binary
found later in `PATH`. This guide deploys that fallback safely on a host
where the wrappers are not currently in use.

**A PATH wrapper is not equivalent to a native pre-tool hook and must never
be presented as such.** A hook sees every tool call the harness makes,
including structured `Write`/`Edit` payloads, before execution. A wrapper
only sees subprocess invocations that resolve through `PATH`. Everything
the wrapper cannot see is listed under [Blind spots](#blind-spots) below;
read it before trusting this deployment.

## The two pieces

| Piece | Path | Role |
| --- | --- | --- |
| Wrapper directory | `/usr/local/libexec/icg-wrappers` | root-owned, mode 0755, one symlink per pack `tool_keyword`, each pointing at `/usr/local/bin/icg` |
| Launch wrapper | `/usr/local/bin/icg-harness-env` | prepends the wrapper directory to `PATH` for one launched process tree only |

Both are managed by `scripts/deploy-path-wrappers.sh` in this repository.
Every default can be overridden (`--wrapper-dir`, `--icg`, `--pack-dir`,
`--launcher`), which is how the test suite exercises the same code
unprivileged against a staging tree.

### Why the directory is root-owned and outside the login PATH

A wrapper directory the guarded agent can write to is not a guard: the
agent could replace what its own `PATH` resolves to. The directory is
therefore created `root:root` 0755 (never group- or world-writable), and
`verify` fails if that drifts.

For the same reason the wrapper directory must appear **only** in the
target harness's launch environment — never in the operator's login shell,
`.profile`, or `.bashrc`. A global `PATH` entry shadows `git` for every
process the operator runs, including triage and incident response.
`icg-harness-env` is the sanctioned entry point: it `exec`s the harness
command with the wrapper directory prepended for that process tree only,
and its own shell's `PATH` is never modified.

## Deployment procedure

Run from a checkout of this repository. `install`, `canary`, and `remove`
need root for the default paths (they write under `/usr/local`).

### 1. Install

```sh
sudo scripts/deploy-path-wrappers.sh install
```

This creates the root-owned wrapper directory, asks `icg install` to
populate it with symlinks for every `tool_keyword` in the shipped packs
(`/etc/icg/packs` by default; override with `--pack-dir`), installs the
launch wrapper, and then **runs `verify` as part of install** — an install
that has not been verified is indistinguishable from a broken one. Install
refuses to proceed over a foreign non-symlink entry in the wrapper
directory rather than clobber it, and refuses an empty pack set (an empty
directory guards nothing).

`kubectl` is never shadowed: the shipped `kubectl` pack lists it in
`tool_keywords`, but `icg install` skips that keyword, so the pack guards
the hook front-end only. Cluster triage stays un-intercepted by a wrapper
on purpose; see [Blind spots](#blind-spots).

### 2. Run the canaries

```sh
sudo scripts/deploy-path-wrappers.sh canary
```

Five probes through the real wrapper directory, each harmless:

1. a fake target (`icgwrapcanary`) in a throwaway directory is reached
   through the wrapper — the guard ran, argv was preserved, and the real
   binary later in `PATH` executed;
2. an enforced denial blocks before the real binary runs (`git commit -m`
   with no pathspec, attempted from a directory that is not a git
   repository, so even a mis-enforcing wrapper can only reach git's own
   harmless error);
3. practice mode reports the same denial *and still execs* the real git —
   proving the report/block distinction is real;
4. a wrapper with no real binary later in `PATH` refuses with an explicit
   icg error instead of recursing or silently succeeding;
5. a second icg symlink later in `PATH` is skipped, not execed — wrapper
   recursion is impossible by construction.

The canaries clean up after themselves. **Do not enforce until they
pass.** Note that probe 2 deliberately does not use `git push --force`:
that pattern is a rewrite (`updated_input`), not a deny, so the wrapper
would strip the flag and exec git rather than refuse.

### 3. Trial in practice mode

```sh
/usr/local/bin/icg-harness-env --practice <harness command ...>
```

Practice mode (`ICG_PRACTICE=1`, exported to the harness process tree)
reports would-be denials and blocks nothing. Run the harness on real work
and review what would have been denied:

```sh
icg status --denials --pattern-summary --since 7d
```

### 4. Enforce

```sh
/usr/local/bin/icg-harness-env <harness command ...>
```

The same launcher without `--practice` is enforcing: denials block the
tool call before the real binary runs.

### Auditing the deployment

```sh
scripts/deploy-path-wrappers.sh status   # read-only report, PATH visibility included
scripts/deploy-path-wrappers.sh verify   # ownership, symlink integrity, resolution audit
```

`verify` checks that the directory is root-owned and not group/world-
writable, that every entry is a symlink to the icg binary, that `kubectl`
is absent, and that each wrapper resolves a *real, non-icg* binary later
in `PATH` without executing anything. A wrapped tool with no real binary
(for example `vault` on a host that only has `bao`) is a warning, not a
failure: the wrapper refuses it with an explicit icg error, turning a
silent `command not found` into an attributed refusal.

## Removal

```sh
sudo scripts/deploy-path-wrappers.sh remove
```

Removal is idempotent and preserves what it does not own: it removes only
symlinks that resolve to the icg binary (plus `icg install --uninstall`
for its own links), leaves unrelated entries — and therefore a non-empty
directory — in place, and removes the launch wrapper only if it is
byte-identical to the checked-in `scripts/icg-harness-env.sh`. The icg
binary and the rule packs are untouched. Re-running `remove` on an already
removed deployment succeeds. The systemd unit layer, if installed, is
unrelated and unaffected.

## Blind spots

The wrapper sees only a `PATH`-resolved subprocess `exec`. Each blind spot
below is real, documented here on purpose, and covered by a test in
`tests/path_wrapper_fallback_tests.rs` where it can be exercised
mechanically.

| Blind spot | What actually happens |
| --- | --- |
| Absolute-path invocation | `/usr/bin/git push …` bypasses the wrapper entirely; icg is never invoked. Only `PATH` resolution reaches the guard. |
| Structured edits | A harness `Write`/`Edit` tool call does not spawn a process; the wrapper cannot see it. Only the native hook front-end evaluates content. |
| MCP and library calls | Tools invoked over MCP, or destructive operations inside a program (a Rust binary calling git plumbing), never cross a shell `PATH` lookup. |
| `ICG_DISABLED=1` | The documented emergency bypass stands the guard down with a loud warning on stderr; the real binary runs without rule evaluation (the activation itself is recorded as an emergency-bypass event). `icg-harness-env` prints a warning when it is set at launch. |
| Tools without wrappers | Only pack `tool_keywords` get wrappers, and `icg install` hard-skips `kubectl` even though the shipped `kubectl` pack lists it (`never shadowed per policy`) — that pack guards the hook front-end only, so cluster triage stays un-intercepted. |
| Harness's own hook | If the harness has a native PreToolUse hook, that hook — not this fallback — is the enforcement path; deploying both double-evaluates Bash commands. |
| Every process on the launched PATH | The wrapper shadows tools for the whole launched process tree (build scripts, test suites), not only the agent's tool calls. Misfires there are the cost of the launch-scoped PATH. |

The launcher refuses to start a harness when the wrapper directory is
missing or holds no symlinks, so an unguarded launch cannot masquerade as
a guarded one. That check is a convenience, not a security boundary:
launching the harness without `icg-harness-env` is always possible, which
is one more reason this channel is a fallback and not a substitute for
the native hook.

## See also

- [Deployment guide](deployment-guide.md) — the native hook installation
  path and the manual version of the agent-scoped wrapper setup this
  guide automates.
- [Practice mode](practice-mode.md) — how practice reporting works across
  front ends.
- [Troubleshooting](troubleshooting.md) — "A PATH wrapper does not block
  a command" walks through every bypass cause listed above.
