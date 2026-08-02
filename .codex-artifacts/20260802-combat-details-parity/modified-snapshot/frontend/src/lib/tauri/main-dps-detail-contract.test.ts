import { describe, expect, it } from "vitest";

import { parseMainDpsDetailSnapshot } from "./main-dps-detail-contract";

function snapshot(overrides: Record<string, unknown> = {}) {
  return {
    contractVersion: 2,
    generation: "12",
    kind: "character",
    abyssHalf: "first",
    characterId: 1004,
    characterName: "角色",
    characterColor: "#112233",
    filter: "all",
    qteType: null,
    skillFilter: null,
    metrics: {
      totalOutput: 123,
      dps: 41,
      outputCount: 1,
      incomingCount: 0,
      totalDamageTaken: 0,
      durationSeconds: 3,
    },
    direction: {
      confirmedOutput: 123,
      confirmedHits: 1,
      candidateOutput: 0,
      candidateHits: 0,
      incomingOutput: 0,
      incomingHits: 0,
      candidateSharePercent: 0,
    },
    hitTypes: [
      { id: "all", hits: 1, damage: 123 },
      { id: "outgoing", hits: 1, damage: 123 },
      { id: "incoming", hits: 0, damage: 0 },
    ],
    attribution: {
      totalDamage: 123,
      characterDamage: 123,
      characterFilter: "characterAttributed",
      reactionDamage: 0,
      sharedDamage: 0,
      unattributedDamage: 0,
      separateReactionDamage: false,
    },
    qteSummaries: [],
    skills: [
      {
        id: "Skill",
        name: "Skill",
        category: "Basic Attack",
        hits: 1,
        damage: 123,
        sharePercent: 100,
      },
    ],
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
        primaryDamage: 100,
        followUpDamage: 23,
        skillId: "Skill",
        skill: "Skill",
        damageType: "Basic Attack",
        target: "Target",
        targetHpAfter: 877,
        targetMaxHp: 1000,
        targetHpPercent: 87.7,
      },
    ],
    ...overrides,
  };
}

describe("main DPS detail contract", () => {
  it("parses the old-detail parity projection and target HP", () => {
    const parsed = parseMainDpsDetailSnapshot(snapshot());
    expect(parsed.kind).toBe("character");
    expect(parsed.metrics.dps).toBe(41);
    expect(parsed.skills[0]?.sharePercent).toBe(100);
    expect(parsed.rows[0]?.targetHpPercent).toBe(87.7);
  });

  it("accepts a selected reaction type", () => {
    const parsed = parseMainDpsDetailSnapshot(
      snapshot({ filter: "qteType", qteType: "创生花" }),
    );
    expect(parsed.filter).toBe("qteType");
    expect(parsed.qteType).toBe("创生花");
  });

  it("rejects unknown filters", () => {
    expect(() =>
      parseMainDpsDetailSnapshot(snapshot({ filter: "future" })),
    ).toThrow(/filter/);
  });
});
