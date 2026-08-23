import { describe, expect, it } from "vitest";

import {
  MAIN_DPS_DETAIL_CONTRACT_VERSION,
  MAIN_DPS_DETAIL_MAX_TEXT_BYTES,
  parseMainDpsDetailSnapshot,
} from "./main-dps-detail-contract";

function snapshot(overrides: Record<string, unknown> = {}) {
  return {
    contractVersion: MAIN_DPS_DETAIL_CONTRACT_VERSION,
    generation: "12",
    kind: "character",
    abyssHalf: "first",
    characterId: 1004,
    characterName: "角色",
    characterColor: "#112233",
    filter: "all",
    qteType: null,
    skillFilter: null,
    columns: {
      showTime: true,
      showCharacter: true,
      showType: true,
      showDamage: true,
      showTarget: true,
      timeWidth: 92,
      characterWidth: 132,
      typeWidth: 250,
      damageWidth: 130,
      targetWidth: 180,
    },
    actions: { canStartCapture: true, canImportReplay: true },
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
    qteSummaryTotalCount: 0,
    qteSummariesTruncated: false,
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
    skillTotalCount: 1,
    skillsTruncated: false,
    textTruncated: false,
    totalHits: 1,
    totalDamage: 123,
    maxRowDamage: 123,
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
        overkillDamage: 0,
        maxHpReduction: 200,
        skillId: "Skill",
        skill: "Skill",
        damageType: "Basic Attack",
        typeLabel: "Basic Attack·Skill",
        reactionTextKey: null,
        damageDigitKey: "灵",
        followUpDamageDigitKey: "光",
        target: "Target",
        targetMonsterId: "mon_01",
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
    expect(parsed.rows[0]?.typeLabel).toBe("Basic Attack·Skill");
    expect(parsed.rows[0]?.overkillDamage).toBe(0);
    expect(parsed.rows[0]?.maxHpReduction).toBe(200);
    expect(parsed.columns.typeWidth).toBe(250);
  });

  it("records the exact overkill damage projected by Rust", () => {
    const value = snapshot();
    value.rows[0]!.primaryDamage = 1500;
    value.rows[0]!.damage = 1500;
    value.rows[0]!.followUpDamage = 0;
    value.rows[0]!.overkillDamage = 500;

    expect(parseMainDpsDetailSnapshot(value).rows[0]?.overkillDamage).toBe(500);
  });

  it("accepts a selected reaction type", () => {
    const parsed = parseMainDpsDetailSnapshot(
      snapshot({ filter: "qteType", qteType: "创生花" }),
    );
    expect(parsed.filter).toBe("qteType");
    expect(parsed.qteType).toBe("创生花");
  });

  it("fails loudly instead of silently repairing bounded arrays", () => {
    expect(() =>
      parseMainDpsDetailSnapshot(
        snapshot({
          qteSummaries: Array.from({ length: 33 }, () => ({
            attackType: "创生花",
            hits: 1,
            damage: 1,
            sharePercent: 1,
          })),
          qteSummaryTotalCount: 33,
          qteSummariesTruncated: true,
        }),
      ),
    ).toThrow(/qteSummaries exceeds/);
    expect(() =>
      parseMainDpsDetailSnapshot(
        snapshot({
          skills: [],
          skillTotalCount: 1,
          skillsTruncated: false,
        }),
      ),
    ).toThrow(/truncation metadata/);
  });

  it("rejects unknown filters", () => {
    expect(() =>
      parseMainDpsDetailSnapshot(snapshot({ filter: "future" })),
    ).toThrow(/filter/);
  });

  it("validates dynamic strings by UTF-8 bytes", () => {
    expect(() =>
      parseMainDpsDetailSnapshot(
        snapshot({
          characterName: "界".repeat(MAIN_DPS_DETAIL_MAX_TEXT_BYTES),
        }),
      ),
    ).toThrow(/UTF-8 bytes/);
    expect(
      parseMainDpsDetailSnapshot(
        snapshot({ characterName: "x".repeat(MAIN_DPS_DETAIL_MAX_TEXT_BYTES) }),
      ).characterName,
    ).toHaveLength(MAIN_DPS_DETAIL_MAX_TEXT_BYTES);
  });
});
