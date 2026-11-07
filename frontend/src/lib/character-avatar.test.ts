import { afterEach, describe, expect, it } from "vitest";
import characters from "@res/data/characters/characters.json";
import { panelImage } from "@/features/character-data/character-build-model";

import {
  replaceCharacterAvatarCatalog,
  resolveCharacterAvatar,
} from "./character-avatar";

afterEach(() => replaceCharacterAvatarCatalog([]));

describe("runtime character avatar catalog", () => {
  it.each(["1074", "1090"] as const)(
    "resolves appearance-named avatars for character %i in both views",
    (id) => {
      const avatar = characters.characters[id].avatar;
      const file = avatar.split("/").at(-1)!;
      replaceCharacterAvatarCatalog([{ id: Number(id), avatar }]);
      expect(resolveCharacterAvatar(Number(id))).toContain(file);
      expect(panelImage("avatar", String(id))).toContain(file);
    },
  );

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
