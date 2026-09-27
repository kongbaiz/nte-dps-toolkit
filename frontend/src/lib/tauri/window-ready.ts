import { invoke } from "@tauri-apps/api/core";
import { getCurrentWindow } from "@tauri-apps/api/window";

import { MAIN_DPS_WINDOW_LABEL } from "@/lib/tauri/window-labels";

type FrameScheduler = (callback: FrameRequestCallback) => number;

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
