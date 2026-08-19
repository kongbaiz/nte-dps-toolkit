import { describe, expect, it } from "vitest";

import {
  MAIN_DPS_MAX_HISTORY_RECORDS,
  MAIN_DPS_MAX_ROUNDS,
  parseMainDpsSnapshot,
} from "./main-dps-contract";

const sample = {
  contractVersion: 4,
  generation: "7",
  captureGeneration: "3",
  presentationGeneration: "2",
  historyGeneration: "1",
  adapterVersion: "0.3.6",
  capture: {
    phase: "idle",
    messageKey: "Idle",
    messageArguments: [],
    issue: null,
  },
  processingPaused: false,
  pausedPendingEvents: "0",
  pausedDebugPackets: "0",
  replayRunning: false,
  alwaysOnTop: true,
  passthrough: false,
  appearance: {
    language: "zh-CN",
    darkMode: false,
    themePreset: "zinc",
    accent: "zinc",
    density: "cozy",
    reduceMotion: false,
    opacity: 1,
  },
  rounds: [{ id: null, live: true, displayTime: null, abyssFloor: null }],
  selectedRoundId: null,
  readout: {
    dataState: "empty",
    summary: {
      teamDps: 0,
      durationSeconds: 0,
      totalDamage: 0,
      totalDamageTaken: 0,
    },
    characters: [],
    damageAttribution: {
      totalDamage: 0,
      characterDirectDamage: 0,
      characterReactionDamage: 0,
      sharedDamage: 0,
      unattributedDamage: 0,
      separateReactionDamage: false,
    },
    abyss: { detected: false, floor: null, half: null, success: false },
  },
  actions: {
    canStartCapture: true,
    canStopCapture: false,
    canReset: false,
    canStartNewRound: false,
    canPause: false,
    canResume: false,
    canImportReplay: true,
    characterDetailsAvailable: false,
    teamDetailsAvailable: false,
    startCaptureRequiresConfirmation: false,
    resetRequiresConfirmation: false,
    importReplayRequiresConfirmation: false,
  },
  gameDetected: false,
  gameDetectionStatus: "notRunning",
  hasLiveSessionData: false,
  onboarding: {
    done: true,
    step: 0,
    captureDeviceCount: 1,
    captureDevicesAvailable: true,
    gameDetected: false,
    gameDetectionStatus: "notRunning",
    passthroughHotkeyLabel: "F12",
    passthroughHotkeyReady: true,
  },
};

