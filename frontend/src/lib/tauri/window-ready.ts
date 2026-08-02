import { invoke } from "@tauri-apps/api/core";
import { getCurrentWindow } from "@tauri-apps/api/window";

import {
  CONSOLE_WINDOW_LABEL,
  MAIN_DPS_WINDOW_LABEL,
} from "@/lib/tauri/window-labels";

const SHOW_CONSOLE_WHEN_READY_COMMAND = "show_console_when_ready";

type FrameScheduler = (callback: FrameRequestCallback) => number;

interface WindowReadyDependencies {
  windowLabel: string;
  scheduleFrame: FrameScheduler;
  showConsole(): Promise<unknown>;
}

export async function revealPrimaryWindowAfterFirstPaint(
  dependencies: {
    windowLabel: string;
    scheduleFrame: FrameScheduler;
    showMain(): Promise<unknown>;
  } = {
    windowLabel: getCurrentWindow().label,
    scheduleFrame: requestAnimationFrame,
    showMain: () => invoke("show_main_dps_when_ready"),
  },
): Promise<void> {
  if (dependencies.windowLabel !== MAIN_DPS_WINDOW_LABEL) return;
  await waitForAnimationFrames(dependencies.scheduleFrame, 2);
  await dependencies.showMain();
}

export async function revealConsoleAfterFirstPaint(
  dependencies: WindowReadyDependencies = {
    windowLabel: getCurrentWindow().label,
    scheduleFrame: requestAnimationFrame,
    showConsole: () => invoke(SHOW_CONSOLE_WHEN_READY_COMMAND),
  },
): Promise<void> {
  if (dependencies.windowLabel !== CONSOLE_WINDOW_LABEL) return;
  await waitForAnimationFrames(dependencies.scheduleFrame, 2);
  await dependencies.showConsole();
}

function waitForAnimationFrames(
  scheduleFrame: FrameScheduler,
  remaining: number,
): Promise<void> {
  if (remaining === 0) return Promise.resolve();
  return new Promise((resolve) => {
    scheduleFrame(() => {
      void waitForAnimationFrames(scheduleFrame, remaining - 1).then(resolve);
    });
  });
}
