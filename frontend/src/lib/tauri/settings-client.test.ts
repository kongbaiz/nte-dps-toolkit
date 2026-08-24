import { describe, expect, it, vi } from "vitest";

import { HUD_MODULE_IDS } from "@/lib/tauri/technical-contract";

import { createSettingsClient } from "./settings-client";
import {
  SETTINGS_CONTRACT_VERSION,
  type SettingsSnapshot,
} from "./settings-contract";

const encodeDelivery = (events: unknown[]) => ({
  streamProtocolVersion: 1,
  events,
});

function settingsFixture(): SettingsSnapshot {
  return {
    contractVersion: SETTINGS_CONTRACT_VERSION,
    generation: "0",
    adapterVersion: "0.3.6",
    interface: {
      language: "zh-CN",
      darkMode: false,
      themePreset: "zinc",
      accent: "zinc",
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
      devices: [],
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
        { action: "capture", binding: null },
        { action: "reset", binding: null },
        { action: "hud", binding: null },
        { action: "new-round", binding: null },
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
    teamData: { available: true, upperImported: false, lowerImported: false },
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

describe("settings client", () => {
  it("uses typed command names and camelCase arguments", async () => {
    const calls: Array<{
      command: string;
      arguments_?: Record<string, unknown>;
    }> = [];
    const client = createSettingsClient({
      invoke: async (command, arguments_) => {
        calls.push({ command, arguments_ });
        return settingsFixture();
      },
      createChannel: () => ({}),
    });

    await client.setHudOption("damage_taken", true);
    await client.setInterface({
      language: "ja",
      darkMode: true,
      themePreset: "tactical",
      accent: "orange",
      density: "compact",
      reduceMotion: true,
      islandNotifications: false,
      islandOffsetX: 40,
    });
    await client.setHotkeyBinding("capture", {
      ctrl: true,
      alt: false,
      shift: false,
      key: "F8",
    });
    await client.setMainDpsDisplay({
      metrics: ["team-dps", "duration"],
      attributions: ["character", "max-hp-reduction"],
    });
    await client.applyHudPreset("detailed");
    await client.moveHudModule("title", "summary", true);
    await client.setHudWidth(512);
    await client.downloadUpdate("mods-plugin");
    await client.installUpdate();

    expect(calls).toEqual([
      {
        command: "set_settings_hud_option",
        arguments_: { option: "damage_taken", enabled: true },
      },
      {
        command: "set_settings_interface",
        arguments_: {
          settings: {
            language: "ja",
            darkMode: true,
            themePreset: "tactical",
            accent: "orange",
            density: "compact",
            reduceMotion: true,
            islandNotifications: false,
            islandOffsetX: 40,
          },
        },
      },
      {
        command: "set_settings_hotkey_binding",
        arguments_: {
          action: "capture",
          binding: {
            ctrl: true,
            alt: false,
            shift: false,
            key: "F8",
          },
        },
      },
      {
        command: "set_settings_main_dps_display",
        arguments_: {
          settings: {
            metrics: ["team-dps", "duration"],
            attributions: ["character", "max-hp-reduction"],
          },
        },
      },
      {
        command: "apply_settings_hud_preset",
        arguments_: { preset: "detailed" },
      },
      {
        command: "move_settings_hud_module",
        arguments_: {
          dragged: "title",
          target: "summary",
          insertAfter: true,
        },
      },
      {
        command: "set_settings_hud_width",
        arguments_: { width: 512 },
      },
      {
        command: "download_settings_update",
        arguments_: { component: "mods-plugin" },
      },
      {
        command: "install_settings_update",
        arguments_: undefined,
      },
    ]);
  });

  it("normalizes Rust command failures", async () => {
    const client = createSettingsClient({
      invoke: async () => {
        throw {
          code: "hud_config_save_failed",
          messageKey: "Failed to save HUD configuration.",
          messageArguments: [],
        };
      },
      createChannel: () => ({}),
    });

    await expect(client.getSnapshot()).rejects.toMatchObject({
      code: "hud_config_save_failed",
      messageKey: "Failed to save HUD configuration.",
    });
  });

  it("distinguishes a saved export from a cancelled save dialog", async () => {
    const results = [{ saved: true }, { saved: false }];
    const client = createSettingsClient({
      invoke: async (command) => {
        expect(command).toBe("export_settings_team_data");
        return results.shift();
      },
      createChannel: () => ({}),
    });

    await expect(client.exportTeamData()).resolves.toBe(true);
    await expect(client.exportTeamData()).resolves.toBe(false);
  });

  it("uses the native team data import command and preserves cancellation", async () => {
    const calls: string[] = [];
    const client = createSettingsClient({
      invoke: async (command) => {
        calls.push(command);
        return { performed: false, settings: settingsFixture() };
      },
      createChannel: () => ({}),
    });

    await expect(client.importTeamDataFile()).resolves.toMatchObject({
      performed: false,
      settings: { generation: "0" },
    });
    expect(calls).toEqual(["import_settings_team_data_file"]);
  });

  it("streams settled startup update snapshots and unregisters on cleanup", async () => {
    let onMessage: (message: unknown) => void = () => {
      throw new Error("settings channel was not initialized");
    };
    const calls: Array<{
      command: string;
      arguments_?: Record<string, unknown>;
    }> = [];
    const snapshots: ReturnType<typeof settingsFixture>[] = [];
    const settledEvent = {
      event: "snapshot",
      payload: {
        ...settingsFixture(),
        updates: {
          ...settingsFixture().updates,
          status: "up-to-date",
          messageKey: "NTE DPS Tool is up to date",
        },
      },
    };
    const client = createSettingsClient(
      {
        invoke: async (command, arguments_) => {
          calls.push({ command, arguments_ });
          if (command === "subscribe_settings") {
            return {
              subscriptionId: "settings-test",
              streamKind: "settings",
              streamIntervalMs: 200,
              streamProtocolVersion: 1,
              streamGeneration: "1",
            };
          }
          return undefined;
        },
        createChannel: (callback) => {
          onMessage = callback;
          return { channel: true };
        },
      },
      () => "settings-test",
    );

    const unsubscribe = client.subscribe(
      (snapshot) =>
        snapshots.push(snapshot as ReturnType<typeof settingsFixture>),
      (error) => {
        throw new Error(error.code);
      },
    );
    onMessage(encodeDelivery([settledEvent]));
    await vi.waitFor(() => expect(snapshots).toHaveLength(1));
    await unsubscribe();

    expect(snapshots[0]?.updates.status).toBe("up-to-date");
    expect(calls.map((call) => call.command)).toEqual([
      "subscribe_settings",
      "unsubscribe_settings",
    ]);
  });
});
