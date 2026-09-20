// Runtime suite for the ICG OpenCode plugin (opencode-plugin/icg.ts).
//
// The suite runs on plain node --test with zero dependencies; icg.ts is
// imported directly (Node ≥ 23.6 strips its erasable types). A stub spawn
// rides the plugin's deps.spawn seam for the envelope semantics, the stderr
// channel is intercepted for the pinned diagnostics, and the hostile-PATH
// and real-boundary tests drive REAL subprocesses through the REAL
// spawnSync: the pinned binary under a hostile PATH (which cannot redirect
// it to a decoy, because an absolute spawn never consults PATH), a missing
// binary, and one wedged past the stall cap.

import test from "node:test";
import assert from "node:assert/strict";
import { chmodSync, existsSync, mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { spawnSync as realSpawnSync } from "node:child_process";

// The deployed file carries exactly one export — the default factory
// (1.18.29's loader rejects modules with additional static named exports).
// The named test surface rides on the factory as properties.
import icgPlugin from "../icg.ts";

const {
  applyArgsInPlace,
  gateToolExecuteBefore,
  GATED_TOOLS,
  ICG_ARGS,
  ICG_BINARY,
  LOG_PREFIX,
  SPAWN_TIMEOUT_MS,
} = icgPlugin;

const SESSION = "ses_2f8a91c4";
const CALL = "call_01j4funsup";

/// A spawn stub that records every call and returns a canned result.
function stubSpawn(result) {
  const calls = [];
  const spawn = (binary, args, options) => {
    calls.push({
      binary,
      args,
      payload: JSON.parse(options.input),
      options,
    });
    return result;
  };
  spawn.calls = calls;
  return spawn;
}

const okResult = (stdout) => ({
  status: 0,
  signal: null,
  stdout,
  stderr: "",
});

/// Run the gate with console.error intercepted and hand back every line it
/// printed. The stderr channel is part of the pinned contract — the
/// outcome lines are the rewrite's only audit trail, and the fail-open
/// diagnostic is the operator's only signal that a call ran ungated — so
/// the outcome tests assert on it directly.
async function captureStderr(run) {
  const lines = [];
  const original = console.error;
  console.error = (...parts) => lines.push(parts.map(String).join(" "));
  try {
    await run();
  } finally {
    console.error = original;
  }
  return lines;
}

/// The pinned fail-open diagnostic: present, prefixed, and labeled.
function assertFailOpenDiagnostic(lines, label) {
  const line = lines.find((candidate) => candidate.includes("outcome=fail-open"));
  assert.notEqual(line, undefined, `${label}: a fail-open diagnostic must be emitted`);
  assert.ok(
    line.startsWith(LOG_PREFIX),
    `${label}: the diagnostic carries the ${LOG_PREFIX} prefix (got: ${line})`,
  );
  return line;
}

test("the plugin mounts with the pinned loading contract", async () => {
  assert.equal(typeof icgPlugin, "function", "default export is a factory");
  const hooks = await icgPlugin({});
  assert.equal(
    typeof hooks["tool.execute.before"],
    "function",
    "the factory exposes tool.execute.before",
  );
});

test("the module ships exactly one export — the 1.18.29 loader constraint", async () => {
  // The installed 1.18.29 loader rejects any module carrying a static
  // named export next to the default ("Plugin export is not a function";
  // live-probed 2026-09-20, surface research §10.3). The named surface
  // must stay on the factory as properties — never become `export const`.
  const ns = await import("../icg.ts");
  assert.deepEqual(
    Object.keys(ns).sort(),
    ["default"],
    "an added named export would make the real loader drop this plugin quietly",
  );
  assert.equal(typeof ns.default.ICG_BINARY, "string", "the named surface rides on the factory");
});

test("the pins hold: absolute binary, adapter invocation, tool scope", () => {
  assert.equal(ICG_BINARY, "/usr/local/bin/icg");
  assert.deepEqual(ICG_ARGS, ["hook", "--harness", "opencode"]);
  assert.deepEqual([...GATED_TOOLS].sort(), ["apply_patch", "bash", "edit", "write"]);
  assert.equal(SPAWN_TIMEOUT_MS > 0, true, "the stall cap is positive");
});

test("unsupported tools fail open without spawning icg", async () => {
  for (const tool of ["read", "glob", "grep", "webfetch", "task", "question", "mcp__fs__write"]) {
    const spawn = stubSpawn(okResult('{"action":"deny","message":"ICG: nope"}'));
    const output = { args: { pattern: "**/*.rs" } };
    await gateToolExecuteBefore({ tool, sessionID: SESSION, callID: CALL }, output, {
      spawn,
    });
    assert.equal(spawn.calls.length, 0, `${tool} must not reach icg`);
    assert.deepEqual(output.args, { pattern: "**/*.rs" }, `${tool} args untouched`);
  }
});

test("shell commands reach icg before execution with the pinned payload", async () => {
  const args = { command: "git push --force origin main", timeout: 120000, workdir: "/project" };
  const spawn = stubSpawn(okResult('{"action":"allow"}'));
  await gateToolExecuteBefore({ tool: "bash", sessionID: SESSION, callID: CALL }, { args }, {
    spawn,
  });
  assert.equal(spawn.calls.length, 1);
  const call = spawn.calls[0];
  assert.equal(call.binary, ICG_BINARY, "the root-owned absolute path is spawned");
  assert.deepEqual(call.args, ["hook", "--harness", "opencode"]);
  // The hook payload: tool/sessionID/callID plus the args object itself —
  // never tool_name/tool_input, and the envelope never leaks into args.
  assert.deepEqual(call.payload, { tool: "bash", sessionID: SESSION, callID: CALL, args });
  assert.equal(call.options.timeout, SPAWN_TIMEOUT_MS);
});

test("write and edit inputs reach the content-mode checks", async () => {
  const cases = [
    ["write", { filePath: "/project/a.rs", content: "fn main() {}\n" }],
    ["edit", { filePath: "/project/a.rs", oldString: "fn main() {}", newString: "fn main() -> () {}", replaceAll: false }],
  ];
  for (const [tool, args] of cases) {
    const spawn = stubSpawn(okResult('{"action":"allow"}'));
    await gateToolExecuteBefore({ tool, sessionID: SESSION, callID: CALL }, { args }, { spawn });
    assert.equal(spawn.calls.length, 1, `${tool} reaches icg`);
    assert.equal(spawn.calls[0].payload.tool, tool);
    assert.deepEqual(spawn.calls[0].payload.args, args);
  }
});

test("apply_patch is routed so the classification-boundary diagnostic is real", async () => {
  const args = [{ type: "update", filePath: "/project/a.rs", diff: "*** Update File", additions: 1, deletions: 1 }];
  const spawn = stubSpawn(okResult('{"action":"allow"}'));
  await gateToolExecuteBefore({ tool: "apply_patch", sessionID: SESSION, callID: CALL }, { args }, { spawn });
  assert.equal(spawn.calls.length, 1, "apply_patch must reach icg, not fail open silently");
});

test("deny prevents execution and carries the icg reason verbatim", async () => {
  const reason = "ICG: force-push denied — reconcile with a merge commit (policy: git-safety)";
  const spawn = stubSpawn(okResult(JSON.stringify({ action: "deny", message: reason })));
  const output = { args: { command: "git push --force origin main" } };
  const lines = await captureStderr(() =>
    assert.rejects(
      gateToolExecuteBefore({ tool: "bash", sessionID: SESSION, callID: CALL }, output, { spawn }),
      (error) => error instanceof Error && error.message === reason,
    ),
  );
  // The thrown message is the model-visible channel; the stderr outcome line
  // is the operator-visible one. The reason must be verbatim on both.
  assert.ok(
    lines.some((line) => line.includes("outcome=deny") && line.includes(reason)),
    `the deny diagnostic must carry the reason verbatim (got: ${JSON.stringify(lines)})`,
  );
});

test("a deny without a reason still blocks, with an icg-attributed fallback", async () => {
  const spawn = stubSpawn(okResult('{"action":"deny"}'));
  await assert.rejects(
    gateToolExecuteBefore(
      { tool: "bash", sessionID: SESSION, callID: CALL },
      { args: { command: "x" } },
      { spawn },
    ),
    (error) =>
      error instanceof Error && error.message.startsWith("ICG: denied"),
  );
});

test("rewrite supplies the replacement that actually executes, in place", async () => {
  const replacement = {
    command: "git push --force-with-lease origin main",
    timeout: 120000,
    workdir: "/project",
  };
  const spawn = stubSpawn(okResult(JSON.stringify({ action: "rewrite", args: replacement })));
  const args = { command: "git push --force origin main", timeout: 120000, workdir: "/project" };
  const output = { args };
  const lines = await captureStderr(() =>
    gateToolExecuteBefore({ tool: "bash", sessionID: SESSION, callID: CALL }, output, { spawn }),
  );
  assert.equal(output.args, args, "the hook's args object identity is preserved (in-place mutation)");
  assert.equal(args.command, "git push --force-with-lease origin main", "the rewritten command is what executes");
  assert.deepEqual(args, replacement, "every field the harness sent survives with only the rewrite key substituted");
  // OpenCode's transcript records the model's original args, so this stderr
  // line is the rewrite's only audit trail — it must say the keys moved.
  assert.ok(
    lines.some((line) => line.includes("outcome=rewrite")),
    `a rewrite audit line must be emitted (got: ${JSON.stringify(lines)})`,
  );
});

test("a rewrite drops keys absent from the replacement (complete replacement, not a merge)", async () => {
  const spawn = stubSpawn(okResult('{"action":"rewrite","args":{"command":"echo safe"}}'));
  const args = { command: "rm -rf /", fromModel: "sneaky-extra-field" };
  const output = { args };
  await gateToolExecuteBefore({ tool: "bash", sessionID: SESSION, callID: CALL }, output, { spawn });
  assert.deepEqual(args, { command: "echo safe" });
});

test("a __proto__ key in a rewrite is an own data property, never a prototype rebind", async () => {
  // JSON.parse materializes a "__proto__" key as an own property of the
  // parsed envelope — a real icg response can carry it, because the engine
  // merges the model-controlled original args into its replacement. Applied
  // through Object.assign ([[Set]]) that key would trip the
  // Object.prototype.__proto__ setter and rebind the executed args'
  // prototype; the gate must define it as plain data instead.
  const spawn = stubSpawn(
    okResult('{"action":"rewrite","args":{"command":"echo safe","__proto__":{"injected":true}}}'),
  );
  const args = { command: "git push --force origin main" };
  const output = { args };
  await gateToolExecuteBefore({ tool: "bash", sessionID: SESSION, callID: CALL }, output, { spawn });
  assert.equal(args.command, "echo safe", "the replacement still lands");
  assert.equal(
    Object.getPrototypeOf(args),
    Object.prototype,
    "the executed args keep Object.prototype — no rebind",
  );
  assert.equal(Object.hasOwn(args, "__proto__"), true, "__proto__ travels as an own property");
  assert.deepEqual(
    Object.getOwnPropertyDescriptor(args, "__proto__")?.value,
    { injected: true },
    "the own property carries the envelope's value as data, not as a prototype link",
  );
});

test("a malformed rewrite fails open with args untouched", async () => {
  for (const envelope of [
    { action: "rewrite", args: "not-an-object" },
    { action: "rewrite", args: [1, 2] },
    { action: "rewrite" },
  ]) {
    const spawn = stubSpawn(okResult(JSON.stringify(envelope)));
    const args = { command: "keep me" };
    const output = { args };
    await gateToolExecuteBefore({ tool: "bash", sessionID: SESSION, callID: CALL }, output, { spawn });
    assert.deepEqual(args, { command: "keep me" }, "fail-open leaves args alone");
  }
});

test("allow preserves normal execution with args untouched", async () => {
  const spawn = stubSpawn(okResult('{"action":"allow"}'));
  const args = { command: "cargo build" };
  const output = { args };
  const lines = await captureStderr(() =>
    gateToolExecuteBefore({ tool: "bash", sessionID: SESSION, callID: CALL }, output, { spawn }),
  );
  assert.deepEqual(args, { command: "cargo build" });
  assert.ok(
    lines.some((line) => line.includes("outcome=allow")),
    `the allow outcome line must be emitted (got: ${JSON.stringify(lines)})`,
  );
});

test("a warning degrades to a bare allow: advisory text never blocks or reaches args", async () => {
  // render_opencode_envelope renders a Warn as {"action":"allow"}; an
  // envelope carrying extra advisory fields must behave identically. There
  // is no advisory channel at tool-call time, so the degraded outcome is
  // the same bare allow line an unconditional allow gets.
  const spawn = stubSpawn(
    okResult('{"action":"allow","message":"advisory: this command touched the state store"}'),
  );
  const args = { command: "cargo build" };
  const output = { args };
  const lines = await captureStderr(() =>
    gateToolExecuteBefore({ tool: "bash", sessionID: SESSION, callID: CALL }, output, { spawn }),
  );
  assert.deepEqual(args, { command: "cargo build" });
  assert.ok(
    lines.some((line) => line.includes("outcome=allow")),
    "a Warn must render exactly the allow outcome (degraded advisory)",
  );
});

test("infrastructure failures fail open: nonzero exit, spawn error, bad JSON, unknown action", async () => {
  const args = { command: "echo still runs" };
  const cases = [
    { label: "nonzero exit", result: { status: 1, signal: null, stdout: "", stderr: "boom" } },
    { label: "missing binary (ENOENT)", result: { status: null, signal: "SIGTERM", stdout: "", stderr: "", error: new Error("spawn /usr/local/bin/icg ENOENT") } },
    { label: "timeout", result: { status: null, signal: "SIGTERM", stdout: "", stderr: "", error: new Error("spawn timed out") } },
    { label: "non-JSON stdout", result: { status: 0, signal: null, stdout: "not json", stderr: "" } },
    { label: "non-object JSON", result: { status: 0, signal: null, stdout: '["not","an","object"]', stderr: "" } },
    { label: "unknown action", result: { status: 0, signal: null, stdout: '{"action":"block","message":"??"}', stderr: "" } },
  ];
  for (const { label, result } of cases) {
    const spawn = stubSpawn(result);
    const output = { args: { ...args } };
    const lines = await captureStderr(() =>
      gateToolExecuteBefore({ tool: "bash", sessionID: SESSION, callID: CALL }, output, { spawn }),
    );
    assert.deepEqual(output.args, { ...args }, `${label}: fail-open never mutates or blocks`);
    assertFailOpenDiagnostic(lines, label);
  }
});

test("a missing icg binary fails open through the real spawn boundary", async () => {
  // The stub matrix pins the result shapes; this one pins the real
  // spawnSync ENOENT path end to end — the shape OpenCode's process layer
  // actually produces when the binary is not there.
  const output = { args: { command: "echo ungated either way" } };
  const lines = await captureStderr(() =>
    gateToolExecuteBefore(
      { tool: "bash", sessionID: SESSION, callID: CALL },
      output,
      { binary: "/nonexistent/icg-missing-binary", spawn: realSpawnSync },
    ),
  );
  assert.deepEqual(output.args, { command: "echo ungated either way" }, "fail-open leaves the call alone");
  assertFailOpenDiagnostic(lines, "missing binary");
});

test("a wedged icg is killed at the stall cap and fails open", async () => {
  // SPAWN_TIMEOUT_MS itself is pinned by the payload test (10s is a stall
  // cap, not a budget the suite should wait out); here the REAL spawnSync
  // timeout machinery runs against a helper that ignores its args and
  // stays alive, with only the cap shortened through the same seam. The
  // helper is node-with-a-pending-timer rather than `sleep`: /bin/sleep is
  // not a given off-NixOS/Linux defaults, and node exists wherever this
  // suite itself runs.
  const dir = mkdtempSync(join(tmpdir(), "icg-stall-cap-"));
  const slowBinary = join(dir, "slow-icg");
  writeFileSync(slowBinary, "#!/usr/bin/env node\nsetTimeout(() => {}, 60000);\n", {
    mode: 0o755,
  });
  chmodSync(slowBinary, 0o755);
  const spawn = (binary, args, options) => realSpawnSync(binary, args, { ...options, timeout: 250 });
  const output = { args: { command: "echo wedged" } };
  try {
    const lines = await captureStderr(() =>
      gateToolExecuteBefore(
        { tool: "bash", sessionID: SESSION, callID: CALL },
        output,
        { binary: slowBinary, spawn },
      ),
    );
    assert.deepEqual(output.args, { command: "echo wedged" }, "the killed call proceeds ungated");
    assertFailOpenDiagnostic(lines, "wedged binary");
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});

test("invocation is PATH-independent: the absolute binary spawns under a hostile PATH", async (t) => {
  if (!existsSync(ICG_BINARY)) {
    t.skip(`the pinned binary ${ICG_BINARY} is not installed on this box`);
    return;
  }
  const raw = [];
  const spy = (binary, args, options) => {
    const result = realSpawnSync(binary, args, options);
    raw.push({ binary, args, result });
    return result;
  };
  const previousPath = process.env.PATH;
  process.env.PATH = "/nonexistent";
  const output = { args: { command: "echo hostile-path-probe" } };
  try {
    await gateToolExecuteBefore(
      { tool: "bash", sessionID: SESSION, callID: CALL },
      output,
      { spawn: spy },
    );
  } finally {
    if (previousPath === undefined) {
      delete process.env.PATH;
    } else {
      process.env.PATH = previousPath;
    }
  }
  assert.equal(raw.length, 1);
  assert.equal(raw[0].binary, ICG_BINARY);
  // The spawn itself succeeded — `error` null/undefined means the child was
  // located and executed purely via the absolute path, with PATH unusable.
  // The exit status is deliberately not pinned: an icg that predates the
  // opencode harness rejects the flag (nonzero), a current one evaluates the
  // probe and may allow (zero) — the gate fails open with a stderr
  // diagnostic either way, which is exactly what the probe above shows (it
  // did not throw and did not touch args).
  assert.equal(raw[0].result.error ?? null, null, "absolute-path spawn ignores PATH entirely");
  assert.deepEqual(output.args, { command: "echo hostile-path-probe" });
});

test("a poisoned PATH carrying a decoy icg cannot redirect the absolute invocation", async (t) => {
  if (!existsSync(ICG_BINARY)) {
    t.skip(`the pinned binary ${ICG_BINARY} is not installed on this box`);
    return;
  }
  // The realistic redirection attempt: PATH holds a directory that owns a
  // look-alike `icg`. If the gate ever resolved the binary through PATH,
  // this decoy would execute instead of the root-owned one and leave the
  // marker behind.
  const decoyDir = mkdtempSync(join(tmpdir(), "icg-hostile-path-"));
  const marker = join(decoyDir, "decoy-ran");
  writeFileSync(join(decoyDir, "icg"), `#!/bin/sh\necho hijacked > '${marker}'\n`, {
    mode: 0o755,
  });
  const previousPath = process.env.PATH;
  process.env.PATH = decoyDir;
  const raw = [];
  const spy = (binary, args, options) => {
    const result = realSpawnSync(binary, args, options);
    raw.push({ binary, result });
    return result;
  };
  const output = { args: { command: "echo redirection-probe" } };
  try {
    await gateToolExecuteBefore(
      { tool: "bash", sessionID: SESSION, callID: CALL },
      output,
      { spawn: spy },
    );
  } finally {
    if (previousPath === undefined) {
      delete process.env.PATH;
    } else {
      process.env.PATH = previousPath;
    }
    rmSync(decoyDir, { recursive: true, force: true });
  }
  assert.equal(raw.length, 1, "exactly one spawn, straight at the pinned binary");
  assert.equal(raw[0].binary, ICG_BINARY, "the root-owned absolute path is what spawns");
  assert.equal(existsSync(marker), false, "the poisoned PATH's icg never executed");
  assert.equal(raw[0].result.error ?? null, null, "the real binary ran via its absolute path");
  assert.deepEqual(output.args, { command: "echo redirection-probe" });
});

test("an emptied PATH cannot hide the absolute invocation", async (t) => {
  if (!existsSync(ICG_BINARY)) {
    t.skip(`the pinned binary ${ICG_BINARY} is not installed on this box`);
    return;
  }
  const previousPath = process.env.PATH;
  process.env.PATH = "";
  const raw = [];
  const spy = (binary, args, options) => {
    const result = realSpawnSync(binary, args, options);
    raw.push({ binary, result });
    return result;
  };
  const output = { args: { command: "echo emptied-path-probe" } };
  try {
    await gateToolExecuteBefore(
      { tool: "bash", sessionID: SESSION, callID: CALL },
      output,
      { spawn: spy },
    );
  } finally {
    if (previousPath === undefined) {
      delete process.env.PATH;
    } else {
      process.env.PATH = previousPath;
    }
  }
  assert.equal(raw.length, 1);
  assert.equal(raw[0].binary, ICG_BINARY);
  assert.equal(
    raw[0].result.error ?? null,
    null,
    "with PATH empty the absolute spawn still locates and runs the real binary",
  );
  assert.deepEqual(output.args, { command: "echo emptied-path-probe" });
});

test("applyArgsInPlace deletes stale keys and assigns the replacement", () => {
  const target = { a: 1, b: 2, stale: 3 };
  applyArgsInPlace(target, { a: 10, b: 2, fresh: 4 });
  assert.deepEqual(target, { a: 10, b: 2, fresh: 4 });
});

test("the log prefix marks every observable line", () => {
  assert.equal(LOG_PREFIX, "[icg]");
});
