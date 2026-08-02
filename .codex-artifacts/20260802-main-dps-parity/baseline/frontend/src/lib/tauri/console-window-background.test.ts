import { describe, expect, it, vi } from "vitest";

import {
  consoleWindowBackgroundColor,
  syncConsoleWindowBackground,
} from "./console-window-background";

describe("Console window background", () => {
  it("matches the opaque light and dark presentation colors", () => {
    expect(consoleWindowBackgroundColor(false)).toEqual([246, 247, 248, 255]);
    expect(consoleWindowBackgroundColor(true)).toEqual([1, 6, 15, 255]);
  });

  it("updates both opaque desktop window layers", async () => {
    const setBackgroundColor = vi.fn(async () => undefined);
    await syncConsoleWindowBackground(false, {
      windowLabel: "console",
      setBackgroundColor,
    });
    expect(setBackgroundColor).toHaveBeenCalledWith([246, 247, 248, 255]);

    setBackgroundColor.mockClear();
    await syncConsoleWindowBackground(true, {
      windowLabel: "main-dps",
      setBackgroundColor,
    });
    expect(setBackgroundColor).toHaveBeenCalledWith([1, 6, 15, 255]);

    setBackgroundColor.mockClear();
    await syncConsoleWindowBackground(false, {
      windowLabel: "hud-spike",
      setBackgroundColor,
    });
    expect(setBackgroundColor).not.toHaveBeenCalled();
  });
});
