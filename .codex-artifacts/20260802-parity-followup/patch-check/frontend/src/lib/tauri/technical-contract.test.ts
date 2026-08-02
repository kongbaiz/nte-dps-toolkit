import { describe, expect, it } from "vitest";

import {
  parseHudSnapshot,
  parseTechnicalEvent,
  parseTechnicalSnapshot,
  TechnicalContractError,
} from "./technical-contract";

const snapshot = {
  contractVersion: 5,
  sequence: "9007199254740993",
  bridgeStatus: "ready",
  adapterVersion: "0.3.6",
  windowLabel: "hud-spike",
  uptimeMs: "1200",
  streamIntervalMs: 100,
  supportedLocales: ["en", "zh-CN"],
  window: {
    passthrough: false,
    alwaysOnTop: true,
  },
  capture: {
    phase: "idle",
    messageKey: "No live capture task right now",
    messageArguments: [],
    issue: null,
  },
  hud: {
    version: 3,
    dataState: "preview",
    config: {
      width: 380,
      moduleOrder: ["title", "summary", "status", "characters", "timeline"],
      showTitle: false,
      showTeamDps: true,
      showDuration: true,
      showTotalDamage: true,
      showCharacterRows: true,
      showDamageTaken: false,
      showAbyssHalf: false,
      showPassthroughState: false,
      showMiniTimeline: false,
    },
    summary: {
      teamDps: 64301,
      durationSeconds: 34.9,
      totalDamage: 2246285,
      totalDamageTaken: 2834,
    },
    characters: [
      {
        characterId: 1,
        name: "",
        previewLabelSuffix: "A",
        hits: "9007199254740993",
        damage: 1227500,
        dps: 35171.92,
        damageSharePercent: 54.65,
        damageTaken: 0,
        color: "#A72648",
      },
    ],
    status: {
      abyssDetected: false,
      abyssFloor: null,
      abyssHalf: null,
      abyssSuccess: false,
    },
    timeline: null,
  },
};

describe("technical contract", () => {
  it("keeps 64-bit counters as validated decimal strings", () => {
    expect(parseTechnicalSnapshot(snapshot).sequence).toBe("9007199254740993");
    expect(parseTechnicalSnapshot(snapshot).hud.characters[0].hits).toBe(
      "9007199254740993",
    );
  });

  it("accepts an unknown bridge status for forward-compatible display", () => {
    expect(
      parseTechnicalSnapshot({
        ...snapshot,
        bridgeStatus: "future_state",
      }).bridgeStatus,
    ).toBe("future_state");
  });

  it("validates stable capture status and issue fields", () => {
    const parsed = parseTechnicalSnapshot({
      ...snapshot,
      capture: {
        phase: "failed",
        messageKey: "Capture parser stopped unexpectedly",
        messageArguments: [],
        issue: {
          code: "game_not_detected",
          messageKey: "Game not detected",
          messageArguments: [],
        },
      },
    });

    expect(parsed.capture.phase).toBe("failed");
    expect(parsed.capture.issue?.code).toBe("game_not_detected");
  });

  it("rejects malformed boundary data", () => {
    expect(() => parseTechnicalSnapshot({ ...snapshot, sequence: -1 })).toThrow(
      TechnicalContractError,
    );
    expect(() =>
      parseTechnicalEvent({ event: "future_event", payload: snapshot }),
    ).toThrow("Unknown technical event");
    expect(() =>
      parseTechnicalSnapshot({ ...snapshot, contractVersion: 5 }),
    ).not.toThrow();
    expect(() =>
      parseTechnicalSnapshot({ ...snapshot, contractVersion: 6 }),
    ).toThrow("Unsupported technical contract version");
  });

  it("accepts unknown HUD enums and rejects invalid numeric payloads", () => {
    expect(
      parseHudSnapshot({
        ...snapshot.hud,
        dataState: "future_state",
      }).dataState,
    ).toBe("future_state");
    expect(() =>
      parseHudSnapshot({
        ...snapshot.hud,
        summary: { ...snapshot.hud.summary, teamDps: Number.NaN },
      }),
    ).toThrow("finite non-negative number");
  });

  it("validates bounded timeline buckets and decimal hit counts", () => {
    const timeline = {
      bucketSeconds: 1,
      durationSeconds: 2,
      peakDps: 200,
      buckets: [
        {
          startSeconds: 0,
          endSeconds: 1,
          damage: 100,
          dps: 100,
          hits: "9007199254740993",
        },
        {
          startSeconds: 1,
          endSeconds: 2,
          damage: 200,
          dps: 200,
          hits: "2",
        },
      ],
    };
    const parsed = parseHudSnapshot({ ...snapshot.hud, timeline });

    expect(parsed.timeline?.buckets[0].hits).toBe("9007199254740993");
    expect(() =>
      parseHudSnapshot({
        ...snapshot.hud,
        timeline: {
          ...timeline,
          buckets: Array.from({ length: 61 }, () => timeline.buckets[0]),
        },
      }),
    ).toThrow("exceeds 60 entries");
    expect(() =>
      parseHudSnapshot({
        ...snapshot.hud,
        timeline: {
          ...timeline,
          buckets: [
            {
              ...timeline.buckets[0],
              startSeconds: 2,
              endSeconds: 1,
            },
          ],
        },
      }),
    ).toThrow("must not precede startSeconds");
  });
});
