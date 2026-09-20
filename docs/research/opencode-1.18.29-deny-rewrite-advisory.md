# OpenCode 1.18.29 — deny / rewrite / advisory semantics for `tool.execute.before`

Bead: `irrevers-dd6f88c9` (split child of `irrevers-54c51194`). Companion to
the hook inventory (`opencode-1.18.29-plugin-surface.md`, bead
`irrevers-be384aee`), which pinned the hook names and payload shapes; §9 of
that file cross-references this one. The inventory established **which hook
fires and with what payload**; this file pins **what a hook can do about it**:
how a deny propagates, whether a rewrite executes, and whether any advisory
can reach the agent.

Same artifacts as the inventory: `opencode` **1.18.29**, binary sha256
`ca6c0e1f42be3120595bf6848937e7586ec862c87fa7aa111e89c7cc6e9a4650`
(re-hashed 2026-09-20), `@opencode-ai/plugin` 1.18.29
(`~/.config/opencode/node_modules/@opencode-ai/plugin/dist/index.d.ts`).
Every offset is a byte index into that binary; §5 reproduces each window.

## 0. One-line verdicts

- **DENY** — throw from `tool.execute.before`: the error propagates through
  `Plugin.trigger`, the tool call dies **before execution and before any
  permission ask**, the tool part becomes `status:"error"`, the model
  receives `Tool execution failed: <message>` as the tool result, and the
  agent loop **continues** (deny is per-call, not session-killing).
- **REWRITE** — mutate `output.args` properties **in place**: hook and
  executor share the same args object at all three wrapper sites, so the
  mutation is what executes; reassigning `output.args = {...}` is a no-op.
  The transcript/UI records the model's **original** args either way.
- **ADVISORY** — none at tool-call time: the hook's only channels are the
  `{args}` output object and a discarded return value. The degraded signal
  path is **deny-with-message** (throw `Error("ICG: …")`); a true post-hoc
  advisory channel exists on `tool.execute.after` only.

## 1. Deny: mechanism, error path, what the agent sees

### 1.1 Mechanism — throwing is the only denial channel

`Plugin.trigger` (offset 98343185) calls each plugin's handler and discards
what it returns:

```js
J = v.fn("Plugin.trigger")(function*(W, K, U) {      // W=hook name, K=input, U=output
  if (!W) return U;
  let B = yield* l0.get(X);
  for (let z of B.hooks) {
    let M = z[W];
    if (!M) continue;
    yield* v.promise(async () => M(K, U))            // a throw here rejects the effect
  }
  return U;                                          // hook return values never read
})
```

The declared return type is `Promise<void>` (`index.d.ts:235-241`), and the
runtime never reads a return value — so there is no `{decision: "deny"}`
envelope to return, unlike Claude Code's `PreToolUse` (§6.1 of the
contract). A throw is the only way to stop the call: it rejects the
`v.promise` effect, which fails the generator that called `trigger`, which
is the tool wrapper's `execute` itself.

### 1.2 Ordering: the throw lands before execution and before the permission ask

All three wrapper shapes run the before-hook first:

- Built-in tools (offset 96848200): `trigger("tool.execute.before", …,
  {args:b})` → `u.execute(b, H)` — a throw prevents the tool body, and the
  permission asks that live inside tool bodies never fire.
- Generic/MCP tools (offset 96854450): `trigger(…, {args:V})` →
  `x.ask({permission:u, patterns:["*"], …})` → `w(V, K)` — the throw lands
  before `ask`, so no permission prompt is raised either.
- Subagent `task` (offset 96860500, `SessionPrompt.handleSubtask`):
  `trigger(…, {args:ie})` → `Ge.execute(ie, …)`.

This is stronger than §6.3 assumed: a hook deny also **pre-empts the
permission system** — the user is never prompted for a call ICG is about to
deny.

### 1.3 Error path: who catches the throw, and what text is recorded

Main path (every registered tool, model-issued calls): the AI-SDK stream
emits a `tool-error` chunk when `execute` rejects, and the
`SessionProcessor` routes it to the per-call failure handler (offset
~96821600, stream-part switch):

```js
case"tool-result":{ … if(c.result.type==="error"){ yield* F(c.id, c.result.value); return } …
case"tool-error":{ yield* F(c.id, c.error ?? Error(c.message)); return }
```

`F` is `SessionProcessor.failToolCall` (offset 96817133):

