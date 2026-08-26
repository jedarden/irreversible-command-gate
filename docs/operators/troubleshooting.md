# Troubleshooting `icg`

This page is a quick diagnostic reference for the current implementation. For
the complete installation procedure, see the
[installation and deployment guide](deployment-guide.md).

## First-response checklist

Run these read-only checks before changing policy or replacing artifacts:

```bash
command -v icg
icg --version
icg status
icg trust show
stat -c '%U:%G %a %n' /usr/local/bin/icg /etc/icg /etc/icg/packs
```

Capture stderr from the failing hook and record the exact binary commit,
rule-pack file, trust pointer, harness version, and hook input shape. Never
include tokens, credentials, or secret values in a support bundle.

## Symptom lookup

| Symptom | Likely cause | First check |
| --- | --- | --- |
| `icg` is not found | Binary is not installed or PATH is different inside the harness | `ls -l /usr/local/bin/icg`; use its absolute path in the hook |
| Every call is allowed | Pack is missing, unreadable, invalid, or the hook is not firing | Run the direct stdin test with `--rule-pack` |
| Expected call is not denied | Rule does not match the command/content or the wrong pack is loaded | `icg status`; inspect the exact test payload |
| Harness never invokes `icg` | Wrong event, matcher, configuration scope, or unsupported harness version | Use harness hook diagnostics and check `PreToolUse` |
| `permission denied` under `/etc/icg` | Operator command lacks privilege or path ownership is wrong | `namei -l /etc/icg/packs` |
| `icg update` fails | Missing pointer, unavailable exact release, missing `rule-pack` asset, or no network | `icg trust show` and the updater checks below |
| Telemetry warning mentions `/var/cache/icg` | Hook identity cannot write auxiliary telemetry | Fix only cache permissions; do not loosen `/etc/icg` |
| PATH symlink does not block | Symlink is missing or not first in `PATH`, the invocation used an absolute path, no loaded pack covers that tool, or `ICG_RULE_PACK` points somewhere without one | `ls -l /usr/local/bin/<tool>`; see "PATH wrapper and absolute paths" below |

## Test the hook without executing a command

The direct test sends one JSON object to one `icg hook` process. It does not
run Vault, Git, or any other command:

```bash
printf '%s\n' '{"tool_name":"Bash","tool_input":{"command":"vault kv destroy secret/test"}}' \
  | /usr/local/bin/icg hook --rule-pack /etc/icg/packs
```

For a rule pack that contains this pattern, the output should contain:

```text
"permissionDecision": "deny"
```

Test an allowed command too:

```bash
printf '%s\n' '{"tool_name":"Bash","tool_input":{"command":"vault status"}}' \
  | /usr/local/bin/icg hook --rule-pack /etc/icg/packs
```

An empty JSON object is an allowed/no-op result. It does not prove that a
particular rule pack is installed; use a command known to be covered by the
approved pack.

## Installation and build failures

### `cargo build` cannot compile OpenSSL or `native-tls`

Install the system build dependencies and retry with the locked dependency
set:

```bash
# Debian/Ubuntu
sudo apt-get install --yes build-essential pkg-config libssl-dev

# Fedora/RHEL-like systems
sudo dnf install gcc gcc-c++ make pkgconf-pkg-config openssl-devel

cargo build --release --locked
```

The current project does not vendor OpenSSL. A package builder must provide
the matching development and runtime libraries for its target distribution.

### `icg` is not found or the hook gets a different binary

The interactive shell's PATH may not be the harness's PATH. Check both the
file and its identity:

```bash
ls -l /usr/local/bin/icg
command -v icg || true
sha256sum /usr/local/bin/icg
```

Configure the hook with `/usr/local/bin/icg hook`, not just `icg hook`.

### `/etc/icg` or the rule pack is not readable

Check every path component and ownership:

```bash
namei -l /etc/icg/packs
stat -c '%U:%G %a %n' /etc/icg /etc/icg/packs /etc/icg/trust-pointer.json
```

The hook needs read access. Installation and updates need administrator
privilege. Keep `/etc/icg` root-owned and not world-writable; do not fix this
by granting the agent write access.

