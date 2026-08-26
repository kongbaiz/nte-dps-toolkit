import { describe, expect, it } from "vitest";

import {
  HUD_MODULE_IDS,
  TechnicalContractError,
} from "@/lib/tauri/technical-contract";

import {
  compareSettingsGeneration,
  parseTeamDataImportFileResult,
  parseSettingsSnapshot,
  SETTINGS_CONTRACT_VERSION,
} from "./settings-contract";

function settingsFixture(): Record<string, unknown> {
  return {
    contractVersion: SETTINGS_CONTRACT_VERSION,
    generation: "7",
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
      available: [],
      activeComponent: null,
      downloadedBytes: "0",
      totalBytes: "0",
      prepared: null,
      installEnabled: false,
      installBlockedMessageKey: null,
    },
    capture: {
      bpfFilter: "udp",
      devicesAvailable: true,
      devices: [{ id: "device", label: "Ethernet · 192.0.2.1" }],
      manualCaptureDevice: null,
      serverDamageCalibration: false,
      includeMaxHpReductionInTotalDamage: false,
      separateReactionDamage: false,
      autoRoundAfterIdle: false,
      autoRoundIdleSeconds: 30,
      autoRoundIdleSecondsMin: 5,
      autoRoundIdleSecondsMax: 600,
      dpsTimeMode: "time-stop-adjusted",
      dpsTimeRuntime: {
        configuredMode: "time-stop-adjusted",
        effectiveMode: "real-time",
        combatClockHealth: "unknown",
        degraded: true,
        warningMessageKey:
          "Time-stop adjustment has not been verified for this session.",
      },
      passthroughHotkey: {
        ctrl: false,
        alt: false,
        shift: false,
        key: "Home",
      },
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
        {
          action: "new-round",
          binding: null,
        },
      ],
    },
    mainDps: {
      metrics: ["team-dps", "total-damage", "total-damage-taken", "duration"],
      attributions: [
        "character",
        "reaction",
        "shared",
        "unattributed",
        "max-hp-reduction",
      ],
    },
    captureFiles: { count: 0, totalBytes: "0", formattedSize: "0 B" },
    teamData: {
      available: true,
      upperImported: false,
      lowerImported: false,
    },
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
    expect(snapshot.generation).toBe("7");
    expect(snapshot.hud.moduleOrder).toEqual(HUD_MODULE_IDS);
    expect(snapshot.hud.showTeamDps).toBe(true);
    expect(snapshot.capture.bpfFilter).toBe("udp");
    expect(snapshot.capture.devicesAvailable).toBe(true);
    expect(snapshot.capture.includeMaxHpReductionInTotalDamage).toBe(false);
    expect(snapshot.teamData.available).toBe(true);
    expect(snapshot.hotkeys.bindings).toHaveLength(4);
    expect(snapshot.capture.passthroughHotkey.key).toBe("Home");
    expect(snapshot.mainDps.attributions).toContain("max-hp-reduction");
  });

  it("accepts custom keys and rejects duplicate display identifiers", () => {
    const custom = settingsFixture();
    (custom.capture as Record<string, unknown>).passthroughHotkey = {
      ctrl: true,
      alt: false,
      shift: true,
      key: "K",
    };
    expect(parseSettingsSnapshot(custom).capture.passthroughHotkey.key).toBe(
      "K",
    );

    const duplicate = settingsFixture();
    (duplicate.mainDps as Record<string, unknown>).metrics = [
      "team-dps",
      "team-dps",
    ];
    expect(() => parseSettingsSnapshot(duplicate)).toThrow(
      /display identifiers must be unique/,
    );
  });

  it("requires explicit availability instead of treating failures as empty state", () => {
    const missingDevices = settingsFixture();
    delete (missingDevices.capture as Record<string, unknown>).devicesAvailable;
    expect(() => parseSettingsSnapshot(missingDevices)).toThrow(
      /settings.capture.devicesAvailable must be a boolean/,
    );

    const missingTeams = settingsFixture();
    delete (missingTeams.teamData as Record<string, unknown>).available;
    expect(() => parseSettingsSnapshot(missingTeams)).toThrow(
      /settings.teamData.available must be a boolean/,
    );
  });

  it("parses native team data import results without a local path", () => {
    const result = parseTeamDataImportFileResult({
      performed: true,
      settings: settingsFixture(),
    });

    expect(result.performed).toBe(true);
    expect(result.settings.generation).toBe("7");
    expect(result).not.toHaveProperty("path");
  });

  it("orders decimal generations without unsafe number conversion", () => {
    expect(compareSettingsGeneration("9", "10")).toBeLessThan(0);
    expect(
      compareSettingsGeneration("18446744073709551615", "11"),
    ).toBeGreaterThan(0);
    expect(compareSettingsGeneration("7", "7")).toBe(0);
  });

  it("parses available, downloading and prepared update projections", () => {
    const fixture = settingsFixture();
    fixture.updates = {
      ...(fixture.updates as Record<string, unknown>),
      status: "downloading",
      available: [
        {
          component: "app",
          version: "0.4.0",
          publishedAt: "2026-07-31T00:00:00Z",
          notes: "Verified release",
          artifactSize: "2048",
        },
      ],
      activeComponent: "app",
      downloadedBytes: "1024",
      totalBytes: "2048",
      prepared: null,
    };

    const downloading = parseSettingsSnapshot(fixture);
    expect(downloading.updates.available[0]?.component).toBe("app");
    expect(downloading.updates.downloadedBytes).toBe("1024");

    fixture.updates = {
      ...(fixture.updates as Record<string, unknown>),
      status: "ready",
      activeComponent: null,
      downloadedBytes: "2048",
      prepared: { component: "app", version: "0.4.0" },
      installEnabled: true,
    };
    expect(parseSettingsSnapshot(fixture).updates.prepared?.version).toBe(
      "0.4.0",
    );
  });

  it("rejects inconsistent update component and progress projections", () => {
    const duplicate = settingsFixture();
    duplicate.updates = {
      ...(duplicate.updates as Record<string, unknown>),
      available: [
        {
          component: "app",
          version: "0.4.0",
          publishedAt: "date",
          notes: "notes",
          artifactSize: "1",
        },
        {
          component: "app",
          version: "0.4.1",
          publishedAt: "date",
          notes: "notes",
          artifactSize: "1",
        },
      ],
    };
    expect(() => parseSettingsSnapshot(duplicate)).toThrow(/at most once/);

    const invalidProgress = settingsFixture();
    invalidProgress.updates = {
      ...(invalidProgress.updates as Record<string, unknown>),
      downloadedBytes: "2",
      totalBytes: "1",
    };
    expect(() => parseSettingsSnapshot(invalidProgress)).toThrow(
      /must not exceed/,
    );
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
      /must be a valid decimal string/,
    );
  });
});