```js
F = s.fn("SessionProcessor.failToolCall")(function*(c, N) {
  let $ = yield* m(c); if(!$ || $.part.state.status!=="running") return !1;
  yield* e.updatePart({…$, state:{status:"error", input:$.part.state.input,
      error:Yo(N), metadata:$.part.state.metadata,
      time:{start:…, end:Date.now()}}}),
  N instanceof Ml.RejectedError || N instanceof mh.RejectedError)
    h.blocked = h.shouldBreak;
  return yield* x(c), !0 })
```

Subagent path: `handleSubtask` catches the failed effect with `catchCause`,
squashes the cause to an `Error` (non-Error throws included:
`B instanceof Error ? B : Error(String(B))`), and records the literal text
(offset 96863400):

```js
state:{status:"error",
       error: oe ? `Tool execution failed: ${oe.message}` : "Tool execution failed", …}
```

A third, step-level safety net (`SessionRunner.failUnsettledTools`, offset
98847183) marks any tool whose step failed around it with the same
`Tool execution failed: <cause>` text (call site offset 98860073) and
publishes a `Tool.Failed` event. Interrupted calls record
`"Tool execution aborted"` with `metadata.interrupted: true` (offset
96824700) — distinct from a hook deny.

### 1.4 What the agent (model) sees

The message → model conversion (`MessageV2.toModelMessages`, offset
98470228) turns an error-state tool part into an **`output-error` tool
result** whose `errorText` is the recorded error string:

```js
if(N.state.status==="error"){ …
  F.parts.push({type:"tool-"+N.tool, state:"output-error", toolCallId:N.callID,
                input:N.state.input, errorText:N.state.error, …}) }
```

So a hook that throws `new Error("ICG: denied …")` causes the model to
receive, as the result of its own tool call, the text
`Tool execution failed: ICG: denied …` (subagent/step paths — the message
rides `.message`; main path passes it through the module-local serializer
`Yo(N)`). **The thrown message is the one string ICG controls that the
agent is guaranteed to see on denial.** Non-`Error` throwables are
`String()`-ed, so `throw "text"` also survives, but `Error` with a `.message`
is the supported shape.

### 1.5 Loop liveness: a hook deny does not stop the turn

`SessionProcessor.process` initializes `shouldBreak` from config (offset
96826100) and only a permission-class rejection blocks the loop:

- `h.shouldBreak = (config).experimental?.continue_loop_on_deny !== !0` —
  i.e. true (turn should stop on denial) unless that experimental flag is
  set.
- `failToolCall` sets `h.blocked = h.shouldBreak` **only** for
  `RejectedError` — the error class the *config permission engine* raises
  (offset 96826800: `if(h.blocked||h.assistantMessage.error) return "stop";
  return "continue"`).

A plugin hook throw is an ordinary `Error`, not a `RejectedError`, so
`blocked` stays false and the session **continues**: the model sees the
failed tool result and may attempt a different route. Two consequences for
ICG: (a) a deny does not end the agent's turn — expect follow-up attempts,
which is fine because each is gated again; (b) if a "stop the loop" deny is
ever wanted, the plugin-API cannot produce a `RejectedError` by contract —
denial config rules are the only channel that stops the loop (subject to
`continue_loop_on_deny`).

### 1.6 What the user sees

The part update in §1.3 is streamed to clients (the TUI renders the tool
call as errored with that text). `Event.Error` is **not** published for a
hook throw — the subagent path publishes it only for its own
unknown-agent validation failure (offset 96861238), and `failUnsettledTools`
publishes `Tool.Failed` only for step-level failures. ICG denials surface
to the user exactly as the errored tool call.

**Deny verdict:** deny-by-throw confirmed; error path
hook → `Plugin.trigger` reject → `tool-error` chunk → `failToolCall` /
`catchCause` → part `status:"error"` (`Tool execution failed: <msg>`) →
model sees `errorText` as its tool result; loop continues.

## 2. Rewrite: exact shape, and does it execute?

### 2.1 The return shape is "no shape" — output mutation only

`index.d.ts:235-241` types the hook `… => Promise<void>`; `Plugin.trigger`
discards handler return values (§1.1). There is no
`{ updatedInput }`-style response. The only rewrite channel is mutating the
`output` object handed in as the second parameter.

### 2.2 Object identity: the hook's `output.args` IS the executor's args

The same args reference flows to the hook and to the executor at every
wrapper site:

