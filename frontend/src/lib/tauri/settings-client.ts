import { invokeCommand, type CommandError } from "@/lib/tauri/command-error";
import type { CaptureSettingsInput } from "@/lib/tauri/generated/CaptureSettingsInput";
import type { GlobalHotkeyAction } from "@/lib/tauri/generated/GlobalHotkeyAction";
import type { HotkeyBindingSnapshot } from "@/lib/tauri/generated/HotkeyBindingSnapshot";
import type { HudModule } from "@/lib/tauri/generated/HudModule";
import type { HudPreset } from "@/lib/tauri/generated/HudPreset";
import type { HudSettingOption } from "@/lib/tauri/generated/HudSettingOption";
import type { InterfaceSettingsInput } from "@/lib/tauri/generated/InterfaceSettingsInput";
import type { LayoutProfile } from "@/lib/tauri/generated/LayoutProfile";
import type { MainDpsDisplayConfig } from "@/lib/tauri/generated/MainDpsDisplayConfig";
import type { SettingsEvent } from "@/lib/tauri/generated/SettingsEvent";
import type { SettingsSnapshot } from "@/lib/tauri/generated/SettingsSnapshot";
import type { TeamDataExportResult } from "@/lib/tauri/generated/TeamDataExportResult";
import type { TeamDataImportFileResult } from "@/lib/tauri/generated/TeamDataImportFileResult";
import type { UpdateComponent } from "@/lib/tauri/generated/UpdateComponent";
import type { UpdateSettingsInput } from "@/lib/tauri/generated/UpdateSettingsInput";
import { subscribeStream } from "@/lib/tauri/stream-client";

const snapshot = (command: string, arguments_?: Record<string, unknown>) =>
  invokeCommand<SettingsSnapshot>(command, arguments_);

export const settingsClient = {
  getSnapshot: () => snapshot("get_settings_snapshot"),
  setInterface: (settings: InterfaceSettingsInput) =>
    snapshot("set_settings_interface", { settings }),
  setUpdatePreferences: (settings: UpdateSettingsInput) =>
    snapshot("set_settings_update_preferences", { settings }),
  checkUpdates: () => snapshot("check_settings_updates"),
  downloadUpdate: (component: UpdateComponent) =>
    snapshot("download_settings_update", { component }),
  installUpdate: () => snapshot("install_settings_update"),
  setCapture: (settings: CaptureSettingsInput) =>
    snapshot("set_settings_capture", { settings }),
  refreshCaptureDevices: () => snapshot("refresh_settings_capture_devices"),
  setHotkeysEnabled: (enabled: boolean) =>
    snapshot("set_settings_hotkeys_enabled", { enabled }),
  setHotkeyBinding: (
    action: GlobalHotkeyAction,
    binding: HotkeyBindingSnapshot | null,
  ) => snapshot("set_settings_hotkey_binding", { action, binding }),
  setMainDpsDisplay: (settings: MainDpsDisplayConfig) =>
    snapshot("set_settings_main_dps_display", { settings }),
  applyLayoutProfile: (profile: LayoutProfile) =>
    snapshot("apply_settings_layout_profile", { profile }),
  openAbyssValues: () => snapshot("open_settings_abyss_values"),
  importTeamData: (json: string) =>
    snapshot("import_settings_team_data", { json }),
  importTeamDataFile: () =>
    invokeCommand<TeamDataImportFileResult>("import_settings_team_data_file"),
  exportTeamData: async () =>
    (await invokeCommand<TeamDataExportResult>("export_settings_team_data"))
      .saved,
  refreshCaptureFiles: () => snapshot("refresh_settings_capture_files"),
  clearCaptureFiles: () => snapshot("clear_settings_capture_files"),
  setHudOption: (option: HudSettingOption, enabled: boolean) =>
    snapshot("set_settings_hud_option", { option, enabled }),
  applyHudPreset: (preset: HudPreset) =>
    snapshot("apply_settings_hud_preset", { preset }),
  setHudModuleVisibility: (module: HudModule, visible: boolean) =>
    snapshot("set_settings_hud_module_visibility", { module, visible }),
  moveHudModule: (
    dragged: HudModule,
    target: HudModule,
    insertAfter: boolean,
  ) => snapshot("move_settings_hud_module", { dragged, target, insertAfter }),
  setHudWidth: (width: number) => snapshot("set_settings_hud_width", { width }),
  setHudAlwaysOnTop: (enabled: boolean) =>
    snapshot("set_settings_hud_always_on_top", { enabled }),
  openHudEditor: () => snapshot("open_settings_hud_editor"),
  subscribe: (
    onSnapshot: (snapshot: SettingsSnapshot) => void,
    onError: (error: CommandError) => void,
  ) =>
    subscribeStream<SettingsEvent>({
      streamKind: "settings",
      subscribeCommand: "subscribe_settings",
      unsubscribeCommand: "unsubscribe_settings",
      onEvent: (event) => onSnapshot(event.payload),
      onError,
    }),
};
