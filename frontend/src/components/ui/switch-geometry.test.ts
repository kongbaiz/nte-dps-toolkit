import { describe, expect, it } from "vitest";

import { SWITCH_GEOMETRY } from "./switch-geometry";

describe("Switch geometry", () => {
  it("keeps the thumb inside both density-independent tracks", () => {
    for (const geometry of Object.values(SWITCH_GEOMETRY)) {
      const borderInsetPx = 1;

      expect(geometry.trackHeightPx - borderInsetPx * 2).toBe(
        geometry.thumbDiameterPx,
      );
      expect(geometry.trackWidthPx - borderInsetPx * 2).toBe(
        geometry.thumbDiameterPx + geometry.thumbTravelPx,
      );
    }
  });
});
