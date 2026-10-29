// The one bridge to the Rust back end: rpc("domain.verb", params).
import { invoke } from "@tauri-apps/api/core";

export class RpcError extends Error {
  code: string;
  constructor(code: string, message: string) {
    super(message);
    this.code = code;
  }
}

export async function rpc<T = unknown>(method: string, params: object = {}): Promise<T> {
  try {
    return await invoke<T>("rpc", { method, params });
  } catch (e) {
    const err = e as { code?: string; message?: string } | string;
    if (typeof err === "string") throw new RpcError("internal", err);
    throw new RpcError(err.code ?? "internal", err.message ?? String(e));
  }
}

/** An error for the log file (app.log): a failed rpc by its code (the back end has logged the method;
 *  the message can quote names, addresses, files), anything else as text (masked again there). */
export function errorForLog(e: unknown, withStack = false): string {
  if (e instanceof RpcError) return `rpc error (${e.code})`;
  if (withStack && e instanceof Error && e.stack) return e.stack;
  return String(e);
}

// Exposed for scripts/cdp.mjs end-to-end tests and debugging from devtools.
(window as unknown as { __cm: unknown }).__cm = { rpc };
