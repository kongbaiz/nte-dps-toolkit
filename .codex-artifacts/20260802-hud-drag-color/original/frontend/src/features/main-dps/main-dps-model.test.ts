import { describe, expect, it } from "vitest";

import {
  appearancePanelPosition,
  characterAccent,
  damagePercent,
  formatMainMetric,
  isGenerationNewer,
  mainCaptureStatusTone,
  mainCharacterListState,
  mainDpsContentState,
  roundLabel,
} from "./main-dps-model";

describe("main DPS view model", () => {
  it("formats bounded summary metrics and localized round metadata", () => {
    expect(formatMainMetric(5688311.4)).toMatch(/5.688.311|5,688,311/);
    expect(
      roundLabel({ id: null, live: true, displayTime: null, abyssFloor: null }),
    ).toBe("实时");
    expect(
      roundLabel({
        id: "round",
        live: false,
        displayTime: "01:46:07",
        abyssFloor: 2,
      }),
    ).toContain("2");
  });

  it("formats attribution percentages and stable character accents", () => {
    expect(damagePercent(27.2, 100)).toBeCloseTo(27.2);
    expect(damagePercent(1, 0)).toBe(0);
    expect(characterAccent(1010, "#123abc")).toBe("#123abc");
    expect(characterAccent(1010, "not-a-color")).toBe("#059669");
    expect(characterAccent(1010, null)).toBe("#059669");
  });

  it("orders string generations without Number precision loss", () => {
    expect(isGenerationNewer("9007199254740993", "9007199254740992")).toBe(
      true,
    );
    expect(isGenerationNewer("9007199254740992", "9007199254740993")).toBe(
      false,
    );
  });

  it("distinguishes capture and empty ranking presentation states", () => {
    expect(mainCaptureStatusTone("running")).toBe("active");
    expect(mainCaptureStatusTone("starting")).toBe("transition");
    expect(mainCaptureStatusTone("failed")).toBe("error");
    expect(mainCaptureStatusTone("idle")).toBe("idle");
    expect(mainCharacterListState(10, 0, 0)).toBe("unattributed");
    expect(mainCharacterListState(0, 2, 0)).toBe("hidden");
    expect(mainCharacterListState(0, 0, 0)).toBe("combat-empty");
    expect(mainCharacterListState(10, 2, 1)).toBe("rows");
    expect(mainDpsContentState(true)).toBe("replay-loading");
    expect(mainDpsContentState(false)).toBe("ready");
  });

  it("keeps the appearance panel inside a compact main window", () => {
    expect(
      appearancePanelPosition(
        { left: 350, right: 412, top: 48, bottom: 76 },
        { width: 420, height: 360 },
      ),
    ).toEqual({ left: 236, top: 82 });

    expect(
      appearancePanelPosition(
        { left: 350, right: 412, top: 320, bottom: 348 },
        { width: 420, height: 360 },
      ),
    ).toEqual({ left: 236, top: 154 });
  });
});
