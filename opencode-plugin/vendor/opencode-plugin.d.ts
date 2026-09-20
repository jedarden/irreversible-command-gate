// Minimal offline declarations for the surface of `@opencode-ai/plugin`
// 1.18.29 this plugin uses, so `scripts/opencode-plugin-typecheck` needs no
// node_modules at all (a clean `git archive` extraction has none). The
// shapes are transcribed from the pinned SDK installed at
// ~/.config/opencode/node_modules/@opencode-ai/plugin/dist/index.d.ts
// (`tool.execute.before` at index.d.ts:235-241, sha256 f3ec1a15… — see
// docs/research/opencode-1.18.29-plugin-surface.md §4.1). This file pins
// the V1 surface only; a changed upstream index.d.ts sha256 is a re-pin
// trigger, not something to absorb here silently.

declare module "@opencode-ai/plugin" {
  export interface ToolExecuteBeforeInput {
    tool: string;
    sessionID: string;
    callID: string;
  }

  export interface ToolExecuteBeforeOutput {
    args: any;
  }

  export interface Hooks {
    dispose?: () => Promise<void>;
    "tool.execute.before"?: (
      input: ToolExecuteBeforeInput,
      output: ToolExecuteBeforeOutput,
    ) => Promise<void>;
  }

  export interface PluginInput {
    project: unknown;
    client: unknown;
    $: unknown;
    directory: string;
    worktree: string;
    [key: string]: unknown;
  }

  export type Plugin = (input: PluginInput) => Promise<Hooks>;
}
