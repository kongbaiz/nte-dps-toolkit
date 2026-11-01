import { afterEach, describe, expect, it } from "vitest";

import {
  replaceCharacterAvatarCatalog,
  resolveCharacterAvatar,
} from "./character-avatar";

afterEach(() => replaceCharacterAvatarCatalog([]));

describe("runtime character avatar catalog", () => {
  it("replaces an older mapping instead of retaining the compiled catalog", () => {
    replaceCharacterAvatarCatalog([
      {
        id: 1076,
        avatar: "res/images/characters/player_zhenhong.png",
      },
    ]);
    expect(resolveCharacterAvatar(1076)).toContain("player_zhenhong.png");

    replaceCharacterAvatarCatalog([
      {
        id: 1003,
        avatar: "res/images/characters/player_003.png",
      },
    ]);
    expect(resolveCharacterAvatar(1076)).toBeNull();
    expect(resolveCharacterAvatar(1003)).toContain("player_003.png");
  });

  it("ignores empty avatar paths", () => {
    replaceCharacterAvatarCatalog([{ id: 1076, avatar: "  " }]);
    expect(resolveCharacterAvatar(1076)).toBeNull();
  });
});
