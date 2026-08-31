import { describe, expect, it, vi } from "vitest";

import { revealPrimaryWindowAfterFirstPaint } from "./window-ready";

describe("window readiness", () => {
  it("reveals the main DPS window after two painted frames", async () => {
    const frames: FrameRequestCallback[] = [];
    const showMain = vi.fn(async () => undefined);
    const pending = revealPrimaryWindowAfterFirstPaint({
      windowLabel: "main-dps",
      scheduleFrame: (callback) => frames.push(callback),
      showMain,
    });

    frames.shift()?.(0);
    expect(showMain).not.toHaveBeenCalled();
    frames.shift()?.(16);
    await pending;
    expect(showMain).toHaveBeenCalledOnce();
  });
});
