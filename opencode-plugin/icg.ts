// @icg-opencode-plugin v1
//
// The ICG gate for OpenCode 1.18.29 (the sole support target; no runtime
// version probing — see docs/research/opencode-1.18.29-plugin-surface.md
// §12 for the go/no-go). OpenCode's plugin API is in-process JS/TS, so the
// subprocess boundary lives inside this file: the `tool.execute.before`
// hook serializes the hook payload and shells out to the root-owned
// absolute ICG path — never via PATH — and acts on the one JSON object the
// engine prints back (contract §6.3, `render_opencode_envelope`):
//
//   {"action": "allow"}    return normally, args untouched. A warning
//                          renders this too: there is no advisory channel
//                          at tool-call time on this hook, so a Warn
//                          degrades to a bare allow.
//   {"action": "rewrite",  the complete replacement args object; applied
//    "args": {...}}        by copying its properties onto `output.args` IN
//                          PLACE — reassigning `output.args` is a no-op at
//                          every 1.18.29 call site, because the hook and
//                          the executor share one args object. OpenCode's
//                          transcript records the model's original args, so
//                          the stderr line below is the rewrite's audit
//                          trail.
//   {"action": "deny",     throw `new Error(message)` verbatim. The thrown
//    "message": "..."}     message is the only model-visible text the gate
//                          controls here (`Tool execution failed: ICG: …`),
//                          it lands before execution and before OpenCode's
//                          own permission ask, and it does not end the
//                          agent loop — retries arrive and are gated again.
//
// Any infrastructure failure — the icg binary missing, exiting non-zero,
// timing out, or printing anything that is not one of the envelopes above —
// fails OPEN with a stderr diagnostic: a broken gate must never turn into a
// wrong decision. Tools outside GATED_TOOLS fail open without spawning icg
// at all. This plugin never reads or writes OpenCode's configuration; it is
// deployed (and its file recognized) by `icg install-opencode-plugin`.
//
// EXPORT SHAPE — exactly one, the default factory. The installed 1.18.29
// loader rejects any module with a static named export alongside the
// default ("Plugin export is not a function"; live-probed 2026-09-20,
// recorded in surface research §10.3), so everything below — the pins, the
// gate, the rewrite applicator — rides on the default function object as
// properties. `test/icg.test.mjs` destructures them off of it.

import type { Plugin } from "@opencode-ai/plugin";
import { spawnSync } from "node:child_process";

/// The root-owned absolute ICG path (install.sh's target), pinned so the
/// gate never depends on PATH — a hostile or minimal PATH cannot redirect
/// or hide it, because an absolute spawn never consults PATH.
const ICG_BINARY = "/usr/local/bin/icg";

/// The adapter invocation this plugin serves: the OpenCode admission path
/// (`read_opencode_payload_from_stdin`) and its one-object envelope
/// (`render_opencode_envelope`). `opencode` is the telemetry slug spelling;
/// clap's derived `open-code` is its alias.
const ICG_ARGS: readonly string[] = ["hook", "--harness", "opencode"];

/// The tools this gate evaluates: the three the engine models — `bash`
/// (the command shape), `write` and `edit` (the content-mode shapes,
/// camelCase spellings per the pinned payload inventory §4.5) — plus
/// `apply_patch`, whose op-list payload does NOT map onto the engine's
/// patch classification and fails open at that boundary with a stderr
/// diagnostic (contract §6.3 coverage boundary). Routing it here is what
/// makes that diagnostic real instead of a silent allow. Every other tool
/// — the read-only set (`read`, `glob`, `grep`, `webfetch`), subagent
/// `task`, MCP keys — fails open without spawning icg, mirroring the
/// anchored three-tool matcher the Gemini and Cursor installers write.
const GATED_TOOLS: readonly string[] = [
  "bash",
  "write",
  "edit",
  "apply_patch",
];

