import { invoke } from "@tauri-apps/api/core";

import {
  parseSettingsSnapshot,
  type CaptureSettingsInput,
  type GlobalHotkeyActionId,
  type HotkeyBinding,
  type HudPresetId,
  type HudSettingOptionId,
  type InterfaceSettingsInput,
  type LayoutProfileId,
  type SettingsCommandError,
  type SettingsSnapshot,
  type UpdateSettingsInput,
} from "@/lib/tauri/settings-contract";
import {
  parseTechnicalCommandError,
  TechnicalContractError,
  type HudModuleId,
} from "@/lib/tauri/technical-contract";

const COMMANDS = {
  applyHudPreset: "apply_settings_hud_preset",
  applyLayoutProfile: "apply_settings_layout_profile",
  checkUpdates: "check_settings_updates",
  clearCaptureFiles: "clear_settings_capture_files",
  exportTeamData: "export_settings_team_data",
  getSnapshot: "get_settings_snapshot",
  importTeamData: "import_settings_team_data",
  moveHudModule: "move_settings_hud_module",
  openHudEditor: "open_settings_hud_editor",
  openAbyssValues: "open_settings_abyss_values",
  refreshCaptureDevices: "refresh_settings_capture_devices",
  refreshCaptureFiles: "refresh_settings_capture_files",
  setCapture: "set_settings_capture",
  setHudAlwaysOnTop: "set_settings_hud_always_on_top",
  setHudModuleVisibility: "set_settings_hud_module_visibility",
  setHudOption: "set_settings_hud_option",
  setHudWidth: "set_settings_hud_width",
  setHotkeyBinding: "set_settings_hotkey_binding",
  setHotkeysEnabled: "set_settings_hotkeys_enabled",
  setInterface: "set_settings_interface",
  setUpdatePreferences: "set_settings_update_preferences",
} as const;

interface SettingsTransport {
  invoke(
    command: string,
    arguments_?: Record<string, unknown>,
  ): Promise<unknown>;
}

export interface SettingsClient {
  getSnapshot(): Promise<SettingsSnapshot>;
  setInterface(settings: InterfaceSettingsInput): Promise<SettingsSnapshot>;
  setUpdatePreferences(
    settings: UpdateSettingsInput,
  ): Promise<SettingsSnapshot>;
  checkUpdates(): Promise<SettingsSnapshot>;
  setCapture(settings: CaptureSettingsInput): Promise<SettingsSnapshot>;
  refreshCaptureDevices(): Promise<SettingsSnapshot>;
  setHotkeysEnabled(enabled: boolean): Promise<SettingsSnapshot>;
  setHotkeyBinding(
    action: GlobalHotkeyActionId,
    binding: HotkeyBinding | null,
  ): Promise<SettingsSnapshot>;
  applyLayoutProfile(profile: LayoutProfileId): Promise<SettingsSnapshot>;
  openAbyssValues(): Promise<SettingsSnapshot>;
  importTeamData(json: string): Promise<SettingsSnapshot>;
  exportTeamData(): Promise<string>;
  refreshCaptureFiles(): Promise<SettingsSnapshot>;
  clearCaptureFiles(): Promise<SettingsSnapshot>;
  setHudOption(
    option: HudSettingOptionId,
    enabled: boolean,
  ): Promise<SettingsSnapshot>;
  applyHudPreset(preset: HudPresetId): Promise<SettingsSnapshot>;
  setHudModuleVisibility(
    module: HudModuleId,
    visible: boolean,
  ): Promise<SettingsSnapshot>;
  moveHudModule(
    dragged: HudModuleId,
    target: HudModuleId,
    insertAfter: boolean,
  ): Promise<SettingsSnapshot>;
  setHudWidth(width: number): Promise<SettingsSnapshot>;
  setHudAlwaysOnTop(enabled: boolean): Promise<SettingsSnapshot>;
  openHudEditor(): Promise<SettingsSnapshot>;
}

const tauriTransport: SettingsTransport = {
  invoke: (command, arguments_) => invoke<unknown>(command, arguments_),
};

export function createSettingsClient(
  transport: SettingsTransport = tauriTransport,
): SettingsClient {
  async function snapshotCommand(
    command: string,
    arguments_?: Record<string, unknown>,
  ): Promise<SettingsSnapshot> {
    try {
      return parseSettingsSnapshot(await transport.invoke(command, arguments_));
    } catch (error) {
      if (error instanceof TechnicalContractError) {
        throw error;
      }
      throw parseTechnicalCommandError(error);
    }
  }

  return {
    getSnapshot: () => snapshotCommand(COMMANDS.getSnapshot),
    setInterface: (settings) =>
      snapshotCommand(COMMANDS.setInterface, { settings }),
    setUpdatePreferences: (settings) =>
      snapshotCommand(COMMANDS.setUpdatePreferences, { settings }),
    checkUpdates: () => snapshotCommand(COMMANDS.checkUpdates),
    setCapture: (settings) =>
      snapshotCommand(COMMANDS.setCapture, { settings }),
    refreshCaptureDevices: () =>
      snapshotCommand(COMMANDS.refreshCaptureDevices),
    setHotkeysEnabled: (enabled) =>
      snapshotCommand(COMMANDS.setHotkeysEnabled, { enabled }),
    setHotkeyBinding: (action, binding) =>
      snapshotCommand(COMMANDS.setHotkeyBinding, { action, binding }),
    applyLayoutProfile: (profile) =>
      snapshotCommand(COMMANDS.applyLayoutProfile, { profile }),
    openAbyssValues: () => snapshotCommand(COMMANDS.openAbyssValues),
    importTeamData: (json) =>
      snapshotCommand(COMMANDS.importTeamData, { json }),
    exportTeamData: async () => {
      try {
        const value = await transport.invoke(COMMANDS.exportTeamData);
        if (typeof value !== "string") {
          throw new TechnicalContractError(
            "team data export must be a JSON string",
          );
        }
        return value;
      } catch (error) {
        if (error instanceof TechnicalContractError) {
          throw error;
        }
        throw parseTechnicalCommandError(error);
      }
    },
    refreshCaptureFiles: () => snapshotCommand(COMMANDS.refreshCaptureFiles),
    clearCaptureFiles: () => snapshotCommand(COMMANDS.clearCaptureFiles),
    setHudOption: (option, enabled) =>
      snapshotCommand(COMMANDS.setHudOption, { option, enabled }),
    applyHudPreset: (preset) =>
      snapshotCommand(COMMANDS.applyHudPreset, { preset }),
    setHudModuleVisibility: (module, visible) =>
      snapshotCommand(COMMANDS.setHudModuleVisibility, { module, visible }),
    moveHudModule: (dragged, target, insertAfter) =>
      snapshotCommand(COMMANDS.moveHudModule, {
        dragged,
        target,
        insertAfter,
      }),
    setHudWidth: (width) => snapshotCommand(COMMANDS.setHudWidth, { width }),
    setHudAlwaysOnTop: (enabled) =>
      snapshotCommand(COMMANDS.setHudAlwaysOnTop, { enabled }),
    openHudEditor: () => snapshotCommand(COMMANDS.openHudEditor),
  };
}

export function settingsError(error: unknown): SettingsCommandError {
  return parseTechnicalCommandError(error);
}

export const settingsClient = createSettingsClient();
