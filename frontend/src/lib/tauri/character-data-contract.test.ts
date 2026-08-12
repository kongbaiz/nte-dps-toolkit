import { describe, expect, it } from "vitest";

import {
  CHARACTER_DATA_MAX_RECORDS,
  parseCharacterDataSnapshot,
} from "@/lib/tauri/character-data-contract";

const RECORD = {
  id: 1003,
  nameZh: "早雾",
  nameEn: "Sagiri",
  codename: "Sagiri",
  attribute: "咒",
  verified: true,
  color: "#123ABC",
  avatar: "res/images/characters/player_003.png",
};

describe("Character data contract", () => {
  it("parses the bounded character snapshot", () => {
    expect(
      parseCharacterDataSnapshot({
        contractVersion: 1,
        generation: "9007199254740992",
        attributes: ["灵", "咒"],
        records: [RECORD],
      }),
    ).toEqual(
      expect.objectContaining({
        generation: "9007199254740992",
        records: [expect.objectContaining({ id: 1003, verified: true })],
      }),
    );
  });

  it("rejects unsupported versions and oversized record lists", () => {
    expect(() =>
      parseCharacterDataSnapshot({
        contractVersion: 2,
        generation: "0",
        attributes: [],
        records: [],
      }),
    ).toThrow(/Unsupported character data contract version/);
    expect(() =>
      parseCharacterDataSnapshot({
        contractVersion: 1,
        generation: "0",
        attributes: [],
        records: Array.from(
          { length: CHARACTER_DATA_MAX_RECORDS + 1 },
          () => RECORD,
        ),
      }),
    ).toThrow(/exceeds display bounds/);
  });

  it("rejects malformed colors at the typed boundary", () => {
    expect(() =>
      parseCharacterDataSnapshot({
        contractVersion: 1,
        generation: "0",
        attributes: [],
        records: [{ ...RECORD, color: "red" }],
      }),
    ).toThrow(/#RRGGBB/);
  });
});
