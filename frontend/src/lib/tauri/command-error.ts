import { invoke, type InvokeArgs } from "@tauri-apps/api/core";

import type { CommandError } from "@/lib/tauri/generated/CommandError";

export type { CommandError };

const BRIDGE_UNAVAILABLE: CommandError = {
  code: "bridge_unavailable",
  messageKey: "Rust bridge unavailable",
  messageArguments: [],
};

/** Tauri rejects with the Rust `CommandError`, or with a JS error when IPC itself fails. */
export function toCommandError(value: unknown): CommandError {
  return typeof value === "object" && value !== null && "messageKey" in value
    ? (value as CommandError)
    : BRIDGE_UNAVAILABLE;
}

/** `invoke` whose rejection is always a `CommandError`. */
export async function invokeCommand<T>(
  command: string,
  arguments_?: InvokeArgs,
): Promise<T> {
  try {
    return await invoke<T>(command, arguments_);
  } catch (error) {
    throw toCommandError(error);
  }
}
