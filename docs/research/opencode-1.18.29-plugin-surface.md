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
   array. Registration channels, precedence, and load/runtime failure
   behavior are pinned against the same binary in §10 below (bead
   `irrevers-67665fad`).
5. `supports_additional_context: false` stays correct: no advisory channel
   exists on any pre-tool hook (output objects are only
   `{args}`/`{env}`/`{parts}`).
6. The deny / rewrite / advisory **semantics** behind those names — error
   path, what the agent sees, rewrite reachability, the deny-with-message
   degraded advisory — are pinned against the same binary in
   [`opencode-1.18.29-deny-rewrite-advisory.md`](opencode-1.18.29-deny-rewrite-advisory.md)
   (bead `irrevers-dd6f88c9`).

## 10. Registration channels, precedence, and failure behavior (bead `irrevers-67665fad`)

This section pins **how a plugin gets registered** in the installed 1.18.29
and **what happens when one fails** — the two facts §6.3's fail-open design
rests on. It combines the same static binary method as §2–§7 with **live
experiments** against the installed binary (2026-09-20): plugins planted in
every candidate location, an isolated global config dir
(`XDG_CONFIG_HOME`, so the shared `~/.config/opencode` was never touched),
and observed `plugin_origins` via `opencode debug info` plus bootstrap logs.
The scratch projects used were removed after the run; §11 reproduces them.

Binary provenance is unchanged from §1: `opencode` **1.18.29**, binary
sha256 `ca6c0e1f42be3120595bf6848937e7586ec862c87fa7aa111e89c7cc6e9a4650`.
All offsets are byte indexes into that binary. Upstream tag `v1.18.29`
(`sst/opencode`, `packages/opencode/src/config/plugin.ts`, `…/plugin/index.ts`,
`…/plugin/loader.ts`) was cross-checked and matches the binary everywhere
both were read; where they could diverge, the binary is cited.

### 10.1 Registration matrix

