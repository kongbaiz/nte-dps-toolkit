import { describe, expect, it } from "vitest";

import {
  parseSkillsEvent,
  parseSkillsSnapshot,
} from "@/lib/tauri/skills-contract";

const SKILLS_SNAPSHOT_FIXTURE = {
  contractVersion: 1,
  generation: "9007199254740992",
  scope: "all",
  hasData: true,
  totalDamage: 125,
  totalHits: "2",
  characters: [
    {
      id: 7,
      name: "Character",
      color: "#123abc",
      damage: 125,
      entries: 1,
    },
  ],
  rows: [
    {
      id: "skill-0123456789abcdef",
      characterId: 7,
      characterName: "Character",
      name: "Test Skill",
      category: "Skill",
      abilityName: "GA_Test",
      damageName: "Test Damage",
      gameplayEffectIndex: 17,
      gameplayEffectName: "GE_Test",
      followUp: false,
      hits: "2",
      damage: 125,
    },
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

describe("Skills contract", () => {
  it("keeps generations and hit counters as decimal strings", () => {
    const snapshot = parseSkillsSnapshot(SKILLS_SNAPSHOT_FIXTURE);

    expect(snapshot.generation).toBe("9007199254740992");
    expect(snapshot.totalHits).toBe("2");
    expect(snapshot.rows[0].hits).toBe("2");
  });

  it("parses tagged events and rejects unsafe colors", () => {
    expect(
      parseSkillsEvent({
        event: "snapshot",
        payload: SKILLS_SNAPSHOT_FIXTURE,
      }).scope,
    ).toBe("all");
    expect(() =>
      parseSkillsSnapshot({
        ...SKILLS_SNAPSHOT_FIXTURE,
        characters: [
          { ...SKILLS_SNAPSHOT_FIXTURE.characters[0], color: "red" },
        ],
      }),
    ).toThrow(/CSS hex color/);
  });
});
