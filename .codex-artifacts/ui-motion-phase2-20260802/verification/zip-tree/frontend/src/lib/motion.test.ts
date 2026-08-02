import { afterEach, describe, expect, it, vi } from "vitest";

import {
  easeOutCubic,
  interpolateNumber,
  startMotionViewTransition,
} from "./motion";

afterEach(() => vi.unstubAllGlobals());

describe("motion helpers", () => {
  it("clamps easing progress to the animation range", () => {
    expect(easeOutCubic(-1)).toBe(0);
    expect(easeOutCubic(0)).toBe(0);
    expect(easeOutCubic(1)).toBe(1);
    expect(easeOutCubic(2)).toBe(1);
  });

  it("interpolates toward the next value with eased progress", () => {
    expect(interpolateNumber(10, 20, 0)).toBe(10);
    expect(interpolateNumber(10, 20, 1)).toBe(20);
    expect(interpolateNumber(10, 20, 0.5)).toBeGreaterThan(15);
  });

  it("updates immediately when reduced motion is active", () => {
    const transition = vi.fn();
    vi.stubGlobal("document", {
      documentElement: { classList: { contains: () => true } },
      startViewTransition: transition,
    });
    vi.stubGlobal("window", { matchMedia: () => ({ matches: false }) });
    const update = vi.fn();

    startMotionViewTransition(update);

    expect(update).toHaveBeenCalledOnce();
    expect(transition).not.toHaveBeenCalled();
  });

  it("uses the native view transition when motion is enabled", () => {
    const transition = vi.fn((update: () => void) => update());
    vi.stubGlobal("document", {
      documentElement: { classList: { contains: () => false } },
      startViewTransition: transition,
    });
    vi.stubGlobal("window", { matchMedia: () => ({ matches: false }) });
    const update = vi.fn();

    startMotionViewTransition(update);

    expect(transition).toHaveBeenCalledOnce();
    expect(update).toHaveBeenCalledOnce();
  });
});
