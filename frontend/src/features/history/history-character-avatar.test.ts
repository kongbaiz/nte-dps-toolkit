import { describe, expect, it } from "vitest";

import { historyCharacterAvatarUrl } from "./history-character-avatar";

describe("History character avatars", () => {
  it("resolves a bundled avatar by stable character id", () => {
    expect(historyCharacterAvatarUrl(1076)).toContain(
      "player_zhenhong_256.png",
    );
  });

  it("keeps unknown and pseudo characters on the fallback tile", () => {
    expect(historyCharacterAvatarUrl(0)).toBeNull();
    expect(historyCharacterAvatarUrl(999_999)).toBeNull();
  });
});
