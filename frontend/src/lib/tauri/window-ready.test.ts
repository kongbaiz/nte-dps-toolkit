import { describe, expect, it, vi } from "vitest";

import { revealConsoleAfterFirstPaint } from "./window-ready";

describe("window readiness", () => {
  it("shows Console only after two painted frames", async () => {
    const frames: FrameRequestCallback[] = [];
    const showConsole = vi.fn(async () => undefined);
    const pending = revealConsoleAfterFirstPaint({
      windowLabel: "console",
      scheduleFrame: (callback) => frames.push(callback),
      showConsole,
    });

    expect(frames).toHaveLength(1);
    frames.shift()?.(0);
    expect(showConsole).not.toHaveBeenCalled();
    expect(frames).toHaveLength(1);
    frames.shift()?.(16);
    await pending;

    expect(showConsole).toHaveBeenCalledOnce();
  });

  it("keeps non-Console windows under their existing Rust lifecycle", async () => {
    const scheduleFrame = vi.fn(() => 1);
    const showConsole = vi.fn(async () => undefined);

    await revealConsoleAfterFirstPaint({
      windowLabel: "hud-spike",
      scheduleFrame,
      showConsole,
    });

    expect(scheduleFrame).not.toHaveBeenCalled();
    expect(showConsole).not.toHaveBeenCalled();
  });
});
