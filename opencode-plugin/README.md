# ICG plugin for OpenCode

The `tool.execute.before` gate for OpenCode **1.18.29** (the sole support
target — pinned by binary sha256, no runtime version probing; see
`docs/research/opencode-1.18.29-plugin-surface.md` §12 for the V1/V2
go/no-go). The file is plain TypeScript that OpenCode type-strips at load;
its only runtime import is `node:child_process`.

## How it gates

Every `bash`, `write`, `edit`, and `apply_patch` tool call is serialized —
`{tool, sessionID, callID, args}` — and sent to
`/usr/local/bin/icg hook --harness opencode` **by absolute path** (never via
PATH, so a hostile or minimal `PATH` cannot redirect the gate). The engine's
one-object reply (`render_opencode_envelope`, contract §6.3) drives the
action:

| Envelope | Plugin action |
|---|---|
| `{"action": "allow"}` | return; args untouched. A **Warn** renders this too — the hook has no advisory channel, so a warning degrades to a bare allow. |
| `{"action": "rewrite", "args": …}` | the complete replacement args, copied onto the hook's args **in place** (property mutation is the only form that executes on 1.18.29; reassigning `output.args` is a no-op). The rewrite's audit trail is the plugin's stderr line — OpenCode's transcript records the model's original args. |
| `{"action": "deny", "message": …}` | `throw new Error(message)` — OpenCode aborts the call before execution and before its own permission ask, the model sees `Tool execution failed: ICG: …`, and the loop continues. |

Read-only tools (`read`, `glob`, `grep`, `webfetch`), `task`, and MCP keys
fail open **without spawning icg at all**. `apply_patch` is routed to the
engine so its unmodeled op-list payload fails open with a diagnostic at the
classification boundary instead of silently passing as an allow.

Any infrastructure failure — icg missing, nonzero exit, timeout
(`SPAWN_TIMEOUT_MS`), non-JSON output, unknown action — fails **open** with a
`[icg] … outcome=fail-open` stderr diagnostic. A broken gate must never turn
into a wrong decision.

## Deployment

```sh
icg install-opencode-plugin            # global: ~/.config/opencode/plugin/icg.ts
icg install-opencode-plugin --uninstall
```

The installer deploys the copy embedded in the `icg` binary, recognizes its
own artifacts by content (the `@icg-opencode-plugin v1` marker), backs up a
differing prior ICG copy once as `<target>.icg-backup`, and refuses to touch
a foreign file. It never reads or writes OpenCode's configuration — the
global plugin directory needs no config change, and OpenCode's own
`permissions` system keeps working underneath the gate (a hook deny even
pre-empts its prompt). Prefer the global directory over per-project
registration: it loads for every project and evaluates before any
project-local plugin.

## Residual risks (pinned, not fixable at this layer)

- `opencode --pure` / `OPENCODE_PURE=1` silently skips **every** external
  plugin, this gate included; the PATH-wrapper layer is the backstop.
- A plugin that fails to *load* is dropped quietly — import-stage failures
  leave no log line — so verify after deploying (e.g. the factory-throw
  probe recipe in
  `docs/research/opencode-1.18.29-plugin-surface.md` §10.5).
- The gate is only as current as the installed `/usr/local/bin/icg`: an icg
  that predates the `--harness opencode` slug rejects the flag, and every
  call fails open until the binary is updated.

## Development

```sh
npm test                               # node --test test/ — zero dependencies, Node ≥ 23.6
../scripts/opencode-plugin-typecheck   # tsc over tsconfig.json + vendor/*.d.ts (skips loudly without tsc)
```
