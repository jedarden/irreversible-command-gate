# Installation and deployment guide

This guide describes how to install the current `icg` command, load a
released rule pack, connect the native hook adapter, and operate upgrades.
It is intentionally written against the command and file paths implemented in
this repository.

## Before you begin

`icg` is a per-invocation guard. A hook process reads one PreToolUse JSON
object from standard input, evaluates it, writes a JSON decision, and exits.
It does not run as a daemon and there is no service to start or restart.

The supported production path in the current tree is the native hook adapter:

```text
Claude Code or local Codex CLI
        │  PreToolUse JSON on stdin
        ▼
    icg hook
        │  deny / updatedInput / additionalContext JSON
        ▼
     harness
```

The `icg wrapper` subcommand is not a production wrapper yet. It currently
parses and prints command segments and then allows the invocation; it does
not locate or execute the real `git`, `vault`, or other binary. Do not create
PATH-shadowing symlinks to this build. The native hook is the deployment
mechanism to install.

The guard is a backstop for honest mistakes, not a boundary against a
malicious process. Local host hooks do not cover cloud-hosted agent jobs,
direct library calls, or tools that bypass the configured hook. Keep the
harness's own approval and sandbox controls enabled.

## Choose an installation scenario

| Scenario | Use this procedure | Network needed at runtime |
| --- | --- | --- |
| Build and test on one host | [Source installation](#source-installation) plus [direct smoke test](#7-verify-the-installation) | No |
| Install for Claude Code | Source installation plus [Claude Code hook](#claude-code) | No |
| Install for local Codex CLI | Source installation plus [Codex CLI hook](#codex-cli) | No |
| Host has no release-network access | [Offline/manual rule-pack installation](#offline-or-manual-rule-pack-installation) | No, after artifacts are copied |
| Roll out a canary release | [Channel-specific rollout](#channel-specific-rollout) | Depends on the approved artifact source |
| Install an approved binary release | [Package and artifact distribution](#package-and-artifact-distribution) | Depends on the artifact source |

## System requirements

### Runtime host

- A Unix-like host with the Linux layout used by the deployment (`/usr/local/bin`,
  `/etc/icg`, and `/var/cache/icg`). Linux is the supported production target.
- A user account that can read `/usr/local/bin/icg` and `/etc/icg/packs/`.
- Root or an equivalent deployment identity to install the binary and
  administrator-controlled artifacts.
- Claude Code or a local Codex CLI version that supports a `PreToolUse`
  command hook, if hook integration is wanted. Hook configuration is owned by
  each harness and can change; verify the version's current hook schema before
  applying it.
- `curl` or another approved artifact-transfer tool is useful for manual
  distribution. It is not required by `icg hook`.

`icg hook` evaluates commands without network access. Only `icg update` talks
to the GitHub Releases API, and it does so once when explicitly invoked.

### Build host

Building from this repository requires:

- Rust and Cargo (stable is recommended; the crate uses Rust edition 2021).
- A C compiler and linker.
- `pkg-config` and OpenSSL development headers because the current `reqwest`
  dependency uses the platform TLS backend.

On Debian or Ubuntu, the usual prerequisites are:

```bash
sudo apt-get update
sudo apt-get install --yes build-essential pkg-config libssl-dev
```

On Fedora or RHEL-like systems, use the equivalent packages:

```bash
sudo dnf install gcc gcc-c++ make pkgconf-pkg-config openssl-devel
```

The repository does not currently declare an MSRV, provide a vendored OpenSSL
build, or ship a Windows installer. Confirm the toolchain and OS used by the
release process when producing a fleet artifact.

### Resource and permission expectations

The guard is not a long-running service. Each hook call starts a short-lived
process and loads the rule pack, so plan for process-start and JSON parsing
overhead rather than daemon memory.

Production deployment should use this ownership model:

| Path | Purpose | Suggested owner and mode |
| --- | --- | --- |
| `/usr/local/bin/icg` | Guard executable | `root:root`, `0755` |
| `/etc/icg/` | Release-controlled configuration | `root:root`, not world-writable |
| `/etc/icg/packs/` | Active modular rule-pack directory | `root:root`, `0755` |
| `/etc/icg/packs/*.json` | Individual active rule packs | `root:root`, `0644` |
| `/etc/icg/packs.previous/` | Prior active directory retained by `icg update` | `root:root`, `0755` |
| `/etc/icg/rule-pack.json` | Legacy single-pack compatibility artifact | `root:root`, `0644` |
| `/etc/icg/trust-pointer.json` | Trusted release reference | `root:root`, `0644` |
| `/etc/icg/last-update-check.json` | Updater bookkeeping | `root:root`, `0644` |
| `/var/cache/icg/telemetry.json` | Rolling evaluation telemetry | writable by the hook identity, if telemetry is wanted |

The rule pack and trust pointer must not be writable by the agent process. The
current hook initializes its telemetry store before evaluating input, so the
hook identity must be able to create/read `/var/cache/icg/telemetry.json`.
Later telemetry processing and persistence errors are reported as warnings,
but a permission failure during initialization can prevent that invocation
from returning a decision.

## Source installation

This is the reproducible installation path when a prebuilt, approved binary
is not available.

### 1. Obtain and inspect the source

Use the repository commit that was approved for the deployment. Do not build
from an unreviewed working tree.

```bash
git clone https://git.ardenone.com/jedarden/irreversible-command-gate.git
cd irreversible-command-gate
git status --short
git rev-parse HEAD
```

If the repository is private in your environment, authenticate to Forgejo by
the organization's normal mechanism. Keep credentials out of command output
and do not put them in a rule pack or hook configuration.

### 2. Build and test

```bash
cargo test --locked
cargo build --release --locked
```

Install only the resulting binary, not the repository's `target` directory:

```bash
sudo install -o root -g root -m 0755 \
  target/release/icg /usr/local/bin/icg
```

For a developer-only installation, `cargo install --path . --locked` is
convenient, but a user-owned `~/.cargo/bin/icg` is not an acceptable location
for a production guard or its rule pack.

### 3. Create protected directories

```bash
sudo install -d -o root -g root -m 0755 /etc/icg /etc/icg/packs
sudo install -d -o root -g root -m 0750 /var/cache/icg
```

The hook identity must be able to create or update the telemetry file if you
intend to use telemetry. If the hook runs as an unprivileged user, have the
deployment system grant only the required cache-directory access; never make
`/etc/icg` user-writable just to solve a cache permission problem.

### 4. Install the approved modular rule-pack artifact

The production hook loads every JSON manifest in `/etc/icg/packs/`. This is
required for packs with empty `tool_keywords`: the secrets pack scans every
Bash command, while image-tag and storage-class scan YAML writes. Those packs
cannot be folded into the merged legacy `rule-pack.json` without losing their
dispatch semantics.

For an online host, `icg update` downloads and activates the exact
`icg-packs.tar.gz` asset after the trust pointer is set in the next step. It
validates every manifest before it touches the active path, then atomically
exchanges the complete directory and retains the former directory at
`/etc/icg/packs.previous/`.

The release archive contains only root-level regular JSON manifests (for
example `./secrets.json`, `./image-tag.json`, and `./storage-class.json`), not
a nested `packs/` directory. The updater rejects nested paths, traversal,
links, special files, duplicate names, and invalid manifests. Use a manual
staging procedure only for an offline bootstrap:

```bash
stage_dir=$(mktemp -d)
tar -xzf /path/to/approved/icg-packs.tar.gz -C "$stage_dir"
test -f "$stage_dir/secrets.json"
test -f "$stage_dir/image-tag.json"
test -f "$stage_dir/storage-class.json"
# Have an administrator review the staged directory, then install it with a
# directory rename rather than copying files over the active directory.
```

Do not use `tests/fixtures/current-release-clean.json` or the merged
`rule-pack.json` as production policy; both have intentionally incomplete
coverage for the modular packs above. A legacy single-file installation is
supported only for compatibility with already-deployed hosts.

### 5. Initialize the trust pointer

The trust pointer is a separate, exact release reference. It is not a
`latest` alias. Set it only to the release that passed the project's Layer 1
regression/coverage checks and Layer 2 human review.

```bash
sudo /usr/local/bin/icg trust set vX.Y.Z \
  --justification "Approved release record: <ticket-or-review-reference>"
sudo /usr/local/bin/icg trust show
sudo /usr/local/bin/icg trust check vX.Y.Z
```

The command writes `/etc/icg/trust-pointer.json`. On an online host, activate
the approved modular release immediately afterward:

```bash
sudo /usr/local/bin/icg update
sudo /usr/local/bin/icg status
```

If a release has not yet been published to the updater's configured release
repository, do not run `icg update`; use the offline bootstrap procedure and
record the exact artifact-to-release mapping in the deployment record.

### 6. Connect the hook

Install the hook in the harness configuration that is actually used on the
host. The hook command must be an absolute path so it does not depend on the
agent's working directory or PATH.

#### Claude Code

Add a `PreToolUse` command hook to `~/.claude/settings.json` for the tools
whose input should be checked. Merge this into an existing `hooks` object;
do not overwrite unrelated settings:

```json
{
  "hooks": {
    "PreToolUse": [
      {
        "matcher": "Bash|Write|Edit",
        "hooks": [
          {
            "type": "command",
            "command": "/usr/local/bin/icg hook",
            "timeout": 10
          }
        ]
      }
    ]
  }
}
```

`Bash` supplies command-mode input. `Write` and `Edit` supply file content
for content-mode packs such as image-tag and storage-class checks. Claude Code
uses the `tool_name`/`tool_input` payload shape; `icg` also accepts the
camelCase spelling used by older fixtures.

Confirm the hook appears in Claude Code's hook inspection UI or diagnostics
before relying on it. Hook matchers are case-sensitive. If the host uses a
managed settings policy, place the hook in the administrator-controlled
location instead of weakening that policy.

#### Codex CLI

For a local Codex CLI that supports native `PreToolUse` command hooks, add the
equivalent handler to the supported user or project hook configuration. A
typical configuration shape is:

```json
{
  "hooks": {
    "PreToolUse": [
      {
        "matcher": "Bash|apply_patch",
        "hooks": [
          {
            "type": "command",
            "command": "/usr/local/bin/icg hook",
            "timeout": 10
          }
        ]
      }
    ]
  }
}
```

Use the file and schema documented by the installed Codex CLI version. The
current adapter accepts `Bash` command input and Codex `apply_patch` input,
including patches that touch multiple files. A cloud-hosted Codex job does
not execute this host's `/usr/local/bin/icg` and is outside this deployment.

### 7. Verify the installation

First verify the files and pointer:

```bash
command -v icg
icg --version
stat -c '%U:%G %a %n' /usr/local/bin/icg /etc/icg/packs /etc/icg/packs/*.json /etc/icg/trust-pointer.json
icg trust show
icg status
```

Then exercise the hook directly with a known test command. This does not
execute the command; it only invokes the guard:

```bash
printf '%s\n' '{"tool_name":"Bash","tool_input":{"command":"vault kv destroy secret/test"}}' \
  | icg hook --rule-pack /etc/icg/packs
```

For a pack containing that rule, the output should contain
`"permissionDecision":"deny"`. A safe command should produce an empty JSON
object or an allow decision:

```bash
printf '%s\n' '{"tool_name":"Bash","tool_input":{"command":"vault status"}}' \
  | icg hook --rule-pack /etc/icg/packs
```

Test content-mode input separately when the installed pack covers it:

```bash
printf '%s\n' '{"tool_name":"Write","tool_input":{"filePath":"deploy/app.yaml","content":"storageClassName: ssd\n"}}' \
  | icg hook --rule-pack /etc/icg/packs
```

Finally run one real, non-destructive command through the configured harness
and inspect its hook diagnostics. Never use a real Vault destroy, force-push,
secret deletion, or other destructive operation as an installation test.

## Common deployment configurations

### Hook-only workstation

For one developer workstation, install the binary and rule pack under the
protected system paths, then configure only the local harness hook. This is
the smallest useful deployment. Add Unix PATH symlinks only when the real
shadowed binaries are available later in `PATH` and the wrapper's limitations
are acceptable; the native hook remains the required harness integration.

### Shared host with both harnesses

Install one root-owned binary and rule pack, then configure both Claude Code
and local Codex CLI to call that same absolute path. Both adapters use the
same rule-pack format and engine. Test each harness independently because
their hook registration and failure behavior are separate.

### Offline or manual rule-pack installation

When a host cannot reach the release API, an administrator can transfer the
approved `icg-packs.tar.gz` artifact through the organization's existing
signed/controlled distribution path and use the offline bootstrap procedure
above. Set the trust pointer to the release reference recorded with that
artifact, but do not run `icg update` on the offline host. Keep the previous
directory until the new one has passed the direct hook smoke tests.

### Channel-specific rollout

Channels select a different trust-pointer, pack, and state path. The updater
derives `/etc/icg/trust-pointer-canary.json`, `/etc/icg/packs-canary`, and
`/etc/icg/last-update-check-canary.json` for `--channel canary`, so a canary
can share a host with stable without mixing policy trees:

```bash
sudo icg trust set --channel canary vX.Y.Z \
  --justification "Canary cohort approved for vX.Y.Z"
sudo icg update --channel canary
sudo icg trust show --channel canary
```

Configure the canary hook with the matching pack:

```text
/usr/local/bin/icg hook --rule-pack /etc/icg/packs-canary
```

The current `icg status --channel` command reports the default compatibility
artifact path, not an arbitrary pack directory; use `icg trust show --channel`
and inspect the canary directory explicitly when using this layout. Do not
advance the stable pointer until the canary observation and release review are
complete.

### Release-bound repository override

An override is not a local disable switch. It is a reviewed
`overrides/<repo>.toml` release artifact whose `release_ref` must equal the
host's trusted reference, whose expiry and 90-day re-justification dates must
be current, and whose rule IDs must exist in the loaded pack.

When an approved deployment requires one, pass all three hook options:

```text
/usr/local/bin/icg hook \
  --rule-pack /etc/icg/packs \
  --override-file /path/to/overrides/example.toml \
  --repository jedarden/example \
  --trusted-ref vX.Y.Z
```

Supplying only some of these options is an error. See
[`per-repo-overrides.md`](../notes/per-repo-overrides.md) for the manifest
schema and release-gate commands.

## Upgrades and version migration

There are two independently versioned things to upgrade: the executable and
the rule-pack artifact. Upgrade them deliberately and keep a known-good copy
of both.

### Upgrade the executable from source

1. Select the reviewed commit or tag.
2. Run `cargo test --locked` and `cargo build --release --locked` from that
   source.
3. Back up the current binary and record its checksum.
4. Install the new binary to `/usr/local/bin/icg` with root ownership and
   mode `0755`.
5. Re-run `icg --version`, the direct deny/allow hook smoke tests, and one
   harness-level test.

Because the guard is per-invocation, new hook calls use the new binary without
a service restart. A hook process already running continues with the binary
it already loaded.

Example backup and replacement:

```bash
sudo cp --preserve=mode,ownership \
  /usr/local/bin/icg /usr/local/bin/icg.previous
sha256sum /usr/local/bin/icg /usr/local/bin/icg.previous
sudo install -o root -g root -m 0755 target/release/icg /usr/local/bin/icg
```

### Modular directory upgrade with the self-updater

The updater does not discover or choose a latest release. It reads the exact
reference from the trust pointer, requests that tag from the configured GitHub
Releases API, downloads the exact `icg-packs.tar.gz` asset, validates its
root-level regular JSON manifests and their runtime regexes, and atomically
replaces `/etc/icg/packs/`. The prior tree is retained at
`/etc/icg/packs.previous/`; packs omitted from the release are removed because
the whole directory is exchanged rather than modified in place. `rule-pack.json`
remains a legacy compatibility fallback only.

After Layer 1 and Layer 2 approval of a release:

```bash
sudo icg trust set vX.Y.Z \
  --justification "Layer 1 passed; Layer 2 approved <review-reference>"
sudo icg trust check vX.Y.Z
sudo icg update
sudo icg status
```

If the updater cannot find the pointer, exact archive asset, or a valid archive
layout, it exits without changing the active directory. It stages the download
beside the target directory, so activation is same-filesystem atomic. `icg
update` updates the rule packs only; it does not upgrade `/usr/local/bin/icg`.
The updater does not validate a checksum or signature itself, so the operator
must bind the selected release asset to the approved release record and verify
any required provenance before advancing the pointer.

### Version migration checklist

For each release, record:

- old and new executable commit/version;
- old and new trusted release references;
- rule-pack artifact checksum and source release;
- the `regression-suite` result and `coverage-diff/v1` report;
- any new, removed, disabled, narrowed, or widened rules;
- the canary cohort and observation result, if used; and
- every repository override whose `release_ref`, expiry, or justification
  changed.

Run the release checks before changing the pointer:

```bash
icg regression-suite path/to/current-release.json \
  --output /tmp/icg-regression-suite.json
icg coverage-diff path/to/previous-release.json \
  path/to/current-release.json
```

A coverage regression requires a non-blank `--justification` and human review;
that flag is evidence for review, not a substitute for it. If an override is
present, use `--previous-override` and `--current-override` as described in
[`per-repo-overrides.md`](../notes/per-repo-overrides.md).

There is no automatic schema migration for arbitrary manifests and there is no
`icg update --rollback-to` command. Treat a malformed or incompatible archive
as a failed deployment. A rejected archive leaves `/etc/icg/packs/` untouched;
to roll back an already activated release, restore the retained directory and
pointer:

```bash
sudo mv /etc/icg/packs /etc/icg/packs.failed
sudo mv /etc/icg/packs.previous /etc/icg/packs
sudo icg trust set vPREVIOUS \
  --justification "Rollback to last-known-good deployment"
sudo icg trust check vPREVIOUS
```

If the previous artifact is available from the approved release repository,
the pointer can instead be set to `vPREVIOUS` and `sudo icg update` can fetch
it. Re-run the smoke tests after either rollback path.

### Fail-closed migration caution

Fail-Closed is a durable, administrator-controlled policy decision. Do not
enable it fleet-wide based only on this installation guide. Follow the
[Fail-Closed mode guide](fail-closed-mode.md), validate the harness behavior
for process errors/timeouts/missing responses, confirm health and telemetry
evidence, and obtain the required operational approval first.

`ICG_FAIL_CLOSED=true` is retained as a stricter local/test override. It is not
the durable activation path, and `ICG_FAIL_CLOSED=false` cannot demote a valid
durable Fail-Closed policy.

## Package and artifact distribution

The repository currently provides a Cargo project and release-integrity
commands. It does not currently commit a Debian/RPM/Homebrew package,
container image, installer script, checksum/signature manifest, or a release
workflow that this guide can safely treat as an always-available download.

Accordingly:

- Source builds are the supported installation path in this checkout.
- `cargo install --path .` is a developer convenience, not a protected fleet
  installation.
- A prebuilt binary may be used only when it comes from an approved release
  record and its architecture, checksum, and provenance have been verified by
  the operator.
- Do not copy a guessed `/releases/latest/download/...` URL into automation.
  The exact asset name and release reference must come from the release record.
- `icg update` downloads the exact `icg-packs.tar.gz` modular release asset,
  validates it, and atomically activates the complete production pack
  directory. It never installs or upgrades the binary.
- The current updater does not perform checksum or signature verification;
  downstream packaging or deployment automation must do that before invoking
  the updater or staging an artifact.

If a downstream package is created, it should install the binary at
`/usr/local/bin/icg` or the distribution's administrator-controlled equivalent,
create `/etc/icg` without user write access, preserve the rule-pack and trust
pointer across binary upgrades, and run the hook smoke test as a package
post-install check. The package must not silently set the trust pointer to a
moving release alias.

## Installation troubleshooting

### `cargo build` fails while compiling TLS dependencies

Install the build prerequisites for the host (`build-essential pkg-config
libssl-dev` on Debian/Ubuntu, or the Fedora/RHEL equivalents above), then
rerun `cargo build --release --locked`. If the release build used a different
toolchain, use that pinned toolchain rather than weakening the TLS dependency.

### `icg: command not found`

Check the installed path and the invoking user's PATH:

```bash
command -v icg || true
ls -l /usr/local/bin/icg
echo "$PATH"
```

Use `/usr/local/bin/icg` in the hook configuration even if an interactive
shell resolves `icg` correctly.

### Permission denied reading or writing `/etc/icg`

Check the directory and file ownership without making them writable by the
agent:

```bash
namei -l /etc/icg/packs
stat -c '%U:%G %a %n' /etc/icg /etc/icg/packs /etc/icg/packs/*.json /etc/icg/trust-pointer.json
```

Use `sudo install` for deployment operations. The normal hook only needs read
access to the rule pack and trust pointer; an operator running `trust set` or
`update` needs the corresponding administrator privilege.

### The direct hook test returns `{}` for every command

That is the expected fail-open behavior when no pack is loaded or when the
input is for an unrecognized tool. Confirm:

```bash
test -r /etc/icg/packs/secrets.json
icg status
```

Then pass the pack explicitly with `--rule-pack` and test a command that the
pack actually contains. A test fixture is not evidence that the production
pack has the same rules.

### The hook never fires in the harness

Check all of the following:

1. The harness configuration is in the scope actually loaded (user, project,
   or managed policy).
2. The event is `PreToolUse` and the matcher uses the exact tool name and
   capitalization.
3. The command is an absolute executable path and is executable by the hook
   user.
4. The hook receives JSON on stdin and has no diagnostic text on stdout.
5. The direct pipe test works outside the harness.

For Claude Code, use its hook inspection/diagnostics command and verify the
`Bash|Write|Edit` matcher. For Codex CLI, verify that the installed version
supports native `PreToolUse` hooks and use its current configuration path.

### The hook reports invalid JSON or missing fields

The required envelope is:

```json
{
  "tool_name": "Bash",
  "tool_input": {"command": "git status"}
}
```

For `Write`, supply `filePath` and `content`; for `Edit`, supply `filePath`,
`oldString`, and `newString`; for Codex `apply_patch`, supply the patch in the
`command` field. Do not wrap the JSON in shell diagnostics or send multiple
JSON objects to one invocation.

### The rule pack is valid JSON but still does not load

`icg` supports JSON pack loading, and the hook's default is the
`/etc/icg/packs/` directory (falling back to the legacy single-file artifact
only when that directory is absent). Check each pack's file extension,
permissions, and manifest schema against the release artifact. A malformed
pack causes the invocation to fail open and reports the failure to stderr.
Restore the last-known-good approved directory rather than editing a production
pack in place.

### `icg trust show` says no pointer exists

Initialize it with the exact approved release reference:

```bash
sudo icg trust set vX.Y.Z --justification "<review reference>"
sudo icg trust show
```

For a channel, use `--channel NAME` on both `trust` and `update`. A custom
`--trust-pointer-path` is useful for tests and isolated staging, but the
default production pointer belongs under `/etc/icg`.

### `icg update` cannot reach the release or find an asset

The updater needs network access to `https://api.github.com`, a trust pointer,
an exact release reference, and an `icg-packs.tar.gz` release asset. Check
these in order:

```bash
icg trust show
curl --fail --silent --show-error https://api.github.com/repos/jedarden/irreversible-command-gate/releases/tags/vX.Y.Z >/dev/null
```

Use the manual artifact procedure for an intentionally offline host. Do not
change the trust pointer to `latest` to work around an unavailable tag.

### Telemetry warnings mention `/var/cache/icg`

Telemetry is auxiliary to the immediate hook verdict. Give the hook identity
the narrowly scoped ability to write `/var/cache/icg`, or accept that the
rolling baseline is unavailable while the rule evaluation continues. Do not
make `/etc/icg` writable. Inspect the file with:

```bash
sudo icg telemetry status
```

### A PATH wrapper does not block a command

This is expected for the current repository state. `icg wrapper` is a parser
scaffold and does not execute a real binary or enforce a rule pack. Remove any
experimental symlink and rely on the native hook until a completed wrapper
implementation is released and documented.

### A deployment must be rolled back

Do not use an undocumented flag or delete the active configuration. Restore a
known-good rule pack, point the trust pointer to the corresponding exact
release, and repeat the direct hook tests:

```bash
sudo mv /etc/icg/packs /etc/icg/packs.failed
sudo mv /etc/icg/packs.previous /etc/icg/packs
sudo icg trust set vPREVIOUS --justification "Incident rollback"
sudo icg trust check vPREVIOUS
```

Preserve the failed artifact, stderr, trust-pointer contents, and deployment
commit for incident review. Never use a real destructive operation to prove
that rollback succeeded.

## Operational references

- [Operator documentation index](README.md)
- [Deny-message interpretation](deny-messages.md)
- [Migration from `org-rule-guard.py`](migration-from-org-rule-guard.md)
- [Fail-closed transition design](../design/fail-closed-transition.md)
- [Release-cutting runbook](../runbooks/release-cutting.md)
- [Per-repository overrides](../notes/per-repo-overrides.md)