/// How long the icg subprocess may run before it is killed and the call
/// fails open. OpenCode gives an in-process hook no timeout of its own, so
/// this bound is what stops a wedged icg from stalling a tool call forever.
/// The engine evaluates in well under a second; this is a stall cap, not a
/// budget.
const SPAWN_TIMEOUT_MS = 10_000;

/// Cap on the envelope read back from icg, so a runaway engine cannot
/// balloon the hook's memory.
const SPAWN_MAX_BUFFER = 8 * 1024 * 1024;

/// Every observable line this plugin prints carries this prefix.
const LOG_PREFIX = "[icg]";

interface ToolExecuteBeforeInput {
  tool: string;
  sessionID: string;
  callID: string;
}

interface ToolExecuteBeforeOutput {
  args?: unknown;
}

/** The one JSON object `render_opencode_envelope` prints. */
interface IcgEnvelope {
  action: "allow" | "rewrite" | "deny";
  args?: unknown;
  message?: unknown;
}

interface GateDeps {
  /// Override for the spawned binary — tests only. Production always uses
  /// the pinned ICG_BINARY.
  binary?: string;
  /// Override for the spawn itself — tests inject a stub here. Production
  /// always uses node:child_process's spawnSync.
  spawn?: typeof spawnSync;
}

function isPlainObject(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function describe(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

function oneLine(text: string): string {
  return text.replace(/\r?\n/g, "\\n");
}

/**
 * Apply a complete replacement args object onto the hook's args IN PLACE.
 *
 * This is the only working rewrite form on 1.18.29: the hook and the
 * executor share one args object, so property mutation is what executes,
 * while reassigning `output.args` rebinds a property of the throwaway
 * output wrapper nobody reads again. Stale keys are deleted so the result
 * is exactly the replacement, not a merge.
 *
 * The replacement's properties are defined, not assigned: `Object.assign`
 * copies through [[Set]], so a `__proto__` key — which `JSON.parse`
 * materializes as an own property of the parsed envelope — would trip the
 * `Object.prototype.__proto__` setter and rebind the executed args'
 * prototype to a model-controlled object. `defineProperty` gives every
 * key (that one included) an own data property, the same shape
 * `JSON.parse` itself would have produced.
 */
function applyArgsInPlace(
  target: Record<string, unknown>,
  replacement: Record<string, unknown>,
): void {
  for (const key of Object.keys(target)) {
    if (!Object.prototype.hasOwnProperty.call(replacement, key)) {
      delete target[key];
    }
  }
  for (const key of Object.keys(replacement)) {
    Object.defineProperty(target, key, {
      value: replacement[key],
      writable: true,
      enumerable: true,
      configurable: true,
    });
  }
}

/**
 * The `tool.execute.before` gate: evaluate one tool call against ICG and
 * act on the engine's envelope.
 *
 * Never throws for infrastructure failure — those paths return (fail open)
 * after a stderr diagnostic. The only throw is a deny, which is the point.
 */
async function gateToolExecuteBefore(
  input: ToolExecuteBeforeInput,
  output: ToolExecuteBeforeOutput,
  deps: GateDeps = {},
): Promise<void> {
  const scope = `tool=${input.tool} session=${input.sessionID} call=${input.callID}`;

  // Unsupported tools fail open without a subprocess: the gate must not
  // cost the read-only tools it can never judge anything about.
  if (!GATED_TOOLS.includes(input.tool)) {
    return;
  }

  const payload = JSON.stringify({
    tool: input.tool,
    sessionID: input.sessionID,
    callID: input.callID,
    args: output.args,
  });

  const spawn = deps.spawn ?? spawnSync;
  const binary = deps.binary ?? ICG_BINARY;
  let result: ReturnType<typeof spawn>;
  try {
    result = spawn(binary, ICG_ARGS, {
      input: payload,
      timeout: SPAWN_TIMEOUT_MS,
      encoding: "utf8",
      maxBuffer: SPAWN_MAX_BUFFER,
    });
  } catch (error) {
    console.error(
      `${LOG_PREFIX} ${scope} outcome=fail-open (icg could not be spawned: ${describe(error)})`,
    );
    return;
  }

  if (result.error) {
    console.error(
      `${LOG_PREFIX} ${scope} outcome=fail-open (icg did not run cleanly, status=${result.status} signal=${result.signal}: ${describe(result.error)})`,
    );
    return;
  }
  if (result.status !== 0) {
    console.error(
      `${LOG_PREFIX} ${scope} outcome=fail-open (icg exited ${result.status}; proceeding ungated)`,
    );
    return;
  }

  let envelope: unknown;
  try {
    envelope = JSON.parse(result.stdout);
  } catch (error) {
    console.error(
      `${LOG_PREFIX} ${scope} outcome=fail-open (icg stdout is not JSON: ${describe(error)})`,
    );
    return;
  }
  if (!isPlainObject(envelope)) {
    console.error(
      `${LOG_PREFIX} ${scope} outcome=fail-open (icg output is not a JSON object)`,
    );
    return;
  }

  const action = envelope.action;
  if (action === "allow") {
    // A warning renders this envelope too: with no advisory channel at
    // tool-call time, a non-blocking Warn degrades to a bare allow (§5).
    console.error(`${LOG_PREFIX} ${scope} outcome=allow`);
    return;
  }

  if (action === "deny") {
    const message =
      typeof envelope.message === "string" && envelope.message.trim() !== ""
        ? envelope.message
        : "ICG: denied (no reason provided)";
    console.error(
      `${LOG_PREFIX} ${scope} outcome=deny reason=${oneLine(message)}`,
    );
    // Deny-by-throw, the only denial channel on this hook: the error
    // propagates out of the hook, OpenCode aborts the call before
    // execution and before its own permission ask, and the message reaches
    // the model as `Tool execution failed: <message>`.
    throw new Error(message);
  }

  if (action === "rewrite") {
    if (!isPlainObject(envelope.args) || !isPlainObject(output.args)) {
      console.error(
        `${LOG_PREFIX} ${scope} outcome=fail-open (rewrite needs object args on both sides)`,
      );
      return;
    }
    const before = Object.keys(output.args).sort().join(",");
    try {
      applyArgsInPlace(output.args, envelope.args);
    } catch (error) {
      // A non-extensible or sealed args object is the only realistic cause;
      // the plugin's invariant is that nothing but a deny ever throws out
      // of this hook, so a failed application degrades like any other
      // infrastructure failure rather than surfacing as a plugin error.
      console.error(
        `${LOG_PREFIX} ${scope} outcome=fail-open (rewrite could not be applied: ${describe(error)})`,
      );
      return;
    }
    const after = Object.keys(output.args).sort().join(",");
    // The rewrite's audit trail is the plugin's own: OpenCode's transcript
    // records the model's original args, so this line is the only record
    // that the executed input differs from it.
    console.error(
      `${LOG_PREFIX} ${scope} outcome=rewrite keys ${before} -> ${after}`,
    );
    return;
  }

  console.error(
    `${LOG_PREFIX} ${scope} outcome=fail-open (icg returned an unrecognized action: ${oneLine(String(action))})`,
  );
}

const icgPlugin: Plugin = async () => ({
  "tool.execute.before": async (input, output) => {
    await gateToolExecuteBefore(input, output);
  },
});

// The named surface above travels as properties on the default factory —
// the single-export constraint is a loader fact (see the header), not a
// style choice. Tests destructure these off the default import.
Object.assign(icgPlugin, {
  ICG_BINARY,
  ICG_ARGS,
  GATED_TOOLS,
  SPAWN_TIMEOUT_MS,
  SPAWN_MAX_BUFFER,
  LOG_PREFIX,
  applyArgsInPlace,
  gateToolExecuteBefore,
});

export default icgPlugin;
