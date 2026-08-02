import { invoke } from "@tauri-apps/api/core";

import { parseTechnicalCommandError } from "@/lib/tauri/technical-contract";

export type ConsoleControlAction =
  | "toggle-capture"
  | "reset-session"
  | "toggle-hud"
  | "toggle-passthrough"
  | "toggle-processing"
  | "toggle-pin"
  | "open-team-details"
  | "open-capture-logs";

interface ConsoleControlTransport {
  invoke(
    command: string,
    arguments_?: Record<string, unknown>,
  ): Promise<unknown>;
}

const tauriTransport: ConsoleControlTransport = {
  invoke: (command, arguments_) => invoke(command, arguments_),
};

export function createConsoleControlClient(
  transport: ConsoleControlTransport = tauriTransport,
) {
  return {
    async execute(action: ConsoleControlAction): Promise<void> {
      try {
        await transport.invoke("execute_console_control", { action });
      } catch (error) {
        throw parseTechnicalCommandError(error);
      }
    },
  };
}

export const consoleControlClient = createConsoleControlClient();
