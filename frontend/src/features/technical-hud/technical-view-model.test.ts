import { describe, expect, it } from "vitest";

import type { TechnicalSnapshot } from "@/lib/tauri/technical-contract";

import {
  acceptSnapshot,
  bridgeTone,
  localeSummary,
  type TechnicalPageState,
} from "./technical-view-model";

const snapshot = (sequence: string): TechnicalSnapshot => ({
  contractVersion: 1,
  sequence,
  bridgeStatus: "ready",
  adapterVersion: "0.3.6",
  windowLabel: "hud-spike",
  uptimeMs: "1200",
  streamIntervalMs: 750,
  supportedLocales: ["en"],
  window: {
    passthrough: false,
    alwaysOnTop: true,
  },
});

describe("technical HUD projection", () => {
  it("projects loading, empty and error states explicitly", () => {
    const loading: TechnicalPageState = { status: "loading" };
    const error: TechnicalPageState = {
      status: "error",
      error: {
        code: "bridge_error",
        messageKey: "Rust bridge unavailable",
        messageArguments: [],
      },
    };

    expect(loading.status).toBe("loading");
    expect(localeSummary([])).toBeUndefined();
    expect(error.status).toBe("error");
  });

  it("ignores duplicate and out-of-order snapshots", () => {
    const current: TechnicalPageState = {
      status: "ready",
      snapshot: snapshot("12"),
    };

    expect(acceptSnapshot(current, snapshot("11"))).toBe(current);
    expect(acceptSnapshot(current, snapshot("12"))).toBe(current);
    expect(acceptSnapshot(current, snapshot("13"))).not.toBe(current);
  });

  it("maps unknown bridge enums to the visible unknown tone", () => {
    expect(bridgeTone("future_state")).toBe("unknown");
  });
});
