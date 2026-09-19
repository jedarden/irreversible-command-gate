# OpenCode 1.18.29 plugin hook surface — inventory from the installed binary

Bead: `irrevers-be384aee` (split child of `irrevers-54c51194`). Purpose: pin the
hook names and payload shapes OpenCode 1.18.29 actually fires **before shell
execution** and **before write/edit tool execution**, so the harness-adapter
contract (§6.3 of `docs/notes/harness-adapter-contract.md`) rests on the
installed version rather than on the current docs site.

Every claim below cites either the bundled
`@opencode-ai/plugin` `.d.ts` (file:line) or a **byte offset in the installed
binary itself**. Reproduction commands are in §8.

## 1. Provenance

| Artifact | Value |
|---|---|
| `opencode --version` | `1.18.29` |
| Binary | `~/.local/lib/node_modules/opencode-ai/node_modules/opencode-linux-x64/bin/opencode` (Bun-compiled, 184,666,240 bytes), reached via `~/.local/bin/opencode` → `../lib/node_modules/opencode-ai/bin/opencode.exe` (hardlink) |
| Binary sha256 | `ca6c0e1f42be3120595bf6848937e7586ec862c87fa7aa111e89c7cc6e9a4650` |
| npm launcher pkg | `opencode-ai` 1.18.29 (`~/.local/lib/node_modules/opencode-ai/package.json`) |
| Plugin SDK | `@opencode-ai/plugin` **1.18.29**, `~/.config/opencode/node_modules/@opencode-ai/plugin/` (installed under `~/.config/opencode/package.json` deps) |
| `index.d.ts` sha256 | `f3ec1a150d1354be3c9d93928fa130edc118c63fb468533ebb01eb3d6ed77f92` |
| SDK types | `@opencode-ai/sdk` 1.18.29, `dist/gen/types.gen.d.ts` sha256 `1fdbed5b58ed5882b411cc9b1b03fdfe2fb131b0be39a0fd2622d12ab6f50f0f` |
| Date of inspection | 2026-09-19 |

The binary embeds the JS bundle in plain text; hook names are greppable with
`grep -a` and context windows recover the minified call sites. Offsets below
are byte indexes into the sha256-pinned binary.

## 2. Verdict on the three §6.3-relevant names

| Assumed name | Verdict in 1.18.29 | Evidence |
|---|---|---|
| `tool.execute.before` | **confirmed-in-1.18.29** | `.d.ts` `index.d.ts:235-241`; binary: 5 exact-string matches, all `trigger` call sites (offsets 96848392, 96849786, 96851579, 96853308, 96854610, 96862693, 116931206) |
| `shell.create.before` | **absent-in-1.18.29** | 0 matches in the binary (`grep -a -c` = 0); not in `Hooks` (`.d.ts` `index.d.ts:173-322`); no composed variant (no `"permission"+`, no `` `permission.${…}` `` anywhere) |
| `permission.evaluate` | **absent-in-1.18.29** | 0 matches in the binary; not in `Hooks`. Nearest real name: `permission.ask` — **declared in `.d.ts` `index.d.ts:225-227` but never fired** by the server (see §5) |

**Consequence:** `tool.execute.before` is the only hook in 1.18.29 that can
gate both pre-shell (`tool === "bash"`) and pre-write/edit (`tool === "edit"`,
`"write"`, `"apply_patch"`) execution from a plugin. This confirms the name
§6.3 actually pins, and kills the other two (they do not exist under any
spelling in this build).

## 3. Full `Hooks` surface of 1.18.29

From `~/.config/opencode/node_modules/@opencode-ai/plugin/dist/index.d.ts`
(the `Hooks` interface, lines 173–322):

`dispose` (174), `event` (175), `config` (178), `tool` (179, custom tool
definitions), `auth` (182), `provider` (183), `chat.message` (187),
`chat.params` (203), `chat.headers` (216), `permission.ask` (225),
`command.execute.before` (228), `tool.execute.before` (235), `shell.env`
(242), `tool.execute.after` (249), `experimental.chat.messages.transform`
(259), `experimental.chat.system.transform` (265),
`experimental.provider.small_model` (271), `experimental.session.compacting`
(283), `experimental.compaction.autocontinue` (296),
`experimental.text.complete` (306), `tool.definition` (316).

