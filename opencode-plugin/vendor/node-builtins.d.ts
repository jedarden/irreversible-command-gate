// Minimal offline declarations for the node builtins the plugin uses, so
// the typecheck needs no @types/node. Only what icg.ts touches is declared.

declare module "node:child_process" {
  export interface SpawnSyncResult {
    status: number | null;
    signal: string | null;
    stdout: string;
    stderr: string;
    error?: Error;
  }

  export interface SpawnSyncOptions {
    input: string;
    timeout: number;
    encoding: "utf8";
    maxBuffer: number;
  }

  export function spawnSync(
    binary: string,
    args: readonly string[],
    options: SpawnSyncOptions,
  ): SpawnSyncResult;
}

// lib.es2022 has no console (that rides the DOM or @types/node).
declare var console: {
  error: (...data: unknown[]) => void;
};
