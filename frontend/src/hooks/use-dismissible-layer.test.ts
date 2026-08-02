import { describe, expect, it } from "vitest";

import { dismissibleLayerEventIsOutside } from "./use-dismissible-layer";

describe("dismissible layer", () => {
  it("keeps trigger and layer interactions open", () => {
    const triggerChild = {} as Node;
    const layerChild = {} as Node;
    const outside = {} as Node;
    const trigger = { contains: (target: Node) => target === triggerChild };
    const layer = { contains: (target: Node) => target === layerChild };

    expect(dismissibleLayerEventIsOutside(triggerChild, layer, trigger)).toBe(
      false,
    );
    expect(dismissibleLayerEventIsOutside(layerChild, layer, trigger)).toBe(
      false,
    );
    expect(dismissibleLayerEventIsOutside(outside, layer, trigger)).toBe(true);
  });

  it("treats a missing trigger as an outside interaction", () => {
    const target = {} as Node;
    expect(dismissibleLayerEventIsOutside(target, null, null)).toBe(true);
  });
});
