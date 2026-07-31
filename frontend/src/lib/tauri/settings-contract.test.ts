import { describe, expect, it } from "vitest";

import {
  HUD_MODULE_IDS,
  TechnicalContractError,
} from "@/lib/tauri/technical-contract";

import {
  parseSettingsSnapshot,
  SETTINGS_CONTRACT_VERSION,
} from "./settings-contract";

function settingsFixture(): Record<string, unknown> {
  return {
    contractVersion: SETTINGS_CONTRACT_VERSION,
    adapterVersion: "0.3.6",
    interface: {
      language: "zh-CN",
      darkMode: false,
      themePreset: "zinc",
      accent: "blue",
      density: "cozy",
      reduceMotion: false,
      islandNotifications: true,
      islandOffsetX: 0,
    },
    updates: {
      currentVersion: "0.3.6",
      autoCheck: true,
      autoDownload: false,
      status: "idle",
      messageKey: "Updates have not been checked in this session",
      messageArguments: [],
    },
    capture: {
      bpfFilter: "udp",
      devices: [{ id: "device", label: "Ethernet · 192.0.2.1" }],
      manualCaptureDevice: null,
      serverDamageCalibration: false,
      separateReactionDamage: false,
      autoRoundAfterIdle: false,
      autoRoundIdleSeconds: 30,
      autoRoundIdleSecondsMin: 5,
      autoRoundIdleSecondsMax: 600,
      dpsTimeMode: "time-stop-adjusted",
      passthroughHotkey: "home",
    },
    hotkeys: {
      enabled: true,
      bindings: [
        {
          action: "capture",
          binding: {
            ctrl: true,
            alt: false,
            shift: false,
            key: "F9",
          },
        },
        {
          action: "reset",
          binding: {
            ctrl: true,
            alt: false,
            shift: false,
            key: "F10",
          },
        },
        {
          action: "hud",
          binding: {
            ctrl: true,
            alt: false,
            shift: false,
            key: "F11",
          },
        },
      ],
    },
    captureFiles: { count: 0, totalBytes: "0", formattedSize: "0 B" },
    teamData: { upperImported: false, lowerImported: false },
    alwaysOnTop: true,
    hudWidthMin: 280,
    hudWidthMax: 3840,
    hud: {
      width: 380,
      moduleOrder: [...HUD_MODULE_IDS],
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
  };
}

describe("settings contract", () => {
  it("parses the versioned HUD settings projection", () => {
    const snapshot = parseSettingsSnapshot(settingsFixture());

    expect(snapshot.contractVersion).toBe(SETTINGS_CONTRACT_VERSION);
    expect(snapshot.hud.moduleOrder).toEqual(HUD_MODULE_IDS);
    expect(snapshot.hud.showTeamDps).toBe(true);
    expect(snapshot.capture.bpfFilter).toBe("udp");
    expect(snapshot.hotkeys.bindings).toHaveLength(3);
  });

  it("rejects an unknown version and malformed module order", () => {
    const future = settingsFixture();
    future.contractVersion = SETTINGS_CONTRACT_VERSION + 1;
    expect(() => parseSettingsSnapshot(future)).toThrow(TechnicalContractError);

    const malformed = settingsFixture();
    malformed.hud = {
      ...(malformed.hud as Record<string, unknown>),
      moduleOrder: ["summary", "summary"],
    };
    expect(() => parseSettingsSnapshot(malformed)).toThrow(
      /every stable HUD module exactly once/,
    );
  });

  it("rejects a width outside the advertised bounds", () => {
    const malformed = settingsFixture();
    malformed.hud = {
      ...(malformed.hud as Record<string, unknown>),
      width: 120,
    };

    expect(() => parseSettingsSnapshot(malformed)).toThrow(
      /inside the advertised bounds/,
    );
  });

  it("rejects duplicate hotkey actions and unsafe integer projections", () => {
    const duplicate = settingsFixture();
    const hotkeys = duplicate.hotkeys as Record<string, unknown>;
    hotkeys.bindings = [
      ...(hotkeys.bindings as unknown[]).slice(0, 2),
      (hotkeys.bindings as unknown[])[0],
    ];
    expect(() => parseSettingsSnapshot(duplicate)).toThrow(
      /every stable action exactly once/,
    );

    const invalidBytes = settingsFixture();
    invalidBytes.captureFiles = {
      count: 1,
      totalBytes: 42,
      formattedSize: "42 B",
    };
    expect(() => parseSettingsSnapshot(invalidBytes)).toThrow(
      /must be a string/,
    );
  });
});