Cross-check: the binary's own embedded plugin documentation (offset
103619680) lists the same callback surface — `event`, `config`,
`chat.message`, `chat.params`, `chat.headers`, `tool.execute.before`,
`tool.execute.after`, `tool.definition`, `command.execute.before`,
`shell.env`, `permission.ask`, plus the five `experimental.*` callbacks —
and adds: *"Hook surface (mutate `output` in place; return `void`)"*.

Server-side trigger inventory: every `.trigger("<name>"` literal in the
binary was enumerated (§8). Hook names actually triggered by the server:
`tool.execute.before`, `tool.execute.after`, `shell.env`,
`command.execute.before`, `chat.message`, `chat.params`, `chat.headers`,
`tool.definition`, `experimental.chat.system.transform`,
`experimental.text.complete`, `experimental.session.compacting`,
`experimental.chat.messages.transform`, `experimental.compaction.autocontinue`,
`experimental.provider.small_model`. **`permission.ask` is not among them.**

## 4. The gate hook: `tool.execute.before`

### 4.1 Declaration (`.d.ts` `index.d.ts:235-241`)

```ts
"tool.execute.before"?: (input: {
    tool: string;
    sessionID: string;
    callID: string;
}, output: {
    args: any;
}) => Promise<void>;
```

### 4.2 How the server invokes hooks (offset 98343185, `"Plugin.trigger"`)

```js
J = v.fn("Plugin.trigger")(function*(W, K, U) {   // W=hook name, K=input, U=output
  if (!W) return U;
  let B = yield* l0.get(X);
  for (let z of B.hooks) {
    let M = z[W];                                  // look up hook by name per plugin
    if (!M) continue;
    yield* v.promise(async () => M(K, U))          // call hook(input, output)
  }
  return U;                                        // return value ignored
})
```

Semantics that matter for a gate:

- Hooks run **sequentially over all plugins**; the return value is discarded.
  The only output channel is **mutation of `output`**; the only denial channel
  is **throwing** (the thrown error propagates through `v.promise` and fails
  the enclosing Effect, aborting the tool call). §6.3's "Deny by throwing" is
  correct.
- **Mutation caveat (not in §6.3):** the executor passes its *original args
  object* onward, not `output.args`. E.g. the generic tool wrapper at offset
  96854610: `trigger("tool.execute.before", {tool:u,…}, {args:V})` then
  `ask(…)` then `w(V,K)` — and the built-in wrapper at 96848392: same shape,
  `u.execute(b, H)` after `trigger(…, {args:b})`. So **reassigning**
  `output.args = {…}` is invisible to the executor; a Rewrite must mutate
  properties **in place** (`output.args.command = "…"`). The embedded docs'
  phrase "mutate `output` in place" is literal.

### 4.3 Firing order vs. permission checks

For every built-in tool the wrapper order is:
`tool.execute.before` → tool body → (inside the body: permission ask) →
`tool.execute.after`. Example for the generic/MCP wrapper (offset 96854610):
before-hook fires **before** `ask({permission:u,…})`. A throw in the
before-hook therefore rejects the call before any permission prompt is
raised.

### 4.4 `input.tool` values (registered tool names)

Registrations recovered from the tools chunk (`…=j("<name>"` pattern,
offsets in parentheses): `bash` (96923964, via `j(lo.ToolID)` with
`var Gi="bash"` at 96901850), `plan_exit` (96898973), `question` (96901121),
`edit` (96931267), `glob` (96940175), `grep` (96942157), `read` (96945298),
`todowrite` (96965200), `webfetch` (96966803), `write` (96970389),
`invalid` (96971611), `skill` (96972339), `lsp` (96975336), `apply_patch`
(96985765). Subagent tool `task` exists separately (permission constant
`ir="task"` at 96955779). MCP tools get namespaced keys (e.g. `mcp:*`-guarded
`list`/`read` at 96849786/96853308).

