import { describe, expect, it } from "vitest";

import type { SkillsSnapshot } from "@/lib/tauri/skills-contract";

import {
  buildSkillCharacterSummaries,
  skillDamageShare,
  skillRowsForCharacter,
  skillViewMetrics,
} from "./skills-ui-model";

const SNAPSHOT: SkillsSnapshot = {
  contractVersion: 1,
  generation: "1",
  scope: "all",
  hasData: true,
  totalDamage: 3_940_855,
  totalHits: "202",
  characters: [
    {
      id: 1076,
      name: "真红",
      color: "#ef4444",
      damage: 2_945_156,
      entries: 5,
    },
    {
      id: 1025,
      name: "哈索尔",
      color: "#f59e0b",
      damage: 994_610,
      entries: 3,
    },
    {
      id: 1010,
      name: "娜娜莉",
      color: "#8b5cf6",
      damage: 1_089,
      entries: 2,
    },
  ],
  rows: [
    row("a", 1076, 2_945_156),
    row("b", 1025, 493_420),
    row("c", 1025, 286_930),
    row("d", 1025, 214_260),
    row("e", 1010, 1_000),
    row("f", 1010, 89),
  ],
  diagnostics: {
    unknownCharacterCount: "0",
    unknownCharacterHits: "0",
    unknownDirectionHits: "0",
    unknownDirectionDamage: 0,
    unmappedSkillRows: "0",
    unmappedSkillHits: "0",
    unmappedSkillDamage: 0,
    unmappedGameplayEffects: [],
  },
};

function row(id: string, characterId: number, damage: number) {
  return {
    id,
    characterId,
    characterName: String(characterId),
    name: id,
    category: "Skill",
    abilityName: null,
    damageName: null,
    gameplayEffectIndex: null,
    gameplayEffectName: null,
    followUp: false,
    hits: "1",
    damage,
  };
}

describe("Skills UI model", () => {
  it("aggregates characters in descending damage order", () => {
    const summaries = buildSkillCharacterSummaries(SNAPSHOT);

    expect(summaries.map((character) => character.id)).toEqual([
      1076, 1025, 1010,
    ]);
    expect(summaries[0]).toMatchObject({ entries: 5, damage: 2_945_156 });
    expect(
      summaries.reduce((sum, character) => sum + character.share, 0),
    ).toBeCloseTo(1);
  });

  it("filters and sorts skill rows for a selected character", () => {
    const rows = skillRowsForCharacter(SNAPSHOT, 1025);

    expect(rows).toHaveLength(3);
    expect(rows.every((row) => row.characterId === 1025)).toBe(true);
    expect(rows.map((row) => row.damage)).toEqual([493_420, 286_930, 214_260]);
  });

  it("derives visible metrics and guards empty shares", () => {
    const rows = skillRowsForCharacter(SNAPSHOT, 1010);

    expect(skillViewMetrics(rows)).toEqual({ damage: 1_089, entries: 2 });
    expect(skillDamageShare(25, 100)).toBe(0.25);
    expect(skillDamageShare(25, 0)).toBe(0);
  });
});
