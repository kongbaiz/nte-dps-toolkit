import { describe, expect, it } from "vitest";

import { parseIslandSnapshot } from "./island-contract";

describe("notification island contract", () => {
  it("parses a bounded undo notice", () => {
    expect(
      parseIslandSnapshot({
        contractVersion: 1,
        enabled: true,
        notice: {
          id: "notice-7",
          tone: "success",
          messageKey: "Stats reset",
          messageArguments: [],
          undoAvailable: true,
          remainingMs: 4_900,
        },
      }).notice,
    ).toMatchObject({ id: "notice-7", undoAvailable: true });
  });

  it("rejects unknown visual tones", () => {
    expect(() =>
      parseIslandSnapshot({
        contractVersion: 1,
        enabled: true,
        notice: {
          id: "notice-8",
          tone: "custom",
          messageKey: "Stats reset",
          messageArguments: [],
          undoAvailable: false,
          remainingMs: 1,
        },
      }),
    ).toThrow();
  });
});
