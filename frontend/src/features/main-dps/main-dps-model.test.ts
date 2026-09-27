import { describe, expect, it } from "vitest";

import {
  characterAccent,
  damagePercent,
  formatMainMetric,
  isGenerationNewer,
  mainCaptureStatusTone,
  mainCharacterListState,
  mainDpsContentState,
  maxHpCompression,
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
    expect(characterAccent(1010, "not-a-color")).toBe("#40a31f");
    expect(characterAccent(1010, null)).toBe("#40a31f");
  });

  it("projects an observed max-HP reduction as a bounded bar compression", () => {
    const observed = maxHpCompression(2_924_242, 623_492);
    expect(observed.remainingMaxHp).toBe(2_300_750);
    expect(observed.reductionPercent).toBeCloseTo(21.3215, 4);
    expect(observed.remainingPercent).toBeCloseTo(78.6785, 4);
    expect(maxHpCompression(1_000, 1_500)).toEqual({
      remainingMaxHp: 0,
      reductionPercent: 100,
      remainingPercent: 0,
    });
    expect(maxHpCompression(0, 500)).toEqual({
      remainingMaxHp: 0,
      reductionPercent: 0,
      remainingPercent: 0,
    });
  });

  it("orders string generations without Number precision loss", () => {
    expect(isGenerationNewer("9007199254740993", "9007199254740992")).toBe(
      true,
    );
    expect(isGenerationNewer("9007199254740992", "9007199254740993")).toBe(
      false,
    );
    expect(
      isGenerationNewer("18446744073709551615", "18446744073709551614"),
    ).toBe(true);
    expect(
      isGenerationNewer("18446744073709551614", "18446744073709551615"),
    ).toBe(false);
    expect(isGenerationNewer("18446744073709551615", null)).toBe(true);
    expect(
      isGenerationNewer("18446744073709551615", "18446744073709551615"),
    ).toBe(false);
    expect(() => isGenerationNewer("corrupt", "1")).toThrow(TypeError);
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
});
