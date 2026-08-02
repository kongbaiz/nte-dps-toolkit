import { describe, expect, it } from "vitest";

import { parseMainDpsDetailSnapshot } from "./main-dps-detail-contract";

describe("main DPS detail contract", () => {
  it("parses a bounded page from the native detail projection", () => {
    const snapshot = parseMainDpsDetailSnapshot({
      contractVersion: 1,
      generation: "12",
      kind: "character",
      characterId: 1004,
      characterName: "角色",
      filter: "all",
      totalHits: 1,
      totalDamage: 123,
      offset: 0,
      rows: [
        {
          id: "1:0",
          timestamp: 1,
          characterId: 1004,
          characterName: "角色",
          direction: "outgoing",
          damage: 123,
          skill: "Skill",
          target: "Target",
        },
      ],
    });
    expect(snapshot.kind).toBe("character");
    expect(snapshot.rows[0]?.damage).toBe(123);
  });

  it("rejects unknown filters", () => {
    expect(() =>
      parseMainDpsDetailSnapshot({
        contractVersion: 1,
        generation: "1",
        kind: "team",
        characterId: null,
        characterName: null,
        filter: "future",
        totalHits: 0,
        totalDamage: 0,
        offset: 0,
        rows: [],
      }),
    ).toThrow(/filter/);
  });
});
