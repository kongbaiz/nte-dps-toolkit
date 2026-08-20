import { describe, expect, it } from "vitest";

import {
  compareDecimalStrings,
  shouldAcceptDecimalVersion,
} from "./decimal-string";

describe("compareDecimalStrings", () => {
  it("orders decimal generations without losing u64 precision", () => {
    expect(compareDecimalStrings("9", "10")).toBeLessThan(0);
    expect(
      compareDecimalStrings("18446744073709551615", "9007199254740992"),
    ).toBeGreaterThan(0);
    expect(compareDecimalStrings("42", "42")).toBe(0);
  });

  it("rejects stale versions and only accepts equal refresh responses", () => {
    expect(shouldAcceptDecimalVersion(null, "7")).toBe(true);
    expect(shouldAcceptDecimalVersion("10", "9")).toBe(false);
    expect(shouldAcceptDecimalVersion("10", "10")).toBe(false);
    expect(shouldAcceptDecimalVersion("10", "10", true)).toBe(true);
    expect(shouldAcceptDecimalVersion("10", "11")).toBe(true);
  });

  it("fails loudly for non-canonical versions instead of accepting corruption", () => {
    expect(() => shouldAcceptDecimalVersion("1", "invalid")).toThrow(
      /incoming must be a canonical decimal string/,
    );
    expect(() => shouldAcceptDecimalVersion("01", "2")).toThrow(
      /accepted must be a canonical decimal string/,
    );
    expect(() => compareDecimalStrings("01", "1")).toThrow(TypeError);
  });
});
