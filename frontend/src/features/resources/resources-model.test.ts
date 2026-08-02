import { describe, expect, it } from "vitest";

import type { ResourceItemSnapshot } from "@/lib/tauri/resources-contract";

import { filterResourceItems, resourcesContentKind } from "./resources-model";

const ITEMS: ResourceItemSnapshot[] = [
  {
    severity: "error",
    category: "file",
    resourceId: "broken",
    displayName: "broken",
    messageKey: "Broken",
    messageArguments: [],
    suggestedSource: "res/data/broken.json",
  },
  {
    severity: "warning",
    category: "character",
    resourceId: "1001",
    displayName: "Character",
    messageKey: "Missing",
    messageArguments: [],
    suggestedSource: "res/data/characters.json",
  },
];

describe("Resources view model", () => {
  it("filters severity and category together", () => {
    expect(filterResourceItems(ITEMS, "warning", "character")).toEqual([
      ITEMS[1],
    ]);
    expect(filterResourceItems(ITEMS, "error", "character")).toEqual([]);
    expect(filterResourceItems(ITEMS, "all", "all")).toEqual(ITEMS);
  });

  it("selects loading, error, clear, filtered-empty and list states", () => {
    expect(resourcesContentKind("loading", 0, 0)).toBe("loading");
    expect(resourcesContentKind("error", 0, 0)).toBe("error");
    expect(resourcesContentKind("ready", 0, 0)).toBe("clear");
    expect(resourcesContentKind("ready", 2, 0)).toBe("filtered-empty");
    expect(resourcesContentKind("ready", 2, 1)).toBe("list");
  });
});