- Built-in tools (offset 96848200):

  ```js
  execute(b, w){ return l.promise(s.gen(function*(){
    let H = g(b, w);
    yield* i.trigger("tool.execute.before", {…}, {args:b});  // hook sees b
    let h = yield* u.execute(b, H), …                        // executor gets b
  ```

- Generic/MCP tools (offset 96854450): `trigger(…, {args:V})` …
  `yield* s.promise(() => w(V, K))` — same `V`.
- Subagent `task` (offset 96860500): `ie = {prompt, …}; trigger(…,
  {args:ie}) … Ge.execute(ie, {…})` — same `ie`.

Therefore: **mutating a property** (`output.args.command = "…"` for `bash`,
`output.args.content` for `write`, `output.args.newString` for `edit`) edits
the very object the executor receives — the rewrite **reaches execution**.
**Reassigning** (`output.args = {command:"…"}`) rebinds a property of the
throwaway output wrapper, which nobody reads again — a **no-op**. The
inventory's §4.2 caveat and the embedded plugin docs' "mutate `output` in
place" (offset 103619680) are literal. The permission metadata built by
tool bodies (`metadata:{command:r.command}` in `ShellTool.ask`, offset
96923082) reads the same mutated object, so prompts reflect the rewrite.

### 2.3 The rewrite executes but is invisible in the transcript

The tool part's recorded `state.input` is written once when the model's
tool call is parsed — `case"tool-call": … input:N` from `c.input` (offset
~96819000) — and every later part update passes it through unchanged:
`completeToolCall` (`input:$.part.state.input`, offset 96816786),
`failToolCall` (offset 96817133), and the subagent completions
(`input:Z.state.input`, offset 96863400). No observed path rewrites
`state.input` from the wrapper's post-hook args. Consequence: the session
log / UI / `tool.execute.after` part input show the model's **original**
args while the **mutated** args executed. An ICG adapter that rewrites must
log the rewrite itself (bead, stderr, or its own audit store) — OpenCode's
transcript will not record that a rewrite happened. (The after-hook *does*
receive the mutated args — `{args:b}` at offset 96848200 — but only
in-process.)

**Rewrite verdict:** in-place property mutation of `output.args` reaches
execution (identity proven at 96848200 / 96854450 / 96860500); reassignment
is discarded; the replacement is execution-real but display-invisible.

## 3. Advisory: can a warning reach the agent?

### 3.1 No advisory channel on the pre-tool hook — §6.3 confirmed

`tool.execute.before`'s output type is `{args: any}` and nothing else
(`index.d.ts:235-241`, re-read 2026-09-20), and handler return values are
discarded (§1.1). There is no field, envelope, or event by which a
*non-blocking* warning attached to an **allowed** call could reach the
model. `supports_additional_context: false` in §6.3 is correct.

Grepping the binary for `additionalContext` (12 hits) finds only third-party
SDK internals embedded in the bundle — a vendor AI-workflow client's start
request (`additional_context: eX.additionalContext`, offset 97470217) and
AWS Smithy middleware context (`_additionalContext` / `SMITHY_CONTEXT_KEY`,
offset 97550312). Neither is an OpenCode plugin surface.

The plugin SDK's only "additional context" language is
`experimental.chat.system.transform`: *"`context`: Additional context
strings appended to the default prompt"* (`index.d.ts:280`). That is a
**per-message system-prompt** channel on the chat path, not reachable at
tool-call time — a warning about a specific tool call cannot ride it, and
it is `experimental.`-namespaced.

### 3.2 The degraded signal path: deny-with-message

What §6.3 called "a Warn degrades to a bare allow" is only one of two
degradations. The other is **loud**: throw with an explanatory message. Per
§1.3–1.4 the model then receives
`Tool execution failed: ICG: <reason>` as the tool result — the reason is
delivered verbatim, at the exact call site, in the same turn. For ICG this
means:

- **Warn on an otherwise-allowed call** (advisory, execution proceeds): no
  channel — degrades to a bare allow, as §6.3 says.
- **Deny with explanation** (execution blocked): fully supported and the
  *only* model-visible signal — throw `new Error("ICG: <one-line reason>")`.
  The message should be self-contained and actionable (what was denied and
  why), because it arrives inside the `Tool execution failed:` prefix and
  the loop continues (§1.5) — the model will read it and try again
  differently.

