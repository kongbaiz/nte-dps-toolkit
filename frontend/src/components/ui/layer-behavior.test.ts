import { describe, expect, it, vi } from "vitest";

import { dismissLayerWhenClosed } from "./layer-behavior";

describe("dismissLayerWhenClosed", () => {
  it("does not dismiss while a controlled layer remains open", () => {
    const onDismiss = vi.fn();

    dismissLayerWhenClosed(true, onDismiss);

    expect(onDismiss).not.toHaveBeenCalled();
  });

  it("dismisses exactly once when a controlled layer closes", () => {
    const onDismiss = vi.fn();

    dismissLayerWhenClosed(false, onDismiss);

    expect(onDismiss).toHaveBeenCalledTimes(1);
  });
});
