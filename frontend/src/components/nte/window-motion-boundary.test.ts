import { describe, expect, it } from "vitest";

import { isWindowMotionTarget } from "./window-motion-target";

describe("window motion target", () => {
  it("plays the entrance only for the window named by the native event", () => {
    expect(isWindowMotionTarget("main-dps", "main-dps")).toBe(true);
    expect(isWindowMotionTarget("console", "main-dps")).toBe(false);
    expect(isWindowMotionTarget("hud-spike", undefined)).toBe(false);
  });
});
