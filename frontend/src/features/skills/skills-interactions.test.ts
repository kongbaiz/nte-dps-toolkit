import { describe, expect, it, vi } from "vitest";

import {
  copyGameplayEffectIndex,
  skillTechnicalDetails,
} from "./skills-interactions";

describe("Skills interactions", () => {
  it("keeps every available technical identifier in display order", () => {
    expect(
      skillTechnicalDetails({
        abilityName: "GA_Test",
        gameplayEffectName: "GE_Test",
        gameplayEffectIndex: 17,
      }),
    ).toEqual([
      { label: "GA", value: "GA_Test" },
      { label: "GE", value: "GE_Test" },
      { label: "GE Index", value: "17" },
    ]);
  });

  it("omits unavailable technical identifiers", () => {
    expect(
      skillTechnicalDetails({
        abilityName: null,
        gameplayEffectName: null,
        gameplayEffectIndex: null,
      }),
    ).toEqual([]);
  });

  it("copies only the unmapped gameplay effect index", async () => {
    const writeText = vi.fn().mockResolvedValue(undefined);

    await copyGameplayEffectIndex(4741, { writeText });

    expect(writeText).toHaveBeenCalledExactlyOnceWith("4741");
  });
});
