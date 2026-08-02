import { describe, expect, it } from "vitest";

import { parseMainDpsSnapshot } from "./main-dps-contract";

const sample = {
  contractVersion: 2,
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
  },
  gameDetected: false,
};

describe("main DPS contract", () => {
  it("parses the versioned empty projection", () => {
    expect(parseMainDpsSnapshot(sample).rounds[0]?.live).toBe(true);
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
