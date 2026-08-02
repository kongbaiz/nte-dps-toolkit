import { describe, expect, it } from "vitest";

import type { CharacterDataRecord } from "@/lib/tauri/character-data-contract";

import {
  characterDraftMatchesRecord,
  characterRecordDraft,
  filterCharacterRecords,
  newCharacterDraft,
} from "./character-data-model";

const RECORD: CharacterDataRecord = {
  id: 1003,
  nameZh: "早雾",
  nameEn: "Sagiri",
  codename: "Sagiri",
  attribute: "咒",
  verified: true,
  color: "#123ABC",
  avatar: "res/images/characters/player_003_256.png",
};

describe("Character data model", () => {
  it("searches by ID, localized names, codename and attribute", () => {
    expect(filterCharacterRecords([RECORD], "1003")).toEqual([RECORD]);
    expect(filterCharacterRecords([RECORD], "sagiri")).toEqual([RECORD]);
    expect(filterCharacterRecords([RECORD], "咒")).toEqual([RECORD]);
    expect(filterCharacterRecords([RECORD], "missing")).toEqual([]);
  });

  it("tracks existing and new drafts without mutating records", () => {
    const existing = characterRecordDraft(RECORD);
    expect(characterDraftMatchesRecord(existing, RECORD)).toBe(true);
    expect(
      characterDraftMatchesRecord({ ...existing, nameZh: "修改" }, RECORD),
    ).toBe(false);
    expect(newCharacterDraft("1080")).toEqual(
      expect.objectContaining({ originalId: null, id: "1080" }),
    );
  });
});
