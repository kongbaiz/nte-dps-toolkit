import { describe, expect, it } from "vitest";

import {
  parseIslandSnapshot,
  parseIslandCommandError,
} from "./island-contract";

describe("notification island contract", () => {
  it("accepts the normalized info tone used for capture lifecycle notices", () => {
    const state = {
      contractVersion: 1,
      enabled: true,
      notice: {
        id: "notice-1",
        tone: "info",
        messageKey: "Starting live capture...",
        messageArguments: [],
        undoAvailable: false,
        remainingMs: 1000,
      },
    };
    expect(parseIslandSnapshot(state).notice?.tone).toBe("info");
    try {
      parseIslandSnapshot({
        ...state,
        notice: { ...state.notice, tone: "status" },
      });
    } catch (error) {
      expect(parseIslandCommandError(error)).toMatchObject({
        code: "island_invalid_snapshot",
        messageKey: "Notification data is invalid.",
      });
    }
    expect(parseIslandCommandError(new Error("transport"))).toMatchObject({
      code: "island_unavailable",
    });
  });
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
