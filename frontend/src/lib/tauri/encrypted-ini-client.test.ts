import { describe, expect, it, vi } from "vitest";

import { createEncryptedIniClient } from "@/lib/tauri/encrypted-ini-client";

const SNAPSHOT = {
  contractVersion: 1,
  generation: "1",
  opened: false,
  displayPath: null,
  fileName: null,
  key: "global",
  plaintext: "",
  encryptedLineCount: 0,
  maxBytes: 8 * 1024 * 1024,
};

describe("Encrypted INI client", () => {
  it("uses only the five typed commands", async () => {
    const invoke = vi.fn(async (command: string) => {
      if (command === "open_encrypted_ini") {
        return { opened: false, snapshot: SNAPSHOT };
      }
      if (command === "save_encrypted_ini") {
        return { saved: false, snapshot: SNAPSHOT };
      }
      return SNAPSHOT;
    });
    const client = createEncryptedIniClient({ invoke });

    await client.getSnapshot();
    await client.open();
    await client.reload();
    await client.save({
      expectedGeneration: "1",
      key: "global",
      plaintext: "Value=1",
    });
    await client.clear();

    expect(invoke.mock.calls).toEqual([
      ["get_encrypted_ini_snapshot"],
      ["open_encrypted_ini"],
      ["reload_encrypted_ini"],
      [
        "save_encrypted_ini",
        {
          request: {
            expectedGeneration: "1",
            key: "global",
            plaintext: "Value=1",
          },
        },
      ],
      ["clear_encrypted_ini"],
    ]);
  });
});
