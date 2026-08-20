import { describe, expect, it } from "vitest";

import {
  RESOURCES_MAX_ITEMS,
  parseResourcesSnapshot,
} from "@/lib/tauri/resources-contract";

const SNAPSHOT = {
  contractVersion: 2,
  errorCount: 1,
  warningCount: 0,
  itemCount: 1,
  displayLimit: RESOURCES_MAX_ITEMS,
  counts: {
    characters: 2,
    skillDamage: 3,
    mappedEffects: 4,
    semanticEffects: 5,
    reactions: 7,
  },
  items: [
    {
      severity: "error",
      category: "file",
      resourceId: "res/data/test.json",
      displayName: "test.json",
      messageKey: "Resource JSON is invalid.",
      messageArguments: [],
      suggestedSource: "res/data/test.json",
    },
  ],
  redactedReport: "redacted",
};

describe("Resources contract", () => {
  it("parses the bounded runtime audit projection", () => {
    expect(parseResourcesSnapshot(SNAPSHOT)).toEqual(SNAPSHOT);
  });

  it("rejects unknown enums and oversized item collections", () => {
    expect(() =>
      parseResourcesSnapshot({
        ...SNAPSHOT,
        items: [{ ...SNAPSHOT.items[0], category: "externalExport" }],
      }),
    ).toThrow(/category/);
    expect(() =>
      parseResourcesSnapshot({
        ...SNAPSHOT,
        displayLimit: RESOURCES_MAX_ITEMS,
        items: Array.from({ length: RESOURCES_MAX_ITEMS + 1 }, () => ({})),
      }),
    ).toThrow(/UI bounds/);
  });
});
