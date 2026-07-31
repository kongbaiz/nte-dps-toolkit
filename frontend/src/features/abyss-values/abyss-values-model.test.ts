import { describe, expect, it } from "vitest";

import type {
  AbyssMonster,
  AbyssSeason,
} from "@/lib/tauri/abyss-values-contract";

import {
  filterMonsters,
  findFloor,
  formatSeconds,
  initialFloorKey,
  lineTotalHp,
  monsterImageUrl,
  monsterImageStemCandidates,
  predictedSeconds,
  requiredDps,
  summarizeWaves,
} from "./abyss-values-model";

function monster(overrides: Partial<AbyssMonster> = {}): AbyssMonster {
  return {
    packId: "pack:0",
    attributeId: "attribute",
    monsterPoolId: "pool",
    monsterId: "mon_35_Red_BP",
    name: "罐头锡兵",
    count: 2,
    level: 77,
    half: 0,
    wave: 1,
    isBoss: false,
    hpMaxBase: 1000,
    rawProps: { HPMaxBase: 1000 },
    ...overrides,
  };
}

describe("abyss values view model", () => {
  it("uses the latest season and finds the selected floor", () => {
    const seasons = [
      { season: 6, floors: [] },
      {
        season: 7,
        floors: [{ season: 7, floor: 8 }],
      },
    ] as AbyssSeason[];

    const key = initialFloorKey(seasons);
    expect(key).toEqual({ season: 7, floor: 8 });
    expect(findFloor(seasons, key)?.floor).toBe(8);
  });

  it("filters by every stable monster identifier", () => {
    const values = [monster()];

    expect(filterMonsters(values, "锡兵")).toHaveLength(1);
    expect(filterMonsters(values, "PACK:0")).toHaveLength(1);
    expect(filterMonsters(values, "mon_35")).toHaveLength(1);
    expect(filterMonsters(values, "missing")).toHaveLength(0);
  });

  it("calculates line HP, target DPS, predictions, and wave groups", () => {
    const values = [
      monster(),
      monster({
        packId: "pack:1",
        count: 1,
        hpMaxBase: 3000,
        wave: 2,
      }),
    ];

    expect(lineTotalHp(values)).toBe(5000);
    expect(requiredDps(values, 100)).toBe(50);
    expect(predictedSeconds(values, { dps: 100, members: [] })).toBe(50);
    expect(summarizeWaves(values)).toEqual([
      { wave: 1, hp: 2000, monsterCount: 2 },
      { wave: 2, hp: 3000, monsterCount: 1 },
    ]);
    expect(formatSeconds(420)).toBe("7m00.0s");
  });

  it("resolves the same trimmed portrait stems as the original page", () => {
    expect(monsterImageStemCandidates("mon_35_Red_BP")).toEqual(
      expect.arrayContaining(["mon_35_Red_BP", "mon_35_Red", "mon_35"]),
    );
    expect(monsterImageStemCandidates("mon_019_BP")).toContain("mon_19");
    expect(monsterImageStemCandidates("mon_030_BP")).toContain("mon_30");
    expect(monsterImageStemCandidates("Boss_016_BP")).toContain("Boss_16");
    expect(monsterImageStemCandidates("mon_025_double_1_BP")).toContain(
      "mon_25",
    );

    expect(monsterImageUrl("mon_019_BP")).toBeTruthy();
    expect(monsterImageUrl("mon_030_BP")).toBeTruthy();
  });
});
