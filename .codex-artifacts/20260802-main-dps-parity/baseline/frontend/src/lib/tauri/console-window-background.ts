import { getCurrentWebviewWindow } from "@tauri-apps/api/webviewWindow";
import type { Color } from "@tauri-apps/api/window";

import {
  CONSOLE_WINDOW_LABEL,
  MAIN_DPS_WINDOW_LABEL,
} from "@/lib/tauri/window-labels";

interface ConsoleWindowBackgroundDependencies {
  windowLabel: string;
  setBackgroundColor(color: Color): Promise<void>;
}

export function consoleWindowBackgroundColor(darkMode: boolean): Color {
  return darkMode ? [1, 6, 15, 255] : [246, 247, 248, 255];
}

const OPAQUE_DESKTOP_WINDOW_LABELS = [
  CONSOLE_WINDOW_LABEL,
  MAIN_DPS_WINDOW_LABEL,
];

export async function syncConsoleWindowBackground(
  darkMode: boolean,
  dependencies?: ConsoleWindowBackgroundDependencies,
): Promise<void> {
  if (dependencies) {
    if (!OPAQUE_DESKTOP_WINDOW_LABELS.includes(dependencies.windowLabel))
      return;
    await dependencies.setBackgroundColor(
      consoleWindowBackgroundColor(darkMode),
    );
    return;
  }

  const currentWindow = getCurrentWebviewWindow();
  if (!OPAQUE_DESKTOP_WINDOW_LABELS.includes(currentWindow.label)) return;
  await currentWindow.setBackgroundColor(
    consoleWindowBackgroundColor(darkMode),
  );
}
