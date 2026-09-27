import { describe, expect, it, vi } from "vitest";
import { userCharacterFixture } from "../../../dev/user-character-fixture";
import {
  createUserCharactersClient,
  INITIAL_CHARACTER_REQUEST,
  parseUserCharacters,
} from "./user-characters-client";

describe("User character snapshot contract", () => {
  it("reads typed plugin data rather than resources or truncated previews", async () => {
    const invoke = vi.fn().mockResolvedValue(userCharacterFixture());
    const result = await createUserCharactersClient({ invoke }).read(
      INITIAL_CHARACTER_REQUEST,
    );
    expect(invoke).toHaveBeenCalledExactlyOnceWith("get_user_characters", {
      request: INITIAL_CHARACTER_REQUEST,
    });
    expect(result.page?.detail?.sections).toHaveLength(7);
    expect(result.page?.detail?.sections[6].available).toBe(false);
  });
  it("preserves null and exact raw zero/decimal strings", () => {
    const f = userCharacterFixture();
    f.page!.records[0].level = null;
    const result = parseUserCharacters(f);
    expect(result.page?.records[0].level).toBeNull();
    expect(
      result.page?.detail?.sections[5].entries[0].fields.at(-1)?.value,
    ).toBe("0.123456789");
  });
  it("rejects malformed, oversized, stale and incompatible responses", () => {
    for (const mutate of [
      (f: Record<string, unknown>) => {
        f.contractVersion = 2;
      },
      (f: Record<string, unknown>) => {
        delete f.page;
      },
      (f: Record<string, unknown>) => {
        f.dirty = true;
      },
      (f: Record<string, unknown>) => {
        f.state = "reading";
      },
      (f: Record<string, unknown>) => {
        f.sdkCompatible = false;
      },
    ]) {
      const f = { ...userCharacterFixture() };
      mutate(f);
      expect(() => parseUserCharacters(f)).toThrow();
    }
    const f = userCharacterFixture();
    f.page!.records = Array.from({ length: 17 }, () => f.page!.records[0]);
    expect(() => parseUserCharacters(f)).toThrow();
    const missing = userCharacterFixture();
    missing.page!.detail!.sections.pop();
    expect(() => parseUserCharacters(missing)).toThrow();
    const long = userCharacterFixture();
    long.page!.records[0].name = "x".repeat(513);
    expect(() => parseUserCharacters(long)).toThrow();
  });
  it("accepts polling and empty states only without invented rows", () => {
    const f = userCharacterFixture();
    expect(
      parseUserCharacters({ ...f, state: "reading", dirty: true, page: null })
        .page,
    ).toBeNull();
    f.page!.records = [];
    f.page!.total = 0;
    f.page!.detail = null;
    expect(parseUserCharacters(f).page?.total).toBe(0);
  });
});
