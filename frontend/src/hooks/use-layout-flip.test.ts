import { describe, expect, it } from "vitest";

import { layoutDelta } from "./use-layout-flip";

describe("layoutDelta", () => {
  it("inverts a downward move so the item starts at its old position", () => {
    expect(layoutDelta({ left: 10, top: 20 }, { left: 10, top: 60 })).toEqual({
      x: 0,
      y: -40,
    });
  });

  it("tracks horizontal and vertical movement independently", () => {
    expect(layoutDelta({ left: 48, top: 72 }, { left: 12, top: 24 })).toEqual({
      x: 36,
      y: 48,
    });
  });
});