### 4.5 `output.args` shapes for the ICG-relevant tools

Parameter structs recovered from the bundle:

- **`bash`** (schema fn `jr()` at 96903500):
  `{ command: string; timeout?: number /* ms */; workdir?: string }`
  (`p.Struct({command:p.String…, timeout:p.optional(Je)…, workdir:p.optional(p.String)…})`;
  usage `w.command`/`w.workdir`/`w.timeout` in the execute body at 96927480).
- **`edit`** (struct `ls` at 96930800):
  `{ filePath: string; oldString: string; newString: string; replaceAll?: boolean }`.
  Empty `oldString` means create/replace whole file (guarded at 96932048).
- **`write`** (struct `en` at 96969900):
  `{ filePath: string; content: string }`.
- **`apply_patch`** (execute body at 96988164): a list of patch operations,
  each `{ type: "add"|"update"|"delete", filePath, movePath?, newContent?,
  diff, additions, deletions }`.
- `read`/`glob`/`grep`/`webfetch` are read-shaped; irrelevant to an
  irreversible-command gate but they pass through the same hook.

### 4.6 Which hook the *bash tool* itself raises

`ShellTool.ask` (offset 96923082) issues up to two permission asks with this
payload (consumed by the permission engine, not by plugins — §5):

```js
// always, when any pattern was extracted:
ask({ permission: "bash",                    // lo.ToolID === "bash" (96901850)
      patterns: Array.from(e.patterns),      // per-command text patterns
      always:   Array.from(e.always),
      metadata: { command: r.command } })
// additionally, when the command touches paths outside the worktree:
ask({ permission: "external_directory",
      patterns: dirs.map(d => join(d, "*")), always: same,
      metadata: { command, directories: [...], patterns: [...] } })
```

## 5. Pre-shell reality: `shell.env`, and the dead `permission.ask`

### 5.1 `shell.env` (`.d.ts` `index.d.ts:242-248`)

```ts
"shell.env"?: (input: {
    cwd: string;
    sessionID?: string;
    callID?: string;
}, output: {
    env: Record<string, string>;
}) => Promise<void>;
```

It fires per shell spawn, but it is an **environment-injection hook, not a
gate**: the output is merged into the spawned process env and nothing can be
rejected. Call sites in the binary:

- `ShellTool.shellEnv` (offset 96925160) — the bash tool's shell:
  `trigger("shell.env", {cwd:h, sessionID, callID}, {env:{}})` →
  `{...process.env, ...b.env}`.
- `PtyEnvironment.get` (96438783) and `PtyHttpApi.create` (96579986) — PTY
  creation paths (no sessionID/callID).
- offset 96867054 — an internal tool spawning a fixed-purpose process
  (`TERM:"dumb"`).

**Bash-tool ordering** (execute body ending at 96927480): parse →
pattern/`dirs` collection → `ShellTool.ask` (§4.6) → `shell.env` →
`ShellTool.run` → spawn. So `shell.env` is the **last** plugin contact before
exec — after permission approval — and cannot veto.

### 5.2 `permission.ask` is declared but never fired

- Declared: `.d.ts` `index.d.ts:225-227` —
  `"permission.ask"?: (input: Permission, output: {status: "ask"|"deny"|"allow"}) => Promise<void>`.
- The input type is the SDK `Permission`
  (`sdk/dist/gen/types.gen.d.ts:369-383`): `{ id, type: string, pattern?:
  string|string[], sessionID, messageID, callID?, title, metadata:
  Record<string, unknown>, time: {created: number} }`.
- **The server never triggers it.** The literal string `permission.ask` occurs
  in the binary only inside the embedded plugin documentation (offset
  103619696); there is no `trigger("permission.ask", …)` call site and no
  composed name construction (`"permission"+…` → 0 matches). All other
  `Hooks` names have literal trigger sites (§3).