Message budget notes: the text rides the thrown `.message` (subagent path:
`oe.message`; step path: `squash(cause).message`), the part error is a
plain string, and there is no truncation of `state.error` observed in the
conversion path — but `output` (not `error`) is length-capped elsewhere
(`toolOutputMaxChars`), so treat the message as effectively short-form.

### 3.3 Post-hoc advisory exists on `tool.execute.after` — after the fact

`tool.execute.after`'s output `{title, output, metadata}`
(`index.d.ts:249-258`) is mutable and **model-visible**: the built-in
wrapper returns the same result object it handed the after-hook
(`trigger("tool.execute.after", …, V); … return V`, offset 96848200), and
the subagent path stores `output:G.output` after triggering it (offset
96863400); `completeToolCall` writes that `output` into the part (offset
96816786), which the next model turn receives as the tool result. An
adapter can therefore append an audit/warning line to a *completed* call's
output — but execution has already happened, so this is an audit channel,
never prevention.

**Advisory verdict:** no advisory channel at tool-call time (§6.3's
assumption confirmed); degraded path is deny-with-message via the thrown
error message; `tool.execute.after` output mutation is a post-hoc,
execution-already-happened advisory only.

## 4. Consequences for §6.3 of `docs/notes/harness-adapter-contract.md`

1. "A Deny is delivered by throwing an error from the hook" — confirmed,
   with the addition that the throw **pre-empts permission prompts** (§1.2)
   and does **not** stop the agent loop (§1.5).
2. "a Rewrite by mutating `args` in place" — confirmed as the only working
   form; reassignment is a no-op (§2.2), and the rewrite is invisible in
   the transcript, so ICG must audit rewrites itself (§2.3).
3. "`supports_additional_context: false` … a Warn degrades to a bare
   allow" — the capability flag is right, but the degraded-path statement
   should be extended: **deny-with-message** is available and
   model-visible (`Tool execution failed: ICG: <reason>`), so a
   must-be-seen warning can be delivered by escalating to a deny with an
   explanatory message (§3.2). A non-blocking warn still degrades to a
   silent allow.
4. Adapter error-message contract: the string ICG puts in the thrown
   `Error` is the only model-visible text it controls (§1.4) — it should be
   produced by the plugin from the canonical deny reason, prefixed `ICG:`,
   and kept to a single actionable line.

## 5. Reproduction

```bash
opencode --version            # 1.18.29
sha256sum ~/.local/lib/node_modules/opencode-ai/node_modules/opencode-linux-x64/bin/opencode
# → ca6c0e1f42be3120595bf6848937e7586ec862c87fa7aa111e89c7cc6e9a4650

P=~/.config/opencode/node_modules/@opencode-ai/plugin/dist/index.d.ts
sed -n '235,241p' $P          # tool.execute.before — output {args:any}, Promise<void>
sed -n '249,258p' $P          # tool.execute.after — output {title,output,metadata}
sed -n '276,284p' $P          # experimental.chat.system.transform — "Additional context" doc (line 280)

python3 - <<'EOF'
B="/home/coding/.local/lib/node_modules/opencode-ai/node_modules/opencode-linux-x64/bin/opencode"
d=open(B,"rb").read()
def w(off,ln,label):
    print(f"===== {label} @{off} ====="); print(d[off:off+ln].decode("utf-8","replace")); print()
w(98343185, 650,  "Plugin.trigger — hooks called, returns discarded")
w(96848200, 1100, "built-in wrapper — {args:b} then u.execute(b,H)")
w(96854450, 900,  "generic/MCP wrapper — {args:V} then ask then w(V,K)")
w(96860500, 2900, "handleSubtask — {args:ie} then Ge.execute(ie,…)")
w(96863400, 1400, "subagent catchCause → 'Tool execution failed: '+oe.message")
w(96817100, 900,  "SessionProcessor.failToolCall — part → status:error, error:Yo(N)")
w(96818500, 3450, "stream switch — tool-result error / tool-error → F")
w(98469700, 1600, "toModelMessages — error part → state:'output-error', errorText")
w(96826100, 500,  "process — shouldBreak = continue_loop_on_deny !== true")
w(98846800, 1000, "SessionRunner.failUnsettledTools — Tool.Failed event")
w(97469950, 700,  "additionalContext hit A — vendor workflow client (not OpenCode)")
w(97550050, 1500, "additionalContext hit B — AWS Smithy middleware (not OpenCode)")
EOF
```
