import { describe, expect, it, vi } from "vitest";

import { createResourcesClient } from "@/lib/tauri/resources-client";

const SNAPSHOT = {
  contractVersion: 1,
  errorCount: 0,
  warningCount: 0,
  itemCount: 0,
  displayLimit: 20_000,
  counts: {
    characters: 0,
    skillDamage: 0,
    mappedEffects: 0,
    semanticEffects: 0,
    abyssMonsters: 0,
    reactions: 0,
  },
  items: [],
  redactedReport: "redacted",
};

describe("Resources client", () => {
  it("uses the typed runtime audit command", async () => {
    const invoke = vi.fn(async () => SNAPSHOT);
    const client = createResourcesClient({ invoke });

    await client.getSnapshot();

    expect(invoke).toHaveBeenCalledWith("get_resources_snapshot");
  });
});
