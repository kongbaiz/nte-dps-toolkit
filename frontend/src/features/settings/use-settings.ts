import { useCallback, useEffect, useRef, useState } from "react";

import {
  settingsClient,
  settingsError,
  type SettingsClient,
} from "@/lib/tauri/settings-client";
import type {
  CaptureSettingsInput,
  GlobalHotkeyActionId,
  HotkeyBinding,
  HudPresetId,
  HudSettingOptionId,
  InterfaceSettingsInput,
  LayoutProfileId,
  SettingsCommandError,
  SettingsSnapshot,
} from "@/lib/tauri/settings-contract";
import { applySettingsPresentation } from "@/lib/settings-presentation";
import type { HudModuleId } from "@/lib/tauri/technical-contract";

import type { SettingsPageState } from "./settings-view-model";

export function useSettings(client: SettingsClient = settingsClient) {
  const [state, setState] = useState<SettingsPageState>({
    status: "loading",
  });
  const [pendingAction, setPendingAction] = useState<string | null>(null);
  const [mutationError, setMutationError] =
    useState<SettingsCommandError | null>(null);
  const mounted = useRef(true);
  const mutationPending = useRef(false);
  const loadGeneration = useRef(0);

  useEffect(() => {
    mounted.current = true;
    return () => {
      mounted.current = false;
    };
  }, []);

  const refresh = useCallback(async () => {
    const generation = ++loadGeneration.current;
    setState({ status: "loading" });
    setMutationError(null);
    try {
      const snapshot = await client.getSnapshot();
      if (mounted.current && generation === loadGeneration.current) {
        applySettingsPresentation(snapshot.interface);
        setState({ status: "ready", snapshot });
      }
    } catch (error) {
      if (mounted.current && generation === loadGeneration.current) {
        setState({ status: "error", error: settingsError(error) });
      }
    }
  }, [client]);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  const mutate = useCallback(
    async (
      action: string,
      command: () => Promise<SettingsSnapshot>,
    ): Promise<void> => {
      if (mutationPending.current) {
        return;
      }
      mutationPending.current = true;
      setPendingAction(action);
      setMutationError(null);
      try {
        const snapshot = await command();
        if (mounted.current) {
          applySettingsPresentation(snapshot.interface);
          loadGeneration.current += 1;
          setState({ status: "ready", snapshot });
        }
      } catch (error) {
        if (mounted.current) {
          setMutationError(settingsError(error));
        }
      } finally {
        mutationPending.current = false;
        if (mounted.current) {
          setPendingAction(null);
        }
      }
    },
    [],
  );

  return {
    state,
    pendingAction,
    mutationError,
    clearMutationError: () => setMutationError(null),
    refresh,
    setInterface: (settings: InterfaceSettingsInput) =>
      mutate("interface", () => client.setInterface(settings)),
    setUpdatePreferences: (autoCheck: boolean, autoDownload: boolean) =>
      mutate("update-preferences", () =>
        client.setUpdatePreferences({ autoCheck, autoDownload }),
      ),
    checkUpdates: () => mutate("update-check", () => client.checkUpdates()),
    setCapture: (settings: CaptureSettingsInput) =>
      mutate("capture", () => client.setCapture(settings)),
    refreshCaptureDevices: () =>
      mutate("capture-devices", () => client.refreshCaptureDevices()),
    setHotkeysEnabled: (enabled: boolean) =>
      mutate("hotkeys-enabled", () => client.setHotkeysEnabled(enabled)),
    setHotkeyBinding: (
      action: GlobalHotkeyActionId,
      binding: HotkeyBinding | null,
    ) =>
      mutate(`hotkey:${action}`, () =>
        client.setHotkeyBinding(action, binding),
      ),
    applyLayoutProfile: (profile: LayoutProfileId) =>
      mutate(`layout:${profile}`, () => client.applyLayoutProfile(profile)),
    openAbyssValues: () =>
      mutate("abyss-values", () => client.openAbyssValues()),
    importTeamData: (json: string) =>
      mutate("team-import", () => client.importTeamData(json)),
    exportTeamData: async (): Promise<string | null> => {
      if (mutationPending.current) {
        return null;
      }
      mutationPending.current = true;
      setPendingAction("team-export");
      setMutationError(null);
      try {
        return await client.exportTeamData();
      } catch (error) {
        if (mounted.current) {
          setMutationError(settingsError(error));
        }
        return null;
      } finally {
        mutationPending.current = false;
        if (mounted.current) {
          setPendingAction(null);
        }
      }
    },
    refreshCaptureFiles: () =>
      mutate("capture-files-refresh", () => client.refreshCaptureFiles()),
    clearCaptureFiles: () =>
      mutate("capture-files-clear", () => client.clearCaptureFiles()),
    setHudOption: (option: HudSettingOptionId, enabled: boolean) =>
      mutate(`option:${option}`, () => client.setHudOption(option, enabled)),
    applyHudPreset: (preset: HudPresetId) =>
      mutate(`preset:${preset}`, () => client.applyHudPreset(preset)),
    setHudModuleVisibility: (module: HudModuleId, visible: boolean) =>
      mutate(`module:${module}`, () =>
        client.setHudModuleVisibility(module, visible),
      ),
    moveHudModule: (
      dragged: HudModuleId,
      target: HudModuleId,
      insertAfter: boolean,
    ) =>
      mutate(`move:${dragged}`, () =>
        client.moveHudModule(dragged, target, insertAfter),
      ),
    setHudWidth: (width: number) =>
      mutate("width", () => client.setHudWidth(width)),
    setHudAlwaysOnTop: (enabled: boolean) =>
      mutate("always-on-top", () => client.setHudAlwaysOnTop(enabled)),
    openHudEditor: () => mutate("open-editor", () => client.openHudEditor()),
  };
}
