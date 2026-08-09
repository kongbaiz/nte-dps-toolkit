import { afterEach, describe, expect, it } from "vitest";

import { replaceCharacterAvatarCatalog } from "@/lib/character-avatar";

import { historyCharacterAvatarUrl } from "./history-character-avatar";

afterEach(() => replaceCharacterAvatarCatalog([]));

describe("History character avatars", () => {
  it("resolves a bundled avatar by stable character id", () => {
    replaceCharacterAvatarCatalog([
      {
        id: 1076,
        avatar: "res/images/characters/player_zhenhong.png",
      },
    ]);
    expect(historyCharacterAvatarUrl(1076)).toContain("player_zhenhong.png");
  });

  it("keeps unknown and pseudo characters on the fallback tile", () => {
    expect(historyCharacterAvatarUrl(0)).toBeNull();
    expect(historyCharacterAvatarUrl(999_999)).toBeNull();
  });
});