describe("main DPS contract", () => {
  it("parses the versioned empty projection", () => {
    const parsed = parseMainDpsSnapshot(sample);
    expect(parsed.rounds[0]?.live).toBe(true);
    expect(parsed.onboarding.captureDevicesAvailable).toBe(true);
  });

  it("requires an explicit capture-device availability status", () => {
    const onboarding = { ...sample.onboarding } as Record<string, unknown>;
    delete onboarding.captureDevicesAvailable;

    expect(() => parseMainDpsSnapshot({ ...sample, onboarding })).toThrow(
      /onboarding.captureDevicesAvailable must be boolean/,
    );
  });

  it.each([
    "generation",
    "captureGeneration",
    "presentationGeneration",
    "historyGeneration",
  ] as const)("rejects non-canonical u64 values for %s", (field) => {
    for (const invalid of [
      "",
      "-1",
      "+1",
      "01",
      "1.0",
      "not-a-generation",
      "100000000000000000000",
      "18446744073709551616",
    ]) {
      expect(() =>
        parseMainDpsSnapshot({ ...sample, [field]: invalid }),
      ).toThrow(`${field} must be a u64 decimal string`);
    }
  });

  it("preserves canonical u64 generations without Number precision loss", () => {
    const generation = "18446744073709551615";
    const parsed = parseMainDpsSnapshot({
      ...sample,
      generation,
      captureGeneration: generation,
      presentationGeneration: generation,
      historyGeneration: generation,
    });

    expect(parsed.generation).toBe(generation);
    expect(parsed.captureGeneration).toBe(generation);
    expect(parsed.presentationGeneration).toBe(generation);
    expect(parsed.historyGeneration).toBe(generation);
    expect(
      parseMainDpsSnapshot({ ...sample, generation: "0" }).generation,
    ).toBe("0");
  });

  it("preserves probe failure instead of treating it as a normal negative", () => {
    const parsed = parseMainDpsSnapshot({
      ...sample,
      gameDetectionStatus: "probeFailed",
      onboarding: {
        ...sample.onboarding,
        gameDetectionStatus: "probeFailed",
      },
    });

    expect(parsed.gameDetected).toBe(false);
    expect(parsed.gameDetectionStatus).toBe("probeFailed");
    expect(parsed.onboarding.gameDetectionStatus).toBe("probeFailed");
  });

  it("accepts the Rust history limit plus exactly one live row", () => {
    const history = Array.from(
      { length: MAIN_DPS_MAX_HISTORY_RECORDS },
      (_, index) => ({
        id: `record-${index}`,
        live: false,
        displayTime: "00:00:00",
        abyssFloor: null,
      }),
    );

    const parsed = parseMainDpsSnapshot({
      ...sample,
      rounds: [
        ...history,
        { id: null, live: true, displayTime: null, abyssFloor: null },
      ],
    });

    expect(parsed.rounds).toHaveLength(MAIN_DPS_MAX_ROUNDS);
    expect(parsed.rounds.filter((round) => round.live)).toHaveLength(1);
  });

  it("rejects an over-limit history list instead of silently slicing it", () => {
    const rounds: Array<{
      id: string | null;
      live: boolean;
      displayTime: string | null;
      abyssFloor: number | null;
    }> = Array.from({ length: MAIN_DPS_MAX_ROUNDS + 1 }, (_, index) => ({
      id: `record-${index}`,
      live: false,
      displayTime: "00:00:00",
      abyssFloor: null,
    }));
    rounds.push({ id: null, live: true, displayTime: null, abyssFloor: null });

    expect(() => parseMainDpsSnapshot({ ...sample, rounds })).toThrow(
      "rounds exceeds the contract limit",
    );
  });

  it("rejects snapshots without exactly one live row", () => {
    expect(() => parseMainDpsSnapshot({ ...sample, rounds: [] })).toThrow(
      "rounds must contain exactly one live row",
    );
    expect(() =>
      parseMainDpsSnapshot({
        ...sample,
        rounds: [
          { id: null, live: true, displayTime: null, abyssFloor: null },
          { id: null, live: true, displayTime: null, abyssFloor: null },
        ],
      }),
    ).toThrow("rounds must contain exactly one live row");
    expect(() =>
      parseMainDpsSnapshot({
        ...sample,
        rounds: [
          {
            id: "live-with-id",
            live: true,
            displayTime: null,
            abyssFloor: null,
          },
        ],
      }),
    ).toThrow("live row must have a null id");
  });

  it("rejects unbounded numeric values at the bridge", () => {
    expect(() =>
      parseMainDpsSnapshot({
        ...sample,
        readout: {
          ...sample.readout,
          summary: { ...sample.readout.summary, totalDamage: Number.NaN },
        },
      }),
    ).toThrow();
  });

  it("parses detailed character and damage-attribution fields", () => {
    const parsed = parseMainDpsSnapshot({
      ...sample,
      readout: {
        ...sample.readout,
        damageAttribution: {
          totalDamage: 100,
          characterDirectDamage: 72.5,
          characterReactionDamage: 27.2,
          sharedDamage: 0.3,
          unattributedDamage: 0,
          separateReactionDamage: true,
        },
        characters: [
          {
            characterId: 1010,
            name: "Nanally",
            hits: "139",
            damage: 72.5,
            dps: 40,
            damageSharePercent: 72.5,
            damageTaken: 3,
            durationSeconds: 29,
            color: null,
            attribute: "灵",
          },
        ],
      },
    });

    expect(parsed.readout.characters[0]).toMatchObject({
      name: "Nanally",
      durationSeconds: 29,
    });
    expect(parsed.readout.damageAttribution.separateReactionDamage).toBe(true);
  });
});
