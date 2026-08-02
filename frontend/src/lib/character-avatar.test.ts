import { afterEach, describe, expect, it } from "vitest";

import {
  characterAvatarUrl,
  replaceCharacterAvatarCatalog,
} from "./character-avatar";

afterEach(() => replaceCharacterAvatarCatalog([]));

describe("runtime character avatar catalog", () => {
  it("replaces an older mapping instead of retaining the compiled catalog", () => {
    replaceCharacterAvatarCatalog([
      {
        id: 1076,
        avatar: "res/images/characters/player_zhenhong_256.png",
      },
    ]);
    expect(characterAvatarUrl(1076)).toContain("player_zhenhong_256.png");

    replaceCharacterAvatarCatalog([
      {
        id: 1003,
        avatar: "res/images/characters/player_003_256.png",
      },
    ]);
    expect(characterAvatarUrl(1076)).toBeNull();
    expect(characterAvatarUrl(1003)).toContain("player_003_256.png");
  });

  it("ignores empty avatar paths", () => {
    replaceCharacterAvatarCatalog([{ id: 1076, avatar: "  " }]);
    expect(characterAvatarUrl(1076)).toBeNull();
  });
});