### Rule-pack parse or validation failure

The default hook resolution loads every JSON pack under `/etc/icg/packs`
(the modular production shape); the legacy single-file
`/etc/icg/rule-pack.json` is used only when the directory is absent. Confirm
the loaded location holds the approved JSON artifact(s) and not a release
HTML page, a partial transfer, or a test fixture:

```bash
ls -l /etc/icg/packs
head -c 120 /etc/icg/packs/openbao.json; printf '\n'
```

A malformed or unreadable pack causes the current engine to fail open and
reports the problem on stderr. Keep the previous known-good artifact and
restore it rather than editing the live pack in place.

## Hook registration failures

### Claude Code

The handler belongs under the `PreToolUse` event in the settings scope that
Claude Code loads. A minimal shape is:

```json
{
  "hooks": {
    "PreToolUse": [
      {
        "matcher": "Bash|Write|Edit",
        "hooks": [
          {"type": "command", "command": "/usr/local/bin/icg hook", "timeout": 10}
        ]
      }
    ]
  }
}
```

Check that the matcher is case-sensitive and that the file was merged with,
not substituted for, the existing settings. Use Claude Code's hook inspection
or debug facilities to confirm that the handler is registered.

### Codex CLI

The installed Codex CLI must support native `PreToolUse` command hooks. Verify
the current version's supported hook file and schema, then use an absolute
command path. The adapter expects `Bash` and `apply_patch` inputs with JSON on
stdin. Cloud-hosted Codex jobs cannot call this host's binary.

### Hook output is rejected

`icg` must own stdout for the response JSON. Do not wrap the configured command
in a shell script that prints banners or diagnostics to stdout. Diagnostics
belong on stderr. A valid deny response has the shape:

```json
{
  "hookSpecificOutput": {
    "hookEventName": "PreToolUse",
    "permissionDecision": "deny",
    "permissionDecisionReason": "..."
  }
}
```

### Invalid input fields

The minimal accepted payloads are:

```json
{"tool_name":"Bash","tool_input":{"command":"git status"}}
{"tool_name":"Write","tool_input":{"filePath":"deploy/app.yaml","content":"image: app:1.2.3\n"}}
{"tool_name":"Edit","tool_input":{"filePath":"deploy/app.yaml","oldString":"old","newString":"new"}}
{"tool_name":"apply_patch","tool_input":{"command":"*** Begin Patch\n*** Add File: example.txt\n+x\n*** End Patch"}}
```

`apply_patch` also needs at least one file header to produce a content input.
Do not send multiple JSON objects to a single process. The parser accepts both
snake_case and the older camelCase field aliases, but the harness must still
send a valid tool envelope.

## Policy and verdict problems

### A known dangerous command is allowed

Work through these checks without running the dangerous command:

1. Confirm the hook fired by checking harness diagnostics.
2. Run the direct pipe test with `--rule-pack /etc/icg/packs`.
3. Confirm the command spelling and shell segmentation match the rule.
4. Confirm the rule is enabled in the approved release pack.
5. Confirm no repository override is exempting the rule.

If the direct test denies but the harness allows, the harness registration or
its output handling is wrong. If both allow, stop the rollout and report a
rule-pack coverage defect; do not add an unreviewed local regex.

### A safe command is denied

Record the denial reason, pack ID, and pattern ID from the hook response. Do
not bypass it by editing `/etc/icg/rule-pack.json`. Check whether the command
matches an intended safe pattern or whether an approved release has narrowed
the false positive. The correction path is a reviewed rule-pack release.

For a temporary, repository-specific exception, use the release-bound override
contract in [`per-repo-overrides.md`](../notes/per-repo-overrides.md). A local
TOML file without a matching trusted release, current expiry, and recent
justification is rejected.

### The hook seems to allow after an internal error

The current default is fail-open. That is intentional until the operator has
validated the durable policy, harness failure behavior, and release evidence.
`ICG_FAIL_CLOSED=true` is only a stricter compatibility override; it is not the
durable activation path. Follow the [Fail-Closed mode guide](fail-closed-mode.md)
before enabling the policy.

## Trust pointer and updater failures

### No trust pointer exists

