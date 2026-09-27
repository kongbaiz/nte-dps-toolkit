import { parseTechnicalCommandError } from "@/lib/tauri/technical-contract";
import {
  tauriInvokeTransport,
  type InvokeTransport,
} from "@/lib/tauri/stream-client";

export type ConsoleControlAction =
  | "toggle-capture"
  | "reset-session"
  | "toggle-hud"
  | "toggle-passthrough"
  | "toggle-processing"
  | "toggle-pin"
  | "open-team-details"
  | "open-capture-logs";

export function createConsoleControlClient(
  transport: InvokeTransport = tauriInvokeTransport,
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