| Channel | Honored in 1.18.29? | Position in the load order | Evidence |
|---|---|---|---|
| Global config file `plugin` array — first existing of `<config>/opencode.jsonc`, `opencode.json`, `config.json` (`~/.config/opencode/` by default); entries may be npm specs, `file://` URLs, absolute/relative paths (resolved against the config file), or `[spec, options]` tuples | **yes** | 1 | live: `./cfg-global-rel.js` from the isolated global config listed first; binary `yield*g(e.Path.config,D2,"global")` @103659515; candidate order `rW` @103654495 |
| `OPENCODE_CONFIG` file's `plugin` array (env override) | yes (when set) | 2 | binary order in `Config.loadInstanceState` @103659747 area (not live-tested) |
| Project config `plugin` array — `opencode.json(c)` collected from cwd up the worktree root (`QW.files("opencode", directory, worktree)` @103659747); running from a subdirectory still picks them up | **yes** | 3 | live: root `opencode.json` array entries at positions 2–3, identical from a subdirectory; scope forced `"local"` |
| `.opencode/opencode.json(c)` of each project config dir (+ `OPENCODE_CONFIG_DIR`) | **yes** | 4 (per dir: json before that dir's glob) | live: `.opencode/opencode.json` entry at position 6, before the same dir's glob files |
| Global plugin directory glob `<config>/{plugin,plugins}/*.{ts,js}` — **both spellings honored**, `dot`+`symlink` included | **yes** | 5 (immediately after the global config file's own array, before project `.opencode` entries) | live: `plugin/g-global-singular.js` and `plugins/h-global-plural.js` at positions 4–5; glob string @103760240 in `cJ` = `ConfigPlugin.load` |
| Project plugin directory glob `<dir>/.opencode/{plugin,plugins}/*.{ts,js}` — both spellings | **yes** | 6 (per `.opencode` dir, after its json) | live: positions 7–9; same glob `cJ` |
| npm package specs via any `plugin` array (`"plugin": ["some-pkg"]`, tuple form with options) | yes — installed on demand (`Failed to install plugin pkg@ver` stage @98342234; per-config-dir background install of `@opencode-ai/plugin` @103660480; `waitForDependencies` gate) | per declaring config, in array order | binary + upstream `loader.ts` (not live-tested — no npm install was run) |
| Edge channels: `OPENCODE_CONFIG_CONTENT`, well-known/remote org config, managed config dir — each may carry a `plugin` array | yes | after the above, in `loadInstanceState` order | binary @103659800–103661000 |

Not a channel: the **`plugins` (plural) config key** and the sorted
`{plugin,plugins}` glob at offset 103535246 belong to a *separate* Effect-style
loader (`config-plugin` / `ConfigProviderPlugin`), not to the hook host that
fires `tool.execute.before`. Don't cite that offset for server plugin
discovery.

Ordering details that matter:

- The observable origin order in the live run was exactly:
  global config array → project root config array → **global plugin dirs** →
  `.opencode` config array → `.opencode` plugin dirs. The directories walked
  (`QW.directories`) put the global config dir before the project's
  `.opencode` dir.
- **Directory glob order is filesystem readdir order — not sorted.** The
  server-side `cJ` (@103760182) does no sort (unlike the unrelated
  Effect-side loader, which does `d.sort()`); the live glob order *changed
  between two runs on the same directory*. Nothing may depend on filename
  ordering within a plugin dir.
- **Duplicate identity across channels: last declaration wins.**
  `deduplicatePluginOrigins` (@103760756) iterates `toReversed()`, keeping the
  last occurrence (its spec resolution *and* scope) at the last position.
  Live-verified: a file declared in the project config array *and* discovered
  by the `.opencode` glob appeared exactly once — at the glob position.
  Identity = the `file://` URL for file specs, the npm package name otherwise.
- Consequence of the order: a **global** plugin's hooks run before any
  project-local plugin's hooks (`Plugin.trigger` walks hooks in origin order,
  §4.2). An ICG gate deployed globally evaluates before project plugins.
- `scope: "global"|"local"` on an origin (@103658202: http(s) → global;
  inside the project dir → local; else global) is provenance metadata for the
  merge — it does not gate loading.

### 10.2 The kill switch: `--pure`

`opencode --pure …` (or `OPENCODE_PURE=1`) skips **all external plugins**:
`let A = Q.pure ? [] : w.plugin_origins ?? []` @98341808. Internal/default
plugins are a separate list (`Q.disableDefaultPlugins ? [] : ek(Q)`
@98341584; runtime flag `disableDefaultPlugins` from env
`OPENCODE_DISABLE_DEFAULT_PLUGINS`, RuntimeFlags @104003475) and are
unaffected by `--pure`. Live: both `opencode --pure debug info` and
`OPENCODE_PURE=1 opencode debug info` print
`external plugins disabled (--pure)` instead of the origin list.
**An ICG gate deployed as a plugin is silently absent under `--pure`.**

### 10.3 Load-time failure behavior — every failure is fail-open

Observed live (three runs) and matching the binary; the failing plugin is
dropped and **startup, session creation, and tool execution all proceed**:

| Failure mode | What happens | Evidence |
|---|---|---|
| Module import fails — syntax error, top-level `throw` | `loadExternal` reports stage `"load"`; `report.error` → `B(...)` @98341007 **publishes an `Event.Error`** on the bus (`Failed to load plugin <spec>: <cause>`, @98342289/98342334) and the plugin is dropped. **Nothing is logged.** No plugin can observe the publish — hooks don't exist yet — and run-mode doesn't render it; only a TUI/client attached during bootstrap sees it. | live: `02-broken-syntax.js`, `03-throws-at-import.js` produced zero log lines across 3 runs while the session was created and the model stream attempted |
| Exported factory throws when invoked | `Qy` wrapped in `tryPromise` → `logError("failed to load plugin", {path, spec, error})` @98342494, swallowed (`v.catch(→void)`) | live ×3: `failed to load plugin … error="05 factory exploded"` |
| Export is not a function (e.g. `export default 42`) | `Zy` throws `TypeError("Plugin export is not a function")` @98340540 → same logError + skip | live ×3: `error="Plugin export is not a function"` |
| `config` hook throws | `logError("plugin config hook failed")` @98342699 + `v.ignore` | live ×3: `error="06 config hook exploded"` |
| Internal (built-in) plugin factory throws | `logError("failed to load internal plugin")` @98341701, skipped via `v.option` | binary |
| npm install fails | `Failed to install plugin <pkg>@<ver>: <cause>` via the same `Event.Error` path | binary @98342146 |

The live sequence (single run, DEBUG logs): config files load → plugin
failures logged at `:03.445` → session created at `:03.593` → model stream
attempted. A project whose every plugin is broken behaves exactly like a
project with no plugins.

### 10.4 Runtime failure behavior — the throw boundary

- **`tool.execute.before` throw** = deny for that one tool call: the call
  aborts before execution and before any permission ask, the model sees
  `Tool execution failed: <message>`, the loop continues. Pinned at binary
  level in [`opencode-1.18.29-deny-rewrite-advisory.md`](opencode-1.18.29-deny-rewrite-advisory.md)
  §1 (same sha256). A **runtime** hook throw is fail-*closed* for that call.
- **`event` hook throw**: the bus listener fires hooks fire-and-forget; a
  throw surfaces as an unhandled rejection printed to stderr
  (`error: 07 event hook exploded: <type>` — 65+ lines across 10 event types
  in the live capture) and delivery of later events continues; the run
  proceeded through session creation and the model-call phase.
- **`config` hook throw** at bootstrap: logged, ignored (§10.3).
- Nothing observed or found in the binary lets a plugin failure kill the
  process or abort a session.

### 10.5 The fail-open bound this puts on §6.3

1. **A plugin gate fails open by default, and the failure is quiet.** If the
   ICG plugin has a syntax error, a bad import, or a throwing factory, every
   tool call proceeds ungated; for the two import-stage failure classes there
   is *no log line at all*. §6.3's fail-open-for-unsupported-tools posture is
   consistent with the platform, but the platform extends it to *the gate
   itself*.
2. **`opencode debug info` lists registrations, not loads.** It prints
   `plugin_origins` (config-level), so it shows a broken plugin as
   "registered" while it never loaded. There is no CLI command that
   enumerates successfully loaded external hooks. **ICG must self-verify**:
   e.g. the plugin's `config` hook proves liveness at bootstrap (a throw
   there *is* logged at ERROR), or the adapter pings the ICG daemon on first
   `tool.execute.before`.