- What actually enforces permissions in 1.18.29 — `Permission.ask` (offset
  99209586): evaluates the **config ruleset** per pattern
  (`action: "deny"` → immediate `DeniedError`; all patterns `"allow"` →
  proceed; otherwise) → publishes a `permission.asked` event and awaits a
  client `reply` (`"once"`/`"always"`/`"reject"`). No plugin is consulted at
  any point in this path.

A plugin gate that waits on `permission.ask` (or a hypothetical
`permission.evaluate`) in 1.18.29 never runs. §6.3 does not rely on it — this
section records why it must not.

## 6. `command.execute.before` is *not* the bash-tool hook

Declared `.d.ts` `index.d.ts:228-234`:

```ts
"command.execute.before"?: (input: {
    command: string;          // the slash-command name
    sessionID: string;
    arguments: string;        // free-form arg text
}, output: { parts: Part[] }) => Promise<void>;
```

Its only server call site (offset 96884460) is the **user custom-command
(slash command) pipeline**, followed by an `Event.Executed` publish — it
never wraps a model-issued `bash` tool call despite the suggestive name. An
ICG adapter must not hook here.

## 7. `tool.execute.after` (post-hoc only)

`.d.ts` `index.d.ts:249-258`: input `{ tool, sessionID, callID, args }`,
output `{ title, output, metadata }` — the tool's *result* is mutable, but
execution already happened. Useless for prevention; noted so nobody reaches
for it as a fallback gate.

## 8. Reproduction

```bash
opencode --version                                   # → 1.18.29

P=~/.config/opencode/node_modules/@opencode-ai/plugin
sed -n '173,322p' $P/dist/index.d.ts                 # Hooks interface
sed -n '225,248p' $P/dist/index.d.ts                 # permission.ask, command.execute.before, tool.execute.before, shell.env
sed -n '369,383p' ~/.config/opencode/node_modules/@opencode-ai/sdk/dist/gen/types.gen.d.ts   # Permission

B=~/.local/lib/node_modules/opencode-ai/node_modules/opencode-linux-x64/bin/opencode
grep -a -c 'tool.execute.before' $B   # 5
grep -a -c 'shell.create.before' $B   # 0
grep -a -c 'permission.evaluate' $B   # 0
grep -a -o 'permission.ask[^e]' $B | wc -l   # docs-blob only (no trigger site)

# context windows (the offsets cited above):
python3 - <<'EOF'
B="/home/coding/.local/lib/node_modules/opencode-ai/node_modules/opencode-linux-x64/bin/opencode"
d=open(B,"rb").read()
for off,ln in [(98343185,900),(96848382,420),(96854610,700),(96923082,340),
               (96925160,300),(96884460,420),(103619696,400),(96901850,260)]:
    print(off, d[off:off+ln].decode("utf-8","replace").replace("\n","\\n")[:600], "\n")
EOF
```

## 9. Consequences for `harness-adapter-contract.md` §6.3

1. §6.3's pinned hook `tool.execute.before` with input
   `{tool, sessionID, callID}` / output `{args}`, deny-by-throw, is
   **confirmed against 1.18.29**; the per-tool `args` spellings for the
   mapping table (§3 of the contract) are §4.5 here:
   `command` (bash), `filePath` + `oldString`/`newString` (edit),
   `filePath` + `content` (write).
2. Rewrite semantics need the §4.2 caveat recorded: **mutate `args`
   properties in place; reassigning `output.args` is a no-op** at every
   1.18.29 call site.
3. `shell.create.before` and `permission.evaluate` do not exist in 1.18.29
   under any spelling; `permission.ask` exists but is dead in this build.
   Any planning text referencing them should be struck.
4. The plugin is in-process JS — the "shells out to
   `icg hook --harness opencode`" plan in §6.3 stands; the process boundary
   lives inside the plugin's `tool.execute.before` handler. Plugin
   discovery: `*.ts`/`*.js` in `.opencode/plugin/` or `.opencode/plugins/`
   (embedded docs, offset 103619232) or npm packages via the `plugin` config
   array.
5. `supports_additional_context: false` stays correct: no advisory channel
   exists on any pre-tool hook (output objects are only
   `{args}`/`{env}`/`{parts}`).
