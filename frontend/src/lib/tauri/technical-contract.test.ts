import { describe, expect, it } from "vitest";

import {
  parseTechnicalEvent,
  parseTechnicalSnapshot,
  TechnicalContractError,
} from "./technical-contract";

const snapshot = {
  contractVersion: 1,
  sequence: "9007199254740993",
  bridgeStatus: "ready",
  adapterVersion: "0.3.6",
  windowLabel: "hud-spike",
  uptimeMs: "1200",
  streamIntervalMs: 750,
  supportedLocales: ["en", "zh-CN"],
  window: {
    passthrough: false,
    alwaysOnTop: true,
  },
};

describe("technical contract", () => {
  it("keeps 64-bit counters as validated decimal strings", () => {
    expect(parseTechnicalSnapshot(snapshot).sequence).toBe("9007199254740993");
  });

  it("accepts an unknown bridge status for forward-compatible display", () => {
    expect(
      parseTechnicalSnapshot({
        ...snapshot,
        bridgeStatus: "future_state",
      }).bridgeStatus,
    ).toBe("future_state");
  });

  it("rejects malformed boundary data", () => {
    expect(() => parseTechnicalSnapshot({ ...snapshot, sequence: -1 })).toThrow(
      TechnicalContractError,
    );
    expect(() =>
      parseTechnicalEvent({ event: "future_event", payload: snapshot }),
    ).toThrow("Unknown technical event");
    expect(() =>
      parseTechnicalSnapshot({ ...snapshot, contractVersion: 2 }),
    ).toThrow("Unsupported technical contract version");
  });
});
