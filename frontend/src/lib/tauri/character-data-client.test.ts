import { describe, expect, it, vi } from "vitest";

import { createCharacterDataClient } from "@/lib/tauri/character-data-client";

const SNAPSHOT = {
  contractVersion: 1,
  generation: "1",
  attributes: ["灵"],
  records: [],
};

describe("Character data client", () => {
  it("uses typed snapshot and save commands", async () => {
    const invoke = vi.fn(async () => SNAPSHOT);
    const client = createCharacterDataClient({ invoke });
    const input = {
      originalId: null,
      id: "1080",
      nameZh: "新角色",
      nameEn: "New Character",
      codename: "New",
      attribute: "灵",
      verified: false,
      color: "#123ABC",
      avatar: "",
    };

    await client.getSnapshot();
    await client.saveRecord(input);

    expect(invoke).toHaveBeenNthCalledWith(
      1,
      "get_character_data_snapshot",
      undefined,
    );
    expect(invoke).toHaveBeenNthCalledWith(2, "save_character_data_record", {
      input,
    });
  });
});
