import { afterEach, describe, expect, it, vi } from "vitest";

import { startMotionViewTransition } from "./motion";

afterEach(() => vi.unstubAllGlobals());

describe("motion helpers", () => {
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
