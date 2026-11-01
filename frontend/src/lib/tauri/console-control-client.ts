import { invokeCommand } from "@/lib/tauri/command-error";
import type { ConsoleControlAction } from "@/lib/tauri/generated/ConsoleControlAction";

export const consoleControlClient = {
  execute: (action: ConsoleControlAction) =>
    invokeCommand<void>("execute_console_control", { action }),
};
