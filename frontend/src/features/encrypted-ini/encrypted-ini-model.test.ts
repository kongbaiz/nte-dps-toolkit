import { describe, expect, it } from "vitest";

import {
  encryptedIniCenteredScrollOffset,
  encryptedIniLineColumn,
  findEncryptedIniMatches,
  nextEncryptedIniMatch,
  previousEncryptedIniMatch,
} from "./encrypted-ini-model";

describe("Encrypted INI model", () => {
  it("finds non-overlapping case-insensitive matches", () => {
    expect(findEncryptedIniMatches("Header=1\nheader=2", "HDR")).toEqual([]);
    expect(findEncryptedIniMatches("Header=1\nheader=2", "header")).toEqual([
      0, 9,
    ]);
  });

  it("cycles search positions", () => {
    expect(nextEncryptedIniMatch(null, 2)).toBe(0);
    expect(nextEncryptedIniMatch(1, 2)).toBe(0);
    expect(previousEncryptedIniMatch(null, 2)).toBe(1);
    expect(previousEncryptedIniMatch(0, 2)).toBe(1);
    expect(nextEncryptedIniMatch(null, 0)).toBeNull();
  });

  it("reports one-based line and column", () => {
    expect(encryptedIniLineColumn("Alpha\nBeta", 8)).toEqual({
      line: 2,
      column: 3,
    });
  });

  it("centers a match and clamps at both scroll boundaries", () => {
    expect(encryptedIniCenteredScrollOffset(24, 240, 2400)).toBe(0);
    expect(encryptedIniCenteredScrollOffset(1200, 240, 2400)).toBe(1080);
    expect(encryptedIniCenteredScrollOffset(2390, 240, 2400)).toBe(2160);
  });
});
