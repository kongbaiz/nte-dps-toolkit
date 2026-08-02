import { describe, expect, it } from "vitest";

import type { EmptyCurtainItem } from "@/lib/tauri/empty-curtain-contract";

import {
  activeEquipmentFilterCount,
  emptyEquipmentFilters,
  filterEquipmentItems,
  matchesEquipmentSearch,
} from "./equipment-filters";

function item(
  filterId: string,
  characterId: number | null,
  mainstat: string,
  substats: string[],
): EmptyCurtainItem {
  return {
    uid: { slot: Number(filterId), serial: 1 },
    itemId: filterId,
    filterId,
    kind: "core",
    quality: "orange",
    name: filterId,
    icon: null,
    level: 20,
    maxLevel: 20,
    locked: false,
    discarded: false,
    equippedCharacterUid: null,
    equippedCharacterId: characterId,
    equippedPlacement: null,
    stats: [
      {
        property: mainstat,
        label: mainstat,
        value: 1,
        percent: false,
        main: true,
        unlockLevel: null,
        unlocked: true,
      },
      ...substats.map((property) => ({
        property,
        label: property,
        value: 1,
        percent: false,
        main: false,
        unlockLevel: null,
        unlocked: true,
      })),
    ],
    setName: null,
    setEffects: [],
  };
}

describe("equipment filters", () => {
  const items = [
    item("1", 1004, "attack", ["crit", "damage"]),
    item("2", 1054, "health", ["crit", "defense"]),
    item("3", null, "defense", ["damage"]),
  ];

  it("matches any selected main stat and all selected substats", () => {
    const filters = {
      ...emptyEquipmentFilters(),
      mainstats: ["attack", "health"],
      substats: ["crit", "damage"],
    };
    expect(
      filterEquipmentItems(items, filters).map((row) => row.filterId),
    ).toEqual(["1"]);
  });

  it("combines character and cassette filters", () => {
    const filters = {
      ...emptyEquipmentFilters(),
      filterIds: ["2"],
      characterIds: [1054],
    };
    expect(filterEquipmentItems(items, filters)).toEqual([items[1]]);
  });

  it("counts main stats as active filters", () => {
    expect(
      activeEquipmentFilterCount({
        ...emptyEquipmentFilters(),
        mainstats: ["attack"],
      }),
    ).toBe(1);
  });

  it("searches localized names case-insensitively and ignores edge spaces", () => {
    expect(matchesEquipmentSearch("Shinku: Twin Butterflies", " twin ")).toBe(
      true,
    );
    expect(matchesEquipmentSearch("真红：双生蝶", "真红")).toBe(true);
    expect(matchesEquipmentSearch("失落光芒", "双生")).toBe(false);
  });
});
