import { describe, expect, it } from "vitest";

import { TechnicalContractError } from "./technical-contract";
import {
  ABYSS_VALUES_CONTRACT_VERSION,
  parseAbyssPredictionTeams,
  parseAbyssValuesSnapshot,
} from "./abyss-values-contract";

export function abyssValuesFixture(): Record<string, unknown> {
  return {
    contractVersion: ABYSS_VALUES_CONTRACT_VERSION,
    dataVersion: "fixture-v1",
    dataUpdatedAt: "2026-08-20T00:00:00Z",
    dataStale: false,
    seasonCount: 1,
    floorCount: 1,
    monsterCount: 2,
    seasons: [
      {
        season: 7,
        name: "月恒环线",
        floors: [
          {
            season: 7,
            seasonName: "月恒环线",
            floor: 8,
            name: "第八站",
            monsterCount: 2,
            waveCount: 1,
            maxSeconds: 600,
            starThresholds: [{ stars: 3, seconds: 300 }],
            recommendedElements: {
              firstHalf: ["光"],
              secondHalf: ["灵"],
            },
            monsters: [
              {
                packId: "pack:0",
                attributeId: "attribute",
                monsterPoolId: "pool",
                monsterId: "mon_01_BP",
                name: "测试怪物",
                count: 2,
                level: 79,
                half: 0,
                wave: 1,
                isBoss: false,
                hpMaxBase: 1000,
                rawProps: { HPMaxBase: 1000, CritBase: 0.2 },
              },
            ],
          },
        ],
      },
    ],
    teams: {
      upper: {
        dps: 500,
        members: [{ id: 3, dps: 500, name: "角色" }],
      },
      lower: null,
    },
    currentTeamAvailable: {
      upper: true,
      lower: false,
    },
  };
}

describe("abyss values contract", () => {
  it("parses the authoritative floor and stat projection", () => {
    const snapshot = parseAbyssValuesSnapshot(abyssValuesFixture());

    expect(snapshot.seasons[0].floors[0].monsters[0]).toMatchObject({
      monsterId: "mon_01_BP",
      hpMaxBase: 1000,
      rawProps: { HPMaxBase: 1000, CritBase: 0.2 },
    });
    expect(snapshot.teams.upper?.dps).toBe(500);
    expect(snapshot.currentTeamAvailable).toEqual({
      upper: true,
      lower: false,
    });
    expect(snapshot.dataStale).toBe(false);
  });

  it("rejects version, count, and numeric drift", () => {
    const future = abyssValuesFixture();
    future.contractVersion = ABYSS_VALUES_CONTRACT_VERSION + 1;
    expect(() => parseAbyssValuesSnapshot(future)).toThrow(
      TechnicalContractError,
    );

    const countMismatch = abyssValuesFixture();
    countMismatch.floorCount = 2;
    expect(() => parseAbyssValuesSnapshot(countMismatch)).toThrow(
      "floorCount does not match floors",
    );

    expect(() =>
      parseAbyssPredictionTeams({
        upper: { dps: Number.NaN, members: [] },
        lower: null,
      }),
    ).toThrow(TechnicalContractError);
  });
});
