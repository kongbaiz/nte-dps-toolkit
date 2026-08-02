import { describe, expect, it } from "vitest";

import {
  parseEncryptedIniSnapshot,
  parseOpenEncryptedIniResult,
  parseSaveEncryptedIniResult,
} from "@/lib/tauri/encrypted-ini-contract";

const SNAPSHOT = {
  contractVersion: 1,
  generation: "9",
  opened: true,
  displayPath: "C:\\fixture\\Engine.ini",
  fileName: "Engine.ini",
  key: "china",
  plaintext: "Value=1",
  encryptedLineCount: 4,
  maxBytes: 8 * 1024 * 1024,
};

describe("Encrypted INI contract", () => {
  it("parses snapshots and operation wrappers", () => {
    expect(parseEncryptedIniSnapshot(SNAPSHOT).key).toBe("china");
    expect(
      parseOpenEncryptedIniResult({ opened: true, snapshot: SNAPSHOT }).opened,
    ).toBe(true);
    expect(
      parseSaveEncryptedIniResult({ saved: true, snapshot: SNAPSHOT }).saved,
    ).toBe(true);
  });

  it("rejects inconsistent open state and unsupported keys", () => {
    expect(() =>
      parseEncryptedIniSnapshot({ ...SNAPSHOT, opened: false }),
    ).toThrow(/open state/i);
    expect(() =>
      parseEncryptedIniSnapshot({ ...SNAPSHOT, key: "unknown" }),
    ).toThrow(/key/i);
  });

  it("bounds plaintext using the advertised byte limit", () => {
    expect(() =>
      parseEncryptedIniSnapshot({
        ...SNAPSHOT,
        plaintext: "中文",
        maxBytes: 5,
      }),
    ).toThrow(/maxBytes/);
  });
});