The updater requires an exact trusted reference:

```bash
sudo /usr/local/bin/icg trust set vX.Y.Z \
  --justification "Approved release record: <reference>"
sudo /usr/local/bin/icg trust show
```

Do not use `latest`. The pointer is an administrator-controlled release
decision, not an update check result.

### `icg update` cannot find the release

The default updater uses the GitHub repository
`jedarden/irreversible-command-gate`, requests the exact pointer reference,
and requires a release asset named exactly `icg-packs.tar.gz` (the modular
pack archive; no substring match is accepted).

```bash
icg trust show
curl --fail --silent --show-error \
  https://api.github.com/repos/jedarden/irreversible-command-gate/releases/tags/vX.Y.Z \
  >/dev/null
```

If the host is offline or the release is intentionally not on GitHub, copy the
approved artifact manually and record the artifact checksum and release
reference. Do not weaken the pointer to make the updater choose a different
release.

### A channel deployment changed the wrong pack

`--channel NAME` selects `/etc/icg/trust-pointer-NAME.json` and activates
into `/etc/icg/packs-NAME` (the channel's own directory), leaving the
default `/etc/icg/packs` untouched. Use `--pack-dir` (alias
`--artifact-path`) to override the activation directory explicitly, and
pass that same directory to `icg hook --rule-pack`.

```bash
sudo icg update --channel canary
sudo icg hook --rule-pack /etc/icg/packs-canary
```

`icg status --channel` does not accept a pack directory and reports the
default location; inspect a custom canary directory directly.

## Telemetry and health state

Telemetry and health data are auxiliary operational state. They do not replace
the root-owned rule pack or trust pointer.

```bash
icg telemetry status
icg health status
```

The hook initializes telemetry before evaluating input, so a missing or
unwritable `/var/cache/icg` directory can make the hook fail before it returns
a decision. Grant the hook identity narrowly scoped access to that cache
directory; do not loosen `/etc/icg`. If health state is corrupt, first use
`icg health status` to identify the active path, then preserve a copy for
diagnosis before resetting it. For a deployment that explicitly configured
`/var/cache/icg/health-state.json`, the commands are:

```bash
sudo cp --preserve=mode,ownership \
  /var/cache/icg/health-state.json /var/cache/icg/health-state.json.failed
sudo icg health reset --force
```

Do not delete the rule pack, trust pointer, or release evidence as a health
cleanup step.

## PATH wrapper and absolute paths

On Unix, the installed PATH wrapper evaluates a shadowed command, denies
matching rules, and `exec`s the real binary found later in `PATH`. Check the
symlink, PATH ordering, `ICG_RULE_PACK`, and `icg coverage --list` if it does
not behave as expected. The wrapper does not cover absolute-path invocations
or direct library calls, so keep the native hook and harness controls in place.

## Rollback

There is no `icg update --rollback-to` flag. A successful `icg update`
renames the prior pack tree to `/etc/icg/packs.previous` before activating
the new one, so that sibling directory is the built-in pack rollback
target. Restore packs, binary, and pointer deliberately:

```bash
sudo rm -rf /etc/icg/packs.failed && sudo mv /etc/icg/packs /etc/icg/packs.failed
sudo mv /etc/icg/packs.previous /etc/icg/packs
sudo install -o root -g root -m 0755 \
  /usr/local/bin/icg.previous /usr/local/bin/icg
sudo icg trust set vPREVIOUS --justification "Incident rollback"
sudo icg trust check vPREVIOUS
```

Repeat the direct deny/allow tests and one harness-level test. Preserve the
failed artifact, stderr, trust pointer, and exact deployment commit for the
incident record.

## Escalation bundle

Collect the following, after redacting sensitive values:

```bash
icg --version > icg-version.txt 2>&1
icg status > icg-status.txt 2>&1
icg trust show > icg-trust.txt 2>&1
stat -c '%U:%G %a %n' /usr/local/bin/icg /etc/icg /etc/icg/packs \
  > icg-permissions.txt 2>&1
```

Attach the harness version and a sanitized failing hook payload. Do not attach
the rule pack if it contains secrets or private operational data; provide its
checksum and release reference instead.