3. **`--pure`/`OPENCODE_PURE` disables the gate entirely** (§10.2) with no
   warning beyond `debug info`. Deployment docs must name this flag.
4. **Deployment channel choice:** `~/.config/opencode/plugin/icg.<js|ts>`
   (global directory glob) — or, if options are needed, a `file://` entry in
   the global config's `plugin` array. Both are honored (§10.1); the
   directory form needs no dependency install; global scope loads for every
   project (subdirectory runs included) and evaluates before any
   project-local plugin. Avoid npm specs (install machinery, `compatibility`
   gate) and per-project registration (partial coverage).
5. **Never rely on filename order** within a plugin directory (§10.1) — one
   file per concern, or an aggregator module.

### 10.6 Version evidence summary

- `opencode --version` → `1.18.29`; binary re-hashed as in §1 (2026-09-20).
- Live behavior captured from the installed binary in disposable projects
  under `~/scratch` (removed after the run; recreated by §16).
- `opencode debug info`, `opencode --pure debug info`, `opencode serve`,
  `opencode run --print-logs --log-level DEBUG` outputs quoted above were
  produced by that binary; log lines are from
  `--print-logs` stderr and `~/.local/share/opencode/log/opencode.log`.

### 11. Reproduction of the §10 experiments

```bash
opencode --version            # 1.18.29
sha256sum ~/.local/lib/node_modules/opencode-ai/node_modules/opencode-linux-x64/bin/opencode

# Registration matrix (isolated global dir; nothing under ~/.config touched)
S=$(mktemp -d ~/scratch/ocplug-XXXX)
mkdir -p $S/xdg/opencode/plugin $S/xdg/opencode/plugins \
         $S/proj/.opencode/plugin $S/proj/.opencode/plugins
echo 'export default async () => ({})' > $S/proj/.opencode/plugin/a.js
echo 'export default async () => ({})' > $S/proj/.opencode/plugins/b.js
echo 'export default async () => ({})' > $S/xdg/opencode/plugin/g.js
echo 'export default async () => ({})' > $S/xdg/opencode/plugins/h.js
echo 'export default async () => ({})' > $S/xdg/opencode/rel.js
echo 'export default async () => ({})' > $S/proj/root.js
printf '{ "plugin": ["./rel.js"] }' > $S/xdg/opencode/opencode.json
printf '{ "plugin": ["./root.js"] }' > $S/proj/opencode.json
cd $S/proj && git init -q && git add -A && git -c user.email=t@t -c user.name=t commit -qm i
XDG_CONFIG_HOME=$S/xdg opencode debug info | sed -n '/plugins:/,$p'
# → order: global cfg array, project cfg array, global plugin/ + plugins/,
#   .opencode glob (readdir order, unsorted)

# Kill switch
opencode --pure debug info | sed -n '/plugins:/,$p'      # "external plugins disabled (--pure)"
OPENCODE_PURE=1 opencode debug info | sed -n '/plugins:/,$p'

# Load-failure behavior (broken plugins; bogus provider avoids any LLM call)
mkdir -p $S/fail/.opencode/plugin && cd $S/fail && git init -q
printf 'export default async () => ({ broken!!!' > $S/fail/.opencode/plugin/02-syntax.js
printf 'throw new Error("boom")\nexport default async () => ({})' > $S/fail/.opencode/plugin/03-importthrow.js
printf 'export default 42' > $S/fail/.opencode/plugin/04-badexport.js
printf 'export default async () => { throw new Error("factory") }' > $S/fail/.opencode/plugin/05-factory.js
printf 'export default async () => ({ config: async () => { throw new Error("cfg") } })' > $S/fail/.opencode/plugin/06-confighook.js
printf '{ "provider": { "bogus": { "npm": "@ai-sdk/openai-compatible", "options": { "baseURL": "http://127.0.0.1:1/v1" }, "models": { "m": {} } } }, "model": "bogus/m" }' > $S/fail/opencode.json
git add -A && git -c user.email=t@t -c user.name=t commit -qm i
timeout 25 opencode run --print-logs --log-level DEBUG "x" 2>$S/err.txt
grep -a 'failed to load plugin\|config hook failed' $S/err.txt   # 04/05/06 logged; 02/03 absent
rm -rf $S

# Binary citations
python3 - <<'EOF'
B="/home/coding/.local/lib/node_modules/opencode-ai/node_modules/opencode-linux-x64/bin/opencode"
d=open(B,"rb").read()
for off,ln,label in [(98341007,120,"B = publish Event.Error"),
                     (98341808,90,"--pure strips external origins"),
                     (98341584,120,"disableDefaultPlugins / internal list"),
                     (103760182,220,"ConfigPlugin.load glob {plugin,plugins}"),
                     (103760756,260,"dedupe: last declaration wins"),
                     (103659515,60,"global config file merged first"),
                     (103659747,80,"project config files walk"),
                     (103658202,120,"scope: global|local"),
                     (98340540,60,"TypeError: Plugin export is not a function"),
                     (98342699,60,"plugin config hook failed"),
                     (98341701,80,"failed to load internal plugin"),
                     (98342146,220,"loadExternal error report: install/entry/load")]:
    print(f"== {label} @{off} =="); print(d[off:off+ln].decode("utf-8","replace")); print()
EOF
```
